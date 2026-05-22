//! Tor hidden service — exposes API and P2P ports as a .onion address.
//!
//! Connects to the local Tor control port (default 9051) and calls ADD_ONION
//! to create a v3 hidden service.  The service lives as long as the control
//! connection is open, so it disappears automatically when the node stops.
//!
//! The ed25519 private key returned by Tor is stored in chain state
//! (`tor:private_key`) so the same .onion address is reused across restarts.
//!
//! Env vars:
//!   BTCPC_TOR                 — "true" to activate
//!   BTCPC_TOR_CONTROL_PORT    — Tor control port (default 9051)
//!   BTCPC_TOR_CONTROL_PASSWORD — password for HASHEDPASSWORD auth (optional)

use std::sync::Arc;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::TcpStream;
use tracing::{info, warn};

use crate::chain::Chain;

/// Start a Tor hidden service.  Returns the `.onion` address, or an empty
/// string if Tor is disabled or the control connection fails.
pub async fn start_hidden_service(chain: &Arc<Chain>, api_port: u16, p2p_port: u16) -> String {
    if std::env::var("BTCPC_TOR").as_deref() != Ok("true") {
        return String::new();
    }

    let control_port: u16 = std::env::var("BTCPC_TOR_CONTROL_PORT")
        .ok().and_then(|s| s.parse().ok()).unwrap_or(9051);

    match try_start(chain, api_port, p2p_port, control_port).await {
        Some(addr) => addr,
        None => {
            warn!("tor: hidden service not started (is tor running with ControlPort {}?)", control_port);
            String::new()
        }
    }
}

async fn try_start(
    chain: &Arc<Chain>,
    api_port: u16,
    p2p_port: u16,
    control_port: u16,
) -> Option<String> {
    let stream = TcpStream::connect(format!("127.0.0.1:{}", control_port))
        .await
        .map_err(|e| warn!("tor: cannot reach control port {}: {}", control_port, e))
        .ok()?;

    let (reader, mut writer) = stream.into_split();
    let mut lines = BufReader::new(reader).lines();

    // Authenticate
    let password = std::env::var("BTCPC_TOR_CONTROL_PASSWORD").unwrap_or_default();
    let auth_cmd = if password.is_empty() {
        "AUTHENTICATE \"\"\r\n".to_owned()
    } else {
        format!("AUTHENTICATE \"{}\"\r\n", password)
    };
    writer.write_all(auth_cmd.as_bytes()).await.ok()?;

    match lines.next_line().await {
        Ok(Some(resp)) if resp.starts_with("250") => {}
        Ok(Some(resp)) => {
            warn!("tor: authentication failed: {}", resp);
            return None;
        }
        _ => {
            warn!("tor: no response to AUTHENTICATE");
            return None;
        }
    }

    // Reuse the stored private key so we get the same address after restart.
    let stored_key = chain.store.state_get("tor:private_key")
        .and_then(|b| String::from_utf8(b).ok())
        .unwrap_or_default();

    let add_onion_cmd = if stored_key.is_empty() {
        format!(
            "ADD_ONION NEW:ED25519-V3 Port={api},{api} Port={p2p},{p2p}\r\n",
            api = api_port, p2p = p2p_port
        )
    } else {
        format!(
            "ADD_ONION {key} Port={api},{api} Port={p2p},{p2p}\r\n",
            key = stored_key, api = api_port, p2p = p2p_port
        )
    };
    writer.write_all(add_onion_cmd.as_bytes()).await.ok()?;

    let mut service_id = String::new();
    let mut private_key = String::new();

    loop {
        let line = match lines.next_line().await {
            Ok(Some(l)) => l,
            _ => {
                warn!("tor: connection closed during ADD_ONION");
                return None;
            }
        };

        if let Some(id) = line.strip_prefix("250-ServiceID=") {
            service_id = id.to_owned();
        } else if let Some(key) = line.strip_prefix("250-PrivateKey=") {
            private_key = key.to_owned();
        }

        if line.starts_with("250 OK") {
            break;
        }
        if !line.starts_with("250") {
            warn!("tor: ADD_ONION failed: {}", line);
            // If stored key is corrupt, clear it so next restart generates fresh
            if !stored_key.is_empty() {
                let _ = chain.store.state_set("tor:private_key", b"");
                warn!("tor: cleared stored key — a new address will be generated on next start");
            }
            return None;
        }
    }

    if service_id.is_empty() {
        warn!("tor: ADD_ONION returned no ServiceID");
        return None;
    }

    // Persist new key for address reuse across restarts
    if !private_key.is_empty() && stored_key.is_empty() {
        let _ = chain.store.state_set("tor:private_key", private_key.as_bytes());
    }

    let onion = format!("{}.onion", service_id);
    info!("tor: hidden service active at {} (api:{} p2p:{})", onion, api_port, p2p_port);

    // Hold the control connection open — the service lives as long as this task does.
    tokio::spawn(async move {
        let _writer = writer;
        while let Ok(Some(_)) = lines.next_line().await {}
        warn!("tor: control connection closed — hidden service ended");
    });

    Some(onion)
}
