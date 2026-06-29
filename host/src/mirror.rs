//! `chatons mirror --window <id> [--port <p>] [--bind <addr>]` — serve a live, controllable
//! view of a kitty window in the browser, rendered with **xterm.js**. Default bind is
//! `127.0.0.1` (the trust boundary); other binds warn (no auth/TLS yet — that's the remote TODO).
//!
//!   GET  /           a self-contained page hosting an xterm.js terminal
//!   GET  /xterm.js   vendored xterm.js (MIT) — see vendor/xterm.LICENSE
//!   GET  /xterm.css  vendored xterm.css
//!   GET  /size       the source window's {cols,rows}
//!   GET  /stream     SSE; base64 frames of raw ANSI written verbatim into xterm.js
//!   POST /key        bytes from xterm's onData, replayed into the window
//!
//! Speed: we talk to kitty's remote-control **unix socket directly** (one persistent connection,
//! ~0.5ms/query) instead of spawning `kitty @` per frame/keystroke (~10ms fork+exec). The poll is
//! adaptive (≈30fps while the screen changes, backing off when idle) and frames are **row-diffed**
//! (only changed rows are sent). Still a poll — kitty has no screen-change event over `@`.

use anyhow::{Context, Result};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde_json::json;
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::time::Duration;

// (cert hash as dotted-hex, quic port) — set once at startup if WebTransport comes up, then read
// by page() to bootstrap the browser's serverCertificateHashes. Empty hash ⇒ SSE-only.
static WT_INFO: OnceLock<(String, u16)> = OnceLock::new();

fn pidfile() -> PathBuf {
    crate::chatons_home().join("mirror.pid")
}

/// Kill whatever is listening on TCP `port` (one daemon owns both its TCP and udp/QUIC sockets, so
/// this clears the whole process). Robust against a clobbered/stale pidfile — returns how many.
pub(crate) fn kill_port(port: u16) -> usize {
    let Ok(out) = Command::new("ss").args(["-ltnpH", &format!("sport = :{port}")]).output() else {
        return 0;
    };
    let text = String::from_utf8_lossy(&out.stdout);
    let mut killed = 0;
    for seg in text.split("pid=").skip(1) {
        let pid: String = seg.chars().take_while(|c| c.is_ascii_digit()).collect();
        if !pid.is_empty() {
            let _ = Command::new("kill").arg(&pid).status();
            killed += 1;
        }
    }
    killed
}

fn stop(port: u16) -> Result<()> {
    // best-effort pidfile, then anything still bound to the port (the reliable path)
    if let Ok(pid) = std::fs::read_to_string(pidfile()) {
        let _ = Command::new("kill").arg(pid.trim()).status();
    }
    let _ = std::fs::remove_file(pidfile());
    let n = kill_port(port);
    println!("mirror stopped ({n} listener(s) on :{port})");
    Ok(())
}

