//! Soulbound tokens â€” non-transferable credentials bound to a NAME (identity), not a
//! wallet, so they survive key rotation/recovery (they are about *who you are*).
//!
//! PROPOSED. Gated in tx.rs behind IDENTITY_NFT_ACTIVATION_EPOCH. apply_* run from
//! Chain::apply_entry after signature/nonce checks.
//!
//! Classes:
//!   HolderScoped (default) â€” verifications, reputation, KYC, achievements. Survive
//!     recovery (same holder, new key); STRIP on transfer to a new holder (Stage 3).
//!   Provenance â€” the name's record (minted-by, awards-as-history); persists across
//!     transfers too. Not a claim about the current holder.
//! `wallet_bound` marks an ephemeral, key-specific token that should NOT survive a key
//! change (handled where key rotation lands; stored here).
//!
//! Must-accept: issuance creates a pending grant; the name's owner accepts or rejects â€”
//! nobody can brand an identity with an unwanted badge. Issuer may revoke. A revoked or
//! stripped token is kept as inactive provenance, never silently deleted.
//!
//! State:
//!   sbt:{token_id}              -> SoulboundToken { .. , accepted, revoked, stripped }
//!   sbt_pending:{name}:{tid}    -> "1"  (offered, awaiting accept)
//!   sbt_on:{name}:{tid}         -> "1"  (active, accepted, on the name)
#![allow(dead_code)]

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use crate::chain::Chain;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum SbtClass {
    #[default]
    HolderScoped,
    Provenance,
}

