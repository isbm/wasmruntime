use crate::cfg::WasmConfig;
use anyhow::{Context, Result};
use serde_json::Value;
use std::collections::HashMap;
use std::{fs, sync::Mutex};
use wasmtime::{Config, Engine, Linker, Module, Store};
use wasmtime_wasi::p2::pipe::{MemoryInputPipe, MemoryOutputPipe};
use wasmtime_wasi::preview1::WasiP1Ctx;
use wasmtime_wasi::preview1::add_to_linker_async;

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

    pub async fn run(&self, id: &str, opts: Vec<String>, args: HashMap<String, Value>, data: Vec<u8>) -> Result<Value> {
        let module = self.get_or_load_module(id)?;
        let mut input = serde_json::json!({ "opts": opts, "args": args }).to_string().into_bytes();
        input.push(b'\n');

        input.extend_from_slice(&data);

        let stdin = MemoryInputPipe::new(input);
        let stdout = MemoryOutputPipe::new(64 * 1024);
        let stderr = MemoryOutputPipe::new(64 * 1024);

        let mut wb = wasmtime_wasi::WasiCtxBuilder::new();
        let mut wb = wb
            .stdin(stdin)
            .stdout(stdout.clone())
            .stderr(stderr.clone())
            .allow_tcp(self.cfg.get_allow_network())
            .allow_udp(self.cfg.get_allow_network());

        if self.cfg.get_allow_write() {
            if std::fs::metadata(self.cfg.get_host_path()).is_err() {
                std::fs::create_dir_all(self.cfg.get_host_path()).with_context(|| format!("creating host path {:?}", self.cfg.get_host_path()))?;
            }
            wb = wb.preopened_dir(self.cfg.get_host_path(), self.cfg.get_guest_path(), self.cfg.get_dir_perms(), self.cfg.get_file_perms())?;
        }

        let wasi = wb.build_p1();
        let mut store: Store<wasmtime_wasi::preview1::WasiP1Ctx> = Store::new(&self.engine, wasi);
        let instance = self.linker.instantiate_async(&mut store, &module).await?;
        let start = instance.get_typed_func::<(), ()>(&mut store, "_start").context("module missing _start")?;

        match start.call_async(&mut store, ()).await {
            Ok(()) => {}
            Err(e) => {
                if let Some(exit) = e.downcast_ref::<wasmtime_wasi::I32Exit>() {
                    if exit.0 != 0 {
                        anyhow::bail!("module exited with status {}", exit.0);
                    }
                } else if let Some(exit) = e.downcast_ref::<wasi_common::I32Exit>() {
                    if exit.0 != 0 {
                        anyhow::bail!("module exited with status {}", exit.0);
                    }
                } else {
                    return Err(e);
                }
            }
        }

        let text = String::from_utf8(stdout.contents().to_vec())?;
        let val = if text.trim().is_empty() {
            serde_json::json!(null)
        } else {
            serde_json::from_str(&text).with_context(|| format!("stdout was not valid JSON:\n{text}"))?
        };

        let err = stderr.contents();
        if !err.is_empty() {
            eprintln!("guest stderr:\n{}", String::from_utf8_lossy(&err));
        }
        Ok(val)
    }
}
