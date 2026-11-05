//! Sponsor vouchers: an invite link may carry `?v=<voucher secret>`; the installer passes it as
//! HONE_VOUCHER and leaves it in the data dir; the listener moves it into secure storage at once.
//! Once the station key exists and the chain's VOUCHERS feature is active, the listener submits
//! SENSOR_SEAT_CLAIM to POST <chain>/api/sensor/seat/claim, signed by the voucher key and
//! countersigned by the station (seat) key. The secret never leaves this computer: only the
//! voucher PUBLIC key and the two signatures are sent, and only in that claim.
//!
//! PROVISIONAL wire format (the chain agent's SENSOR_SEAT_CLAIM was not published when this was
//! written; match it before turning VOUCHERS_ACTIVE on):
//!   message = {"chain_id":"hone","type":"SENSOR_SEAT_CLAIM","voucher_pubkey":<hex>,
//!              "seat_pubkey":<hex>,"owner":<account>,"ts":<unix s>}   (keys in this order, compact)
//!   body    = {"voucher_pubkey","seat_pubkey","owner","ts","voucher_sig","seat_sig"} (sigs hex, over message)
//! The voucher secret is the voucher key's 32-byte ed25519 seed, as 64 hex or 43 base64url chars.

use crate::config::{now_ms, state_dir, Config};
use crate::{canonical, secrets};
use base64::Engine;
use ed25519_dalek::SigningKey;
use serde_json::{json, Value};

/// Chain feature gate. Off until the chain agent ships VOUCHERS.
pub const VOUCHERS_ACTIVE: bool = false;
pub const SAVED: &str = "Your invite is saved";

pub fn valid(s: &str) -> bool {
    regex::Regex::new(r"^[A-Za-z0-9_-]{16,128}$").unwrap().is_match(s)
}

/// Move a voucher left by the installer (or the page) into secure storage. Never logged.
pub fn import_from_disk() {
    let f = state_dir().join("voucher");
    if let Ok(v) = std::fs::read_to_string(&f) {
        let v = v.trim();
        if valid(v) && secrets::get("voucher").is_none() {
            let _ = secrets::set("voucher", v);
        }
        let _ = std::fs::remove_file(&f);
    }
}

pub fn key_of(secret: &str) -> Option<SigningKey> {
    let b = if secret.len() == 64 && secret.chars().all(|c| c.is_ascii_hexdigit()) {
        hex::decode(secret).ok()?
    } else {
        base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(secret.trim_end_matches('=')).ok()?
    };
    Some(SigningKey::from_bytes(&<[u8; 32]>::try_from(b.as_slice()).ok()?))
}

pub fn message(voucher_pub: &str, seat_pub: &str, owner: &str, ts: u64) -> String {
    let q = |s: &str| canonical::encode(&json!(s));
    format!("{{\"chain_id\":\"hone\",\"type\":\"SENSOR_SEAT_CLAIM\",\"voucher_pubkey\":{},\"seat_pubkey\":{},\"owner\":{},\"ts\":{ts}}}",
        q(voucher_pub), q(seat_pub), q(owner))
}

pub fn claim_body(voucher: &SigningKey, seat: &SigningKey, owner: &str, ts: u64) -> Value {
    let (vp, sp) = (canonical::pub_hex(voucher), canonical::pub_hex(seat));
    let m = message(&vp, &sp, owner, ts);
    json!({"voucher_pubkey": vp, "seat_pubkey": sp, "owner": owner, "ts": ts,
        "voucher_sig": canonical::sign_hex(voucher, &m), "seat_sig": canonical::sign_hex(seat, &m)})
}

/// State line for the page, or None when there is no voucher.
pub fn state() -> Option<String> {
    let v = secrets::get("voucher")?;
    if secrets::get("voucher_claimed").is_some() {
        return Some("Your invite was used: this listener's place is sponsored".into());
    }
    let _ = v;
    Some(if VOUCHERS_ACTIVE { "Your invite is being used…".into() } else { SAVED.into() })
}

/// Submit the claim once (gated). Ok(true) = claimed.
pub fn try_claim(cfg: &Config) -> Result<bool, String> {
    if !VOUCHERS_ACTIVE || secrets::get("voucher_claimed").is_some() {
        return Ok(false);
    }
    let Some(secret) = secrets::get("voucher") else { return Ok(false) };
    let Some(acct) = cfg.account.as_ref() else { return Ok(false) };
    let vk = key_of(&secret).ok_or("invite code is not a voucher key")?;
    let seat = secrets::station_key()?;
    let owner = acct.referrer().unwrap_or(&acct.handle).to_string();
    let body = claim_body(&vk, &seat, &owner, now_ms() / 1000);
    let r = crate::account::agent().post(&format!("{}/api/sensor/seat/claim", cfg.chain)).set("Content-Type", "application/json")
        .send_string(&body.to_string());
    let j: Value = match r {
        Ok(r) => r.into_json().unwrap_or(json!({})),
        Err(ureq::Error::Status(_, r)) => r.into_json().unwrap_or(json!({})),
        Err(e) => return Err(format!("offline ({})", e.kind())),
    };
    if j["accepted"].as_bool() == Some(true) {
        let _ = secrets::set("voucher_claimed", &now_ms().to_string());
        return Ok(true);
    }
    Err(j["error"].as_str().unwrap_or("claim refused").chars().take(120).collect())
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signature, Verifier};

    #[test]
    fn claim_is_signed_by_both_keys_and_carries_no_secret() {
        let secret = hex::encode([5u8; 32]);
        let vk = key_of(&secret).unwrap();
        let seat = SigningKey::from_bytes(&[9u8; 32]);
        let b = claim_body(&vk, &seat, "maria", 1_793_000_000);
        let m = message(b["voucher_pubkey"].as_str().unwrap(), b["seat_pubkey"].as_str().unwrap(), "maria", 1_793_000_000);
        assert!(m.contains("\"type\":\"SENSOR_SEAT_CLAIM\"") && m.contains(&canonical::pub_hex(&seat)));
        for (k, key) in [("voucher_sig", vk.verifying_key()), ("seat_sig", seat.verifying_key())] {
            let s = Signature::from_slice(&hex::decode(b[k].as_str().unwrap()).unwrap()).unwrap();
            assert!(key.verify(m.as_bytes(), &s).is_ok(), "{k}");
        }
        assert!(!b.to_string().contains(&secret));
        let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([5u8; 32]);
        assert_eq!(canonical::pub_hex(&key_of(&b64).unwrap()), canonical::pub_hex(&vk));
        assert!(!VOUCHERS_ACTIVE);
        assert!(valid(&secret) && !valid("short") && !valid("has space in it......"));
    }
}
