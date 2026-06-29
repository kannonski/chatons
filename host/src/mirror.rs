//! `chatons mirror --window <id> [--port <p>]` — serve a live, controllable view of a kitty
//! window in the **local** browser. Localhost only (the trust boundary), no auth, no QR.
//!
//!   GET  /         a self-contained page: a <pre> mirror + JS (SSE in, keydown POST out)
//!   GET  /stream   SSE; each event is the window's screen (`get-text --ansi` → HTML)
//!   POST /key      raw bytes replayed into the window via `kitty @ send-text --stdin`
//!
//! The screen is a *snapshot* poll of `kitty @ get-text`, not a PTY stream, so it's
//! glance-and-control smooth, not 144fps. Good enough to watch + drive a real tab.

use anyhow::{Context, Result};
use std::io::{BufRead, BufReader, Read, Write};
use std::net::{TcpListener, TcpStream};
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::Duration;

const DEFAULT_FG: &str = "#d3d7cf";
const DEFAULT_BG: &str = "#1e1e1e";

fn pidfile() -> PathBuf {
    crate::chatons_home().join("mirror.pid")
}

/// Stop a running mirror daemon (started earlier; pid recorded in mirror.pid).
fn stop() -> Result<()> {
    match std::fs::read_to_string(pidfile()) {
        Ok(pid) => {
            let pid = pid.trim();
            let _ = Command::new("kill").arg(pid).status();
            let _ = std::fs::remove_file(pidfile());
            println!("mirror stopped (pid {pid})");
        }
        Err(_) => println!("no mirror running"),
    }
    Ok(())
}

pub fn run(args: &[String]) -> Result<()> {
    if args.iter().any(|a| a == "--stop") {
        return stop();
    }
    let mut window: Option<String> = None;
    let mut port: u16 = 9123;
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
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
            _ => {}
        }
        i += 1;
    }
    let window = window.context("usage: chatons mirror --window <id> [--port <p>]")?;
    let matchspec = format!("id:{window}");

    let listener = TcpListener::bind(("127.0.0.1", port))
        .with_context(|| format!("binding 127.0.0.1:{port}"))?;
    let _ = std::fs::create_dir_all(crate::chatons_home());
    let _ = std::fs::write(pidfile(), std::process::id().to_string());
    println!("chatons mirror → http://127.0.0.1:{port}/  (window {window})");

    for stream in listener.incoming() {
        let Ok(stream) = stream else { continue };
        let m = matchspec.clone();
        std::thread::spawn(move || {
            let _ = handle(stream, &m);
        });
    }
    Ok(())
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
        if let Some(v) = t.get(..15).filter(|h| h.eq_ignore_ascii_case("content-length:")) {
            let _ = v;
            content_length = t[15..].trim().parse().unwrap_or(0);
        }
    }

    match (method, path) {
        ("GET", "/") => {
            let body = page();
            write!(
                stream,
                "HTTP/1.1 200 OK\r\nContent-Type: text/html; charset=utf-8\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            )?;
            stream.write_all(body.as_bytes())?;
        }
        ("GET", "/stream") => stream_loop(&mut stream, matchspec)?,
        ("POST", "/key") => {
            let mut body = vec![0u8; content_length];
            reader.read_exact(&mut body)?;
            send_keys(matchspec, &body);
            write!(stream, "HTTP/1.1 204 No Content\r\nConnection: close\r\n\r\n")?;
        }
        _ => {
            write!(stream, "HTTP/1.1 404 Not Found\r\nConnection: close\r\n\r\n")?;
        }
    }
    Ok(())
}

/// SSE: push the window's screen whenever it changes. Each terminal row is one `data:` field,
/// so EventSource rejoins them with '\n' client-side (rendered by the <pre>).
fn stream_loop(stream: &mut TcpStream, matchspec: &str) -> Result<()> {
    write!(
        stream,
        "HTTP/1.1 200 OK\r\nContent-Type: text/event-stream\r\nCache-Control: no-cache\r\nConnection: close\r\n\r\n"
    )?;
    let mut last = String::new();
    loop {
        let frame = capture(matchspec);
        if frame != last {
            for row in frame.split('\n') {
                write!(stream, "data: {row}\r\n")?;
            }
            stream.write_all(b"\r\n")?; // blank line ends the SSE event
            stream.flush()?;
            last = frame;
        } else {
            stream.write_all(b": ping\r\n\r\n")?; // keep-alive comment
            stream.flush()?;
        }
        std::thread::sleep(Duration::from_millis(120));
    }
}

fn capture(matchspec: &str) -> String {
    Command::new("kitty")
        .args(["@", "get-text", "--match", matchspec, "--extent", "screen", "--ansi"])
        .output()
        .map(|o| ansi_to_html(&String::from_utf8_lossy(&o.stdout)))
        .unwrap_or_default()
}

