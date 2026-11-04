//! Find-My grants signed by the owner's hide key (spec: hardware Find-My grants, 2026-11-03).
//!
//! An owner's devices broadcast rotating ids derived from
//! `id_seed_g = HKDF(hide_private, "hone-id-seed" || device_pubkey [|| be32(g) when g > 0])`.
//! A phone can find them without holding the hide key once the owner publishes a standing
//! `FIND_GRANT` for the phone's hardware key. Every entry here is signed by the owner's
//! CURRENT on-chain `hide` key (strictly: no fallback to posting or any other role).
//!
//! Validity is decided at read time: a grant is active only while the hide key that signed
//! it is still the account's hide key, so rotating `hide` (AccountUpdateKey, owner-signed)
//! voids every older grant with no state rewrite. Data only: no balance is touched apart
//! from the normal entry fee.
//!
//! State keys:
//!   find_grant:{owner}:{hw_key}       -> grant record (JSON)
//!   find_gen:{device_pubkey}:{owner}  -> current seed generation (decimal u32)

use anyhow::{bail, Result};
use ed25519_dalek::{Signature, VerifyingKey};
use hone_types::LedgerEntry;

use crate::chain::Chain;

/// Most devices one grant may name.
pub const MAX_DEVICES: usize = 64;

/// The activation gate (test-overridable).
pub fn activation_epoch() -> u64 {
    #[cfg(test)]
    if let Some(v) = TEST_FIND_EPOCH.with(|c| c.get()) { return v; }
    hone_types::FIND_GRANT_EPOCH
}
#[cfg(test)]
thread_local! {
    pub static TEST_FIND_EPOCH: std::cell::Cell<Option<u64>> = const { std::cell::Cell::new(None) };
}

/// 64 lowercase hex characters (a 32-byte id or key).
pub fn is_hex32(s: &str) -> bool {
    s.len() == 64 && s.bytes().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

pub fn grant_key(owner: &str, hw_key: &str) -> String { format!("find_grant:{owner}:{hw_key}") }
pub fn gen_key(device: &str, owner: &str) -> String { format!("find_gen:{device}:{owner}") }

/// The owner's current on-chain hide public key (hex), if it has one.
pub fn current_hide_key(chain: &Chain, owner: &str) -> Option<String> {
    chain.store.get_account(owner).ok().flatten()
        .and_then(|a| a.get("keys").and_then(|k| k.get("hide")).and_then(|v| v.as_str()).map(str::to_owned))
        .filter(|k| !k.is_empty())
}

/// Current seed generation of `device` under `owner` (0 when never bumped).
pub fn generation(chain: &Chain, owner: &str, device: &str) -> u32 {
    chain.store.state_get(&gen_key(device, owner))
        .and_then(|b| String::from_utf8(b).ok())
        .and_then(|s| s.parse().ok())
        .unwrap_or(0)
}
/// Verify `sig_hex` over the entry's canonical message under the owner's CURRENT hide key.
/// Returns the hide key that verified.
pub fn verify_hide_signature(chain: &Chain, owner: &str, entry: &LedgerEntry, sig_hex: Option<&str>) -> Result<String> {
    let Some(pk_hex) = current_hide_key(chain, owner) else {
        bail!("account '{}' has no hide key registered — cannot sign a Find-My entry", owner);
    };
    let sig_str = sig_hex.filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("hide-key signature required for '{}'", owner))?;
    let pk: [u8; 32] = hex::decode(&pk_hex).ok().and_then(|b| b.try_into().ok())
        .ok_or_else(|| anyhow::anyhow!("stored hide key for '{}' is not 32 bytes hex", owner))?;
    let vk = VerifyingKey::from_bytes(&pk).map_err(|e| anyhow::anyhow!("invalid hide key: {}", e))?;
    let sig: [u8; 64] = hex::decode(sig_str).ok().and_then(|b| b.try_into().ok())
        .ok_or_else(|| anyhow::anyhow!("signature must be 64 bytes hex"))?;
    let msg = crate::tx::canonical_signing_message(entry, &chain.chain_id)?;
    vk.verify_strict(msg.as_bytes(), &Signature::from_bytes(&sig))
        .map_err(|_| anyhow::anyhow!("signature does not verify against '{}' current hide key", owner))?;
    Ok(pk_hex)
}

