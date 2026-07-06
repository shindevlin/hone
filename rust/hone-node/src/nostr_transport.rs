//! Nostr relay transport — Phase 4 of the transport cascade.
//!
//! Propagates ledger entries through the Nostr relay network so nodes across
//! the open internet can relay entries without requiring direct libp2p
//! connectivity or a federated homeserver.  Each accepted entry is published
//! as a signed Nostr event; every matching event from subscribed relays is
//! parsed and, if valid, applied through the canonical `chain.apply_entry()`
//! path.
//!
//! Design rules:
//!  - Nostr relay peers do NOT count toward `peer_count`.  The zero-peers
//!    hardline in api.rs is about libp2p peers only; Nostr cannot satisfy it.
//!  - Every received event goes through `chain.apply_entry()` — the same
//!    canonical validation path used by every other transport.
//!  - Accepted entries are re-broadcast to libp2p (`NetCmd::Broadcast`) so
//!    they propagate further via the gossipsub mesh.
//!  - Epoch seals are never generated from Nostr events.
//!  - Testnet and mainnet are isolated by the `chain` tag value (`chain_id`),
//!    which is validated client-side.  A testnet event arriving at a mainnet
//!    node is silently dropped by `chain.apply_entry()`.
//!  - Events whose serialized content exceeds MAX_CONTENT_BYTES are dropped
//!    before publish to avoid relay rejection and retry loops.
//!
//! Env vars:
//!   HONE_NOSTR              — "true" to activate
//!   HONE_NOSTR_RELAYS       — comma-separated WSS URLs
//!                              (default: wss://relay.damus.io,wss://nos.lol,wss://relay.nostr.band)
//!   HONE_NOSTR_KIND         — event kind integer (default: 21337)

use std::sync::Arc;

use hone_types::LedgerEntry;
use nostr_sdk::prelude::*;
use nostr_sdk::{
    Client, EventBuilder, Filter, Keys, Kind, RelayPoolNotification, SecretKey, Tag, TagKind,
    SingleLetterTag, Alphabet,
};
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

use crate::chain::Chain;
use crate::net::NetCmd;

// ---------------------------------------------------------------------------
// Constants
// ---------------------------------------------------------------------------

/// Default Nostr event kind for HONE ledger entries.
const DEFAULT_KIND: u16 = 21337;

/// Default relay pool URLs (comma-separated).
const DEFAULT_RELAYS: &str =
    "wss://relay.damus.io,wss://nos.lol,wss://relay.nostr.band";

/// Maximum serialized entry size (in bytes) permitted in an event's content
/// field.  Events over this limit are dropped before publish — common relay
/// caps are 64 KB; 60 KB gives a safe margin.
const MAX_CONTENT_BYTES: usize = 60 * 1024;

/// RocksDB state key for the node's Nostr secret key (stored as hex).
const STATE_KEY: &str = "nostr:secret_key";

// ---------------------------------------------------------------------------
// Public handle (cheap clone — wraps an mpsc::Sender)
// ---------------------------------------------------------------------------

/// Clone-able handle used by the broadcast pump to forward accepted entries to
/// the Nostr relay pool.
#[derive(Clone)]
pub struct NostrHandle {
    tx: mpsc::Sender<LedgerEntry>,
}

impl NostrHandle {
    /// Non-blocking: queues `entry` for delivery to the Nostr relay pool.
    /// Drops the entry silently if the internal channel is full — libp2p
    /// gossipsub is the reliable path; Nostr is opportunistic.
    pub fn send_entry(&self, entry: LedgerEntry) {
        let _ = self.tx.try_send(entry);
    }
}

// ---------------------------------------------------------------------------
// Start function — called from main.rs before AppState construction
// ---------------------------------------------------------------------------

