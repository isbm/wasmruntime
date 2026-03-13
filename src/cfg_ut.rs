use crate::cfg::WasmConfig;
use std::path::PathBuf;

#[test]
fn wasm_config_defaults_are_sane() {
    let cfg = WasmConfig::default();

    assert_eq!(cfg.get_guest_path(), "/");
    assert_eq!(cfg.get_wasm_ext(), "wasm");
    assert!(!cfg.get_allow_write());
    assert!(!cfg.get_allow_network());
    assert!(cfg.get_host_path().is_absolute());
    assert!(cfg.get_root_path().is_absolute());
}

#[test]
fn wasm_config_resolves_relative_host_path_against_rootdir() {
    let mut cfg = WasmConfig::default();
    cfg.set_rootdir("/tmp/wasmruntime-root");
    cfg.set_host_path("cache/output").expect("relative host path should resolve against rootdir");

    assert_eq!(cfg.get_root_path(), PathBuf::from("/tmp/wasmruntime-root").as_path());
    assert_eq!(cfg.get_host_path(), PathBuf::from("/tmp/wasmruntime-root/cache/output").as_path());
}

#[test]
fn wasm_config_updates_guest_runtime_flags_and_extension() {
    let mut cfg = WasmConfig::default();
    cfg.set_guest_path("/sandbox");
    cfg.set_wasm_ext("cwasm");
    cfg.set_allow_write(true);
    cfg.set_allow_network(true);

    assert_eq!(cfg.get_guest_path(), "/sandbox");
    assert_eq!(cfg.get_wasm_ext(), "cwasm");
    assert!(cfg.get_allow_write());
    assert!(cfg.get_allow_network());
}
