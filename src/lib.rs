use crate::cfg::WasmConfig;
use anyhow::{Context, Result};
use serde::Deserialize;
use serde_json::Value;
use std::collections::HashMap;
use std::path::PathBuf;
use std::process::Command;
use std::{fs, sync::Mutex};
use wasmtime::{Caller, Config, Engine, Extern, Linker, Memory, Module, Store};
use wasmtime_wasi::p2::pipe::{MemoryInputPipe, MemoryOutputPipe};
use wasmtime_wasi::preview1::WasiP1Ctx;
use wasmtime_wasi::preview1::add_to_linker_async;

pub mod cfg;

#[derive(Debug, Deserialize)]
struct ExecReq {
    argv: Vec<String>,
    #[serde(default)]
    cwd: Option<String>,
    // later: env: HashMap<String,String>, stdin, etc.
}

fn add_sysinspect_imports(linker: &mut Linker<WasiP1Ctx>) -> Result<()> {
    linker.func_wrap("api", "exec", |mut caller: Caller<'_, WasiP1Ctx>, req_ptr: i32, req_len: i32, out_ptr: i32, out_cap: i32| -> i32 {
        let mem: Memory = match caller.get_export("memory") {
            Some(Extern::Memory(m)) => m,
            _ => return -2,
        };

        if req_ptr < 0 || req_len <= 0 || out_ptr < 0 || out_cap <= 0 {
            return -2;
        }
        let (req_ptr, req_len) = (req_ptr as usize, req_len as usize);
        let (out_ptr, out_cap) = (out_ptr as usize, out_cap as usize);

        // bounds check
        let data = mem.data(&caller);
        let req_end = match req_ptr.checked_add(req_len) {
            Some(v) => v,
            None => return -2,
        };
        let out_end = match out_ptr.checked_add(out_cap) {
            Some(v) => v,
            None => return -2,
        };
        if req_end > data.len() || out_end > data.len() {
            return -2;
        }

        // JSON
        let req_bytes = &data[req_ptr..req_end];
        let req_str = match std::str::from_utf8(req_bytes) {
            Ok(s) => s,
            Err(_) => return -2,
        };

        let req: ExecReq = match serde_json::from_str(req_str) {
            Ok(r) => r,
            Err(_) => return -2,
        };

        if req.argv.is_empty() {
            return -2;
        }

        // XXX: Here should be some security checks, e.g., allowed commands, etc.
        //      For now, we just run whatever is given. :-)
        let mut cmd = Command::new(&req.argv[0]);
        if req.argv.len() > 1 {
            cmd.args(&req.argv[1..]);
        }
        if let Some(cwd) = &req.cwd {
            cmd.current_dir(cwd);
        }

        let output = match cmd.output() {
            Ok(o) => o,
            Err(e) => {
                // Return JSON error into out buffer
                let resp = serde_json::json!({
                    "exit_code": 127,
                    "stdout": "",
                    "stderr": e.to_string(),
                });
                let bytes = serde_json::to_vec(&resp).unwrap_or_else(|_| b"{}".to_vec());
                let n = bytes.len().min(out_cap);
                let data_mut = mem.data_mut(&mut caller);
                data_mut[out_ptr..out_ptr + n].copy_from_slice(&bytes[..n]);
                return n as i32;
            }
        };

        let exit_code = output.status.code().unwrap_or(1);
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        let resp = serde_json::json!({
            "exit_code": exit_code,
            "stdout": stdout,
            "stderr": stderr,
        });

        let bytes = match serde_json::to_vec(&resp) {
            Ok(b) => b,
            Err(_) => b"{}".to_vec(),
        };

        let n = bytes.len().min(out_cap);
        let data_mut = mem.data_mut(&mut caller);
        data_mut[out_ptr..out_ptr + n].copy_from_slice(&bytes[..n]);
        n as i32
    })?;

    Ok(())
}

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
        cfg.cranelift_opt_level(wasmtime::OptLevel::SpeedAndSize);

        let engine = Engine::new(&cfg)?;
        let mut linker: Linker<WasiP1Ctx> = Linker::new(&engine);
        add_to_linker_async(&mut linker, |cx| cx)?;
        add_sysinspect_imports(&mut linker)?;

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

    /// Precompile `<id>.wasm` into `<id>.cwasm` using this host's Engine.
    pub fn precompile_module(&self, id: &str) -> Result<()> {
        let root = self.cfg.get_root_path();
        let wasm_path: PathBuf = root.join(format!("{id}.wasm"));
        let cwasm_path: PathBuf = root.join(format!("{id}.cwasm"));

        let wasm_bytes = std::fs::read(&wasm_path).with_context(|| format!("reading wasm file {wasm_path:?} for module '{id}'"))?;
        let compiled_bytes = self.engine.precompile_module(&wasm_bytes).with_context(|| format!("precompiling module '{id}' from {wasm_path:?}"))?;

        if let Some(parent) = cwasm_path.parent() {
            std::fs::create_dir_all(parent).with_context(|| format!("creating directory {parent:?} for cwasm output"))?;
        }

        std::fs::write(&cwasm_path, &compiled_bytes).with_context(|| format!("writing precompiled module to {cwasm_path:?}"))?;

        Ok(())
    }

    pub fn get_or_load_module(&self, id: &str) -> Result<Module> {
        if let Some(m) = self.modules.lock().unwrap().get(id).cloned() {
            return Ok(m);
        }

        let root = self.cfg.get_root_path();
        let cwasm_path = root.join(format!("{id}.cwasm"));

        // Precompile if .cwasm missing
        if !cwasm_path.exists() {
            self.precompile_module(id)?;
        }

        // Try to load .cwasm
        let first_attempt = unsafe { Module::deserialize_file(&self.engine, &cwasm_path) };
        let module = match first_attempt {
            Ok(module) => module,
            Err(err1) => {
                eprintln!(
                    "Failed to deserialize precompiled module {:?} (first attempt):\n{:#}\n\
                     Assuming it was precompiled with a different engine. Refreshing cwasm binary.",
                    cwasm_path, err1
                );

                let _ = std::fs::remove_file(&cwasm_path);
                self.precompile_module(id)?;

                match unsafe { Module::deserialize_file(&self.engine, &cwasm_path) } {
                    Ok(module) => module,
                    Err(err2) => {
                        // At this point something is really wrong (FS, engine, etc.)
                        return Err(anyhow::anyhow!(
                            "Failed to deserialize precompiled module {cwasm_path:?} even \
                             after deleting and recompiling.\nFirst error:\n{err1:#}\n\n\
                             Second error:\n{err2:#}"
                        ));
                    }
                }
            }
        };

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
