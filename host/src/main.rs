//! chatons — a WASM plugin host for kitty.
//!
//! v0.2: an interactive host. A crossterm raw-mode loop reads keys, feeds each to the chaton's
//! `on_key`, and paints whatever the chaton draws through the `host_render` host function. The
//! chaton owns its state and its view; the host owns the loop, the terminal, and (soon) kitty.
//!
//! The contract so far:
//!   guest exports  init()            paint the first frame (optional)
//!                  on_key(u32)->u32  handle a key; return 0 to quit, else 1
//!   host provides  host_render(ptr,len)   draw a UTF-8 screen
//!
//! Roadmap: v0.3 `kitty @` bridge (host fns to open layouts / focus windows) · v0.4 images via
//! the kitty graphics protocol · v0.5 stabilize the contract as WIT + a chaton-sdk.

use anyhow::{Context, Result};
use crossterm::{
    cursor,
    event::{self, Event, KeyCode, KeyEventKind},
    execute, queue,
    style::Print,
    terminal::{
        disable_raw_mode, enable_raw_mode, Clear, ClearType, EnterAlternateScreen,
        LeaveAlternateScreen,
    },
};
use std::io::{stdout, Write};
use wasmtime::{Caller, Engine, Extern, Linker, Module, Store, TypedFunc};

fn main() -> Result<()> {
    let wasm_path = std::env::args().nth(1).unwrap_or_else(|| {
        "examples/hello/target/wasm32-unknown-unknown/release/hello_chaton.wasm".to_string()
    });

    let engine = Engine::default();
    let module = Module::from_file(&engine, &wasm_path)
        .with_context(|| format!("loading chaton {wasm_path}"))?;

    let mut store = Store::new(&engine, ());
    let mut linker = Linker::new(&engine);

    // host_render(ptr,len): the chaton draws a full screen. We read it from guest memory and
    // paint it in the alt-screen (raw mode → newlines need an explicit carriage return).
    linker.func_wrap(
        "chatons",
        "host_render",
        |mut caller: Caller<'_, ()>, ptr: i32, len: i32| {
            let memory = match caller.get_export("memory") {
                Some(Extern::Memory(m)) => m,
                _ => return,
            };
            let data = memory.data(&caller);
            let (start, len) = (ptr as usize, len as usize);
            if let Some(bytes) = data.get(start..start.saturating_add(len)) {
                let screen = String::from_utf8_lossy(bytes).replace('\n', "\r\n");
                let mut out = stdout();
                let _ = queue!(out, Clear(ClearType::All), cursor::MoveTo(0, 0), Print(screen));
                let _ = out.flush();
            }
        },
    )?;

    let instance = linker.instantiate(&mut store, &module)?;
    let init = instance.get_typed_func::<(), ()>(&mut store, "init").ok();
    let on_key = instance
        .get_typed_func::<u32, u32>(&mut store, "on_key")
        .context("chaton must export `on_key(u32) -> u32`")?;

    // Enter the TUI; whatever happens, restore the terminal before returning the result.
    enable_raw_mode()?;
    execute!(stdout(), EnterAlternateScreen, cursor::Hide)?;
    let res = event_loop(&mut store, init, on_key);
    execute!(stdout(), cursor::Show, LeaveAlternateScreen)?;
    disable_raw_mode()?;
    res
}

fn event_loop(
    store: &mut Store<()>,
    init: Option<TypedFunc<(), ()>>,
    on_key: TypedFunc<u32, u32>,
) -> Result<()> {
    if let Some(init) = init {
        init.call(&mut *store, ())?;
    }
    loop {
        let Event::Key(key) = event::read()? else {
            continue; // ignore resize/mouse/paste for now
        };
        if key.kind != KeyEventKind::Press {
            continue;
        }
        let code: u32 = match key.code {
            KeyCode::Char(c) => c as u32,
            KeyCode::Esc => 27,
            KeyCode::Enter => 13,
            _ => 0,
        };
        if on_key.call(&mut *store, code)? == 0 {
            return Ok(());
        }
    }
}
