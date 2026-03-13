use crate::{API_NAMESPACE, HostState};
use serde_json::json;
use std::sync::{Arc, Mutex};

#[test]
fn api_namespace_is_stable() {
    assert_eq!(API_NAMESPACE, "api");
}

#[test]
fn host_state_exposes_module_and_header() {
    let wasi = wasmtime_wasi::WasiCtxBuilder::new().build_p1();
    let logs = Arc::new(Mutex::new(Vec::new()));
    let header = json!({ "args": { "name": "world" }, "opts": ["fast"] });
    let state = HostState::new(wasi, logs, "demo-module".to_string(), header.clone());

    assert_eq!(state.module(), "demo-module");
    assert_eq!(state.header(), &header);
}

#[test]
fn host_state_logs_drains_buffer() {
    let wasi = wasmtime_wasi::WasiCtxBuilder::new().build_p1();
    let logs = Arc::new(Mutex::new(vec!["line one".to_string(), "line two".to_string()]));
    let state = HostState::new(wasi, logs, "demo-module".to_string(), json!({}));

    assert_eq!(state.logs(), vec!["line one".to_string(), "line two".to_string()]);
    assert!(state.logs().is_empty());
}