/// Field and state checks shared by validation and apply (deterministic, state-only).
pub fn check(chain: &Chain, entry: &LedgerEntry) -> Result<()> {
    match entry {
        LedgerEntry::FindGrant { owner, hw_key, attestation_digest, devices, signed_by, .. } => {
            if signed_by != owner { bail!("signed_by must equal owner"); }
            if !is_hex32(hw_key) { bail!("hw_key must be 64 lowercase hex"); }
            if !is_hex32(attestation_digest) { bail!("attestation_digest must be 64 lowercase hex"); }
            if devices.is_empty() || devices.len() > MAX_DEVICES {
                bail!("a grant names 1..={} devices", MAX_DEVICES);
            }
            for (i, d) in devices.iter().enumerate() {
                if !is_hex32(&d.device_pubkey) { bail!("device_pubkey must be 64 lowercase hex"); }
                if i > 0 && devices[i - 1].device_pubkey >= d.device_pubkey {
                    bail!("devices must be sorted by device_pubkey with no duplicates");
                }
                let cur = generation(chain, owner, &d.device_pubkey);
                if d.generation > cur {
                    bail!("device {} generation {} is ahead of its published generation {}", d.device_pubkey, d.generation, cur);
                }
            }
        }
        LedgerEntry::FindRevoke { owner, hw_key, signed_by, .. } => {
            if signed_by != owner { bail!("signed_by must equal owner"); }
            if !is_hex32(hw_key) { bail!("hw_key must be 64 lowercase hex"); }
            if chain.store.state_get(&grant_key(owner, hw_key)).is_none() {
                bail!("no Find-My grant for hw_key {} under '{}'", hw_key, owner);
            }
        }
        LedgerEntry::SeedGeneration { owner, device_pubkey, generation: g, signed_by, .. } => {
            if signed_by != owner { bail!("signed_by must equal owner"); }
            if !is_hex32(device_pubkey) { bail!("device_pubkey must be 64 lowercase hex"); }
            let cur = generation(chain, owner, device_pubkey);
            if *g <= cur { bail!("seed generation must increase: {} <= current {}", g, cur); }
        }
        _ => bail!("not a Find-My entry"),
    }
    Ok(())
}
/// Apply (called from Chain::apply_entry). The hide key recorded on a grant is the owner's
/// current hide key at apply time, which is the key validation verified the signature with.
pub fn apply(chain: &Chain, entry: &LedgerEntry) -> Result<()> {
    check(chain, entry)?;
    match entry {
        LedgerEntry::FindGrant { owner, hw_key, attestation_digest, devices, epoch, .. } => {
            let Some(hide) = current_hide_key(chain, owner) else { bail!("'{}' has no hide key", owner) };
            let rec = serde_json::json!({
                "hw_key": hw_key,
                "attestation_digest": attestation_digest,
                "devices": devices,
                "granted_epoch": epoch,
                "signed_hide_key": hide,
            });
            chain.store.state_set(&grant_key(owner, hw_key), &serde_json::to_vec(&rec)?)?;
        }
        LedgerEntry::FindRevoke { owner, hw_key, .. } => {
            chain.store.state_delete(&grant_key(owner, hw_key))?;
        }
        LedgerEntry::SeedGeneration { owner, device_pubkey, generation: g, .. } => {
            chain.store.state_set(&gen_key(device_pubkey, owner), g.to_string().as_bytes())?;
        }
        _ => {}
    }
    Ok(())
}

