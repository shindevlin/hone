//! Sponsor vouchers: an invite link may carry `?v=<voucher secret>`; the installer passes it as
//! HONE_VOUCHER and leaves it in the data dir; the listener moves it into secure storage at once.
//! Once the station key exists it submits SENSOR_SEAT_CLAIM (POST <chain>/api/sensor/seat/claim),
//! signed by the voucher key, with the station (seat) key's countersignature, so the claim
//! cannot be redirected to another seat. The secret never leaves this computer: the claim
//! carries only the voucher PUBLIC key and the two signatures.
//!
//! Wire format = the chain's (rust/hone-node src/tx.rs canonical_signing_message, branch
//! `vouchers` 9c77207e): message = compact JSON, keys in exactly this order (not sorted):
//!   {"chain_id","type":"SENSOR_SEAT_CLAIM","voucher_pubkey","seat_pubkey","kind"}; body = {voucher_pubkey, seat_pubkey, kind, seat_sig,
//! signature}; the node fills `epoch` (not signed). The claim registers seat
//! `{sponsor}@{seat_pubkey}`, backed by the sponsor's stake.
//! Gating is the chain's: until VOUCHERS is active (or while the chain doesn't know the entry)
//! the claim is refused and retried later; the page shows "Your invite is saved".
//! The voucher secret is the voucher key's 32-byte ed25519 seed, as 64 hex or 43 base64url chars.

use crate::config::{now_ms, state_dir, Config};
use crate::{canonical, secrets};
use base64::Engine;
use ed25519_dalek::SigningKey;
use serde_json::{json, Value};

pub const SAVED: &str = "Your invite is saved";
pub const KIND: &str = "station";

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

/// The chain's signing message, byte for byte: compact JSON with the keys in THIS order
/// (hone-node builds it with serde_json's preserve_order on, so it is not key-sorted).
pub fn message(chain_id: &str, voucher_pub: &str, seat_pub: &str) -> String {
    let q = |s: &str| canonical::encode(&json!(s));
    format!("{{\"chain_id\":{},\"type\":\"SENSOR_SEAT_CLAIM\",\"voucher_pubkey\":{},\"seat_pubkey\":{},\"kind\":{}}}",
        q(chain_id), q(voucher_pub), q(seat_pub), q(KIND))
}

pub fn claim_body(chain_id: &str, voucher: &SigningKey, seat: &SigningKey) -> Value {
    let (vp, sp) = (canonical::pub_hex(voucher), canonical::pub_hex(seat));
    let m = message(chain_id, &vp, &sp);
    json!({"voucher_pubkey": vp, "seat_pubkey": sp, "kind": KIND, "seat_sig": canonical::sign_hex(seat, &m), "signature": canonical::sign_hex(voucher, &m)})
}

/// Refusals that will never succeed: stop retrying and tell the user.
pub fn is_final(err: &str) -> Option<&'static str> {
    let e = err.to_lowercase();
    if e.contains("expired") { return Some("Your invite has expired. Ask the person who invited you for a new one."); }
    if e.contains("fully claimed") { return Some("Your invite has been used up. Ask for a new one."); }
    if e.contains("not open") { return Some("Your invite was cancelled. Ask for a new one."); }
    if e.contains("already registered") { return Some("This listener already has its place."); }
    None
}

/// State line for the page, or None when there is no voucher.
pub fn state() -> Option<String> {
    secrets::get("voucher")?;
    if secrets::get("voucher_claimed").is_some() {
        return Some("Your invite was used: this listener's place is sponsored".into());
    }
    if let Some(f) = secrets::get("voucher_final") {
        return Some(f);
    }
    Some(SAVED.into())
}

fn chain_id(cfg: &Config) -> String {
    crate::account::agent().get(&format!("{}/api/node/info", cfg.chain)).call().ok()
        .and_then(|r| r.into_json::<Value>().ok()).and_then(|j| j["chain_id"].as_str().map(String::from)).unwrap_or_else(|| "hone".into())
}

