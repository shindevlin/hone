//! Password hashing, TOTP 2FA, and API-key generation/hashing.
//!
//! Two credential paths, kept structurally separate per the PRD:
//! - Humans: email + argon2id password + mandatory TOTP for owner/admin ->
//!   opaque session cookie (HttpOnly/Secure/SameSite=Lax), server-side
//!   session store.
//! - Machines: `Authorization: Bearer btcpc_live_<secret>` API keys, shown
//!   once, stored only as a salted SHA-256 hash.

use argon2::password_hash::{PasswordHash, PasswordHasher, PasswordVerifier, SaltString};
use argon2::Argon2;
use rand::RngCore;
use sha2::{Digest, Sha256};
use subtle::ConstantTimeEq;

// ------------------------------- Passwords ---------------------------------

#[derive(Debug, thiserror::Error)]
pub enum AuthError {
    #[error("hashing failed")]
    HashFailed,
    #[error("invalid credentials")]
    InvalidCredentials,
}

/// Hash a plaintext password with argon2id + a random salt. Never store the
/// plaintext; this PHC string is what goes in `member.password_hash`.
pub fn hash_password(plaintext: &str) -> Result<String, AuthError> {
    let salt = SaltString::generate(&mut rand::rngs::OsRng);
    let argon2 = Argon2::default();
    argon2
        .hash_password(plaintext.as_bytes(), &salt)
        .map(|h| h.to_string())
        .map_err(|_| AuthError::HashFailed)
}

pub fn verify_password(plaintext: &str, stored_hash: &str) -> bool {
    let parsed = match PasswordHash::new(stored_hash) {
        Ok(p) => p,
        Err(_) => return false,
    };
    Argon2::default().verify_password(plaintext.as_bytes(), &parsed).is_ok()
}

// --------------------------------- TOTP -------------------------------------

/// Generate a new random TOTP secret (base32-encoded, RFC 4648, no padding),
/// suitable for `totp_secret` and for rendering an otpauth:// URI / QR code.
pub fn generate_totp_secret() -> String {
    let mut bytes = [0u8; 20]; // 160-bit secret, standard for TOTP
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    base32::encode(base32::Alphabet::Rfc4648 { padding: false }, &bytes)
}

pub fn totp_provisioning_uri(secret_b32: &str, org_name: &str, email: &str) -> String {
    format!(
        "otpauth://totp/Verasens:{email}?secret={secret}&issuer=Verasens%20({org})&algorithm=SHA1&digits=6&period=30",
        email = urlencode(email),
        secret = secret_b32,
        org = urlencode(org_name),
    )
}

/// Verify a 6-digit TOTP code against a base32 secret, allowing +/-1 step of
/// clock skew (standard practice).
pub fn verify_totp(secret_b32: &str, code: &str) -> bool {
    let secret_bytes = match base32::decode(base32::Alphabet::Rfc4648 { padding: false }, secret_b32) {
        Some(b) => b,
        None => return false,
    };
    let totp = match totp_rs::TOTP::new(
        totp_rs::Algorithm::SHA1,
        6,
        1,
        30,
        secret_bytes,
    ) {
        Ok(t) => t,
        Err(_) => return false,
    };
    totp.check_current(code).unwrap_or(false)
}

fn urlencode(s: &str) -> String {
    let mut out = String::new();
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{:02X}", b)),
        }
    }
    out
}

// ------------------------------- API keys -----------------------------------

pub const API_KEY_LIVE_PREFIX: &str = "btcpc_live_";

/// A freshly-minted API key: `raw` is shown to the caller exactly once and
/// never persisted; `prefix` + `hash` are what get stored.
pub struct NewApiKey {
    pub raw: String,
    pub prefix: String,
    pub hash: String,
}

/// Generate a new API key secret. Format: `btcpc_live_<32 hex chars derived
/// from 16 random bytes>`. The full raw string is what the caller sends as
/// the Bearer token; `prefix` is a short, non-secret slice used for display
/// / audit-log correlation without ever re-deriving the secret from it.
pub fn generate_api_key() -> NewApiKey {
    let mut bytes = [0u8; 24];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    let secret_hex = hex::encode(bytes);
    let raw = format!("{API_KEY_LIVE_PREFIX}{secret_hex}");
    // Prefix used for lookup/display: scheme + first 8 hex chars. Never
    // enough to reconstruct the secret (24 bytes total vs 4 exposed).
    let prefix = format!("{API_KEY_LIVE_PREFIX}{}", &secret_hex[..8]);
    let hash = hash_api_key(&raw);
    NewApiKey { raw, prefix, hash }
}

/// Salted SHA-256 hash of the raw API key. The salt is a fixed
/// application-level pepper combined with the key itself (the key is
/// already a 24-byte random secret, so this is a keyed hash rather than a
/// password-style slow hash — matches common API-key storage practice,
/// distinct from the argon2id path used for human passwords).
pub fn hash_api_key(raw: &str) -> String {
    let pepper = std::env::var("VERASENS_API_KEY_PEPPER").unwrap_or_else(|_| "hone-dev-pepper".to_string());
    let mut hasher = Sha256::new();
    hasher.update(pepper.as_bytes());
    hasher.update(b":");
    hasher.update(raw.as_bytes());
    hex::encode(hasher.finalize())
}

pub fn verify_api_key(raw: &str, stored_hash: &str) -> bool {
    let computed = hash_api_key(raw);
    // Constant-time compare to avoid timing side-channels on hash equality.
    computed.as_bytes().ct_eq(stored_hash.as_bytes()).into()
}

/// Extract the display prefix from a raw API key the client presented, for
/// the `api_key` table lookup (`WHERE prefix = ...`) prior to verifying the
/// full hash.
pub fn prefix_of(raw: &str) -> Option<String> {
    if !raw.starts_with(API_KEY_LIVE_PREFIX) {
        return None;
    }
    let secret_part = &raw[API_KEY_LIVE_PREFIX.len()..];
    if secret_part.len() < 8 {
        return None;
    }
    Some(format!("{API_KEY_LIVE_PREFIX}{}", &secret_part[..8]))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn password_hash_roundtrip() {
        let h = hash_password("correct horse battery staple").unwrap();
        assert!(!h.contains("correct horse"));
        assert!(verify_password("correct horse battery staple", &h));
        assert!(!verify_password("wrong password", &h));
    }

    #[test]
    fn totp_roundtrip() {
        let secret = generate_totp_secret();
        let secret_bytes = base32::decode(base32::Alphabet::Rfc4648 { padding: false }, &secret).unwrap();
        let totp = totp_rs::TOTP::new(totp_rs::Algorithm::SHA1, 6, 1, 30, secret_bytes).unwrap();
        let code = totp.generate_current().unwrap();
        assert!(verify_totp(&secret, &code));
        assert!(!verify_totp(&secret, "000000") || code == "000000");
    }

    #[test]
    fn api_key_roundtrip_and_prefix_lookup() {
        let key = generate_api_key();
        assert!(key.raw.starts_with(API_KEY_LIVE_PREFIX));
        assert_eq!(prefix_of(&key.raw).unwrap(), key.prefix);
        assert!(verify_api_key(&key.raw, &key.hash));
        assert!(!verify_api_key("btcpc_live_deadbeefdeadbeefdeadbeefdeadbeefdeadbeefdeadbeef", &key.hash));
    }

    #[test]
    fn api_key_hash_never_contains_raw_secret() {
        let key = generate_api_key();
        assert!(!key.hash.contains(&key.raw));
    }
}
