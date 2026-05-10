//! TOTP 2FA implementation.
//!
//! TOTP = HOTP(K, floor(unix_time / 30)), 6-digit, HMAC-SHA1.
//! Uses existing hmac + sha1 crates — no new dependency needed.
#![allow(dead_code)]

#![allow(dead_code)]
use hmac::{Hmac, Mac};
use sha1::Sha1;

type HmacSha1 = Hmac<Sha1>;

/// Compute the current 6-digit TOTP code for a base32-encoded secret.
pub fn current_code(secret_b32: &str) -> Option<u32> {
    let key = base32_decode(secret_b32)?;
    let counter = now_ms() / 1000 / 30;
    Some(hotp(&key, counter))
}

/// Verify a user-supplied code. Accepts current window ± 1 (clock skew).
pub fn verify(secret_b32: &str, code: u32) -> bool {
    let key = match base32_decode(secret_b32) { Some(k) => k, None => return false };
    let counter = now_ms() / 1000 / 30;
    for delta in [0u64, 1, counter.saturating_sub(1).wrapping_sub(counter - 1)] {
        let c = counter.wrapping_add(delta).min(counter + 1);
        if c > counter + 1 { continue; }
        if hotp(&key, c) == code { return true; }
    }
    // Check counter - 1 explicitly for backward window.
    if counter > 0 && hotp(&key, counter - 1) == code { return true; }
    false
}

/// Generate a random 20-byte (160-bit) base32 secret.
pub fn generate_secret() -> String {
    use rand::RngCore;
    let mut bytes = [0u8; 20];
    rand::thread_rng().fill_bytes(&mut bytes);
    base32_encode(&bytes)
}

/// Build an otpauth:// URI for QR code scanners.
pub fn otpauth_uri(account: &str, secret_b32: &str) -> String {
    format!(
        "otpauth://totp/BTCPC%3A{}?secret={}&issuer=BTCPC&algorithm=SHA1&digits=6&period=30",
        percent_encode(account),
        secret_b32,
    )
}

/// Generate 8 single-use backup codes (hex strings, no spaces).
pub fn generate_backup_codes() -> Vec<String> {
    use rand::RngCore;
    (0..8).map(|_| {
        let mut bytes = [0u8; 6];
        rand::thread_rng().fill_bytes(&mut bytes);
        hex::encode(bytes)
    }).collect()
}

/// Hash a backup code for safe storage (SHA-256 hex).
pub fn hash_backup_code(code: &str) -> String {
    use sha2::{Digest, Sha256};
    hex::encode(Sha256::digest(code.as_bytes()))
}

// ── Internal helpers ─────────────────────────────────────────────────────────

fn hotp(key: &[u8], counter: u64) -> u32 {
    let msg = counter.to_be_bytes();
    let mut mac = HmacSha1::new_from_slice(key).expect("HMAC accepts any key size");
    mac.update(&msg);
    let result = mac.finalize().into_bytes();

    // Dynamic truncation per RFC 4226.
    let offset = (result[19] & 0x0f) as usize;
    let code = ((result[offset] as u32 & 0x7f) << 24)
        | ((result[offset + 1] as u32) << 16)
        | ((result[offset + 2] as u32) << 8)
        | (result[offset + 3] as u32);
    code % 1_000_000
}

fn now_ms() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now().duration_since(UNIX_EPOCH).unwrap_or_default().as_millis() as u64
}

/// Minimal RFC 4648 base32 (upper-case alphabet, no padding required on decode).
fn base32_decode(s: &str) -> Option<Vec<u8>> {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let s = s.to_uppercase().replace('=', "");
    let mut bits: u32 = 0;
    let mut bit_count: u32 = 0;
    let mut out = Vec::new();
    for ch in s.bytes() {
        let val = ALPHABET.iter().position(|&b| b == ch)? as u32;
        bits = (bits << 5) | val;
        bit_count += 5;
        if bit_count >= 8 {
            bit_count -= 8;
            out.push((bits >> bit_count) as u8);
        }
    }
    Some(out)
}

fn base32_encode(data: &[u8]) -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZ234567";
    let mut out = String::new();
    let mut bits: u32 = 0;
    let mut bit_count: u32 = 0;
    for &byte in data {
        bits = (bits << 8) | byte as u32;
        bit_count += 8;
        while bit_count >= 5 {
            bit_count -= 5;
            out.push(ALPHABET[(bits >> bit_count & 0x1f) as usize] as char);
        }
    }
    if bit_count > 0 {
        out.push(ALPHABET[((bits << (5 - bit_count)) & 0x1f) as usize] as char);
    }
    out
}

fn percent_encode(s: &str) -> String {
    s.chars().map(|c| match c {
        'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => c.to_string(),
        c => format!("%{:02X}", c as u32),
    }).collect()
}
