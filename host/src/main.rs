//! chatons — a WASM plugin host for kitty.
//!
//! v0.1 spike: load a `.wasm` chaton, expose one host function (`host_render`) it can call,
//! and invoke its exported `run()`. This proves the core mechanic — host ↔ guest calls and
//! reading guest memory — before we add WASI, a key/event loop, a renderer, and the `kitty @`
//! bridge.
//!
//! Roadmap (each is "add a host function + grow the contract from a real plugin"):
//!   v0.2  crossterm input loop → feed key events to the chaton; render its output
//!   v0.3  kitty bridge: host fns that shell out to `kitty @` (open layout, focus, …)
//!   v0.4  images: a host fn that emits the kitty graphics protocol on the chaton's behalf
//!   v0.5  stabilize the contract as WIT (Component Model) → other languages, a chaton-sdk

use anyhow::{Context, Result};
use wasmtime::{Caller, Engine, Extern, Linker, Module, Store};

fn main() -> Result<()> {
    let wasm_path = std::env::args().nth(1).unwrap_or_else(|| {
        "examples/hello/target/wasm32-unknown-unknown/release/hello_chaton.wasm".to_string()
    });

    let engine = Engine::default();
    let module = Module::from_file(&engine, &wasm_path)
        .with_context(|| format!("loading chaton {wasm_path}"))?;

    let mut store = Store::new(&engine, ());
    let mut linker = Linker::new(&engine);

    // The host API the chaton may import. v0.1 = one function: render a UTF-8 string.
    // It reads the bytes straight out of the guest's linear memory.
    linker.func_wrap(
        "chatons",
        "host_render",
        |mut caller: Caller<'_, ()>, ptr: i32, len: i32| {
            let memory = match caller.get_export("memory") {
                Some(Extern::Memory(m)) => m,
                _ => {
                    eprintln!("[chatons] chaton exports no memory — can't read its string");
                    return;
                }
            };
            let data = memory.data(&caller);
            let (start, len) = (ptr as usize, len as usize);
            match data.get(start..start.saturating_add(len)) {
                Some(bytes) => println!("[chaton] {}", String::from_utf8_lossy(bytes)),
                None => eprintln!("[chatons] host_render: ptr/len out of bounds"),
            }
        },
    )?;

    let instance = linker.instantiate(&mut store, &module)?;
    let run = instance
        .get_typed_func::<(), ()>(&mut store, "run")
        .context("chaton must export `run()`")?;
    run.call(&mut store, ())?;
    Ok(())
}
