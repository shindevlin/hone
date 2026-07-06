//! Self-hosted LoRaWAN transport — Phase 8 of the HONE transport cascade.
//!
//! Implements the Semtech UDP Packet Forwarder protocol so that a Nebra
//! (or any SX1302-based) LoRaWAN gateway running the Semtech packet forwarder
//! (or ChirpStack Gateway Bridge in Semtech mode) can forward received LoRa
//! packets directly into the HONE node, and receive downlink hashes from it.
//!
//! Design rules:
//!  - Delivery layer only. LoRaWAN peers NEVER count toward peer_count.
//!  - Never seal epochs from LoRaWAN data.
//!  - Every received entry goes through chain.apply_entry() — the canonical
//!    validation path. No alternate trust model.
//!  - Accepted entries are re-broadcast to libp2p so they propagate beyond LoRa.
//!  - 32-byte binary payload = entry hash; forward to peers if not found locally.
//!  - JSON payload starting with `{"entry":` = inline LedgerEntry (small entries).
//!
//! Env:
//!   HONE_LORAWAN=true          — enable the module (off by default)
//!   HONE_LORAWAN_PORT          — UDP port to listen on (default 1700)
//!
//! Semtech UDP Packet Forwarder frame format:
//!   Byte 0     : protocol version (0x02)
//!   Bytes 1–2  : random token (uint16 LE) — echoed in ACK
//!   Byte 3     : identifier
//!                  0x00 PUSH_DATA  gateway → server (rxpk)
//!                  0x01 PUSH_ACK  server → gateway
//!                  0x02 PULL_DATA  gateway → server (heartbeat / pull request)
//!                  0x03 PULL_RESP  server → gateway (txpk downlink)
//!                  0x04 PULL_ACK  server → gateway
//!   Bytes 4–11 : gateway MAC (present in PUSH_DATA and PULL_DATA only)
//!   Bytes 12+  : JSON payload (present in PUSH_DATA and PULL_RESP only)

use std::collections::VecDeque;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use base64::engine::general_purpose::STANDARD as B64;
use base64::Engine as _;
use serde_json::Value;
use tokio::net::UdpSocket;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use hone_types::LedgerEntry;

use crate::chain::Chain;
use crate::net::NetCmd;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const DEFAULT_PORT: u16 = 1700;
const MAX_DGRAM: usize = 2048;

// Semtech UDP Packet Forwarder frame identifiers
const PUSH_DATA: u8 = 0x00;
const PUSH_ACK: u8  = 0x01;
const PULL_DATA: u8 = 0x02;
const PULL_RESP: u8 = 0x03;
const PULL_ACK: u8  = 0x04;
const PROTOCOL_VERSION: u8 = 0x02;

// Minimum valid PUSH_DATA frame size: 1 (ver) + 2 (token) + 1 (id) + 8 (mac) = 12
const PUSH_DATA_MIN: usize = 12;
// Minimum valid PULL_DATA frame size: same 12 bytes, no JSON body
const PULL_DATA_MIN: usize = 12;

// ---------------------------------------------------------------------------
// Public handle — cheap Clone, exposes queue_hash()
// ---------------------------------------------------------------------------

/// Handle passed to AppState and the epoch-seal hook.
/// Sending a hash queues it for the next downlink window to any connected
/// LoRaWAN gateway. Non-blocking — drops silently when the channel is full.
#[derive(Clone)]
pub struct LoraWanHandle {
    tx: mpsc::Sender<[u8; 32]>,
}

impl LoraWanHandle {
    /// Queue a 32-byte entry hash for LoRa downlink on the next PULL_DATA window.
    /// Non-blocking: if the internal channel is full the hash is silently dropped.
    /// LoRa is best-effort; libp2p gossipsub remains the reliable path.
    pub fn queue_hash(&self, hash: [u8; 32]) {
        let _ = self.tx.try_send(hash);
    }
}

// ---------------------------------------------------------------------------
// Module entry point
// ---------------------------------------------------------------------------

