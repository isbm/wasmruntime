use crate::cfg::WasmConfig;
use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::{collections::HashMap, fs, sync::Mutex};
use wasi_common::I32Exit as CoreI32Exit;
use wasmtime::{Config, Engine, Linker, Module, Store};
use wasmtime_wasi::{I32Exit as WasiI32Exit, preview1::add_to_linker_async};
use wasmtime_wasi::{WasiCtxBuilder, p2::pipe::MemoryInputPipe};
use wasmtime_wasi::{p2::pipe::MemoryOutputPipe, preview1::WasiP1Ctx};

pub mod cfg;
pub struct WasmRuntime {
    engine: Engine,
    cfg: WasmConfig,
    linker: Linker<WasiP1Ctx>,
    modules: Mutex<HashMap<String, Module>>,
}

impl WasmRuntime {
    pub fn new(wcfg: WasmConfig) -> Result<Self> {
        let mut cfg = Config::new();
        cfg.async_support(true);
        let engine = Engine::new(&cfg)?;
        let mut linker: Linker<WasiP1Ctx> = Linker::new(&engine);
        add_to_linker_async(&mut linker, |cx| cx)?;

        Ok(Self { engine, linker, cfg: wcfg, modules: Mutex::new(HashMap::new()) })
    }

    pub fn objects(&self) -> Result<Vec<String>> {
        let mut ids = Vec::new();
        for entry in fs::read_dir(self.cfg.get_root_path())? {
            let entry = entry?;
            if entry.file_type()?.is_file() {
                let p = entry.path();
                if p.extension().and_then(|s| s.to_str()) == Some("wasm")
                    && let Some(stem) = p.file_stem().and_then(|s| s.to_str())
                {
                    ids.push(stem.to_string());
                }
            }
        }
        ids.sort();
        Ok(ids)
    }

    fn get_or_load_module(&self, id: &str) -> Result<Module> {
        if let Some(m) = self.modules.lock().unwrap().get(id).cloned() {
            return Ok(m);
        }
        // Load + compile, cache
        let path = self.cfg.get_root_path().join(format!("{id}.wasm"));
        let module = Module::from_file(&self.engine, &path).with_context(|| format!("loading {path:?}"))?;
        self.modules.lock().unwrap().insert(id.to_string(), module.clone());
        Ok(module)
    }

    pub async fn run(&self, id: &str, opts: Vec<String>, args: HashMap<String, Value>) -> Result<Value> {
        let module = self.get_or_load_module(id)?;

        // JSON-in on stdin; JSON-out on stdout
        let input = json!({ "opts": opts, "args": args }).to_string().into_bytes();
        let stdin = MemoryInputPipe::new(input);
        let stdout = MemoryOutputPipe::new(64 * 1024);
        let stderr = MemoryOutputPipe::new(64 * 1024);

        let mut wb = WasiCtxBuilder::new();
        let mut wb = wb
            .stdin(stdin)
            .stdout(stdout.clone())
            .stderr(stderr.clone())
            .allow_tcp(self.cfg.get_allow_network())
            .allow_udp(self.cfg.get_allow_network());

        if self.cfg.get_allow_write() {
            if fs::metadata(self.cfg.get_host_path()).is_err() {
                fs::create_dir_all(self.cfg.get_host_path()).with_context(|| format!("creating host path {:?}", self.cfg.get_host_path()))?;
            }
            wb = wb.preopened_dir(self.cfg.get_host_path(), self.cfg.get_guest_path(), self.cfg.get_dir_perms(), self.cfg.get_file_perms())?;
        }

        let wasi = wb.build_p1();

        let mut store: Store<WasiP1Ctx> = Store::new(&self.engine, wasi);
        let instance = self.linker.instantiate_async(&mut store, &module).await?;
        let start = instance.get_typed_func::<(), ()>(&mut store, "_start").context("module missing _start")?;
        match start.call_async(&mut store, ()).await {
            Ok(()) => {}
            Err(e) => {
                // Accept proc_exit(0) as success (both possible I32Exit types)
                if let Some(exit) = e.downcast_ref::<WasiI32Exit>() {
                    if exit.0 != 0 {
                        bail!("module exited with status {}", exit.0);
                    }
                } else if let Some(exit) = e.downcast_ref::<CoreI32Exit>() {
                    if exit.0 != 0 {
                        bail!("module exited with status {}", exit.0);
                    }
                } else {
                    return Err(e);
                }
            }
        }
        let text = String::from_utf8(stdout.contents().to_vec())?;
        let val: Value = if text.trim().is_empty() {
            json!(null)
        } else {
            serde_json::from_str(&text).with_context(|| format!("stdout was not valid JSON:\n{text}"))?
        };

        let err_bytes = stderr.contents();
        if !err_bytes.is_empty() {
            eprintln!("guest stderr:\n{}", String::from_utf8_lossy(&err_bytes));
        }
        Ok(val)
    }
}
