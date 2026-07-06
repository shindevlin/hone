//! Lightweight UDP multicast gossip — Phase 2 of the transport cascade.
//!
//! Propagates low-latency, small entries (bids) over LAN multicast so peers
//! on the same network segment receive them in <1ms rather than waiting for
//! the libp2p gossipsub mesh to converge.
//!
//! Design rules:
//!  - LAN-only: TTL=1, multicast group 239.1.1.1 never crosses a router.
//!  - Strict entry allowlist: only InferenceJobBid. Not blocks, not seals.
//!  - Every received datagram goes through chain.apply_entry() — the
//!    canonical validation path. No alternate trust model.
//!  - UDP neighbors do NOT count toward peer_count. The zero-peers hardline
//!    in api.rs is about libp2p peers only; UDP cannot satisfy it.
//!  - Accepted entries are re-broadcast to libp2p so they propagate beyond LAN.
//!
//! Env:
//!   HONE_UDP_GOSSIP_PORT  — UDP port (default 6944)
//!   HONE_UDP_GOSSIP_TTL   — multicast TTL (default 1 = LAN only)

use std::net::{Ipv4Addr, SocketAddr, SocketAddrV4};
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use tokio::net::UdpSocket;
use tokio::sync::mpsc;
use tracing::{debug, warn};

use hone_types::LedgerEntry;

use crate::chain::Chain;
use crate::net::NetCmd;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

const MULTICAST_GROUP: Ipv4Addr = Ipv4Addr::new(239, 1, 1, 1);
const DEFAULT_PORT: u16 = 6944;
const DEFAULT_TTL: u32 = 1; // LAN-only — never crosses a router
const MAX_DGRAM: usize = 65_000;

// ---------------------------------------------------------------------------
// Public handle (cheap clone — just wraps an mpsc::Sender)
// ---------------------------------------------------------------------------

#[derive(Clone)]
pub struct UdpGossipHandle {
    tx: mpsc::Sender<LedgerEntry>,
}

impl UdpGossipHandle {
    /// Multicast `entry` to LAN peers if it qualifies as a light entry type.
    /// Non-blocking: drops the entry if the internal channel is full rather
    /// than blocking the caller.
    pub fn send_if_light(&self, entry: LedgerEntry) {
        if !is_light_entry(&entry) {
            return;
        }
        // try_send: if the channel is full we silently discard — a lost bid
        // on UDP is fine; libp2p gossipsub is still the reliable path.
        let _ = self.tx.try_send(entry);
    }
}

// ---------------------------------------------------------------------------
// Task
// ---------------------------------------------------------------------------

pub struct UdpGossip {
    rx:       mpsc::Receiver<LedgerEntry>,
    chain:    Arc<Chain>,
    p2p_tx:   mpsc::Sender<NetCmd>,
    chain_id: String,
}

impl UdpGossip {
    pub fn new(
        chain:    Arc<Chain>,
        p2p_tx:   mpsc::Sender<NetCmd>,
        chain_id: String,
    ) -> (Self, UdpGossipHandle) {
        let (tx, rx) = mpsc::channel(512);
        let gossip = Self { rx, chain, p2p_tx, chain_id };
        let handle = UdpGossipHandle { tx };
        (gossip, handle)
    }

    pub async fn run(mut self) {
        let port = std::env::var("HONE_UDP_GOSSIP_PORT")
            .ok().and_then(|s| s.parse().ok()).unwrap_or(DEFAULT_PORT);
        let ttl = std::env::var("HONE_UDP_GOSSIP_TTL")
            .ok().and_then(|s| s.parse().ok()).unwrap_or(DEFAULT_TTL);

        let socket = match bind_multicast_socket(port, ttl).await {
            Ok(s) => Arc::new(s),
            Err(e) => {
                warn!("udp-gossip: failed to bind UDP/{}: {} — disabled", port, e);
                return;
            }
        };

        let multicast_dest: SocketAddr =
            SocketAddrV4::new(MULTICAST_GROUP, port).into();
        let recv_socket = socket.clone();
        let send_socket = socket.clone();

        tracing::info!(
            "udp-gossip: listening on 239.1.1.1:{} (LAN multicast, TTL={})",
            port, ttl,
        );

        let mut buf = vec![0u8; MAX_DGRAM];

        loop {
            tokio::select! {
                // ── Receive path ───────────────────────────────────────────
                result = recv_socket.recv_from(&mut buf) => {
                    match result {
                        Ok((len, _src)) => {
                            self.handle_datagram(&buf[..len], &send_socket, multicast_dest).await;
                        }
                        Err(e) => {
                            warn!("udp-gossip: recv error: {}", e);
                            tokio::time::sleep(Duration::from_millis(100)).await;
                        }
                    }
                }

                // ── Send path ─────────────────────────────────────────────
                Some(entry) = self.rx.recv() => {
                    self.send_entry(&entry, &send_socket, multicast_dest).await;
                }
            }
        }
    }