/// GET /api/find/grants/:owner
pub fn grants_view(chain: &Chain, owner: &str) -> serde_json::Value {
    let hide = current_hide_key(chain, owner);
    let mut gens = serde_json::Map::new();
    let mut grants = Vec::new();
    for (_, v) in chain.store.state_scan_prefix(&format!("find_grant:{owner}:")) {
        let Ok(mut g) = serde_json::from_slice::<serde_json::Value>(&v) else { continue };
        let active = hide.is_some() && g["signed_hide_key"].as_str() == hide.as_deref();
        if let Some(devs) = g["devices"].as_array_mut() {
            for d in devs.iter_mut() {
                let dev = d["device_pubkey"].as_str().unwrap_or("").to_owned();
                let cur = generation(chain, owner, &dev);
                d["current_generation"] = cur.into();
                d["current"] = (d["generation"].as_u64() == Some(cur as u64)).into();
                gens.insert(dev, cur.into());
            }
        }
        g["active"] = active.into();
        grants.push(g);
    }
    serde_json::json!({ "owner": owner, "hide_key": hide, "generations": gens, "grants": grants })
}

/// GET /api/find/generation/:device — the generation under every owner that published one.
pub fn generation_view(chain: &Chain, device: &str, owner: Option<&str>) -> serde_json::Value {
    let pre = format!("find_gen:{device}:");
    let mut by_owner = serde_json::Map::new();
    for (k, v) in chain.store.state_scan_prefix(&pre) {
        let o = k[pre.len()..].to_owned();
        let g: u32 = String::from_utf8(v).ok().and_then(|s| s.parse().ok()).unwrap_or(0);
        by_owner.insert(o, g.into());
    }
    let mut out = serde_json::json!({ "device_pubkey": device, "generations": by_owner });
    if let Some(o) = owner { out["owner"] = o.into(); out["generation"] = generation(chain, o, device).into(); }
    out
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::tx::{canonical_signing_message, validate_and_apply};
    use ed25519_dalek::{Signer, SigningKey};
    use hone_types::{FindDevice, NATIVE_TOKEN, RECYCLE_FUND_ACCOUNT};

    const E: u64 = 1_000;
    const HW: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";

    fn world() -> (Chain, tempfile::TempDir, SigningKey) {
        TEST_FIND_EPOCH.with(|c| c.set(Some(E)));
        let dir = tempfile::Builder::new().prefix("hone_find_test_").tempdir().unwrap();
        let chain = Chain::new(crate::store::Store::open(dir.path()).unwrap(), "test-node".into(), "hone-testnet".into());
        chain.apply_entry(&LedgerEntry::AccountCreate {
            account: "josh".into(), keys: Default::default(), chain_proofs: vec![], epoch: 0, funded_by: None, machine_fingerprint: None,
        }).unwrap();
        for (role, seed) in [("posting", 0x11u8), ("owner", 0x12), ("hide", 0x13)] {
            chain.apply_entry(&LedgerEntry::AccountUpdateKey {
                account: "josh".into(), role: role.into(),
                new_public_key: hex::encode(SigningKey::from_bytes(&[seed; 32]).verifying_key().to_bytes()),
                epoch: 0, signed_by: "josh".into(),
            }).unwrap();
        }
        chain.apply_entry(&LedgerEntry::GenesisAlloc { account: "josh".into(), amount: 1_000_000_000_000, token: NATIVE_TOKEN.into() }).unwrap();
        (chain, dir, SigningKey::from_bytes(&[0x13; 32]))
    }
    fn nonce(chain: &Chain) -> u64 {
        chain.store.get_account("josh").unwrap().unwrap()["nonce"].as_u64().unwrap_or(0) + 1
    }
    fn sig(sk: &SigningKey, e: &LedgerEntry) -> String {
        hex::encode(sk.sign(canonical_signing_message(e, "hone-testnet").unwrap().as_bytes()).to_bytes())
    }
    fn dev(n: u8) -> String { hex::encode([n; 32]) }
    fn grant(chain: &Chain, devs: &[(u8, u32)]) -> LedgerEntry {
        LedgerEntry::FindGrant {
            owner: "josh".into(), hw_key: HW.into(), attestation_digest: "bb".repeat(32),
            devices: devs.iter().map(|(n, g)| FindDevice { device_pubkey: dev(*n), generation: *g }).collect(),
            nonce: nonce(chain), epoch: E, signed_by: "josh".into(),
        }
    }
    fn submit(chain: &Chain, sk: &SigningKey, e: &LedgerEntry) -> Result<String> { validate_and_apply(chain, e, Some(&sig(sk, e))) }

    #[test]
    fn canonical_message_is_written_order_without_epoch() {
        let (chain, _d, _) = world();
        let m = canonical_signing_message(&grant(&chain, &[(1, 0)]), "hone").unwrap();
        assert_eq!(m, format!("{{\"chain_id\":\"hone\",\"type\":\"FIND_GRANT\",\"owner\":\"josh\",\"hw_key\":\"{HW}\",\"attestation_digest\":\"{}\",\"devices\":[{{\"device_pubkey\":\"{}\",\"generation\":0}}],\"nonce\":1}}", "bb".repeat(32), dev(1)));
        let r = LedgerEntry::FindRevoke { owner: "josh".into(), hw_key: HW.into(), nonce: 2, epoch: E, signed_by: "josh".into() };
        assert_eq!(canonical_signing_message(&r, "hone").unwrap(),
            format!("{{\"chain_id\":\"hone\",\"type\":\"FIND_REVOKE\",\"owner\":\"josh\",\"hw_key\":\"{HW}\",\"nonce\":2}}"));
        let s = LedgerEntry::SeedGeneration { owner: "josh".into(), device_pubkey: dev(1), generation: 3, nonce: 4, epoch: E, signed_by: "josh".into() };
        assert_eq!(canonical_signing_message(&s, "hone").unwrap(),
            format!("{{\"chain_id\":\"hone\",\"type\":\"SEED_GENERATION\",\"owner\":\"josh\",\"device_pubkey\":\"{}\",\"generation\":3,\"nonce\":4}}", dev(1)));
    }

    #[test]
    fn hide_key_signs_and_only_the_fee_moves() {
        let (chain, _d, hide) = world();
        let (j0, r0) = (chain.get_balance("josh", NATIVE_TOKEN), chain.get_balance(RECYCLE_FUND_ACCOUNT, NATIVE_TOKEN));
        submit(&chain, &hide, &grant(&chain, &[(1, 0), (2, 0)])).unwrap();
        let v = grants_view(&chain, "josh");
        assert_eq!(v["grants"].as_array().unwrap().len(), 1);
        assert_eq!(v["grants"][0]["active"], true);
        assert_eq!(v["grants"][0]["signed_hide_key"], v["hide_key"]);
        let (j1, r1) = (chain.get_balance("josh", NATIVE_TOKEN), chain.get_balance(RECYCLE_FUND_ACCOUNT, NATIVE_TOKEN));
        assert_eq!(j0 - j1, r1 - r0, "only the entry fee moves");
        assert_eq!(hone_types::entry_weight(&grant(&chain, &[(1, 0)])), 1);
    }
    #[test]
    fn non_hide_signer_is_rejected() {
        let (chain, _d, hide) = world();
        for seed in [0x11u8, 0x12, 0x99] { // posting, owner, a stranger
            let g = grant(&chain, &[(1, 0)]);
            assert!(submit(&chain, &SigningKey::from_bytes(&[seed; 32]), &g).is_err(), "seed {seed:#x} must not sign");
        }
        assert!(validate_and_apply(&chain, &grant(&chain, &[(1, 0)]), None).is_err());
        let mut g = grant(&chain, &[(1, 0)]);
        if let LedgerEntry::FindGrant { signed_by, .. } = &mut g { *signed_by = "bob".into(); }
        assert!(submit(&chain, &hide, &g).is_err(), "signed_by must be the owner");
        assert!(chain.store.state_scan_prefix("find_grant:").is_empty());
    }

    #[test]
    fn gated_before_activation_epoch() {
        let (chain, _d, hide) = world();
        let mut g = grant(&chain, &[(1, 0)]);
        if let LedgerEntry::FindGrant { epoch, .. } = &mut g { *epoch = E - 1; }
        assert!(submit(&chain, &hide, &g).is_err());
    }

    #[test]
    fn rotating_hide_key_voids_every_grant() {
        let (chain, _d, hide) = world();
        submit(&chain, &hide, &grant(&chain, &[(1, 0)])).unwrap();
        let owner_sk = SigningKey::from_bytes(&[0x12; 32]);
        let rot = LedgerEntry::AccountUpdateKey {
            account: "josh".into(), role: "hide".into(),
            new_public_key: hex::encode(SigningKey::from_bytes(&[0x14; 32]).verifying_key().to_bytes()),
            epoch: E, signed_by: "josh".into(),
        };
        validate_and_apply(&chain, &rot, Some(&sig(&owner_sk, &rot))).unwrap();
        assert_eq!(grants_view(&chain, "josh")["grants"][0]["active"], false, "old grant void after rotation");
        assert!(submit(&chain, &hide, &grant(&chain, &[(1, 0)])).is_err(), "old hide key can no longer sign");
        submit(&chain, &SigningKey::from_bytes(&[0x14; 32]), &grant(&chain, &[(1, 0)])).unwrap();
        assert_eq!(grants_view(&chain, "josh")["grants"][0]["active"], true);
    }

    #[test]
    fn revoke_removes_the_grant() {
        let (chain, _d, hide) = world();
        submit(&chain, &hide, &grant(&chain, &[(1, 0)])).unwrap();
        let r = |chain: &Chain| LedgerEntry::FindRevoke { owner: "josh".into(), hw_key: HW.into(), nonce: nonce(chain), epoch: E, signed_by: "josh".into() };
        assert!(submit(&chain, &SigningKey::from_bytes(&[0x11; 32]), &r(&chain)).is_err(), "posting key cannot revoke");
        submit(&chain, &hide, &r(&chain)).unwrap();
        assert!(grants_view(&chain, "josh")["grants"].as_array().unwrap().is_empty());
        assert!(submit(&chain, &hide, &r(&chain)).is_err(), "nothing left to revoke");
    }

    #[test]
    fn generation_is_monotonic_and_bounds_grants() {
        let (chain, _d, hide) = world();
        let sg = |chain: &Chain, g: u32| LedgerEntry::SeedGeneration {
            owner: "josh".into(), device_pubkey: dev(1), generation: g, nonce: nonce(chain), epoch: E, signed_by: "josh".into() };
        assert!(submit(&chain, &hide, &sg(&chain, 0)).is_err(), "0 is the implicit start");
        assert!(submit(&chain, &hide, &grant(&chain, &[(1, 1)])).is_err(), "grant ahead of published generation");
        submit(&chain, &hide, &sg(&chain, 2)).unwrap();
        assert_eq!(generation(&chain, "josh", &dev(1)), 2);
        assert!(submit(&chain, &hide, &sg(&chain, 2)).is_err());
        assert!(submit(&chain, &hide, &sg(&chain, 1)).is_err());
        assert!(submit(&chain, &SigningKey::from_bytes(&[0x11; 32]), &sg(&chain, 3)).is_err(), "posting key cannot bump");
        submit(&chain, &hide, &grant(&chain, &[(1, 2)])).unwrap();
        let v = generation_view(&chain, &dev(1), Some("josh"));
        assert_eq!(v["generation"], 2);
        assert_eq!(v["generations"]["josh"], 2);
        submit(&chain, &hide, &sg(&chain, 3)).unwrap();
        assert_eq!(grants_view(&chain, "josh")["grants"][0]["devices"][0]["current"], false, "grant now names a stale generation");
    }

    #[test]
    fn devices_must_be_sorted_and_unique() {
        let (chain, _d, hide) = world();
        assert!(submit(&chain, &hide, &grant(&chain, &[(2, 0), (1, 0)])).is_err());
        assert!(submit(&chain, &hide, &grant(&chain, &[(1, 0), (1, 0)])).is_err());
        assert!(submit(&chain, &hide, &grant(&chain, &[])).is_err());
    }
}