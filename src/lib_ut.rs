use crate::{WasmRuntime, cfg::WasmConfig};
use serde_json::json;
use std::{
    collections::HashMap,
    fs,
    path::{Path, PathBuf},
    process::Command,
    sync::OnceLock,
};
use tempfile::TempDir;

static HEADERPEEK_CARGO_TOML: &str = r#"
[package]
name = "headerpeek"
version = "0.1.0"
edition = "2024"

[workspace]

[dependencies]
serde_json = "1"

[profile.release]
opt-level = "z"
lto = true
codegen-units = 1
panic = "abort"
strip = true
"#;

static HEADERPEEK_MAIN_RS: &str = r##"
use serde_json::{Value, json};
use std::io::{self, Write};

#[link(wasm_import_module = "api")]
unsafe extern "C" {
    #[link_name = "header_has"]
    fn header_has(req_ptr: u32, req_len: u32) -> i32;
    #[link_name = "header_get"]
    fn header_get(req_ptr: u32, req_len: u32, out_ptr: u32, out_cap: u32) -> i32;
    #[link_name = "log"]
    fn host_log(level: i32, msg_ptr: u32, msg_len: u32);
}

fn read_json(pointer: &str) -> Value {
    let mut out = vec![0u8; 64 * 1024];
    let n = unsafe { header_get(pointer.as_ptr() as u32, pointer.len() as u32, out.as_mut_ptr() as u32, out.len() as u32) };
    if n < 0 {
        return Value::Null;
    }

    serde_json::from_slice(&out[..n as usize]).unwrap_or(Value::Null)
}

fn main() {
    let msg = b"header inspected";
    unsafe { host_log(1, msg.as_ptr() as u32, msg.len() as u32) };

    let out = json!({
        "has_args_msg": unsafe { header_has(b"/args/msg".as_ptr() as u32, b"/args/msg".len() as u32) != 0 },
        "has_first_opt": unsafe { header_has(b"/opts/0".as_ptr() as u32, b"/opts/0".len() as u32) != 0 },
        "msg": read_json("/args/msg"),
        "first_opt": read_json("/opts/0"),
        "missing": read_json("/args/missing"),
        "header_has_args": read_json("/args").is_object(),
    });

    print!("{}", out);
    io::stdout().flush().expect("stdout flush");
}
"##;

static PLAINTEXT_CARGO_TOML: &str = r#"
[package]
name = "plaintext"
version = "0.1.0"
edition = "2024"

[workspace]

[profile.release]
opt-level = "z"
lto = true
codegen-units = 1
panic = "abort"
strip = true
"#;

static PLAINTEXT_MAIN_RS: &str = r##"
fn main() {
    print!("hello from guest");
}
"##;

static SILENT_CARGO_TOML: &str = r#"
[package]
name = "silent"
version = "0.1.0"
edition = "2024"

[workspace]

[profile.release]
opt-level = "z"
lto = true
codegen-units = 1
panic = "abort"
strip = true
"#;

static SILENT_MAIN_RS: &str = r##"
fn main() {}
"##;

fn wasm_cache_dir() -> &'static Path {
    static CACHE_DIR: OnceLock<PathBuf> = OnceLock::new();
    CACHE_DIR.get_or_init(|| {
        let dir = std::env::temp_dir().join(format!("wasmruntime-ut-cache-{}", std::process::id()));
        fs::create_dir_all(&dir).unwrap_or_else(|err| panic!("failed to create wasm cache directory {}: {err}", dir.display()));
        dir
    })
}

fn mk_tmp_runtime_root() -> TempDir {
    tempfile::Builder::new().prefix("wasmruntime-ut-").tempdir().unwrap_or_else(|err| panic!("failed to create temporary runtime root: {err}"))
}

fn stage_rust_example(name: &str, files: &[(&str, &str)]) -> TempDir {
    let dir = tempfile::Builder::new()
        .prefix(&format!("wasmruntime-rust-{name}-"))
        .tempdir()
        .unwrap_or_else(|err| panic!("failed to create temporary Rust example directory: {err}"));

    for (rel, body) in files {
        let path = dir.path().join(rel);
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap_or_else(|err| panic!("failed to create {}: {err}", parent.display()));
        }
        fs::write(&path, body).unwrap_or_else(|err| panic!("failed to write {}: {err}", path.display()));
    }

    dir
}

fn build_rust_example(example_dir: &Path, output_name: &str, bin_name: &str) -> PathBuf {
    let out = wasm_cache_dir().join(output_name);
    if out.exists() {
        return out;
    }

    let target_dir = example_dir.join("target");
    let status = Command::new("cargo")
        .current_dir(example_dir)
        .env("CARGO_TARGET_DIR", &target_dir)
        .arg("build")
        .arg("--release")
        .arg("--target")
        .arg("wasm32-wasip1")
        .status()
        .unwrap_or_else(|err| panic!("failed to run cargo build in {}: {err}", example_dir.display()));
    if !status.success() {
        panic!("Rust wasm build failed in {} with status {}", example_dir.display(), status);
    }

    let built = target_dir.join("wasm32-wasip1/release").join(format!("{bin_name}.wasm"));
    fs::copy(&built, &out).unwrap_or_else(|err| panic!("failed to cache wasm module {}: {err}", built.display()));
    out
}

