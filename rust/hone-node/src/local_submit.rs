//! Node-originated entries go through consensus, like any user entry.
//!
//! Background roles (storage, service, sensor, worker) used to `chain.apply_entry()` their
//! own heartbeats/bids locally and gossip them UNSIGNED. Peers queued the unsigned copy,
//! `validate_and_apply` rejected it at seal ("signature required"), so every node held only
//! its OWN `storage_beat:` / `service_beat:` / `sensor_commit:` rows and the deterministic
//! reward driver paid each pool to itself alone: a per-node balance fork (grouchly vs
//! beastly state_root drift).
//!
//! `submit_own` signs the entry with this node's posting key and queues it exactly like
//! `api::submit_entry`: pending pool + outbox + signed gossip. It is applied only at a
//! quorum seal, on every node, through `validate_and_apply`.
//!
//! Hardline: refused while peer_count == 0, and while the state healer has halted the node.

use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;

use anyhow::{bail, Result};
use hone_types::LedgerEntry;

use crate::chain::Chain;
use crate::net::NetCmd;

/// libp2p peer count, mirrored from main's shared counter (UDP/Nostr/etc. never count).
pub static PEER_COUNT: AtomicUsize = AtomicUsize::new(0);
static SIGNER: OnceLock<ed25519_dalek::SigningKey> = OnceLock::new();

/// Install the node's posting key (once, at startup).
pub fn set_signer(sk: &ed25519_dalek::SigningKey) {
    let _ = SIGNER.set(sk.clone());
}

/// Sign `entry` the way `tx::check_signature` verifies it (canonical message, posting key).
pub fn sign(entry: &LedgerEntry, chain_id: &str) -> Option<String> {
    let sk = SIGNER.get()?;
    let msg = crate::tx::canonical_signing_message(entry, chain_id).ok()?;
    use ed25519_dalek::Signer;
    Some(hex::encode(sk.sign(msg.as_bytes()).to_bytes()))
}

/// Sign raw bytes with the node's posting key (peer self-identification; not an entry).
pub fn sign_bytes(msg: &[u8]) -> Option<String> {
    use ed25519_dalek::Signer;
    Some(hex::encode(SIGNER.get()?.sign(msg).to_bytes()))
}

/// Gossip envelope carrying the signature, the shape the receive path expects.
pub fn envelope(entry: &LedgerEntry, sig: Option<&str>) -> Option<Vec<u8>> {
    let mut v = serde_json::json!({ "entry": entry });
    if let Some(s) = sig.filter(|s| !s.is_empty()) {
        v["sig"] = serde_json::Value::String(s.to_owned());
    }
    serde_json::to_vec(&v).ok()
}

/// Queue a node-originated entry for consensus. Returns the entry hash.
pub fn queue_own(chain: &Chain, entry: &LedgerEntry, signed: bool) -> Result<(String, Option<String>)> {
    if PEER_COUNT.load(Ordering::Relaxed) == 0 {
        bail!("not connected to network — node entry held to prevent local fork");
    }
    if crate::state_healer::halted() {
        bail!("state healer has halted this node (diverged state) — not submitting");
    }
    let sig = if signed {
        match sign(entry, &chain.chain_id) {
            Some(s) => Some(s),
            None => bail!("no posting key loaded — cannot sign node entry"),
        }
    } else {
        None
    };
    crate::tx::precheck_submission(chain, entry, sig.as_deref())?;
    let hash = entry.hash();
    if chain.push_pending(entry.clone(), sig.clone()) {
        chain.outbox_push(entry.clone(), sig.clone(), chain.current_epoch());
    }
    Ok((hash, sig))
}

/// An entry that arrived over a side transport (UDP multicast, Nostr, I2P, Matrix,
/// LoRaWAN). These used to `apply_entry` it on the spot — no signature check, no seal —
/// so the LAN nodes that heard a datagram applied it and the rest never did. Now it joins
/// the pending pool like gossip and is validated at seal. Returns true if newly queued.
pub fn queue_relayed(chain: &Chain, entry: LedgerEntry, sig: Option<String>) -> bool {
    if crate::tx::is_system_entry(&entry) { return false; }
    if crate::tx::precheck_submission(chain, &entry, sig.as_deref()).is_err() { return false; }
    chain.push_pending(entry, sig)
}

/// `queue_own` + gossip on hone/entries.
pub async fn submit_own(
    chain: &Chain,
    cmd_tx: &tokio::sync::mpsc::Sender<NetCmd>,
    entry: &LedgerEntry,
) -> Result<String> {
    let (hash, sig) = queue_own(chain, entry, true)?;
    if let Some(data) = envelope(entry, sig.as_deref()) {
        let _ = cmd_tx.send(NetCmd::Broadcast { topic: "hone/entries", data }).await;
    }
    Ok(hash)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn chain(label: &str) -> (Chain, tempfile::TempDir) {
        let dir = tempfile::tempdir().unwrap();
        let store = crate::store::Store::open(dir.path()).unwrap();
        (Chain::new(store, format!("n-{label}"), "hone-test".into()), dir)
    }

    fn beat(node: &str, epoch: u64) -> LedgerEntry {
        LedgerEntry::StorageHeartbeat {
            node_id: node.into(), epoch, bytes_proven: 1 << 30, query_count: 0,
            challenge_response: None, tier: Some(1), signed_by: node.into(),
        }
    }

    /// The old path: apply locally. Only the originating node gets the storage_beat row,
    /// so the reward driver on the two nodes sees different storage sets.
    #[test]
    fn local_apply_forks_storage_beats() {
        let (a, _da) = chain("a");
        let (b, _db) = chain("b");
        a.apply_entry(&beat("alice", 7)).unwrap();
        assert!(a.store.state_get("storage_beat:7:alice").is_some());
        assert!(b.store.state_get("storage_beat:7:alice").is_none());
    }

    /// The new path: queued, not applied, until a seal drains the pool. Hardline holds.
    #[test]
    fn queue_own_refuses_without_peers_and_never_applies_locally() {
        let _g = crate::state_healer::TEST_GLOBALS.lock();
        let (a, _da) = chain("q");
        PEER_COUNT.store(0, Ordering::Relaxed);
        assert!(queue_own(&a, &beat("alice", 9), false).is_err());
        assert_eq!(a.pending_count(), 0);

        PEER_COUNT.store(3, Ordering::Relaxed);
        queue_own(&a, &beat("alice", 9), false).unwrap();
        assert_eq!(a.pending_count(), 1);
        assert!(a.store.state_get("storage_beat:9:alice").is_none(), "must not apply before seal");
        assert_eq!(a.outbox.lock().len(), 1);
        PEER_COUNT.store(0, Ordering::Relaxed);
    }

    /// The envelope carries the signature where the gossip receive path reads it.
    #[test]
    fn envelope_carries_sig() {
        let v: serde_json::Value =
            serde_json::from_slice(&envelope(&beat("x", 1), Some("ab")).unwrap()).unwrap();
        assert_eq!(v["sig"], "ab");
        assert!(v["entry"]["StorageHeartbeat"].is_object() || v["entry"].is_object());
    }
}
