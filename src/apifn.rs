use anyhow::Result;
use serde::Deserialize;
use serde_json::Value;
use std::{
    process::Command,
    sync::{Arc, Mutex, OnceLock},
};
use wasmtime::{Caller, Extern, Linker, Memory};
use wasmtime_wasi::preview1::WasiP1Ctx;

/// Import module name exposed to guest Wasm code.
pub const API_NAMESPACE: &str = "api";

/// Per-instance host state carried inside the Wasmtime store.
///
/// This stores the WASI context plus generic request metadata and buffered log
/// lines that host helper functions may need while serving guest calls.
pub struct HostState {
    wasi: WasiP1Ctx,
    logs: Arc<Mutex<Vec<String>>>,
    module: String,
    header: Value,
}

impl HostState {
    /// Create a new host state value for a single guest module run.
    pub fn new(wasi: WasiP1Ctx, logs: Arc<Mutex<Vec<String>>>, module: String, header: Value) -> Self {
        Self { wasi, logs, module, header }
    }

    /// Return the mutable WASI Preview 1 context used by the guest instance.
    pub fn wasi(&mut self) -> &mut WasiP1Ctx {
        &mut self.wasi
    }

    /// Drain buffered host-side log lines accumulated during the guest run.
    pub fn logs(&self) -> Vec<String> {
        match self.logs.lock() {
            Ok(mut g) => std::mem::take(&mut *g),
            Err(_) => Vec::new(),
        }
    }

    /// Return the logical module identifier of the currently running guest.
    pub fn module(&self) -> &str {
        &self.module
    }

    /// Return the JSON header associated with the current guest run.
    pub fn header(&self) -> &Value {
        &self.header
    }
}

/// Host `exec` request payload accepted from guest code.
#[derive(Debug, Deserialize)]
struct ExecReq {
    #[serde(default)]
    argv: Vec<String>,
    #[serde(default)]
    cwd: Option<String>,
}

/// Return a shared JSON `null` value for helpers that need a default payload.
fn null_value() -> &'static Value {
    static NULL: OnceLock<Value> = OnceLock::new();
    NULL.get_or_init(|| Value::Null)
}

/// Copy a byte slice into guest memory, truncating to the output capacity.
fn write_bytes(mem: &Memory, caller: &mut Caller<'_, HostState>, out_ptr: usize, out_cap: usize, bytes: &[u8]) -> i32 {
    let n = bytes.len().min(out_cap);
    let data_mut = mem.data_mut(caller);
    data_mut[out_ptr..out_ptr + n].copy_from_slice(&bytes[..n]);
    n as i32
}

/// Serialize JSON into guest memory and return the number of bytes written.
pub fn write_json(mem: &Memory, caller: &mut Caller<'_, HostState>, out_ptr: usize, out_cap: usize, value: &serde_json::Value) -> i32 {
    write_bytes(mem, caller, out_ptr, out_cap, &serde_json::to_vec(value).unwrap_or_else(|_| b"{}".to_vec()))
}

/// Serialize a simple `{ "error": ... }` JSON object into guest memory.
pub fn write_error(mem: &Memory, caller: &mut Caller<'_, HostState>, out_ptr: usize, out_cap: usize, msg: &str) -> i32 {
    write_json(mem, caller, out_ptr, out_cap, &serde_json::json!({ "error": msg }))
}

/// Validate and normalize a guest output buffer region.
///
/// Returns the output pointer/capacity pair when the region is inside guest
/// memory and large enough to hold at least one byte.
pub fn output_region(caller: &Caller<'_, HostState>, mem: &Memory, out_ptr: i32, out_cap: i32) -> Option<(usize, usize)> {
    if out_ptr < 0 || out_cap <= 0 {
        return None;
    }
    let (out_ptr, out_cap) = (out_ptr as usize, out_cap as usize);
    let data = mem.data(caller);
    let out_end = out_ptr.checked_add(out_cap)?;
    (out_end <= data.len()).then_some((out_ptr, out_cap))
}

/// Borrow a guest request byte slice from guest memory.
///
/// Returns `None` when the pointer/length pair is invalid or out of bounds.
pub fn request_bytes<'a>(caller: &'a Caller<'_, HostState>, mem: &'a Memory, req_ptr: i32, req_len: i32) -> Option<&'a [u8]> {
    if req_ptr < 0 || req_len <= 0 {
        return None;
    }
    let (req_ptr, req_len) = (req_ptr as usize, req_len as usize);
    let data = mem.data(caller);
    let req_end = req_ptr.checked_add(req_len)?;
    (req_end <= data.len()).then_some(&data[req_ptr..req_end])
}