pub fn run(args: &[String]) -> Result<()> {
    let mut window: Option<String> = None;
    let mut port: u16 = 9123;
    let mut bind = "127.0.0.1".to_string();
    let mut do_stop = false;
    let mut p2p = false;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--stop" => do_stop = true,
            "--p2p" => p2p = true,
            "--window" | "-w" => {
                window = args.get(i + 1).cloned();
                i += 1;
            }
            "--port" | "-p" => {
                if let Some(p) = args.get(i + 1).and_then(|s| s.parse().ok()) {
                    port = p;
                }
                i += 1;
            }
            "--bind" | "-b" => {
                if let Some(b) = args.get(i + 1) {
                    bind = b.clone();
                }
                i += 1;
            }
            _ => {}
        }
        i += 1;
    }
    if do_stop {
        return stop(port);
    }
    let window = window.context("usage: chatons mirror --window <id> [--port <p>] [--bind <addr>]")?;
    let matchspec = format!("id:{window}");

    // bind, retrying briefly so a just-killed predecessor has time to release the port
    let mut listener = None;
    for _ in 0..15 {
        match TcpListener::bind((bind.as_str(), port)) {
            Ok(l) => {
                listener = Some(l);
                break;
            }
            Err(_) => std::thread::sleep(Duration::from_millis(100)),
        }
    }
    let listener = listener.with_context(|| format!("binding {bind}:{port}"))?;
    if bind != "127.0.0.1" && bind != "localhost" && bind != "::1" {
        eprintln!(
            "WARNING: bound to {bind} with no auth — anyone who can reach this port gets a live \
             shell. localhost-only is the supported mode until auth/TLS lands."
        );
    }
    let _ = std::fs::create_dir_all(crate::chatons_home());
    let _ = std::fs::write(pidfile(), std::process::id().to_string());
    println!("chatons mirror → http://{bind}:{port}/  (window {window})");

    // WebTransport (HTTP/3 over QUIC) fast path on udp/<port+1> — additive; the browser pins the
    // self-signed cert via the hash injected into the page, and falls back to SSE if it can't.
    let quic_port = port.wrapping_add(1);
    match wtransport::Identity::self_signed(["localhost", "127.0.0.1", "::1", bind.as_str()]) {
        Ok(identity) => {
            let hash = identity.certificate_chain().as_slice()[0]
                .hash()
                .fmt(wtransport::tls::Sha256DigestFmt::DottedHex);
            let _ = WT_INFO.set((hash, quic_port));
            let (b, ms) = (bind.clone(), matchspec.clone());
            std::thread::spawn(move || crate::mirror_wt::serve(quic_port, b, identity, ms));
            println!("  WebTransport on udp/{quic_port}");
        }
        Err(e) => eprintln!("  WebTransport disabled (cert: {e}); SSE only"),
    }

    // P2P (iroh) — opt-in via --p2p; dial-from-anywhere over QUIC + NAT traversal + relay
    if p2p {
        let ms = matchspec.clone();
        std::thread::spawn(move || crate::mirror_p2p::serve(ms));
    }

    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let m = matchspec.clone();
        std::thread::spawn(move || {
            let _ = handle(stream, &m);
        });
    }
    Ok(())
}

fn respond(stream: &mut TcpStream, status: &str, ctype: &str, body: &[u8]) -> std::io::Result<()> {
    write!(
        stream,
        "HTTP/1.1 {status}\r\nContent-Type: {ctype}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        body.len()
    )?;
    stream.write_all(body)
}

fn handle(mut stream: TcpStream, matchspec: &str) -> Result<()> {
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut request_line = String::new();
    if reader.read_line(&mut request_line)? == 0 {
        return Ok(());
    }
    let mut parts = request_line.split_whitespace();
    let method = parts.next().unwrap_or("");
    let path = parts.next().unwrap_or("");

    let mut content_length = 0usize;
    loop {
        let mut line = String::new();
        if reader.read_line(&mut line)? == 0 {
            break;
        }
        let t = line.trim_end();
        if t.is_empty() {
            break;
        }
        if t.len() >= 15 && t[..15].eq_ignore_ascii_case("content-length:") {
            content_length = t[15..].trim().parse().unwrap_or(0);
        }
    }

    match (method, path) {
        ("GET", "/") => respond(&mut stream, "200 OK", "text/html; charset=utf-8", page().as_bytes())?,
        ("GET", "/xterm.js") => respond(
            &mut stream,
            "200 OK",
            "application/javascript; charset=utf-8",
            include_str!("vendor/xterm.js").as_bytes(),
        )?,
        ("GET", "/xterm.css") => respond(
            &mut stream,
            "200 OK",
            "text/css; charset=utf-8",
            include_str!("vendor/xterm.css").as_bytes(),
        )?,
        ("GET", "/size") => {
            let (c, r) = window_size(matchspec);
            respond(&mut stream, "200 OK", "application/json", format!("{{\"cols\":{c},\"rows\":{r}}}").as_bytes())?;
        }
        ("GET", "/stream") => stream_loop(&mut stream, matchspec)?,
        ("POST", "/key") => {
            let mut body = vec![0u8; content_length];
            reader.read_exact(&mut body)?;
            send_input(matchspec, &body);
            write!(stream, "HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n")?;
        }
        _ => write!(stream, "HTTP/1.1 404 Not Found\r\nConnection: close\r\n\r\n")?,
    }
    Ok(())
}

