//! chatons — a WASM **component-model** plugin host for kitty.
//!
//! The plugin contract lives in `wit/chaton.wit` (the language-neutral source of truth). A
//! chaton is a WASM *component*: it imports the host interface (render / kitty / show-image /
//! write-file / read-file) and exports `init` + `on-key`. The chaton paints by *calling*
//! `host.render` — the host doesn't pull a render. Rich WIT types cross the boundary natively
//! (note `read-file -> option<string>`: no buffer dance).

use anyhow::{Context, Result};
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEventKind},
    execute, queue,
    style::Print,
    terminal::{
        Clear, ClearType, EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode,
        enable_raw_mode,
    },
};
use std::io::{Write, stdout};
use std::process::{Command, Stdio};
use wasmtime::component::{Component, Linker};
use wasmtime::{Engine, Store};
use wasmtime_wasi::{ResourceTable, WasiCtx, WasiCtxBuilder, WasiView};

wasmtime::component::bindgen!({ world: "chaton", path: "../wit" });

use base64::{Engine as _, engine::general_purpose::STANDARD};

// Components built for wasm32-wasip2 import WASI (std uses it), so the host must provide it.
struct State {
    ctx: WasiCtx,
    table: ResourceTable,
}

impl WasiView for State {
    fn ctx(&mut self) -> &mut WasiCtx {
        &mut self.ctx
    }
    fn table(&mut self) -> &mut ResourceTable {
        &mut self.table
    }
}

// `types` only defines the `key` variant (no functions), but bindgen still wants the impl.
impl chatons::plugin::types::Host for State {}

// The host side of the WIT `host` interface — same behaviour as before, but with native types
// (String args, `Option<String>` return) instead of pointer/length marshalling.
impl chatons::plugin::host::Host for State {
    fn render(&mut self, text: String) {
        let screen = text.replace('\n', "\r\n");
        let mut out = stdout();
        let _ = write!(out, "\x1b_Ga=d\x1b\\"); // clear any kitty images first
        let _ = queue!(out, Clear(ClearType::All), cursor::MoveTo(0, 0), Print(screen));
        let _ = out.flush();
    }

    fn kitty(&mut self, args: String) -> i32 {
        let parts: Vec<&str> = args.split_whitespace().collect();
        match Command::new("kitty")
            .arg("@")
            .args(&parts)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
        {
            Ok(status) => status.code().unwrap_or(-1),
            Err(_) => -1,
        }
    }

    fn show_image(&mut self, path: String) -> i32 {
        // kitty opens the file itself (its cwd ≠ ours), so send an absolute path.
        let abs = std::fs::canonicalize(&path)
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or(path);
        let b64 = STANDARD.encode(abs.as_bytes());
        let mut out = stdout();
        let _ = queue!(out, cursor::MoveTo(0, 8));
        let _ = write!(out, "\x1b_Gf=100,a=T,t=f,q=2;{b64}\x1b\\");
        let _ = out.flush();
        0
    }

    fn write_file(&mut self, path: String, data: String) -> i32 {
        std::fs::write(&path, data.as_bytes()).map(|_| 0).unwrap_or(-1)
    }

    fn read_file(&mut self, path: String) -> Option<String> {
        std::fs::read_to_string(&path).ok()
    }
}

fn main() -> Result<()> {
    let wasm_path = std::env::args().nth(1).unwrap_or_else(|| {
        "examples/hello/target/wasm32-wasip2/release/hello_chaton.wasm".to_string()
    });

    let engine = Engine::default();
    let component = Component::from_file(&engine, &wasm_path)
        .with_context(|| format!("loading chaton {wasm_path}"))?;
    let mut linker = Linker::new(&engine);
    wasmtime_wasi::add_to_linker_sync(&mut linker)?; // WASI, for the component's std
    Chaton::add_to_linker(&mut linker, |s: &mut State| s)?;
    let state = State {
        ctx: WasiCtxBuilder::new().inherit_stderr().build(),
        table: ResourceTable::new(),
    };
    let mut store = Store::new(&engine, state);
    let bindings = Chaton::instantiate(&mut store, &component, &linker)?;

    enable_raw_mode()?;
    execute!(stdout(), EnterAlternateScreen, cursor::Hide)?;
    let res = event_loop(&mut store, &bindings);
    execute!(stdout(), cursor::Show, LeaveAlternateScreen)?;
    disable_raw_mode()?;
    res
}

fn event_loop(store: &mut Store<State>, bindings: &Chaton) -> Result<()> {
    bindings.call_init(&mut *store)?;
    loop {
        let Event::Key(key) = event::read()? else {
            continue;
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        let k = match key.code {
            KeyCode::Char(c) => Key::Text(c),
            KeyCode::Enter => Key::Enter,
            KeyCode::Backspace => Key::Backspace,
            KeyCode::Esc => Key::Escape,
            _ => Key::Other(0),
        };
        if !bindings.call_on_key(&mut *store, k)? {
            return Ok(());
        }
    }
}
