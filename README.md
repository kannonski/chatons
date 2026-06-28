# chatons

A **WASM plugin host for [kitty](https://sw.kovidgoyal.net/kitty/)** — *kittens, but in any
language, and sandboxed.*

kitty's extensions ("kittens") are Python-only and run with full trust. **chatons** are kitty
plugins compiled to **WebAssembly** — write them in Rust, Go (TinyGo), Zig, JS, whatever — that
the host loads, sandboxes, and lets drive kitty through its remote-control API.

> Status: **v0.1 spike.** A Rust host (wasmtime) loads a `.wasm` chaton, exposes a host
> function, and runs it. The kitty bridge, event loop, renderer, and graphics come next.

## Why

- **Polyglot** — a chaton is just a wasm module; pick your language.
- **Sandboxed** — a chaton can only do what the host grants (vs kittens' full trust).
- **kitty-native** — the host drives `kitty @` and can emit the kitty graphics protocol, so
  chatons get real windows/layouts and crisp inline images.

## Build & run (v0.1)

```sh
rustup target add wasm32-unknown-unknown

# build the example chaton (a wasm module)
cargo build --manifest-path examples/hello/Cargo.toml --target wasm32-unknown-unknown --release

# run the host on it (opens an interactive screen — press keys, q or esc to quit)
cargo run -p chatons -- examples/hello/target/wasm32-unknown-unknown/release/hello_chaton.wasm
```

> Needs a real terminal (raw mode), and kitty with `allow_remote_control` + `listen_on`.
> `n` → a new kitty tab · `i` → an inline image · `q` → quit. Run from the repo root so the
> demo image path resolves.

## Layout

```
host/                  the chatons host binary (wasmtime + crossterm + kitty bridge)
chaton-sdk/            the crate you write a chaton against (the Chaton trait + chaton! macro)
examples/hello/        the reference chaton — drives kitty + inline image
examples/notepad/      a persistent scratch notepad — loads + saves (read_file/write_file)
```

Run a different chaton by pointing the host at its `.wasm`:

```sh
cargo build --manifest-path examples/notepad/Cargo.toml --target wasm32-unknown-unknown --release
cargo run -p chatons -- examples/notepad/target/wasm32-unknown-unknown/release/notepad_chaton.wasm
# type · Backspace · Esc saves to /tmp/chaton-notes.txt and quits
```

## Write a chaton

A chaton is a struct that implements `Chaton` — no `unsafe`, no FFI:

```rust
use chaton_sdk::{Chaton, Flow, Key, View, chaton, kitty};

struct Mine { tabs: u32 }

impl Chaton for Mine {
    fn new() -> Self { Mine { tabs: 0 } }
    fn on_key(&mut self, key: Key) -> Flow {
        match key {
            Key::Char('q') | Key::Esc => return Flow::Quit,
            Key::Char('n') => { kitty("launch --type=tab"); self.tabs += 1; }
            _ => {}
        }
        Flow::Continue
    }
    fn render(&self) -> View {
        View::text(format!("tabs: {}\n\nn  new tab · q  quit", self.tabs))
    }
}

chaton!(Mine);
```

Build it for `wasm32-unknown-unknown` and hand the `.wasm` to the host. (Today the SDK is
Rust; once the contract is pinned as WIT, chatons in any language.)

## Roadmap

| | |
|---|---|
| v0.1 | host loads a chaton, host↔guest calls *(done)* |
| v0.2 | crossterm event loop — feed keys to the chaton, render its output *(done)* |
| v0.3 | `kitty @` bridge — a `kitty(args)` host fn; the chaton drives kitty *(done)* |
| v0.4 | images — a `show_image(path)` host fn via the kitty graphics protocol *(done)* |
| v0.5 | `chaton-sdk` — the `Chaton` trait + `chaton!` macro, write a chaton without FFI *(done)* |
| v0.6 | host→guest data — `read_file` (notepad loads its notes); the read direction *(done)* |
| v0.7 | stabilize the contract as **WIT** (Component Model) → chatons in any language |

## License

[MIT](LICENSE).