/// SSE: poll the screen over a persistent kitty socket, send only changed rows, poll fast while
/// active and back off when idle.
/// Produces successive screen frames from the kitty socket — polls, SGR-normalises, row-diffs, and
/// tracks idle for adaptive pacing. Shared by the SSE (`stream_loop`) and WebTransport transports
/// so "what's on screen" has one implementation and two wire formats.
pub(crate) struct FrameSource {
    conn: Option<KittyConn>,
    prev: Vec<Vec<u8>>,
    first: bool,
    idle: u32,
}

impl FrameSource {
    pub(crate) fn new() -> Self {
        Self { conn: None, prev: Vec::new(), first: true, idle: 0 }
    }

    /// Poll once: the bytes to write to the client terminal if the screen changed (a full repaint
    /// on the first call / after a reconnect, a row-diff otherwise), else `None`.
    pub(crate) fn poll(&mut self, matchspec: &str) -> Option<Vec<u8>> {
        let body = sgr_to_legacy(&get_screen(&mut self.conn, matchspec));
        if body.is_empty() {
            return None;
        }
        let cur: Vec<Vec<u8>> = body.split(|&b| b == b'\n').map(<[u8]>::to_vec).collect();
        if self.first || cur != self.prev {
            let payload = frame_diff(&self.prev, &cur, self.first);
            self.prev = cur;
            self.first = false;
            self.idle = 0;
            Some(payload)
        } else {
            self.idle = self.idle.saturating_add(1);
            None
        }
    }

    /// Adaptive poll delay: ~30fps while active, easing to ~5fps when idle.
    pub(crate) fn delay_ms(&self) -> u64 {
        if self.idle < 15 { 33 } else { 200 }
    }
}

fn stream_loop(stream: &mut TcpStream, matchspec: &str) -> Result<()> {
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n"
    )?;
    let mut src = FrameSource::new();
    loop {
        match src.poll(matchspec) {
            Some(payload) => {
                write!(stream, "data: {}\r\n\r\n", STANDARD.encode(&payload))?;
                stream.flush()?;
            }
            None => {
                stream.write_all(b": ping\r\n\r\n")?;
                stream.flush()?;
            }
        }
        std::thread::sleep(Duration::from_millis(src.delay_ms()));
    }
}

/// One xterm write: on the first frame, disable wrap + clear + paint every row; after that, only
/// the rows that changed (each cleared first), plus blanking any rows that vanished on a resize.
/// Absolute `CSI row;1H` positioning means a width disagreement can't cascade; the inline cursor
/// escape (from `--add-cursor`) travels with the last content chunk, so it repositions for free.
fn frame_diff(prev: &[Vec<u8>], cur: &[Vec<u8>], first: bool) -> Vec<u8> {
    let mut out: Vec<u8> = Vec::new();
    if first {
        out.extend_from_slice(b"\x1b[?7l\x1b[m\x1b[2J"); // wrap off, reset, clear
    }
    for (i, row) in cur.iter().enumerate() {
        if first || prev.get(i) != Some(row) {
            out.extend_from_slice(format!("\x1b[{};1H\x1b[m\x1b[2K", i + 1).as_bytes());
            out.extend_from_slice(row);
        }
    }
    for i in cur.len()..prev.len() {
        out.extend_from_slice(format!("\x1b[{};1H\x1b[m\x1b[2K", i + 1).as_bytes());
    }
    out
}

/// kitty emits truecolor SGR in the colon sub-parameter form `38:2:R:G:B` (no colour-space
/// field). xterm.js follows the ISO form `38:2:<cs>:R:G:B`, so it reads the 5-part form
/// off-by-one — channels shift and blue drops, casting everything green. Normalise SGR colons to
/// the legacy `38;2;R;G;B` semicolon form, which every parser reads correctly. Only rewrites bytes
/// inside CSI `…m` (SGR) sequences — never text content (which can contain colons).
fn sgr_to_legacy(data: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(data.len());
    let mut i = 0;
    while i < data.len() {
        if data[i] == 0x1b && data.get(i + 1) == Some(&b'[') {
            let mut j = i + 2;
            while j < data.len() && !(0x40..=0x7e).contains(&data[j]) {
                j += 1;
            }
            if j >= data.len() {
                out.extend_from_slice(&data[i..]); // unterminated CSI
                break;
            }
            if data[j] == b'm' {
                out.extend_from_slice(b"\x1b[");
                out.extend(data[i + 2..j].iter().map(|&b| if b == b':' { b';' } else { b }));
                out.push(b'm');
            } else {
                out.extend_from_slice(&data[i..=j]); // other CSI verbatim
            }
            i = j + 1;
            continue;
        }
        out.push(data[i]);
        i += 1;
    }
    out
}

