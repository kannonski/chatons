//! chatons — a WASM plugin host for kitty.
//!
//! v0.2: an interactive host. A crossterm raw-mode loop reads keys, feeds each to the chaton's
//! `on_key`, and paints whatever the chaton draws through the `host_render` host function. The
//! chaton owns its state and its view; the host owns the loop, the terminal, and (soon) kitty.
//!
//! The contract so far:
//!   guest exports  init()            paint the first frame (optional)
//!                  on_key(u32)->u32  handle a key; return 0 to quit, else 1
//!   host provides  host_render(ptr,len)      draw a UTF-8 screen
//!                  kitty(ptr,len)->i32       run `kitty @ <args>`, return exit code  [v0.3]
//!                  show_image(ptr,len)->i32  display a PNG inline (kitty graphics)   [v0.4]
//!                  write_file(p,lp,d,ld)->i32 persist data to a file (guest → host)
//!                  read_file(p,lp,buf,cap)->i32 read a file into a guest buffer (host → guest) [v0.6]
//!
//! The guest side of this contract is wrapped by the `chaton-sdk` crate (the `Chaton` trait +
//! `chaton!` macro), so chaton authors never touch FFI. Roadmap: stabilize the contract as WIT
//! (Component Model) for polyglot chatons, and read kitty state back into the guest.

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
use base64::{engine::general_purpose::STANDARD, Engine as _};
use std::io::{stdout, Write};
use std::process::{Command, Stdio};
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
                let _ = write!(out, "\x1b_Ga=d\x1b\\"); // clear any kitty images first
                let _ = queue!(out, Clear(ClearType::All), cursor::MoveTo(0, 0), Print(screen));
                let _ = out.flush();
            }
        },
    )?;

    // show_image(ptr,len) -> i32: read a PNG path, display it inline a few rows down via the
    // kitty graphics protocol (kitty reads + decodes the file itself). Returns 0, or -1 if the
    // path read fails. PNG-only for now (f=100,t=f); arbitrary formats = decode-to-RGBA later.
    linker.func_wrap(
        "chatons",
        "show_image",
        |mut caller: Caller<'_, ()>, ptr: i32, len: i32| -> i32 {
            let memory = match caller.get_export("memory") {
                Some(Extern::Memory(m)) => m,
                _ => return -1,
            };
            let data = memory.data(&caller);
            let (start, len) = (ptr as usize, len as usize);
            let Some(bytes) = data.get(start..start.saturating_add(len)) else {
                return -1;
            };
            // kitty reads the file itself (its cwd ≠ ours), so send an absolute path.
            let given = String::from_utf8_lossy(bytes).into_owned();
            let abs = std::fs::canonicalize(&given)
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or(given);
            let b64 = STANDARD.encode(abs.as_bytes());
            let mut out = stdout();
            let _ = queue!(out, cursor::MoveTo(0, 8));
            // f=100 PNG · a=T transmit+display · t=f path is a file · q=2 suppress responses
            let _ = write!(out, "\x1b_Gf=100,a=T,t=f,q=2;{b64}\x1b\\");
            let _ = out.flush();
            0
        },
    )?;

    // kitty(ptr,len) -> i32: the chaton's hook into kitty. Runs `kitty @ <args>` (args split on
    // whitespace) — open tabs, focus windows, set titles, … Returns the exit code (0 = ok,
    // -1 = couldn't spawn). Child stdio is nulled so it can't corrupt the chaton's screen.
    // Requires chatons to run inside kitty with remote control enabled.
    linker.func_wrap(
        "chatons",
        "kitty",
        |mut caller: Caller<'_, ()>, ptr: i32, len: i32| -> i32 {
            let memory = match caller.get_export("memory") {
                Some(Extern::Memory(m)) => m,
                _ => return -1,
            };
            let data = memory.data(&caller);
            let (start, len) = (ptr as usize, len as usize);
            let Some(bytes) = data.get(start..start.saturating_add(len)) else {
                return -1;
            };
            let cmd = String::from_utf8_lossy(bytes).into_owned();
            let args: Vec<&str> = cmd.split_whitespace().collect();
            match Command::new("kitty")
                .arg("@")
                .args(&args)
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
            {
                Ok(status) => status.code().unwrap_or(-1),
                Err(_) => -1,
            }
        },
    )?;

    // write_file(path, data) -> i32: persist data to a file (guest → host direction). 0 = ok.
    // Unrestricted for now — a per-chaton permission/capability grant is the "sandboxed" part,
    // a later step. The reverse (read a file back into the guest) needs a memory-write protocol.
    linker.func_wrap(
        "chatons",
        "write_file",
        |mut caller: Caller<'_, ()>, ppath: i32, lpath: i32, pdata: i32, ldata: i32| -> i32 {
            let memory = match caller.get_export("memory") {
                Some(Extern::Memory(m)) => m,
                _ => return -1,
            };
            let data = memory.data(&caller);
            let read = |p: i32, l: i32| -> Option<String> {
                let (p, l) = (p as usize, l as usize);
                data.get(p..p.saturating_add(l))
                    .map(|b| String::from_utf8_lossy(b).into_owned())
            };
            let (Some(path), Some(content)) = (read(ppath, lpath), read(pdata, ldata)) else {
                return -1;
            };
            std::fs::write(&path, content.as_bytes()).map(|_| 0).unwrap_or(-1)
        },
    )?;

    // read_file(path, buf, cap) -> i32: the host→guest data direction. Copies up to `cap` bytes
    // of the file into the guest buffer at `buf`, and returns the file's FULL length — so if the
    // buffer was too small the guest grows it and calls again. -1 on error. The chaton-sdk hides
    // this grow-and-retry behind a plain `read_file(path) -> Option<String>`.
    linker.func_wrap(
        "chatons",
        "read_file",
        |mut caller: Caller<'_, ()>, ppath: i32, lpath: i32, pbuf: i32, cap: i32| -> i32 {
            let memory = match caller.get_export("memory") {
                Some(Extern::Memory(m)) => m,
                _ => return -1,
            };
            let path = {
                let data = memory.data(&caller);
                let (p, l) = (ppath as usize, lpath as usize);
                match data.get(p..p.saturating_add(l)) {
                    Some(b) => String::from_utf8_lossy(b).into_owned(),
                    None => return -1,
                }
            };
            let contents = match std::fs::read(&path) {
                Ok(c) => c,
                Err(_) => return -1,
            };
            let n = contents.len().min(cap.max(0) as usize);
            let dst = memory.data_mut(&mut caller);
            let p = pbuf as usize;
            match dst.get_mut(p..p.saturating_add(n)) {
                Some(slot) => slot.copy_from_slice(&contents[..n]),
                None => return -1,
            }
            contents.len() as i32
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
            KeyCode::Backspace => 8,
            _ => 0,
        };
        if on_key.call(&mut *store, code)? == 0 {
            return Ok(());
        }
    }
}
