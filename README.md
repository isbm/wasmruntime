# WASM Runtime

A simple wrapper to run WASI-compiled artifacts, communicating JSON.

## Usage

1. Create something, e.g. using TinyGo or Go 1.21+ that can accept JSON and return JSON back (see `examples` directory)
2. Compile to WASM
3. Place it as `hello.wasm` to some path of your choice
4. Then:

```rust
let rt = WasmRuntime::new("/path/to/wasm/files")?;
let out = rt.run("hello", vec![], json!({"name": "John"}))?;
println!("JSON output: {}", out);
```

It is that simple.