/// Start the LoRaWAN module if `HONE_LORAWAN=true`.
/// Returns `Some(handle)` if the socket bound successfully, `None` otherwise.
pub async fn start_lorawan(
    chain: Arc<Chain>,
    cmd_tx: mpsc::Sender<NetCmd>,
) -> Option<LoraWanHandle> {
    if std::env::var("HONE_LORAWAN").as_deref() != Ok("true") {
        return None;
    }

    let port: u16 = std::env::var("HONE_LORAWAN_PORT")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_PORT);

    let bind_addr = format!("0.0.0.0:{}", port);
    let socket = match UdpSocket::bind(&bind_addr).await {
        Ok(s) => Arc::new(s),
        Err(e) => {
            warn!("lorawan: failed to bind UDP/{}: {} — disabled", port, e);
            return None;
        }
    };

    info!("lorawan: Semtech UDP listener on {} (Phase 8 transport)", bind_addr);

    let (tx, rx) = mpsc::channel::<[u8; 32]>(256);
    let handle = LoraWanHandle { tx };

    tokio::spawn(async move {
        run(socket, chain, cmd_tx, rx).await;
    });

    Some(handle)
}

// ---------------------------------------------------------------------------
// Main run loop
// ---------------------------------------------------------------------------

async fn run(
    socket: Arc<UdpSocket>,
    chain: Arc<Chain>,
    cmd_tx: mpsc::Sender<NetCmd>,
    mut hash_rx: mpsc::Receiver<[u8; 32]>,
) {
    let mut buf = vec![0u8; MAX_DGRAM];

    // Gateway MAC → most recent source addr (for PULL_RESP targeting).
    // Using a simple Vec since we expect at most a handful of gateways.
    let mut gateway_addrs: Vec<([u8; 8], SocketAddr)> = Vec::new();

    // Hashes queued for downlink — drained when a PULL_DATA arrives.
    let mut downlink_queue: VecDeque<[u8; 32]> = VecDeque::new();

    loop {
        tokio::select! {
            // ── Inbound packet from gateway ──────────────────────────────
            result = socket.recv_from(&mut buf) => {
                match result {
                    Ok((len, src)) => {
                        let frame = &buf[..len];
                        handle_frame(
                            frame,
                            src,
                            &socket,
                            &chain,
                            &cmd_tx,
                            &mut gateway_addrs,
                            &mut downlink_queue,
                        ).await;
                    }
                    Err(e) => {
                        warn!("lorawan: recv error: {}", e);
                        tokio::time::sleep(Duration::from_millis(100)).await;
                    }
                }
            }

            // ── Hash queued by the epoch-seal hook ───────────────────────
            Some(hash) = hash_rx.recv() => {
                downlink_queue.push_back(hash);
                // Trim to a sane bound so we never build up unbounded backlog.
                while downlink_queue.len() > 128 {
                    let _ = downlink_queue.pop_front();
                }
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Frame dispatch
// ---------------------------------------------------------------------------

async fn handle_frame(
    frame: &[u8],
    src: SocketAddr,
    socket: &UdpSocket,
    chain: &Arc<Chain>,
    cmd_tx: &mpsc::Sender<NetCmd>,
    gateway_addrs: &mut Vec<([u8; 8], SocketAddr)>,
    downlink_queue: &mut VecDeque<[u8; 32]>,
) {
    // Must have at least: version + token(2) + identifier = 4 bytes.
    if frame.len() < 4 {
        debug!("lorawan: frame too short ({} bytes), dropping", frame.len());
        return;
    }

    if frame[0] != PROTOCOL_VERSION {
        debug!("lorawan: unknown protocol version 0x{:02x}, dropping", frame[0]);
        return;
    }

    let token = [frame[1], frame[2]];
    let identifier = frame[3];

    match identifier {
        PUSH_DATA => {
            if frame.len() < PUSH_DATA_MIN {
                debug!("lorawan: PUSH_DATA too short");
                return;
            }
            let mac = extract_mac(frame);
            record_gateway(gateway_addrs, mac, src);
            track_gateway(chain, mac);

            // Send PUSH_ACK immediately.
            send_ack(socket, src, token, PUSH_ACK).await;

            // JSON payload starts at byte 12.
            if frame.len() > PUSH_DATA_MIN {
                let json_bytes = &frame[12..];
                process_push_data(json_bytes, chain, cmd_tx).await;
            }
        }

        PULL_DATA => {
            if frame.len() < PULL_DATA_MIN {
                debug!("lorawan: PULL_DATA too short");
                return;
            }
            let mac = extract_mac(frame);
            record_gateway(gateway_addrs, mac, src);
            track_gateway(chain, mac);

            // Send PULL_ACK immediately.
            send_ack(socket, src, token, PULL_ACK).await;

            // Drain all queued downlink hashes toward this gateway.
            while let Some(hash) = downlink_queue.pop_front() {
                send_pull_resp(socket, src, &hash).await;
            }
        }

        _ => {
            debug!("lorawan: unexpected frame identifier 0x{:02x} from {}", identifier, src);
        }
    }
}

// ---------------------------------------------------------------------------
// PUSH_DATA processing — extract rxpk entries and apply to chain
// ---------------------------------------------------------------------------

async fn process_push_data(
    json_bytes: &[u8],
    chain: &Arc<Chain>,
    cmd_tx: &mpsc::Sender<NetCmd>,
) {
    let json: Value = match serde_json::from_slice(json_bytes) {
        Ok(v) => v,
        Err(e) => {
            debug!("lorawan: PUSH_DATA JSON parse error: {}", e);
            return;
        }
    };

    let rxpk = match json.get("rxpk").and_then(|v| v.as_array()) {
        Some(arr) => arr,
        None => return, // stat-only frame; no received packets
    };

    for pkt in rxpk {
        let b64_data = match pkt.get("data").and_then(|v| v.as_str()) {
            Some(s) => s,
            None => continue,
        };

        let payload = match B64.decode(b64_data) {
            Ok(bytes) => bytes,
            Err(e) => {
                debug!("lorawan: rxpk base64 decode error: {}", e);
                continue;
            }
        };

        handle_lora_payload(&payload, chain, cmd_tx).await;
    }
}

// ---------------------------------------------------------------------------
// Payload dispatch — 32-byte hash or inline LedgerEntry JSON
// ---------------------------------------------------------------------------

async fn handle_lora_payload(
    payload: &[u8],
    chain: &Arc<Chain>,
    cmd_tx: &mpsc::Sender<NetCmd>,
) {
    if payload.len() == 32 {
        // 32-byte binary = entry hash
        let hex = hex::encode(payload);
        let key = format!("entry_hash:{}", hex);
        match chain.store.state_get(&key) {
            Some(entry_bytes) => {
                match serde_json::from_slice::<LedgerEntry>(&entry_bytes) {
                    Ok(entry) => {
                        apply_and_broadcast(chain, cmd_tx, &entry).await;
                    }
                    Err(e) => {
                        debug!("lorawan: entry_hash:{} found but deserialize failed: {}", hex, e);
                    }
                }
            }
            None => {
                // Not found locally — a libp2p peer will need to relay this.
                info!("lorawan: received hash {} — entry unknown, peer will need to relay", hex);
            }
        }
    } else if payload.starts_with(b"{\"entry\":") {
        // Inline LedgerEntry JSON — small enough to fit in a LoRa frame.
        let json: Value = match serde_json::from_slice(payload) {
            Ok(v) => v,
            Err(e) => {
                debug!("lorawan: inline entry JSON parse error: {}", e);
                return;
            }
        };
        let entry_val = match json.get("entry") {
            Some(v) => v,
            None => return,
        };
        match serde_json::from_value::<LedgerEntry>(entry_val.clone()) {
            Ok(entry) => {
                apply_and_broadcast(chain, cmd_tx, &entry).await;
            }
            Err(e) => {
                debug!("lorawan: inline entry deserialize error: {}", e);
            }
        }
    } else {
        debug!(
            "lorawan: unrecognised payload ({} bytes), dropping",
            payload.len()
        );
    }
}

// ---------------------------------------------------------------------------
// Apply entry and re-broadcast to libp2p
// ---------------------------------------------------------------------------

async fn apply_and_broadcast(
    chain: &Arc<Chain>,
    cmd_tx: &mpsc::Sender<NetCmd>,
    entry: &LedgerEntry,
) {
    match chain.apply_entry(entry) {
        Ok(_) => {
            debug!("lorawan: applied entry via LoRaWAN, re-broadcasting to libp2p");
            let envelope = serde_json::json!({ "entry": entry });
            if let Ok(bytes) = serde_json::to_vec(&envelope) {
                let _ = cmd_tx.send(NetCmd::Broadcast {
                    topic: "hone/entries",
                    data: bytes,
                }).await;
            }
        }
        Err(_) => {
            // Duplicate, bad sig, wrong state — silently drop; do not re-broadcast.
        }
    }
}

// ---------------------------------------------------------------------------
// Send helpers
// ---------------------------------------------------------------------------

/// Send a 4-byte ACK frame (PUSH_ACK or PULL_ACK) back to `dst`.
async fn send_ack(socket: &UdpSocket, dst: SocketAddr, token: [u8; 2], identifier: u8) {
    let frame = [PROTOCOL_VERSION, token[0], token[1], identifier];
    if let Err(e) = socket.send_to(&frame, dst).await {
        debug!("lorawan: send ACK error: {}", e);
    }
}

/// Send a PULL_RESP frame containing `hash` as the LoRa payload to `dst`.
/// Uses `"imme": true` for immediate transmission (no RX window timing math).
async fn send_pull_resp(socket: &UdpSocket, dst: SocketAddr, hash: &[u8; 32]) {
    // Use a static token — the gateway only checks it for PULL_ACK correlation.
    let token: [u8; 2] = [0xAB, 0xCD];

    let payload_b64 = B64.encode(hash);
    let txpk = serde_json::json!({
        "txpk": {
            "imme": true,
            "freq": 869.525,
            "rfch": 0,
            "powe": 14,
            "modu": "LORA",
            "datr": "SF7BW125",
            "codr": "4/5",
            "ipol": true,
            "size": hash.len(),
            "data": payload_b64
        }
    });

    let json_bytes = match serde_json::to_vec(&txpk) {
        Ok(b) => b,
        Err(e) => {
            warn!("lorawan: PULL_RESP serialize error: {}", e);
            return;
        }
    };

    // PULL_RESP: version + token(2) + 0x03 + JSON (no MAC bytes)
    let mut frame = Vec::with_capacity(4 + json_bytes.len());
    frame.extend_from_slice(&[PROTOCOL_VERSION, token[0], token[1], PULL_RESP]);
    frame.extend_from_slice(&json_bytes);

    if let Err(e) = socket.send_to(&frame, dst).await {
        debug!("lorawan: PULL_RESP send error: {}", e);
    }
}

// ---------------------------------------------------------------------------
// Gateway tracking helpers
// ---------------------------------------------------------------------------

/// Extract 8-byte gateway MAC from bytes 4–11 of a PUSH_DATA or PULL_DATA frame.
fn extract_mac(frame: &[u8]) -> [u8; 8] {
    let mut mac = [0u8; 8];
    mac.copy_from_slice(&frame[4..12]);
    mac
}

/// Update the in-memory gateway → src_addr map (replace existing or append new).
fn record_gateway(
    gateway_addrs: &mut Vec<([u8; 8], SocketAddr)>,
    mac: [u8; 8],
    src: SocketAddr,
) {
    if let Some(entry) = gateway_addrs.iter_mut().find(|(m, _)| m == &mac) {
        entry.1 = src;
    } else {
        gateway_addrs.push((mac, src));
    }
}

/// Persist gateway last-seen timestamp to RocksDB state for monitoring.
/// Key: `lorawan:gateways:{hex_mac}` → now_ms as little-endian u64.
fn track_gateway(chain: &Arc<Chain>, mac: [u8; 8]) {
    let hex_mac = hex::encode(mac);
    let now = crate::utils::now_ms();
    let _ = chain.store.state_set(
        &format!("lorawan:gateways:{}", hex_mac),
        &now.to_le_bytes(),
    );
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
            .prefix("hone_lorawan_test_")
            .tempdir()
            .expect("tempdir");
        let store = crate::store::Store::open(dir.path()).expect("store open");
        let chain = Arc::new(Chain::new(store, "lorawan-test-node".into(), "hone-testnet".into()));
        let (cmd_tx, _cmd_rx) = tokio::sync::mpsc::channel(64);
        (chain, cmd_tx, dir)
    }

    /// When `HONE_LORAWAN` is not set, `start_lorawan` must return `None`
    /// immediately without binding any socket.
    #[tokio::test]
    async fn lorawan_disabled_returns_none_when_env_unset() {
        let _guard = ENV_LOCK.lock();
        std::env::remove_var("HONE_LORAWAN");

        let (chain, cmd_tx, _dir) = make_chain_and_cmd_tx();
        let result = start_lorawan(chain, cmd_tx).await;

        assert!(
            result.is_none(),
            "expected None when HONE_LORAWAN is unset"
        );
    }

    /// When `HONE_LORAWAN=true` with a high-numbered port, `start_lorawan` must
    /// return `Some(handle)` and the handle must be Clone.
    #[tokio::test]
    async fn lorawan_enabled_binds_and_returns_handle() {
        let _guard = ENV_LOCK.lock();

        // Pick an ephemeral port by binding to :0 then releasing it.
        // We pass this port to start_lorawan via env var.
        let probe = tokio::net::UdpSocket::bind("0.0.0.0:0")
            .await
            .expect("probe bind");
        let port = probe.local_addr().expect("local_addr").port();
        drop(probe);

        std::env::set_var("HONE_LORAWAN", "true");
        std::env::set_var("HONE_LORAWAN_PORT", port.to_string());

        let (chain, cmd_tx, _dir) = make_chain_and_cmd_tx();
        let result = start_lorawan(chain, cmd_tx).await;

        std::env::remove_var("HONE_LORAWAN");
        std::env::remove_var("HONE_LORAWAN_PORT");

        assert!(
            result.is_some(),
            "expected Some(handle) when HONE_LORAWAN=true and port is bindable"
        );
        // Verify Clone works on the handle.
        let handle = result.unwrap();
        let _handle2 = handle.clone();
    }

    /// The Semtech PUSH_ACK wire format must be exactly four bytes:
    ///   [0x02, token[0], token[1], 0x01]
    ///
    /// Verified by sending a real ACK over loopback UDP and checking the
    /// received bytes match the Semtech spec (Table 3 of the protocol doc).
    #[tokio::test]
    async fn semtech_push_ack_frame_format_is_correct() {
        // Two sockets: server (sends the ACK) and client (receives it).
        let server_sock = UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("bind server socket");
        let client_sock = UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("bind client socket");

        let client_addr = client_sock.local_addr().expect("client addr");

        let token: [u8; 2] = [0xAB, 0xCD];

        // send_ack is private async fn — call it directly via super::*.
        send_ack(&server_sock, client_addr, token, PUSH_ACK).await;

        let mut buf = [0u8; 16];
        let (len, _src) = client_sock.recv_from(&mut buf).await.expect("recv_from");

        assert_eq!(len, 4, "PUSH_ACK frame must be exactly 4 bytes");
        assert_eq!(
            &buf[..4],
            &[PROTOCOL_VERSION, token[0], token[1], PUSH_ACK],
            "PUSH_ACK frame bytes must be [0x02, tok0, tok1, 0x01]"
        );
    }

    /// The Semtech PULL_ACK wire format must be exactly four bytes:
    ///   [0x02, token[0], token[1], 0x04]
    #[tokio::test]
    async fn semtech_pull_ack_frame_format_is_correct() {
        let server_sock = UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("bind server socket");
        let client_sock = UdpSocket::bind("127.0.0.1:0")
            .await
            .expect("bind client socket");

        let client_addr = client_sock.local_addr().expect("client addr");
        let token: [u8; 2] = [0x12, 0x34];

        send_ack(&server_sock, client_addr, token, PULL_ACK).await;

        let mut buf = [0u8; 16];
        let (len, _src) = client_sock.recv_from(&mut buf).await.expect("recv_from");

        assert_eq!(len, 4, "PULL_ACK frame must be exactly 4 bytes");
        assert_eq!(
            &buf[..4],
            &[PROTOCOL_VERSION, token[0], token[1], PULL_ACK],
            "PULL_ACK frame bytes must be [0x02, tok0, tok1, 0x04]"
        );
    }
}