/// Decode a UTF-8 request string from guest memory.
fn request_string(caller: &Caller<'_, HostState>, mem: &Memory, req_ptr: i32, req_len: i32) -> Option<String> {
    request_bytes(caller, mem, req_ptr, req_len).and_then(|bytes| std::str::from_utf8(bytes).ok()).map(str::to_string)
}

/// Register the generic host logging import exposed as `api.log`.
///
/// The guest passes a log level and a UTF-8 message pointer/length pair. The
/// host prefixes the message with timestamp and module id, then buffers it in
/// `HostState` for later retrieval by the runtime host.
pub fn fn_api_log(linker: &mut Linker<HostState>) -> anyhow::Result<()> {
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

/// Register the generic host command execution import exposed as `api.exec`.
///
/// Guests pass a JSON payload describing `argv` and optional `cwd`. The host
/// executes the command and returns a JSON object containing `exit_code`,
/// `stdout`, and `stderr`.
pub fn fn_api_exec(linker: &mut Linker<HostState>) -> Result<()> {
    linker.func_wrap(API_NAMESPACE, "exec", |mut caller: Caller<'_, HostState>, req_ptr: i32, req_len: i32, out_ptr: i32, out_cap: i32| -> i32 {
        let mem: Memory = match caller.get_export("memory") {
            Some(Extern::Memory(m)) => m,
            _ => return -2,
        };

        let Some(req_bytes) = request_bytes(&caller, &mem, req_ptr, req_len) else {
            return -2;
        };
        let Some((out_ptr, out_cap)) = output_region(&caller, &mem, out_ptr, out_cap) else {
            return -2;
        };

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
                return write_json(
                    &mem,
                    &mut caller,
                    out_ptr,
                    out_cap,
                    &serde_json::json!({
                        "exit_code": 127,
                        "stdout": "",
                        "stderr": e.to_string(),
                    }),
                );
            }
        };

        write_json(
            &mem,
            &mut caller,
            out_ptr,
            out_cap,
            &serde_json::json!({
                "exit_code": output.status.code().unwrap_or(1),
                "stdout": String::from_utf8_lossy(&output.stdout).to_string(),
                "stderr": String::from_utf8_lossy(&output.stderr).to_string(),
            }),
        )
    })?;

    Ok(())
}

/// Register generic JSON-header access helpers.
///
/// These imports expose the caller-provided runtime header to guest code
/// without assigning any domain-specific meaning to its contents:
/// * `api.header` returns the full header object.
/// * `api.header_has` checks whether a JSON pointer resolves inside the header.
/// * `api.header_get` returns the value at a JSON pointer, or `null`.
pub fn fn_api_header(linker: &mut Linker<HostState>) -> Result<()> {
    linker
        .func_wrap(API_NAMESPACE, "header", |mut caller: Caller<'_, HostState>, out_ptr: i32, out_cap: i32| -> i32 {
            let mem: Memory = match caller.get_export("memory") {
                Some(Extern::Memory(m)) => m,
                _ => return -2,
            };
            let Some((out_ptr, out_cap)) = output_region(&caller, &mem, out_ptr, out_cap) else {
                return -2;
            };

            let header = caller.data().header().clone();
            write_json(&mem, &mut caller, out_ptr, out_cap, &header)
        })
        .map_err(|err| anyhow::anyhow!("Failed to register Wasm header helper: {err}"))?;

    linker
        .func_wrap(API_NAMESPACE, "header_has", |mut caller: Caller<'_, HostState>, req_ptr: i32, req_len: i32| -> i32 {
            let mem: Memory = match caller.get_export("memory") {
                Some(Extern::Memory(m)) => m,
                _ => return -2,
            };
            let Some(pointer) = request_string(&caller, &mem, req_ptr, req_len) else {
                return -2;
            };

            if caller.data().header().pointer(pointer.trim()).is_some() { 1 } else { 0 }
        })
        .map_err(|err| anyhow::anyhow!("Failed to register Wasm header_has helper: {err}"))?;

    linker
        .func_wrap(API_NAMESPACE, "header_get", |mut caller: Caller<'_, HostState>, req_ptr: i32, req_len: i32, out_ptr: i32, out_cap: i32| -> i32 {
            let mem: Memory = match caller.get_export("memory") {
                Some(Extern::Memory(m)) => m,
                _ => return -2,
            };
            let Some(pointer) = request_string(&caller, &mem, req_ptr, req_len) else {
                return -2;
            };
            let Some((out_ptr, out_cap)) = output_region(&caller, &mem, out_ptr, out_cap) else {
                return -2;
            };

            let value = caller.data().header().pointer(pointer.trim()).cloned().unwrap_or_else(|| null_value().clone());
            write_json(&mem, &mut caller, out_ptr, out_cap, &value)
        })
        .map_err(|err| anyhow::anyhow!("Failed to register Wasm header_get helper: {err}"))?;

    Ok(())
}
