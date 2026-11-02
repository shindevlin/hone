//! Identity NFTs — names as freely transferable assets with optional social recovery.
//!
//! PROPOSED. Entry points are gated in tx.rs behind
//! `hone_types::IDENTITY_NFT_ACTIVATION_EPOCH`. These apply_* fns carry the business
//! rules and run from `Chain::apply_entry` after signature/nonce/authority checks.
//!
//! Model (docs/hone-identity-model):
//!   A *name* is a domain — freely transferable; the chain never arbitrates identity.
//!   Protection levels (owner's choice):
//!     Open    = transferable, no recovery.           active key transfers.
//!     Guarded = transferable + recovery authority.    active key transfers; guardians
//!               can also recover to the PINNED destination if the key is lost.
//!     Locked  = NOT transferable; only guardian recovery (to the pinned destination)
//!               can move it. Anti-theft.
//!   The invariant: the active key may only TIGHTEN (set first recovery, lock, drop the
//!   founder backstop). LOOSENING (recover, unlock, change guardians/destination,
//!   re-add backstop) needs the recovery authority — enforced in tx.rs. Recovery can
//!   only ever move a name to the destination pinned in advance, so a guardian cannot
//!   redirect it to themselves, and `recovery_seq` stops authorization replay.
//!
//! State:
//!   name_nft:{name}            -> NameNft { name, owner, minted_epoch, transferable,
//!                                           recovery: Option<Recovery>, recovery_seq }
//!   name_owner:{name}          -> owner account (reverse index; auction + gateway compat)
//!   name_held:{account}:{name} -> "1"   (inventory index)
//!   account_name:{account}     -> primary name
//!   name_pending:{name}        -> PendingOffer { from, to, offered_epoch }
//!   identity_registered_count  -> u64   (issuance-phase counter)
#![allow(dead_code)]

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

use crate::chain::Chain;

/// shindevlin — issuer of reserved / trademarked names in every phase.
pub const ISSUER_ACCOUNT: &str = "shindevlin";