fn install_module(root: &Path, src: &Path, dst_name: &str) {
    fs::copy(src, root.join(dst_name)).unwrap_or_else(|err| panic!("failed to copy wasm module {}: {err}", src.display()));
}

#[test]
fn runtime_lists_wasm_objects_sorted() {
    let root = mk_tmp_runtime_root();
    fs::write(root.path().join("zeta.wasm"), b"wasm").unwrap_or_else(|err| panic!("failed to write zeta.wasm: {err}"));
    fs::write(root.path().join("alpha.wasm"), b"wasm").unwrap_or_else(|err| panic!("failed to write alpha.wasm: {err}"));
    fs::write(root.path().join("ignore.txt"), b"txt").unwrap_or_else(|err| panic!("failed to write ignore.txt: {err}"));

    let mut cfg = WasmConfig::default();
    cfg.set_rootdir(root.path());
    let rt = WasmRuntime::new(cfg).expect("runtime should initialize");

    assert_eq!(rt.objects().expect("objects should list"), vec!["alpha".to_string(), "zeta".to_string()]);
}

#[test]
fn runtime_precompiles_modules() {
    let root = mk_tmp_runtime_root();
    let src = stage_rust_example("plaintext", &[("Cargo.toml", PLAINTEXT_CARGO_TOML), ("src/main.rs", PLAINTEXT_MAIN_RS)]);
    let wasm = build_rust_example(src.path(), "plaintext.wasm", "plaintext");
    install_module(root.path(), &wasm, "plaintext.wasm");

    let mut cfg = WasmConfig::default();
    cfg.set_rootdir(root.path());
    let rt = WasmRuntime::new(cfg).expect("runtime should initialize");
    rt.precompile_module("plaintext").expect("precompile should succeed");

    assert!(root.path().join("plaintext.cwasm").exists());
}

#[tokio::test]
async fn runtime_run_exposes_header_helpers_and_logs() {
    let root = mk_tmp_runtime_root();
    let src = stage_rust_example("headerpeek", &[("Cargo.toml", HEADERPEEK_CARGO_TOML), ("src/main.rs", HEADERPEEK_MAIN_RS)]);
    let wasm = build_rust_example(src.path(), "headerpeek.wasm", "headerpeek");
    install_module(root.path(), &wasm, "headerpeek.wasm");

    let mut cfg = WasmConfig::default();
    cfg.set_rootdir(root.path());
    let rt = WasmRuntime::new(cfg).expect("runtime should initialize");

    let args: HashMap<_, _> = [("msg".to_string(), json!("hello"))].into_iter().collect();
    let out = rt.run("headerpeek", vec!["fast".to_string()], args, Vec::new()).await.expect("module should run");

    let logs = out.get("__module-logs").and_then(|v| v.as_array()).cloned().unwrap_or_default();
    assert_eq!(logs.len(), 1);
    assert_eq!(
        out,
        json!({
            "has_args_msg": true,
            "has_first_opt": true,
            "msg": "hello",
            "first_opt": "fast",
            "missing": null,
            "header_has_args": true,
            "__module-logs": logs
        })
    );
    assert!(logs[0].as_str().unwrap_or_default().contains("header inspected"));
}

#[tokio::test]
async fn runtime_wraps_plaintext_stdout() {
    let root = mk_tmp_runtime_root();
    let src = stage_rust_example("plaintext", &[("Cargo.toml", PLAINTEXT_CARGO_TOML), ("src/main.rs", PLAINTEXT_MAIN_RS)]);
    let wasm = build_rust_example(src.path(), "plaintext.wasm", "plaintext");
    install_module(root.path(), &wasm, "plaintext.wasm");

    let mut cfg = WasmConfig::default();
    cfg.set_rootdir(root.path());
    let rt = WasmRuntime::new(cfg).expect("runtime should initialize");

    let out = rt.run_with_header("plaintext", json!({"demo":true}), Vec::new()).await.expect("module should run");
    assert_eq!(out, json!({ "data": "hello from guest", "__module-logs": [] }));
}

#[tokio::test]
async fn runtime_wraps_empty_stdout_as_null_data() {
    let root = mk_tmp_runtime_root();
    let src = stage_rust_example("silent", &[("Cargo.toml", SILENT_CARGO_TOML), ("src/main.rs", SILENT_MAIN_RS)]);
    let wasm = build_rust_example(src.path(), "silent.wasm", "silent");
    install_module(root.path(), &wasm, "silent.wasm");

    let mut cfg = WasmConfig::default();
    cfg.set_rootdir(root.path());
    let rt = WasmRuntime::new(cfg).expect("runtime should initialize");

    let out = rt.run_with_header("silent", json!({}), Vec::new()).await.expect("module should run");
    assert_eq!(out, json!({ "data": null, "__module-logs": [] }));
}
