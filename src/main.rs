use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    sync::Mutex,
};
use wasi_common::I32Exit as CoreI32Exit;
use wasmtime::{Config, Engine, Linker, Module, Store};
use wasmtime_wasi::I32Exit as WasiI32Exit;
use wasmtime_wasi::{WasiCtxBuilder, p2::pipe::MemoryInputPipe};
use wasmtime_wasi::{
    p2::pipe::MemoryOutputPipe,
    preview1::{WasiP1Ctx, add_to_linker_sync},
};

pub struct WasmRuntime {
    engine: Engine,
    dir: PathBuf,
    linker: Linker<WasiP1Ctx>,
    modules: Mutex<HashMap<String, Module>>,
}

impl WasmRuntime {
    pub fn new<P: AsRef<Path>>(dir: P) -> Result<Self> {
        let cfg = Config::new();
        let engine = Engine::new(&cfg)?;
        let mut linker: Linker<WasiP1Ctx> = Linker::new(&engine);
        add_to_linker_sync(&mut linker, |cx| cx)?;

        Ok(Self {
            engine,
            linker,
            dir: dir.as_ref().to_path_buf(),
            modules: Mutex::new(HashMap::new()),
        })
    }

    pub fn objects(&self) -> Result<Vec<String>> {
        let mut ids = Vec::new();
        for entry in fs::read_dir(&self.dir)? {
            let entry = entry?;
            if entry.file_type()?.is_file() {
                let p = entry.path();
                if p.extension().and_then(|s| s.to_str()) == Some("wasm")
                    && let Some(stem) = p.file_stem().and_then(|s| s.to_str()) {
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
        let path = self.dir.join(format!("{id}.wasm"));
        let module =
            Module::from_file(&self.engine, &path).with_context(|| format!("loading {path:?}"))?;
        self.modules
            .lock()
            .unwrap()
            .insert(id.to_string(), module.clone());
        Ok(module)
    }

    pub fn run(&self, id: &str, opts: Vec<String>, args: HashMap<String, Value>) -> Result<Value> {
        let module = self.get_or_load_module(id)?;

        // JSON-in on stdin; JSON-out on stdout
        let input = json!({ "opts": opts, "args": args })
            .to_string()
            .into_bytes();
        let stdin = MemoryInputPipe::new(input);
        let stdout = MemoryOutputPipe::new(64 * 1024);
        let stderr = MemoryOutputPipe::new(64 * 1024);

        let wasi = WasiCtxBuilder::new()
            .stdin(stdin)
            .stdout(stdout.clone())
            .stderr(stderr.clone())
            .build_p1();

        let mut store: Store<WasiP1Ctx> = Store::new(&self.engine, wasi);
        let instance = self.linker.instantiate(&mut store, &module)?;
        let start = instance
            .get_typed_func::<(), ()>(&mut store, "_start")
            .context("module missing _start")?;
        match start.call(&mut store, ()) {
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
            serde_json::from_str(&text)
                .with_context(|| format!("stdout was not valid JSON:\n{text}"))?
        };

        let err_bytes = stderr.contents();
        if !err_bytes.is_empty() {
            eprintln!("guest stderr:\n{}", String::from_utf8_lossy(&err_bytes));
        }
        Ok(val)
    }
}

fn main() -> anyhow::Result<()> {
    let rt = WasmRuntime::new("./wasm_bins")?;
    let ids = rt.objects()?;
    if ids.is_empty() {
        println!("no .wasm found in ./wasm_bins — put one there, e.g. wasm_bins/echo.wasm");
        return Ok(());
    }
    println!("found: {ids:?}");

    // demo inputs
    let opts = vec!["--demo".into(), "fast".into()];
    let args: HashMap<_, _> = [
        ("msg".to_string(), json!("hi from host")),
        ("n".to_string(), json!(3)),
    ]
    .into_iter()
    .collect();

    // run the first id
    for id in &ids {
        println!("running {id}...");
        let out = rt.run(id, opts.clone(), args.clone())?;
        println!("{} -> {}", id, out);
    }

    Ok(())
}
