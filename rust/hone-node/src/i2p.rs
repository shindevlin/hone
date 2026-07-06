//! I2P datagram transport — Phase 6 of the transport cascade.
//!
//! Connects to a local I2P SAM bridge (Simple Anonymous Messaging, port 7656)
//! and creates an anonymous datagram session.  Incoming I2P datagrams are
//! parsed as `{"entry": <LedgerEntry>}` and applied through the canonical
//! `chain.apply_entry()` path; accepted entries are re-broadcast to libp2p.
//!
//! Design rules:
//!  - I2P peers do NOT count toward `peer_count`.  The zero-peers hardline in
//!    api.rs is about libp2p peers only; I2P cannot satisfy it.
//!  - Every received datagram goes through `chain.apply_entry()` — the same
//!    canonical validation path used by every other transport.
//!  - Accepted entries are re-broadcast to libp2p (`NetCmd::Broadcast`) so
//!    they propagate further via the gossipsub mesh.
//!  - Epoch seals are never generated from I2P events.
//!
//! SAM protocol (line-delimited ASCII, bare LF — not CRLF):
//!   1. TCP connect to SAM host:port (default 127.0.0.1:7656)
//!   2. Handshake:  HELLO VERSION MIN=3.0 MAX=3.3\n
//!      Response:   HELLO REPLY RESULT=OK VERSION=3.x
//!   3. Session:    SESSION CREATE STYLE=DATAGRAM ID=hone-i2p DESTINATION=TRANSIENT PORT=<udp>\n
//!      Response:   SESSION STATUS RESULT=OK DESTINATION=<base64>
//!   4. Actual datagrams arrive on the UDP port specified in step 3.
//!      The TCP control connection must stay open — dropping it kills the session.
//!
//! Env vars:
//!   HONE_I2P              — "true" to activate
//!   HONE_I2P_SAM_HOST     — SAM bridge host (default: 127.0.0.1)
//!   HONE_I2P_SAM_PORT     — SAM bridge TCP port (default: 7656)

use std::sync::Arc;

use hone_types::LedgerEntry;
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpStream, UdpSocket};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use crate::chain::Chain;
use crate::net::NetCmd;

// ---------------------------------------------------------------------------
// Public handle (cheap clone — wraps an mpsc::Sender)
// ---------------------------------------------------------------------------

/// Clone-able handle the broadcast pump can use to forward accepted entries
/// into the I2P network.
#[derive(Clone)]
pub struct I2pHandle {
    tx: mpsc::Sender<LedgerEntry>,
}

impl I2pHandle {
    /// Non-blocking: queues `entry` for delivery to configured I2P peers.
    /// Drops the entry silently if the internal channel is full — libp2p
    /// gossipsub is the reliable path; I2P is opportunistic.
    pub fn send_entry(&self, entry: LedgerEntry) {
        let _ = self.tx.try_send(entry);
    }
}

// ---------------------------------------------------------------------------
// Entry point — called from main.rs before AppState construction
// ---------------------------------------------------------------------------

/// Initialise the I2P datagram transport.  Returns `None` if `HONE_I2P` is
/// not set to `"true"` or if the SAM bridge is unreachable.
///
/// On success spawns a receive task and returns a cloneable [`I2pHandle`]
/// that can be used to forward entries toward I2P peers.
pub async fn start_i2p(
    chain:  Arc<Chain>,
    cmd_tx: mpsc::Sender<NetCmd>,
) -> Option<I2pHandle> {
    if std::env::var("HONE_I2P").as_deref() != Ok("true") {
        return None;
    }

    let sam_host = std::env::var("HONE_I2P_SAM_HOST")
        .unwrap_or_else(|_| "127.0.0.1".to_owned());
    let sam_port: u16 = std::env::var("HONE_I2P_SAM_PORT")
        .ok().and_then(|s| s.parse().ok()).unwrap_or(7656);

    match try_start(chain, cmd_tx, sam_host, sam_port).await {
        Some(handle) => Some(handle),
        None => {
            warn!("i2p: transport not started (is I2P running with SAM enabled?)");
            None
        }
    }
}

// ---------------------------------------------------------------------------
// Internal setup
// ---------------------------------------------------------------------------