/// A pending transfer offer is void after this many epochs (~24h at 30s epochs).
pub const OFFER_EXPIRY_EPOCHS: u64 = 2_880;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Recovery {
    /// Accounts/names that may authorize recovery (M-of-N). A single self-owned
    /// backup wallet is just a 1-of-1 guardian.
    pub guardians: Vec<String>,
    /// Distinct guardian signatures required (M of guardians.len()).
    pub threshold: u8,
    /// PINNED destination: recovery can move the name ONLY here. Set in advance.
    pub destination: String,
    /// While true, a founder 2-of-3 may also recover (removable backstop).
    pub founder_backstop: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NameNft {
    pub name: String,
    pub owner: String,
    pub minted_epoch: u64,
    /// false = Locked (permalock): the active key cannot transfer it.
    pub transferable: bool,
    pub recovery: Option<Recovery>,
    /// Bumps on every loosening action; the signed authorization names the current
    /// value, so an old authorization cannot be replayed.
    #[serde(default)]
    pub recovery_seq: u64,
}

impl NameNft {
    /// Human-readable protection level.
    pub fn level(&self) -> &'static str {
        if !self.transferable { "locked" }
        else if self.recovery.is_some() { "guarded" }
        else { "open" }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingOffer {
    pub from: String,
    pub to: String,
    pub offered_epoch: u64,
}

// ── key helpers ────────────────────────────────────────────────────────────────
fn nft_key(name: &str) -> String { format!("name_nft:{}", name) }
fn owner_key(name: &str) -> String { format!("name_owner:{}", name) }
fn held_key(account: &str, name: &str) -> String { format!("name_held:{}:{}", account, name) }
fn pending_key(name: &str) -> String { format!("name_pending:{}", name) }
fn primary_key(account: &str) -> String { format!("account_name:{}", account) }
const COUNT_KEY: &str = "identity_registered_count";

// ── validation / phase ──────────────────────────────────────────────────────────
pub fn is_valid_identity_name(name: &str) -> bool {
    let n = name.len();
    if n == 0 || n > 63 { return false; }
    if name.starts_with('-') || name.ends_with('-') { return false; }
    name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

pub fn is_reserved(name: &str) -> bool {
    crate::reserved_names::all_reserved().iter().any(|r| r == name)
}

pub fn registered_count(chain: &Chain) -> u64 {
    chain.store.state_get(COUNT_KEY)
        .and_then(|b| String::from_utf8(b).ok())
        .and_then(|s| s.trim().parse::<u64>().ok())
        .unwrap_or(0)
}

fn bump_count(chain: &Chain) -> Result<()> {
    let n = registered_count(chain) + 1;
    chain.store.state_set(COUNT_KEY, n.to_string().as_bytes())?;
    Ok(())
}

pub fn is_open_phase(chain: &Chain) -> bool {
    registered_count(chain) < hone_types::IDENTITY_OPEN_PHASE_LIMIT
}

// ── loaders ──────────────────────────────────────────────────────────────────────
pub fn get(chain: &Chain, name: &str) -> Option<NameNft> {
    chain.store.state_get(&nft_key(name)).and_then(|b| serde_json::from_slice(&b).ok())
}
fn get_pending(chain: &Chain, name: &str) -> Option<PendingOffer> {
    chain.store.state_get(&pending_key(name)).and_then(|b| serde_json::from_slice(&b).ok())
}
fn put(chain: &Chain, nft: &NameNft) -> Result<()> {
    chain.store.state_set(&nft_key(&nft.name), &serde_json::to_vec(nft)?)?;
    chain.store.state_set(&owner_key(&nft.name), nft.owner.as_bytes())?;
    Ok(())
}

/// Validate a proposed recovery config (shape only; authority is checked in tx.rs).
pub fn validate_recovery(guardians: &[String], threshold: u8, destination: &str) -> Result<()> {
    if guardians.is_empty() { bail!("recovery needs at least one guardian"); }
    if guardians.len() > 16 { bail!("too many guardians (max 16)"); }
    let mut seen = std::collections::HashSet::new();
    for g in guardians {
        if g.trim().is_empty() { bail!("empty guardian name"); }
        if !seen.insert(g) { bail!("duplicate guardian '{}'", g); }
    }
    if threshold == 0 { bail!("threshold must be at least 1"); }
    if (threshold as usize) > guardians.len() { bail!("threshold exceeds guardian count"); }
    if destination.trim().is_empty() { bail!("recovery destination must be set"); }
    Ok(())
}

// ── reserved-name migration + name/account control (NAME_NFT_MIGRATION_EPOCH) ──
//
// CONTROL RULE. A name is a NameNft; an account of the same name is controlled by the
// NFT only when it is BOUND (`name_bound:{name}`, set by the migration). For a bound
// account N whose NFT is held by H != N:
//   * N's own key map is ignored. Every signature for N is checked against H's keys:
//     owner/active roles accept only H's owner or active key (never a posting key);
//     the posting role accepts H's posting, active or owner key.
//   * N's balance, nonce and entries are N's; only authority moves with the NFT.
//   * AccountTransfer on N is refused (move the NFT instead). AccountUpdateKey and
//     AccountSetPrimary on N need H's owner/active signature.
//   * IdentityNftTransfer (H's active/owner key) + IdentityNftAccept by the recipient R
//     moves control of N to R atomically, with no key rotation.
//   * To make N self-sovereign, H sets N's own keys (AccountUpdateKey, signed by H) and
//     transfers the NFT to N itself; N's own owner or active key must exist before that
//     accept applies. Once the NFT is held by N, N's own keys apply as for any account.
// Unbound accounts are untouched by any of this.

const MIGRATION_DONE_KEY: &str = "name_migration_done";
fn bound_key(name: &str) -> String { format!("name_bound:{}", name) }

pub fn is_bound(chain: &Chain, name: &str) -> bool {
    chain.store.state_get(&bound_key(name)).is_some()
}

#[cfg(test)]
thread_local! {
    /// Per-test override of NAME_NFT_MIGRATION_EPOCH (tests run on separate threads).
    pub static TEST_MIGRATION_EPOCH: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) };
}

/// The activation epoch for the migration and the control rule.
pub fn migration_epoch() -> u64 {
    #[cfg(test)]
    if let Some(v) = TEST_MIGRATION_EPOCH.with(|c| c.get()) { return v; }
    hone_types::NAME_NFT_MIGRATION_EPOCH
}

/// The account that controls `account` under the control rule at `epoch`, if it is a
/// bound name held by someone else.
pub fn controller(chain: &Chain, account: &str, epoch: u64) -> Option<String> {
    if epoch < migration_epoch() || !is_bound(chain, account) { return None; }
    get(chain, account).map(|n| n.owner).filter(|o| o != account)
}

/// Run the one-time migration if `epoch` has reached activation. Idempotent: a marker
/// records completion, and a re-run after a crash skips rows already migrated.
pub fn migrate_reserved(chain: &Chain, epoch: u64) -> Result<usize> {
    migrate_reserved_with(chain, epoch, crate::reserved_names::SHINDEVLIN_POSTING_KEY)
}

/// Deterministic: walks the compiled reserved list in its fixed order and reads only
/// each account's key map, so every node produces identical rows. A reserved account is
/// migrated iff its keys are exactly {posting: issuer genesis posting key} and the name
/// is not already an NFT. It becomes a NameNft owned by shindevlin (minted_epoch = the
/// activation epoch, not the trigger epoch), is bound, and its own key map is emptied so
/// the genesis posting key no longer speaks for it. The issuance counter is NOT bumped:
/// reserved names do not consume open-phase slots.
pub fn migrate_reserved_with(chain: &Chain, epoch: u64, issuer_posting_key: &str) -> Result<usize> {
    let act = migration_epoch();
    if epoch < act || chain.store.state_get(MIGRATION_DONE_KEY).is_some() { return Ok(0); }
    let mut n = 0usize;
    for name in crate::reserved_names::all_reserved() {
        let Some(mut acct) = chain.store.get_account(&name)? else { continue };
        let keys: std::collections::BTreeMap<String, String> = acct.get("keys")
            .and_then(|k| k.as_object())
            .map(|m| m.iter().filter_map(|(r, v)| v.as_str().filter(|s| !s.is_empty())
                .map(|s| (r.clone(), s.to_owned()))).collect())
            .unwrap_or_default();
        if keys.len() != 1 || keys.get("posting").map(String::as_str) != Some(issuer_posting_key) { continue; }
        if get(chain, &name).is_some() || chain.store.state_get(&owner_key(&name)).is_some() { continue; }
        put(chain, &NameNft {
            name: name.clone(), owner: ISSUER_ACCOUNT.to_string(), minted_epoch: act,
            transferable: true, recovery: None, recovery_seq: 0,
        })?;
        chain.store.state_set(&held_key(ISSUER_ACCOUNT, &name), b"1")?;
        chain.store.state_set(&bound_key(&name), b"1")?;
        acct["keys"] = serde_json::json!({});
        chain.store.set_account(&name, &acct)?;
        n += 1;
    }
    chain.store.state_set(MIGRATION_DONE_KEY, n.to_string().as_bytes())?;
    tracing::info!("identity: reserved-name migration at epoch {}: {} name(s) now NFTs held by {}",
        epoch, n, ISSUER_ACCOUNT);
    Ok(n)
}

// ── apply fns (run after ensure_account + signature/authority + nonce in tx.rs) ──

pub fn apply_mint(chain: &Chain, name: &str, to: &str, minter: &str, epoch: u64) -> Result<()> {
    if !is_valid_identity_name(name) { bail!("'{}' is not a valid identity name", name); }
    if get(chain, name).is_some() || chain.store.state_get(&owner_key(name)).is_some() {
        bail!("name '{}' is already registered", name);
    }
    if chain.store.state_get(&format!("name_auction_active:{}", name)).is_some() {
        bail!("name '{}' is in an active auction", name);
    }
    let issuer = minter == ISSUER_ACCOUNT;
    if to != minter && !issuer { bail!("can only mint a name to your own account"); }
    if is_reserved(name) && !issuer { bail!("'{}' is a reserved name — only {} can issue it", name, ISSUER_ACCOUNT); }
    if !is_open_phase(chain) && !issuer { bail!("name issuance is auction-only now — open a NameAuction for '{}'", name); }
    let nft = NameNft {
        name: name.to_string(), owner: to.to_string(), minted_epoch: epoch,
        transferable: true, recovery: None, recovery_seq: 0,
    };
    put(chain, &nft)?;
    chain.store.state_set(&held_key(to, name), b"1")?;
    bump_count(chain)?;
    Ok(())
}

/// Offer a free (unlocked) name. Ownership does not change until Accept.
pub fn apply_transfer(chain: &Chain, name: &str, from: &str, to: &str, epoch: u64) -> Result<()> {
    let nft = get(chain, name).ok_or_else(|| anyhow::anyhow!("name '{}' is not minted", name))?;
    if nft.owner != from { bail!("'{}' does not own '{}'", from, name); }
    if !nft.transferable { bail!("'{}' is locked and can only move via recovery", name); }
    if from == to { bail!("cannot transfer a name to its current owner"); }
    let offer = PendingOffer { from: from.to_string(), to: to.to_string(), offered_epoch: epoch };
    chain.store.state_set(&pending_key(name), &serde_json::to_vec(&offer)?)?;
    Ok(())
}

/// Recipient accepts a pending offer; ownership moves (a holder change → Stage 3 strips
/// holder-scoped credentials here).
pub fn apply_accept(chain: &Chain, name: &str, to: &str, epoch: u64) -> Result<()> {
    let offer = get_pending(chain, name).ok_or_else(|| anyhow::anyhow!("no pending offer for '{}'", name))?;
    if offer.to != to { bail!("'{}' is not the offered recipient of '{}'", to, name); }
    if epoch > offer.offered_epoch.saturating_add(OFFER_EXPIRY_EPOCHS) {
        chain.store.state_delete(&pending_key(name))?;
        bail!("the transfer offer for '{}' has expired", name);
    }
    let mut nft = get(chain, name).ok_or_else(|| anyhow::anyhow!("name '{}' is not minted", name))?;
    if !nft.transferable {
        chain.store.state_delete(&pending_key(name))?;
        bail!("'{}' became locked after the offer — transfer is blocked", name);
    }
    if nft.owner != offer.from {
        chain.store.state_delete(&pending_key(name))?;
        bail!("offer for '{}' is stale — owner changed", name);
    }
    // A bound account taking its own NFT must already have its own value key, or the
    // account would end up keyless (and keyless accounts bootstrap unsigned).
    if to == name && is_bound(chain, name) && epoch >= migration_epoch() {
        let acct = chain.store.get_account(name)?.unwrap_or_default();
        let has = |r: &str| acct.get("keys").and_then(|k| k.get(r)).and_then(|v| v.as_str())
            .map(|v| !v.is_empty()).unwrap_or(false);
        if !has("owner") && !has("active") {
            bail!("'{}' must have its own owner or active key before it can hold its own name", name);
        }
    }
    let from = offer.from.clone();
    chain.store.state_delete(&held_key(&from, name))?;
    nft.owner = to.to_string();
    // A sale/gift is a holder change: recovery config does not carry to a new holder.
    nft.recovery = None;
    nft.transferable = true;
    put(chain, &nft)?;
    chain.store.state_set(&held_key(to, name), b"1")?;
    if chain.store.state_get(&primary_key(&from)).as_deref() == Some(name.as_bytes()) {
        chain.store.state_delete(&primary_key(&from))?;
    }
    chain.store.state_delete(&pending_key(name))?;
    // A holder change strips holder-scoped credentials (kept as provenance); provenance
    // tokens persist. Recovery (same holder) does not strip.
    crate::soulbound::strip_holder_scoped(chain, name)?;
    Ok(())
}

pub fn apply_reject(chain: &Chain, name: &str, signed_by: &str) -> Result<()> {
    let offer = get_pending(chain, name).ok_or_else(|| anyhow::anyhow!("no pending offer for '{}'", name))?;
    if signed_by != offer.to && signed_by != offer.from {
        bail!("'{}' is not a party to the offer for '{}'", signed_by, name);
    }
    chain.store.state_delete(&pending_key(name))?;
    Ok(())
}

pub fn apply_set_primary(chain: &Chain, account: &str, name: &str) -> Result<()> {
    let nft = get(chain, name).ok_or_else(|| anyhow::anyhow!("name '{}' is not minted", name))?;
    if nft.owner != account { bail!("'{}' does not own '{}'", account, name); }
    chain.store.state_set(&primary_key(account), name.as_bytes())?;
    Ok(())
}

/// Set / change the recovery config. `first_set` (recovery was None) is a tightening
/// action (active key, checked in tx.rs); a change is loosening (recovery authority).
/// `expected_seq` must equal the current recovery_seq on a change.
pub fn apply_set_recovery(chain: &Chain, name: &str, caller: &str,
    guardians: Vec<String>, threshold: u8, destination: &str, founder_backstop: bool) -> Result<()> {
    let mut nft = get(chain, name).ok_or_else(|| anyhow::anyhow!("name '{}' is not minted", name))?;
    if nft.owner != caller { bail!("'{}' does not own '{}'", caller, name); }
    // Recovery config is owner-controlled and may change only while the name is UNLOCKED.
    // Once locked the config is frozen: guardians can recover (to the pinned destination)
    // or unlock, but no one can re-point the destination — so guardians (even a colluding
    // quorum) can never redirect a locked name to themselves.
    if !nft.transferable { bail!("unlock '{}' before changing its recovery configuration", name); }
    validate_recovery(&guardians, threshold, destination)?;
    nft.recovery = Some(Recovery { guardians, threshold, destination: destination.to_string(), founder_backstop });
    put(chain, &nft)?;
    Ok(())
}

/// Lock a name (Guarded → Locked). Tightening (active key). Requires a recovery path
/// so the name cannot be bricked.
pub fn apply_lock(chain: &Chain, name: &str, caller: &str) -> Result<()> {
    let mut nft = get(chain, name).ok_or_else(|| anyhow::anyhow!("name '{}' is not minted", name))?;
    if nft.owner != caller { bail!("'{}' does not own '{}'", caller, name); }
    if nft.recovery.is_none() { bail!("set a recovery path before locking '{}' (else it cannot be recovered)", name); }
    nft.transferable = false;
    put(chain, &nft)?;
    Ok(())
}

/// Unlock a name (Locked → Guarded). Loosening: recovery authority (checked in tx.rs).
pub fn apply_unlock(chain: &Chain, name: &str, expected_seq: u64) -> Result<()> {
    let mut nft = get(chain, name).ok_or_else(|| anyhow::anyhow!("name '{}' is not minted", name))?;
    if nft.transferable { bail!("'{}' is not locked", name); }
    if expected_seq != nft.recovery_seq { bail!("stale recovery_seq for '{}'", name); }
    nft.transferable = true;
    nft.recovery_seq = nft.recovery_seq.wrapping_add(1);
    put(chain, &nft)?;
    Ok(())
}

/// Execute recovery: move the name to its PINNED destination. Loosening: recovery
/// authority (checked in tx.rs). Works regardless of lock state. Credentials survive
/// (Stage 3). No arbitrary destination — only the pre-pinned one.
pub fn apply_recover(chain: &Chain, name: &str, expected_seq: u64) -> Result<()> {
    let mut nft = get(chain, name).ok_or_else(|| anyhow::anyhow!("name '{}' is not minted", name))?;
    let rec = nft.recovery.clone().ok_or_else(|| anyhow::anyhow!("'{}' has no recovery configured", name))?;
    if expected_seq != nft.recovery_seq { bail!("stale recovery_seq for '{}'", name); }
    let dest = rec.destination.clone();
    let from = nft.owner.clone();
    if dest == from {
        // already there; just bump seq to consume the authorization
        nft.recovery_seq = nft.recovery_seq.wrapping_add(1);
        put(chain, &nft)?;
        return Ok(());
    }
    chain.store.state_delete(&held_key(&from, name))?;
    nft.owner = dest.clone();
    nft.recovery_seq = nft.recovery_seq.wrapping_add(1);
    put(chain, &nft)?;
    chain.store.state_set(&held_key(&dest, name), b"1")?;
    if chain.store.state_get(&primary_key(&from)).as_deref() == Some(name.as_bytes()) {
        chain.store.state_delete(&primary_key(&from))?;
    }
    chain.store.state_delete(&pending_key(name))?;
    Ok(())
}

/// Toggle the founder backstop. Disabling = tightening (active key). Enabling =
/// loosening (recovery authority). Checked in tx.rs.
pub fn apply_set_backstop(chain: &Chain, name: &str, caller: &str, enabled: bool) -> Result<()> {
    let mut nft = get(chain, name).ok_or_else(|| anyhow::anyhow!("name '{}' is not minted", name))?;
    if nft.owner != caller { bail!("'{}' does not own '{}'", caller, name); }
    let mut rec = nft.recovery.clone().ok_or_else(|| anyhow::anyhow!("'{}' has no recovery configured", name))?;
    // Dropping the backstop is a tightening, allowed any time; re-enabling it is a
    // loosening, allowed only while unlocked (owner-controlled config).
    if enabled && !nft.transferable { bail!("unlock '{}' before re-enabling the founder backstop", name); }
    rec.founder_backstop = enabled;
    nft.recovery = Some(rec);
    put(chain, &nft)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_chain() -> (Chain, tempfile::TempDir) {
        let dir = tempfile::Builder::new().prefix("hone_idnft_").tempdir().unwrap();
        let store = crate::store::Store::open(dir.path()).unwrap();
        let chain = Chain::new(store, "node".to_string(), "hone-test".to_string());
        (chain, dir)
    }
    fn gl(v: &[&str]) -> Vec<String> { v.iter().map(|s| s.to_string()).collect() }

    #[test]
    fn mint_is_open_and_transferable() {
        let (c, _d) = make_chain();
        apply_mint(&c, "satoshi0", "alice", "alice", 1).unwrap();
        let n = get(&c, "satoshi0").unwrap();
        assert_eq!(n.owner, "alice"); assert!(n.transferable); assert!(n.recovery.is_none());
        assert_eq!(n.level(), "open");
    }

    #[test]
    fn transfer_pending_until_accept_and_resets_recovery() {
        let (c, _d) = make_chain();
        apply_mint(&c, "coolname", "alice", "alice", 1).unwrap();
        apply_set_recovery(&c, "coolname", "alice", gl(&["backup"]), 1, "backup", true).unwrap();
        apply_transfer(&c, "coolname", "alice", "bob", 2).unwrap();
        assert_eq!(get(&c, "coolname").unwrap().owner, "alice");
        apply_accept(&c, "coolname", "bob", 3).unwrap();
        let n = get(&c, "coolname").unwrap();
        assert_eq!(n.owner, "bob");
        assert!(n.recovery.is_none(), "recovery config must not carry to a new holder");
        assert!(n.transferable);
    }

    #[test]
    fn locked_name_cannot_be_transferred() {
        let (c, _d) = make_chain();
        apply_mint(&c, "coolname", "alice", "alice", 1).unwrap();
        // cannot lock without recovery
        assert!(apply_lock(&c, "coolname", "alice").is_err());
        apply_set_recovery(&c, "coolname", "alice", gl(&["backup"]), 1, "backup", false).unwrap();
        apply_lock(&c, "coolname", "alice").unwrap();
        assert_eq!(get(&c, "coolname").unwrap().level(), "locked");
        assert!(apply_transfer(&c, "coolname", "alice", "bob", 2).is_err());
    }

    #[test]
    fn recover_only_to_pinned_destination() {
        let (c, _d) = make_chain();
        apply_mint(&c, "coolname", "alice", "alice", 1).unwrap();
        apply_set_recovery(&c, "coolname", "alice", gl(&["g1", "g2", "g3"]), 2, "alice-backup", true).unwrap();
        apply_lock(&c, "coolname", "alice").unwrap();
        let seq = get(&c, "coolname").unwrap().recovery_seq;
        apply_recover(&c, "coolname", seq).unwrap();
        let n = get(&c, "coolname").unwrap();
        assert_eq!(n.owner, "alice-backup", "recovery moves only to the pinned destination");
        assert_eq!(n.recovery_seq, seq + 1, "seq bumps to stop replay");
        // replay with the old seq fails
        assert!(apply_recover(&c, "coolname", seq).is_err());
    }

    #[test]
    fn set_recovery_rejects_bad_config() {
        let (c, _d) = make_chain();
        apply_mint(&c, "coolname", "alice", "alice", 1).unwrap();
        assert!(apply_set_recovery(&c, "coolname", "alice", gl(&[]), 1, "d", false).is_err());
        assert!(apply_set_recovery(&c, "coolname", "alice", gl(&["a", "b"]), 3, "d", false).is_err());
        assert!(apply_set_recovery(&c, "coolname", "alice", gl(&["a", "a"]), 1, "d", false).is_err());
        assert!(apply_set_recovery(&c, "coolname", "alice", gl(&["a"]), 1, "", false).is_err());
    }

    #[test]
    fn recovery_config_frozen_while_locked() {
        let (c, _d) = make_chain();
        apply_mint(&c, "coolname", "alice", "alice", 1).unwrap();
        apply_set_recovery(&c, "coolname", "alice", gl(&["g1"]), 1, "d1", false).unwrap();
        // while unlocked the owner may reconfigure freely
        apply_set_recovery(&c, "coolname", "alice", gl(&["g2"]), 1, "d2", false).unwrap();
        assert_eq!(get(&c, "coolname").unwrap().recovery.unwrap().destination, "d2");
        apply_lock(&c, "coolname", "alice").unwrap();
        // once locked the destination/guardians are frozen — no redirect possible
        assert!(apply_set_recovery(&c, "coolname", "alice", gl(&["attacker"]), 1, "attacker", false).is_err());
        assert_eq!(get(&c, "coolname").unwrap().recovery.unwrap().destination, "d2");
    }

    #[test]
    fn set_primary_requires_ownership() {
        let (c, _d) = make_chain();
        apply_mint(&c, "coolname", "alice", "alice", 1).unwrap();
        assert!(apply_set_primary(&c, "bob", "coolname").is_err());
        apply_set_primary(&c, "alice", "coolname").unwrap();
        assert_eq!(c.store.state_get("account_name:alice").unwrap(), b"coolname");
    }

    #[test]
    fn reserved_only_issuer() {
        let (c, _d) = make_chain();
        assert!(apply_mint(&c, "a", "alice", "alice", 1).is_err());
        apply_mint(&c, "a", "buyer", ISSUER_ACCOUNT, 1).unwrap();
        assert_eq!(get(&c, "a").unwrap().owner, "buyer");
    }
}
