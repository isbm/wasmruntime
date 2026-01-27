use anyhow::Result;
use serde::Deserialize;
use std::{
    process::Command,
    sync::{Arc, Mutex},
};
use wasmtime::{Caller, Extern, Linker, Memory};
use wasmtime_wasi::preview1::WasiP1Ctx;

static API_NAMESPACE: &str = "api";

pub(crate) struct HostState {
    wasi: WasiP1Ctx,
    logs: Arc<Mutex<Vec<String>>>,
    module: String,
}

impl HostState {
    pub(crate) fn new(wasi: WasiP1Ctx, logs: Arc<Mutex<Vec<String>>>, module: String) -> Self {
        Self { wasi, logs, module }
    }

    pub(crate) fn wasi(&mut self) -> &mut WasiP1Ctx {
        &mut self.wasi
    }

    pub(crate) fn logs(&self) -> Vec<String> {
        match self.logs.lock() {
            Ok(mut g) => std::mem::take(&mut *g),
            Err(_) => Vec::new(),
        }
    }
}

#[derive(Debug, Deserialize)]
struct ExecReq {
    #[serde(default)]
    argv: Vec<String>,
    #[serde(default)]
    cwd: Option<String>,
    // later: env: HashMap<String,String>, stdin, etc.
}

pub(crate) fn fn_api_log(linker: &mut Linker<HostState>) -> anyhow::Result<()> {
    linker.func_wrap("api", "log", |mut caller: Caller<'_, HostState>, level: i32, msg_ptr: i32, msg_len: i32| {
        let mem = match caller.get_export("memory") {
            Some(Extern::Memory(m)) => m,
            _ => return,
        };

        if msg_ptr < 0 || msg_len <= 0 {
            return;
        }
        let (ptr, len) = (msg_ptr as usize, msg_len as usize);

        let data = mem.data(&caller);
        let end = match ptr.checked_add(len) {
            Some(v) => v,
            None => return,
        };
        if end > data.len() {
            return;
        }

        let msg = match std::str::from_utf8(&data[ptr..end]) {
            Ok(s) => s,
            Err(_) => return,
        };

        let level_s = match level {
            0 => "DEBUG",
            1 => "INFO",
            2 => "WARN",
            3 => "ERROR",
            _ => "INFO",
        };

        let module = caller.data().module.as_str();
        let ts = chrono::Local::now().format("%d/%m/%Y %H:%M:%S");
        let line = format!("[{ts}] - {level_s}: [{module}] {msg}");

        if let Ok(mut g) = caller.data().logs.lock() {
            g.push(line);
        }
    })?;
    Ok(())
}

pub(crate) fn fn_api_exec(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(API_NAMESPACE, "exec", |mut caller: Caller<'_, HostState>, req_ptr: i32, req_len: i32, out_ptr: i32, out_cap: i32| -> i32 {
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
