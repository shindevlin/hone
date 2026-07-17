//! The chain submission boundary — the ONE mocked seam.
//!
//! In production `ChainSubmit::submit` wires to `apply_and_broadcast` in
//! `rust/hone-node/src/api.rs` (the normal chain path: validate -> gossip ->
//! seal, split by the EXISTING `SensorDataPurchase` split logic; the bridge
//! invents no new split). A documented pre-existing rustc ICE blocks building
//! the full `hone-node` binary on this machine, so we cannot exercise that path
//! end-to-end here — the bridge's own logic is tested against `MockChain`,
//! which models the chain's REAL semantics that the bridge depends on:
//!
//! - a per-account monotonic nonce (`check_nonce`: submitted must == current+1;
//!   a reused nonce is rejected with a "nonce already used" style error), which
//!   is the chain-side idempotency backstop the bridge relies on;
//! - the no-peers hardline (`apply_and_broadcast` rejects when peer_count == 0),
//!   a retryable, non-success condition;
//! - insufficient balance (underfunded settlement account), retryable.
//!
//! NO code here claims a real on-chain post succeeded — `MockChain` is
//! explicitly a model, not the chain.

use hone_types::entry::LedgerEntry;
use hone_types::AccountId;

/// Outcome of a submit, mapped to what the worker needs to decide.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubmitOutcome {
    /// Accepted and (for our purposes) sealed. Carries the entry hash.
    Sealed { entry_hash: String },
    /// The nonce was already consumed by a prior post of THIS entry. This is
    /// the "already posted" reconcile path, NOT a failure — the worker resolves
    /// the existing entry hash and marks the intent posted.
    NonceAlreadyUsed,
    /// Retryable: node has no peers (hardline), so nothing was applied.
    NoPeers,
    /// Retryable: settlement account underfunded.
    InsufficientBalance,
    /// Retryable transport/other error.
    Transient(String),
}

/// The mocked boundary. Real impl calls `apply_and_broadcast`.
pub trait ChainSubmit {
    /// Submit a fully-constructed, signed entry. `sig_hex` is the ed25519
    /// signature over the canonical signing message (see `sign_entry`).
    fn submit(&mut self, entry: &LedgerEntry, sig_hex: &str) -> SubmitOutcome;

    /// Query whether an entry previously submitted at `(buyer, nonce)` was
    /// sealed, returning its hash. Used by crash-recovery to resolve a
    /// `posting` intent without re-submitting (mirrors querying chain state for
    /// the pinned entry). Returns `None` if not found.
    fn find_sealed(&self, buyer: &str, nonce: u64) -> Option<String>;

    /// Current chain epoch (captured onto the intent at post time).
    fn current_epoch(&self) -> u64;

    /// Next expected nonce for `account` (chain nonce + 1). The worker pins the
    /// value it uses so a retry re-posts the identical entry.
    fn next_nonce(&self, account: &str) -> u64;
}

/// Deterministic canonical signing message for a `SensorDataPurchase`, matching
/// `rust/hone-node/src/tx.rs::canonical_signing_message`. `SensorDataPurchase`
/// is NOT in that function's explicit arm list, so it hits the catch-all:
/// `{ "chain_id": <id>, "entry": <serde_json::to_string(entry)> }`. We
/// reproduce that exact shape so a signature produced here verifies on-chain.
pub fn canonical_signing_message(entry: &LedgerEntry, chain_id: &str) -> anyhow::Result<String> {
    let msg = serde_json::json!({
        "chain_id": chain_id,
        "entry": serde_json::to_string(entry)?,
    });
    Ok(serde_json::to_string(&msg)?)
}

/// Sign the canonical message for `entry` with the settlement account's ed25519
/// signing key, returning hex. The key comes from env at the call site and is
/// never persisted (see `SigningIdentity`).
pub fn sign_entry(
    signing_key: &ed25519_dalek::SigningKey,
    entry: &LedgerEntry,
    chain_id: &str,
) -> anyhow::Result<String> {
    use ed25519_dalek::Signer;
    let msg = canonical_signing_message(entry, chain_id)?;
    let sig = signing_key.sign(msg.as_bytes());
    Ok(hex::encode(sig.to_bytes()))
}

/// The settlement account identity: the account id (chain `buyer`) plus its
/// ed25519 signing key. Loaded from env ONLY — the key is never written to the
/// intent store, logs, or git.
pub struct SigningIdentity {
    pub account: AccountId,
    pub signing_key: ed25519_dalek::SigningKey,
}

