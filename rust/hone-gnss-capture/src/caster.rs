use crate::config::Caster;
use base64::Engine;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tokio::sync::mpsc;
use tracing::{info, warn};

pub type FrameSender = mpsc::Sender<Vec<u8>>;

pub fn spawn_caster(caster: Caster) -> FrameSender {
    let (tx, mut rx) = mpsc::channel::<Vec<u8>>(32);
    tokio::spawn(async move {
        loop {
            // Drain stale frames before reconnecting — RTCM3 corrections are time-critical
            while rx.try_recv().is_ok() {}

            match connect(&caster).await {
                Ok(mut stream) => {
                    info!("{}: streaming ✓", caster.name);
                    while let Some(frame) = rx.recv().await {
                        let chunk_hdr = format!("{:x}\r\n", frame.len());
                        let ok = stream.write_all(chunk_hdr.as_bytes()).await.is_ok()
                            && stream.write_all(&frame).await.is_ok()
                            && stream.write_all(b"\r\n").await.is_ok();
                        if !ok {
                            warn!("{}: write error — reconnecting", caster.name);
                            break;
                        }
                    }
                }
                Err(e) => {
                    warn!("{}: {e} — retry in 30s", caster.name);
                    tokio::time::sleep(std::time::Duration::from_secs(30)).await;
                }
            }
        }
    });
    tx
}

async fn connect(c: &Caster) -> anyhow::Result<TcpStream> {
    let mut stream = TcpStream::connect((c.host.as_str(), c.port)).await?;
    let auth = base64::engine::general_purpose::STANDARD
        .encode(format!("{}:{}", c.username, c.password));
    let mp = if c.mountpoint.is_empty() { "/".to_string() } else { format!("/{}", c.mountpoint) };
    let req = format!(
        "POST {mp} HTTP/1.1\r\nHost: {host}\r\nNtrip-Version: Ntrip/2.0\r\n\
         Authorization: Basic {auth}\r\nUser-Agent: HONE-GNSS-Capture/1.0\r\n\
         Content-Type: application/octet-stream\r\nTransfer-Encoding: chunked\r\n\r\n",
        host = c.host
    );
    stream.write_all(req.as_bytes()).await?;

    let mut resp = [0u8; 256];
    let n = stream.read(&mut resp).await?;
    let resp_str = String::from_utf8_lossy(&resp[..n]);
    if !resp_str.contains("200") {
        anyhow::bail!("caster rejected: {}", resp_str.trim());
    }
    Ok(stream)
}
