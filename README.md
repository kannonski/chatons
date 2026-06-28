# chatons

A **WASM plugin host for [kitty](https://sw.kovidgoyal.net/kitty/)** — *kittens, but in any
language, and sandboxed.*

kitty's extensions ("kittens") are Python-only and run with full trust. **chatons** are kitty
plugins compiled to **WebAssembly** — write them in Rust, Go (TinyGo), Zig, JS, whatever — that
the host loads, sandboxes, and lets drive kitty through its remote-control API.

> Status: a working **WASM component-model** host. A chaton implements the `chaton` WIT world
> (`wit/chaton.wit`); the host (wasmtime) loads the component, provides the host interface, and
> runs an interactive loop — it drives kitty (`kitty @`), renders, and shows inline images.

## Why

- **Polyglot** — a chaton is just a wasm module; pick your language.
- **Sandboxed** — a chaton can only do what the host grants (vs kittens' full trust).
- **kitty-native** — the host drives `kitty @` and can emit the kitty graphics protocol, so
  chatons get real windows/layouts and crisp inline images.

## Build & run

```sh
rustup target add wasm32-wasip2   # components build for this target

# build the example chaton (a wasm component)
cargo build --manifest-path examples/hello/Cargo.toml --release --target wasm32-wasip2

# run the host on it
cargo run -p chatons -- examples/hello/target/wasm32-wasip2/release/hello_chaton.wasm
```

> Needs a real terminal (raw mode) + kitty with `allow_remote_control` + `listen_on`.
> `n` → a new kitty tab · `i` → an inline image · `q` → quit. Run from the repo root so the
> demo image path resolves.

## Layout

```
wit/chaton.wit         the plugin contract — the language-neutral source of truth
host/                  the host (wasmtime + wasmtime-wasi + crossterm + kitty bridge)
examples/hello/        the reference chaton — drives kitty + inline image
examples/notepad/      a persistent scratch notepad — loads + saves
examples/qr/           a self-contained app — type text → a live QR code (scan it)
```

Chatons aren't only launchers: `qr` is a little app that just renders into the terminal (it
even works without kitty's graphics protocol — it's unicode blocks).

Run a different chaton by pointing the host at its component `.wasm`:

```sh
cargo build --manifest-path examples/notepad/Cargo.toml --release --target wasm32-wasip2
cargo run -p chatons -- examples/notepad/target/wasm32-wasip2/release/notepad_chaton.wasm
# type · Backspace · Esc saves to /tmp/chaton-notes.txt and quits
```

## Write a chaton

A chaton is a **WASM component** implementing the [`chaton` world](wit/chaton.wit). In Rust,
`wit-bindgen` turns the WIT into a `Guest` trait to implement and `host::*` functions to call:

```rust
wit_bindgen::generate!({ world: "chaton", path: "path/to/wit" });
use chatons::plugin::host;

struct App;
impl Guest for App {
    fn init() { host::render("hello — n: new tab · q: quit"); }
    fn on_key(k: Key) -> bool {        // false = quit
        match k {
            Key::Text('q') | Key::Escape => return false,
            Key::Text('n') => { host::kitty("launch --type=tab"); }
            _ => {}
        }
        true
    }
}
export!(App);
```

Build it for `wasm32-wasip2`. The contract is the WIT — implement it in **any language** with
component-model tooling, not just Rust. (See `examples/` for the full versions with state.)

## Roadmap

| | |
|---|---|
| v0.1 | host loads a chaton, host↔guest calls *(done)* |
| v0.2 | crossterm event loop — feed keys to the chaton, render its output *(done)* |
| v0.3 | `kitty @` bridge — a `kitty(args)` host fn; the chaton drives kitty *(done)* |
| v0.4 | images — a `show_image(path)` host fn via the kitty graphics protocol *(done)* |
| v0.5 | `chaton-sdk` — the `Chaton` trait + `chaton!` macro, write a chaton without FFI *(done)* |
| v0.6 | host→guest data — `read_file` (notepad loads its notes); the read direction *(done)* |
| v0.7 | **WIT / Component Model** — the contract is `wit/chaton.wit`, rich types, chatons in any language *(done)* |
| next | a chaton in a second language (TinyGo / Zig) to exercise the polyglot promise |

## License

[MIT](LICENSE).
