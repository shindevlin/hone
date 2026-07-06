//! Recoverable encrypted keystore — the fix for HONE's deepest gap.
//!
//! # The problem this solves
//!
//! Before this, `Wallet::save_to_file` wrote only public keys, on the assumption
//! that the user copied down their mnemonic. In practice the mnemonic was never
//! durably captured, so accounts became **unrecoverable** — we owned accounts we
//! could not sign for. See `docs/GENESIS_JULY4_2026.md`.
//!
//! # What this provides
//!
//! A portable `<account>.keystore.json` file that encrypts the wallet's secret
//! (the BIP39 mnemonic) at rest, unlockable with a user-chosen password:
//!
//!   * **KDF: Argon2id** — memory-hard, resistant to GPU/ASIC brute force. This
//!     is deliberately NOT the node's `secret_store::derive_key` (a single
//!     SHA-256), which is only safe for high-entropy hardware-derived keys, not
//!     human passwords.
//!   * **Cipher: AES-256-GCM** — authenticated encryption; tampering is detected
//!     on decrypt (the GCM tag fails), so a corrupted or altered keystore is
//!     rejected rather than silently mis-decrypted.
//!   * **Format** modeled on the widely-audited **Ethereum keystore V3** shape:
//!     `{ version, account, crypto: { kdf, kdfparams, cipher, cipherparams,
//!     ciphertext, mac } }` — inspectable, standard, and self-describing.
//!
//! The password never leaves the device; only ciphertext is ever written or
//! (optionally, later) uploaded to a relay for backup.

use aes_gcm::aead::{Aead, KeyInit, OsRng};
use aes_gcm::{Aes256Gcm, Key, Nonce};
use anyhow::{anyhow, bail, Context, Result};
use argon2::{Argon2, Algorithm, Params, Version};
use rand::RngCore;
use serde::{Deserialize, Serialize};
use std::path::Path;

/// Keystore format version (the file schema, not the wallet).
pub const KEYSTORE_VERSION: u8 = 1;

const SALT_LEN: usize = 16;
const NONCE_LEN: usize = 12; // AES-GCM standard nonce
const KEY_LEN: usize = 32; // AES-256

// Argon2id parameters. Tuned for interactive wallet unlock: strong against
// offline brute force while remaining ~sub-second on commodity hardware. Stored
// in the file so they can be raised over time without breaking old keystores.
const ARGON_M_COST_KIB: u32 = 64 * 1024; // 64 MiB
const ARGON_T_COST: u32 = 3; // iterations
const ARGON_P_COST: u32 = 1; // lanes

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct KdfParams {
    /// Argon2 variant — always "argon2id".
    pub variant: String,
    /// Memory cost in KiB.
    pub m_cost: u32,
    /// Time cost (iterations).
    pub t_cost: u32,
    /// Parallelism (lanes).
    pub p_cost: u32,
    /// Hex-encoded salt.
    pub salt: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CryptoBlock {
    /// Key-derivation function id — "argon2id".
    pub kdf: String,
    pub kdfparams: KdfParams,
    /// Symmetric cipher id — "aes-256-gcm".
    pub cipher: String,
    /// Hex-encoded 12-byte GCM nonce.
    pub nonce: String,
    /// Hex-encoded ciphertext (includes the GCM auth tag).
    pub ciphertext: String,
}

/// The on-disk keystore file. `<account>.keystore.json`.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Keystore {
    pub version: u8,
    /// Account name this keystore belongs to (public, for identification).
    pub account: String,
    /// What the ciphertext holds once decrypted — "bip39-mnemonic".
    pub secret_kind: String,
    pub crypto: CryptoBlock,
}

