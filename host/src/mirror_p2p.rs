//! P2P transport via **iroh** — dial your terminal from anywhere (QUIC + NAT traversal + relay),
//! no port-forwarding or public IP.
//!
//!   `chatons mirror … --p2p`        runs an iroh node for the mirrored window; prints + writes a
//!                                   ticket (the full EndpointAddr — id + relay + direct addrs).
//!   `chatons mirror-open <ticket>`  dials it and renders the remote terminal locally — raw ANSI
//!                                   straight to a real terminal, so fidelity is perfect (no xterm).
//!
//! Same wire protocol as the other transports: frames server→client and input client→server on
//! QUIC uni streams, length-prefixed (u32-BE len + payload). Discovery-by-id is left out on
//! purpose (n0 DNS discovery was unreliable here) — the ticket carries the relay, which is what
//! makes it work across the internet.

use crate::mirror::{FrameSource, send_input};
use anyhow::{Context, Result};
use base64::{Engine as _, engine::general_purpose::STANDARD};
use crossterm::terminal::{
    EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode,
};
use crossterm::{cursor, execute};
use iroh::endpoint::presets;
use iroh::{Endpoint, EndpointAddr, Watcher};
use std::io::{Read, Write};
use std::time::Duration;

const ALPN: &[u8] = b"chatons/mirror/0";

fn ticket_path() -> std::path::PathBuf {
    crate::chatons_home().join("mirror.ticket")
}

/// Run the iroh node for `matchspec` (own tokio runtime + thread). Prints + writes a ticket.
pub(crate) fn serve(matchspec: String) {
    let rt = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("  P2P: tokio runtime failed: {e}");
            return;
        }
    };
    rt.block_on(async move {
        let endpoint = match Endpoint::builder(presets::N0).alpns(vec![ALPN.to_vec()]).bind().await {
            Ok(e) => e,
            Err(e) => {
                eprintln!("  P2P: iroh bind failed: {e}");
                return;
            }
        };
        endpoint.online().await; // wait for a home relay so the addr is reachable
        let addr = {
            let mut w = endpoint.watch_addr();
            w.get()
        };
        let ticket = STANDARD.encode(serde_json::to_vec(&addr).unwrap_or_default());
        let _ = std::fs::write(ticket_path(), &ticket);
        println!("  P2P ready — from another machine: chatons mirror-open <ticket>\n  ticket: {ticket}");
        loop {
            let Some(incoming) = endpoint.accept().await else { break };
            let ms = matchspec.clone();
            tokio::spawn(async move {
                let _ = session(incoming, ms).await;
            });
        }
    });
}

async fn session(incoming: iroh::endpoint::Incoming, matchspec: String) -> Result<()> {
    let conn = incoming.await?;

    // input: client opens a uni stream; read length-prefixed chunks → kitty
    {
        let conn = conn.clone();
        let ms = matchspec.clone();
        tokio::spawn(async move {
            let Ok(mut recv) = conn.accept_uni().await else { return };
            loop {
                let mut lenb = [0u8; 4];
                if recv.read_exact(&mut lenb).await.is_err() {
                    break;
                }
                let n = u32::from_be_bytes(lenb) as usize;
                if n == 0 || n > (1 << 20) {
                    break;
                }
                let mut buf = vec![0u8; n];
                if recv.read_exact(&mut buf).await.is_err() {
                    break;
                }
                let ms = ms.clone();
                let _ = tokio::task::spawn_blocking(move || send_input(&ms, &buf)).await;
            }
        });
    }

    // frames: open a uni stream; poll FrameSource (blocking) → length-prefixed payloads
    let mut send = conn.open_uni().await?;
    let mut src = FrameSource::new();
    loop {
        let ms = matchspec.clone();
        let (payload, delay, src_back) = tokio::task::spawn_blocking(move || {
            let payload = src.poll(&ms);
            let delay = src.delay_ms();
            (payload, delay, src)
        })
        .await?;
        src = src_back;
        if let Some(p) = payload {
            send.write_all(&(p.len() as u32).to_be_bytes()).await?;
            send.write_all(&p).await?;
        }
        tokio::time::sleep(Duration::from_millis(delay)).await;
    }
}

/// Client: dial a ticket and render the remote terminal here (raw ANSI → this terminal).
pub fn open(ticket: Option<&String>) -> Result<()> {
    let ticket = ticket.context("usage: chatons mirror-open <ticket>")?;
    let bytes = STANDARD.decode(ticket.trim()).context("invalid ticket (base64)")?;
    let addr: EndpointAddr = serde_json::from_slice(&bytes).context("invalid ticket (addr)")?;
    let rt = tokio::runtime::Builder::new_multi_thread().enable_all().build()?;
    rt.block_on(async move {
        let endpoint = Endpoint::builder(presets::N0)
            .bind()
            .await
            .map_err(|e| anyhow::anyhow!("iroh bind: {e}"))?;
        eprintln!("connecting (Ctrl-] to disconnect)…");
        let conn = endpoint
            .connect(addr, ALPN)
            .await
            .map_err(|e| anyhow::anyhow!("connect: {e}"))?;
        enable_raw_mode()?;
        let _ = execute!(std::io::stdout(), EnterAlternateScreen, cursor::Hide);
        let res = client_loop(conn).await;
        let _ = execute!(std::io::stdout(), cursor::Show, LeaveAlternateScreen);
        let _ = disable_raw_mode();
        res
    })
}

async fn client_loop(conn: iroh::endpoint::Connection) -> Result<()> {
    let mut input = conn.open_uni().await?;

    // frames: server's uni stream → raw bytes to our stdout (it's a real terminal → perfect render)
    let frame_conn = conn.clone();
    let frames = tokio::spawn(async move {
        let mut recv = frame_conn.accept_uni().await?;
        let mut out = std::io::stdout();
        loop {
            let mut lenb = [0u8; 4];
            if recv.read_exact(&mut lenb).await.is_err() {
                break;
            }
            let n = u32::from_be_bytes(lenb) as usize;
            let mut buf = vec![0u8; n];
            if recv.read_exact(&mut buf).await.is_err() {
                break;
            }
            out.write_all(&buf)?;
            out.flush()?;
        }
        anyhow::Ok::<()>(())
    });

    // local stdin → input stream (raw bytes, forwarded as-is); Ctrl-] (0x1d) disconnects
    let (tx, mut rx) = tokio::sync::mpsc::channel::<Vec<u8>>(16);
    std::thread::spawn(move || {
        let mut stdin = std::io::stdin();
        let mut b = [0u8; 256];
        loop {
            match stdin.read(&mut b) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if b[..n].contains(&0x1d) {
                        break;
                    }
                    if tx.blocking_send(b[..n].to_vec()).is_err() {
                        break;
                    }
                }
            }
        }
    });
    while let Some(chunk) = rx.recv().await {
        input.write_all(&(chunk.len() as u32).to_be_bytes()).await?;
        input.write_all(&chunk).await?;
    }
    frames.abort();
    Ok(())
}
