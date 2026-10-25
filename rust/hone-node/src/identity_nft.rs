//! Identity NFTs â€” names as transferable, soulbound-capable assets.
//!
//! PROPOSED. The entry points are gated in tx.rs behind
//! `hone_types::IDENTITY_NFT_ACTIVATION_EPOCH`; these apply_* functions carry the
//! business rules and are called from `Chain::apply_entry` after the usual
//! `ensure_account` / signature / nonce checks.
//!
//! A *name* is the first-class asset ("the identity NFT"). It is held by a keypair
//! account and reuses the name layer the auction engine already writes:
//!   name_owner:{name}        -> owner account   (fast reverse index; auction + gateway)
//!   account_name:{account}   -> that account's PRIMARY name
//! plus the NFT-specific state:
//!   name_nft:{name}          -> NameNft { name, owner, minted_epoch, soulbound }
//!   name_held:{account}:{name} -> "1"   (inventory index; prefix-scannable)
//!   name_pending:{name}      -> PendingOffer { from, to, offered_epoch }
//!   identity_registered_count -> u64    (issuance-phase counter)
//!
//! Security invariants (see tests + docs/IDENTITY_NFT.md):
//!   * Transfer moves value â†’ the ACTIVE key signs it (enforced in tx.rs, no posting
//!     fallback). This module assumes that check already passed.
//!   * Ownership never changes on Transfer alone â€” only on Accept by the offered
//!     recipient (permissive receipt). No account can be force-fed a name.
//!   * A soulbound name cannot be transferred. Permalock can be lifted only by a
//!     founder 2-of-3 multisig (membership + distinctness enforced here; the second
//!     signature is verified in tx.rs).
#![allow(dead_code)]

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};

use crate::chain::Chain;

/// shindevlin â€” issuer of reserved / trademarked names in every phase.
pub const ISSUER_ACCOUNT: &str = "shindevlin";

