//! Encrypted secret store.
//!
//! Source of truth: `~/.hone/secrets.enc` (AES-256-GCM, key from hw fingerprint or
//! `HONE_SECRETS_PASSPHRASE` env var). RocksDB CF_META prefix `secret:` is a runtime
//! index populated at boot and kept in sync on every write. Deleting the file wipes all
//! secrets cleanly.
#![allow(dead_code)]

#![allow(dead_code)]
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use anyhow::{anyhow, Result};
use aes_gcm::{Aes256Gcm, Key, Nonce, KeyInit};
use aes_gcm::aead::{Aead, OsRng};
use aes_gcm::aead::rand_core::RngCore;
use sha2::{Digest, Sha256};
use tracing::{info, warn};

use crate::store::Store;

const MAGIC: &[u8] = b"HONE_SECRETS_V1";
const NONCE_LEN: usize = 12;

pub struct SecretStore {
    file_path: PathBuf,
    cipher:    Aes256Gcm,
    store:     Store,
}

impl SecretStore {
    /// Open or create the secret store. Key is derived from `HONE_SECRETS_PASSPHRASE`
    /// env var if set, otherwise from `hw_fingerprint`.
    pub fn open(data_dir: &Path, hw_fingerprint: &str, store: Store) -> Result<Self> {
        let key_material = std::env::var("HONE_SECRETS_PASSPHRASE")
            .unwrap_or_else(|_| hw_fingerprint.to_owned());
        let key_bytes = derive_key(key_material.as_bytes());
        let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key_bytes));
        let file_path = data_dir.join("secrets.enc");

        let ss = Self { file_path, cipher, store };

        // Populate RocksDB index from file at boot.
        if ss.file_path.exists() {
            match ss.load_file() {
                Ok(map) => {
                    for (k, v) in &map {
                        let _ = ss.store.state_set(&format!("secret:{}", k), v.as_bytes());
                    }
                    info!("[secrets] loaded {} secret(s) from {:?}", map.len(), ss.file_path);
                }
                Err(e) => warn!("[secrets] failed to load secrets file: {} — starting empty", e),
            }
        } else {
            info!("[secrets] no secrets file at {:?} — starting empty", ss.file_path);
        }

        Ok(ss)
    }

    pub fn get(&self, key: &str) -> Option<String> {
        self.store.state_get(&format!("secret:{}", key))
            .and_then(|b| String::from_utf8(b).ok())
    }

    pub fn set(&self, key: &str, value: &str) -> Result<()> {
        self.store.state_set(&format!("secret:{}", key), value.as_bytes())
            .map_err(|e| anyhow!("rocksdb write failed: {}", e))?;
        self.flush_to_file()
    }

    pub fn delete(&self, key: &str) -> Result<()> {
        let _ = self.store.state_delete(&format!("secret:{}", key));
        self.flush_to_file()
    }

    pub fn scan_prefix(&self, prefix: &str) -> Vec<(String, String)> {
        let db_prefix = format!("secret:{}", prefix);
        self.store.state_scan_prefix(&db_prefix)
            .into_iter()
            .filter_map(|(k, v)| {
                let key = k.strip_prefix("secret:")?.to_owned();
                let val = String::from_utf8(v).ok()?;
                Some((key, val))
            })
            .collect()
    }

    pub fn export_plaintext(&self, path: &Path) -> Result<()> {
        let map = self.load_file().unwrap_or_default();
        let json = serde_json::to_vec_pretty(&map)?;
        std::fs::write(path, json)?;
        Ok(())
    }

    pub fn import_file(&self, path: &Path, passphrase: Option<&str>) -> Result<usize> {
        let raw = std::fs::read(path)?;
        let map: HashMap<String, String> = if let Some(pp) = passphrase {
            let key_bytes = derive_key(pp.as_bytes());
            let cipher = Aes256Gcm::new(Key::<Aes256Gcm>::from_slice(&key_bytes));
            decrypt_map(&cipher, &raw)?
        } else {
            serde_json::from_slice(&raw)?
        };
        let count = map.len();
        for (k, v) in &map {
            let _ = self.store.state_set(&format!("secret:{}", k), v.as_bytes());
        }
        self.flush_to_file()?;
        Ok(count)
    }

    // ── Internal helpers ─────────────────────────────────────────────────────

    fn all_secrets(&self) -> HashMap<String, String> {
        self.store.state_scan_prefix("secret:")
            .into_iter()
            .filter_map(|(k, v)| {
                let key = k.strip_prefix("secret:")?.to_owned();
                let val = String::from_utf8(v).ok()?;
                Some((key, val))
            })
            .collect()
    }

    fn flush_to_file(&self) -> Result<()> {
        let map = self.all_secrets();
        let plaintext = serde_json::to_vec(&map)?;
        let ciphertext = encrypt_map(&self.cipher, &plaintext)?;
        if let Some(parent) = self.file_path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        // Write to temp file then rename for atomicity.
        let tmp = self.file_path.with_extension("enc.tmp");
        std::fs::write(&tmp, &ciphertext)?;
        std::fs::rename(&tmp, &self.file_path)?;
        Ok(())
    }

    fn load_file(&self) -> Result<HashMap<String, String>> {
        let raw = std::fs::read(&self.file_path)?;
        decrypt_map(&self.cipher, &raw)
    }
}

fn derive_key(material: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(b"hone-secret-store-v1:");
    hasher.update(material);
    hasher.finalize().into()
}

fn encrypt_map(cipher: &Aes256Gcm, plaintext: &[u8]) -> Result<Vec<u8>> {
    let mut nonce_bytes = [0u8; NONCE_LEN];
    OsRng.fill_bytes(&mut nonce_bytes);
    let nonce = Nonce::from_slice(&nonce_bytes);
    let ciphertext = cipher.encrypt(nonce, plaintext)
        .map_err(|e| anyhow!("encrypt failed: {:?}", e))?;
    let mut out = Vec::with_capacity(MAGIC.len() + NONCE_LEN + ciphertext.len());
    out.extend_from_slice(MAGIC);
    out.extend_from_slice(&nonce_bytes);
    out.extend_from_slice(&ciphertext);
    Ok(out)
}

fn decrypt_map(cipher: &Aes256Gcm, data: &[u8]) -> Result<HashMap<String, String>> {
    if data.len() < MAGIC.len() + NONCE_LEN {
        return Err(anyhow!("secrets file too short"));
    }
    if &data[..MAGIC.len()] != MAGIC {
        return Err(anyhow!("secrets file has wrong magic — wrong key or corrupted"));
    }
    let nonce = Nonce::from_slice(&data[MAGIC.len()..MAGIC.len() + NONCE_LEN]);
    let ciphertext = &data[MAGIC.len() + NONCE_LEN..];
    let plaintext = cipher.decrypt(nonce, ciphertext)
        .map_err(|_| anyhow!("secrets decryption failed — wrong passphrase?"))?;
    Ok(serde_json::from_slice(&plaintext)?)
}