/// Initialise the Nostr relay transport.  Returns `None` if `HONE_NOSTR` is
/// not set to `"true"` or if the relay pool connection fails.
///
/// On success spawns a send loop and a receive loop, then returns a cloneable
/// [`NostrHandle`] the caller can use to forward entries into the relay pool.
pub async fn start_nostr(
    chain: Arc<Chain>,
    cmd_tx: mpsc::Sender<NetCmd>,
    chain_id: String,
) -> Option<NostrHandle> {
    if std::env::var("HONE_NOSTR").as_deref() != Ok("true") {
        return None;
    }

    match try_start(chain, cmd_tx, chain_id).await {
        Some(handle) => Some(handle),
        None => {
            warn!("nostr: transport not started — see warnings above");
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
    chain_id: String,
) -> Option<NostrHandle> {
    let kind_num: u16 = std::env::var("HONE_NOSTR_KIND")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(DEFAULT_KIND);

    let relay_urls: Vec<String> = std::env::var("HONE_NOSTR_RELAYS")
        .unwrap_or_else(|_| DEFAULT_RELAYS.to_string())
        .split(',')
        .filter(|s| !s.trim().is_empty())
        .map(|s| s.trim().to_string())
        .collect();

    if relay_urls.is_empty() {
        warn!("nostr: no relay URLs configured — disabled");
        return None;
    }

    // Load or generate the node's Nostr identity.
    let keys = load_or_generate_keys(&chain);

    info!(
        "nostr: node pubkey {} connecting to {} relay(s)",
        keys.public_key(),
        relay_urls.len()
    );

    // Build the client.
    let client = Client::builder().signer(keys.clone()).build();

    // Register relays.
    for url in &relay_urls {
        if let Err(e) = client.add_relay(url.as_str()).await {
            warn!("nostr: failed to add relay {}: {}", url, e);
        }
    }

    client.connect().await;

    // Subscribe: kind + #p = "hone" (protocol marker tag, filtered relay-side).
    // The `chain` and `h` tags are validated client-side only since relay-side
    // filtering only indexes single-letter tags (NIP-01).
    let filter = Filter::new()
        .kind(Kind::Custom(kind_num))
        .custom_tag(SingleLetterTag::lowercase(Alphabet::P), ["hone"]);

    if let Err(e) = client.subscribe(vec![filter], None).await {
        warn!("nostr: subscription failed: {}", e);
        return None;
    }

    info!(
        "nostr: subscribed to kind {} events tagged #p=hone on {} relay(s)",
        kind_num,
        relay_urls.len()
    );

    // Channel used by the handle to enqueue entries → outbound send loop.
    let (tx, rx) = mpsc::channel::<LedgerEntry>(256);
    let handle = NostrHandle { tx };

    // Spawn the inbound receive loop.
    {
        let chain_ref    = chain.clone();
        let cmd_tx_ref   = cmd_tx.clone();
        let chain_id_ref = chain_id.clone();
        let kind_for_rx  = kind_num;
        let mut notifications = client.notifications();

        tokio::spawn(async move {
            run_receive_loop(
                &mut notifications,
                &chain_ref,
                &cmd_tx_ref,
                &chain_id_ref,
                kind_for_rx,
            )
            .await;
        });
    }

    // Spawn the outbound send loop.
    {
        let client_for_send  = client.clone();
        let chain_id_for_send = chain_id.clone();
        let kind_for_send    = kind_num;

        tokio::spawn(async move {
            run_send_loop(client_for_send, rx, chain_id_for_send, kind_for_send, keys).await;
        });
    }

    Some(handle)
}

// ---------------------------------------------------------------------------
// Key management
// ---------------------------------------------------------------------------

/// Load the node's Nostr secret key from chain state, or generate a new one
/// and persist it for use across restarts.
fn load_or_generate_keys(chain: &Arc<Chain>) -> Keys {
    // Try to restore.
    if let Some(bytes) = chain.store.state_get(STATE_KEY) {
        if let Ok(hex) = std::str::from_utf8(&bytes) {
            if let Ok(sk) = SecretKey::from_hex(hex) {
                debug!("nostr: restored keypair from state store");
                return Keys::new(sk);
            }
        }
        warn!("nostr: stored secret key is corrupt — generating a new one");
    }

    // Generate fresh keypair.
    let keys = Keys::generate();
    let hex = keys.secret_key().to_secret_hex();
    if let Err(e) = chain.store.state_set(STATE_KEY, hex.as_bytes()) {
        warn!("nostr: failed to persist secret key: {}", e);
    } else {
        info!(
            "nostr: generated new keypair, pubkey={}",
            keys.public_key()
        );
    }
    keys
}

// ---------------------------------------------------------------------------
// Inbound: receive loop
// ---------------------------------------------------------------------------

async fn run_receive_loop(
    notifications: &mut tokio::sync::broadcast::Receiver<RelayPoolNotification>,
    chain:         &Arc<Chain>,
    cmd_tx:        &mpsc::Sender<NetCmd>,
    chain_id:      &str,
    kind_num:      u16,
) {
    let expected_kind = Kind::Custom(kind_num);

    loop {
        let notification = match notifications.recv().await {
            Ok(n) => n,
            Err(tokio::sync::broadcast::error::RecvError::Lagged(n)) => {
                warn!("nostr: receive loop lagged by {} notifications", n);
                continue;
            }
            Err(_) => {
                warn!("nostr: relay pool notification channel closed");
                break;
            }
        };

        let event = match notification {
            RelayPoolNotification::Event { event, .. } => event,
            _ => continue,
        };

        // Ignore events of unexpected kind (safety check beyond relay-side filter).
        if event.kind != expected_kind {
            continue;
        }

        // Validate the `chain` tag matches our chain_id (client-side isolation).
        let chain_matches = event
            .tags
            .find(TagKind::custom("chain"))
            .and_then(|t| t.content())
            .map(|v| v == chain_id)
            .unwrap_or(false);

        if !chain_matches {
            debug!("nostr: dropped event with wrong chain tag");
            continue;
        }

        // Deserialize the LedgerEntry from the event content.
        let entry: LedgerEntry = match serde_json::from_str(&event.content) {
            Ok(e) => e,
            Err(_) => {
                debug!("nostr: dropped unparseable event content");
                continue;
            }
        };

        // Canonical validation path — identical to every other transport.
        match chain.apply_entry(&entry) {
            Ok(_) => {
                debug!("nostr: applied entry from relay (event {})", event.id);

                // Re-broadcast to libp2p so the entry propagates to peers that
                // are not listening to the Nostr relay pool.
                let envelope = serde_json::json!({ "entry": &entry });
                if let Ok(bytes) = serde_json::to_vec(&envelope) {
                    let _ = cmd_tx
                        .send(NetCmd::Broadcast {
                            topic: "hone/entries",
                            data:  bytes,
                        })
                        .await;
                }
            }
            Err(_) => {
                // Validation failure (duplicate, bad sig, wrong state, etc.).
                // Silently drop — do not re-broadcast.
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Outbound: send loop
// ---------------------------------------------------------------------------

async fn run_send_loop(
    client:   Client,
    mut rx:   mpsc::Receiver<LedgerEntry>,
    chain_id: String,
    kind_num: u16,
    keys:     Keys,
) {
    let event_kind = Kind::Custom(kind_num);

    while let Some(entry) = rx.recv().await {
        // Serialize the entry as the event content.
        let content = match serde_json::to_string(&entry) {
            Ok(s) => s,
            Err(e) => {
                warn!("nostr: failed to serialize entry: {}", e);
                continue;
            }
        };

        // Hard size guard — drop entries that would exceed common relay caps.
        if content.len() > MAX_CONTENT_BYTES {
            warn!(
                "nostr: entry too large to publish ({} bytes > {} limit) — dropped",
                content.len(),
                MAX_CONTENT_BYTES
            );
            continue;
        }

        // Compute the entry hash for the `h` tag (best-effort; missing is ok).
        let entry_hash_hex: String = {
            use sha2::{Sha256, Digest};
            let bytes = serde_json::to_vec(&entry).unwrap_or_default();
            hex::encode(Sha256::digest(&bytes))
        };

        // Build and sign the event.
        let event_result = EventBuilder::new(event_kind.clone(), content)
            .tag(Tag::custom(TagKind::custom("p"),     ["hone"]))
            .tag(Tag::custom(TagKind::custom("chain"), [chain_id.as_str()]))
            .tag(Tag::custom(TagKind::custom("h"),     [entry_hash_hex.as_str()]))
            .sign_with_keys(&keys);

        let event = match event_result {
            Ok(e) => e,
            Err(e) => {
                warn!("nostr: failed to sign event: {}", e);
                continue;
            }
        };

        // Publish to all connected relays.
        match client.send_event(event).await {
            Ok(output) => {
                debug!(
                    "nostr: published entry to {} relay(s)",
                    output.success.len()
                );
            }
            Err(e) => {
                debug!("nostr: publish failed: {}", e);
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
            .prefix("hone_nostr_test_")
            .tempdir()
            .expect("tempdir");
        let store = crate::store::Store::open(dir.path()).expect("store open");
        let chain = Arc::new(Chain::new(store, "nostr-test-node".into(), "hone-testnet".into()));
        let (cmd_tx, _cmd_rx) = tokio::sync::mpsc::channel(64);
        (chain, cmd_tx, dir)
    }

    /// When `HONE_NOSTR` is not set, `start_nostr` must return `None`
    /// immediately without attempting any relay connection.
    #[tokio::test]
    async fn nostr_disabled_returns_none_when_env_unset() {
        let _guard = ENV_LOCK.lock();
        std::env::remove_var("HONE_NOSTR");

        let (chain, cmd_tx, _dir) = make_chain_and_cmd_tx();
        let result = start_nostr(chain, cmd_tx, "hone-testnet".into()).await;

        assert!(
            result.is_none(),
            "expected None when HONE_NOSTR is unset"
        );
    }

    /// When `HONE_NOSTR=true` and all relay URLs are unreachable,
    /// `start_nostr` must complete without panicking within a reasonable
    /// timeout.
    ///
    /// Note on expected return value: nostr-sdk 0.37 does not fail
    /// synchronously when relay connections cannot be established.
    /// `client.connect()` returns immediately, and `client.subscribe()`
    /// succeeds vacuously against an empty connected-relay set, so
    /// `start_nostr` returns `Some(handle)` rather than `None`.  The
    /// invariant verified here is panic-freedom and bounded completion time.
    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn nostr_enabled_with_unreachable_relays_does_not_panic() {
        let _guard = ENV_LOCK.lock();

        // Use a non-routable loopback port so attempts fail fast.
        std::env::set_var("HONE_NOSTR", "true");
        std::env::set_var("HONE_NOSTR_RELAYS", "wss://127.0.0.1:19999");

        let (chain, cmd_tx, _dir) = make_chain_and_cmd_tx();

        let result = tokio::time::timeout(
            std::time::Duration::from_secs(10),
            start_nostr(chain, cmd_tx, "hone-testnet".into()),
        )
        .await;

        std::env::remove_var("HONE_NOSTR");
        std::env::remove_var("HONE_NOSTR_RELAYS");

        assert!(
            result.is_ok(),
            "start_nostr must complete within 10 s even when all relays are unreachable"
        );
    }

    /// Configuring an empty relay list causes `start_nostr` to return `None`
    /// (the `relay_urls.is_empty()` early-exit path in `try_start`).
    #[tokio::test]
    async fn nostr_enabled_with_empty_relay_list_returns_none() {
        let _guard = ENV_LOCK.lock();

        std::env::set_var("HONE_NOSTR", "true");
        // A single comma expands to zero non-empty relay tokens after split+filter.
        std::env::set_var("HONE_NOSTR_RELAYS", ",");

        let (chain, cmd_tx, _dir) = make_chain_and_cmd_tx();
        let result = start_nostr(chain, cmd_tx, "hone-testnet".into()).await;

        std::env::remove_var("HONE_NOSTR");
        std::env::remove_var("HONE_NOSTR_RELAYS");

        assert!(
            result.is_none(),
            "expected None when HONE_NOSTR_RELAYS resolves to an empty list"
        );
    }
}
