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
host/                  the chatons host binary (wasmtime + — later — ratatui + kitty bridge)
examples/hello/        the smallest chaton (Rust → wasm)
chaton-sdk/            (later) the guest crate you build a chaton against
```

## Roadmap

| | |
|---|---|
| v0.1 | host loads a chaton, host↔guest calls *(done)* |
| v0.2 | crossterm event loop — feed keys to the chaton, render its output *(done)* |
| v0.3 | `kitty @` bridge — a `kitty(args)` host fn; the chaton drives kitty *(done)* |
| v0.4 | images — a `show_image(path)` host fn via the kitty graphics protocol *(done)* |
| v0.5 | stabilize the contract as **WIT** (Component Model) → a `chaton-sdk`, other languages |

## License

[MIT](LICENSE).