/// Try the claim (the chain decides whether vouchers are live). Ok(true) = claimed.
pub fn try_claim(cfg: &Config) -> Result<bool, String> {
    if secrets::get("voucher_claimed").is_some() || secrets::get("voucher_final").is_some() {
        return Ok(false);
    }
    let Some(secret) = secrets::get("voucher") else { return Ok(false) };
    let vk = key_of(&secret).ok_or("invite code is not a voucher key")?;
    let seat = secrets::station_key()?;
    let body = claim_body(&chain_id(cfg), &vk, &seat);
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
    let err: String = j["error"].as_str().unwrap_or("claim refused").chars().take(160).collect();
    if let Some(msg) = is_final(&err) {
        let _ = secrets::set("voucher_final", msg);
    }
    Err(err)
}

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signature, Verifier};

    /// Vector from the chain itself: rust/hone-node (branch vouchers, 9c77207e)
    /// canonical_signing_message(SensorSeatClaim{voucher=seed 0x42, seat=seed 0x43, kind "station"}, "hone").
    const CHAIN_MSG: &str = r#"{"chain_id":"hone","type":"SENSOR_SEAT_CLAIM","voucher_pubkey":"2152f8d19b791d24453242e15f2eab6cb7cffa7b6a5ed30097960e069881db12","seat_pubkey":"22fc297792f0b6ffc0bfcfdb7edb0c0aa14e025a365ec0e342e86e3829cb74b6","kind":"station"}"#;
    const CHAIN_VSIG: &str = "83c0dd430536161116d0a687c80811886e020daf7a4f869f35ad544a92a1744afff2bd2196a968f01d206712d0a0b7175406862f6edea2d590fc6189edc88f0c";
    const CHAIN_SSIG: &str = "44320e9270e52e5fc4fc34d6400b81d55f7f915c3b7383dc36fff41cc77bed186ad906c4993019f79d74dc27948f993771904221d8d9b47e7ed141a42af26206";

    #[test]
    fn matches_the_chain_vector() {
        let v = SigningKey::from_bytes(&[0x42; 32]);
        let s = SigningKey::from_bytes(&[0x43; 32]);
        let b = claim_body("hone", &v, &s);
        assert_eq!(message("hone", b["voucher_pubkey"].as_str().unwrap(), b["seat_pubkey"].as_str().unwrap()), CHAIN_MSG);
        assert_eq!(b["signature"], CHAIN_VSIG);
        assert_eq!(b["seat_sig"], CHAIN_SSIG);
        assert_eq!(b["kind"], "station");
        assert!(b.get("owner").is_none() && b.get("ts").is_none());
    }

    #[test]
    fn claim_is_signed_by_both_keys_and_carries_no_secret() {
        let secret = hex::encode([5u8; 32]);
        let vk = key_of(&secret).unwrap();
        let seat = SigningKey::from_bytes(&[9u8; 32]);
        let b = claim_body("hone", &vk, &seat);
        let m = message("hone", b["voucher_pubkey"].as_str().unwrap(), b["seat_pubkey"].as_str().unwrap());
        for (k, key) in [("signature", vk.verifying_key()), ("seat_sig", seat.verifying_key())] {
            let sg = Signature::from_slice(&hex::decode(b[k].as_str().unwrap()).unwrap()).unwrap();
            assert!(key.verify(m.as_bytes(), &sg).is_ok(), "{k}");
        }
        assert!(!b.to_string().contains(&secret));
        let b64 = base64::engine::general_purpose::URL_SAFE_NO_PAD.encode([5u8; 32]);
        assert_eq!(canonical::pub_hex(&key_of(&b64).unwrap()), canonical::pub_hex(&vk));
        assert!(valid(&secret) && !valid("short"));
        assert!(is_final("voucher expired").is_some() && is_final("SENSOR_SEAT_CLAIM is not active").is_none());
    }
}