/// Get the screen via the persistent socket; (re)connect lazily, fall back to spawning `kitty @`
/// if the socket is unavailable.
fn get_screen(conn: &mut Option<KittyConn>, matchspec: &str) -> Vec<u8> {
    if conn.is_none() {
        *conn = KittyConn::connect();
    }
    if let Some(c) = conn.as_mut() {
        if let Some(b) = c.get_text(matchspec) {
            return b;
        }
        *conn = None; // socket went bad → drop, reconnect next tick
    }
    capture_spawn(matchspec)
}

pub(crate) fn send_input(matchspec: &str, bytes: &[u8]) {
    if let Some(mut c) = KittyConn::connect() {
        if c.send_text(matchspec, bytes).is_some() {
            return;
        }
    }
    send_keys_spawn(matchspec, bytes);
}

// ── kitty remote-control over the unix socket (no fork+exec) ──────────────────────────────────

/// A persistent connection to kitty's remote-control socket (`$KITTY_LISTEN_ON`). Speaks the
/// DCS-framed JSON protocol: `ESC P @kitty-cmd {json} ESC \`.
struct KittyConn {
    sock: UnixStream,
}

impl KittyConn {
    fn connect() -> Option<KittyConn> {
        let path = std::env::var("KITTY_LISTEN_ON").ok()?.strip_prefix("unix:")?.to_string();
        let sock = UnixStream::connect(path).ok()?;
        sock.set_read_timeout(Some(Duration::from_secs(3))).ok()?;
        Some(KittyConn { sock })
    }

    fn cmd(&mut self, name: &str, payload: serde_json::Value) -> Option<serde_json::Value> {
        let msg = json!({"cmd": name, "version": [0, 14, 2], "payload": payload});
        self.sock.write_all(b"\x1bP@kitty-cmd").ok()?;
        self.sock.write_all(msg.to_string().as_bytes()).ok()?;
        self.sock.write_all(b"\x1b\\").ok()?;
        self.sock.flush().ok()?;
        let mut buf = Vec::new();
        let mut chunk = [0u8; 8192];
        loop {
            let n = self.sock.read(&mut chunk).ok()?;
            if n == 0 {
                return None;
            }
            buf.extend_from_slice(&chunk[..n]);
            if buf.ends_with(b"\x1b\\") {
                break;
            }
        }
        let start = buf.windows(10).position(|w| w == b"@kitty-cmd")? + 10;
        let end = buf.len().checked_sub(2)?; // strip trailing ESC \
        (start <= end).then(|| serde_json::from_slice(&buf[start..end]).ok()).flatten()
    }

    fn get_text(&mut self, matchspec: &str) -> Option<Vec<u8>> {
        let r = self.cmd(
            "get-text",
            json!({"match": matchspec, "extent": "screen", "ansi": true, "add_cursor": true}),
        )?;
        if r.get("ok").and_then(serde_json::Value::as_bool) == Some(true) {
            r.get("data").and_then(serde_json::Value::as_str).map(|s| s.as_bytes().to_vec())
        } else {
            None
        }
    }

    fn send_text(&mut self, matchspec: &str, bytes: &[u8]) -> Option<()> {
        // send-text wants its data encoding-tagged; base64 keeps control bytes intact
        let data = format!("base64:{}", STANDARD.encode(bytes));
        let r = self.cmd("send-text", json!({"match": matchspec, "data": data}))?;
        (r.get("ok").and_then(serde_json::Value::as_bool) == Some(true)).then_some(())
    }
}

// ── spawn-based fallbacks (used only if the socket is unavailable) ─────────────────────────────

fn capture_spawn(matchspec: &str) -> Vec<u8> {
    Command::new("kitty")
        .args(["@", "get-text", "--match", matchspec, "--extent", "screen", "--ansi", "--add-cursor"])
        .output()
        .map(|o| o.stdout)
        .unwrap_or_default()
}

