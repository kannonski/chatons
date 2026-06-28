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
use std::path::{Path, PathBuf};
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

    // Live currency rates (USD base). The host has network; the sandboxed chaton doesn't.
    fn exchange_rates(&mut self) -> Vec<chatons::plugin::types::Rate> {
        fetch_rates()
            .into_iter()
            .map(|(code, per_usd)| chatons::plugin::types::Rate { code, per_usd })
            .collect()
    }
}

/// Fetch USD-base exchange rates (units of <code> per 1 USD), cached to ~/.config/chatons/
/// rates.json with a 12h TTL. Falls back to the cache (even stale) when offline.
fn fetch_rates() -> Vec<(String, f64)> {
    let cache = chatons_home().join("rates.json");
    let fresh = std::fs::metadata(&cache)
        .and_then(|m| m.modified())
        .ok()
        .and_then(|t| t.elapsed().ok())
        .is_some_and(|age| age < std::time::Duration::from_secs(12 * 3600));

    let json = if fresh {
        std::fs::read_to_string(&cache).ok()
    } else {
        match ureq::get("https://open.er-api.com/v6/latest/USD").call() {
            Ok(resp) => {
                let body = resp.into_string().ok();
                if let Some(b) = &body {
                    let _ = std::fs::create_dir_all(chatons_home());
                    let _ = std::fs::write(&cache, b);
                }
                body.or_else(|| std::fs::read_to_string(&cache).ok())
            }
            Err(_) => std::fs::read_to_string(&cache).ok(), // offline → stale cache if any
        }
    };

    let Some(json) = json else { return vec![] };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&json) else {
        return vec![];
    };
    let mut out = Vec::new();
    if let Some(rates) = v["rates"].as_object() {
        for (code, rate) in rates {
            if let Some(per_usd) = rate.as_f64() {
                out.push((code.clone(), per_usd));
            }
        }
    }
    out
}

fn cmd_rates() -> Result<()> {
    let rates = fetch_rates();
    println!("{} currencies (USD base)", rates.len());
    for (code, per_usd) in rates.iter().filter(|(c, _)| {
        ["USD", "EUR", "CHF", "GBP", "JPY"].contains(&c.as_str())
    }) {
        println!("  1 USD = {per_usd} {code}");
    }
    Ok(())
}

fn main() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("list") => cmd_list(),
        Some("keys") => cmd_keys(),
        Some("rates") => cmd_rates(),
        Some("run") => {
            let target = args.get(1).context("usage: chatons run <name|path.wasm>")?;
            run_chaton(&resolve(target))
        }
        // a bare arg is a chaton name (in the home) or a .wasm path (dev): `chatons qr`
        Some(target) => run_chaton(&resolve(target)),
        None => {
            eprintln!(
                "chatons — a WASM plugin host for kitty\n\nusage:\n  chatons run <name>   run a chaton from ~/.config/chatons\n  chatons list         list installed chatons\n  chatons keys         print kitty keybindings for enabled chatons"
            );
            std::process::exit(2);
        }
    }
}

/// Where installed chatons + the manifest live: $CHATONS_HOME, else $XDG_CONFIG_HOME/chatons,
/// else ~/.config/chatons.
fn chatons_home() -> PathBuf {
    if let Ok(home) = std::env::var("CHATONS_HOME") {
        return PathBuf::from(home);
    }
    let config = std::env::var("XDG_CONFIG_HOME").map(PathBuf::from).unwrap_or_else(|_| {
        PathBuf::from(std::env::var("HOME").unwrap_or_default()).join(".config")
    });
    config.join("chatons")
}

/// Resolve a chaton name (→ ~/.config/chatons/<name>.wasm) or a direct `.wasm` path.
fn resolve(target: &str) -> PathBuf {
    if target.ends_with(".wasm") || target.contains('/') {
        PathBuf::from(target)
    } else {
        chatons_home().join(format!("{target}.wasm"))
    }
}

struct Entry {
    name: String,
    key: Option<String>,
    enabled: bool,
}

/// Parse chatons.toml: `[name]` sections with optional `key = "..."` and `enabled = false`.
fn manifest() -> Vec<Entry> {
    let data = std::fs::read_to_string(chatons_home().join("chatons.toml")).unwrap_or_default();
    let mut out: Vec<Entry> = Vec::new();
    for line in data.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(name) = line.strip_prefix('[').and_then(|s| s.strip_suffix(']')) {
            out.push(Entry { name: name.trim().to_string(), key: None, enabled: true });
        } else if let Some((k, v)) = line.split_once('=') {
            if let Some(e) = out.last_mut() {
                match k.trim() {
                    "key" => e.key = Some(v.trim().trim_matches('"').to_string()),
                    "enabled" => e.enabled = v.trim() != "false",
                    _ => {}
                }
            }
        }
    }
    out
}

fn cmd_list() -> Result<()> {
    let home = chatons_home();
    println!("chatons in {}", home.display());
    for e in manifest() {
        let present = if home.join(format!("{}.wasm", e.name)).exists() {
            "✓"
        } else {
            "missing!"
        };
        println!(
            "  {}  {:12} {:18} {present}",
            if e.enabled { "on " } else { "off" },
            e.name,
            e.key.as_deref().unwrap_or("(no key)")
        );
    }
    Ok(())
}

fn cmd_keys() -> Result<()> {
    println!("# generated by `chatons keys` — include this from kitty.conf");
    for e in manifest() {
        if let (true, Some(key)) = (e.enabled, &e.key) {
            println!("map {key} launch --type=overlay --cwd=current chatons run {}", e.name);
        }
    }
    Ok(())
}

/// Load a chaton component and run its interactive loop.
fn run_chaton(path: &Path) -> Result<()> {
    let engine = Engine::default();
    let component = Component::from_file(&engine, path)
        .with_context(|| format!("loading chaton {}", path.display()))?;
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