impl Keystore {
    /// Encrypt `secret` (typically a BIP39 mnemonic phrase) under `password`,
    /// producing a keystore for `account`. Uses a fresh random salt + nonce.
    pub fn seal(account: &str, secret: &str, password: &str) -> Result<Self> {
        if password.is_empty() {
            bail!("keystore password must not be empty");
        }
        let mut salt = [0u8; SALT_LEN];
        OsRng.fill_bytes(&mut salt);
        let mut nonce_bytes = [0u8; NONCE_LEN];
        OsRng.fill_bytes(&mut nonce_bytes);

        let key = derive_key(password.as_bytes(), &salt, ARGON_M_COST_KIB, ARGON_T_COST, ARGON_P_COST)?;
        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key));
        let ciphertext = cipher
            .encrypt(Nonce::from_slice(&nonce_bytes), secret.as_bytes())
            .map_err(|e| anyhow!("keystore encryption failed: {e:?}"))?;

        Ok(Keystore {
            version: KEYSTORE_VERSION,
            account: account.to_string(),
            secret_kind: "bip39-mnemonic".to_string(),
            crypto: CryptoBlock {
                kdf: "argon2id".to_string(),
                kdfparams: KdfParams {
                    variant: "argon2id".to_string(),
                    m_cost: ARGON_M_COST_KIB,
                    t_cost: ARGON_T_COST,
                    p_cost: ARGON_P_COST,
                    salt: hex::encode(salt),
                },
                cipher: "aes-256-gcm".to_string(),
                nonce: hex::encode(nonce_bytes),
                ciphertext: hex::encode(ciphertext),
            },
        })
    }

    /// Decrypt the secret with `password`. Returns an error on wrong password or
    /// any tampering (the GCM tag will fail to verify).
    pub fn open(&self, password: &str) -> Result<String> {
        if self.crypto.kdf != "argon2id" {
            bail!("unsupported kdf '{}'", self.crypto.kdf);
        }
        if self.crypto.cipher != "aes-256-gcm" {
            bail!("unsupported cipher '{}'", self.crypto.cipher);
        }
        let salt = hex::decode(&self.crypto.kdfparams.salt).context("bad salt hex")?;
        let nonce = hex::decode(&self.crypto.nonce).context("bad nonce hex")?;
        let ciphertext = hex::decode(&self.crypto.ciphertext).context("bad ciphertext hex")?;
        if nonce.len() != NONCE_LEN {
            bail!("bad nonce length");
        }

        let key = derive_key(
            password.as_bytes(),
            &salt,
            self.crypto.kdfparams.m_cost,
            self.crypto.kdfparams.t_cost,
            self.crypto.kdfparams.p_cost,
        )?;
        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key));
        let plaintext = cipher
            .decrypt(Nonce::from_slice(&nonce), ciphertext.as_ref())
            .map_err(|_| anyhow!("keystore decrypt failed: wrong password or corrupted/tampered file"))?;
        String::from_utf8(plaintext).context("decrypted secret is not valid UTF-8")
    }

    /// Write the keystore to `path` as pretty JSON.
    pub fn save(&self, path: &Path) -> Result<()> {
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        std::fs::write(path, serde_json::to_string_pretty(self)?)
            .with_context(|| format!("writing keystore {}", path.display()))?;
        Ok(())
    }

    /// Load a keystore file from `path` (does not decrypt).
    pub fn load(path: &Path) -> Result<Self> {
        let raw = std::fs::read_to_string(path)
            .with_context(|| format!("reading keystore {}", path.display()))?;
        let ks: Keystore = serde_json::from_str(&raw).context("parsing keystore json")?;
        if ks.version != KEYSTORE_VERSION {
            bail!("unsupported keystore version {} (expected {})", ks.version, KEYSTORE_VERSION);
        }
        Ok(ks)
    }
}

/// Argon2id password → 32-byte AES key.
fn derive_key(password: &[u8], salt: &[u8], m_cost: u32, t_cost: u32, p_cost: u32) -> Result<[u8; KEY_LEN]> {
    let params = Params::new(m_cost, t_cost, p_cost, Some(KEY_LEN))
        .map_err(|e| anyhow!("bad argon2 params: {e}"))?;
    let argon = Argon2::new(Algorithm::Argon2id, Version::V0x13, params);
    let mut out = [0u8; KEY_LEN];
    argon
        .hash_password_into(password, salt, &mut out)
        .map_err(|e| anyhow!("argon2id derivation failed: {e}"))?;
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MNEMONIC: &str =
        "legal winner thank year wave sausage worth useful legal winner thank yellow";

    #[test]
    fn round_trip_recovers_the_secret() {
        let ks = Keystore::seal("bullship", MNEMONIC, "correct horse battery staple").unwrap();
        let out = ks.open("correct horse battery staple").unwrap();
        assert_eq!(out, MNEMONIC);
        assert_eq!(ks.account, "bullship");
        assert_eq!(ks.secret_kind, "bip39-mnemonic");
    }

    #[test]
    fn wrong_password_is_rejected() {
        let ks = Keystore::seal("bullship", MNEMONIC, "right-password").unwrap();
        let err = ks.open("wrong-password").unwrap_err();
        assert!(err.to_string().contains("wrong password") || err.to_string().contains("decrypt"));
    }

    #[test]
    fn tampered_ciphertext_is_rejected() {
        let mut ks = Keystore::seal("bullship", MNEMONIC, "pw").unwrap();
        // Flip a byte in the ciphertext — GCM auth must catch it.
        let mut ct = hex::decode(&ks.crypto.ciphertext).unwrap();
        ct[0] ^= 0xff;
        ks.crypto.ciphertext = hex::encode(ct);
        assert!(ks.open("pw").is_err());
    }

    #[test]
    fn empty_password_rejected_on_seal() {
        assert!(Keystore::seal("x", MNEMONIC, "").is_err());
    }

    #[test]
    fn save_and_load_preserves_and_decrypts() {
        let dir = std::env::temp_dir().join(format!("hone_ks_{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("bullship.keystore.json");
        let ks = Keystore::seal("bullship", MNEMONIC, "pw123").unwrap();
        ks.save(&path).unwrap();
        let loaded = Keystore::load(&path).unwrap();
        assert_eq!(loaded.open("pw123").unwrap(), MNEMONIC);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn distinct_salts_produce_distinct_ciphertext() {
        // Same secret + password must not produce identical files (fresh salt/nonce).
        let a = Keystore::seal("x", MNEMONIC, "pw").unwrap();
        let b = Keystore::seal("x", MNEMONIC, "pw").unwrap();
        assert_ne!(a.crypto.ciphertext, b.crypto.ciphertext);
        assert_ne!(a.crypto.kdfparams.salt, b.crypto.kdfparams.salt);
    }
}