async fn try_start(
    chain:    Arc<Chain>,
    cmd_tx:   mpsc::Sender<NetCmd>,
    sam_host: String,
    sam_port: u16,
) -> Option<I2pHandle> {
    // ── Bind a local UDP socket for inbound datagrams ────────────────────────
    // SAM delivers datagrams to this port; we tell SAM the port number during
    // SESSION CREATE.  Bind to 0 so the OS picks a free port.
    let udp = match UdpSocket::bind("127.0.0.1:0").await {
        Ok(s) => Arc::new(s),
        Err(e) => {
            warn!("i2p: failed to bind UDP socket: {}", e);
            return None;
        }
    };
    let udp_port = match udp.local_addr() {
        Ok(a) => a.port(),
        Err(e) => {
            warn!("i2p: failed to read local UDP port: {}", e);
            return None;
        }
    };

    // ── TCP control connection to SAM bridge ─────────────────────────────────
    let stream = TcpStream::connect(format!("{}:{}", sam_host, sam_port))
        .await
        .map_err(|e| warn!("i2p: cannot reach SAM bridge at {}:{}: {}", sam_host, sam_port, e))
        .ok()?;

    let (reader, mut writer) = stream.into_split();
    let mut lines = BufReader::new(reader).lines();

    // ── HELLO handshake ──────────────────────────────────────────────────────
    writer.write_all(b"HELLO VERSION MIN=3.0 MAX=3.3\n").await.ok()?;

    let hello_reply = match lines.next_line().await {
        Ok(Some(l)) => l,
        _ => {
            warn!("i2p: no response to HELLO");
            return None;
        }
    };

    if !hello_reply.contains("RESULT=OK") {
        warn!("i2p: HELLO rejected: {}", hello_reply);
        return None;
    }
    debug!("i2p: SAM handshake OK ({})", hello_reply.trim());

    // ── SESSION CREATE (DATAGRAM) ────────────────────────────────────────────
    let session_cmd = format!(
        "SESSION CREATE STYLE=DATAGRAM ID=hone-i2p DESTINATION=TRANSIENT PORT={}\n",
        udp_port
    );
    writer.write_all(session_cmd.as_bytes()).await.ok()?;

    let session_reply = match lines.next_line().await {
        Ok(Some(l)) => l,
        _ => {
            warn!("i2p: no response to SESSION CREATE");
            return None;
        }
    };

    if !session_reply.contains("RESULT=OK") {
        warn!("i2p: SESSION CREATE failed: {}", session_reply);
        return None;
    }

    // Extract the local I2P destination (base64) from the reply.
    // Reply format: SESSION STATUS RESULT=OK DESTINATION=<base64>
    let destination = session_reply
        .split_whitespace()
        .find_map(|tok| tok.strip_prefix("DESTINATION="))
        .unwrap_or("")
        .to_owned();

    if destination.is_empty() {
        warn!("i2p: SESSION CREATE returned no DESTINATION");
        return None;
    }

    info!("i2p: session active — destination: {}…", &destination[..destination.len().min(32)]);

    // Persist the destination so other nodes can discover us via chain state.
    let _ = chain.store.state_set("i2p:destination", destination.as_bytes());

    // ── Hold the control connection open ─────────────────────────────────────
    // The SAM session lives only as long as the TCP control connection is open.
    // Spawn a background task that reads (and discards) any async lines SAM
    // may send, and keeps `writer` alive so the connection stays open.
    tokio::spawn(async move {
        let _writer = writer; // keep alive
        while let Ok(Some(line)) = lines.next_line().await {
            debug!("i2p: control message: {}", line);
        }
        warn!("i2p: SAM control connection closed — I2P session ended");
    });

    // ── Channel for outbound entries ─────────────────────────────────────────
    let (tx, rx) = mpsc::channel::<LedgerEntry>(256);
    let handle = I2pHandle { tx };

    // ── Receive loop (UDP datagrams from SAM) ────────────────────────────────
    {
        let chain_ref  = chain.clone();
        let cmd_tx_ref = cmd_tx.clone();
        let udp_ref    = udp.clone();
        tokio::spawn(async move {
            run_recv_loop(chain_ref, cmd_tx_ref, udp_ref).await;
        });
    }

    // ── Send loop (outbound → I2P peers) ─────────────────────────────────────
    {
        let chain_ref = chain.clone();
        let udp_ref   = udp.clone();
        tokio::spawn(async move {
            run_send_loop(chain_ref, udp_ref, rx).await;
        });
    }

    Some(handle)
}

// ---------------------------------------------------------------------------
// Receive loop — reads inbound I2P datagrams from the local UDP socket
// ---------------------------------------------------------------------------

async fn run_recv_loop(
    chain:  Arc<Chain>,
    cmd_tx: mpsc::Sender<NetCmd>,
    socket: Arc<UdpSocket>,
) {
    let mut buf = vec![0u8; 65_000];
    loop {
        let len = match socket.recv(&mut buf).await {
            Ok(n) => n,
            Err(e) => {
                warn!("i2p: UDP recv error: {}", e);
                tokio::time::sleep(std::time::Duration::from_millis(100)).await;
                continue;
            }
        };

        handle_datagram(&buf[..len], &chain, &cmd_tx).await;
    }
}