impl SigningIdentity {
    /// Load from env: `HONE_SETTLEMENT_ACCOUNT` (account id) and
    /// `HONE_SETTLEMENT_SIGNING_KEY` (64-hex-char ed25519 secret seed). Errors
    /// (rather than defaulting) if either is missing — the bridge must never
    /// post with an implicit/blank key.
    pub fn from_env() -> anyhow::Result<Self> {
        let account = std::env::var("HONE_SETTLEMENT_ACCOUNT")
            .map_err(|_| anyhow::anyhow!("HONE_SETTLEMENT_ACCOUNT not set"))?;
        let key_hex = std::env::var("HONE_SETTLEMENT_SIGNING_KEY")
            .map_err(|_| anyhow::anyhow!("HONE_SETTLEMENT_SIGNING_KEY not set"))?;
        let seed = hex::decode(key_hex.trim())
            .map_err(|e| anyhow::anyhow!("HONE_SETTLEMENT_SIGNING_KEY not valid hex: {e}"))?;
        let seed: [u8; 32] = seed
            .as_slice()
            .try_into()
            .map_err(|_| anyhow::anyhow!("HONE_SETTLEMENT_SIGNING_KEY must be 32 bytes (64 hex)"))?;
        Ok(SigningIdentity {
            account,
            signing_key: ed25519_dalek::SigningKey::from_bytes(&seed),
        })
    }

    /// Construct directly (tests only).
    pub fn new(account: impl Into<String>, signing_key: ed25519_dalek::SigningKey) -> Self {
        SigningIdentity {
            account: account.into(),
            signing_key,
        }
    }
}

/// Build the `SensorDataPurchase` entry the bridge posts for an intent. `buyer`
/// is the platform settlement account; `owner` is the sensor owner from the
/// intent (the load-bearing field the chain actually pays); `fee` is the
/// resolved hunits; `batch_hash` is the aggregator provenance discriminator.
#[allow(clippy::too_many_arguments)]
pub fn build_purchase_entry(
    sensor_id: &str,
    buyer: &str,
    owner: &str,
    batch_hash: &str,
    fee_hunits: u64,
    epoch: u64,
    nonce: u64,
) -> LedgerEntry {
    LedgerEntry::SensorDataPurchase {
        sensor_id: sensor_id.to_string(),
        buyer: buyer.to_string(),
        owner: owner.to_string(),
        batch_hash: batch_hash.to_string(),
        fee: fee_hunits,
        epoch,
        nonce,
        signed_by: buyer.to_string(),
    }
}

/// A model of the chain that enforces the REAL nonce/no-peers/balance semantics
/// the bridge depends on. NOT the chain — a test double for the mocked
/// `ChainSubmit` boundary. Configurable to simulate no-peers, low balance, and
/// crash-in-flight.
pub struct MockChain {
    epoch: u64,
    /// account -> next expected nonce (chain nonce is next-1).
    next_nonce: std::collections::HashMap<String, u64>,
    /// Sealed entries keyed by (buyer, nonce) -> entry hash.
    sealed: std::collections::HashMap<(String, u64), String>,
    /// If false, every submit returns NoPeers (hardline).
    pub has_peers: bool,
    /// Settlement account balance in hunits (for insufficient-balance sim).
    pub balance: u64,
    /// When set, the NEXT submit records the entry as sealed but then reports
    /// `Transient("crash")` (simulating a crash AFTER the chain applied but
    /// BEFORE the worker recorded `posted`). One-shot.
    pub crash_after_seal_once: bool,
}

impl MockChain {
    pub fn new(epoch: u64) -> Self {
        MockChain {
            epoch,
            next_nonce: std::collections::HashMap::new(),
            sealed: std::collections::HashMap::new(),
            has_peers: true,
            balance: u64::MAX,
            crash_after_seal_once: false,
        }
    }

    fn entry_hash(entry: &LedgerEntry, sig_hex: &str) -> String {
        use sha2::{Digest, Sha256};
        let mut h = Sha256::new();
        h.update(serde_json::to_vec(entry).unwrap_or_default());
        h.update(sig_hex.as_bytes());
        hex::encode(h.finalize())
    }
}