fn send_keys_spawn(matchspec: &str, bytes: &[u8]) {
    if let Ok(mut child) = Command::new("kitty")
        .args(["@", "send-text", "--match", matchspec, "--stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
    {
        if let Some(mut stdin) = child.stdin.take() {
            let _ = stdin.write_all(bytes);
        }
        let _ = child.wait();
    }
}

/// The source window's grid size (so xterm matches it). Page-load only, so a spawn is fine.
fn window_size(matchspec: &str) -> (u16, u16) {
    let id = matchspec.strip_prefix("id:").unwrap_or("");
    let out = Command::new("kitty").args(["@", "ls"]).output().map(|o| o.stdout).unwrap_or_default();
    if let Ok(v) = serde_json::from_slice::<serde_json::Value>(&out) {
        for ow in v.as_array().into_iter().flatten() {
            for tab in ow["tabs"].as_array().into_iter().flatten() {
                for w in tab["windows"].as_array().into_iter().flatten() {
                    if w["id"].as_u64().map(|x| x.to_string()).as_deref() == Some(id) {
                        let c = w["columns"].as_u64().unwrap_or(80) as u16;
                        let r = w["lines"].as_u64().unwrap_or(24) as u16;
                        return (c.max(1), r.max(1));
                    }
                }
            }
        }
    }
    (80, 24)
}

/// kitty's full colour set from `get-colors` (background/foreground + the 16 ANSI palette
/// entries + cursor/selection). Page-load only.
fn kitty_colors() -> std::collections::HashMap<String, String> {
    let out = Command::new("kitty").args(["@", "get-colors"]).output().map(|o| o.stdout).unwrap_or_default();
    let mut m = std::collections::HashMap::new();
    for l in String::from_utf8_lossy(&out).lines() {
        let mut it = l.split_whitespace();
        if let (Some(k), Some(v)) = (it.next(), it.next()) {
            if v.starts_with('#') {
                m.insert(k.to_string(), v.to_string());
            }
        }
    }
    m
}

/// xterm.js `theme` object built from kitty's colours, so indexed/ANSI colours (what fzf and a
/// 256-colour nvim use) match your palette instead of xterm's saturated built-in defaults.
/// Indices 16–255 are the standard xterm cube in both, so only 0–15 need mapping.
fn theme_json(c: &std::collections::HashMap<String, String>) -> String {
    let names = [
        ("black", "color0"), ("red", "color1"), ("green", "color2"), ("yellow", "color3"),
        ("blue", "color4"), ("magenta", "color5"), ("cyan", "color6"), ("white", "color7"),
        ("brightBlack", "color8"), ("brightRed", "color9"), ("brightGreen", "color10"),
        ("brightYellow", "color11"), ("brightBlue", "color12"), ("brightMagenta", "color13"),
        ("brightCyan", "color14"), ("brightWhite", "color15"),
    ];
    let mut obj = serde_json::Map::new();
    let mut put = |xkey: &str, val: Option<&String>| {
        if let Some(v) = val {
            obj.insert(xkey.to_string(), json!(v));
        }
    };
    put("background", c.get("background"));
    put("foreground", c.get("foreground"));
    put("cursor", c.get("cursor"));
    put("selectionBackground", c.get("selection_background"));
    for (xname, kname) in names {
        put(xname, c.get(kname));
    }
    serde_json::Value::Object(obj).to_string()
}

/// The terminal font, read from kitty.conf. kitty clamps wide nerd/powerline glyphs to one cell
/// itself; the browser won't, so prefer the font's **Mono** variant (glyphs pre-clamped to a
/// single cell) to keep columns aligned.
fn font_family() -> String {
    let path = std::env::var("HOME")
        .map(|h| PathBuf::from(h).join(".config/kitty/kitty.conf"))
        .unwrap_or_default();
    let configured = std::fs::read_to_string(path).ok().and_then(|c| {
        c.lines().rev().find_map(|l| {
            l.trim()
                .strip_prefix("font_family")
                .map(|r| r.trim().to_string())
                .filter(|s| !s.is_empty() && !s.eq_ignore_ascii_case("auto"))
        })
    });
    match configured {
        Some(f) => {
            let mut fams = Vec::new();
            if f.contains("Nerd Font") && !f.contains("Mono") && !f.contains("Propo") {
                fams.push(format!("'{f} Mono'"));
            }
            fams.push(format!("'{f}'"));
            fams.push("'Symbols Nerd Font Mono'".into());
            fams.push("monospace".into());
            fams.join(", ")
        }
        None => "'Symbols Nerd Font Mono', monospace".to_string(),
    }
}

fn page() -> String {
    let c = kitty_colors();
    let bg = c.get("background").cloned().unwrap_or_else(|| "#1e1e1e".into());
    let (wt_hash, wt_port) = WT_INFO.get().cloned().unwrap_or_default(); // ("", 0) ⇒ SSE only
    PAGE.replace("__FONT__", &font_family())
        .replace("__THEME__", &theme_json(&c))
        .replace("__BG__", &bg)
        .replace("__WT_HASH__", &wt_hash)
        .replace("__WT_PORT__", &wt_port.to_string())
}

// Self-contained page (r##".."## so inner double-quotes are fine). xterm.js does the emulation;
// we pipe frames in and keystrokes out, and scale the source-sized grid to fit the window.
const PAGE: &str = r##"<!doctype html>
<html><head><meta charset=utf-8><title>chatons mirror</title>
<link rel=stylesheet href=/xterm.css>
<style>
 html,body{margin:0;height:100%;background:__BG__;overflow:hidden}
 #wrap{position:absolute;inset:0;display:flex;align-items:center;justify-content:center}
 #term{transform-origin:center center}
 #bar{position:fixed;bottom:0;right:0;font:11px monospace;color:#999;background:#000000aa;padding:3px 7px;z-index:10}
 .xterm,.xterm-viewport{background:__BG__ !important}
</style></head>
<body>
<div id=wrap><div id=term></div></div>
<div id=bar>chatons mirror · live</div>
<script src=/xterm.js></script>
<script>
 const bar=document.getElementById('bar'),host=document.getElementById('term');
 const term=new Terminal({fontFamily:"__FONT__",fontSize:14,cursorBlink:false,scrollback:0,
   theme:__THEME__});
 term.open(host);
 function fit(){const w=host.offsetWidth,h=host.offsetHeight;if(!w||!h)return;
   host.style.transform='scale('+Math.min(innerWidth/w,innerHeight/h)+')';}
 fetch('/size').then(r=>r.json()).then(s=>{term.resize(s.cols,s.rows);fit();}).catch(()=>{});
 // input goes through an indirection so the active transport can swap it
 let sendInput=d=>fetch('/key',{method:'POST',body:d});
 term.onData(d=>sendInput(d));
 function startSSE(){
   const es=new EventSource('/stream');
   es.onmessage=e=>{term.write(Uint8Array.from(atob(e.data),c=>c.charCodeAt(0)));fit();};
   es.onerror=()=>{bar.textContent='chatons mirror · disconnected'};
 }
 async function startWT(){
   const hash=Uint8Array.from("__WT_HASH__".split(':').map(h=>parseInt(h,16)));
   const wt=new WebTransport('https://'+location.hostname+':'+__WT_PORT__+'/mirror',
     {serverCertificateHashes:[{algorithm:'sha-256',value:hash}]});
   await wt.ready;
   bar.textContent='chatons mirror · live · quic';
   const w=(await wt.createUnidirectionalStream()).getWriter();
   sendInput=d=>{const b=new TextEncoder().encode(d);const h=new Uint8Array(4);
     new DataView(h.buffer).setUint32(0,b.length);w.write(h);w.write(b);};
   const reader=wt.incomingUnidirectionalStreams.getReader();
   const fr=(await reader.read()).value.getReader();
   let buf=new Uint8Array(0);
   for(;;){const {value,done}=await fr.read();if(done)break;
     const nb=new Uint8Array(buf.length+value.length);nb.set(buf);nb.set(value,buf.length);buf=nb;
     for(;;){if(buf.length<4)break;
       const n=new DataView(buf.buffer,buf.byteOffset,4).getUint32(0);
       if(buf.length<4+n)break;
       term.write(buf.slice(4,4+n));fit();buf=buf.subarray(4+n);}
   }
 }
 if("__WT_HASH__"){startWT().catch(e=>{console.warn('WebTransport failed → SSE',e);
   bar.textContent='chatons mirror · live · sse';startSSE();});}
 else{startSSE();}
 addEventListener('resize',fit);
</script>
</body></html>"##;
