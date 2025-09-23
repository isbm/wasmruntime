use serde_json::json;
use std::collections::HashMap;
use wasmruntime::{WasmRuntime, cfg::WasmConfig};

#[tokio::main]
pub async fn main() -> anyhow::Result<()> {
    let mut cfg = WasmConfig::default();
    cfg.set_rootdir("./wasm_bins");
    cfg.set_host_path("/tmp/wasm-playground")?;
    cfg.set_allow_write(true);

    let rt = WasmRuntime::new(cfg)?;
    let ids = rt.objects()?;
    if ids.is_empty() {
        println!("no .wasm files found in ./wasm_bins — put one there, e.g. wasm_bins/echo.wasm");
        return Ok(());
    }
    println!("found: {ids:?}");

    // demo inputs
    let opts = vec!["--demo".into(), "fast".into()];
    let args: HashMap<_, _> = [("msg".to_string(), json!("hi from host")), ("n".to_string(), json!(3))].into_iter().collect();

    // run the first id
    for id in &ids {
        println!("running {id}...");
        let out = rt.run(id, opts.clone(), args.clone()).await?;
        println!("{} -> {}", id, out);
    }

    Ok(())
}