impl SbtClass {
    pub fn parse(s: &str) -> Result<SbtClass> {
        match s {
            "holder_scoped" | "holder" | "" => Ok(SbtClass::HolderScoped),
            "provenance" => Ok(SbtClass::Provenance),
            other => bail!("unknown soulbound class '{}'", other),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SoulboundToken {
    pub token_id: String,
    pub name: String,        // the identity this credential is bound to
    pub issuer: String,
    pub class: SbtClass,
    #[serde(default)]
    pub wallet_bound: bool,
    pub data: String,        // uri / hash of the credential payload
    pub issued_epoch: u64,
    #[serde(default)]
    pub accepted: bool,
    #[serde(default)]
    pub revoked: bool,
    #[serde(default)]
    pub stripped: bool,      // set when a holder change voids a holder-scoped token
}

fn tok_key(tid: &str) -> String { format!("sbt:{}", tid) }
fn pending_key(name: &str, tid: &str) -> String { format!("sbt_pending:{}:{}", name, tid) }
fn on_key(name: &str, tid: &str) -> String { format!("sbt_on:{}:{}", name, tid) }

pub fn get(chain: &Chain, tid: &str) -> Option<SoulboundToken> {
    chain.store.state_get(&tok_key(tid)).and_then(|b| serde_json::from_slice(&b).ok())
}
fn put(chain: &Chain, t: &SoulboundToken) -> Result<()> {
    chain.store.state_set(&tok_key(&t.token_id), &serde_json::to_vec(t)?)?;
    Ok(())
}

fn name_owner(chain: &Chain, name: &str) -> Option<String> {
    crate::identity_nft::get(chain, name).map(|n| n.owner)
}

/// Issue a credential to a name. Creates a PENDING grant (must be accepted).
pub fn apply_issue(chain: &Chain, token_id: &str, name: &str, issuer: &str,
    class: &str, wallet_bound: bool, data: &str, epoch: u64) -> Result<()> {
    if token_id.trim().is_empty() { bail!("empty token_id"); }
    if get(chain, token_id).is_some() { bail!("soulbound token '{}' already exists", token_id); }
    if crate::identity_nft::get(chain, name).is_none() { bail!("name '{}' is not minted", name); }
    let class = SbtClass::parse(class)?;
    let t = SoulboundToken {
        token_id: token_id.to_string(), name: name.to_string(), issuer: issuer.to_string(),
        class, wallet_bound, data: data.to_string(), issued_epoch: epoch,
        accepted: false, revoked: false, stripped: false,
    };
    put(chain, &t)?;
    chain.store.state_set(&pending_key(name, token_id), b"1")?;
    Ok(())
}

/// The name's current owner accepts a pending credential; it becomes active.
pub fn apply_accept(chain: &Chain, token_id: &str, acceptor: &str) -> Result<()> {
    let mut t = get(chain, token_id).ok_or_else(|| anyhow::anyhow!("no soulbound token '{}'", token_id))?;
    if t.accepted { bail!("token '{}' already accepted", token_id); }
    match name_owner(chain, &t.name) {
        Some(o) if o == acceptor => {}
        Some(_) => bail!("only the current holder of '{}' may accept this credential", t.name),
        None => bail!("name '{}' is not minted", t.name),
    }
    t.accepted = true;
    put(chain, &t)?;
    chain.store.state_delete(&pending_key(&t.name, token_id))?;
    chain.store.state_set(&on_key(&t.name, token_id), b"1")?;
    Ok(())
}

/// Decline a pending credential, or the holder drops an accepted one. Either the name's
/// holder or the issuer may do it.
pub fn apply_reject(chain: &Chain, token_id: &str, signer: &str) -> Result<()> {
    let t = get(chain, token_id).ok_or_else(|| anyhow::anyhow!("no soulbound token '{}'", token_id))?;
    let is_holder = name_owner(chain, &t.name).as_deref() == Some(signer);
    if !is_holder && signer != t.issuer {
        bail!("only the holder of '{}' or the issuer may reject this credential", t.name);
    }
    chain.store.state_delete(&pending_key(&t.name, token_id))?;
    chain.store.state_delete(&on_key(&t.name, token_id))?;
    chain.store.state_delete(&tok_key(token_id))?;
    Ok(())
}

/// Issuer revokes an attestation. Record is kept (revoked=true) for provenance, but it is
/// no longer active on the name.
pub fn apply_revoke(chain: &Chain, token_id: &str, revoker: &str) -> Result<()> {
    let mut t = get(chain, token_id).ok_or_else(|| anyhow::anyhow!("no soulbound token '{}'", token_id))?;
    if revoker != t.issuer { bail!("only the issuer may revoke '{}'", token_id); }
    t.revoked = true;
    put(chain, &t)?;
    chain.store.state_delete(&on_key(&t.name, token_id))?;
    chain.store.state_delete(&pending_key(&t.name, token_id))?;
    Ok(())
}

/// Stage 3 hook: list active (accepted, non-revoked) token ids on a name.
pub fn active_on(chain: &Chain, name: &str) -> Vec<String> {
    let prefix = format!("sbt_on:{}:", name);
    chain.store.state_scan_prefix(&prefix)
        .into_iter()
        .filter_map(|(k, _)| k.rsplit(':').next().map(|s| s.to_string()))
        .collect()
}

/// A holder change (transfer/sale) strips HolderScoped credentials from the name,
/// keeping each as inactive provenance (`stripped=true`), while Provenance-class tokens
/// persist. Called from `identity_nft::apply_accept`. Recovery does NOT call this: it is
/// the same holder on a new key, and tokens are keyed by name, so everything survives
/// automatically.
pub fn strip_holder_scoped(chain: &Chain, name: &str) -> Result<()> {
    for tid in active_on(chain, name) {
        if let Some(mut t) = get(chain, &tid) {
            if t.class == SbtClass::HolderScoped {
                t.stripped = true;
                put(chain, &t)?;
                chain.store.state_delete(&on_key(name, &tid))?;
            }
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use hone_types::LedgerEntry;

    fn make_chain() -> (Chain, tempfile::TempDir) {
        let dir = tempfile::Builder::new().prefix("hone_sbt_").tempdir().unwrap();
        let store = crate::store::Store::open(dir.path()).unwrap();
        let chain = Chain::new(store, "node".to_string(), "hone-test".to_string());
        (chain, dir)
    }
    fn mint(chain: &Chain, name: &str, owner: &str) {
        chain.apply_entry(&LedgerEntry::IdentityNftMint {
            name: name.into(), to: owner.into(), epoch: 1, nonce: 0,
            signed_by: owner.into(), signature: None,
        }).unwrap();
    }

    #[test]
    fn issue_is_pending_until_accept() {
        let (c, _d) = make_chain();
        mint(&c, "alice", "alice");
        apply_issue(&c, "kyc1", "alice", "verifier", "holder_scoped", false, "ipfs://x", 2).unwrap();
        assert!(!get(&c, "kyc1").unwrap().accepted);
        assert!(active_on(&c, "alice").is_empty());
        apply_accept(&c, "kyc1", "alice").unwrap();
        assert!(get(&c, "kyc1").unwrap().accepted);
        assert_eq!(active_on(&c, "alice"), vec!["kyc1".to_string()]);
    }

    #[test]
    fn only_holder_can_accept() {
        let (c, _d) = make_chain();
        mint(&c, "alice", "alice");
        apply_issue(&c, "kyc1", "alice", "verifier", "", false, "x", 2).unwrap();
        assert!(apply_accept(&c, "kyc1", "mallory").is_err());
        apply_accept(&c, "kyc1", "alice").unwrap();
    }

    #[test]
    fn cannot_be_force_branded_reject_removes_it() {
        let (c, _d) = make_chain();
        mint(&c, "alice", "alice");
        apply_issue(&c, "spam", "alice", "mallory", "holder_scoped", false, "bad", 2).unwrap();
        apply_reject(&c, "spam", "alice").unwrap();
        assert!(get(&c, "spam").is_none());
    }

    #[test]
    fn issuer_can_revoke_kept_as_provenance() {
        let (c, _d) = make_chain();
        mint(&c, "alice", "alice");
        apply_issue(&c, "kyc1", "alice", "verifier", "holder_scoped", false, "x", 2).unwrap();
        apply_accept(&c, "kyc1", "alice").unwrap();
        assert!(apply_revoke(&c, "kyc1", "someone").is_err());
        apply_revoke(&c, "kyc1", "verifier").unwrap();
        assert!(get(&c, "kyc1").unwrap().revoked);
        assert!(active_on(&c, "alice").is_empty());
    }

    #[test]
    fn duplicate_token_id_rejected() {
        let (c, _d) = make_chain();
        mint(&c, "alice", "alice");
        apply_issue(&c, "t", "alice", "iss", "", false, "x", 2).unwrap();
        assert!(apply_issue(&c, "t", "alice", "iss", "", false, "x", 2).is_err());
    }

    #[test]
    fn class_parsing() {
        assert_eq!(SbtClass::parse("holder_scoped").unwrap(), SbtClass::HolderScoped);
        assert_eq!(SbtClass::parse("").unwrap(), SbtClass::HolderScoped);
        assert_eq!(SbtClass::parse("provenance").unwrap(), SbtClass::Provenance);
        assert!(SbtClass::parse("bogus").is_err());
    }

    #[test]
    fn transfer_strips_holder_scoped_keeps_provenance() {
        let (c, _d) = make_chain();
        mint(&c, "alice", "alice");
        apply_issue(&c, "kyc", "alice", "verifier", "holder_scoped", false, "x", 2).unwrap();
        apply_accept(&c, "kyc", "alice").unwrap();
        apply_issue(&c, "mintedby", "alice", "alice", "provenance", false, "y", 2).unwrap();
        apply_accept(&c, "mintedby", "alice").unwrap();
        // a real holder change: transfer + accept
        crate::identity_nft::apply_transfer(&c, "alice", "alice", "bob", 3).unwrap();
        crate::identity_nft::apply_accept(&c, "alice", "bob", 4).unwrap();
        assert!(get(&c, "kyc").unwrap().stripped, "holder-scoped must strip on transfer");
        assert!(!get(&c, "mintedby").unwrap().stripped, "provenance must persist");
        let act = active_on(&c, "alice");
        assert!(act.contains(&"mintedby".to_string()));
        assert!(!act.contains(&"kyc".to_string()));
    }

    #[test]
    fn recovery_keeps_all_credentials() {
        let (c, _d) = make_chain();
        mint(&c, "alice", "alice");
        apply_issue(&c, "kyc", "alice", "verifier", "holder_scoped", false, "x", 2).unwrap();
        apply_accept(&c, "kyc", "alice").unwrap();
        crate::identity_nft::apply_set_recovery(&c, "alice", "alice", vec!["bk".into()], 1, "alice-bk", false).unwrap();
        crate::identity_nft::apply_lock(&c, "alice", "alice").unwrap();
        let seq = crate::identity_nft::get(&c, "alice").unwrap().recovery_seq;
        crate::identity_nft::apply_recover(&c, "alice", seq).unwrap();
        assert_eq!(crate::identity_nft::get(&c, "alice").unwrap().owner, "alice-bk");
        assert!(!get(&c, "kyc").unwrap().stripped, "recovery keeps credentials");
        assert!(active_on(&c, "alice").contains(&"kyc".to_string()));
    }

}
