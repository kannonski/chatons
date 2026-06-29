//! WebTransport (HTTP/3 over QUIC) transport for the mirror — the fast / remote path. Additive:
//! the SSE+POST path in `mirror.rs` stays as the fallback. The browser pins the self-signed cert
//! via `serverCertificateHashes` (the hash is injected into the page by `mirror::page`).
//!
//! Per session: frames go server→client on a uni stream (length-prefixed payloads — the same diff
//! bytes the SSE path sends); input goes client→server on a uni stream. The sync kitty work
//! (`FrameSource` over the kitty socket) runs on a blocking pool so it never stalls the runtime.

use crate::mirror::{FrameSource, send_input};
use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use std::time::Duration;
use wtransport::endpoint::IncomingSession;
use wtransport::{Endpoint, Identity, ServerConfig};

/// Run the WebTransport server (own tokio runtime, own thread). Returns only on fatal error.
pub(crate) fn serve(quic_port: u16, bind: String, identity: Identity, matchspec: String) {
    let rt = match tokio::runtime::Builder::new_multi_thread().enable_all().build() {
        Ok(rt) => rt,
        Err(e) => {
            eprintln!("  WebTransport: tokio runtime failed: {e}");
            return;
        }
    };
    rt.block_on(async move {
        let ip: IpAddr = bind.parse().unwrap_or(IpAddr::V4(Ipv4Addr::LOCALHOST));
        let config = ServerConfig::builder()
            .with_bind_address(SocketAddr::new(ip, quic_port))
            .with_identity(identity)
            .build();
        let endpoint = match Endpoint::server(config) {
            Ok(e) => e,
            Err(e) => {
                eprintln!("  WebTransport bind udp/{quic_port} failed: {e}");
                return;
            }
        };
        loop {
            let incoming = endpoint.accept().await;
            let ms = matchspec.clone();
            tokio::spawn(async move {
                // per-session errors are just disconnects — ignore
                let _ = session(incoming, ms).await;
            });
        }
    });
}

async fn session(incoming: IncomingSession, matchspec: String) -> anyhow::Result<()> {
    let conn = incoming.await?.accept().await?;

    // input: client opens a uni stream; read length-prefixed chunks and replay into the window
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

    // frames: server opens a uni stream; poll the screen (on the blocking pool — FrameSource is
    // sync over the kitty socket) and write length-prefixed payloads
    let mut send = conn.open_uni().await?.await?;
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