fn send_keys(matchspec: &str, bytes: &[u8]) {
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

// ── ANSI (SGR) → HTML ────────────────────────────────────────────────────────────────────

#[derive(Clone, PartialEq, Default)]
struct Style {
    fg: Option<String>,
    bg: Option<String>,
    bold: bool,
    dim: bool,
    italic: bool,
    underline: bool,
    reverse: bool,
}

// Tango-ish 16-colour palette (0–7 normal, 8–15 bright).
const BASIC: [&str; 16] = [
    "#2e3436", "#cc0000", "#4e9a06", "#c4a000", "#3465a4", "#75507b", "#06989a", "#d3d7cf",
    "#555753", "#ef2929", "#8ae234", "#fce94f", "#729fcf", "#ad7fa8", "#34e2e2", "#eeeeec",
];

fn color256(n: u8) -> String {
    if n < 16 {
        return BASIC[n as usize].to_string();
    }
    if n >= 232 {
        let l = 8 + (n as u32 - 232) * 10;
        return format!("#{l:02x}{l:02x}{l:02x}");
    }
    let n = n as u32 - 16;
    let conv = |c: u32| if c == 0 { 0 } else { 55 + 40 * c };
    let (r, g, b) = (conv(n / 36), conv((n / 6) % 6), conv(n % 6));
    format!("#{r:02x}{g:02x}{b:02x}")
}

fn rgb(r: i64, g: i64, b: i64) -> String {
    let c = |v: i64| v.clamp(0, 255);
    format!("#{:02x}{:02x}{:02x}", c(r), c(g), c(b))
}

/// A simple SGR code (attributes + the 16 indexed colours).
fn apply_simple(style: &mut Style, n: i64) {
    match n {
        0 => *style = Style::default(),
        1 => style.bold = true,
        2 => style.dim = true,
        3 => style.italic = true,
        4 => style.underline = true,
        7 => style.reverse = true,
        22 => {
            style.bold = false;
            style.dim = false;
        }
        23 => style.italic = false,
        24 => style.underline = false,
        27 => style.reverse = false,
        30..=37 => style.fg = Some(BASIC[(n - 30) as usize].to_string()),
        90..=97 => style.fg = Some(BASIC[(n - 90 + 8) as usize].to_string()),
        39 => style.fg = None,
        40..=47 => style.bg = Some(BASIC[(n - 40) as usize].to_string()),
        100..=107 => style.bg = Some(BASIC[(n - 100 + 8) as usize].to_string()),
        49 => style.bg = None,
        _ => {}
    }
}

/// Extended colour from colon sub-params (`38:2:r:g:b`, `38:5:n`, possibly with an empty
/// colour-space field). The r/g/b are the trailing three numbers.
fn color_from_subs(subs: &[i64]) -> Option<String> {
    match subs.get(1) {
        Some(2) if subs.len() >= 5 => {
            let n = subs.len();
            Some(rgb(subs[n - 3], subs[n - 2], subs[n - 1]))
        }
        Some(5) => subs.last().map(|&n| color256(n.clamp(0, 255) as u8)),
        _ => None,
    }
}

/// Extended colour from the legacy semicolon form (`38;2;r;g;b`, `38;5;n`); returns how many
/// extra parts it consumed.
fn ext_color_semicolon(parts: &[&str], i: usize) -> (Option<String>, usize) {
    let num = |k: usize| parts.get(i + k).and_then(|s| s.parse::<i64>().ok());
    match num(1) {
        Some(2) => (Some(rgb(num(2).unwrap_or(0), num(3).unwrap_or(0), num(4).unwrap_or(0))), 4),
        Some(5) => (num(2).map(|n| color256(n.clamp(0, 255) as u8)), 2),
        _ => (None, 1),
    }
}

fn apply_sgr(style: &mut Style, params: &str) {
    // SGR params are ';'-separated; a single param may carry ':'-separated sub-params (kitty
    // emits truecolor as `38:2:r:g:b`). Handle both that and the legacy `38;2;r;g;b` form.
    let parts: Vec<&str> = if params.is_empty() { vec!["0"] } else { params.split(';').collect() };
    let mut i = 0;
    while i < parts.len() {
        let part = parts[i];
        if part.contains(':') {
            let subs: Vec<i64> = part.split(':').map(|s| s.parse().unwrap_or(-1)).collect();
            match subs.first().copied().unwrap_or(-1) {
                38 => style.fg = color_from_subs(&subs),
                48 => style.bg = color_from_subs(&subs),
                n => apply_simple(style, n),
            }
        } else {
            match part.parse::<i64>().unwrap_or(0) {
                38 => {
                    let (col, adv) = ext_color_semicolon(&parts, i);
                    style.fg = col;
                    i += adv;
                }
                48 => {
                    let (col, adv) = ext_color_semicolon(&parts, i);
                    style.bg = col;
                    i += adv;
                }
                n => apply_simple(style, n),
            }
        }
        i += 1;
    }
}

fn style_to_css(s: &Style) -> String {
    let (mut fg, mut bg) = (s.fg.clone(), s.bg.clone());
    if s.reverse {
        fg = Some(s.bg.clone().unwrap_or_else(|| DEFAULT_BG.to_string()));
        bg = Some(s.fg.clone().unwrap_or_else(|| DEFAULT_FG.to_string()));
    }
    let mut css = String::new();
    if let Some(c) = fg {
        css.push_str(&format!("color:{c};"));
    }
    if let Some(c) = bg {
        css.push_str(&format!("background:{c};"));
    }
    if s.bold {
        css.push_str("font-weight:bold;");
    }
    if s.dim {
        css.push_str("opacity:.6;");
    }
    if s.italic {
        css.push_str("font-style:italic;");
    }
    if s.underline {
        css.push_str("text-decoration:underline;");
    }
    css
}

fn ansi_to_html(input: &str) -> String {
    let mut out = String::new();
    let mut style = Style::default();
    let mut open: Option<Style> = None;
    let mut it = input.chars().peekable();
    while let Some(c) = it.next() {
        if c == '\x1b' {
            match it.peek() {
                Some('[') => {
                    // CSI: params (0x20–0x3f) then a final byte (0x40–0x7e)
                    it.next();
                    let mut params = String::new();
                    let mut final_byte = ' ';
                    while let Some(&p) = it.peek() {
                        it.next();
                        if ('\x40'..='\x7e').contains(&p) {
                            final_byte = p;
                            break;
                        }
                        params.push(p);
                    }
                    if final_byte == 'm' {
                        apply_sgr(&mut style, &params);
                    }
                }
                Some(']') => {
                    // OSC (e.g. shell-integration 133 markers): skip to BEL or ST (ESC \)
                    it.next();
                    while let Some(c2) = it.next() {
                        if c2 == '\x07' {
                            break;
                        }
                        if c2 == '\x1b' {
                            if it.peek() == Some(&'\\') {
                                it.next();
                            }
                            break;
                        }
                    }
                }
                Some(_) => {
                    it.next(); // other two-byte escape — drop its second byte
                }
                None => {}
            }
            continue;
        }
        if c == '\r' {
            continue;
        }
        let css = style_to_css(&style);
        let want = if css.is_empty() { None } else { Some(style.clone()) };
        if open != want {
            if open.is_some() {
                out.push_str("</span>");
            }
            if !css.is_empty() {
                out.push_str(&format!("<span style=\"{css}\">"));
            }
            open = want;
        }
        match c {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            _ => out.push(c),
        }
    }
    if open.is_some() {
        out.push_str("</span>");
    }
    out
}

/// The terminal's font, read from kitty.conf so nerd-font glyphs render in the browser (which
/// uses the *client* machine's installed fonts — the same box, for localhost). Falls back to a
/// nerd-font list, with `Symbols Nerd Font` so glyphs resolve even over a non-nerd base font.
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
        Some(f) => format!("'{f}','Symbols Nerd Font',monospace"),
        None => "'Symbols Nerd Font','JetBrainsMono Nerd Font',monospace".to_string(),
    }
}