/// A pending transfer offer is void after this many epochs (~24h at 30s epochs),
/// so a stale offer cannot be redeemed long after the owner's intent changed.
pub const OFFER_EXPIRY_EPOCHS: u64 = 2_880;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Soulbound {
    #[default]
    None,
    /// Owner can clear it later (accidental-transfer guard).
    Soft,
    /// Clears only via founder 2-of-3 multisig.
    Permalock,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NameNft {
    pub name: String,
    pub owner: String,
    pub minted_epoch: u64,
    #[serde(default)]
    pub soulbound: Soulbound,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PendingOffer {
    pub from: String,
    pub to: String,
    pub offered_epoch: u64,
}

// â”€â”€ key helpers â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
fn nft_key(name: &str) -> String { format!("name_nft:{}", name) }
fn owner_key(name: &str) -> String { format!("name_owner:{}", name) }
fn held_key(account: &str, name: &str) -> String { format!("name_held:{}:{}", account, name) }
fn pending_key(name: &str) -> String { format!("name_pending:{}", name) }
fn primary_key(account: &str) -> String { format!("account_name:{}", account) }
const COUNT_KEY: &str = "identity_registered_count";

// â”€â”€ validation / phase â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€

/// A valid identity name: 1â€“63 chars, lowercase ascii letters/digits/hyphen,
/// no leading/trailing hyphen. (Excludes ':' '/' '.' '@' used by other namespaces.)
pub fn is_valid_identity_name(name: &str) -> bool {
    let n = name.len();
    if n == 0 || n > 63 { return false; }
    if name.starts_with('-') || name.ends_with('-') { return false; }
    name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

/// Reserved names (singles, doubles, popular first names) are pre-held by shindevlin
/// and may only be issued by shindevlin.
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

/// Open-mint phase: before the registration limit, non-reserved names may be
/// freely minted. After, new names must go through NameAuction.
pub fn is_open_phase(chain: &Chain) -> bool {
    registered_count(chain) < hone_types::IDENTITY_OPEN_PHASE_LIMIT
}

fn is_founder(account: &str) -> bool {
    hone_types::IDENTITY_FOUNDER_ACCOUNTS.contains(&account)
}

// â”€â”€ loaders â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€â”€
pub fn get(chain: &Chain, name: &str) -> Option<NameNft> {
    chain.store.state_get(&nft_key(name))
        .and_then(|b| serde_json::from_slice(&b).ok())
}

fn get_pending(chain: &Chain, name: &str) -> Option<PendingOffer> {
    chain.store.state_get(&pending_key(name))
        .and_then(|b| serde_json::from_slice(&b).ok())
}

fn put(chain: &Chain, nft: &NameNft) -> Result<()> {
    chain.store.state_set(&nft_key(&nft.name), &serde_json::to_vec(nft)?)?;
    chain.store.state_set(&owner_key(&nft.name), nft.owner.as_bytes())?;
    Ok(())
}

// â”€â”€ apply fns (called from Chain::apply_entry after ensure_account + sig + nonce) â”€â”€

/// Mint a fresh name NFT to `to`. `minter` is the signer.
pub fn apply_mint(chain: &Chain, name: &str, to: &str, minter: &str, epoch: u64) -> Result<()> {
    if !is_valid_identity_name(name) {
        bail!("'{}' is not a valid identity name", name);
    }
    if get(chain, name).is_some() || chain.store.state_get(&owner_key(name)).is_some() {
        bail!("name '{}' is already registered", name);
    }
    if chain.store.state_get(&format!("name_auction_active:{}", name)).is_some() {
        bail!("name '{}' is in an active auction", name);
    }
    let issuer = minter == ISSUER_ACCOUNT;
    // Only the issuer may put a name into an account other than the signer's own.
    if to != minter && !issuer {
        bail!("can only mint a name to your own account");
    }
    if is_reserved(name) && !issuer {
        bail!("'{}' is a reserved name â€” only {} can issue it", name, ISSUER_ACCOUNT);
    }
    if !is_open_phase(chain) && !issuer {
        bail!("name issuance is auction-only now â€” open a NameAuction for '{}'", name);
    }
    let nft = NameNft { name: name.to_string(), owner: to.to_string(), minted_epoch: epoch, soulbound: Soulbound::None };
    put(chain, &nft)?;
    chain.store.state_set(&held_key(to, name), b"1")?;
    bump_count(chain)?;
    Ok(())
}

/// Offer `name` from `from` to `to`. Ownership does NOT change yet (permissive receipt).
pub fn apply_transfer(chain: &Chain, name: &str, from: &str, to: &str, epoch: u64) -> Result<()> {
    let nft = get(chain, name).ok_or_else(|| anyhow::anyhow!("name '{}' is not minted", name))?;
    if nft.owner != from {
        bail!("'{}' does not own '{}'", from, name);
    }
    if nft.soulbound != Soulbound::None {
        bail!("'{}' is soulbound and cannot be transferred", name);
    }
    if from == to {
        bail!("cannot transfer a name to its current owner");
    }
    // ensure the recipient is a real account (created by the caller's ensure_account)
    let offer = PendingOffer { from: from.to_string(), to: to.to_string(), offered_epoch: epoch };
    chain.store.state_set(&pending_key(name), &serde_json::to_vec(&offer)?)?;
    Ok(())
}

/// Recipient accepts a pending offer; ownership moves now.
pub fn apply_accept(chain: &Chain, name: &str, to: &str, epoch: u64) -> Result<()> {
    let offer = get_pending(chain, name).ok_or_else(|| anyhow::anyhow!("no pending offer for '{}'", name))?;
    if offer.to != to {
        bail!("'{}' is not the offered recipient of '{}'", to, name);
    }
    if epoch > offer.offered_epoch.saturating_add(OFFER_EXPIRY_EPOCHS) {
        chain.store.state_delete(&pending_key(name))?;
        bail!("the transfer offer for '{}' has expired", name);
    }
    let mut nft = get(chain, name).ok_or_else(|| anyhow::anyhow!("name '{}' is not minted", name))?;
    // A name locked (soft or permalock) AFTER the offer must not move via a stale accept.
    if nft.soulbound != Soulbound::None {
        chain.store.state_delete(&pending_key(name))?;
        bail!("'{}' became soulbound after the offer â€” transfer is blocked", name);
    }
    // Owner must still be the offerer (offer is void if ownership changed underneath).
    if nft.owner != offer.from {
        chain.store.state_delete(&pending_key(name))?;
        bail!("offer for '{}' is stale â€” owner changed", name);
    }
    let from = offer.from.clone();
    chain.store.state_delete(&held_key(&from, name))?;
    nft.owner = to.to_string();
    put(chain, &nft)?;
    chain.store.state_set(&held_key(to, name), b"1")?;
    // If it was the sender's primary, clear that pointer.
    if chain.store.state_get(&primary_key(&from)).as_deref() == Some(name.as_bytes()) {
        chain.store.state_delete(&primary_key(&from))?;
    }
    chain.store.state_delete(&pending_key(name))?;
    Ok(())
}

/// Reject / withdraw a pending offer. Either party may do it. Ownership unchanged.
pub fn apply_reject(chain: &Chain, name: &str, signed_by: &str) -> Result<()> {
    let offer = get_pending(chain, name).ok_or_else(|| anyhow::anyhow!("no pending offer for '{}'", name))?;
    if signed_by != offer.to && signed_by != offer.from {
        bail!("'{}' is not a party to the offer for '{}'", signed_by, name);
    }
    chain.store.state_delete(&pending_key(name))?;
    Ok(())
}

/// Set soulbound mode. `owner` must own the name. Permalock cannot be set over an
/// existing permalock, and set never *removes* a lock (use unset).
pub fn apply_set_soulbound(chain: &Chain, name: &str, owner: &str, mode: &str) -> Result<()> {
    let mut nft = get(chain, name).ok_or_else(|| anyhow::anyhow!("name '{}' is not minted", name))?;
    if nft.owner != owner {
        bail!("'{}' does not own '{}'", owner, name);
    }
    if nft.soulbound == Soulbound::Permalock {
        bail!("'{}' is permalocked and cannot be reconfigured", name);
    }
    nft.soulbound = match mode {
        "soft" => Soulbound::Soft,
        "permalock" => Soulbound::Permalock,
        other => bail!("unknown soulbound mode '{}'", other),
    };
    put(chain, &nft)?;
    // Locking a name voids any outstanding transfer offer.
    let _ = chain.store.state_delete(&pending_key(name));
    Ok(())
}

/// Remove soulbound. Soft: the owner (`signer`) clears it. Permalock: founder 2-of-3
/// â€” `signer` and `cosigner` must be two DISTINCT founders (signatures verified in
/// tx.rs against their owner keys).
pub fn apply_unset_soulbound(chain: &Chain, name: &str, signer: &str, cosigner: Option<&str>) -> Result<()> {
    let mut nft = get(chain, name).ok_or_else(|| anyhow::anyhow!("name '{}' is not minted", name))?;
    match nft.soulbound {
        Soulbound::None => bail!("'{}' is not soulbound", name),
        Soulbound::Soft => {
            if nft.owner != signer {
                bail!("'{}' does not own '{}'", signer, name);
            }
        }
        Soulbound::Permalock => {
            if !is_founder(signer) {
                bail!("permalock on '{}' can only be lifted by a founder", name);
            }
            let co = cosigner.ok_or_else(|| anyhow::anyhow!("permalock unlock requires a second founder cosigner"))?;
            if !is_founder(co) {
                bail!("cosigner '{}' is not a founder", co);
            }
            if co == signer {
                bail!("permalock unlock requires two DISTINCT founders");
            }
        }
    }
    nft.soulbound = Soulbound::None;
    put(chain, &nft)?;
    Ok(())
}

/// Set an account's primary name. The account must own the name.
pub fn apply_set_primary(chain: &Chain, account: &str, name: &str) -> Result<()> {
    let nft = get(chain, name).ok_or_else(|| anyhow::anyhow!("name '{}' is not minted", name))?;
    if nft.owner != account {
        bail!("'{}' does not own '{}'", account, name);
    }
    chain.store.state_set(&primary_key(account), name.as_bytes())?;
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

    #[test]
    fn mint_then_owner_holds_name() {
        let (chain, _d) = make_chain();
        apply_mint(&chain, "satoshi0", "alice", "alice", 1).unwrap();
        assert_eq!(get(&chain, "satoshi0").unwrap().owner, "alice");
        assert_eq!(chain.store.state_get("name_owner:satoshi0").unwrap(), b"alice");
        assert_eq!(chain.store.state_get(&held_key("alice", "satoshi0")).unwrap(), b"1");
    }

    #[test]
    fn cannot_mint_reserved_unless_issuer() {
        let (chain, _d) = make_chain();
        // "a" is a reserved single-letter name.
        assert!(apply_mint(&chain, "a", "alice", "alice", 1).is_err());
        apply_mint(&chain, "a", "buyer", ISSUER_ACCOUNT, 1).unwrap();
        assert_eq!(get(&chain, "a").unwrap().owner, "buyer");
    }

    #[test]
    fn cannot_mint_into_someone_elses_account() {
        let (chain, _d) = make_chain();
        // non-issuer minting to a different account is rejected
        assert!(apply_mint(&chain, "freename", "bob", "alice", 1).is_err());
    }

    #[test]
    fn transfer_requires_ownership_and_is_pending_until_accept() {
        let (chain, _d) = make_chain();
        apply_mint(&chain, "coolname", "alice", "alice", 1).unwrap();
        // non-owner cannot transfer
        assert!(apply_transfer(&chain, "coolname", "bob", "carol", 2).is_err());
        // owner offers â†’ ownership unchanged, pending set
        apply_transfer(&chain, "coolname", "alice", "bob", 2).unwrap();
        assert_eq!(get(&chain, "coolname").unwrap().owner, "alice");
        assert!(get_pending(&chain, "coolname").is_some());
    }

    #[test]
    fn only_offered_recipient_can_accept() {
        let (chain, _d) = make_chain();
        apply_mint(&chain, "coolname", "alice", "alice", 1).unwrap();
        apply_transfer(&chain, "coolname", "alice", "bob", 2).unwrap();
        assert!(apply_accept(&chain, "coolname", "carol", 3).is_err());
        apply_accept(&chain, "coolname", "bob", 3).unwrap();
        assert_eq!(get(&chain, "coolname").unwrap().owner, "bob");
        assert!(get_pending(&chain, "coolname").is_none());
        // old owner no longer holds it
        assert!(chain.store.state_get(&held_key("alice", "coolname")).is_none());
        assert_eq!(chain.store.state_get(&held_key("bob", "coolname")).unwrap(), b"1");
    }

    #[test]
    fn reject_leaves_ownership_unchanged() {
        let (chain, _d) = make_chain();
        apply_mint(&chain, "coolname", "alice", "alice", 1).unwrap();
        apply_transfer(&chain, "coolname", "alice", "bob", 2).unwrap();
        // a stranger cannot reject
        assert!(apply_reject(&chain, "coolname", "carol").is_err());
        apply_reject(&chain, "coolname", "bob").unwrap();
        assert_eq!(get(&chain, "coolname").unwrap().owner, "alice");
        assert!(get_pending(&chain, "coolname").is_none());
    }

    #[test]
    fn soulbound_blocks_transfer() {
        let (chain, _d) = make_chain();
        apply_mint(&chain, "coolname", "alice", "alice", 1).unwrap();
        apply_set_soulbound(&chain, "coolname", "alice", "soft").unwrap();
        assert!(apply_transfer(&chain, "coolname", "alice", "bob", 2).is_err());
        // soft is owner-reversible
        apply_unset_soulbound(&chain, "coolname", "alice", None).unwrap();
        apply_transfer(&chain, "coolname", "alice", "bob", 3).unwrap();
    }

    #[test]
    fn permalock_needs_two_distinct_founders() {
        let (chain, _d) = make_chain();
        apply_mint(&chain, "coolname", "alice", "alice", 1).unwrap();
        apply_set_soulbound(&chain, "coolname", "alice", "permalock").unwrap();
        // owner alone cannot clear a permalock
        assert!(apply_unset_soulbound(&chain, "coolname", "alice", None).is_err());
        // a single founder cannot
        assert!(apply_unset_soulbound(&chain, "coolname", "shindevlin", None).is_err());
        // same founder twice cannot
        assert!(apply_unset_soulbound(&chain, "coolname", "shindevlin", Some("shindevlin")).is_err());
        // two distinct founders can
        apply_unset_soulbound(&chain, "coolname", "shindevlin", Some("josh")).unwrap();
        assert_eq!(get(&chain, "coolname").unwrap().soulbound, Soulbound::None);
    }

    #[test]
    fn set_primary_requires_ownership() {
        let (chain, _d) = make_chain();
        apply_mint(&chain, "coolname", "alice", "alice", 1).unwrap();
        assert!(apply_set_primary(&chain, "bob", "coolname").is_err());
        apply_set_primary(&chain, "alice", "coolname").unwrap();
        assert_eq!(chain.store.state_get("account_name:alice").unwrap(), b"coolname");
    }

    #[test]
    fn accept_clears_senders_primary() {
        let (chain, _d) = make_chain();
        apply_mint(&chain, "coolname", "alice", "alice", 1).unwrap();
        apply_set_primary(&chain, "alice", "coolname").unwrap();
        apply_transfer(&chain, "coolname", "alice", "bob", 2).unwrap();
        apply_accept(&chain, "coolname", "bob", 3).unwrap();
        assert!(chain.store.state_get("account_name:alice").is_none());
    }

    #[test]
    fn accept_rejected_if_locked_after_offer() {
        let (chain, _d) = make_chain();
        apply_mint(&chain, "coolname", "alice", "alice", 1).unwrap();
        apply_transfer(&chain, "coolname", "alice", "bob", 2).unwrap();
        // Owner locks the name AFTER making the offer.
        apply_set_soulbound(&chain, "coolname", "alice", "soft").unwrap();
        // Locking voided the pending offer, and accept is refused either way.
        assert!(apply_accept(&chain, "coolname", "bob", 3).is_err());
        assert_eq!(get(&chain, "coolname").unwrap().owner, "alice");
    }

    #[test]
    fn expired_offer_cannot_be_accepted() {
        let (chain, _d) = make_chain();
        apply_mint(&chain, "coolname", "alice", "alice", 1).unwrap();
        apply_transfer(&chain, "coolname", "alice", "bob", 10).unwrap();
        // Long past the expiry window -> refused, offer cleared.
        assert!(apply_accept(&chain, "coolname", "bob", 10 + OFFER_EXPIRY_EPOCHS + 1).is_err());
        assert_eq!(get(&chain, "coolname").unwrap().owner, "alice");
        // A fresh offer within the window still works.
        apply_transfer(&chain, "coolname", "alice", "bob", 20).unwrap();
        apply_accept(&chain, "coolname", "bob", 21).unwrap();
        assert_eq!(get(&chain, "coolname").unwrap().owner, "bob");
    }

}