    // -----------------------------------------------------------------------

    async fn send_entry(
        &self,
        entry: &LedgerEntry,
        socket: &UdpSocket,
        dest: SocketAddr,
    ) {
        let payload = match serde_json::to_vec(&serde_json::json!({
            "chain_id": &self.chain_id,
            "entry":    entry,
        })) {
            Ok(b) => b,
            Err(e) => { warn!("udp-gossip: serialize error: {}", e); return; }
        };

        if payload.len() > MAX_DGRAM {
            warn!("udp-gossip: entry too large for UDP ({} bytes), skipping", payload.len());
            return;
        }

        if let Err(e) = socket.send_to(&payload, dest).await {
            debug!("udp-gossip: send error: {}", e);
        }
    }

    async fn handle_datagram(
        &self,
        data: &[u8],
        socket: &UdpSocket,
        dest: SocketAddr,
    ) {
        let json: Value = match serde_json::from_slice(data) {
            Ok(v) => v,
            Err(_) => return, // non-JSON, silently drop
        };

        // Chain ID guard — never process entries from a different chain.
        if json.get("chain_id").and_then(|v| v.as_str()) != Some(self.chain_id.as_str()) {
            return;
        }

        let Some(entry_val) = json.get("entry") else { return; };

        let entry: LedgerEntry = match serde_json::from_value(entry_val.clone()) {
            Ok(e) => e,
            Err(_) => return,
        };

        // Strict allowlist — only expected light types come via UDP.
        if !is_light_entry(&entry) {
            debug!("udp-gossip: rejected non-light entry type");
            return;
        }

        // Canonical validation path — same as every other transport.
        match self.chain.apply_entry(&entry) {
            Ok(_) => {
                debug!("udp-gossip: applied entry via LAN multicast");

                // Re-broadcast to libp2p so the entry propagates beyond LAN.
                let envelope = serde_json::json!({ "entry": &entry });
                if let Ok(bytes) = serde_json::to_vec(&envelope) {
                    let _ = self.p2p_tx.send(NetCmd::Broadcast {
                        topic: "hone/entries",
                        data: bytes,
                    }).await;
                }

                // Also re-multicast so nodes that missed it (packet loss) get it.
                self.send_entry(&entry, socket, dest).await;
            }
            Err(_) => {
                // Validation failure — could be duplicate, bad sig, wrong state.
                // Silently drop; do not re-broadcast.
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Allowlist — only these entry types propagate via UDP multicast.
// Expand this list when Phase 3 (LoRa) adds more light entry types.
// ---------------------------------------------------------------------------

fn is_light_entry(entry: &LedgerEntry) -> bool {
    matches!(entry, LedgerEntry::InferenceJobBid { .. })
}

// ---------------------------------------------------------------------------
// Socket setup
// ---------------------------------------------------------------------------

async fn bind_multicast_socket(port: u16, ttl: u32) -> anyhow::Result<UdpSocket> {
    use socket2::{Domain, Protocol, Socket, Type};

    // socket2 lets us set SO_REUSEADDR/SO_REUSEPORT before binding,
    // which is required for multicast on Linux.
    let s2 = Socket::new(Domain::IPV4, Type::DGRAM, Some(Protocol::UDP))?;
    s2.set_reuse_address(true)?;
    #[cfg(unix)]
    s2.set_reuse_port(true)?;

    s2.bind(&SocketAddrV4::new(Ipv4Addr::UNSPECIFIED, port).into())?;
    s2.set_multicast_ttl_v4(ttl)?;
    // Receive our own multicasts (useful when multiple nodes on same host).
    s2.set_multicast_loop_v4(true)?;
    s2.join_multicast_v4(&MULTICAST_GROUP, &Ipv4Addr::UNSPECIFIED)?;

    s2.set_nonblocking(true)?;
    Ok(UdpSocket::from_std(s2.into())?)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use tempfile::TempDir;

    fn make_chain() -> (Arc<Chain>, TempDir) {
        let dir = tempfile::Builder::new()
            .prefix("hone_udp_test_")
            .tempdir()
            .expect("tempdir");
        let store = crate::store::Store::open(dir.path()).expect("store open");
        let chain = Arc::new(Chain::new(store, "udp-test-node".into(), "hone-testnet".into()));
        (chain, dir)
    }

    /// `UdpGossip::new` must construct without panic and return a valid handle
    /// whose sender channel is immediately usable.
    #[test]
    fn udp_gossip_new_constructs_without_panic() {
        let (chain, _dir) = make_chain();
        let (p2p_tx, _p2p_rx) = tokio::sync::mpsc::channel(64);
        let (_gossip, handle) = UdpGossip::new(chain, p2p_tx, "hone-testnet".into());

        // The handle must be Clone and the sender must be valid (not closed).
        let _handle2 = handle.clone();
        assert!(
            !handle.tx.is_closed(),
            "internal mpsc sender must not be closed immediately after construction"
        );
    }

    /// `UdpGossipHandle::send_if_light` must silently drop entry types that are
    /// not on the allowlist (everything except `InferenceJobBid`), without
    /// panicking and without delivering anything to the internal channel.
    #[test]
    fn send_if_light_drops_non_light_entries_silently() {
        let (chain, _dir) = make_chain();
        let (p2p_tx, _p2p_rx) = tokio::sync::mpsc::channel(64);
        let (_gossip, handle) = UdpGossip::new(chain, p2p_tx, "hone-testnet".into());

        // A Transfer entry is not a light entry — must be dropped, not queued.
        let non_light = LedgerEntry::Transfer {
            from:      "alice".into(),
            to:        "bob".into(),
            amount:    1_000_000,
            token:     hone_types::NATIVE_TOKEN.into(),
            memo:      None,
            epoch:     1,
            nonce:     1,
            signed_by: "alice".into(),
            twofactor: None,
        };
        handle.send_if_light(non_light);

        // The internal receiver must still be empty — nothing was enqueued.
        assert_eq!(
            handle.tx.max_capacity() - handle.tx.capacity(),
            0,
            "non-light entry must not be enqueued in the internal channel"
        );
    }

    /// `is_light_entry` accepts `InferenceJobBid` and rejects everything else.
    /// Tests the allowlist predicate in isolation.
    #[test]
    fn is_light_entry_allowlist_correct() {
        let bid = LedgerEntry::InferenceJobBid {
            job_id:    "job-001".into(),
            bidder:    "alice".into(),
            fee:       500_000,
            role:      "worker".into(),
            epoch:     1,
            nonce:     1,
            signed_by: "alice".into(),
        };
        assert!(
            is_light_entry(&bid),
            "InferenceJobBid must be classified as a light entry"
        );

        let transfer = LedgerEntry::Transfer {
            from:      "alice".into(),
            to:        "bob".into(),
            amount:    1,
            token:     hone_types::NATIVE_TOKEN.into(),
            memo:      None,
            epoch:     1,
            nonce:     2,
            signed_by: "alice".into(),
            twofactor: None,
        };
        assert!(
            !is_light_entry(&transfer),
            "Transfer must not be classified as a light entry"
        );

        let stake = LedgerEntry::Stake {
            account:   "alice".into(),
            amount:    1_000_000,
            epoch:     1,
            signed_by: "alice".into(),
            nonce:     3,
        };
        assert!(
            !is_light_entry(&stake),
            "Stake must not be classified as a light entry"
        );
    }
}