fn page() -> String {
    PAGE.replace("__FONT__", &font_family())
}

// Self-contained page. No double-quotes inside (keeps the raw string simple); SSE in, keys out.
const PAGE: &str = r#"<!doctype html>
<html><head><meta charset=utf-8><title>chatons mirror</title>
<style>
 html,body{margin:0;background:#1e1e1e;color:#d3d7cf;height:100%}
 #screen{font-family:__FONT__;font-size:14px;
   line-height:1.25;white-space:pre;padding:8px;margin:0;tab-size:8}
 #bar{position:fixed;bottom:0;right:0;font:11px monospace;color:#999;background:#000000aa;padding:3px 7px}
</style></head>
<body>
<pre id=screen>connecting…</pre>
<div id=bar>chatons mirror · live</div>
<script>
 const s=document.getElementById('screen'),bar=document.getElementById('bar');
 const es=new EventSource('/stream');
 es.onmessage=e=>{s.innerHTML=e.data};
 es.onerror=()=>{bar.textContent='chatons mirror · disconnected'};
 function seq(e){
   const k=e.key;
   if(e.ctrlKey&&k.length===1){const c=k.toLowerCase().charCodeAt(0);if(c>=97&&c<=122)return String.fromCharCode(c-96);}
   switch(k){
     case 'Enter':return '\r';case 'Backspace':return '\x7f';case 'Tab':return '\t';
     case 'Escape':return '\x1b';case 'ArrowUp':return '\x1b[A';case 'ArrowDown':return '\x1b[B';
     case 'ArrowRight':return '\x1b[C';case 'ArrowLeft':return '\x1b[D';
     case 'Home':return '\x1b[H';case 'End':return '\x1b[F';case 'Delete':return '\x1b[3~';
     case 'PageUp':return '\x1b[5~';case 'PageDown':return '\x1b[6~';
   }
   if(k.length===1)return k;
   return null;
 }
 addEventListener('keydown',e=>{const b=seq(e);if(b===null)return;e.preventDefault();fetch('/key',{method:'POST',body:b});});
</script>
</body></html>"#;