async fn handle_datagram(
    data:   &[u8],
    chain:  &Arc<Chain>,
    cmd_tx: &mpsc::Sender<NetCmd>,
) {
    let json: Value = match serde_json::from_slice(data) {
        Ok(v) => v,
        Err(_) => return, // non-JSON, silently drop
    };

    let Some(entry_val) = json.get("entry") else { return; };

    let entry: LedgerEntry = match serde_json::from_value(entry_val.clone()) {
        Ok(e) => e,
        Err(_) => return, // unrecognised entry shape, silently drop
    };

    // Canonical validation path — same as every other transport.
    match chain.apply_entry(&entry) {
        Ok(_) => {
            debug!("i2p: applied entry received via I2P datagram");

            // Re-broadcast to libp2p so the entry propagates to peers not on I2P.
            let envelope = serde_json::json!({ "entry": &entry });
            if let Ok(bytes) = serde_json::to_vec(&envelope) {
                let _ = cmd_tx.send(NetCmd::Broadcast {
                    topic: "hone/entries",
                    data:  bytes,
                }).await;
            }
        }
        Err(_) => {
            // Validation failure (duplicate, bad sig, wrong state).
            // Silently drop — do not re-broadcast.
        }
    }
}

// ---------------------------------------------------------------------------
// Send loop — delivers outbound entries to known I2P peers
// ---------------------------------------------------------------------------

async fn run_send_loop(
    chain:  Arc<Chain>,
    socket: Arc<UdpSocket>,
    mut rx: mpsc::Receiver<LedgerEntry>,
) {
    while let Some(entry) = rx.recv().await {
        // Load peer list from chain state: `i2p:peers` is a newline-separated
        // list of "<i2p_destination>:<udp_port>" records, written externally.
        let peers_raw = chain.store.state_get("i2p:peers")
            .and_then(|b| String::from_utf8(b).ok())
            .unwrap_or_default();

        if peers_raw.trim().is_empty() {
            debug!("i2p: no peers configured — outbound entry dropped");
            continue;
        }

        let payload = match serde_json::to_vec(&serde_json::json!({ "entry": &entry })) {
            Ok(b) => b,
            Err(e) => {
                warn!("i2p: serialize error: {}", e);
                continue;
            }
        };

        for peer in peers_raw.lines() {
            let peer = peer.trim();
            if peer.is_empty() { continue; }
            // Each peer record is a UDP address the SAM bridge routed to us from.
            // Direct UDP back to it via our local SAM-bound socket.
            match socket.send_to(&payload, peer).await {
                Ok(_)  => debug!("i2p: sent entry to peer {}", peer),
                Err(e) => debug!("i2p: send to {} failed: {}", peer, e),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tempfile::TempDir;

    // Serialise env-var access so parallel test threads do not race.
    static ENV_LOCK: parking_lot::Mutex<()> = parking_lot::const_mutex(());

    fn make_chain_and_cmd_tx() -> (Arc<Chain>, tokio::sync::mpsc::Sender<crate::net::NetCmd>, TempDir) {
        let dir = tempfile::Builder::new()
            .prefix("hone_i2p_test_")
            .tempdir()
            .expect("tempdir");
        let store = crate::store::Store::open(dir.path()).expect("store open");
        let chain = Arc::new(Chain::new(store, "i2p-test-node".into(), "hone-testnet".into()));
        let (cmd_tx, _cmd_rx) = tokio::sync::mpsc::channel(64);
        (chain, cmd_tx, dir)
    }

    /// When `HONE_I2P` is not set, `start_i2p` must return `None` immediately
    /// without attempting a SAM bridge connection.
    #[tokio::test]
    async fn i2p_disabled_returns_none_when_env_unset() {
        let _guard = ENV_LOCK.lock();
        std::env::remove_var("HONE_I2P");

        let (chain, cmd_tx, _dir) = make_chain_and_cmd_tx();
        let result = start_i2p(chain, cmd_tx).await;

        assert!(
            result.is_none(),
            "expected None when HONE_I2P is unset"
        );
    }

    /// When `HONE_I2P=true` but the SAM bridge is not listening on the
    /// configured port, `start_i2p` must return `None` without panicking.
    /// The port is chosen dynamically to avoid conflicts with a real I2P router.
    #[tokio::test]
    async fn i2p_enabled_but_sam_unreachable_returns_none() {
        let _guard = ENV_LOCK.lock();

        // Bind then drop — leaves a closed port that will refuse connections.
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0")
            .await
            .expect("bind ephemeral listener");
        let port = listener.local_addr().expect("local_addr").port();
        drop(listener);

        std::env::set_var("HONE_I2P", "true");
        std::env::set_var("HONE_I2P_SAM_HOST", "127.0.0.1");
        std::env::set_var("HONE_I2P_SAM_PORT", port.to_string());

        let (chain, cmd_tx, _dir) = make_chain_and_cmd_tx();
        let result = start_i2p(chain, cmd_tx).await;

        std::env::remove_var("HONE_I2P");
        std::env::remove_var("HONE_I2P_SAM_HOST");
        std::env::remove_var("HONE_I2P_SAM_PORT");

        assert!(
            result.is_none(),
            "expected None when SAM bridge port is unreachable"
        );
    }
}
