//! VRF (Verifiable Random Function) beacon.
//!
//! Each epoch, clock nodes commit a SHA-256 of (secret || epoch), then reveal
//! the secret. The beacon value = XOR of all valid reveals, providing an
//! unbiasable random beacon that can be used for deterministic selection
//! (validator sets, challenge assignments, etc.).
//!
//! Commit window: first half of the epoch (≤ COMMIT_WINDOW_EPOCHS before deadline).
//! Reveal window: second half (COMMIT_WINDOW_EPOCHS after deadline + reveal window).
//! Non-revealing committers lose their commit stake deposit.
#![allow(dead_code)]

#![allow(dead_code)]
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use tracing::info;

use hone_types::{LedgerEntry, NATIVE_TOKEN, RECYCLE_FUND_ACCOUNT};
use crate::chain::Chain;

const COMMIT_DEPOSIT_HUNITS: u64 = 10_000; // 0.0001 HONE — ensures reveals
const REVEAL_WINDOW_EPOCHS: u64 = 2;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VrfRound {
    pub epoch: u64,
    pub commits: Vec<VrfCommitRecord>,
    pub beacon: Option<String>, // hex of XOR beacon once finalised
    pub finalised: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VrfCommitRecord {
    pub committer: String,
    pub commit_hash: String, // sha256(secret || epoch_be8)
    pub revealed_secret: Option<String>, // hex
    pub deposit_refunded: bool,
}

fn round_key(epoch: u64) -> String { format!("vrf_round:{}", epoch) }

pub fn apply_commit(chain: &Chain, entry: &LedgerEntry) -> Result<()> {
    let LedgerEntry::VrfCommit { committer, commit_hash, epoch, .. } = entry
        else { bail!("wrong entry type") };

    // Ensure committer is a registered clock node.
    let clock_reg_key = format!("clock_node:{}", committer);
    anyhow::ensure!(
        chain.store.state_get(&clock_reg_key).is_some(),
        "committer '{}' is not a registered clock node", committer
    );

    let mut round: VrfRound = chain.store.state_get(&round_key(*epoch))
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or(VrfRound { epoch: *epoch, commits: vec![], beacon: None, finalised: false });

    anyhow::ensure!(!round.finalised, "VRF round {} already finalised", epoch);

    if round.commits.iter().any(|c| &c.committer == committer) {
        bail!("'{}' already committed for epoch {}", committer, epoch);
    }

    // Lock deposit.
    chain.store.debit(committer, NATIVE_TOKEN, COMMIT_DEPOSIT_HUNITS)?;

    round.commits.push(VrfCommitRecord {
        committer: committer.clone(),
        commit_hash: commit_hash.clone(),
        revealed_secret: None,
        deposit_refunded: false,
    });
    chain.store.state_set(&round_key(*epoch), &serde_json::to_vec(&round)?)?;
    Ok(())
}

pub fn apply_reveal(chain: &Chain, entry: &LedgerEntry) -> Result<()> {
    let LedgerEntry::VrfReveal { committer, secret_hex, epoch, .. } = entry
        else { bail!("wrong entry type") };

    let raw = chain.store.state_get(&round_key(*epoch))
        .ok_or_else(|| anyhow::anyhow!("VRF round {} not found", epoch))?;
    let mut round: VrfRound = serde_json::from_slice(&raw)?;

    let commit = round.commits.iter_mut()
        .find(|c| &c.committer == committer)
        .ok_or_else(|| anyhow::anyhow!("no commit found for '{}' in epoch {}", committer, epoch))?;

    anyhow::ensure!(commit.revealed_secret.is_none(), "'{}' already revealed", committer);

    // Verify: sha256(secret || epoch_be8) == commit_hash.
    let secret_bytes = hex::decode(secret_hex)
        .map_err(|_| anyhow::anyhow!("invalid secret hex"))?;
    let mut hasher = Sha256::new();
    hasher.update(&secret_bytes);
    hasher.update(epoch.to_be_bytes());
    let expected = hex::encode(hasher.finalize());

    anyhow::ensure!(expected == commit.commit_hash, "reveal does not match commit for '{}'", committer);

    commit.revealed_secret = Some(secret_hex.clone());
    commit.deposit_refunded = true;
    chain.store.credit(committer, NATIVE_TOKEN, COMMIT_DEPOSIT_HUNITS)?;

    // Check if all commits have revealed → auto-finalise.
    let all_revealed = round.commits.iter().all(|c| c.revealed_secret.is_some());
    if all_revealed {
        finalise_round(chain, &mut round, *epoch)?;
    }

    chain.store.state_set(&round_key(*epoch), &serde_json::to_vec(&round)?)?;
    Ok(())
}

fn finalise_round(chain: &Chain, round: &mut VrfRound, epoch: u64) -> Result<()> {
    let mut beacon = [0u8; 32];
    for commit in &round.commits {
        if let Some(secret_hex) = &commit.revealed_secret {
            if let Ok(bytes) = hex::decode(secret_hex) {
                let hash = Sha256::digest(&bytes);
                for (b, h) in beacon.iter_mut().zip(hash.iter()) {
                    *b ^= h;
                }
            }
        }
    }
    let beacon_hex = hex::encode(beacon);
    round.beacon = Some(beacon_hex.clone());
    round.finalised = true;

    // Store latest beacon for easy lookup.
    chain.store.state_set(
        "vrf:latest_beacon",
        &serde_json::to_vec(&serde_json::json!({ "beacon": beacon_hex, "epoch": epoch }))
            .unwrap_or_default(),
    )?;
    info!("[vrf] epoch {} beacon finalised: {}", epoch, &beacon_hex[..16]);
    Ok(())
}

/// At epoch seal: slash non-revelaers (keep deposit → recycle), then auto-finalise
/// any round that hasn't closed yet.
pub fn sweep_epoch(chain: &Chain, epoch: u64) {
    // Slash previous epoch's non-revealers (reveal window = 2 epochs after commit).
    let slash_epoch = epoch.saturating_sub(REVEAL_WINDOW_EPOCHS);
    if let Some(raw) = chain.store.state_get(&round_key(slash_epoch)) {
        if let Ok(mut round) = serde_json::from_slice::<VrfRound>(&raw) {
            if !round.finalised {
                for commit in round.commits.iter_mut().filter(|c| !c.deposit_refunded) {
                    // Deposit already deducted — send to recycle.
                    let _ = chain.store.credit(RECYCLE_FUND_ACCOUNT, NATIVE_TOKEN, COMMIT_DEPOSIT_HUNITS);
                    commit.deposit_refunded = true;
                }
                // Finalise with whatever was revealed.
                let _ = finalise_round(chain, &mut round, slash_epoch);
                let _ = chain.store.state_set(
                    &round_key(slash_epoch),
                    &serde_json::to_vec(&round).unwrap_or_default(),
                );
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use hone_types::LedgerEntry;
    use tempfile::TempDir;

    fn make_chain() -> (Chain, TempDir) {
        let dir = tempfile::TempDir::new().unwrap();
        let store = crate::store::Store::open(dir.path()).unwrap();
        let chain = Chain::new(
            store, "test".to_string(), "hone-test".to_string(),
        );
        (chain, dir)
    }

    fn fund(chain: &Chain, account: &str, amount: u64) {
        chain.apply_entry(&LedgerEntry::AccountCreate {
            account: account.to_string(),
            keys: Default::default(),
            chain_proofs: vec![],
            epoch: 0,
            funded_by: None,
            machine_fingerprint: None,
        }).ok();
        chain.apply_entry(&LedgerEntry::GenesisAlloc {
            account: account.to_string(),
            amount,
            token: hone_types::NATIVE_TOKEN.to_string(),
        }).unwrap();
    }

    /// Register `account` as a clock node by writing the clock_node key directly.
    fn register_clock(chain: &Chain, account: &str) {
        chain.store.state_set(
            &format!("clock_node:{}", account),
            b"registered",
        ).unwrap();
    }

    /// Compute commit_hash = sha256(secret_bytes || epoch_be8)
    fn vrf_commit_hash(secret_hex: &str, epoch: u64) -> String {
        let secret_bytes = hex::decode(secret_hex).unwrap();
        let mut hasher = Sha256::new();
        hasher.update(&secret_bytes);
        hasher.update(epoch.to_be_bytes());
        hex::encode(hasher.finalize())
    }

    #[test]
    fn commit_stores_commitment() {
        let (chain, _dir) = make_chain();
        fund(&chain, "alice", COMMIT_DEPOSIT_HUNITS * 10);
        register_clock(&chain, "alice");

        let secret_hex = hex::encode([0xab; 32]);
        let epoch: u64 = 5;
        let commit_hash = vrf_commit_hash(&secret_hex, epoch);

        let entry = LedgerEntry::VrfCommit {
            committer: "alice".to_string(),
            commit_hash: commit_hash.clone(),
            epoch,
            nonce: 1,
            signed_by: "alice".to_string(),
            signature: None,
        };
        apply_commit(&chain, &entry).unwrap();

        let raw = chain.store.state_get(&round_key(epoch)).unwrap();
        let round: VrfRound = serde_json::from_slice(&raw).unwrap();
        assert_eq!(round.commits.len(), 1);
        assert_eq!(round.commits[0].commit_hash, commit_hash);
        assert_eq!(round.commits[0].committer, "alice");
    }

    #[test]
    fn reveal_with_correct_preimage_succeeds() {
        let (chain, _dir) = make_chain();
        fund(&chain, "alice", COMMIT_DEPOSIT_HUNITS * 10);
        register_clock(&chain, "alice");

        let secret_hex = hex::encode([0xcd; 32]);
        let epoch: u64 = 7;
        let commit_hash = vrf_commit_hash(&secret_hex, epoch);

        apply_commit(&chain, &LedgerEntry::VrfCommit {
            committer: "alice".to_string(),
            commit_hash,
            epoch,
            nonce: 1,
            signed_by: "alice".to_string(),
            signature: None,
        }).unwrap();

        let result = apply_reveal(&chain, &LedgerEntry::VrfReveal {
            committer: "alice".to_string(),
            secret_hex: secret_hex.clone(),
            epoch,
            nonce: 2,
            signed_by: "alice".to_string(),
            signature: None,
        });
        assert!(result.is_ok(), "correct reveal should succeed: {:?}", result);

        let raw = chain.store.state_get(&round_key(epoch)).unwrap();
        let round: VrfRound = serde_json::from_slice(&raw).unwrap();
        assert_eq!(round.commits[0].revealed_secret, Some(secret_hex));
    }

    #[test]
    fn reveal_with_wrong_preimage_fails() {
        let (chain, _dir) = make_chain();
        fund(&chain, "alice", COMMIT_DEPOSIT_HUNITS * 10);
        register_clock(&chain, "alice");

        let epoch: u64 = 8;
        let correct_secret = hex::encode([0xef; 32]);
        let wrong_secret = hex::encode([0x00; 32]);
        let commit_hash = vrf_commit_hash(&correct_secret, epoch);

        apply_commit(&chain, &LedgerEntry::VrfCommit {
            committer: "alice".to_string(),
            commit_hash,
            epoch,
            nonce: 1,
            signed_by: "alice".to_string(),
            signature: None,
        }).unwrap();

        let result = apply_reveal(&chain, &LedgerEntry::VrfReveal {
            committer: "alice".to_string(),
            secret_hex: wrong_secret,
            epoch,
            nonce: 2,
            signed_by: "alice".to_string(),
            signature: None,
        });
        assert!(result.is_err(), "wrong preimage should fail");
    }

    #[test]
    fn double_commit_fails() {
        let (chain, _dir) = make_chain();
        fund(&chain, "alice", COMMIT_DEPOSIT_HUNITS * 10);
        register_clock(&chain, "alice");

        let epoch: u64 = 9;
        let secret_hex = hex::encode([0x11; 32]);
        let commit_hash = vrf_commit_hash(&secret_hex, epoch);

        let entry = LedgerEntry::VrfCommit {
            committer: "alice".to_string(),
            commit_hash: commit_hash.clone(),
            epoch,
            nonce: 1,
            signed_by: "alice".to_string(),
            signature: None,
        };
        apply_commit(&chain, &entry).unwrap();

        // Second commit from same committer same epoch
        let entry2 = LedgerEntry::VrfCommit {
            committer: "alice".to_string(),
            commit_hash,
            epoch,
            nonce: 2,
            signed_by: "alice".to_string(),
            signature: None,
        };
        assert!(apply_commit(&chain, &entry2).is_err(), "double commit should fail");
    }

    #[test]
    fn reveal_before_commit_fails() {
        let (chain, _dir) = make_chain();
        fund(&chain, "alice", COMMIT_DEPOSIT_HUNITS * 10);
        register_clock(&chain, "alice");

        let epoch: u64 = 10;
        // No commit for this epoch at all
        let result = apply_reveal(&chain, &LedgerEntry::VrfReveal {
            committer: "alice".to_string(),
            secret_hex: hex::encode([0x22; 32]),
            epoch,
            nonce: 1,
            signed_by: "alice".to_string(),
            signature: None,
        });
        assert!(result.is_err(), "reveal without prior commit should fail");
    }
}

/// Derive a deterministic value from the latest beacon (for validator selection etc.).
pub fn derive_from_beacon(chain: &Chain, domain: &str) -> Option<[u8; 32]> {
    let raw = chain.store.state_get("vrf:latest_beacon")?;
    let j: serde_json::Value = serde_json::from_slice(&raw).ok()?;
    let beacon_hex = j["beacon"].as_str()?;
    let beacon = hex::decode(beacon_hex).ok()?;
    let mut hasher = Sha256::new();
    hasher.update(&beacon);
    hasher.update(domain.as_bytes());
    Some(hasher.finalize().into())
}