impl ChainSubmit for MockChain {
    fn submit(&mut self, entry: &LedgerEntry, sig_hex: &str) -> SubmitOutcome {
        if !self.has_peers {
            return SubmitOutcome::NoPeers;
        }
        let (buyer, nonce, fee) = match entry {
            LedgerEntry::SensorDataPurchase {
                buyer, nonce, fee, ..
            } => (buyer.clone(), *nonce, *fee),
            _ => return SubmitOutcome::Transient("unexpected entry type".into()),
        };
        // Real check_nonce semantics: submitted must == current expected.
        let expected = *self.next_nonce.get(&buyer).unwrap_or(&1);
        if nonce < expected {
            // Already consumed (a prior post of this entry sealed).
            return SubmitOutcome::NonceAlreadyUsed;
        }
        if nonce > expected {
            return SubmitOutcome::Transient(format!(
                "invalid nonce: got {nonce}, expected {expected}"
            ));
        }
        if self.balance < fee {
            return SubmitOutcome::InsufficientBalance;
        }
        // Apply: seal the entry, bump nonce, debit balance.
        let hash = Self::entry_hash(entry, sig_hex);
        self.sealed.insert((buyer.clone(), nonce), hash.clone());
        self.next_nonce.insert(buyer.clone(), expected + 1);
        self.balance = self.balance.saturating_sub(fee);
        if self.crash_after_seal_once {
            self.crash_after_seal_once = false;
            // The chain applied, but the worker will observe a crash/transport
            // failure and NOT learn the hash — exactly the dangerous in-flight
            // case redrive must recover.
            return SubmitOutcome::Transient("crash after seal".into());
        }
        SubmitOutcome::Sealed { entry_hash: hash }
    }

    fn find_sealed(&self, buyer: &str, nonce: u64) -> Option<String> {
        self.sealed.get(&(buyer.to_string(), nonce)).cloned()
    }

    fn current_epoch(&self) -> u64 {
        self.epoch
    }

    fn next_nonce(&self, account: &str) -> u64 {
        *self.next_nonce.get(account).unwrap_or(&1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_message_uses_catchall_shape() {
        let e = build_purchase_entry("s1", "settle", "owner", "bh", 1000, 5, 1);
        let msg = canonical_signing_message(&e, "hone-testnet").unwrap();
        // Must be the catch-all form: chain_id + entry-as-string.
        let v: serde_json::Value = serde_json::from_str(&msg).unwrap();
        assert_eq!(v["chain_id"], "hone-testnet");
        assert!(v["entry"].is_string(), "entry must be stringified JSON");
    }

    #[test]
    fn signing_is_deterministic_and_verifies() {
        use ed25519_dalek::{Signer, Verifier};
        let sk = ed25519_dalek::SigningKey::from_bytes(&[7u8; 32]);
        let e = build_purchase_entry("s1", "settle", "owner", "bh", 1000, 5, 1);
        let sig_hex = sign_entry(&sk, &e, "hone-testnet").unwrap();
        let msg = canonical_signing_message(&e, "hone-testnet").unwrap();
        let sig_bytes: [u8; 64] = hex::decode(&sig_hex).unwrap().try_into().unwrap();
        let sig = ed25519_dalek::Signature::from_bytes(&sig_bytes);
        sk.verifying_key().verify(msg.as_bytes(), &sig).unwrap();
        // Deterministic (ed25519 is): same inputs => same signature.
        assert_eq!(sig_hex, sign_entry(&sk, &e, "hone-testnet").unwrap());
        let _ = sk.sign(msg.as_bytes());
    }

    #[test]
    fn mock_chain_enforces_monotonic_nonce() {
        let mut c = MockChain::new(1);
        let sk = ed25519_dalek::SigningKey::from_bytes(&[1u8; 32]);
        let e1 = build_purchase_entry("s1", "settle", "o", "b", 10, 1, 1);
        let sig = sign_entry(&sk, &e1, "hone-testnet").unwrap();
        assert!(matches!(c.submit(&e1, &sig), SubmitOutcome::Sealed { .. }));
        // Re-submitting the SAME nonce => already used.
        assert_eq!(c.submit(&e1, &sig), SubmitOutcome::NonceAlreadyUsed);
        // Skipping ahead => invalid nonce (transient).
        let e3 = build_purchase_entry("s1", "settle", "o", "b", 10, 1, 3);
        let sig3 = sign_entry(&sk, &e3, "hone-testnet").unwrap();
        assert!(matches!(c.submit(&e3, &sig3), SubmitOutcome::Transient(_)));
    }
}
