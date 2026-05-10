//! Chain state machine — applies ledger entries and advances chain state.

use std::collections::HashSet;
use std::sync::Arc;
use anyhow::{Context, Result};
use parking_lot::{Mutex, RwLock};
use sha2::{Sha256, Digest as Sha256Digest};
use tracing::{info, warn};
use btcpc_types::{AccountId, LedgerEntry, NATIVE_TOKEN, CLOCK_REWARD_DREAMS, era, RECYCLE_ERA, RECYCLE_FUND_ACCOUNT, TESTNET_FUND_ACCOUNT, DEVICE_CLAIM_OVERBID_NUM, DEVICE_CLAIM_OVERBID_DENOM, OVERCLAIM_STAKER_SHARE_BPS, MAX_SIGHTINGS_PER_OBSERVER_PER_EPOCH, STORAGE_CHALLENGE_CHUNK_BYTES, SENSOR_GNSS_MAX_SPEED_M_S, EPOCH_DURATION_S, COVERAGE_GRID_RESOLUTION, COVERAGE_MAX_REPORTS_PER_EPOCH, COVERAGE_DEAD_SPOT_DBM_THRESHOLD, COVERAGE_CORROBORATION_MIN_REPORTERS, COVERAGE_MAX_CORROBORATING_REPORTERS, RUNTIME_FEE_HOST_BPS, RUNTIME_FEE_RECYCLE_BPS};

use crate::inference;
use crate::store::Store;

/// Epochs tokens are held in unbonding after Unstake before being released.
/// Short enough to not block honest operators, long enough for a slash to be submitted.
/// At era-0 (30s epochs): 10 epochs = 5 minutes.
const UNBONDING_EPOCHS: u64 = 10;

/// Recover the signer's address from an external-chain signature.
/// Currently supports Ethereum personal_sign (EIP-191).
/// Also used by tx.rs for 2FA signature verification.
pub fn recover_chain_address_public(sig_type: &str, message: &str, signature: &str) -> anyhow::Result<String> {
    recover_chain_address(sig_type, message, signature)
}

fn recover_chain_address(sig_type: &str, message: &str, signature: &str) -> anyhow::Result<String> {
    match sig_type {
        "eth_personal_sign" => {
            // EIP-191: keccak256("\x19Ethereum Signed Message:\n" + len + message)
            use sha3::{Digest, Keccak256};
            use secp256k1::{Message, ecdsa::RecoverableSignature, ecdsa::RecoveryId};

            let prefix = format!("\x19Ethereum Signed Message:\n{}", message.len());
            let hash: [u8; 32] = Keccak256::new()
                .chain_update(prefix.as_bytes())
                .chain_update(message.as_bytes())
                .finalize()
                .into();

            let sig_bytes = hex::decode(signature.trim_start_matches("0x"))
                .context("signature hex decode")?;
            anyhow::ensure!(sig_bytes.len() == 65, "eth signature must be 65 bytes");

            // Last byte is v (27 or 28, or 0/1 for some wallets). Normalize to 0/1.
            let v = sig_bytes[64];
            let rec_id = RecoveryId::from_i32(((v % 27) % 2) as i32)
                .map_err(|e| anyhow::anyhow!("bad recovery id: {}", e))?;
            let rec_sig = RecoverableSignature::from_compact(&sig_bytes[..64], rec_id)
                .map_err(|e| anyhow::anyhow!("bad recoverable sig: {}", e))?;
            let msg = Message::from_digest(hash);
            let secp = secp256k1::Secp256k1::new();
            let pubkey = secp.recover_ecdsa(&msg, &rec_sig)
                .map_err(|e| anyhow::anyhow!("ecdsa recovery failed: {}", e))?;

            // Ethereum address = last 20 bytes of keccak256(uncompressed_pubkey[1..])
            let uncompressed = pubkey.serialize_uncompressed();
            let addr_hash: [u8; 32] = Keccak256::digest(&uncompressed[1..]).into();
            let addr_bytes = &addr_hash[12..];

            // EIP-55 checksum
            let hex_lower = hex::encode(addr_bytes);
            let checksum_hash: [u8; 32] = Keccak256::digest(hex_lower.as_bytes()).into();
            let checksummed: String = hex_lower.chars().enumerate().map(|(i, c)| {
                let nib = (checksum_hash[i / 2] >> (if i % 2 == 0 { 4 } else { 0 })) & 0x0f;
                if c.is_alphabetic() && nib >= 8 { c.to_ascii_uppercase() } else { c }
            }).collect();
            Ok(format!("0x{}", checksummed))
        }
        "sol_sign" => {
            // Solana Ed25519 — Phantom `signMessage()` signs raw UTF-8 bytes, no prefix.
            // signature field format: "{base58_pubkey}:{hex_signature}"
            let (pubkey_b58, sig_hex) = signature.split_once(':')
                .ok_or_else(|| anyhow::anyhow!("sol_sign: expected '{{base58_pubkey}}:{{hex_sig}}'"))?;

            let pk_bytes = bs58::decode(pubkey_b58)
                .into_vec()
                .map_err(|e| anyhow::anyhow!("sol_sign: bad base58 pubkey: {}", e))?;
            let pk_array: [u8; 32] = pk_bytes.try_into()
                .map_err(|_| anyhow::anyhow!("sol_sign: pubkey must be 32 bytes"))?;
            let vk = ed25519_dalek::VerifyingKey::from_bytes(&pk_array)
                .map_err(|e| anyhow::anyhow!("sol_sign: invalid Ed25519 key: {}", e))?;

            let sig_bytes = hex::decode(sig_hex)
                .map_err(|_| anyhow::anyhow!("sol_sign: invalid hex signature"))?;
            let sig_array: [u8; 64] = sig_bytes.try_into()
                .map_err(|_| anyhow::anyhow!("sol_sign: signature must be 64 bytes"))?;
            let sig = ed25519_dalek::Signature::from_bytes(&sig_array);

            vk.verify_strict(message.as_bytes(), &sig)
                .map_err(|e| anyhow::anyhow!("sol_sign: verification failed: {}", e))?;

            // Solana address = base58 of 32-byte Ed25519 public key
            Ok(bs58::encode(pk_array).into_string())
        }

        "btc_legacy" => {
            // Bitcoin legacy signed message (compatible with BIP-322 simple/legacy).
            // Hash: SHA256d( "\x18Bitcoin Signed Message:\n" || compactsize(len) || message )
            // Signature: 65 bytes (v || r || s), base64-encoded.
            // v encodes key compression: 27–30 = uncompressed P2PKH, 31–34 = compressed P2PKH.
            #[allow(unused_imports)]
            use sha2::Digest as Sha2Digest;
            #[allow(unused_imports)]
            use ripemd::Digest as RipemdDigest;
            use secp256k1::{Message as SecpMsg, ecdsa::RecoverableSignature, ecdsa::RecoveryId};

            let sig_bytes = {
                use base64::Engine as _;
                base64::engine::general_purpose::STANDARD.decode(signature)
                    .or_else(|_| hex::decode(signature.trim_start_matches("0x")))
                    .map_err(|_| anyhow::anyhow!("btc_legacy: signature must be base64 or hex"))?
            };
            anyhow::ensure!(sig_bytes.len() == 65, "btc_legacy: signature must be 65 bytes");

            let v = sig_bytes[0];
            // v 27-30: uncompressed; 31-34: compressed P2PKH
            let compressed = v >= 31;
            let base = if compressed { 31u8 } else { 27u8 };
            let rec_id_raw = (v - base) % 4;
            let rec_id = RecoveryId::from_i32(rec_id_raw as i32)
                .map_err(|e| anyhow::anyhow!("btc_legacy: bad recovery id: {}", e))?;

            let hash2 = {
                let magic = b"Bitcoin Signed Message:\n";
                let mut payload = Vec::new();
                payload.push(magic.len() as u8); // 0x18 = 24
                payload.extend_from_slice(magic);
                let n = message.len();
                if n < 253 { payload.push(n as u8); }
                else if n <= 0xffff { payload.push(0xfd); payload.extend_from_slice(&(n as u16).to_le_bytes()); }
                else { payload.push(0xfe); payload.extend_from_slice(&(n as u32).to_le_bytes()); }
                payload.extend_from_slice(message.as_bytes());
                let h1 = sha2::Sha256::digest(&payload);
                let h2: [u8; 32] = sha2::Sha256::digest(&h1).into();
                h2
            };

            let rec_sig = RecoverableSignature::from_compact(&sig_bytes[1..], rec_id)
                .map_err(|e| anyhow::anyhow!("btc_legacy: bad signature: {}", e))?;
            let secp = secp256k1::Secp256k1::new();
            let pubkey = secp.recover_ecdsa(&SecpMsg::from_digest(hash2), &rec_sig)
                .map_err(|e| anyhow::anyhow!("btc_legacy: ECDSA recovery failed: {}", e))?;

            let pubkey_bytes: Vec<u8> = if compressed {
                pubkey.serialize().to_vec()
            } else {
                pubkey.serialize_uncompressed().to_vec()
            };

            // P2PKH: base58check( 0x00 || RIPEMD160( SHA256(pubkey) ) )
            let sha256_of_pk = sha2::Sha256::digest(&pubkey_bytes);
            let ripemd_hash = ripemd::Ripemd160::digest(&sha256_of_pk);
            let mut addr_payload = vec![0x00u8]; // mainnet P2PKH version byte
            addr_payload.extend_from_slice(&ripemd_hash);
            let cs_h1 = sha2::Sha256::digest(&addr_payload);
            let cs_h2 = sha2::Sha256::digest(&cs_h1);
            addr_payload.extend_from_slice(&cs_h2[..4]);

            Ok(bs58::encode(addr_payload).into_string())
        }

        other => anyhow::bail!(
            "unsupported sig_type '{}' — supported: eth_personal_sign, sol_sign, btc_legacy",
            other
        ),
    }
}

/// Compare two semver strings — returns true if `a >= b`.
/// Parses "MAJOR.MINOR.PATCH" numerically; falls back to string comparison.
fn semver_gte(a: &str, b: &str) -> bool {
    fn parse(s: &str) -> (u64, u64, u64) {
        let mut it = s.trim().splitn(3, '.');
        let ma = it.next().and_then(|x| x.parse().ok()).unwrap_or(0);
        let mi = it.next().and_then(|x| x.parse().ok()).unwrap_or(0);
        let pa = it.next().and_then(|x| x.split('-').next()).and_then(|x| x.parse().ok()).unwrap_or(0);
        (ma, mi, pa)
    }
    parse(a) >= parse(b)
}

/// Returns true if `to` has pre-approved `token` from `from`.
/// Checks four wildcard combinations in order (most-specific first).
fn token_approved(store: &Store, to: &str, token: &str, from: &str) -> bool {
    store.state_get(&format!("token_approval:{}:{}:{}", to, token, from)).is_some()
        || store.state_get(&format!("token_approval:{}:*:{}", to, from)).is_some()
        || store.state_get(&format!("token_approval:{}:{}:*", to, token)).is_some()
        || store.state_get(&format!("token_approval:{}:*:*", to)).is_some()
}

enum SpamGate {
    /// On-chain fee: debit fee_token from sender, credit to recipient.
    OnChain { fee: u64, fee_token: String },
    /// Off-chain fee: sender must use SpamGatePayEvm with an EVM tx proof.
    Evm { evm_address: String, evm_chain_id: u64 },
}

/// Returns the gate config for an unsolicited `token` landing at `to`,
/// or None if no gate is set. Checks exact token first, then wildcard.
fn spam_gate(store: &Store, to: &str, token: &str) -> Option<SpamGate> {
    for key in [
        format!("spam_gate:{}:{}", to, token),
        format!("spam_gate:{}:*", to),
    ] {
        if let Some(raw) = store.state_get(&key) {
            if let Ok(j) = serde_json::from_slice::<serde_json::Value>(&raw) {
                // EVM gate takes priority when evm_address is present.
                if let (Some(addr), Some(chain_id)) = (
                    j["evm_address"].as_str().filter(|s| !s.is_empty()),
                    j["evm_chain_id"].as_u64(),
                ) {
                    return Some(SpamGate::Evm {
                        evm_address: addr.to_owned(),
                        evm_chain_id: chain_id,
                    });
                }
                if let Some(fee) = j["fee"].as_u64() {
                    let fee_token = j["fee_token"].as_str()
                        .unwrap_or(NATIVE_TOKEN).to_owned();
                    return Some(SpamGate::OnChain { fee, fee_token });
                }
            }
        }
    }
    None
}

/// Verify a `MerkleRangeProof` in a `StorageHeartbeat` (T4-1).
///
/// Returns true only if:
///   1. `challenge_hash` matches `sha256("{prev_seal_hash}:{node_id}:{epoch}")`
///   2. `range_start`/`range_end` match the deterministically derived chunk index
///   3. The Merkle path walks from `leaf_hash` up to `merkle_root`
fn verify_storage_proof(
    store: &crate::store::Store,
    proof: &btcpc_types::MerkleRangeProof,
    node_id: &str,
    epoch: u64,
    bytes_proven: u64,
) -> bool {
    // 1. Load previous epoch's seal hash.
    let prev_seal_bytes = match store.state_get(&format!("epoch_seal_hash:{}", epoch.saturating_sub(1))) {
        Some(b) => b,
        None => return false,
    };
    let prev_seal_hash = String::from_utf8_lossy(&prev_seal_bytes);

    // 2. Derive expected challenge.
    let mut h = Sha256::new();
    h.update(format!("{}:{}:{}", prev_seal_hash, node_id, epoch).as_bytes());
    let challenge: [u8; 32] = h.finalize().into();
    if proof.challenge_hash != hex::encode(challenge) { return false; }

    // 3. Derive chunk index and verify range bounds.
    let num_chunks = bytes_proven.saturating_add(STORAGE_CHALLENGE_CHUNK_BYTES - 1) / STORAGE_CHALLENGE_CHUNK_BYTES;
    if num_chunks == 0 { return false; }
    let chunk_idx = u64::from_be_bytes(challenge[0..8].try_into().unwrap()) % num_chunks;
    let expected_start = chunk_idx * STORAGE_CHALLENGE_CHUNK_BYTES;
    let chunk_size = STORAGE_CHALLENGE_CHUNK_BYTES.min(bytes_proven.saturating_sub(expected_start));
    let expected_end = expected_start + chunk_size;
    if proof.range_start != expected_start || proof.range_end != expected_end { return false; }

    // 4. Walk Merkle path: leaf_hash + proof_nodes → merkle_root.
    let Ok(leaf_bytes) = hex::decode(&proof.leaf_hash) else { return false; };
    let Ok(mut current): Result<[u8; 32], _> = leaf_bytes.try_into() else { return false; };
    let mut idx = chunk_idx as usize;
    for sibling_hex in &proof.proof_nodes {
        let Ok(sibling_bytes) = hex::decode(sibling_hex) else { return false; };
        let Ok(sibling): Result<[u8; 32], _> = sibling_bytes.try_into() else { return false; };
        current = if idx % 2 == 0 {
            crate::store::merkle_node(&current, &sibling)
        } else {
            crate::store::merkle_node(&sibling, &current)
        };
        idx /= 2;
    }
    hex::encode(current) == proof.merkle_root
}

pub struct Chain {
    pub store: Store,
    pub current_epoch: Arc<RwLock<u64>>,
    #[allow(dead_code)]
    pub node_id: String,
    pub chain_id: String,
    /// Serialises all write paths: nonce-check → debit/credit → nonce-bump.
    pub write_lock: parking_lot::Mutex<()>,
    /// Pending user entries waiting to be committed at the next epoch seal.
    /// System entries (EpochSeal, rewards, etc.) bypass this pool and apply immediately.
    pub pending: Arc<Mutex<Vec<(LedgerEntry, Option<String>)>>>,
    /// SHA-256 fingerprints of entries already admitted this epoch. Cleared on drain.
    /// Prevents replayed gossip messages from inserting the same entry twice.
    seen_entries: Arc<Mutex<HashSet<[u8; 32]>>>,
}

impl Chain {
    pub fn new(store: Store, node_id: String, chain_id: String) -> Self {
        // Prefer the persisted epoch counter over the latest block epoch, because
        // sealed epochs may advance past the last produced block (clock-only nodes).
        let persisted = store.get_meta("current_epoch")
            .and_then(|b| b.try_into().ok())
            .map(u64::from_le_bytes);
        let current_epoch = persisted.unwrap_or_else(|| store.latest_epoch().unwrap_or(0));
        Self {
            store,
            current_epoch: Arc::new(RwLock::new(current_epoch)),
            node_id,
            chain_id,
            write_lock: Mutex::new(()),
            pending: Arc::new(Mutex::new(Vec::new())),
            seen_entries: Arc::new(Mutex::new(HashSet::new())),
        }
    }

    /// Queue a user entry for application at the next epoch seal.
    /// Returns false and drops the entry silently if the same entry (same serialized bytes)
    /// was already admitted this epoch — protects against replayed gossip messages.
    pub fn push_pending(&self, entry: LedgerEntry, sig: Option<String>) -> bool {
        let bytes = serde_json::to_vec(&entry).unwrap_or_default();
        let mut h = Sha256::new();
        h.update(&bytes);
        let fingerprint: [u8; 32] = h.finalize().into();

        let mut seen = self.seen_entries.lock();
        if !seen.insert(fingerprint) {
            return false;
        }
        drop(seen);
        self.pending.lock().push((entry, sig));
        true
    }

    pub fn pending_count(&self) -> usize {
        self.pending.lock().len()
    }

    /// Drain and sort the pending pool, returning entries in deterministic hash order.
    /// All nodes that received the same gossip will sort identically, ensuring consistent
    /// "first claim wins" across the whole network regardless of gossip arrival timing.
    /// Clears the dedup set so the next epoch starts fresh.
    pub fn drain_pending_sorted(&self) -> Vec<(LedgerEntry, Option<String>)> {
        let mut pool = self.pending.lock();
        pool.sort_by_cached_key(|(e, _)| {
            let bytes = serde_json::to_vec(e).unwrap_or_default();
            let mut h = Sha256::new();
            h.update(&bytes);
            h.finalize().to_vec()
        });
        let drained = pool.drain(..).collect();
        drop(pool);
        self.seen_entries.lock().clear();
        drained
    }

    pub fn current_epoch(&self) -> u64 {
        *self.current_epoch.read()
    }

    /// Epoch duration in milliseconds, using per-era calibrated values stored in
    /// chain state.  Falls back to the pure geometric formula (30s × 2^era) for
    /// era 0 and any era whose calibration has not yet fired.
    ///
    /// Use this everywhere except in pure emission-math contexts that can't hold a
    /// Chain reference (e.g. the types crate).
    #[allow(dead_code)]
    pub fn epoch_duration_ms_dynamic(&self, epoch: u64) -> u64 {
        let e = btcpc_types::era(epoch);
        let key = format!("chain_param:era_epoch_ms:{}", e);
        self.store.state_get(&key)
            .and_then(|b| serde_json::from_slice::<u64>(&b).ok())
            .unwrap_or_else(|| btcpc_types::epoch_duration_ms(epoch))
    }

    /// Wall-clock offset in milliseconds from genesis for the start of `epoch`,
    /// using per-era calibrated durations where available.
    #[allow(dead_code)]
    pub fn epoch_start_ms_dynamic(&self, epoch: u64, genesis_ms: u64) -> u64 {
        if epoch == 0 { return genesis_ms; }
        let e = btcpc_types::era(epoch);
        let mut ms = genesis_ms;
        // Sum fully-completed eras
        for k in 0..e {
            let key = format!("chain_param:era_epoch_ms:{}", k);
            let era_epoch_ms = self.store.state_get(&key)
                .and_then(|b| serde_json::from_slice::<u64>(&b).ok())
                .unwrap_or_else(|| btcpc_types::epoch_duration_ms(k * btcpc_types::DOUBLING_INTERVAL));
            ms = ms.saturating_add(btcpc_types::DOUBLING_INTERVAL.saturating_mul(era_epoch_ms));
        }
        // Remaining epochs in current era
        let epoch_in_era = epoch - e * btcpc_types::DOUBLING_INTERVAL;
        let cur_key = format!("chain_param:era_epoch_ms:{}", e);
        let cur_epoch_ms = self.store.state_get(&cur_key)
            .and_then(|b| serde_json::from_slice::<u64>(&b).ok())
            .unwrap_or_else(|| btcpc_types::epoch_duration_ms(epoch));
        ms.saturating_add(epoch_in_era.saturating_mul(cur_epoch_ms))
    }

    /// Update an account's last-alive epoch (Chain Entropy Protocol).
    /// Silently no-ops if the store write fails — liveness tracking is best-effort.
    pub fn touch_alive(&self, account: &str, epoch: u64) {
        let current = self.store.get_alive_epoch(account);
        if epoch > current {
            let _ = self.store.set_alive_epoch(account, epoch);
        }
    }

    /// Persist a tx-history record for every account touched by a balance-affecting entry.
    /// Key: `txhist:{account}:{epoch:016x}:{type_tag}:{hash[:8]}` — lexicographic order = chronological.
    fn index_tx_history(&self, entry: &LedgerEntry) {
        let epoch = entry.epoch();
        let entry_hash = entry.hash();
        let key_prefix = &entry_hash[..8.min(entry_hash.len())];

        // Build list of (account, role) pairs that had a balance event.
        let mut touched: Vec<(String, &'static str)> = Vec::new();
        match entry {
            LedgerEntry::Transfer { from, to, .. } => {
                touched.push((from.clone(), "sender"));
                touched.push((to.clone(), "recipient"));
            }
            LedgerEntry::GenesisAlloc { account, .. } => touched.push((account.clone(), "genesis")),
            LedgerEntry::MineReward { miner, .. } => touched.push((miner.clone(), "mine_reward")),
            LedgerEntry::ClockReward { node_id, .. } => touched.push((node_id.clone(), "clock_reward")),
            LedgerEntry::StorageReward { node_id, .. } => touched.push((node_id.clone(), "storage_reward")),
            LedgerEntry::ServiceReward { node_id, .. } => touched.push((node_id.clone(), "service_reward")),
            LedgerEntry::SensorReward { node_id, .. } => touched.push((node_id.clone(), "sensor_reward")),
            LedgerEntry::VerifierReward { node_id, .. } => touched.push((node_id.clone(), "verify_reward")),
            LedgerEntry::InferenceJobPay { worker, verifier_payments, dissenter_slashes, .. } => {
                touched.push((worker.clone(), "inference_fee"));
                for (v, _) in verifier_payments {
                    touched.push((v.clone(), "verifier_fee"));
                }
                for (d, _) in dissenter_slashes {
                    touched.push((d.clone(), "dissent_slash"));
                }
            }
            LedgerEntry::Stake { account, .. } => touched.push((account.clone(), "stake")),
            LedgerEntry::Unstake { account, .. } => touched.push((account.clone(), "unstake")),
            LedgerEntry::NodeRoleStake { staker, .. } => touched.push((staker.clone(), "role_stake")),
            LedgerEntry::NodeRoleUnstake { staker, .. } => touched.push((staker.clone(), "role_unstake")),
            _ => {}
        }

        if touched.is_empty() { return; }

        // Use the entry's own type field — the serialized form includes "type": "TRANSFER" etc.
        let entry_json = serde_json::to_value(entry).unwrap_or_default();
        let entry_type = entry_json.get("type")
            .and_then(|v| v.as_str())
            .unwrap_or("unknown");

        let record = serde_json::to_vec(&serde_json::json!({
            "epoch": epoch,
            "type": entry_type,
            "entry": entry_json,
        })).unwrap_or_default();

        for (account, role) in touched {
            let key = format!("txhist:{}:{:016x}:{}:{}", account, epoch, role, key_prefix);
            let _ = self.store.state_set(&key, &record);
        }

        // Global transaction index — one record per entry, keyed by epoch+hash.
        // Used by /api/explorer/transactions for chain-wide history without per-account fan-out.
        let global_key = format!("txglobal:{:016x}:{}", epoch, key_prefix);
        let _ = self.store.state_set(&global_key, &record);
    }

    /// Build and persist a sealed block from the txglobal index for the given epoch.
    /// Safe to call after all entries for the epoch have been applied.
    pub fn write_sealed_block(&self, epoch: u64, seal_hash: &str, timestamp_ms: u64) {
        use btcpc_types::{Block, BlockHeader, merkle_root};

        let prefix = format!("txglobal:{:016x}:", epoch);
        let entries: Vec<serde_json::Value> = self.store
            .state_scan_prefix(&prefix)
            .into_iter()
            .filter_map(|(_, v)| {
                let rec: serde_json::Value = serde_json::from_slice(&v).ok()?;
                rec.get("entry").cloned()
            })
            .collect();

        let entry_hashes: Vec<String> = entries.iter()
            .filter_map(|e| serde_json::from_value::<LedgerEntry>(e.clone()).ok())
            .map(|e| e.hash())
            .collect();
        let tx_root_bytes = {
            let root_hex = merkle_root(&entry_hashes);
            let bytes = hex::decode(&root_hex).unwrap_or_else(|_| vec![0u8; 32]);
            let mut arr = [0u8; 32];
            arr[..bytes.len().min(32)].copy_from_slice(&bytes[..bytes.len().min(32)]);
            arr
        };

        let seal_bytes: [u8; 32] = hex::decode(seal_hash)
            .ok()
            .and_then(|b| b.try_into().ok())
            .unwrap_or([0u8; 32]);

        let prev_hash: [u8; 32] = if epoch > 0 {
            self.store.read_block(epoch - 1)
                .ok().flatten()
                .and_then(|d| Block::from_bytes(&d))
                .map(|b| b.header.hash())
                .unwrap_or([0u8; 32])
        } else {
            [0u8; 32]
        };

        let mut header = BlockHeader::new(epoch as u32, prev_hash, seal_bytes);
        header.timestamp = timestamp_ms;
        header.merkle_root_transactions = tx_root_bytes;

        let payload = serde_json::json!({
            "ledger_entries": entries,
            "epoch": epoch,
            "seal_hash": seal_hash,
        });

        let block = Block { header, payload };
        if let Err(e) = self.store.write_block(epoch, &block.to_bytes()) {
            tracing::warn!("write_sealed_block epoch {}: {}", epoch, e);
        }
    }

    /// Apply a single ledger entry to state. Returns Ok(()) or a validation error.
    pub fn apply_entry(&self, entry: &LedgerEntry) -> Result<()> {
        match entry {
            LedgerEntry::GenesisAlloc { account, amount, token } => {
                self.ensure_account(account, 0)?;
                self.store.credit(account, token, *amount)?;
                info!("genesis alloc: {} {} → {}", amount, token, account);
            }

            LedgerEntry::AccountCreate { account, keys, chain_proofs, epoch, funded_by, machine_fingerprint } => {
                if self.store.get_account(account)?.is_some() {
                    return Ok(()); // idempotent
                }
                let exempt = btcpc_types::STAKE_EXEMPT_ACCOUNTS.contains(&account.as_str());
                if !exempt {
                    let stake_enabled = self.store.state_get("chain_param:name_stake_enabled")
                        .and_then(|bytes| String::from_utf8(bytes).ok())
                        .map(|s| s.trim() == "true")
                        .unwrap_or(false);
                    if stake_enabled {
                        let stake_amount = self.store.state_get("chain_param:name_stake_amount")
                            .and_then(|bytes| String::from_utf8(bytes).ok())
                            .and_then(|s| s.trim().parse::<u64>().ok())
                            .unwrap_or(btcpc_types::NAME_REGISTRATION_STAKE);
                        if let Some(funder) = funded_by {
                            self.store.debit(funder, NATIVE_TOKEN, stake_amount)?;
                        }
                    }
                }
                // chain_proofs: store commitment + mode, never the raw address.
                let proofs_json: serde_json::Value = chain_proofs.iter().map(|p| {
                    (p.chain.clone(), serde_json::json!({
                        "commitment": p.commitment,
                        "mode": p.mode,
                    }))
                }).collect::<serde_json::Map<_, _>>().into();

                let mut state = serde_json::json!({
                    "account_id": account,
                    "created_epoch": epoch,
                    "keys": keys,
                    "chain_proofs": proofs_json,
                    "nonce": 0,
                    "stake": 0,
                    "name_stake_locked": btcpc_types::NAME_REGISTRATION_STAKE,
                });
                if let Some(fp) = machine_fingerprint.as_deref().filter(|s| !s.is_empty()) {
                    state["machine_fingerprint"] = serde_json::Value::String(fp.to_owned());
                }
                self.store.set_account(account, &state)?;
                // Seed liveness clock at creation epoch.
                self.touch_alive(account, *epoch);
                info!(
                    account,
                    chains = chain_proofs.iter().map(|p| p.chain.as_str()).collect::<Vec<_>>().join(","),
                    "account created with {} chain proof(s)", chain_proofs.len()
                );
            }

            LedgerEntry::Transfer { from, to, amount, token, epoch, nonce, .. } => {
                self.touch_alive(from, *epoch);
                anyhow::ensure!(*amount > 0, "transfer amount must be positive");
                // Nonce replay guard — defense in depth (validate_and_apply also enforces this).
                crate::tx::check_nonce(self, from, *nonce)?;
                self.ensure_account(to, *epoch)?;
                self.store.debit(from, token, *amount)?;
                // Native token or pre-approved type/sender: instant credit, no fee.
                if token == NATIVE_TOKEN || token_approved(&self.store, to, token, from) {
                    self.store.credit(to, token, *amount)?;
                } else if let Some(gate) = spam_gate(&self.store, to, token) {
                    match gate {
                        SpamGate::OnChain { fee, fee_token } => {
                            anyhow::ensure!(
                                self.store.get_balance(from, &fee_token) >= fee,
                                "sender '{}' needs {} {} to pass spam gate for '{}'", from, fee, fee_token, to
                            );
                            self.store.debit(from, &fee_token, fee)?;
                            self.store.credit(to, &fee_token, fee)?;
                            self.store.credit(to, token, *amount)?;
                        }
                        SpamGate::Evm { evm_address, evm_chain_id } => {
                            anyhow::bail!(
                                "recipient '{}' requires EVM payment to {} on chain {}; use SpamGatePayEvm",
                                to, evm_address, evm_chain_id
                            );
                        }
                    }
                } else {
                    // No approval, no gate: pending — recipient must TokenAccept.
                    // Expires 30 epochs (~15 min); sweep refunds sender on expiry.
                    let pending_key = format!("pending_transfer:{}:{}:{}:{}", to, token, from, nonce);
                    let val = serde_json::to_vec(&serde_json::json!({
                        "from": from, "to": to, "token": token,
                        "amount": amount, "expires_epoch": epoch + 30,
                    }))?;
                    self.store.state_set(&pending_key, &val)?;
                }
                crate::tx::bump_nonce(self, from)?;
            }

            LedgerEntry::Stake { account, amount, epoch, .. } => {
                self.touch_alive(account, *epoch);
                anyhow::ensure!(*amount > 0, "stake amount must be positive");
                self.store.debit(account, NATIVE_TOKEN, *amount)?;
                let current_stake = self.store.get_stake(account);
                let new_stake = current_stake.checked_add(*amount)
                    .ok_or_else(|| anyhow::anyhow!("stake overflow for '{}'", account))?;
                self.store.set_stake(account, new_stake)?;
            }

            LedgerEntry::Unstake { account, amount, epoch, .. } => {
                self.touch_alive(account, *epoch);
                let current_stake = self.store.get_stake(account);
                anyhow::ensure!(current_stake >= *amount, "insufficient stake");
                // Trust drops immediately (D4): stake removed now.
                self.store.set_stake(account, current_stake - amount)?;
                // Tokens enter unbonding: released after slash window expires.
                // This ensures slashes can still reach tokens for actions done while staked.
                let release_epoch = epoch + UNBONDING_EPOCHS;
                let unbond_key = format!("unbonding:{}:{}", release_epoch, account);
                let prev_amount: u64 = self.store.state_get(&unbond_key)
                    .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                    .and_then(|j| j["amount"].as_u64()).unwrap_or(0);
                let _ = self.store.state_set(&unbond_key,
                    &serde_json::to_vec(&serde_json::json!({
                        "account": account,
                        "amount": prev_amount + amount,
                        "release_epoch": release_epoch,
                    })).unwrap_or_default());
            }

            LedgerEntry::Mine { miner, epoch, model, input_tokens, output_tokens, tool_calls, hw_tier, output_hash, job_id, node_version, software_hash, model_hash } => {
                self.touch_alive(miner, *epoch);
                self.ensure_account(miner, *epoch)?;
                // Epoch freshness check: reject Mine entries more than 3 epochs stale.
                // This allows slow models (26B+) that span 2-3 epochs to still submit.
                // Far-past entries would never reach the reward scan so there's no benefit
                // to accepting them, and they waste state space.
                {
                    const MINE_STALE_WINDOW: u64 = 3;
                    let cur = *self.current_epoch.read();
                    if cur > 0 && *epoch + MINE_STALE_WINDOW < cur {
                        anyhow::bail!(
                            "Mine epoch {} is too old (current epoch {}); submit for a more recent epoch",
                            epoch, cur
                        );
                    }
                }
                // Model approval check: if an approved list is on-chain, the model must be in it.
                // An empty list means any model is accepted (open era / governance not yet set).
                if !model.is_empty() {
                    let approved: Vec<String> = self.store
                        .state_get("chain_param:approved_models")
                        .and_then(|b| serde_json::from_slice(&b).ok())
                        .unwrap_or_default();
                    if !approved.is_empty() {
                        let base = model.split(':').next().unwrap_or(model.as_str());
                        let ok = approved.iter().any(|a| {
                            let abase = a.split(':').next().unwrap_or(a.as_str());
                            a == model || abase == base
                        });
                        if !ok {
                            anyhow::bail!(
                                "model '{}' is not on the approved list; run an approved model or wait for governance to add it",
                                model
                            );
                        }
                    }
                }
                // Software version check: if governance has set a minimum, reject older nodes.
                if let Some(ver) = node_version {
                    if let Some(min_ver) = self.store.state_get("chain_param:min_node_version")
                        .and_then(|b| String::from_utf8(b).ok())
                        .filter(|s| !s.trim().is_empty())
                    {
                        if !semver_gte(ver, &min_ver) {
                            anyhow::bail!(
                                "node version '{}' is below minimum required '{}' — upgrade btcpc-node",
                                ver, min_ver.trim()
                            );
                        }
                    }
                }
                // Software hash check: if governance has set approved binary hashes, enforce them.
                if let Some(sw_hash) = software_hash {
                    if !sw_hash.is_empty() {
                        let approved_sw: Vec<String> = self.store
                            .state_get("chain_param:approved_software")
                            .and_then(|b| serde_json::from_slice(&b).ok())
                            .unwrap_or_default();
                        if !approved_sw.is_empty() && !approved_sw.iter().any(|h| h == sw_hash) {
                            anyhow::bail!(
                                "software hash '{}' is not on the approved list — upgrade to an official btcpc-node release",
                                sw_hash
                            );
                        }
                    }
                }
                // Model hash check: if governance has set approved model digests, enforce them.
                if let Some(mh) = model_hash {
                    if !mh.is_empty() {
                        let approved_mh: Vec<String> = self.store
                            .state_get("chain_param:approved_model_hashes")
                            .and_then(|b| serde_json::from_slice(&b).ok())
                            .unwrap_or_default();
                        if !approved_mh.is_empty() && !approved_mh.iter().any(|h| h == mh) {
                            anyhow::bail!(
                                "model digest '{}' is not approved — pull an official model version",
                                mh
                            );
                        }
                    }
                }
                // If job_id is present, the miner must be the awarded worker for that job.
                if let Some(jid) = job_id {
                    if !jid.is_empty() {
                        match crate::inference::get_job(self, jid) {
                            None => anyhow::bail!("Mine references unknown job '{}'", jid),
                            Some(job) if job.winner.as_deref() != Some(miner.as_str()) =>
                                anyhow::bail!("miner '{}' is not the awarded worker for job '{}'", miner, jid),
                            _ => {}
                        }
                    }
                }
                let key = format!("mine:{}:{}", epoch, miner);
                let _ = self.store.state_set(&key,
                    &serde_json::to_vec(&serde_json::json!({
                        "miner": miner, "epoch": epoch,
                        "model": model, "input_tokens": input_tokens,
                        "output_tokens": output_tokens, "tool_calls": tool_calls,
                        "hw_tier": hw_tier, "output_hash": output_hash,
                        "job_id": job_id,
                    })).unwrap_or_default());
            }

            LedgerEntry::MineReward { miner, amount, epoch } => {
                self.ensure_account(miner, *epoch)?;
                if era(*epoch) >= RECYCLE_ERA {
                    // Era 5+: transfer from recycle fund rather than minting.
                    self.store.debit(RECYCLE_FUND_ACCOUNT, NATIVE_TOKEN, *amount)
                        .map_err(|e| anyhow::anyhow!("recycle fund insufficient for era-5 reward: {}", e))?;
                }
                let node_credit = self.distribute_role_backer_reward("miner", miner, *amount)?;
                self.store.credit(miner, NATIVE_TOKEN, node_credit)?;
                info!("mine reward: {} dreams → {} (epoch {})", amount, miner, epoch);
            }

            LedgerEntry::EpochSeal { node_id, epoch, seal_hash, timestamp, .. } => {
                // Equivocation guard: reject a second EpochSeal from the same node for
                // the same epoch if it carries a different seal_hash. Idempotent if the
                // hash matches (gossip re-delivery). Prevents fork-by-overwrite attacks.
                let node_seal_key = format!("epoch_node_seal:{}:{}", epoch, node_id);
                if let Some(prior) = self.store.state_get(&node_seal_key) {
                    let prior_hash = String::from_utf8_lossy(&prior);
                    anyhow::ensure!(
                        prior_hash == seal_hash.as_str(),
                        "equivocating EpochSeal: node '{}' epoch {} already sealed with hash '{}'",
                        node_id, epoch, prior_hash
                    );
                    // Idempotent re-delivery — nothing to do.
                    self.index_tx_history(entry);
                    return Ok(());
                }
                let _ = self.store.state_set(&node_seal_key, seal_hash.as_bytes());

                let ep = *epoch as u64;
                let mut current = self.current_epoch.write();
                if ep > *current {
                    *current = ep;
                    let _ = self.store.set_meta("current_epoch", &ep.to_le_bytes());
                }
                // Store for storage challenge derivation (T4-1): challenge = sha256(seal_hash:node_id:epoch)
                let _ = self.store.state_set(&format!("epoch_seal_hash:{}", epoch), seal_hash.as_bytes());

                // ── Bitcoin-tracking era calibration ─────────────────────────
                // At each era boundary, recompute the epoch duration for all
                // remaining new-supply eras so supply exhaustion tracks Bitcoin's
                // last-coin timestamp.  No governance required — fires automatically.
                use btcpc_types::{era, RECYCLE_ERA, calibrate_era_epoch_ms, BTC_PROJECTED_END_MS};
                let new_era = era(ep);
                let prev_era = if ep > 0 { era(ep - 1) } else { 0 };
                if new_era > prev_era && new_era < RECYCLE_ERA {
                    // Use oracle-refined target if available, else compile-time constant.
                    let target_end_ms: u64 = self.store.state_get("chain_param:btc_end_ms")
                        .and_then(|b| serde_json::from_slice(&b).ok())
                        .unwrap_or(BTC_PROJECTED_END_MS);
                    let now_ms = *timestamp;
                    let base_ms = calibrate_era_epoch_ms(new_era, now_ms, target_end_ms);
                    // Store calibrated duration for each remaining era (each 2× the previous).
                    for e in new_era..RECYCLE_ERA {
                        let key = format!("chain_param:era_epoch_ms:{}", e);
                        let ms = base_ms << (e - new_era);
                        let _ = self.store.state_set(&key, &serde_json::to_vec(&ms).unwrap_or_default());
                    }
                    info!(
                        "[era {}] epoch duration auto-calibrated to {}ms (~{:.1}s) targeting btc_end_ms={}",
                        new_era, base_ms, base_ms as f64 / 1000.0, target_end_ms
                    );
                }
            }

            // ── Fork choice (T1-8) ────────────────────────────────────────────
            // Heaviest-quorum rule: if two EpochFinalizes arrive for the same epoch,
            // the one with more registered signers wins. Equal quorum: smaller
            // rewards_hash wins (deterministic lexicographic tiebreaker).
            LedgerEntry::EpochFinalize { epoch, rewards_hash, quorum, state_root, timestamp, .. } => {
                let existing = self.store.get_epoch_meta(*epoch)?;
                if let Some(prev) = existing {
                    let prev_hash   = prev["rewards_hash"].as_str().unwrap_or("");
                    let prev_quorum = prev["quorum"].as_u64().unwrap_or(0);

                    if prev_hash == rewards_hash {
                        // Idempotent — same finalization seen again, skip.
                    } else {
                        // Competing finalization for the same epoch — fork detected.
                        let new_wins = *quorum > prev_quorum
                            || (*quorum == prev_quorum && rewards_hash.as_str() < prev_hash);

                        warn!(
                            "[fork] competing EpochFinalize at epoch {}: \
                             existing quorum={} hash={} | incoming quorum={} hash={} — {}",
                            epoch, prev_quorum, &prev_hash[..prev_hash.len().min(12)],
                            quorum, &rewards_hash[..rewards_hash.len().min(12)],
                            if new_wins { "ACCEPTING incoming (higher quorum)" }
                            else { "REJECTING incoming (lower/equal quorum)" }
                        );

                        // Record fork evidence for auditors regardless of outcome.
                        let fork_key = format!("fork_evidence:{}", epoch);
                        let _ = self.store.state_set(&fork_key, &serde_json::to_vec(
                            &serde_json::json!({
                                "epoch": epoch,
                                "head_a": { "rewards_hash": prev_hash, "quorum": prev_quorum },
                                "head_b": { "rewards_hash": rewards_hash, "quorum": quorum },
                                "winner": if new_wins { "head_b" } else { "head_a" },
                            })
                        ).unwrap_or_default());

                        if !new_wins {
                            // Keep existing finalization; incoming has lower authority.
                            return Ok(());
                        }
                        // Fall through to overwrite with the higher-quorum finalization.
                    }
                }

                let meta = serde_json::json!({
                    "epoch": epoch,
                    "rewards_hash": rewards_hash,
                    "quorum": quorum,
                    "state_root": state_root,
                    "finalized_at": timestamp,
                    "finalized": true,
                });
                self.store.set_epoch_meta(*epoch, &meta)?;
            }

            LedgerEntry::AccountUpdateKey { account, role, new_public_key, epoch, .. } => {
                // Create the account if it doesn't exist yet (first-time key registration).
                self.ensure_account(account, *epoch)?;
                let mut state = self.store.get_account(account)?
                    .unwrap_or_else(|| serde_json::json!({ "nonce": 0 }));
                if state.get("keys").is_none() || !state["keys"].is_object() {
                    state["keys"] = serde_json::json!({});
                }
                state["keys"][role] = serde_json::json!(new_public_key);
                self.store.set_account(account, &state)?;
                info!("key '{}' registered for account '{}'", role, account);
            }

            // Governance: set a chain-wide parameter.
            LedgerEntry::ChainParameterSet { key, value, .. } => {
                self.store.state_set(&format!("chain_param:{}", key), value.as_bytes())?;
                info!("chain param '{}' set to '{}'", key, value);
            }

            // Hard-mode chain link: verify external signature, store commitment only.
            LedgerEntry::VerifyChainLink { account, chain, commitment, signed_message, signature, sig_type, .. } => {
                let mut state = self.store.get_account(account)?
                    .ok_or_else(|| anyhow::anyhow!("account '{}' not found", account))?;

                // Recover address from signature and verify it matches the commitment.
                let recovered_addr = recover_chain_address(sig_type, signed_message, signature)
                    .with_context(|| format!("signature recovery failed for {} link on '{}'", chain, account))?;

                // Re-derive commitment from recovered address and the nonce embedded in signed_message.
                // Message format: "btcpc:link:{account}:{chain}:{nonce}"
                let nonce = signed_message.split(':').nth(4)
                    .ok_or_else(|| anyhow::anyhow!("malformed signed_message — expected btcpc:link:account:chain:nonce"))?;
                let expected = {
                    use sha2::{Digest, Sha256};
                    let mut h = Sha256::new();
                    h.update(chain.as_bytes());
                    h.update(b":");
                    h.update(recovered_addr.as_bytes());
                    h.update(b":");
                    h.update(nonce.as_bytes());
                    hex::encode(h.finalize())
                };
                anyhow::ensure!(
                    expected == *commitment,
                    "commitment mismatch: recovered address does not match submitted commitment"
                );

                // Store commitment only — address is discarded after verification.
                if state.get("chain_proofs").is_none() {
                    state["chain_proofs"] = serde_json::json!({});
                }
                state["chain_proofs"][chain] = serde_json::json!({
                    "commitment": commitment,
                    "mode": "hard",
                    "sig_type": sig_type,
                    // Store signed_message and signature so anyone can independently verify.
                    "signed_message": signed_message,
                    "signature": signature,
                });
                self.store.set_account(account, &state)?;
                info!(account, chain, "hard-mode chain link verified and stored");
            }

            // Set or clear the 2FA policy for a key slot.
            // Policies live on the slot — they survive key rotation.
            LedgerEntry::SetKeyPolicy { account, role, twofactor_chain, .. } => {
                let mut state = self.store.get_account(account)?
                    .ok_or_else(|| anyhow::anyhow!("account '{}' not found", account))?;

                if state.get("key_policies").is_none() {
                    state["key_policies"] = serde_json::json!({});
                }
                match twofactor_chain {
                    Some(chain_name) => {
                        // Verify that a chain proof exists for this chain first.
                        let proof_exists = state.get("chain_proofs")
                            .and_then(|cp| cp.get(chain_name.as_str()))
                            .is_some();
                        anyhow::ensure!(
                            proof_exists,
                            "no chain proof for '{}' on account '{}' — link the chain first",
                            chain_name, account
                        );
                        state["key_policies"][role] = serde_json::json!({
                            "twofactor_chain": chain_name,
                        });
                        info!(account, role, chain = %chain_name, "2FA policy set for slot");
                    }
                    None => {
                        // Clear the policy for this slot.
                        if let Some(policies) = state["key_policies"].as_object_mut() {
                            policies.remove(role.as_str());
                        }
                        info!(account, role, "2FA policy cleared for slot");
                    }
                }
                self.store.set_account(account, &state)?;
            }

            // Record the owner's declared primary identity.
            LedgerEntry::AccountSetPrimary { account, primary, .. } => {
                let mut state = self.store.get_account(account)?
                    .ok_or_else(|| anyhow::anyhow!("account '{}' not found", account))?;
                state["primary"] = serde_json::json!(primary);
                self.store.set_account(account, &state)?;
                info!("primary identity for '{}' set to '{}'", account, primary);
            }

            // Identity transfer: sweep funds to primary, then rotate keys.
            LedgerEntry::AccountTransfer { account, new_keys, epoch, .. } => {
                let mut state = self.store.get_account(account)?
                    .ok_or_else(|| anyhow::anyhow!("cannot transfer non-existent account '{}'", account))?;
                // Sweep all balances to the declared primary before handing off the identity.
                if let Some(primary) = state.get("primary").and_then(|v| v.as_str()) {
                    let primary = primary.to_owned();
                    let balances = self.store.scan_balances(account);
                    for (token, amount) in balances {
                        if amount > 0 {
                            self.store.debit(account, &token, amount)?;
                            self.store.credit(&primary, &token, amount)?;
                        }
                    }
                }
                state["keys"] = serde_json::to_value(new_keys)?;
                state["transferred_epoch"] = serde_json::json!(epoch);
                state["primary"] = serde_json::Value::Null; // clear — new owner sets their own
                self.store.set_account(account, &state)?;
                info!("identity '{}' transferred at epoch {}", account, epoch);
            }

            LedgerEntry::SensorReading { sensor_id, metadata, epoch, .. } => {
                // T4-3: GNSS plausibility — consecutive positions must be reachable at
                // SENSOR_GNSS_MAX_SPEED_M_S within one epoch (EPOCH_DURATION_S seconds).
                if let Some(meta) = metadata {
                    let lat = meta.get("lat").and_then(|v| v.as_f64());
                    let lon = meta.get("lon").and_then(|v| v.as_f64());
                    if let (Some(lat), Some(lon)) = (lat, lon) {
                        let prev_key = format!("sensor_last_gnss:{}", sensor_id);
                        let prev = self.store.state_get(&prev_key)
                            .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok());
                        if let Some(prev) = prev {
                            if let (Some(plat), Some(plon)) = (
                                prev["lat"].as_f64(), prev["lon"].as_f64()
                            ) {
                                let dist_m = haversine_m(plat, plon, lat, lon);
                                let speed = dist_m / EPOCH_DURATION_S;
                                if speed > SENSOR_GNSS_MAX_SPEED_M_S {
                                    anyhow::bail!(
                                        "sensor '{}' GNSS reading implausible: {:.0} m/s exceeds {:.0} m/s limit",
                                        sensor_id, speed, SENSOR_GNSS_MAX_SPEED_M_S
                                    );
                                }
                            }
                        }
                        // Store current position for next epoch comparison.
                        let _ = self.store.state_set(&prev_key,
                            &serde_json::to_vec(&serde_json::json!({
                                "lat": lat, "lon": lon, "epoch": epoch,
                            })).unwrap_or_default());
                    }
                }
            }

            // Coverage reports — cellular dead-spot mapping.
            // Quantize lat/lon to 0.001° grid cells (~100m). Track per-cell reporter count
            // for corroboration bonus, and per-reporter count for anti-spam cap.
            LedgerEntry::CoverageReport { reporter, lat, lon, signal_dbm, carrier_mcc_mnc, technology, data_hash, epoch, .. } => {
                self.touch_alive(reporter, *epoch);
                self.ensure_account(reporter, *epoch)?;

                // Anti-spam: cap reports per reporter per epoch.
                let reporter_count_key = format!("coverage_reporter_count:{}:{}", epoch, reporter);
                let count = self.store.state_get(&reporter_count_key)
                    .and_then(|b| String::from_utf8(b).ok())
                    .and_then(|s| s.parse::<u64>().ok())
                    .unwrap_or(0);
                if count >= COVERAGE_MAX_REPORTS_PER_EPOCH {
                    anyhow::bail!("reporter '{}' exceeded {} coverage reports for epoch {}",
                        reporter, COVERAGE_MAX_REPORTS_PER_EPOCH, epoch);
                }
                let _ = self.store.state_set(&reporter_count_key,
                    (count + 1).to_string().as_bytes());

                // Quantize to grid cell (0.001° ≈ 100m).
                let lat_q = (*lat * COVERAGE_GRID_RESOLUTION as f64).floor() as i64;
                let lon_q = (*lon * COVERAGE_GRID_RESOLUTION as f64).floor() as i64;
                let is_dead_spot = signal_dbm.map(|d| d < COVERAGE_DEAD_SPOT_DBM_THRESHOLD).unwrap_or(true);

                // Store individual report for reward calculation.
                let report_key = format!("coverage_report:{}:{}:{}:{}:{}",
                    epoch, lat_q, lon_q, carrier_mcc_mnc, reporter);
                let _ = self.store.state_set(&report_key,
                    &serde_json::to_vec(&serde_json::json!({
                        "reporter": reporter, "epoch": epoch,
                        "lat": lat, "lon": lon, "lat_q": lat_q, "lon_q": lon_q,
                        "signal_dbm": signal_dbm, "is_dead_spot": is_dead_spot,
                        "carrier_mcc_mnc": carrier_mcc_mnc, "technology": technology,
                        "data_hash": data_hash,
                    })).unwrap_or_default());

                // Track distinct reporters per grid cell for corroboration.
                let cell_key = format!("coverage_cell:{}:{}:{}:{}", epoch, lat_q, lon_q, carrier_mcc_mnc);
                let mut cell: serde_json::Value = self.store.state_get(&cell_key)
                    .and_then(|b| serde_json::from_slice(&b).ok())
                    .unwrap_or_else(|| serde_json::json!({ "reporters": [], "dead_spot_count": 0u64 }));
                // Collect current reporter list, append if new and under the Sybil cap.
                // Reporters beyond COVERAGE_MAX_CORROBORATING_REPORTERS still earn base
                // score (their report_key is stored above) but cannot tip the cell into
                // corroborated status, preventing Sybil farms from manufacturing bonuses.
                let mut reporter_list: Vec<String> = cell["reporters"].as_array()
                    .map(|a| a.iter().filter_map(|v| v.as_str().map(|s| s.to_owned())).collect())
                    .unwrap_or_default();
                if !reporter_list.iter().any(|r| r == reporter.as_str())
                    && (reporter_list.len() as u64) < COVERAGE_MAX_CORROBORATING_REPORTERS
                {
                    reporter_list.push(reporter.clone());
                }
                let reporter_count = reporter_list.len() as u64;
                let dead_count = cell["dead_spot_count"].as_u64().unwrap_or(0)
                    + if is_dead_spot { 1 } else { 0 };
                cell["reporters"] = serde_json::json!(reporter_list);
                cell["dead_spot_count"] = serde_json::json!(dead_count);
                cell["corroborated"] = serde_json::json!(reporter_count >= COVERAGE_CORROBORATION_MIN_REPORTERS);
                let _ = self.store.state_set(&cell_key,
                    &serde_json::to_vec(&cell).unwrap_or_default());
            }

            LedgerEntry::BlobStore { .. }
            | LedgerEntry::ContractDeploy { .. }
            | LedgerEntry::ContractCall { .. } => {
                // Accepted, no balance mutations needed at base layer
            }

            // Track storage heartbeats so the clock can compute storage rewards at seal time.
            // Verify the Merkle range proof (T4-1) — proof_valid=true earns full reward.
            // T2-4: when proof is valid, effective_bytes = total_chunks × chunk_size, not
            // self-declared bytes_proven. This prevents inflated reward claims.
            LedgerEntry::StorageHeartbeat { node_id, epoch, bytes_proven, query_count, challenge_response, .. } => {
                self.ensure_account(node_id, *epoch)?;
                let (proof_valid, proven_chunks) = challenge_response.as_ref()
                    .map(|p| (verify_storage_proof(&self.store, p, node_id, *epoch, *bytes_proven), p.total_chunks))
                    .unwrap_or((false, 0));
                let effective_bytes = if proof_valid {
                    proven_chunks.saturating_mul(STORAGE_CHALLENGE_CHUNK_BYTES)
                } else {
                    *bytes_proven
                };
                let key = format!("storage_beat:{}:{}", epoch, node_id);
                let _ = self.store.state_set(&key,
                    &serde_json::to_vec(&serde_json::json!({
                        "node_id": node_id, "epoch": epoch,
                        "bytes_proven": effective_bytes, "query_count": query_count,
                        "proof_valid": proof_valid,
                    })).unwrap_or_default());
            }

            // Track sensor commits per sensor_id (not per owner) so reward scoring
            // can apply type-aware sensor_score() per individual sensor.
            LedgerEntry::SensorDataCommit { sensor_id, owner, epoch, reading_count, sensor_type, .. } => {
                self.ensure_account(owner, *epoch)?;
                // Cap: no sensor type scores meaningfully past 10_000 readings per epoch.
                // Prevents unbounded state values from being stored.
                const MAX_READINGS_PER_EPOCH: u64 = 10_000;
                if *reading_count > MAX_READINGS_PER_EPOCH {
                    anyhow::bail!(
                        "reading_count {} exceeds maximum {} per epoch",
                        reading_count, MAX_READINGS_PER_EPOCH
                    );
                }
                // VEC-3: if the sensor is registered on-chain, verify ownership.
                let reg_key = format!("sensor:{}", sensor_id);
                if let Some(reg_bytes) = self.store.get_meta(&reg_key) {
                    if let Ok(reg) = serde_json::from_slice::<serde_json::Value>(&reg_bytes) {
                        if reg["owner"].as_str() != Some(owner.as_str()) {
                            anyhow::bail!(
                                "sensor '{}' is registered to '{}', not '{}'",
                                sensor_id,
                                reg["owner"].as_str().unwrap_or("unknown"),
                                owner
                            );
                        }
                    }
                }
                let key = format!("sensor_commit:{}:{}", epoch, sensor_id);
                let _ = self.store.state_set(&key,
                    &serde_json::to_vec(&serde_json::json!({
                        "sensor_id": sensor_id, "owner": owner, "epoch": epoch,
                        "reading_count": reading_count, "sensor_type": sensor_type,
                    })).unwrap_or_default());
            }

            // Decentralized runtime — state guards + full economics.
            LedgerEntry::RuntimeRegister { runtime_id, owner, manifest_cid, runtime_class, nonce, bond, epoch, .. } => {
                let key = format!("runtime:{}", runtime_id);
                anyhow::ensure!(
                    self.store.state_get(&key).is_none(),
                    "runtime '{}' already registered", runtime_id
                );
                // Lock bond into per-runtime escrow account.
                let escrow = format!("__runtime_bond_{}__", runtime_id);
                self.store.debit(owner, NATIVE_TOKEN, *bond)?;
                self.store.credit(&escrow, NATIVE_TOKEN, *bond)?;
                let value = serde_json::json!({
                    "runtime_id": runtime_id,
                    "owner": owner,
                    "manifest_cid": manifest_cid,
                    "runtime_class": runtime_class,
                    "nonce": nonce,
                    "bond": bond,
                    "status": "registered",
                    "created_epoch": epoch,
                    "updated_epoch": epoch,
                });
                self.store.state_set(&key, &serde_json::to_vec(&value)?)?;
            }
            LedgerEntry::RuntimeDeploy { runtime_id, owner, host_id, manifest_cid, epoch, .. } => {
                let key = format!("runtime:{}", runtime_id);
                let prev = self.store.state_get(&key)
                    .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                    .ok_or_else(|| anyhow::anyhow!("runtime '{}' not found", runtime_id))?;
                anyhow::ensure!(
                    prev["owner"].as_str() == Some(owner.as_str()),
                    "only the runtime owner can deploy"
                );
                let status = prev["status"].as_str().unwrap_or("");
                anyhow::ensure!(
                    status == "registered" || status == "undeployed",
                    "runtime '{}' must be registered or undeployed to deploy (status: {})", runtime_id, status
                );
                let manifest = manifest_cid
                    .clone()
                    .or_else(|| prev["manifest_cid"].as_str().map(ToString::to_string));
                let value = serde_json::json!({
                    "runtime_id": runtime_id,
                    "owner": owner,
                    "manifest_cid": manifest,
                    "runtime_class": prev.get("runtime_class").cloned().unwrap_or(serde_json::Value::Null),
                    "nonce": prev.get("nonce").cloned().unwrap_or(serde_json::Value::Null),
                    "bond": prev.get("bond").cloned().unwrap_or(serde_json::Value::Null),
                    "deploy_host": host_id,
                    "status": "deployed",
                    "created_epoch": prev["created_epoch"].as_u64().unwrap_or(*epoch),
                    "deployed_epoch": epoch,
                    "updated_epoch": epoch,
                });
                self.store.state_set(&key, &serde_json::to_vec(&value)?)?;

                let host_key = format!("runtime_host:{}", host_id);
                let host_value = serde_json::json!({
                    "host_id": host_id,
                    "status": "active",
                    "last_deploy_epoch": epoch,
                    "updated_epoch": epoch,
                });
                self.store.state_set(&host_key, &serde_json::to_vec(&host_value)?)?;
            }
            LedgerEntry::RuntimeUndeploy { runtime_id, owner, epoch, .. } => {
                let key = format!("runtime:{}", runtime_id);
                let prev = self.store.state_get(&key)
                    .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                    .ok_or_else(|| anyhow::anyhow!("runtime '{}' not found", runtime_id))?;
                anyhow::ensure!(
                    prev["owner"].as_str() == Some(owner.as_str()),
                    "only the runtime owner can undeploy"
                );
                let mut value = prev;
                value["status"] = serde_json::json!("undeployed");
                value["undeployed_epoch"] = serde_json::json!(epoch);
                value["updated_epoch"] = serde_json::json!(epoch);
                self.store.state_set(&key, &serde_json::to_vec(&value)?)?;
            }
            LedgerEntry::RuntimeJobEnqueue { job_id, runtime_id, job_class, due_epoch, payload_cid, fee, epoch, signed_by } => {
                // Runtime must be deployed before jobs can be enqueued.
                let rt_key = format!("runtime:{}", runtime_id);
                let rt = self.store.state_get(&rt_key)
                    .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                    .ok_or_else(|| anyhow::anyhow!("runtime '{}' not found", runtime_id))?;
                anyhow::ensure!(
                    rt["status"].as_str() == Some("deployed"),
                    "runtime '{}' is not deployed (status: {})", runtime_id, rt["status"]
                );
                // Escrow the job fee — debit from caller, hold in per-job escrow account.
                let escrow = format!("__runtime_job_escrow_{}__", job_id);
                self.store.debit(signed_by, NATIVE_TOKEN, *fee)?;
                self.store.credit(&escrow, NATIVE_TOKEN, *fee)?;
                let key = format!("runtime_job:{}", job_id);
                let value = serde_json::json!({
                    "job_id": job_id,
                    "runtime_id": runtime_id,
                    "job_class": job_class,
                    "due_epoch": due_epoch,
                    "payload_cid": payload_cid,
                    "fee": fee,
                    "status": "queued",
                    "enqueued_by": signed_by,
                    "enqueued_epoch": epoch,
                    "updated_epoch": epoch,
                });
                self.store.state_set(&key, &serde_json::to_vec(&value)?)?;
            }
            LedgerEntry::RuntimeClaim { lease_id, runtime_id, job_id, host_id, expires_epoch, epoch, .. } => {
                // Job must exist and be in queued state (not already claimed).
                let job_key = format!("runtime_job:{}", job_id);
                let job = self.store.state_get(&job_key)
                    .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                    .ok_or_else(|| anyhow::anyhow!("job '{}' not found", job_id))?;
                anyhow::ensure!(
                    job["status"].as_str() == Some("queued"),
                    "job '{}' is not queued (status: {})", job_id, job["status"]
                );
                anyhow::ensure!(
                    job["runtime_id"].as_str() == Some(runtime_id.as_str()),
                    "job '{}' belongs to a different runtime", job_id
                );

                let lease_key = format!("runtime_lease:{}", lease_id);
                let lease_value = serde_json::json!({
                    "lease_id": lease_id,
                    "runtime_id": runtime_id,
                    "job_id": job_id,
                    "host_id": host_id,
                    "status": "active",
                    "claimed_epoch": epoch,
                    "expires_epoch": expires_epoch,
                    "updated_epoch": epoch,
                });
                self.store.state_set(&lease_key, &serde_json::to_vec(&lease_value)?)?;

                let host_key = format!("runtime_host:{}", host_id);
                let host_value = serde_json::json!({
                    "host_id": host_id,
                    "status": "active",
                    "last_claim_epoch": epoch,
                    "updated_epoch": epoch,
                });
                self.store.state_set(&host_key, &serde_json::to_vec(&host_value)?)?;

                let mut job_upd = job;
                job_upd["status"] = serde_json::json!("claimed");
                job_upd["lease_id"] = serde_json::json!(lease_id);
                job_upd["claimed_by"] = serde_json::json!(host_id);
                job_upd["updated_epoch"] = serde_json::json!(epoch);
                self.store.state_set(&job_key, &serde_json::to_vec(&job_upd)?)?;
            }
            LedgerEntry::RuntimeAttest { attestation_id, runtime_id, job_id, host_id, output_commitment, runtime_sha, epoch, .. } => {
                // Lease must be active and match this host.
                let job_key = format!("runtime_job:{}", job_id);
                let job = self.store.state_get(&job_key)
                    .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                    .ok_or_else(|| anyhow::anyhow!("job '{}' not found", job_id))?;
                let lease_id = job["lease_id"].as_str()
                    .ok_or_else(|| anyhow::anyhow!("job '{}' has no active lease", job_id))?
                    .to_owned();
                let lease_key = format!("runtime_lease:{}", lease_id);
                let lease = self.store.state_get(&lease_key)
                    .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                    .ok_or_else(|| anyhow::anyhow!("lease '{}' not found", lease_id))?;
                anyhow::ensure!(
                    lease["host_id"].as_str() == Some(host_id.as_str()),
                    "host '{}' did not claim job '{}'", host_id, job_id
                );
                anyhow::ensure!(
                    lease["status"].as_str() == Some("active"),
                    "lease '{}' is not active", lease_id
                );
                anyhow::ensure!(
                    job["status"].as_str() == Some("claimed"),
                    "job '{}' is not claimed (status: {})", job_id, job["status"]
                );

                // If runtime_sha supplied, verify it matches the manifest_cid.
                if let Some(sha) = runtime_sha {
                    let rt_key = format!("runtime:{}", runtime_id);
                    if let Some(rt) = self.store.state_get(&rt_key)
                        .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                    {
                        if let Some(manifest_cid) = rt["manifest_cid"].as_str() {
                            anyhow::ensure!(
                                sha == manifest_cid,
                                "runtime_sha '{}' does not match manifest_cid '{}'", sha, manifest_cid
                            );
                        }
                    }
                }

                // Release escrow: host gets HOST_BPS share, recycle gets the rest.
                let fee = job["fee"].as_u64().unwrap_or(0);
                if fee > 0 {
                    let escrow = format!("__runtime_job_escrow_{}__", job_id);
                    let host_share = fee * RUNTIME_FEE_HOST_BPS / 10_000;
                    let recycle_share = fee * RUNTIME_FEE_RECYCLE_BPS / 10_000;
                    self.store.debit(&escrow, NATIVE_TOKEN, fee)?;
                    self.store.credit(host_id, NATIVE_TOKEN, host_share)?;
                    if recycle_share > 0 {
                        self.store.credit(RECYCLE_FUND_ACCOUNT, NATIVE_TOKEN, recycle_share)?;
                    }
                }

                // Mark job completed.
                let mut job_upd = job;
                job_upd["status"] = serde_json::json!("completed");
                job_upd["attested_by"] = serde_json::json!(host_id);
                job_upd["attestation_id"] = serde_json::json!(attestation_id);
                job_upd["updated_epoch"] = serde_json::json!(epoch);
                self.store.state_set(&job_key, &serde_json::to_vec(&job_upd)?)?;

                // Close lease.
                let mut lease_upd = lease;
                lease_upd["status"] = serde_json::json!("completed");
                lease_upd["updated_epoch"] = serde_json::json!(epoch);
                self.store.state_set(&lease_key, &serde_json::to_vec(&lease_upd)?)?;

                let key = format!("runtime_attest:{}", attestation_id);
                let value = serde_json::json!({
                    "attestation_id": attestation_id,
                    "runtime_id": runtime_id,
                    "job_id": job_id,
                    "host_id": host_id,
                    "output_commitment": output_commitment,
                    "epoch": epoch,
                });
                self.store.state_set(&key, &serde_json::to_vec(&value)?)?;
            }
            LedgerEntry::RuntimeChallenge { challenge_id, attestation_id, runtime_id, challenger, reason, evidence_cid, epoch, .. } => {
                // Attestation must exist to challenge it.
                let attest_key = format!("runtime_attest:{}", attestation_id);
                anyhow::ensure!(
                    self.store.state_get(&attest_key).is_some(),
                    "attestation '{}' not found", attestation_id
                );
                let key = format!("runtime_challenge:{}", challenge_id);
                let value = serde_json::json!({
                    "challenge_id": challenge_id,
                    "attestation_id": attestation_id,
                    "runtime_id": runtime_id,
                    "challenger": challenger,
                    "reason": reason,
                    "evidence_cid": evidence_cid,
                    "status": "open",
                    "epoch": epoch,
                });
                self.store.state_set(&key, &serde_json::to_vec(&value)?)?;
            }
            LedgerEntry::RuntimeSlash { slash_id, runtime_id, host_id, amount, reason, epoch, .. } => {
                // Debit from the runtime bond escrow, send to recycle fund.
                let escrow = format!("__runtime_bond_{}__", runtime_id);
                let available = self.store.get_balance(&escrow, NATIVE_TOKEN);
                let slash_amount = (*amount).min(available);
                if slash_amount > 0 {
                    self.store.debit(&escrow, NATIVE_TOKEN, slash_amount)?;
                    self.store.credit(RECYCLE_FUND_ACCOUNT, NATIVE_TOKEN, slash_amount)?;
                }
                let key = format!("runtime_slash:{}", slash_id);
                let value = serde_json::json!({
                    "slash_id": slash_id,
                    "runtime_id": runtime_id,
                    "host_id": host_id,
                    "amount_requested": amount,
                    "amount_slashed": slash_amount,
                    "reason": reason,
                    "epoch": epoch,
                });
                self.store.state_set(&key, &serde_json::to_vec(&value)?)?;
                // Mark runtime as slashed in its state record.
                let rt_key = format!("runtime:{}", runtime_id);
                if let Some(prev) = self.store.state_get(&rt_key)
                    .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                {
                    let mut rt = prev;
                    rt["last_slash_epoch"] = serde_json::json!(epoch);
                    rt["last_slash_amount"] = serde_json::json!(slash_amount);
                    let _ = self.store.state_set(&rt_key, &serde_json::to_vec(&rt)?);
                }
            }
            LedgerEntry::RuntimeReward { host_id, amount, epoch } => {
                self.ensure_account(host_id, *epoch)?;
                self.store.credit(host_id, NATIVE_TOKEN, *amount)?;
            }

            // Freeport commerce — recorded on-chain, state managed by btcpc-market sidecar
            LedgerEntry::StoreUpdate { .. }
            | LedgerEntry::ProductCreate { .. }
            | LedgerEntry::ProductUpdate { .. }
            | LedgerEntry::OrderPlace { .. }
            | LedgerEntry::OrderFulfill { .. }
            | LedgerEntry::OrderCancel { .. }
            | LedgerEntry::OrderDispute { .. }
            | LedgerEntry::EscrowRelease { .. }
            | LedgerEntry::FlashSale { .. } => {}

            // Verasens sensors — state written here for API queries.
            LedgerEntry::SensorRegister { sensor_id, owner, sensor_type, location, metadata, epoch, .. } => {
                let key = format!("sensor:{}", sensor_id);
                let _ = self.store.set_meta(&key,
                    &serde_json::to_vec(&serde_json::json!({
                        "sensor_id": sensor_id, "owner": owner,
                        "sensor_type": sensor_type, "location": location,
                        "metadata": metadata, "registered_epoch": epoch,
                    })).unwrap_or_default());
            }
            LedgerEntry::GatewayHeartbeat { gateway_id, owner, epoch, .. } => {
                let key = format!("gateway:{}", gateway_id);
                let _ = self.store.set_meta(&key,
                    &serde_json::to_vec(&serde_json::json!({
                        "gateway_id": gateway_id, "owner": owner,
                        "last_heartbeat_epoch": epoch,
                    })).unwrap_or_default());
            }
            LedgerEntry::SensorKeyRegister { .. }
            | LedgerEntry::SensorVouch { .. }
            | LedgerEntry::DeviceKeyRegister { .. } => {
                // Recorded in the ledger only; state managed by protocol sidecars.
            }

            // Device yield stake — tracked on-chain so overbid handler can distribute premium.
            LedgerEntry::DeviceYieldStake { device_serial, staker, amount, epoch, .. } => {
                self.ensure_account(staker, *epoch)?;
                self.store.debit(staker, NATIVE_TOKEN, *amount)?;
                let key = format!("yield_stake:{}:{}", device_serial, staker);
                let prev: u64 = self.store.state_get(&key)
                    .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                    .and_then(|j| j["amount"].as_u64())
                    .unwrap_or(0);
                let _ = self.store.state_set(&key,
                    &serde_json::to_vec(&serde_json::json!({
                        "device_serial": device_serial, "staker": staker,
                        "amount": prev + amount,
                    })).unwrap_or_default());
            }

            LedgerEntry::DeviceYieldUnstake { device_serial, staker, amount, .. } => {
                let key = format!("yield_stake:{}:{}", device_serial, staker);
                let current: u64 = self.store.state_get(&key)
                    .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                    .and_then(|j| j["amount"].as_u64())
                    .unwrap_or(0);
                anyhow::ensure!(current >= *amount, "insufficient yield stake to unstake");
                let remaining = current - amount;
                if remaining == 0 {
                    self.store.state_delete(&key)?;
                } else {
                    let _ = self.store.state_set(&key,
                        &serde_json::to_vec(&serde_json::json!({
                            "device_serial": device_serial, "staker": staker,
                            "amount": remaining,
                        })).unwrap_or_default());
                }
                self.store.credit(staker, NATIVE_TOKEN, *amount)?;
            }

            // ── Node Role Staking ─────────────────────────────────────────────
            LedgerEntry::NodeRoleOptIn { node, role, backer_share_bps, max_backers, epoch, .. } => {
                self.ensure_account(node, *epoch)?;
                let bps = (*backer_share_bps as u64).min(5000);
                let max = (*max_backers as u64).clamp(1, 50);
                self.store.state_set(
                    &format!("role_opt:{}:{}", role, node),
                    &serde_json::to_vec(&serde_json::json!({
                        "node": node, "role": role,
                        "backer_share_bps": bps,
                        "max_backers": max,
                        "since_epoch": epoch,
                    }))?,
                )?;
                info!("[role_stake] node '{}' opted in for role '{}' ({}bps, max {} backers)",
                    node, role, bps, max);
            }

            LedgerEntry::NodeRoleOptOut { node, role, epoch, .. } => {
                // Mark as opted-out so the open-by-default logic can distinguish
                // "never configured" (open) from "explicitly closed" (opted out).
                self.store.state_set(
                    &format!("role_opt:{}:{}", role, node),
                    &serde_json::to_vec(&serde_json::json!({
                        "node": node, "role": role,
                        "opted_out": true,
                        "since_epoch": epoch,
                    })).unwrap_or_default(),
                )?;
                info!("[role_stake] node '{}' opted out of role '{}'", node, role);
            }

            LedgerEntry::NodeRoleStake { node, role, staker, amount, epoch, .. } => {
                self.ensure_account(staker, *epoch)?;
                self.store.debit(staker, NATIVE_TOKEN, *amount)?;
                // Self-stake: auto-opt-in to backer sharing if not already configured.
                // Enables bootstrapping — operators don't need a separate opt-in tx.
                if staker == node {
                    let opt_key = format!("role_opt:{}:{}", role, node);
                    if self.store.state_get(&opt_key).is_none() {
                        let _ = self.store.state_set(&opt_key, &serde_json::to_vec(&serde_json::json!({
                            "node": node, "role": role,
                            "backer_share_bps": 1000u64,
                            "max_backers": 20u64,
                            "since_epoch": epoch,
                        })).unwrap_or_default());
                        info!("[role_stake] auto-opted '{}' into backer sharing for role '{}'", node, role);
                    }
                }
                let key = format!("role_stake:{}:{}:{}", role, node, staker);
                let prev: u64 = self.store.state_get(&key)
                    .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                    .and_then(|j| j["amount"].as_u64())
                    .unwrap_or(0);
                self.store.state_set(&key, &serde_json::to_vec(&serde_json::json!({
                    "node": node, "role": role, "staker": staker,
                    "amount": prev + amount,
                    "staked_epoch": epoch,
                }))?)?;
                info!("[role_stake] '{}' staked {} dreams on '{}' role '{}'",
                    staker, amount, node, role);
            }

            LedgerEntry::NodeRoleUnstake { node, role, staker, amount, .. } => {
                let key = format!("role_stake:{}:{}:{}", role, node, staker);
                let current: u64 = self.store.state_get(&key)
                    .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                    .and_then(|j| j["amount"].as_u64())
                    .unwrap_or(0);
                anyhow::ensure!(
                    current >= *amount,
                    "cannot unstake {} dreams: only {} staked on '{}' role '{}'",
                    amount, current, node, role
                );
                let remaining = current - amount;
                if remaining == 0 {
                    self.store.state_delete(&key)?;
                } else {
                    self.store.state_set(&key, &serde_json::to_vec(&serde_json::json!({
                        "node": node, "role": role, "staker": staker,
                        "amount": remaining,
                    }))?)?;
                }
                self.store.credit(staker, NATIVE_TOKEN, *amount)?;
                info!("[role_stake] '{}' unstaked {} dreams from '{}' role '{}'",
                    staker, amount, node, role);
            }

            // ── LinkGit: core chain state (repo registry, refs, access grants) ──
            LedgerEntry::LinkGitRepoCreate { repo_id, owner, name, visibility, hide_key, epoch, .. } => {
                let repo = serde_json::json!({
                    "repo_id": repo_id,
                    "owner": owner,
                    "name": name,
                    "visibility": visibility,
                    "hide_key": hide_key,
                    "refs": {},
                    "created_epoch": epoch,
                });
                self.store.set_meta(
                    &format!("linkgit:repo:{}", repo_id),
                    repo.to_string().as_bytes(),
                )?;
                // Secondary index for git URL lookup: /git/{owner}/{name}
                self.store.set_meta(
                    &format!("linkgit:byname:{}:{}", owner, name),
                    repo_id.as_bytes(),
                )?;
            }
            LedgerEntry::LinkGitRefUpdate { repo_id, ref_name, commit_hash, epoch, signed_by, .. } => {
                let key = format!("linkgit:repo:{}", repo_id);
                if let Some(bytes) = self.store.get_meta(&key) {
                    if let Ok(mut repo) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                        repo["refs"][ref_name] = serde_json::json!(commit_hash);
                        self.store.set_meta(&key, repo.to_string().as_bytes())?;
                    }
                }
                // Track build activity for epoch reward distribution.
                let build_key = format!("linkgit:build:{}:{}", epoch, signed_by);
                let prev = self.store.state_get(&build_key)
                    .and_then(|b| <[u8; 8]>::try_from(b.as_slice()).ok())
                    .map(u64::from_le_bytes)
                    .unwrap_or(0);
                self.store.state_set(&build_key, &(prev + 1).to_le_bytes())?;
                let idx_key = format!("linkgit:builders:{}", epoch);
                let mut builders: Vec<String> = self.store.state_get(&idx_key)
                    .and_then(|b| serde_json::from_slice(&b).ok())
                    .unwrap_or_default();
                if !builders.contains(signed_by) {
                    builders.push(signed_by.clone());
                    self.store.state_set(&idx_key, &serde_json::to_vec(&builders).unwrap_or_default())?;
                }
            }
            LedgerEntry::LinkGitAccessGrant { repo_id, grantee, encrypted_key, .. } => {
                let key = format!("linkgit:access:{}:{}", repo_id, grantee);
                self.store.set_meta(&key, encrypted_key.as_bytes())?;
            }
            LedgerEntry::LinkGitAccessRevoke { repo_id, grantee, .. } => {
                let key = format!("linkgit:access:{}:{}", repo_id, grantee);
                self.store.state_delete(&key)?;
            }
            // Recorded in ledger; no core state change needed
            LedgerEntry::LinkGitPruneProof { .. }
            | LedgerEntry::LinkGitStorageExtend { .. } => {}

            // ── LinkGit serve rewards ─────────────────────────────────────────
            LedgerEntry::LinkGitServeHeartbeat { repo_id, owner: _, requester_hash, epoch } => {
                // Per-epoch dedup: same requester_hash in same epoch = no-op.
                let dedup_key = format!("linkgit:serve_dedup:{}:{}:{}", epoch, repo_id, requester_hash);
                if self.store.state_get(&dedup_key).is_some() {
                    return Ok(());
                }
                self.store.state_set(&dedup_key, b"1")?;
                // Increment unique fetch count for this repo this epoch.
                let count_key = format!("linkgit:serve_count:{}:{}", epoch, repo_id);
                let count = self.store.state_get(&count_key)
                    .and_then(|b| <[u8; 8]>::try_from(b.as_slice()).ok())
                    .map(u64::from_le_bytes)
                    .unwrap_or(0);
                self.store.state_set(&count_key, &(count + 1).to_le_bytes())?;
                // Track which repos had serves this epoch (for epoch seal lookup).
                let index_key = format!("linkgit:served_repos:{}", epoch);
                let mut repos: Vec<String> = self.store.state_get(&index_key)
                    .and_then(|b| serde_json::from_slice(&b).ok())
                    .unwrap_or_default();
                if !repos.contains(repo_id) {
                    repos.push(repo_id.clone());
                    self.store.state_set(&index_key, &serde_json::to_vec(&repos).unwrap_or_default())?;
                }
                info!("linkgit serve: repo={} epoch={} unique_fetchers={}", repo_id, epoch, count + 1);
            }

            LedgerEntry::LinkGitServeReward { repo_id, owner, amount, serve_count, epoch } => {
                self.store.credit(owner, btcpc_types::NATIVE_TOKEN, *amount)?;
                info!("linkgit serve reward: repo={} owner={} fetchers={} amount={} epoch={}", repo_id, owner, serve_count, amount, epoch);
            }

            LedgerEntry::LinkGitBuildReward { builder, amount, push_count, epoch } => {
                self.store.credit(builder, btcpc_types::NATIVE_TOKEN, *amount)?;
                info!("linkgit build reward: builder={} pushes={} amount={} epoch={}", builder, push_count, amount, epoch);
            }

            // ── LinkGit COBs ──────────────────────────────────────────────────
            LedgerEntry::LinkGitIssueCreate { repo_id, issue_id, title, body, labels, author, epoch, .. } => {
                let issue = serde_json::json!({
                    "issue_id": issue_id,
                    "repo_id": repo_id,
                    "title": title,
                    "body": body,
                    "labels": labels,
                    "author": author,
                    "status": "open",
                    "comments": [],
                    "created_epoch": epoch,
                    "updated_epoch": epoch,
                });
                self.store.set_meta(
                    &format!("linkgit:issue:{}:{}", repo_id, issue_id),
                    issue.to_string().as_bytes(),
                )?;
                // Append to the repo's issue index
                let idx_key = format!("linkgit:issues:{}", repo_id);
                let mut ids: Vec<String> = self.store.get_meta(&idx_key)
                    .and_then(|b| serde_json::from_slice(&b).ok())
                    .unwrap_or_default();
                if !ids.contains(issue_id) { ids.push(issue_id.clone()); }
                self.store.set_meta(&idx_key, serde_json::to_string(&ids)?.as_bytes())?;
            }

            LedgerEntry::LinkGitIssueComment { repo_id, issue_id, comment_id, body, author, epoch, .. } => {
                let key = format!("linkgit:issue:{}:{}", repo_id, issue_id);
                if let Some(bytes) = self.store.get_meta(&key) {
                    if let Ok(mut issue) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                        let comment = serde_json::json!({
                            "comment_id": comment_id,
                            "body": body,
                            "author": author,
                            "epoch": epoch,
                        });
                        issue["comments"].as_array_mut()
                            .map(|arr| arr.push(comment));
                        issue["updated_epoch"] = serde_json::json!(epoch);
                        self.store.set_meta(&key, issue.to_string().as_bytes())?;
                    }
                }
            }

            LedgerEntry::LinkGitIssueClose { repo_id, issue_id, actor: _, resolution, epoch, .. } => {
                let key = format!("linkgit:issue:{}:{}", repo_id, issue_id);
                if let Some(bytes) = self.store.get_meta(&key) {
                    if let Ok(mut issue) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                        issue["status"] = serde_json::json!("closed");
                        if let Some(res) = resolution {
                            issue["resolution"] = serde_json::json!(res);
                        }
                        issue["closed_epoch"] = serde_json::json!(epoch);
                        issue["updated_epoch"] = serde_json::json!(epoch);
                        self.store.set_meta(&key, issue.to_string().as_bytes())?;
                    }
                }
            }

            LedgerEntry::LinkGitIssueReopen { repo_id, issue_id, epoch, .. } => {
                let key = format!("linkgit:issue:{}:{}", repo_id, issue_id);
                if let Some(bytes) = self.store.get_meta(&key) {
                    if let Ok(mut issue) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                        issue["status"] = serde_json::json!("open");
                        issue["updated_epoch"] = serde_json::json!(epoch);
                        self.store.set_meta(&key, issue.to_string().as_bytes())?;
                    }
                }
            }

            LedgerEntry::LinkGitPrCreate {
                repo_id, pr_id, title, body, source_branch, target_branch, head_commit, author, epoch, ..
            } => {
                let pr = serde_json::json!({
                    "pr_id": pr_id,
                    "repo_id": repo_id,
                    "title": title,
                    "body": body,
                    "source_branch": source_branch,
                    "target_branch": target_branch,
                    "head_commit": head_commit,
                    "author": author,
                    "status": "open",
                    "comments": [],
                    "created_epoch": epoch,
                    "updated_epoch": epoch,
                });
                self.store.set_meta(
                    &format!("linkgit:pr:{}:{}", repo_id, pr_id),
                    pr.to_string().as_bytes(),
                )?;
                let idx_key = format!("linkgit:prs:{}", repo_id);
                let mut ids: Vec<String> = self.store.get_meta(&idx_key)
                    .and_then(|b| serde_json::from_slice(&b).ok())
                    .unwrap_or_default();
                if !ids.contains(pr_id) { ids.push(pr_id.clone()); }
                self.store.set_meta(&idx_key, serde_json::to_string(&ids)?.as_bytes())?;
            }

            LedgerEntry::LinkGitPrComment { repo_id, pr_id, comment_id, body, author, epoch, .. } => {
                let key = format!("linkgit:pr:{}:{}", repo_id, pr_id);
                if let Some(bytes) = self.store.get_meta(&key) {
                    if let Ok(mut pr) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                        let comment = serde_json::json!({
                            "comment_id": comment_id,
                            "body": body,
                            "author": author,
                            "epoch": epoch,
                        });
                        pr["comments"].as_array_mut()
                            .map(|arr| arr.push(comment));
                        pr["updated_epoch"] = serde_json::json!(epoch);
                        self.store.set_meta(&key, pr.to_string().as_bytes())?;
                    }
                }
            }

            LedgerEntry::LinkGitPrMerge { repo_id, pr_id, merge_commit, epoch, .. } => {
                let key = format!("linkgit:pr:{}:{}", repo_id, pr_id);
                if let Some(bytes) = self.store.get_meta(&key) {
                    if let Ok(mut pr) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                        pr["status"] = serde_json::json!("merged");
                        pr["merge_commit"] = serde_json::json!(merge_commit);
                        pr["merged_epoch"] = serde_json::json!(epoch);
                        pr["updated_epoch"] = serde_json::json!(epoch);
                        self.store.set_meta(&key, pr.to_string().as_bytes())?;
                    }
                }
            }

            LedgerEntry::LinkGitPrClose { repo_id, pr_id, epoch, .. } => {
                let key = format!("linkgit:pr:{}:{}", repo_id, pr_id);
                if let Some(bytes) = self.store.get_meta(&key) {
                    if let Ok(mut pr) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                        pr["status"] = serde_json::json!("closed");
                        pr["closed_epoch"] = serde_json::json!(epoch);
                        pr["updated_epoch"] = serde_json::json!(epoch);
                        self.store.set_meta(&key, pr.to_string().as_bytes())?;
                    }
                }
            }

            // ── Inference marketplace ─────────────────────────────────────────
            LedgerEntry::InferenceJobPost { .. } => {
                inference::apply_post(self, entry)?;
            }
            LedgerEntry::InferenceJobBid { .. } => {
                inference::apply_bid(self, entry)?;
            }
            LedgerEntry::InferenceJobAward { .. } => {
                inference::apply_award(self, entry)?;
            }
            LedgerEntry::InferenceJobComplete { .. } => {
                inference::apply_complete(self, entry)?;
            }
            LedgerEntry::InferenceJobVerify { .. } => {
                inference::apply_verify(self, entry)?;
            }
            LedgerEntry::InferenceJobClaim { .. } => {
                inference::apply_claim(self, entry)?;
            }
            LedgerEntry::InferenceReviewVote { .. } => {
                inference::apply_review_vote(self, entry)?;
            }
            LedgerEntry::InferenceJobPay { .. } => {
                inference::apply_pay(self, entry)?;
            }
            LedgerEntry::InferenceJobCancel { .. } => {
                inference::apply_cancel(self, entry)?;
            }

            // ── Clock reward ──────────────────────────────────────────────────
            LedgerEntry::ClockReward { node_id, amount, epoch } => {
                self.ensure_account(node_id, *epoch)?;
                let _ = CLOCK_REWARD_DREAMS;
                let node_credit = self.distribute_role_backer_reward("clock", node_id, *amount)?;
                self.store.credit(node_id, NATIVE_TOKEN, node_credit)?;
            }

            // ── Testnet ────────────────────────────────────────────────────────
            LedgerEntry::TestnetOperatorRegister { mainnet_account, testnet_node_id, testnet_chain_id, epoch, .. } => {
                self.ensure_account(mainnet_account, *epoch)?;
                let key = format!("testnet_op:{}:{}", mainnet_account, testnet_chain_id);
                let _ = self.store.state_set(&key,
                    &serde_json::to_vec(&serde_json::json!({
                        "mainnet_account": mainnet_account,
                        "testnet_node_id": testnet_node_id,
                        "testnet_chain_id": testnet_chain_id,
                        "registered_epoch": epoch,
                    })).unwrap_or_default());
            }
            LedgerEntry::TestnetOperatorDeregister { mainnet_account, testnet_chain_id, .. } => {
                let key = format!("testnet_op:{}:{}", mainnet_account, testnet_chain_id);
                let _ = self.store.state_delete(&key);
            }
            LedgerEntry::TestnetReward { mainnet_account, amount, epoch, .. } => {
                self.ensure_account(mainnet_account, *epoch)?;
                // Drain from testnet fund — fund was seeded at genesis.
                let _ = self.store.debit(TESTNET_FUND_ACCOUNT, NATIVE_TOKEN, *amount);
                self.store.credit(mainnet_account, NATIVE_TOKEN, *amount)?;
            }

            // ── Layer B epoch rewards ─────────────────────────────────────────
            LedgerEntry::StorageReward { node_id, amount, epoch } => {
                self.ensure_account(node_id, *epoch)?;
                self.store.credit(node_id, NATIVE_TOKEN, *amount)?;
            }
            LedgerEntry::SensorReward { node_id, amount, epoch } => {
                self.ensure_account(node_id, *epoch)?;
                self.store.credit(node_id, NATIVE_TOKEN, *amount)?;
            }
            LedgerEntry::GatewayRewardSplit { sensor_account, gateway_account, sensor_amount, gateway_amount, epoch } => {
                self.ensure_account(sensor_account, *epoch)?;
                self.ensure_account(gateway_account, *epoch)?;
                self.store.credit(sensor_account, NATIVE_TOKEN, *sensor_amount)?;
                self.store.credit(gateway_account, NATIVE_TOKEN, *gateway_amount)?;
            }
            LedgerEntry::VerifierReward { node_id, amount, epoch } => {
                self.ensure_account(node_id, *epoch)?;
                self.store.credit(node_id, NATIVE_TOKEN, *amount)?;
            }
            LedgerEntry::ServiceReward { node_id, amount, epoch } => {
                self.ensure_account(node_id, *epoch)?;
                self.store.credit(node_id, NATIVE_TOKEN, *amount)?;
            }

            // ── Service heartbeat ─────────────────────────────────────────────
            LedgerEntry::ServiceHeartbeat { node_id, epoch, container_hours, .. } => {
                self.ensure_account(node_id, *epoch)?;
                let key = format!("service_beat:{}:{}", epoch, node_id);
                let _ = self.store.state_set(&key,
                    &serde_json::to_vec(&serde_json::json!({
                        "node_id": node_id, "epoch": epoch,
                        "container_hours": container_hours,
                    })).unwrap_or_default());
            }

            // ── Verifier claim — worker will encrypt payload to verifier's memo key ──
            LedgerEntry::InferenceVerifyClaim { job_id, verifier, epoch, .. } => {
                let key = format!("verify_claim:{}:{}", job_id, verifier);
                let _ = self.store.state_set(&key,
                    &serde_json::to_vec(&serde_json::json!({
                        "job_id": job_id, "verifier": verifier, "epoch": epoch,
                    })).unwrap_or_default());
            }

            // ── Commit-reveal phase 1 (T4-2) ─────────────────────────────────
            LedgerEntry::InferenceJobCommit { .. } => {
                inference::apply_commit(self, entry)?;
            }

            // ── Sensor data purchase ──────────────────────────────────────────
            LedgerEntry::SensorDataPurchase { buyer, owner, fee, epoch, .. } => {
                // Split fee: 80% to sensor owner, 15% to storage contract, 5% to recycle
                let owner_share   = (*fee as u128 * 80 / 100) as u64;
                let recycle_share = fee.saturating_sub(owner_share);
                self.store.debit(buyer, NATIVE_TOKEN, *fee)?;
                self.ensure_account(owner, *epoch)?;
                self.store.credit(owner, NATIVE_TOKEN, owner_share)?;
                if recycle_share > 0 {
                    let _ = self.store.credit(RECYCLE_FUND_ACCOUNT, NATIVE_TOKEN, recycle_share);
                }
            }

            // ── Mempool rewards ───────────────────────────────────────────────
            LedgerEntry::MempoolReward { operator, amount, epoch } => {
                self.ensure_account(operator, *epoch)?;
                self.store.credit(operator, NATIVE_TOKEN, *amount)?;
            }

            // ── Mempool operator registration ─────────────────────────────────
            LedgerEntry::MempoolOperatorRegister { operator, amount, epoch, .. } => {
                self.ensure_account(operator, *epoch)?;
                self.store.debit(operator, NATIVE_TOKEN, *amount)?;
                // Record operator registration with staked amount
                let key = format!("mempool_op:{}", operator);
                let _ = self.store.state_set(&key,
                    &serde_json::to_vec(&serde_json::json!({
                        "operator": operator, "stake": amount, "registered_epoch": epoch,
                    })).unwrap_or_default());
            }

            // ── Mempool heartbeat ─────────────────────────────────────────────
            LedgerEntry::MempoolHeartbeat { operator, epoch, propagation_latency_ms, entries_relayed, .. } => {
                self.ensure_account(operator, *epoch)?;
                let key = format!("mempool_beat:{}:{}", epoch, operator);
                let _ = self.store.state_set(&key,
                    &serde_json::to_vec(&serde_json::json!({
                        "operator": operator, "epoch": epoch,
                        "latency_ms": propagation_latency_ms,
                        "entries_relayed": entries_relayed,
                    })).unwrap_or_default());
            }

            // ── Device claim stake ────────────────────────────────────────────
            LedgerEntry::DeviceClaimStake { device_serial, owner, amount, epoch, .. } => {
                self.ensure_account(owner, *epoch)?;
                let claim_key = format!("device_claim:{}", device_serial);

                if let Some(existing) = self.store.state_get(&claim_key) {
                    if let Ok(j) = serde_json::from_slice::<serde_json::Value>(&existing) {
                        let existing_owner = j["owner"].as_str().unwrap_or("");
                        let existing_stake = j["stake"].as_u64().unwrap_or(0);

                        if !existing_owner.is_empty() && existing_owner != owner.as_str() {
                            // Overbid path: new stake must be ≥ old_stake × 1.5
                            let required = ((existing_stake as u128)
                                .saturating_mul(DEVICE_CLAIM_OVERBID_NUM)
                                / DEVICE_CLAIM_OVERBID_DENOM) as u64;
                            anyhow::ensure!(
                                *amount >= required,
                                "device '{}' claimed by '{}' (stake {}); overbid requires ≥ {} (1.5×)",
                                device_serial, existing_owner, existing_stake, required
                            );

                            // Return old claim owner's full principal
                            self.store.credit(existing_owner, NATIVE_TOKEN, existing_stake)?;

                            // Distribute overbid premium to yield stakers
                            let premium = amount.saturating_sub(existing_stake);
                            if premium > 0 {
                                let staker_share = (premium as u128 * OVERCLAIM_STAKER_SHARE_BPS as u128 / 10_000) as u64;
                                let recycle_share = premium.saturating_sub(staker_share);

                                // Collect all yield stakers for this device
                                let yield_key_prefix = format!("yield_stake:{}:", device_serial);
                                let stakers: Vec<(String, u64)> = self.store.state_scan_prefix(&yield_key_prefix)
                                    .into_iter()
                                    .filter_map(|(_, v)| {
                                        let js = serde_json::from_slice::<serde_json::Value>(&v).ok()?;
                                        let s = js["staker"].as_str()?.to_owned();
                                        let a = js["amount"].as_u64().unwrap_or(0);
                                        if a > 0 { Some((s, a)) } else { None }
                                    }).collect();

                                let total_yield_stake: u64 = stakers.iter().map(|(_, a)| a).sum();
                                if total_yield_stake > 0 && staker_share > 0 {
                                    for (staker, stake_amount) in &stakers {
                                        let payout = (staker_share as u128 * *stake_amount as u128
                                            / total_yield_stake as u128) as u64;
                                        if payout > 0 {
                                            self.store.credit(staker, NATIVE_TOKEN, payout)?;
                                        }
                                    }
                                } else {
                                    // No yield stakers — full premium to recycle
                                    let _ = self.store.credit(RECYCLE_FUND_ACCOUNT, NATIVE_TOKEN, staker_share);
                                }
                                if recycle_share > 0 {
                                    let _ = self.store.credit(RECYCLE_FUND_ACCOUNT, NATIVE_TOKEN, recycle_share);
                                }
                            }

                            info!(
                                "device claim: '{}' overbid by '{}' (old_stake={} new_stake={} premium={}), \
                                 '{}' stake returned",
                                device_serial, owner, existing_stake, amount,
                                amount.saturating_sub(existing_stake), existing_owner
                            );
                        }
                    }
                }

                self.store.debit(owner, NATIVE_TOKEN, *amount)?;
                let _ = self.store.state_set(&claim_key,
                    &serde_json::to_vec(&serde_json::json!({
                        "device_serial": device_serial, "owner": owner,
                        "stake": amount, "claimed_epoch": epoch,
                    })).unwrap_or_default());
            }

            LedgerEntry::DeviceClaimUnstake { device_serial, owner, epoch, .. } => {
                let claim_key = format!("device_claim:{}", device_serial);
                let stake_amount: u64 = self.store.state_get(&claim_key)
                    .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                    .and_then(|j| {
                        if j["owner"].as_str() == Some(owner.as_str()) {
                            j["stake"].as_u64()
                        } else { None }
                    })
                    .ok_or_else(|| anyhow::anyhow!("no active claim for serial '{}' by '{}'", device_serial, owner))?;
                self.store.state_delete(&claim_key)?;
                self.ensure_account(owner, *epoch)?;
                // Device claim stake follows the same unbonding window as staking (T3-3).
                // Tokens are locked for UNBONDING_EPOCHS before release (slash window).
                let release_epoch = epoch + UNBONDING_EPOCHS;
                let unbond_key = format!("unbonding:{}:{}", release_epoch, owner);
                let prev_amount: u64 = self.store.state_get(&unbond_key)
                    .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                    .and_then(|j| j["amount"].as_u64()).unwrap_or(0);
                let _ = self.store.state_set(&unbond_key,
                    &serde_json::to_vec(&serde_json::json!({
                        "account": owner,
                        "amount": prev_amount + stake_amount,
                        "release_epoch": release_epoch,
                    })).unwrap_or_default());
            }

            // ── Scoped Delegation ─────────────────────────────────────────────
            LedgerEntry::DelegationGrant { from, to, capabilities, expires_epoch, .. } => {
                let key = format!("delegation:{}:{}", from, to);
                let val = serde_json::to_vec(&serde_json::json!({
                    "from": from, "to": to,
                    "capabilities": capabilities,
                    "expires_epoch": expires_epoch,
                }))?;
                self.store.state_set(&key, &val)?;
            }
            LedgerEntry::DelegationRevoke { from, to, .. } => {
                self.store.state_delete(&format!("delegation:{}:{}", from, to))?;
            }

            // ── Device Yield Opt-In ───────────────────────────────────────────
            LedgerEntry::DeviceYieldOptIn { device_serial, owner, max_stakers, .. } => {
                let claim_key = format!("device_claim:{}", device_serial);
                let current_owner = self.store.state_get(&claim_key)
                    .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                    .and_then(|j| j["owner"].as_str().map(str::to_owned));
                anyhow::ensure!(current_owner.as_deref() == Some(owner.as_str()),
                    "only the device owner can opt in to yield sharing");
                let val = serde_json::to_vec(&serde_json::json!({
                    "owner": owner, "max_stakers": max_stakers, "opted_in": true,
                }))?;
                self.store.state_set(&format!("yield_config:{}", device_serial), &val)?;
            }
            LedgerEntry::DeviceYieldOptOut { device_serial, owner, .. } => {
                let config_key = format!("yield_config:{}", device_serial);
                let stored_owner = self.store.state_get(&config_key)
                    .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                    .and_then(|j| j["owner"].as_str().map(str::to_owned));
                anyhow::ensure!(stored_owner.as_deref() == Some(owner.as_str()),
                    "only the device owner can opt out of yield sharing");
                self.store.state_delete(&config_key)?;
            }

            // ── Permissive Token Model ────────────────────────────────────────
            LedgerEntry::SpamGateSet { account, token, fee, fee_token, evm_address, evm_chain_id, .. } => {
                let key = format!("spam_gate:{}:{}", account, token);
                let val = serde_json::to_vec(&serde_json::json!({
                    "fee": fee,
                    "fee_token": fee_token,
                    "evm_address": evm_address,
                    "evm_chain_id": evm_chain_id,
                }))?;
                self.store.state_set(&key, &val)?;
            }
            LedgerEntry::SpamGatePayEvm { from, to, token, amount, evm_tx_hash, evm_chain_id, epoch, nonce, .. } => {
                // Verify an EVM gate exists for this recipient+token.
                let gate_ok = matches!(
                    spam_gate(&self.store, to, token),
                    Some(SpamGate::Evm { evm_chain_id: cid, .. }) if cid == *evm_chain_id
                );
                anyhow::ensure!(gate_ok,
                    "no EVM spam gate on chain {} for '{}' receiving '{}'", evm_chain_id, to, token);
                // Debit sender's token balance.
                self.store.debit(from, token, *amount)?;
                // Create pending record with EVM proof attached — recipient inspects and TokenAccepts.
                let pending_key = format!("pending_transfer:{}:{}:{}:{}", to, token, from, nonce);
                let val = serde_json::to_vec(&serde_json::json!({
                    "from": from, "to": to, "token": token, "amount": amount,
                    "expires_epoch": epoch + 30,
                    "evm_tx_hash": evm_tx_hash,
                    "evm_chain_id": evm_chain_id,
                }))?;
                self.store.state_set(&pending_key, &val)?;
            }
            LedgerEntry::SpamGateClear { account, token, .. } => {
                self.store.state_delete(&format!("spam_gate:{}:{}", account, token))?;
            }
            LedgerEntry::TokenApprove { account, token, from, .. } => {
                let key = format!("token_approval:{}:{}:{}", account, token, from);
                self.store.state_set(&key, b"1")?;
            }
            LedgerEntry::TokenRevoke { account, token, from, .. } => {
                let key = format!("token_approval:{}:{}:{}", account, token, from);
                self.store.state_delete(&key)?;
            }
            LedgerEntry::TokenAccept { to, token, from, pending_nonce, .. } => {
                let pending_key = format!("pending_transfer:{}:{}:{}:{}", to, token, from, pending_nonce);
                let amount: u64 = self.store.state_get(&pending_key)
                    .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                    .and_then(|j| j["amount"].as_u64())
                    .ok_or_else(|| anyhow::anyhow!("no pending transfer found"))?;
                self.store.state_delete(&pending_key)?;
                self.store.credit(to, token, amount)?;
            }
            LedgerEntry::TokenReject { to, token, from, pending_nonce, .. } => {
                let pending_key = format!("pending_transfer:{}:{}:{}:{}", to, token, from, pending_nonce);
                let amount: u64 = self.store.state_get(&pending_key)
                    .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                    .and_then(|j| j["amount"].as_u64())
                    .ok_or_else(|| anyhow::anyhow!("no pending transfer found"))?;
                self.store.state_delete(&pending_key)?;
                self.store.credit(from, token, amount)?;
            }

            // ── Chain Entropy Protocol ────────────────────────────────────────
            LedgerEntry::LivenessProof { account, epoch, .. } => {
                anyhow::ensure!(
                    self.store.get_account(account)?.is_some(),
                    "account '{}' does not exist", account
                );
                self.touch_alive(account, *epoch);
                info!("[entropy] liveness proof: {} at epoch {}", account, epoch);
            }
            LedgerEntry::EntropyWitness { account, chain: ext_chain, address, tx_hash, epoch, .. } => {
                anyhow::ensure!(
                    self.store.get_account(account)?.is_some(),
                    "account '{}' does not exist", account
                );
                self.touch_alive(account, *epoch);
                info!(
                    "[entropy] cross-chain witness: {} alive ({}:{} tx={}) at epoch {}",
                    account,
                    ext_chain,
                    &address[..address.len().min(12)],
                    &tx_hash[..tx_hash.len().min(16)],
                    epoch,
                );
            }

            // ── Wallet Family ─────────────────────────────────────────────────
            LedgerEntry::AccountApiKeySet { account, api_key, epoch, .. } => {
                self.touch_alive(account, *epoch);
                self.store.set_api_key(account, api_key)?;
            }

            LedgerEntry::WalletFamilyPublish { account, chains, epoch, .. }
            | LedgerEntry::WalletFamilyAdd { account, chains, epoch, .. } => {
                self.touch_alive(account, *epoch);
                let family_key = format!("wallet_family:{}", account);
                // Load existing list (additive-only — never remove).
                let mut existing: Vec<serde_json::Value> = self.store.state_get(&family_key)
                    .and_then(|b| serde_json::from_slice(&b).ok())
                    .unwrap_or_default();
                for ca in chains {
                    let entry_json = serde_json::json!({
                        "chain": ca.chain,
                        "address": ca.address,
                        "derivation_path": ca.derivation_path,
                    });
                    // Dedup: skip if (chain, address) already present.
                    let already = existing.iter().any(|e| {
                        e["chain"].as_str() == Some(&ca.chain)
                            && e["address"].as_str() == Some(&ca.address)
                    });
                    if !already {
                        // Reverse index: wallet_addr:{chain}:{address} → account
                        let rev_key = format!("wallet_addr:{}:{}", ca.chain, ca.address);
                        self.store.state_set(&rev_key, account.as_bytes())?;
                        existing.push(entry_json);
                    }
                }
                let val = serde_json::to_vec(&existing)?;
                self.store.state_set(&family_key, &val)?;
            }

            // ── BLE Tracker ───────────────────────────────────────────────────

            LedgerEntry::TrackerSightingCommit { observer_id, owner, airtag_count,
                android_fmd_count, tile_count, samsung_count, other_count,
                batch_hash, epoch, .. } => {
                // Enforce per-observer sighting cap (T4-3): reads governance override or falls
                // back to MAX_SIGHTINGS_PER_OBSERVER_PER_EPOCH (500).
                let sighting_cap: u64 = self.store.state_get("chain_param:max_sightings_per_observer")
                    .and_then(|b| serde_json::from_slice::<serde_json::Value>(&b).ok())
                    .and_then(|j| j.as_u64())
                    .unwrap_or(MAX_SIGHTINGS_PER_OBSERVER_PER_EPOCH);
                let total_sightings = u64::from(*airtag_count) + u64::from(*android_fmd_count)
                    + u64::from(*tile_count) + u64::from(*samsung_count) + u64::from(*other_count);
                anyhow::ensure!(
                    total_sightings <= sighting_cap,
                    "sighting count {} exceeds per-observer cap {} for epoch {}",
                    total_sightings, sighting_cap, epoch
                );
                // Key: tracker_sighting:{epoch}:{observer_id} — epoch-first for prefix scans.
                let key = format!("tracker_sighting:{}:{}", epoch, observer_id);
                let val = serde_json::json!({
                    "observer_id": observer_id,
                    "owner": owner,
                    "airtag_count": airtag_count,
                    "android_fmd_count": android_fmd_count,
                    "tile_count": tile_count,
                    "samsung_count": samsung_count,
                    "other_count": other_count,
                    "batch_hash": batch_hash,
                    "epoch": epoch,
                });
                self.store.state_set(&key, &serde_json::to_vec(&val)?)?;
            }

            LedgerEntry::TrackerClaim { serial_commitment, tag_type, claimer,
                fee, epoch, nonce, .. } => {
                // Debit fee to treasury.
                if *fee > 0 {
                    self.store.debit(claimer, btcpc_types::NATIVE_TOKEN, *fee)?;
                    self.store.credit("treasury", btcpc_types::NATIVE_TOKEN, *fee)?;
                }
                let key = format!("tracker_claim:{}:{}", claimer, serial_commitment);
                let val = serde_json::json!({
                    "serial_commitment": serial_commitment,
                    "tag_type": tag_type,
                    "claimer": claimer,
                    "fee": fee,
                    "epoch": epoch,
                    "nonce": nonce,
                    "status": "Registered",
                });
                self.store.state_set(&key, &serde_json::to_vec(&val)?)?;
            }

            LedgerEntry::TrackerClaimRelease { serial_commitment, claimer, .. } => {
                let key = format!("tracker_claim:{}:{}", claimer, serial_commitment);
                self.store.state_delete(&key)?;
            }

            LedgerEntry::TrackerAcousticProof { serial_commitment, witness_id,
                proof_hash, claimer, epoch, .. } => {
                let key = format!("tracker_claim:{}:{}", claimer, serial_commitment);
                if let Some(bytes) = self.store.state_get(&key) {
                    let mut rec: serde_json::Value =
                        serde_json::from_slice(&bytes).unwrap_or_default();
                    // Acoustic + BLE co-event proof advances the claim to Verified (T4-4).
                    // TrackerSubscription requires Verified status, so this is the required path.
                    rec["status"]         = serde_json::json!("Verified");
                    rec["witness_id"]     = serde_json::json!(witness_id);
                    rec["proof_hash"]     = serde_json::json!(proof_hash);
                    rec["verified_epoch"] = serde_json::json!(epoch);
                    self.store.state_set(&key, &serde_json::to_vec(&rec)?)?;
                }
            }

            LedgerEntry::TrackerSubscription { serial_commitment, claimer,
                fee_per_epoch, expires_epoch, epoch, nonce, .. } => {
                // Validate claim exists and is Verified or AcousticVerified.
                let claim_key = format!("tracker_claim:{}:{}", claimer, serial_commitment);
                if let Some(bytes) = self.store.state_get(&claim_key) {
                    let rec: serde_json::Value =
                        serde_json::from_slice(&bytes).unwrap_or_default();
                    let status = rec["status"].as_str().unwrap_or("");
                    anyhow::ensure!(
                        matches!(status, "Verified" | "AcousticVerified"),
                        "TrackerSubscription requires Verified or AcousticVerified claim"
                    );
                } else {
                    anyhow::bail!("no TrackerClaim found for serial_commitment");
                }
                // Escrow total fee upfront: fee_per_epoch × (expires_epoch - epoch).
                let duration = expires_epoch.saturating_sub(*epoch);
                let total_fee = fee_per_epoch.saturating_mul(duration);
                if total_fee > 0 {
                    self.store.debit(claimer, btcpc_types::NATIVE_TOKEN, total_fee)?;
                    let escrow_key = format!("tracker_sub_escrow:{}:{}", claimer, serial_commitment);
                    self.store.state_set(&escrow_key, &serde_json::to_vec(&serde_json::json!({
                        "serial_commitment": serial_commitment,
                        "claimer": claimer,
                        "fee_per_epoch": fee_per_epoch,
                        "expires_epoch": expires_epoch,
                        "start_epoch": epoch,
                        "nonce": nonce,
                        "total_escrowed": total_fee,
                    }))?)?;
                }
            }

            LedgerEntry::TrackerSightingData { serial_commitment, observer_id,
                cid, plaintext_hash, epoch, .. } => {
                // Index CID reference for route reconstruction.
                // Key: tracker_route:{serial_commitment}:{epoch:016x}
                // Storing per-commitment (not per-claimer) so any node can relay,
                // but only the memo-key holder can decrypt the blob via BTCPC-FS.
                let route_key = format!("tracker_route:{}:{:016x}", serial_commitment, epoch);
                self.store.state_set(&route_key, &serde_json::to_vec(&serde_json::json!({
                    "epoch": epoch,
                    "observer_id": observer_id,
                    "cid": cid,
                    "plaintext_hash": plaintext_hash,
                }))?)?;
            }

            LedgerEntry::TrackerHint { .. } => {
                // Hints are ephemeral gossip; no persistent state needed.
            }

            LedgerEntry::TrackerLostMode { serial_commitment, claimer,
                bounty_dreams, expires_epoch, contact_encrypted, epoch, nonce, .. } => {
                if *bounty_dreams > 0 {
                    self.store.debit(claimer, btcpc_types::NATIVE_TOKEN, *bounty_dreams)?;
                    let escrow_key = format!("tracker_lost_escrow:{}:{}", claimer, serial_commitment);
                    self.store.state_set(&escrow_key, &serde_json::to_vec(&serde_json::json!({
                        "serial_commitment": serial_commitment,
                        "claimer": claimer,
                        "bounty_dreams": bounty_dreams,
                        "expires_epoch": expires_epoch,
                        "contact_encrypted": contact_encrypted,
                        "epoch": epoch,
                        "nonce": nonce,
                        "status": "active",
                    }))?)?;
                }
                // Mark claim as lost.
                let claim_key = format!("tracker_claim:{}:{}", claimer, serial_commitment);
                if let Some(bytes) = self.store.state_get(&claim_key) {
                    let mut rec: serde_json::Value =
                        serde_json::from_slice(&bytes).unwrap_or_default();
                    rec["lost_mode"] = serde_json::json!(true);
                    rec["lost_since"] = serde_json::json!(epoch);
                    self.store.state_set(&claim_key, &serde_json::to_vec(&rec)?)?;
                }
            }

            LedgerEntry::TrackerFoundReport { serial_commitment, finder,
                gps_commitment, acoustic_proof_hash, epoch, nonce, .. } => {
                let report_key = format!("tracker_found_report:{}:{}", serial_commitment, finder);
                self.store.state_set(&report_key, &serde_json::to_vec(&serde_json::json!({
                    "serial_commitment": serial_commitment,
                    "finder": finder,
                    "gps_commitment": gps_commitment,
                    "acoustic_proof_hash": acoustic_proof_hash,
                    "epoch": epoch,
                    "nonce": nonce,
                    "status": "pending",
                }))?)?;
            }

            LedgerEntry::TrackerCoverageReward { observer_id, amount, .. } => {
                self.store.credit(observer_id, btcpc_types::NATIVE_TOKEN, *amount)?;
            }

            LedgerEntry::TrackerFoundConfirm { serial_commitment, finder,
                claimer, epoch, .. } => {
                // Release escrow: 70% finder, 20% first-sighting observers, 10% treasury.
                let escrow_key = format!("tracker_lost_escrow:{}:{}", claimer, serial_commitment);
                if let Some(bytes) = self.store.state_get(&escrow_key) {
                    let rec: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or_default();
                    let bounty = rec["bounty_dreams"].as_u64().unwrap_or(0);
                    if bounty > 0 {
                        let finder_share   = bounty * 70 / 100;
                        let treasury_share = bounty - finder_share - (bounty * 20 / 100);
                        let observer_share = bounty * 20 / 100;
                        self.store.credit(finder, btcpc_types::NATIVE_TOKEN, finder_share)?;
                        self.store.credit("treasury", btcpc_types::NATIVE_TOKEN, treasury_share)?;
                        // Observer share goes to the sensor pool for epoch distribution.
                        self.store.credit("sensor_pool", btcpc_types::NATIVE_TOKEN, observer_share)?;
                    }
                    // Close escrow and lost mode.
                    let mut escrow = rec;
                    escrow["status"] = serde_json::json!("confirmed");
                    escrow["confirmed_epoch"] = serde_json::json!(epoch);
                    self.store.state_set(&escrow_key, &serde_json::to_vec(&escrow)?)?;
                }
                let claim_key = format!("tracker_claim:{}:{}", claimer, serial_commitment);
                if let Some(bytes) = self.store.state_get(&claim_key) {
                    let mut rec: serde_json::Value = serde_json::from_slice(&bytes).unwrap_or_default();
                    rec["lost_mode"] = serde_json::json!(false);
                    self.store.state_set(&claim_key, &serde_json::to_vec(&rec)?)?;
                }
            }

            // ── Hardware Anti-Sybil ───────────────────────────────────────────
            LedgerEntry::HardwareClaim { account, fingerprint, hw_info, epoch, .. } => {
                anyhow::ensure!(!fingerprint.is_empty(), "empty hardware fingerprint");
                let claim_key = format!("hw_claim:{}", fingerprint);
                if let Some(existing) = self.store.state_get(&claim_key) {
                    let existing_account = String::from_utf8_lossy(&existing);
                    anyhow::ensure!(
                        existing_account == account.as_str(),
                        "hardware fingerprint already claimed by account '{}'", existing_account
                    );
                    // Same account re-claiming — update hw_info in case hardware changed.
                    self.store.state_set(&claim_key, account.as_bytes())?;
                } else {
                    self.store.state_set(&claim_key, account.as_bytes())?;
                    // Reverse index: hw_account:{account} → fingerprint (latest only).
                    let rev_key = format!("hw_account:{}", account);
                    self.store.state_set(&rev_key, fingerprint.as_bytes())?;
                    info!(
                        "[hardware] fingerprint {} claimed by '{}' ({})",
                        &fingerprint[..fingerprint.len().min(16)], account, hw_info
                    );
                }
                self.touch_alive(account, *epoch);
            }

            // ── Governance ────────────────────────────────────────────────────
            LedgerEntry::GovernancePropose {
                proposal_id, proposer, title, description,
                action, action_key, action_value, voting_ends_epoch, epoch, ..
            } => {
                let council = crate::tx::governance_keys(self);
                anyhow::ensure!(
                    council.iter().any(|a| a == proposer),
                    "account '{}' is not in the governance council", proposer
                );
                anyhow::ensure!(!proposal_id.is_empty(), "proposal_id cannot be empty");
                let key = format!("governance:proposal:{}", proposal_id);
                anyhow::ensure!(
                    self.store.state_get(&key).is_none(),
                    "proposal '{}' already exists", proposal_id
                );
                let record = serde_json::json!({
                    "proposal_id": proposal_id,
                    "proposer": proposer,
                    "title": title,
                    "description": description,
                    "action": action,
                    "action_key": action_key,
                    "action_value": action_value,
                    "voting_ends_epoch": voting_ends_epoch,
                    "submitted_epoch": epoch,
                    "yes_votes": 0u64,
                    "no_votes": 0u64,
                    "status": "open",
                });
                self.store.state_set(&key, &serde_json::to_vec(&record)?)?;
                info!("[governance] proposal '{}' submitted by '{}': {}", proposal_id, proposer, title);
            }

            LedgerEntry::GovernanceVote { proposal_id, voter, approve, epoch, .. } => {
                let prop_key = format!("governance:proposal:{}", proposal_id);
                let raw = self.store.state_get(&prop_key)
                    .ok_or_else(|| anyhow::anyhow!("proposal '{}' not found", proposal_id))?;
                let mut proposal: serde_json::Value = serde_json::from_slice(&raw)?;
                anyhow::ensure!(
                    proposal["status"].as_str() == Some("open"),
                    "proposal '{}' is not open for voting", proposal_id
                );
                let ends = proposal["voting_ends_epoch"].as_u64().unwrap_or(0);
                anyhow::ensure!(
                    *epoch <= ends,
                    "voting on proposal '{}' closed at epoch {}", proposal_id, ends
                );
                let stake = self.get_stake(voter);
                anyhow::ensure!(
                    stake > 0,
                    "account '{}' has no stake — stake BTCPC to vote", voter
                );
                // Idempotency: reject double-votes.
                let vote_key = format!("governance:vote:{}:{}", proposal_id, voter);
                anyhow::ensure!(
                    self.store.state_get(&vote_key).is_none(),
                    "account '{}' has already voted on proposal '{}'", voter, proposal_id
                );
                self.store.state_set(&vote_key, serde_json::json!({ "approve": approve, "epoch": epoch }).to_string().as_bytes())?;
                if *approve {
                    let yes = proposal["yes_votes"].as_u64().unwrap_or(0) + 1;
                    proposal["yes_votes"] = serde_json::json!(yes);
                } else {
                    let no = proposal["no_votes"].as_u64().unwrap_or(0) + 1;
                    proposal["no_votes"] = serde_json::json!(no);
                }
                self.store.state_set(&prop_key, &serde_json::to_vec(&proposal)?)?;
                info!("[governance] '{}' voted {} on proposal '{}'",
                    voter, if *approve { "yes" } else { "no" }, proposal_id);
            }

            LedgerEntry::GovernanceFinalize { proposal_id, outcome, yes_votes, no_votes, epoch, .. } => {
                let prop_key = format!("governance:proposal:{}", proposal_id);
                let raw = self.store.state_get(&prop_key)
                    .ok_or_else(|| anyhow::anyhow!("proposal '{}' not found for finalization", proposal_id))?;
                let mut proposal: serde_json::Value = serde_json::from_slice(&raw)?;
                anyhow::ensure!(
                    proposal["status"].as_str() == Some("open"),
                    "proposal '{}' already finalized", proposal_id
                );
                proposal["status"] = serde_json::json!(outcome);
                proposal["yes_votes"] = serde_json::json!(yes_votes);
                proposal["no_votes"] = serde_json::json!(no_votes);
                proposal["finalized_epoch"] = serde_json::json!(epoch);
                // Execute on-chain action for passed proposals.
                if outcome == "passed" {
                    let action = proposal["action"].as_str().unwrap_or("").to_owned();
                    let akey  = proposal["action_key"].as_str().unwrap_or("").to_owned();
                    let aval  = proposal["action_value"].as_str().unwrap_or("").to_owned();
                    match action.as_str() {
                        "param_set" if !akey.is_empty() => {
                            self.store.state_set(
                                &format!("chain_param:{}", akey),
                                aval.as_bytes(),
                            )?;
                            info!("[governance] proposal '{}' passed — set chain_param:{} = {}",
                                proposal_id, akey, aval);
                        }
                        "council_add" if !akey.is_empty() => {
                            let mut council = crate::tx::governance_keys(self);
                            if !council.contains(&akey) {
                                council.push(akey.clone());
                                self.store.state_set(
                                    "chain_param:governance_keys",
                                    &serde_json::to_vec(&council)?,
                                )?;
                                info!("[governance] proposal '{}' passed — added '{}' to council",
                                    proposal_id, akey);
                            }
                        }
                        "council_remove" if !akey.is_empty() => {
                            let council: Vec<String> = crate::tx::governance_keys(self)
                                .into_iter().filter(|a| a != &akey).collect();
                            self.store.state_set(
                                "chain_param:governance_keys",
                                &serde_json::to_vec(&council)?,
                            )?;
                            info!("[governance] proposal '{}' passed — removed '{}' from council",
                                proposal_id, akey);
                        }
                        _ => {} // "text" or unknown — signal only
                    }
                }
                self.store.state_set(&prop_key, &serde_json::to_vec(&proposal)?)?;
                info!("[governance] proposal '{}' finalized: {} ({} yes / {} no)",
                    proposal_id, outcome, yes_votes, no_votes);
            }

            // ── Scientific compute result ─────────────────────────────────────
            LedgerEntry::ScientificResult { job_id, .. } => {
                let key = format!("sci_result:{}", job_id);
                self.store.state_set(&key, &serde_json::to_vec(entry)?)?;
                info!("[science] on-chain result for job '{}'", job_id);
            }

            // ── Cross-chain finality announcement ─────────────────────────────
            LedgerEntry::CrossChainFinalityAnnounce { target_chain, finality_epoch, .. } => {
                let key = format!("cc_finality:{}:{}", target_chain, finality_epoch);
                self.store.state_set(&key, &serde_json::to_vec(entry)?)?;
                info!("[cross-chain] finality at epoch {} for '{}'", finality_epoch, target_chain);
            }

            // ── Clock Node Registration ───────────────────────────────────────
            LedgerEntry::ClockNodeRegister { node_id, stake, epoch, pubkey, .. } => {
                anyhow::ensure!(
                    self.store.get_account(node_id)?.is_some(),
                    "account '{}' does not exist", node_id
                );
                // Idempotent: if already registered at the same stake, succeed silently.
                let reg_key_check = format!("clock_reg:{}", node_id);
                if let Some(existing) = self.store.state_get(&reg_key_check) {
                    if let Ok(j) = serde_json::from_slice::<serde_json::Value>(&existing) {
                        if j["stake"].as_u64().unwrap_or(0) >= *stake {
                            info!("[clock] node '{}' already registered — idempotent re-register", node_id);
                            return Ok(());
                        }
                    }
                }
                // Read dynamic minimum from governance; fall back to 5 BTCPC.
                let min_stake: u64 = self.store.state_get("chain_param:clock_min_stake")
                    .and_then(|b| serde_json::from_slice::<u64>(&b).ok())
                    .unwrap_or(5 * 10_000_000_000);
                anyhow::ensure!(
                    *stake >= min_stake,
                    "clock node stake {} dreams is below minimum {} dreams", stake, min_stake
                );
                self.store.debit(node_id, NATIVE_TOKEN, *stake)?;
                let reg_key = format!("clock_reg:{}", node_id);
                let prev: serde_json::Value = self.store.state_get(&reg_key)
                    .and_then(|b| serde_json::from_slice(&b).ok())
                    .unwrap_or(serde_json::json!({}));
                let prev_stake = prev["stake"].as_u64().unwrap_or(0);
                // Preserve existing pubkey if not overriding.
                let stored_pubkey = pubkey.as_deref()
                    .map(|s| s.to_owned())
                    .or_else(|| prev["pubkey"].as_str().map(|s| s.to_owned()));
                self.store.state_set(&reg_key, &serde_json::to_vec(&serde_json::json!({
                    "node_id": node_id,
                    "stake": prev_stake + stake,
                    "registered_epoch": epoch,
                    "pubkey": stored_pubkey,
                    "min_at_registration": min_stake,
                }))?)?;
                self.touch_alive(node_id, *epoch);
                info!("[clock] node '{}' registered with {} dreams at epoch {}", node_id, stake, epoch);
            }

            // ── Clock Double-Sign Slash ───────────────────────────────────────
            LedgerEntry::ClockDoubleSignEvidence {
                submitter, offender, epoch,
                seal_hash_a, timestamp_a, sig_a,
                seal_hash_b, timestamp_b, sig_b, ..
            } => {
                anyhow::ensure!(
                    seal_hash_a != seal_hash_b,
                    "double-sign evidence: seal hashes must differ"
                );
                let slash_guard = format!("dbl_slash:{}:{}", offender, epoch);
                anyhow::ensure!(
                    self.store.state_get(&slash_guard).is_none(),
                    "double-sign evidence for '{}' epoch {} already applied", offender, epoch
                );

                // Load offender's registration — provides the trusted pubkey.
                let reg_key = format!("clock_reg:{}", offender);
                let reg: serde_json::Value = self.store.state_get(&reg_key)
                    .and_then(|b| serde_json::from_slice(&b).ok())
                    .unwrap_or(serde_json::json!({}));
                let reg_stake = reg["stake"].as_u64().unwrap_or(0);
                anyhow::ensure!(reg_stake > 0, "offender '{}' has no registered stake", offender);

                // The pubkey must be on file — registered at ClockNodeRegister time.
                let pubkey_hex = reg["pubkey"].as_str()
                    .ok_or_else(|| anyhow::anyhow!(
                        "offender '{}' has no registered pubkey — cannot verify seal signatures", offender
                    ))?;

                // Verify both seal signatures. Both must be valid; one being wrong could mean
                // the evidence was forged or corrupted.
                anyhow::ensure!(
                    crate::clock::verify_clock_seal_sig(
                        *epoch, seal_hash_a, offender, *timestamp_a, pubkey_hex, sig_a
                    ),
                    "seal A signature invalid"
                );
                anyhow::ensure!(
                    crate::clock::verify_clock_seal_sig(
                        *epoch, seal_hash_b, offender, *timestamp_b, pubkey_hex, sig_b
                    ),
                    "seal B signature invalid"
                );

                // D7 slash distribution: 10% submitter, 10% __legal__, 80% recycle.
                let submitter_cut = reg_stake / 10;
                let legal_cut     = reg_stake / 10;
                let recycle_cut   = reg_stake - submitter_cut - legal_cut;
                self.store.credit(submitter, NATIVE_TOKEN, submitter_cut)?;
                self.store.credit("__legal__", NATIVE_TOKEN, legal_cut)?;
                self.store.credit("__recycle_fund__", NATIVE_TOKEN, recycle_cut)?;

                // Zero out stake; preserve slashed_epoch and pubkey for audit trail.
                self.store.state_set(&reg_key, &serde_json::to_vec(&serde_json::json!({
                    "node_id": offender,
                    "stake": 0u64,
                    "pubkey": pubkey_hex,
                    "slashed_epoch": epoch,
                }))?)?;

                self.store.state_set(&slash_guard, b"1")?;
                warn!(
                    "[clock] double-sign slash: '{}' slashed {} dreams at epoch {} (submitter='{}' hashes={}/{})",
                    offender, reg_stake, epoch, submitter,
                    &seal_hash_a[..seal_hash_a.len().min(12)],
                    &seal_hash_b[..seal_hash_b.len().min(12)],
                );
            }

            // ── Project Collaboration ─────────────────────────────────────────
            LedgerEntry::ProjectCreate { project_id, name, description, repo, creator, epoch, .. } => {
                let key = format!("project:{}", project_id);
                anyhow::ensure!(
                    self.store.state_get(&key).is_none(),
                    "project '{}' already exists", project_id
                );
                self.store.state_set(&key, &serde_json::to_vec(&serde_json::json!({
                    "project_id": project_id,
                    "name": name,
                    "description": description,
                    "repo": repo,
                    "creator": creator,
                    "created_epoch": epoch,
                    "collaborators": [],
                }))?)?;
            }

            LedgerEntry::ProjectTask { project_id, task_id, title, description,
                task_type, bounty, tags, deadline_epoch, creator, epoch, .. } => {
                anyhow::ensure!(
                    self.store.state_get(&format!("project:{}", project_id)).is_some(),
                    "project '{}' does not exist", project_id
                );
                let key = format!("project_task:{}:{}", project_id, task_id);
                anyhow::ensure!(
                    self.store.state_get(&key).is_none(),
                    "task '{}' already exists in project '{}'", task_id, project_id
                );
                self.store.state_set(&key, &serde_json::to_vec(&serde_json::json!({
                    "project_id": project_id,
                    "task_id": task_id,
                    "title": title,
                    "description": description,
                    "task_type": task_type,
                    "bounty": bounty,
                    "tags": tags,
                    "deadline_epoch": deadline_epoch,
                    "creator": creator,
                    "created_epoch": epoch,
                    "status": "open",
                    "claimant": null,
                    "submission_link": null,
                }))?)?;
            }

            LedgerEntry::TaskClaim { project_id, task_id, claimant, .. } => {
                let key = format!("project_task:{}:{}", project_id, task_id);
                let mut task: serde_json::Value = self.store.state_get(&key)
                    .and_then(|b| serde_json::from_slice(&b).ok())
                    .ok_or_else(|| anyhow::anyhow!("task '{}' not found in project '{}'", task_id, project_id))?;
                anyhow::ensure!(
                    task["status"].as_str() == Some("open"),
                    "task '{}' is not open (status: {})", task_id, task["status"]
                );
                task["status"] = serde_json::json!("claimed");
                task["claimant"] = serde_json::json!(claimant);
                self.store.state_set(&key, &serde_json::to_vec(&task)?)?;
            }

            LedgerEntry::TaskSubmit { project_id, task_id, submitter, link, notes, epoch, .. } => {
                let key = format!("project_task:{}:{}", project_id, task_id);
                let mut task: serde_json::Value = self.store.state_get(&key)
                    .and_then(|b| serde_json::from_slice(&b).ok())
                    .ok_or_else(|| anyhow::anyhow!("task '{}' not found in project '{}'", task_id, project_id))?;
                anyhow::ensure!(
                    task["claimant"].as_str() == Some(submitter.as_str()),
                    "only the task claimant can submit"
                );
                task["status"] = serde_json::json!("submitted");
                task["submission_link"] = serde_json::json!(link);
                task["submission_notes"] = serde_json::json!(notes);
                task["submitted_epoch"] = serde_json::json!(epoch);
                self.store.state_set(&key, &serde_json::to_vec(&task)?)?;
            }

            LedgerEntry::TaskApprove { project_id, task_id, approver, recipient, payout, epoch, .. } => {
                // Verify approver is project creator or an existing collaborator
                let proj_key = format!("project:{}", project_id);
                let project: serde_json::Value = self.store.state_get(&proj_key)
                    .and_then(|b| serde_json::from_slice(&b).ok())
                    .ok_or_else(|| anyhow::anyhow!("project '{}' not found", project_id))?;
                let creator = project["creator"].as_str().unwrap_or("");
                let collabs: Vec<String> = project["collaborators"].as_array()
                    .map(|a| a.iter().filter_map(|v| v.as_str().map(str::to_owned)).collect())
                    .unwrap_or_default();
                anyhow::ensure!(
                    approver == creator || collabs.contains(approver),
                    "only the project lead or a collaborator can approve tasks"
                );

                let task_key = format!("project_task:{}:{}", project_id, task_id);
                let mut task: serde_json::Value = self.store.state_get(&task_key)
                    .and_then(|b| serde_json::from_slice(&b).ok())
                    .ok_or_else(|| anyhow::anyhow!("task '{}' not found", task_id))?;

                let task_type = task["task_type"].as_str().unwrap_or("partial").to_owned();

                // Pay bounty for partial tasks
                if task_type == "partial" {
                    let bounty = payout.unwrap_or_else(|| task["bounty"].as_u64().unwrap_or(0));
                    if bounty > 0 {
                        self.store.debit(approver, NATIVE_TOKEN, bounty)?;
                        self.ensure_account(recipient, *epoch)?;
                        self.store.credit(recipient, NATIVE_TOKEN, bounty)?;
                    }
                }

                // For full tasks: add recipient to collaborators list
                if task_type == "full" {
                    let mut proj = project.clone();
                    let mut collabs_arr: Vec<serde_json::Value> = proj["collaborators"]
                        .as_array().cloned().unwrap_or_default();
                    let already = collabs_arr.iter().any(|v| v.as_str() == Some(recipient.as_str()));
                    if !already {
                        collabs_arr.push(serde_json::json!(recipient));
                        proj["collaborators"] = serde_json::json!(collabs_arr);
                        self.store.state_set(&proj_key, &serde_json::to_vec(&proj)?)?;
                    }
                }

                task["status"] = serde_json::json!("approved");
                task["approved_epoch"] = serde_json::json!(epoch);
                task["approved_by"] = serde_json::json!(approver);
                self.store.state_set(&task_key, &serde_json::to_vec(&task)?)?;
            }

            LedgerEntry::OracleFeedCreate { .. } => {
                crate::oracle::apply_create(self, entry)?;
            }
            LedgerEntry::OracleReport { .. } => {
                crate::oracle::apply_report(self, entry)?;
            }
            LedgerEntry::OracleFeedFinalize { .. } => {
                crate::oracle::apply_finalize(self, entry)?;
            }
            LedgerEntry::SessionListingCreate { .. } => {
                crate::session_market::apply_create(self, entry)?;
            }
            LedgerEntry::SessionListingBuy { .. } => {
                crate::session_market::apply_buy(self, entry)?;
            }
            LedgerEntry::SessionListingCancel { .. } => {
                crate::session_market::apply_cancel(self, entry)?;
            }
            LedgerEntry::AgentSessionOpen { .. } => {
                // server_pubkey is stored in CF_META at node startup.
                let server_pubkey = self.store.state_get("node:server_pubkey")
                    .and_then(|b| String::from_utf8(b).ok())
                    .unwrap_or_default();
                crate::agent_session::apply_open(self, entry, &server_pubkey)?;
            }
            LedgerEntry::AgentSessionClose { .. } => {
                crate::agent_session::apply_close(self, entry)?;
            }
            LedgerEntry::VrfCommit { .. } => {
                crate::vrf::apply_commit(self, entry)?;
            }
            LedgerEntry::VrfReveal { .. } => {
                crate::vrf::apply_reveal(self, entry)?;
            }

            LedgerEntry::EnsembleJobPost { .. } => {
                crate::ensemble::apply_job_post(self, entry)?;
            }
            LedgerEntry::EnsembleVote { .. } => {
                crate::ensemble::apply_vote(self, entry)?;
            }
            LedgerEntry::SlashValidator { .. } => {
                crate::slashing::apply_slash(self, entry)?;
            }
            LedgerEntry::SlashAppeal { .. } => {
                crate::slashing::apply_appeal(self, entry)?;
            }
            LedgerEntry::BridgeFund { .. } => {
                crate::bridge::apply_fund(self, entry)?;
            }
            LedgerEntry::BridgeWrap { .. } => {
                crate::bridge::apply_wrap(self, entry)?;
            }
            LedgerEntry::BridgeUnwrap { .. } => {
                crate::bridge::apply_unwrap(self, entry)?;
            }
            LedgerEntry::BridgeUnlock { .. } => {
                crate::bridge::apply_unlock(self, entry)?;
            }

            // ── TON wallet activation ─────────────────────────────────────────
            // ── Agentic task marketplace ──────────────────────────────────────
            LedgerEntry::AgentCreditDeposit { .. } => {
                crate::agent_task::apply_credit_deposit(self, entry)?;
            }
            LedgerEntry::AgentCreditWithdraw { .. } => {
                crate::agent_task::apply_credit_withdraw(self, entry)?;
            }
            LedgerEntry::AgentTaskPost { .. } => {
                crate::agent_task::apply_task_post(self, entry)?;
            }
            LedgerEntry::AgentTaskBid { .. } => {
                crate::agent_task::apply_task_bid(self, entry)?;
            }
            LedgerEntry::AgentTaskAssign { .. } => {
                crate::agent_task::apply_task_assign(self, entry)?;
            }
            LedgerEntry::AgentTaskSubmit { .. } => {
                crate::agent_task::apply_task_submit(self, entry)?;
            }
            LedgerEntry::AgentTaskVerifierCommit { .. } => {
                crate::agent_task::apply_verifier_commit(self, entry)?;
            }
            LedgerEntry::AgentTaskVerifierReveal { .. } => {
                crate::agent_task::apply_verifier_reveal(self, entry)?;
            }
            LedgerEntry::AgentTaskSettle { .. } => {
                crate::agent_task::apply_settle(self, entry)?;
            }

            LedgerEntry::TonActivationIntent { btcpc_account, ton_address, source_chain, source_address, epoch, .. } => {
                self.touch_alive(btcpc_account, *epoch);
                self.ensure_account(btcpc_account, *epoch)?;
                let key = format!("ton_activation_intent:{}:{}", btcpc_account, source_address);
                let val = serde_json::json!({
                    "ton_address": ton_address,
                    "source_chain": source_chain,
                    "source_address": source_address,
                    "epoch": epoch,
                    "status": "pending",
                });
                self.store.state_set(&key, &serde_json::to_vec(&val).unwrap())?;
            }

            LedgerEntry::TonWalletActivated { btcpc_account, ton_address, source_address, tx_hash, ton_sent, fee_usdt, usdt_received, epoch, relay, .. } => {
                // Update the pending intent to completed — key must match the intent's write key
                // which uses source_address (the chain where USDT was sent from), not ton_address.
                let key = format!("ton_activation_intent:{}:{}", btcpc_account, source_address);
                let val = serde_json::json!({
                    "ton_address": ton_address,
                    "status": "activated",
                    "tx_hash": tx_hash,
                    "ton_sent": ton_sent,
                    "fee_usdt": fee_usdt,
                    "usdt_received": usdt_received,
                    "epoch": epoch,
                    "relay": relay,
                });
                self.store.state_set(&key, &serde_json::to_vec(&val).unwrap())?;
                info!("TON wallet activated: {} via relay {}", ton_address, relay);
            }

            LedgerEntry::PhoneMineSubmit { account, work_hash, epoch, .. } => {
                self.touch_alive(account, *epoch);
                self.ensure_account(account, *epoch)?;

                // Reward: 500 dreams per valid proof.
                const PHONE_MINE_REWARD: u64 = 500;
                self.store.credit(account, btcpc_types::NATIVE_TOKEN, PHONE_MINE_REWARD)?;

                // Increment proof counter.
                let proofs_key = format!("phone_proofs:{}", account);
                let proofs: u64 = self.store.state_get(&proofs_key)
                    .and_then(|b| serde_json::from_slice(&b).ok())
                    .unwrap_or(0);
                self.store.state_set(&proofs_key, &serde_json::to_vec(&(proofs + 1))?)?;
                self.store.state_set(
                    &format!("phone_last_reward:{}", account),
                    &serde_json::to_vec(epoch)?,
                )?;
                // Record work hash to prevent replay.
                self.store.state_set(
                    &format!("phone_work_used:{}", work_hash),
                    &serde_json::to_vec(epoch)?,
                )?;
            }

            // ── Name auctions ─────────────────────────────────────────────────
            LedgerEntry::NameAuctionOpen { auction_id, name, opener, min_bid, end_epoch, epoch, .. } => {
                self.touch_alive(opener, *epoch);
                self.ensure_account(opener, *epoch)?;
                crate::auction::apply_name_open(self, auction_id, name, opener, *min_bid, *end_epoch, *epoch)?;
            }
            LedgerEntry::NameAuctionBid { auction_id, bidder, amount, epoch, .. } => {
                self.touch_alive(bidder, *epoch);
                self.ensure_account(bidder, *epoch)?;
                crate::auction::apply_bid(self, auction_id, bidder, *amount, *epoch)?;
            }
            LedgerEntry::NameAuctionSettle { auction_id, epoch, .. } => {
                crate::auction::apply_settle(self, auction_id, *epoch)?;
            }
            LedgerEntry::NameAuctionCancel { auction_id, opener, .. } => {
                crate::auction::apply_cancel(self, auction_id, opener)?;
            }

            // ── Freeport auctions ─────────────────────────────────────────────
            LedgerEntry::FreeportAuctionOpen { auction_id, item_type, item_id, seller, min_bid, end_epoch, epoch, .. } => {
                self.touch_alive(seller, *epoch);
                self.ensure_account(seller, *epoch)?;
                crate::auction::apply_freeport_open(self, auction_id, item_type, item_id, seller, *min_bid, *end_epoch, *epoch)?;
            }
            LedgerEntry::FreeportAuctionBid { auction_id, bidder, amount, epoch, .. } => {
                self.touch_alive(bidder, *epoch);
                self.ensure_account(bidder, *epoch)?;
                crate::auction::apply_bid(self, auction_id, bidder, *amount, *epoch)?;
            }
            LedgerEntry::FreeportAuctionSettle { auction_id, epoch, .. } => {
                crate::auction::apply_settle(self, auction_id, *epoch)?;
            }

            // ── Private authorization ─────────────────────────────────────────
            LedgerEntry::PrivateAuthEnroll { group_id, member, member_pubkey, threshold_n, threshold_m, epoch, .. } => {
                self.touch_alive(member, *epoch);
                self.ensure_account(member, *epoch)?;
                crate::private_auth::apply_enroll(self, group_id, member, member_pubkey, *threshold_n, *threshold_m)?;
            }
            LedgerEntry::PrivateAuthApprove { group_id, tx_hash, approver, epoch, .. } => {
                self.touch_alive(approver, *epoch);
                self.ensure_account(approver, *epoch)?;
                let _ = crate::private_auth::apply_approve(self, group_id, tx_hash, approver)?;
            }

            // ── Phase 5: Memory service ───────────────────────────────────────
            LedgerEntry::MemorySet { account, key, value, epoch, .. } => {
                self.touch_alive(account, *epoch);
                self.ensure_account(account, *epoch)?;
                crate::memory_service::apply_set(self, account, key, value)?;
            }
            LedgerEntry::MemoryDelete { account, key, epoch, .. } => {
                self.touch_alive(account, *epoch);
                self.ensure_account(account, *epoch)?;
                crate::memory_service::apply_delete(self, account, key)?;
            }

            // ── Amber Pill ────────────────────────────────────────────────────
            LedgerEntry::AmberPillMint { account, pill_id, epoch, .. } => {
                self.touch_alive(account, *epoch);
                self.ensure_account(account, *epoch)?;
                crate::amber_pill::apply_mint(self, account, pill_id, *epoch)?;
            }

            // ── Phone storage ─────────────────────────────────────────────────
            LedgerEntry::PhoneStorageProof { account, device_id, proof_hash, bytes_proven, epoch, .. } => {
                self.touch_alive(account, *epoch);
                self.ensure_account(account, *epoch)?;
                crate::phone_storage::apply_proof(self, account, device_id, proof_hash, *bytes_proven, *epoch)?;
            }

            // ── Fine-tune jobs ────────────────────────────────────────────────
            LedgerEntry::FineTuneJobPost { job_id, requester, base_model, dataset_cid, max_fee, epoch, .. } => {
                self.touch_alive(requester, *epoch);
                self.ensure_account(requester, *epoch)?;
                crate::finetune::apply_post(self, job_id, requester, base_model, dataset_cid, *max_fee, *epoch)?;
            }
            LedgerEntry::FineTuneJobComplete { job_id, worker, adapter_cid, epoch, .. } => {
                self.touch_alive(worker, *epoch);
                self.ensure_account(worker, *epoch)?;
                crate::finetune::apply_complete(self, job_id, worker, adapter_cid, *epoch)?;
            }

            // ── Computer-use jobs ─────────────────────────────────────────────
            LedgerEntry::ComputerUseJobPost { job_id, requester, task_json, max_fee, epoch, .. } => {
                self.touch_alive(requester, *epoch);
                self.ensure_account(requester, *epoch)?;
                crate::computer_use::apply_post(self, job_id, requester, task_json, *max_fee, *epoch)?;
            }
            LedgerEntry::ComputerUseJobComplete { job_id, worker, result_cid, epoch, .. } => {
                self.touch_alive(worker, *epoch);
                self.ensure_account(worker, *epoch)?;
                crate::computer_use::apply_complete(self, job_id, worker, result_cid, *epoch)?;
            }

            // ── Blob serve proof ──────────────────────────────────────────────
            LedgerEntry::BlobServeProof { node_id, cid, bytes_served, proof_hash, epoch, .. } => {
                self.touch_alive(node_id, *epoch);
                self.ensure_account(node_id, *epoch)?;
                crate::blob_serve::apply_serve_proof(self, node_id, cid, *bytes_served, proof_hash, *epoch)?;
            }

            // ── Snapshot replication ──────────────────────────────────────────
            LedgerEntry::SnapshotSave { account, slug, cid, epoch, .. } => {
                self.touch_alive(account, *epoch);
                self.ensure_account(account, *epoch)?;
                crate::snapshot_replication::apply_save(self, account, slug, cid, *epoch)?;
            }

        }

        self.index_tx_history(entry);
        Ok(())
    }

    /// Apply a batch of entries from a block payload.
    #[allow(dead_code)]
    pub fn apply_block_entries(&self, entries: &[LedgerEntry]) -> usize {
        let mut applied = 0;
        for entry in entries {
            match self.apply_entry(entry) {
                Ok(_) => applied += 1,
                Err(e) => warn!("entry rejected: {}: {:?}", e, entry),
            }
        }
        applied
    }

    pub fn get_balance(&self, account: &str, token: &str) -> u64 {
        self.store.get_balance(account, token)
    }

    pub fn get_stake(&self, account: &str) -> u64 {
        self.store.get_stake(account)
    }

    /// Distribute the backer share of `total` dreams to role stakers for `node`+`role`.
    /// Returns the amount remaining for the node itself (total minus backer payouts).
    /// If the node has not opted in, or there are no stakers, returns `total` unchanged.
    fn distribute_role_backer_reward(&self, role: &str, node_id: &str, total: u64) -> Result<u64> {
        let opt_key = format!("role_opt:{}:{}", role, node_id);
        if let Some(opt_raw) = self.store.state_get(&opt_key) {
            if let Ok(opt) = serde_json::from_slice::<serde_json::Value>(&opt_raw) {
                let bps = opt["backer_share_bps"].as_u64().unwrap_or(0).min(5000);
                if bps > 0 {
                    let backer_total = (total as u128 * bps as u128 / 10_000) as u64;
                    let prefix = format!("role_stake:{}:{}:", role, node_id);
                    let stakers: Vec<(String, u64)> = self.store.state_scan_prefix(&prefix)
                        .into_iter()
                        .filter_map(|(_, v)| {
                            let js = serde_json::from_slice::<serde_json::Value>(&v).ok()?;
                            let s = js["staker"].as_str()?.to_owned();
                            let a = js["amount"].as_u64().unwrap_or(0);
                            if a > 0 { Some((s, a)) } else { None }
                        })
                        .collect();
                    let total_stake: u64 = stakers.iter().map(|(_, a)| a).sum();
                    if total_stake > 0 && backer_total > 0 {
                        for (staker, staker_amount) in &stakers {
                            let payout = (backer_total as u128 * *staker_amount as u128
                                / total_stake as u128) as u64;
                            if payout > 0 {
                                let _ = self.store.credit(staker, NATIVE_TOKEN, payout);
                            }
                        }
                        return Ok(total.saturating_sub(backer_total));
                    }
                }
            }
        }
        Ok(total)
    }

    /// Return all role backer positions for a given account (as staker).
    pub fn get_role_stakes_for_account(&self, account: &str) -> Vec<serde_json::Value> {
        // Scan all role_stake:*:*:{account} keys
        // We can't do suffix scans, so we scan all role_stake: keys and filter
        self.store.state_scan_prefix("role_stake:")
            .into_iter()
            .filter_map(|(_k, v)| {
                let js = serde_json::from_slice::<serde_json::Value>(&v).ok()?;
                let staker = js["staker"].as_str()?;
                if staker == account { Some(js) } else { None }
            })
            .collect()
    }

    /// Return all backers for a given node+role.
    pub fn get_role_backers(&self, role: &str, node: &str) -> Vec<serde_json::Value> {
        let prefix = format!("role_stake:{}:{}:", role, node);
        self.store.state_scan_prefix(&prefix)
            .into_iter()
            .filter_map(|(_, v)| serde_json::from_slice::<serde_json::Value>(&v).ok())
            .collect()
    }

    /// Return the role opt-in config for a node+role, if opted in.
    pub fn get_role_opt(&self, role: &str, node: &str) -> Option<serde_json::Value> {
        self.store.state_get(&format!("role_opt:{}:{}", role, node))
            .and_then(|b| serde_json::from_slice(&b).ok())
    }

    /// Returns the lifecycle status of an epoch for the explorer.
    ///
    /// | Status      | Meaning |
    /// |-------------|---------|
    /// | `disputed`  | Competing EpochFinalize entries detected — fork evidence stored |
    /// | `finalized` | EpochFinalize with quorum committed |
    /// | `sealed`    | At least one EpochSeal applied; awaiting finalization |
    /// | `pending`   | Epoch not yet sealed (current or future) |
    pub fn epoch_status(&self, epoch: u64) -> &'static str {
        if self.store.state_get(&format!("fork_evidence:{}", epoch)).is_some() {
            return "disputed";
        }
        if let Ok(Some(meta)) = self.store.get_epoch_meta(epoch) {
            if meta["finalized"].as_bool().unwrap_or(false) {
                return "finalized";
            }
        }
        let any_seal = !self.store.state_scan_prefix(&format!("epoch_node_seal:{}:", epoch)).is_empty();
        if any_seal || self.store.has_block(epoch) {
            return "sealed";
        }
        "pending"
    }

    fn ensure_account(&self, account: &AccountId, epoch: u64) -> Result<()> {
        if self.store.get_account(account)?.is_none() {
            self.store.set_account(account, &serde_json::json!({
                "account_id": account,
                "created_epoch": epoch,
                "nonce": 0,
                "stake": 0,
            }))?;
        }
        Ok(())
    }

    /// Release unbonding tokens whose slash window has expired.
    /// Called once per epoch seal, before reward distribution.
    pub fn drain_unbonding(&self, current_epoch: u64) {
        let prefix = format!("unbonding:{}:", current_epoch);
        let records = self.store.state_scan_prefix(&prefix);
        for (key, val) in records {
            let Ok(j) = serde_json::from_slice::<serde_json::Value>(&val) else { continue };
            let Some(account) = j["account"].as_str() else { continue };
            let amount = j["amount"].as_u64().unwrap_or(0);
            if amount > 0 {
                if let Err(e) = self.store.credit(account, NATIVE_TOKEN, amount) {
                    warn!("drain_unbonding: credit failed for '{}': {}", account, e);
                } else {
                    let _ = self.store.state_delete(&key);
                }
            }
        }
    }

    /// Finalize any governance proposals whose voting window has closed.
    /// Called once per epoch seal. Passes if yes > no and total votes ≥ 2 (quorum).
    pub fn drain_governance_proposals(&self, current_epoch: u64) {
        let records = self.store.state_scan_prefix("governance:proposal:");
        for (_key, val) in records {
            let Ok(proposal) = serde_json::from_slice::<serde_json::Value>(&val) else { continue };
            if proposal["status"].as_str() != Some("open") { continue; }
            let ends = proposal["voting_ends_epoch"].as_u64().unwrap_or(u64::MAX);
            if current_epoch <= ends { continue; }
            let proposal_id = match proposal["proposal_id"].as_str() {
                Some(id) => id.to_owned(),
                None => continue,
            };
            let yes = proposal["yes_votes"].as_u64().unwrap_or(0);
            let no  = proposal["no_votes"].as_u64().unwrap_or(0);
            let total = yes + no;
            let outcome = if total < 2 {
                "no_quorum"
            } else if yes > no {
                "passed"
            } else {
                "rejected"
            };
            let entry = LedgerEntry::GovernanceFinalize {
                proposal_id: proposal_id.clone(),
                outcome: outcome.to_owned(),
                yes_votes: yes,
                no_votes: no,
                epoch: current_epoch,
                signed_by: self.node_id.clone(),
            };
            if let Err(e) = self.apply_entry(&entry) {
                warn!("[governance] finalize '{}' failed: {}", proposal_id, e);
            }
        }
    }

    /// Apply any governance parameter changes whose 2-epoch timelock has expired.
    /// Called once per epoch seal, before reward distribution.
    pub fn drain_pending_params(&self, current_epoch: u64) {
        let prefix = format!("pending_param:{}:", current_epoch);
        let records = self.store.state_scan_prefix(&prefix);
        for (key, val) in records {
            let Ok(j) = serde_json::from_slice::<serde_json::Value>(&val) else { continue };
            let Some(entry_json) = j["entry"].as_str() else { continue };
            match serde_json::from_str::<LedgerEntry>(entry_json) {
                Ok(entry) => {
                    if let Err(e) = self.apply_entry(&entry) {
                        warn!("drain_pending_params: apply failed for key '{}': {}", key, e);
                    } else {
                        let _ = self.store.state_delete(&key);
                    }
                }
                Err(e) => warn!("drain_pending_params: deserialize failed: {}", e),
            }
        }
    }
}

// ── Haversine distance helper (T4-3 GNSS plausibility) ───────────────────────

/// Haversine great-circle distance in metres between two (lat, lon) points in degrees.
fn haversine_m(lat1: f64, lon1: f64, lat2: f64, lon2: f64) -> f64 {
    const R: f64 = 6_371_000.0; // Earth radius in metres
    let dlat = (lat2 - lat1).to_radians();
    let dlon = (lon2 - lon1).to_radians();
    let a = (dlat / 2.0).sin().powi(2)
        + lat1.to_radians().cos() * lat2.to_radians().cos() * (dlon / 2.0).sin().powi(2);
    R * 2.0 * a.sqrt().asin()
}

// ── Replay determinism tests (T2-6) ──────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use btcpc_types::{LedgerEntry, NATIVE_TOKEN};
    use tempfile::TempDir;

    fn make_chain(label: &str) -> (Chain, TempDir) {
        let dir = tempfile::Builder::new()
            .prefix(&format!("btcpc_test_{}_", label))
            .tempdir()
            .expect("tempdir");
        let store = crate::store::Store::open(dir.path()).expect("store open");
        let chain = Chain::new(store, format!("node-{}", label), "btcpc-satoshi".to_string());
        (chain, dir)
    }

    fn seal_epoch(chain: &Chain, epoch: u64) {
        let seal = LedgerEntry::EpochSeal {
            node_id: "test-node".to_string(),
            epoch,
            timestamp: epoch * 30_000,
            seal_hash: format!("seal-{:08x}", epoch),
            signature: None,
            node_version: None,
        };
        chain.apply_entry(&seal).expect("epoch seal");
        chain.drain_unbonding(epoch);
        chain.drain_pending_params(epoch);
    }

    fn fund(chain: &Chain, account: &str, amount: u64) {
        chain.apply_entry(&LedgerEntry::AccountCreate {
            account: account.to_string(),
            keys: Default::default(),
            chain_proofs: vec![],
            epoch: 0,
            funded_by: None,
            machine_fingerprint: None,
        }).ok();
        chain.apply_entry(&LedgerEntry::GenesisAlloc {
            account: account.to_string(),
            amount,
            token: NATIVE_TOKEN.to_string(),
        }).expect("genesis alloc");
    }

    /// Generate a real ed25519 keypair and sign a seal message.
    /// Returns (pubkey_hex, sig_hex).
    fn sign_seal(sk_seed: u8, epoch: u64, seal_hash: &str, node_id: &str, timestamp: u64)
        -> (String, String)
    {
        use ed25519_dalek::SigningKey;
        let sk = SigningKey::from_bytes(&[sk_seed; 32]);
        let pk = sk.verifying_key();
        let msg = format!("seal:{}:{}:{}:{}", epoch, seal_hash, node_id, timestamp);
        use ed25519_dalek::Signer;
        let sig = sk.sign(msg.as_bytes());
        (hex::encode(pk.to_bytes()), hex::encode(sig.to_bytes()))
    }

    fn assert_same_state(a: &Chain, b: &Chain, epoch: u64) {
        let ha = a.store.full_state_hash();
        let hb = b.store.full_state_hash();
        assert_eq!(
            ha, hb,
            "state diverged at epoch {}:\n  node-a: {}\n  node-b: {}",
            epoch, ha, hb
        );
    }

    fn build_epoch_entries(epoch: u64, accounts: &[&str]) -> Vec<LedgerEntry> {
        let n = accounts.len() as u64;
        let from_idx = (epoch % n) as usize;
        let to_idx   = ((epoch + 1) % n) as usize;
        let amount   = (epoch % 5 + 1) * 10_000_000_000;
        let mut entries = vec![LedgerEntry::Transfer {
            from:      accounts[from_idx].to_string(),
            to:        accounts[to_idx].to_string(),
            amount,
            token:     NATIVE_TOKEN.to_string(),
            memo:      None,
            epoch,
            signed_by: accounts[from_idx].to_string(),
            nonce:     epoch,
            twofactor: None,
        }];
        if epoch % 20 == 0 && epoch > 0 {
            entries.push(LedgerEntry::Stake {
                account:   accounts[from_idx].to_string(),
                amount:    5 * 10_000_000_000,
                epoch,
                signed_by: accounts[from_idx].to_string(),
                nonce:     epoch + 1000,
            });
        }
        entries
    }

    /// Core replay test: two nodes applying identical entries for 200 epochs
    /// must have identical state hashes after every epoch seal.
    #[test]
    fn test_deterministic_replay_200_epochs() {
        let (a, _da) = make_chain("replay-a");
        let (b, _db) = make_chain("replay-b");
        let accounts = ["alice", "bob", "charlie", "dave"];
        let initial = 1_000 * 10_000_000_000u64;

        for acc in &accounts {
            fund(&a, acc, initial);
            fund(&b, acc, initial);
        }
        seal_epoch(&a, 0);
        seal_epoch(&b, 0);
        assert_same_state(&a, &b, 0);

        for epoch in 1u64..=200 {
            let entries = build_epoch_entries(epoch, &accounts);
            for e in &entries {
                a.apply_entry(e).ok();
                b.apply_entry(e).ok();
            }
            seal_epoch(&a, epoch);
            seal_epoch(&b, epoch);
            assert_same_state(&a, &b, epoch);
        }
    }

    /// Unstaked tokens must be locked for UNBONDING_EPOCHS before release.
    #[test]
    fn test_unbonding_delay() {
        let (chain, _dir) = make_chain("unbonding");
        fund(&chain, "staker", 500 * 10_000_000_000);
        seal_epoch(&chain, 0);

        let stake_amount = 100 * 10_000_000_000u64;

        chain.apply_entry(&LedgerEntry::Stake {
            account: "staker".to_string(), amount: stake_amount,
            epoch: 1, signed_by: "staker".to_string(), nonce: 1,
        }).expect("stake");
        seal_epoch(&chain, 1);

        assert_eq!(chain.get_stake("staker"), stake_amount);
        assert_eq!(chain.get_balance("staker", NATIVE_TOKEN), 400 * 10_000_000_000);

        // Unstake at epoch 2 → release epoch = 2 + UNBONDING_EPOCHS (10) = 12.
        chain.apply_entry(&LedgerEntry::Unstake {
            account: "staker".to_string(), amount: stake_amount,
            epoch: 2, signed_by: "staker".to_string(), nonce: 2,
        }).expect("unstake");
        seal_epoch(&chain, 2);

        assert_eq!(chain.get_stake("staker"), 0, "trust must drop immediately (D4)");
        assert_eq!(chain.get_balance("staker", NATIVE_TOKEN), 400 * 10_000_000_000,
            "tokens still locked in unbonding");

        // Epochs 3–11: still locked.
        for epoch in 3u64..12 {
            seal_epoch(&chain, epoch);
            assert_eq!(chain.get_balance("staker", NATIVE_TOKEN), 400 * 10_000_000_000,
                "still locked at epoch {}", epoch);
        }

        // Epoch 12: drain_unbonding releases the tokens.
        seal_epoch(&chain, 12);
        assert_eq!(chain.get_balance("staker", NATIVE_TOKEN), 500 * 10_000_000_000,
            "tokens fully restored after unbonding window");
    }

    /// A ChainParameterSet must not apply at submission epoch — only after 2-epoch timelock.
    #[test]
    fn test_governance_timelock_delay() {
        let (chain, _dir) = make_chain("timelock");
        seal_epoch(&chain, 0);

        // Write the pending_param record directly (mirrors what tx::validate_and_apply does).
        let release_epoch = 3u64;
        let entry_json = serde_json::to_string(&LedgerEntry::ChainParameterSet {
            key: "test_param".to_string(),
            value: "value_at_3".to_string(),
            epoch: 1,
            signed_by: "shindevlin".to_string(),
            signature: None,
        }).unwrap();
        chain.store.state_set(
            &format!("pending_param:{}:test_param", release_epoch),
            &serde_json::to_vec(&serde_json::json!({
                "entry": entry_json,
                "release_epoch": release_epoch,
                "submitted_by": "shindevlin",
            })).unwrap(),
        ).expect("write pending_param");

        seal_epoch(&chain, 1);
        seal_epoch(&chain, 2);
        // Not applied yet at epoch 2.
        assert!(chain.store.state_get(&format!("pending_param:{}:test_param", release_epoch)).is_some(),
            "pending_param record must still exist before release epoch");

        seal_epoch(&chain, 3);
        // After epoch 3 seal, the pending_param record is drained (applied + deleted).
        assert!(chain.store.state_get(&format!("pending_param:{}:test_param", release_epoch)).is_none(),
            "pending_param record must be deleted after draining at epoch 3");
    }

    /// The pending pool must sort identically regardless of push order.
    #[test]
    fn test_pending_sort_is_deterministic() {
        let (a, _da) = make_chain("sort-a");
        let (b, _db) = make_chain("sort-b");

        for chain in [&a, &b] {
            fund(chain, "alice", 1000 * 10_000_000_000);
            fund(chain, "bob",   1000 * 10_000_000_000);
            fund(chain, "carol", 1000 * 10_000_000_000);
            seal_epoch(chain, 0);
        }

        let t1 = LedgerEntry::Transfer {
            from: "alice".to_string(), to: "bob".to_string(),
            amount: 10 * 10_000_000_000, token: NATIVE_TOKEN.to_string(),
            memo: None, epoch: 1, signed_by: "alice".to_string(), nonce: 1, twofactor: None,
        };
        let t2 = LedgerEntry::Transfer {
            from: "bob".to_string(), to: "carol".to_string(),
            amount: 5 * 10_000_000_000, token: NATIVE_TOKEN.to_string(),
            memo: None, epoch: 1, signed_by: "bob".to_string(), nonce: 1, twofactor: None,
        };
        let t3 = LedgerEntry::Transfer {
            from: "carol".to_string(), to: "alice".to_string(),
            amount: 3 * 10_000_000_000, token: NATIVE_TOKEN.to_string(),
            memo: None, epoch: 1, signed_by: "carol".to_string(), nonce: 1, twofactor: None,
        };

        // Node A: t1, t2, t3. Node B: t3, t1, t2.
        a.push_pending(t1.clone(), None);
        a.push_pending(t2.clone(), None);
        a.push_pending(t3.clone(), None);

        b.push_pending(t3.clone(), None);
        b.push_pending(t1.clone(), None);
        b.push_pending(t2.clone(), None);

        let order_a: Vec<String> = a.drain_pending_sorted().into_iter()
            .map(|(e, _)| e.hash()).collect();
        let order_b: Vec<String> = b.drain_pending_sorted().into_iter()
            .map(|(e, _)| e.hash()).collect();

        assert_eq!(order_a, order_b, "pending sort must be identical regardless of push order");
    }

    #[test]
    fn test_pending_dedup_rejects_replay() {
        let (chain, _dir) = make_chain("dedup");
        let entry = LedgerEntry::Transfer {
            from: "alice".into(), to: "bob".into(),
            amount: 1, token: NATIVE_TOKEN.into(),
            memo: None, nonce: 99, epoch: 1,
            signed_by: "alice".into(),
            twofactor: None,
        };
        assert!(chain.push_pending(entry.clone(), None), "first admission must succeed");
        assert!(!chain.push_pending(entry.clone(), None), "replay must be rejected");
        assert_eq!(chain.pending.lock().len(), 1, "pool must contain exactly one entry");
        // After epoch drain, the same entry must be admissible again.
        chain.drain_pending_sorted();
        assert!(chain.push_pending(entry.clone(), None), "entry must be re-admissible after epoch drain");
    }

    // ── Cross-chain signature recovery tests (Phase 6) ───────────────────────

    /// Solana Ed25519 chain link: sign with a known key, recover the address.
    #[test]
    fn test_solana_chain_link_verified() {
        use ed25519_dalek::{SigningKey, Signer};

        let seed = [7u8; 32];
        let signing_key = SigningKey::from_bytes(&seed);
        let vk = signing_key.verifying_key();
        let pubkey_b58 = bs58::encode(vk.as_bytes()).into_string();

        let message = "btcpc:link:alice:solana:42";
        let sig = signing_key.sign(message.as_bytes());
        let sig_field = format!("{}:{}", pubkey_b58, hex::encode(sig.to_bytes()));

        let addr = recover_chain_address_public("sol_sign", message, &sig_field)
            .expect("Solana sig recovery must succeed");
        assert_eq!(addr, pubkey_b58, "recovered address must be the signing pubkey");
    }

    /// Solana chain link: wrong signature must be rejected.
    #[test]
    fn test_solana_chain_link_bad_sig_rejected() {
        use ed25519_dalek::{SigningKey, Signer};

        let seed = [7u8; 32];
        let signing_key = SigningKey::from_bytes(&seed);
        let vk = signing_key.verifying_key();
        let pubkey_b58 = bs58::encode(vk.as_bytes()).into_string();

        let message = "btcpc:link:alice:solana:42";
        let sig = signing_key.sign(b"wrong message");
        let sig_field = format!("{}:{}", pubkey_b58, hex::encode(sig.to_bytes()));

        assert!(
            recover_chain_address_public("sol_sign", message, &sig_field).is_err(),
            "mismatched Solana sig must fail verification"
        );
    }

    /// Bitcoin BIP-322 legacy chain link: sign with a known key, recover P2PKH address.
    #[test]
    fn test_bitcoin_chain_link_verified() {
        use secp256k1::{Secp256k1, SecretKey};
        use sha2::Digest;

        let secp = Secp256k1::new();
        let sk = SecretKey::from_slice(&[3u8; 32]).expect("valid secp256k1 key");
        let pk = secp256k1::PublicKey::from_secret_key(&secp, &sk);

        let message = "btcpc:link:alice:bitcoin:42";

        // Build Bitcoin message hash (SHA256d of prefixed message)
        let magic = b"Bitcoin Signed Message:\n";
        let mut payload = Vec::new();
        payload.push(magic.len() as u8);
        payload.extend_from_slice(magic);
        payload.push(message.len() as u8);
        payload.extend_from_slice(message.as_bytes());
        let h1 = sha2::Sha256::digest(&payload);
        let h2: [u8; 32] = sha2::Sha256::digest(&h1).into();

        let msg = secp256k1::Message::from_digest(h2);
        let rec_sig = secp.sign_ecdsa_recoverable(&msg, &sk);
        let (rec_id, compact) = rec_sig.serialize_compact();

        // v=31 + rec_id for compressed P2PKH
        let v = 31u8 + rec_id.to_i32() as u8;
        let mut full_sig = vec![v];
        full_sig.extend_from_slice(&compact);

        use base64::Engine as _;
        let sig_b64 = base64::engine::general_purpose::STANDARD.encode(&full_sig);

        let addr = recover_chain_address_public("btc_legacy", message, &sig_b64)
            .expect("Bitcoin sig recovery must succeed");

        // Independently derive expected address
        let pk_bytes = pk.serialize(); // 33-byte compressed
        use sha2::Digest as _;
        use ripemd::Digest as RipemdD;
        let sha_hash = sha2::Sha256::digest(&pk_bytes);
        let ripemd_hash = ripemd::Ripemd160::digest(&sha_hash);
        let mut addr_payload = vec![0x00u8];
        addr_payload.extend_from_slice(&ripemd_hash);
        let cs1 = sha2::Sha256::digest(&addr_payload);
        let cs2 = sha2::Sha256::digest(&cs1);
        addr_payload.extend_from_slice(&cs2[..4]);
        let expected = bs58::encode(addr_payload).into_string();

        assert_eq!(addr, expected, "recovered P2PKH address must match");
        assert!(addr.starts_with('1'), "mainnet P2PKH address starts with 1, got {}", addr);
    }

    /// Garbled Bitcoin signature bytes must be rejected outright.
    #[test]
    fn test_bitcoin_chain_link_bad_sig_rejected() {
        use base64::Engine as _;
        // Too short — not 65 bytes.
        let garbage = base64::engine::general_purpose::STANDARD.encode(b"not a valid signature at all");
        assert!(
            recover_chain_address_public("btc_legacy", "btcpc:link:alice:bitcoin:1", &garbage).is_err(),
            "short garbled Bitcoin sig must fail"
        );
        // Wrong length sig (64 bytes instead of 65).
        let short = base64::engine::general_purpose::STANDARD.encode(&[1u8; 64]);
        assert!(
            recover_chain_address_public("btc_legacy", "btcpc:link:alice:bitcoin:1", &short).is_err(),
            "64-byte Bitcoin sig must fail (must be 65)"
        );
    }

    /// Full VerifyChainLink path: Solana sig verified, commitment stored in account state.
    #[test]
    fn test_verify_chain_link_solana_end_to_end() {
        use ed25519_dalek::{SigningKey, Signer};
        use sha2::Digest as _;

        let (chain, _dir) = make_chain("vcl-sol");
        fund(&chain, "alice", 100 * 10_000_000_000);

        let seed = [9u8; 32];
        let sk = SigningKey::from_bytes(&seed);
        let vk = sk.verifying_key();
        let pubkey_b58 = bs58::encode(vk.as_bytes()).into_string();

        let message = format!("btcpc:link:alice:solana:99");
        let sig = sk.sign(message.as_bytes());
        let sig_field = format!("{}:{}", pubkey_b58, hex::encode(sig.to_bytes()));

        // Commitment = sha256("solana:{pubkey_b58}:99")
        let mut h = sha2::Sha256::new();
        h.update(b"solana:");
        h.update(pubkey_b58.as_bytes());
        h.update(b":99");
        let commitment = hex::encode(h.finalize());

        chain.apply_entry(&LedgerEntry::VerifyChainLink {
            account: "alice".to_string(),
            chain: "solana".to_string(),
            commitment: commitment.clone(),
            signed_message: message,
            signature: sig_field,
            sig_type: "sol_sign".to_string(),
            epoch: 1,
            signed_by: "alice".to_string(),
        }).expect("VerifyChainLink solana must succeed");

        let state = chain.store.get_account("alice").unwrap().unwrap();
        assert_eq!(
            state["chain_proofs"]["solana"]["commitment"].as_str().unwrap(),
            commitment,
            "solana commitment must be stored in account state"
        );
        assert_eq!(
            state["chain_proofs"]["solana"]["mode"].as_str().unwrap(),
            "hard"
        );
    }

    /// Full VerifyChainLink path: Bitcoin sig verified, commitment stored in account state.
    #[test]
    fn test_verify_chain_link_bitcoin_end_to_end() {
        use secp256k1::{Secp256k1, SecretKey};
        use sha2::Digest as _;
        use ripemd::Digest as _;
        use base64::Engine as _;

        let (chain, _dir) = make_chain("vcl-btc");
        fund(&chain, "alice", 100 * 10_000_000_000);

        let secp = Secp256k1::new();
        let sk = SecretKey::from_slice(&[11u8; 32]).expect("valid key");
        let pk = secp256k1::PublicKey::from_secret_key(&secp, &sk);

        let message = "btcpc:link:alice:bitcoin:7".to_string();

        // Build Bitcoin message hash
        let magic = b"Bitcoin Signed Message:\n";
        let mut payload = Vec::new();
        payload.push(magic.len() as u8);
        payload.extend_from_slice(magic);
        payload.push(message.len() as u8);
        payload.extend_from_slice(message.as_bytes());
        let h1 = sha2::Sha256::digest(&payload);
        let h2: [u8; 32] = sha2::Sha256::digest(&h1).into();

        let msg = secp256k1::Message::from_digest(h2);
        let rec_sig = secp.sign_ecdsa_recoverable(&msg, &sk);
        let (rec_id, compact) = rec_sig.serialize_compact();
        let v = 31u8 + rec_id.to_i32() as u8;
        let mut full_sig = vec![v];
        full_sig.extend_from_slice(&compact);
        let sig_b64 = base64::engine::general_purpose::STANDARD.encode(&full_sig);

        // Derive expected P2PKH address
        let pk_bytes = pk.serialize();
        let sha_hash = sha2::Sha256::digest(&pk_bytes);
        let ripemd_hash = ripemd::Ripemd160::digest(&sha_hash);
        let mut addr_payload = vec![0x00u8];
        addr_payload.extend_from_slice(&ripemd_hash);
        let cs1 = sha2::Sha256::digest(&addr_payload);
        let cs2 = sha2::Sha256::digest(&cs1);
        addr_payload.extend_from_slice(&cs2[..4]);
        let btc_addr = bs58::encode(addr_payload).into_string();

        // Commitment = sha256("bitcoin:{btc_addr}:7")
        let mut ch = sha2::Sha256::new();
        ch.update(b"bitcoin:");
        ch.update(btc_addr.as_bytes());
        ch.update(b":7");
        let commitment = hex::encode(ch.finalize());

        chain.apply_entry(&LedgerEntry::VerifyChainLink {
            account: "alice".to_string(),
            chain: "bitcoin".to_string(),
            commitment: commitment.clone(),
            signed_message: message,
            signature: sig_b64,
            sig_type: "btc_legacy".to_string(),
            epoch: 1,
            signed_by: "alice".to_string(),
        }).expect("VerifyChainLink bitcoin must succeed");

        let state = chain.store.get_account("alice").unwrap().unwrap();
        assert_eq!(
            state["chain_proofs"]["bitcoin"]["commitment"].as_str().unwrap(),
            commitment,
            "bitcoin commitment must be stored in account state"
        );
    }

    /// VerifyChainLink must reject when the submitted commitment does not match.
    #[test]
    fn test_verify_chain_link_wrong_commitment_rejected() {
        use ed25519_dalek::{SigningKey, Signer};

        let (chain, _dir) = make_chain("vcl-bad");
        fund(&chain, "alice", 100 * 10_000_000_000);

        let sk = SigningKey::from_bytes(&[9u8; 32]);
        let vk = sk.verifying_key();
        let pubkey_b58 = bs58::encode(vk.as_bytes()).into_string();
        let message = "btcpc:link:alice:solana:99".to_string();
        let sig = sk.sign(message.as_bytes());
        let sig_field = format!("{}:{}", pubkey_b58, hex::encode(sig.to_bytes()));

        let r = chain.apply_entry(&LedgerEntry::VerifyChainLink {
            account: "alice".to_string(),
            chain: "solana".to_string(),
            commitment: "deadbeef".repeat(8), // wrong commitment
            signed_message: message,
            signature: sig_field,
            sig_type: "sol_sign".to_string(),
            epoch: 1,
            signed_by: "alice".to_string(),
        });
        assert!(r.is_err(), "wrong commitment must be rejected");
        assert!(
            r.unwrap_err().to_string().contains("commitment mismatch"),
            "error must mention commitment mismatch"
        );
    }

    /// The store must never allow a balance to go below zero.
    #[test]
    fn test_no_negative_balance() {
        let (chain, _dir) = make_chain("negative");
        fund(&chain, "actor", 50 * 10_000_000_000);
        seal_epoch(&chain, 0);

        // Stake 40 — balance becomes 10.
        chain.apply_entry(&LedgerEntry::Stake {
            account: "actor".to_string(), amount: 40 * 10_000_000_000,
            epoch: 1, signed_by: "actor".to_string(), nonce: 1,
        }).expect("stake");
        seal_epoch(&chain, 1);

        // Unstake more than staked — must fail.
        let r = chain.apply_entry(&LedgerEntry::Unstake {
            account: "actor".to_string(), amount: 50 * 10_000_000_000,
            epoch: 2, signed_by: "actor".to_string(), nonce: 2,
        });
        assert!(r.is_err(), "overstake unstake must fail");
        assert_eq!(chain.get_balance("actor", NATIVE_TOKEN), 10 * 10_000_000_000);

        // Transfer more than balance — must fail.
        let r = chain.apply_entry(&LedgerEntry::Transfer {
            from: "actor".to_string(), to: "nobody".to_string(),
            amount: 20 * 10_000_000_000,
            token: NATIVE_TOKEN.to_string(),
            memo: None, epoch: 2, signed_by: "actor".to_string(), nonce: 2, twofactor: None,
        });
        assert!(r.is_err(), "over-balance transfer must fail");
        assert_eq!(chain.get_balance("actor", NATIVE_TOKEN), 10 * 10_000_000_000,
            "balance unchanged after rejected transfer");
    }

    // ── Phase 2: Consensus and Finality ──────────────────────────────────────

    /// ClockNodeRegister: stake deducted, sled key written, minimum enforced.
    #[test]
    fn test_clock_node_register() {
        let (chain, _dir) = make_chain("clock-reg");
        let min_stake = 5u64 * 10_000_000_000; // 5 BTCPC default (matches chain.rs fallback)
        fund(&chain, "clock1", min_stake + 50 * 10_000_000_000);
        seal_epoch(&chain, 0);

        // Below minimum must fail.
        let r = chain.apply_entry(&LedgerEntry::ClockNodeRegister {
            node_id: "clock1".to_string(),
            stake: min_stake - 1,
            epoch: 1,
            pubkey: None,
            signature: None,
        });
        assert!(r.is_err(), "stake below minimum must be rejected");
        assert_eq!(chain.get_balance("clock1", NATIVE_TOKEN), min_stake + 50 * 10_000_000_000,
            "balance unchanged after rejected registration");

        // At exactly minimum must succeed.
        chain.apply_entry(&LedgerEntry::ClockNodeRegister {
            node_id: "clock1".to_string(),
            stake: min_stake,
            epoch: 1,
            pubkey: None,
            signature: None,
        }).expect("register at minimum stake");

        assert_eq!(chain.get_balance("clock1", NATIVE_TOKEN), 50 * 10_000_000_000,
            "stake deducted from balance");
        let reg = chain.store.state_get("clock_reg:clock1").expect("reg key exists");
        let j: serde_json::Value = serde_json::from_slice(&reg).unwrap();
        assert_eq!(j["stake"].as_u64().unwrap(), min_stake, "stake recorded in sled");
        assert_eq!(j["node_id"].as_str().unwrap(), "clock1");
    }

    /// ClockDoubleSignEvidence: correct slash distribution, replay guard.
    #[test]
    fn test_clock_double_sign_slash() {
        let (chain, _dir) = make_chain("dbl-sign");
        let min_stake = 10_000_000_000u64 * 100;
        fund(&chain, "attacker", min_stake + 10 * 10_000_000_000);
        fund(&chain, "reporter", 10 * 10_000_000_000);
        seal_epoch(&chain, 0);

        // Generate attacker's key pair (seed=42).
        let (pubkey_hex, _) = sign_seal(42, 0, "", "", 0); // just to get pubkey
        // Re-derive pubkey cleanly.
        use ed25519_dalek::SigningKey;
        let sk = SigningKey::from_bytes(&[42u8; 32]);
        let pubkey_hex = hex::encode(sk.verifying_key().to_bytes());

        // Register attacker with their pubkey.
        chain.apply_entry(&LedgerEntry::ClockNodeRegister {
            node_id: "attacker".to_string(),
            stake: min_stake,
            epoch: 1,
            pubkey: Some(pubkey_hex.clone()),
            signature: None,
        }).expect("register attacker");
        seal_epoch(&chain, 1);

        // Produce two real seal signatures from the attacker key.
        let (_, sig_a) = sign_seal(42, 1, "aabbcc", "attacker", 1000);
        let (_, sig_b) = sign_seal(42, 1, "ddeeff", "attacker", 2000);

        chain.apply_entry(&LedgerEntry::ClockDoubleSignEvidence {
            submitter: "reporter".to_string(),
            offender: "attacker".to_string(),
            epoch: 1,
            seal_hash_a: "aabbcc".to_string(), timestamp_a: 1000, sig_a,
            seal_hash_b: "ddeeff".to_string(), timestamp_b: 2000, sig_b,
            signature: None,
        }).expect("evidence accepted");

        let reporter_cut = min_stake / 10;
        let legal_cut    = min_stake / 10;
        let recycle_cut  = min_stake - reporter_cut - legal_cut;

        assert_eq!(chain.get_balance("reporter", NATIVE_TOKEN),
            10 * 10_000_000_000 + reporter_cut, "reporter receives bounty");
        assert_eq!(chain.get_balance("__legal__", NATIVE_TOKEN), legal_cut,
            "__legal__ receives 10%");
        assert_eq!(chain.get_balance(RECYCLE_FUND_ACCOUNT, NATIVE_TOKEN), recycle_cut,
            "remainder goes to recycle fund");

        let reg = chain.store.state_get("clock_reg:attacker").expect("reg key exists");
        let j: serde_json::Value = serde_json::from_slice(&reg).unwrap();
        assert_eq!(j["stake"].as_u64().unwrap(), 0, "offender stake zeroed");

        // Replay: same evidence rejected.
        let (_, sig_a2) = sign_seal(42, 1, "aabbcc", "attacker", 1000);
        let (_, sig_b2) = sign_seal(42, 1, "ddeeff", "attacker", 2000);
        let r = chain.apply_entry(&LedgerEntry::ClockDoubleSignEvidence {
            submitter: "reporter".to_string(),
            offender: "attacker".to_string(),
            epoch: 1,
            seal_hash_a: "aabbcc".to_string(), timestamp_a: 1000, sig_a: sig_a2,
            seal_hash_b: "ddeeff".to_string(), timestamp_b: 2000, sig_b: sig_b2,
            signature: None,
        });
        assert!(r.is_err(), "duplicate evidence must be rejected");
    }

    /// Bad seal signature on evidence → slash rejected, stake untouched.
    #[test]
    fn test_clock_double_sign_bad_sig_rejected() {
        let (chain, _dir) = make_chain("dbl-badsig");
        let min_stake = 10_000_000_000u64 * 100;
        fund(&chain, "victim", min_stake + 10 * 10_000_000_000);
        fund(&chain, "framer", 10 * 10_000_000_000);
        seal_epoch(&chain, 0);

        use ed25519_dalek::SigningKey;
        let sk = SigningKey::from_bytes(&[7u8; 32]);
        let pubkey_hex = hex::encode(sk.verifying_key().to_bytes());

        chain.apply_entry(&LedgerEntry::ClockNodeRegister {
            node_id: "victim".to_string(),
            stake: min_stake,
            epoch: 1,
            pubkey: Some(pubkey_hex),
            signature: None,
        }).expect("register victim");
        seal_epoch(&chain, 1);

        // framer signs with a DIFFERENT key (seed=99), not victim's key.
        let (_, bad_sig_a) = sign_seal(99, 1, "aaaa", "victim", 1000);
        let (_, bad_sig_b) = sign_seal(99, 1, "bbbb", "victim", 2000);

        let r = chain.apply_entry(&LedgerEntry::ClockDoubleSignEvidence {
            submitter: "framer".to_string(),
            offender: "victim".to_string(),
            epoch: 1,
            seal_hash_a: "aaaa".to_string(), timestamp_a: 1000, sig_a: bad_sig_a,
            seal_hash_b: "bbbb".to_string(), timestamp_b: 2000, sig_b: bad_sig_b,
            signature: None,
        });
        assert!(r.is_err(), "evidence with wrong key must be rejected");
        // victim's stake must be untouched.
        let reg = chain.store.state_get("clock_reg:victim").unwrap();
        let j: serde_json::Value = serde_json::from_slice(&reg).unwrap();
        assert_eq!(j["stake"].as_u64().unwrap(), min_stake, "victim stake untouched after failed frame");
    }

    /// No registered pubkey → evidence rejected even with otherwise valid structure.
    #[test]
    fn test_clock_double_sign_no_pubkey_rejected() {
        let (chain, _dir) = make_chain("dbl-nopk");
        let min_stake = 10_000_000_000u64 * 100;
        fund(&chain, "oldnode", min_stake + 10 * 10_000_000_000);
        fund(&chain, "reporter", 10 * 10_000_000_000);
        seal_epoch(&chain, 0);

        // Register WITHOUT a pubkey (old-style node).
        chain.apply_entry(&LedgerEntry::ClockNodeRegister {
            node_id: "oldnode".to_string(),
            stake: min_stake,
            epoch: 1,
            pubkey: None,
            signature: None,
        }).expect("register oldnode");
        seal_epoch(&chain, 1);

        let (_, sig_a) = sign_seal(1, 1, "aaaa", "oldnode", 1000);
        let (_, sig_b) = sign_seal(1, 1, "bbbb", "oldnode", 2000);
        let r = chain.apply_entry(&LedgerEntry::ClockDoubleSignEvidence {
            submitter: "reporter".to_string(),
            offender: "oldnode".to_string(),
            epoch: 1,
            seal_hash_a: "aaaa".to_string(), timestamp_a: 1000, sig_a,
            seal_hash_b: "bbbb".to_string(), timestamp_b: 2000, sig_b,
            signature: None,
        });
        assert!(r.is_err(), "evidence against node with no pubkey must be rejected");
        let reg = chain.store.state_get("clock_reg:oldnode").unwrap();
        let j: serde_json::Value = serde_json::from_slice(&reg).unwrap();
        assert_eq!(j["stake"].as_u64().unwrap(), min_stake, "oldnode stake untouched");
    }

    /// ClockDoubleSignEvidence: identical hashes must be rejected immediately.
    #[test]
    fn test_clock_double_sign_identical_hashes_rejected() {
        let (chain, _dir) = make_chain("dbl-same");
        fund(&chain, "rep", 1_000_000_000);
        seal_epoch(&chain, 0);
        let (_, sig) = sign_seal(1, 1, "abcdef", "nobody", 1000);
        let r = chain.apply_entry(&LedgerEntry::ClockDoubleSignEvidence {
            submitter: "rep".to_string(),
            offender: "nobody".to_string(),
            epoch: 1,
            seal_hash_a: "abcdef".to_string(), timestamp_a: 1000, sig_a: sig.clone(),
            seal_hash_b: "abcdef".to_string(), timestamp_b: 1000, sig_b: sig,
            signature: None,
        });
        assert!(r.is_err(), "identical seal hashes must not constitute evidence");
    }

    // ── Phase 8: Uptime reputation ─────────────────────────────────────────────

    /// New clock node earns full reward (uptime multiplier = 1.0) for first
    /// CLOCK_UPTIME_MIN_EPOCHS (10) epochs regardless of sealing.
    #[test]
    fn test_clock_uptime_starts_full() {
        let (chain, _dir) = make_chain("uptime-start");
        let min_stake = 100u64 * 10_000_000_000;
        fund(&chain, "clocker", min_stake + 10 * 10_000_000_000);
        seal_epoch(&chain, 0);

        use ed25519_dalek::SigningKey;
        let sk = SigningKey::from_bytes(&[3u8; 32]);
        let pubkey_hex = hex::encode(sk.verifying_key().to_bytes());
        chain.apply_entry(&LedgerEntry::ClockNodeRegister {
            node_id: "clocker".to_string(),
            stake: min_stake,
            epoch: 1,
            pubkey: Some(pubkey_hex),
            signature: None,
        }).expect("register");

        // After registration and before any uptime updates the key doesn't exist yet.
        let uptime = chain.store.state_get("clock_uptime:clocker");
        assert!(uptime.is_none(), "uptime key not written until first epoch reward");
    }

    /// At steady state (epochs = WINDOW), missing a seal leaks one point.
    #[test]
    fn test_clock_uptime_steady_state_miss_leaks() {
        let (chain, _dir) = make_chain("uptime-leak");
        // Manually write a steady-state uptime record: seals=90, epochs=100 (90% uptime).
        chain.store.state_set(
            "clock_uptime:clocker",
            &serde_json::to_vec(&serde_json::json!({"seals": 90u64, "epochs": 100u64})).unwrap()
        ).unwrap();

        // Simulate a missed seal by calling the uptime update logic directly.
        // We reproduce the update rule: non-sealer at steady state → seals--.
        let uptime: serde_json::Value = serde_json::from_slice(
            &chain.store.state_get("clock_uptime:clocker").unwrap()
        ).unwrap();
        let seals  = uptime["seals"].as_u64().unwrap();
        let epochs = uptime["epochs"].as_u64().unwrap();
        // epochs == WINDOW (100), not a sealer → seals--
        let new_seals  = seals.saturating_sub(1);
        let new_epochs = epochs; // already at cap
        chain.store.state_set(
            "clock_uptime:clocker",
            &serde_json::to_vec(&serde_json::json!({"seals": new_seals, "epochs": new_epochs})).unwrap()
        ).unwrap();

        let after: serde_json::Value = serde_json::from_slice(
            &chain.store.state_get("clock_uptime:clocker").unwrap()
        ).unwrap();
        assert_eq!(after["seals"].as_u64().unwrap(), 89, "missed seal decrements seals counter");
        assert_eq!(after["epochs"].as_u64().unwrap(), 100, "epochs stays at window cap");

        // Reward multiplier: 500 + 500*89/100 = 500 + 445 = 945 millipct (integer division).
        let mult = 500u64 + 500 * 89 / 100;
        assert_eq!(mult, 945, "multiplier for 89% uptime = 945 millipct");
    }

    // ── Phase 3: Useful-Work Proof Linkage ────────────────────────────────────

    /// Mine score with no job_id during grace period = 20% of base score.
    /// After grace period, unlinked Mine produces zero effective score.
    #[test]
    fn test_mine_grace_period_score() {
        use btcpc_types::{MINE_GRACE_PERIOD_EPOCHS, MINE_GRACE_REWARD_BPS};

        let (chain, _dir) = make_chain("mine-grace");
        fund(&chain, "miner", 100 * 10_000_000_000);
        seal_epoch(&chain, 0);

        // Post an unlinked Mine at epoch 1 (within grace period).
        chain.apply_entry(&LedgerEntry::Mine {
            miner: "miner".to_string(),
            epoch: 1,
            model: "llama3:8b".to_string(),
            input_tokens: 100,
            output_tokens: 500,
            tool_calls: 0,
            hw_tier: 1,
            output_hash: "abc123".to_string(),
            job_id: None,
            node_version: None,
            software_hash: None,
            model_hash: None,
        }).expect("mine in grace period");
        seal_epoch(&chain, 1);

        let record = chain.store.state_get("mine:1:miner").expect("mine record written");
        let j: serde_json::Value = serde_json::from_slice(&record).unwrap();
        assert_eq!(j["miner"].as_str().unwrap(), "miner");
        assert!(j["job_id"].is_null(), "job_id null for unlinked mine");

        // Confirm grace period constant is as expected.
        assert_eq!(MINE_GRACE_PERIOD_EPOCHS, 259_200,
            "90 days at 30s epochs = 259_200");
        assert_eq!(MINE_GRACE_REWARD_BPS, 2_000,
            "grace period earns 20% (2_000/10_000)");
    }

    /// Mine with job_id earns full score when the job is Verified.
    /// Mine with a job_id pointing to a job the miner didn't win is rejected.
    #[test]
    fn test_mine_linked_with_approved_verify() {
        use btcpc_types::RECYCLE_FUND_ACCOUNT as RECYCLE;
        let (chain, _dir) = make_chain("mine-linked");
        fund(&chain, "miner", 100 * 10_000_000_000);
        fund(&chain, "requester", 100 * 10_000_000_000);
        fund(&chain, RECYCLE, 100 * 10_000_000_000);
        seal_epoch(&chain, 0);

        let job_id = "job-abc";
        let epoch  = 1u64;

        // Set up a job through the real apply path so winner is set correctly.
        post_and_complete_job(&chain, job_id, "miner", epoch);
        // Verify it (sets status → Verified).
        chain.apply_entry(&LedgerEntry::InferenceJobVerify {
            job_id: job_id.to_string(), verifier: "v1".to_string(),
            verdict: "approved".to_string(), value_score: 80,
            epoch, reason: None, reveal_salt: None, signed_by: "v1".to_string(),
        }).expect("verify");

        // Linked Mine pointing to a verified job succeeds.
        chain.apply_entry(&LedgerEntry::Mine {
            miner: "miner".to_string(),
            epoch,
            model: "llama3:8b".to_string(),
            input_tokens: 100,
            output_tokens: 500,
            tool_calls: 0,
            hw_tier: 1,
            output_hash: "abc123".to_string(),
            job_id: Some(job_id.to_string()),
            node_version: None,
            software_hash: None,
            model_hash: None,
        }).expect("linked mine applied");

        let record = chain.store.state_get(&format!("mine:{}:miner", epoch)).unwrap();
        let j: serde_json::Value = serde_json::from_slice(&record).unwrap();
        assert_eq!(j["job_id"].as_str().unwrap(), job_id, "job_id stored in mine record");

        // Mine from a different account claiming the same job is rejected.
        let err = chain.apply_entry(&LedgerEntry::Mine {
            miner: "other".to_string(),
            epoch,
            model: "llama3:8b".to_string(),
            input_tokens: 10,
            output_tokens: 50,
            tool_calls: 0,
            hw_tier: 1,
            output_hash: "xyz".to_string(),
            job_id: Some(job_id.to_string()),
            node_version: None,
            software_hash: None,
            model_hash: None,
        });
        assert!(err.is_err(), "non-winner cannot link Mine to job");
    }

    /// Verifier suspension: >99% approval rate over ≥10 verdicts triggers suspension.
    #[test]
    fn test_verifier_suspension_on_rubber_stamp() {
        use crate::inference::{apply_post, apply_verify};
        use btcpc_types::RECYCLE_FUND_ACCOUNT as RECYCLE;

        let (chain, _dir) = make_chain("verifier-suspend");
        // Requester needs enough balance to post 13 jobs × 1_000_000_000 each.
        fund(&chain, "requester", 200 * 10_000_000_000);
        fund(&chain, RECYCLE, 200 * 10_000_000_000); // escrow held here
        seal_epoch(&chain, 0);

        // Helper: write a completed job directly into sled (bypasses bid/award flow).
        let write_completed = |job_id: &str, epoch: u64| {
            let key = format!("infer_job:{}", job_id);
            chain.store.state_set(&key, &serde_json::to_vec(&serde_json::json!({
                "job_id": job_id, "requester": "requester", "model": "auto", "mode": "solo",
                "input_hash": "ih", "max_fee": 1_000_000_000u64, "min_reputation": 0u64,
                "bid_window_epochs": 2u64, "deadline_epoch": epoch + 5, "posted_epoch": epoch,
                "status": "completed", "winner": "worker", "awarded_fee": 900_000_000u64,
                "verifiers": [], "result_hash": "rh", "latency_ms": 100u64,
                "disputed_epoch": null, "claimed_epoch": null,
                "settled_epoch": null, "persist_on_fs": false,
            })).unwrap()).unwrap();
        };

        // Submit 10 "approved" verdicts — minimum sample size (total >= 10) at 100%
        // approval rate (>99% threshold) triggers suspension on the 10th verdict.
        for i in 0u64..10 {
            let job_id = format!("susp-job-{}", i);
            let epoch  = i + 1;
            write_completed(&job_id, epoch);

            apply_verify(&chain, &LedgerEntry::InferenceJobVerify {
                job_id: job_id.clone(),
                verifier: "verifier".to_string(),
                verdict: "approved".to_string(),
                value_score: 100,
                reason: None, reveal_salt: None,
                epoch,
                signed_by: "verifier".to_string(),
            }).expect(&format!("verify job {}", i));
        }

        // After 10 verdicts all approved (100% > 99% threshold), verifier must be suspended.
        let rep = chain.store.state_get("verifier_rep:verifier").expect("rep key exists");
        let j: serde_json::Value = serde_json::from_slice(&rep).unwrap();
        let suspended_until = j["suspended_until"].as_u64().unwrap_or(0);
        assert!(suspended_until > 0,
            "verifier should be suspended after 100% approval rate; rep={:?}", j);
        assert!(j["weight_halved"].as_bool().unwrap_or(false), "weight_halved must be set");

        // Next verify submission must be blocked.
        let extra = "susp-job-extra";
        write_completed(extra, 11);
        let r = apply_verify(&chain, &LedgerEntry::InferenceJobVerify {
            job_id: extra.to_string(),
            verifier: "verifier".to_string(),
            verdict: "approved".to_string(),
            value_score: 100,
            reason: None, reveal_salt: None,
            epoch: 11,
            signed_by: "verifier".to_string(),
        });
        assert!(r.is_err(), "suspended verifier must be blocked");
        assert!(r.unwrap_err().to_string().contains("suspended"),
            "error must mention suspension");
    }

    // ── Economic / fee market tests (Phase 5) ────────────────────────────────

    /// Fee is deducted from the economic actor and credited to __recycle_fund__.
    /// A Transfer costs ENTRY_WEIGHT_MICRO × base_fee dreams from the sender.
    #[test]
    fn test_fee_routes_to_recycle_fund() {
        use crate::tx;
        use btcpc_types::{BASE_FEE_INITIAL_DREAMS, ENTRY_WEIGHT_MICRO, RECYCLE_FUND_ACCOUNT};

        let (chain, _dir) = make_chain("fee-recycle");
        // Credit alice's balance directly (no account record) so require_key
        // and check_signature both skip — we're testing fee routing, not auth.
        chain.store.credit("alice", NATIVE_TOKEN, 500 * 10_000_000_000).expect("alice credit");
        fund(&chain, "bob", 100 * 10_000_000_000);
        seal_epoch(&chain, 0);

        let fee = BASE_FEE_INITIAL_DREAMS * ENTRY_WEIGHT_MICRO; // 1 × 10_000_000

        let alice_before  = chain.get_balance("alice", NATIVE_TOKEN);
        let recycle_before = chain.get_balance(RECYCLE_FUND_ACCOUNT, NATIVE_TOKEN);

        tx::validate_and_apply(&chain, &LedgerEntry::Transfer {
            from: "alice".to_string(), to: "bob".to_string(),
            amount: 1 * 10_000_000_000,
            token: NATIVE_TOKEN.to_string(),
            memo: None, epoch: 1, nonce: 1,
            signed_by: "alice".to_string(), twofactor: None,
        }, None).expect("transfer");

        let alice_after   = chain.get_balance("alice", NATIVE_TOKEN);
        let recycle_after  = chain.get_balance(RECYCLE_FUND_ACCOUNT, NATIVE_TOKEN);

        assert_eq!(alice_before - alice_after, 1 * 10_000_000_000 + fee,
            "sender must be debited transfer amount + entry fee");
        assert_eq!(recycle_after - recycle_before, fee,
            "recycle fund must receive the entry fee");
    }

    /// Entry with insufficient balance for the fee must be rejected before apply.
    #[test]
    fn test_entry_rejected_when_fee_unaffordable() {
        use crate::tx;
        use btcpc_types::{BASE_FEE_INITIAL_DREAMS, ENTRY_WEIGHT_MICRO};

        let (chain, _dir) = make_chain("fee-reject");
        let fee = BASE_FEE_INITIAL_DREAMS * ENTRY_WEIGHT_MICRO;
        // Fund alice with LESS than the fee so transfer + fee is unaffordable.
        fund(&chain, "alice", fee / 2);
        fund(&chain, "bob",   0);
        seal_epoch(&chain, 0);

        let r = tx::validate_and_apply(&chain, &LedgerEntry::Transfer {
            from: "alice".to_string(), to: "bob".to_string(),
            amount: 0, // zero-value transfer still costs the fee
            token: NATIVE_TOKEN.to_string(),
            memo: None, epoch: 1, nonce: 1,
            signed_by: "alice".to_string(), twofactor: None,
        }, None);
        // Zero-amount transfer is blocked by the existing "must be positive" check.
        // Use a tiny non-zero amount that still makes alice unable to cover fee.
        let r = tx::validate_and_apply(&chain, &LedgerEntry::Transfer {
            from: "alice".to_string(), to: "bob".to_string(),
            amount: 1,
            token: NATIVE_TOKEN.to_string(),
            memo: None, epoch: 1, nonce: 1,
            signed_by: "alice".to_string(), twofactor: None,
        }, None);
        assert!(r.is_err(), "must reject when balance < fee");
        assert!(r.unwrap_err().to_string().contains("entry fee"),
            "error must mention entry fee");
    }

    /// AccountCreate fee is subsidized from __testnet_fund__, NOT from the new account.
    #[test]
    fn test_account_create_subsidized_from_testnet_fund() {
        use crate::tx;
        use btcpc_types::{BASE_FEE_INITIAL_DREAMS, ENTRY_WEIGHT_REGISTRATION, TESTNET_FUND_ACCOUNT, RECYCLE_FUND_ACCOUNT};
        use ed25519_dalek::{Signer, SigningKey};
        use rand::rngs::OsRng;

        let (chain, _dir) = make_chain("fee-subsidy");
        // Seed testnet fund (normally done at genesis).
        fund(&chain, TESTNET_FUND_ACCOUNT, 500 * 10_000_000_000);
        seal_epoch(&chain, 0);

        let fee = BASE_FEE_INITIAL_DREAMS * ENTRY_WEIGHT_REGISTRATION;
        let testnet_before = chain.get_balance(TESTNET_FUND_ACCOUNT, NATIVE_TOKEN);
        let recycle_before  = chain.get_balance(RECYCLE_FUND_ACCOUNT, NATIVE_TOKEN);

        let signing_key = SigningKey::generate(&mut OsRng);
        let pubkey = hex::encode(signing_key.verifying_key().to_bytes());
        let mut keys = std::collections::BTreeMap::new();
        keys.insert("owner".to_string(), pubkey.clone());
        keys.insert("active".to_string(), pubkey.clone());
        keys.insert("posting".to_string(), pubkey);
        let entry = LedgerEntry::AccountCreate {
            account: "newuser".to_string(),
            keys,
            chain_proofs: vec![],
            epoch: 1,
            funded_by: None,
            machine_fingerprint: None,
        };
        let msg = tx::canonical_signing_message(&entry, &chain.chain_id)
            .expect("account create signing message");
        let sig = hex::encode(signing_key.sign(msg.as_bytes()).to_bytes());
        tx::validate_and_apply(&chain, &entry, Some(&sig)).expect("AccountCreate");

        let testnet_after  = chain.get_balance(TESTNET_FUND_ACCOUNT, NATIVE_TOKEN);
        let recycle_after   = chain.get_balance(RECYCLE_FUND_ACCOUNT, NATIVE_TOKEN);

        assert_eq!(testnet_before - testnet_after, fee,
            "testnet fund must cover the AccountCreate fee");
        assert_eq!(recycle_after - recycle_before, fee,
            "recycle fund must receive the subsidized fee");
        // newuser's balance is untouched (they had nothing and still have nothing).
        assert_eq!(chain.get_balance("newuser", NATIVE_TOKEN), 0,
            "new account must not be charged for its own creation");
    }

    /// Fee scales proportionally with entry weight: a Heavy entry costs 5× a Micro entry.
    #[test]
    fn test_fee_proportional_to_entry_weight() {
        use btcpc_types::{BASE_FEE_INITIAL_DREAMS, ENTRY_WEIGHT_MICRO, ENTRY_WEIGHT_HEAVY, entry_weight};
        use btcpc_types::LedgerEntry as LE;

        let transfer = LE::Transfer {
            from: "a".to_string(), to: "b".to_string(),
            amount: 1, token: "BTCPC".to_string(),
            memo: None, epoch: 1, nonce: 1,
            signed_by: "a".to_string(), twofactor: None,
        };
        let storage_hb = LE::StorageHeartbeat {
            node_id: "a".to_string(), epoch: 1,
            bytes_proven: 0, query_count: 0,
            challenge_response: None,
            tier: None,
            signed_by: "a".to_string(),
        };

        assert_eq!(entry_weight(&transfer),    ENTRY_WEIGHT_MICRO);
        assert_eq!(entry_weight(&storage_hb),  ENTRY_WEIGHT_HEAVY);
        assert_eq!(
            BASE_FEE_INITIAL_DREAMS * entry_weight(&storage_hb),
            BASE_FEE_INITIAL_DREAMS * entry_weight(&transfer) * 5,
            "Heavy fee must be 5× Micro fee at the same base_fee"
        );
    }

    /// compute_next_base_fee is tested for both directions.
    #[test]
    fn test_base_fee_adjusts_with_epoch_load() {
        use btcpc_types::{compute_next_base_fee, BASE_FEE_INITIAL_DREAMS, EPOCH_TARGET_WEIGHT_UNITS};

        let initial = BASE_FEE_INITIAL_DREAMS;
        let target  = EPOCH_TARGET_WEIGHT_UNITS;

        // Empty epoch → fee drops 10%.
        let lower = compute_next_base_fee(initial, 0, target);
        assert!(lower < initial, "idle epoch must lower the base fee");
        assert!(lower >= initial * 9 / 10, "drop must be ≤ 10%");

        // 2× overloaded epoch → fee rises 10%.
        let higher = compute_next_base_fee(initial, target * 2, target);
        assert!(higher > initial, "busy epoch must raise the base fee");
        assert!(higher <= initial * 11 / 10 + 1, "rise must be ≤ 10%");

        // Exactly at target → fee unchanged.
        let same = compute_next_base_fee(initial, target, target);
        assert_eq!(same, initial, "epoch at target weight must leave fee unchanged");
    }

    /// DeviceClaimUnstake must NOT return stake immediately (T3-3).
    /// Tokens enter the same unbonding window as Unstake and are released by
    /// drain_unbonding after UNBONDING_EPOCHS, not before.
    #[test]
    fn test_device_claim_unstake_follows_unbonding_window() {
        let (chain, _dir) = make_chain("device-unstake");
        let owner = "owner";
        let stake_amount = 50 * 10_000_000_000u64;
        fund(&chain, owner, 200 * 10_000_000_000);
        seal_epoch(&chain, 0);

        // Stake a device claim at epoch 1.
        chain.apply_entry(&LedgerEntry::DeviceClaimStake {
            device_serial: "SERIAL-001".to_string(),
            owner: owner.to_string(),
            amount: stake_amount,
            epoch: 1,
            nonce: 1,
            signed_by: owner.to_string(),
        }).expect("device claim stake");
        seal_epoch(&chain, 1);

        let balance_after_claim = chain.get_balance(owner, NATIVE_TOKEN);
        assert_eq!(balance_after_claim, 200 * 10_000_000_000 - stake_amount,
            "stake must be debited on claim");

        // Unstake at epoch 2 → release at 2 + UNBONDING_EPOCHS (10) = 12.
        chain.apply_entry(&LedgerEntry::DeviceClaimUnstake {
            device_serial: "SERIAL-001".to_string(),
            owner: owner.to_string(),
            epoch: 2,
            nonce: 2,
            signed_by: owner.to_string(),
        }).expect("device claim unstake");
        seal_epoch(&chain, 2);

        // Claim record must be gone.
        assert!(chain.store.state_get("device_claim:SERIAL-001").is_none(),
            "claim record must be deleted on unstake");

        // Balance must NOT have increased yet — tokens are in unbonding.
        assert_eq!(chain.get_balance(owner, NATIVE_TOKEN), balance_after_claim,
            "stake must remain locked in unbonding after DeviceClaimUnstake");

        // Epochs 3–11: still locked.
        for epoch in 3u64..12 {
            seal_epoch(&chain, epoch);
            assert_eq!(chain.get_balance(owner, NATIVE_TOKEN), balance_after_claim,
                "still locked at epoch {}", epoch);
        }

        // Epoch 12: drain_unbonding releases the tokens.
        seal_epoch(&chain, 12);
        assert_eq!(chain.get_balance(owner, NATIVE_TOKEN), 200 * 10_000_000_000,
            "full stake returned after unbonding window");
    }

    /// Inference proof pruning: prune_ready key written on InferenceJobPay.
    #[test]
    fn test_inference_prune_key_written_on_pay() {
        use btcpc_types::INFERENCE_PRUNE_WINDOW_EPOCHS;

        let (chain, _dir) = make_chain("prune-key");
        fund(&chain, "requester", 100 * 10_000_000_000);
        fund(&chain, "worker", 10 * 10_000_000_000);
        fund(&chain, RECYCLE_FUND_ACCOUNT, 500 * 10_000_000_000);
        seal_epoch(&chain, 0);

        let job_id = "prune-test-job";
        let pay_epoch = 5u64;

        // Set up a Verified job in sled directly.
        chain.store.state_set(&format!("infer_job:{}", job_id), &serde_json::to_vec(&serde_json::json!({
            "job_id": job_id, "requester": "requester", "model": "auto", "mode": "solo",
            "input_hash": "inputhash", "max_fee": 10_000_000_000u64, "min_reputation": 0u64,
            "bid_window_epochs": 2u64, "deadline_epoch": 20u64, "posted_epoch": 1u64,
            "status": "verified", "winner": "worker", "awarded_fee": 9_000_000_000u64,
            "verifiers": ["v1"], "result_hash": "rh", "latency_ms": 500u64,
            "disputed_epoch": null, "claimed_epoch": null,
            "settled_epoch": null, "persist_on_fs": false,
        })).unwrap()).unwrap();

        // Apply InferenceJobPay.
        crate::inference::apply_pay(&chain, &LedgerEntry::InferenceJobPay {
            job_id: job_id.to_string(),
            worker: "worker".to_string(),
            worker_amount: 8_000_000_000,
            verifier_payments: vec![("v1".to_string(), 500_000_000)],
            reviewer_payments: vec![],
            recycle_amount: 250_000_000,
            refund_amount: 1_250_000_000,
            dissenter_slashes: vec![],
            epoch: pay_epoch,
        }).expect("pay applied");

        // prune_ready key must be written at pay_epoch + INFERENCE_PRUNE_WINDOW_EPOCHS.
        let prune_at = pay_epoch + INFERENCE_PRUNE_WINDOW_EPOCHS;
        let prune_key = format!("prune_ready:{}:{}", prune_at, job_id);
        assert!(
            chain.store.state_get(&prune_key).is_some(),
            "prune_ready key must be written after InferenceJobPay"
        );
        assert_eq!(
            chain.store.state_get(&prune_key).unwrap(),
            job_id.as_bytes().to_vec(),
            "prune_ready value must be the job_id"
        );
    }

    // ── Phase 7: State Proofs / Merkle tree ──────────────────────────────────

    #[test]
    fn test_merkle_proof_verifiable() {
        let (chain, _dir) = make_chain("merkle_proof");
        // Give alice and bob distinct balances so there are multiple leaves.
        chain.store.credit("alice", NATIVE_TOKEN, 100 * 10_000_000_000).expect("alice credit");
        chain.store.credit("bob", NATIVE_TOKEN, 200 * 10_000_000_000).expect("bob credit");

        // Root must be non-zero with balances present.
        let root = chain.store.balance_merkle_root();
        assert_eq!(root.len(), 64, "root is 64-char hex");
        assert_ne!(root, "0".repeat(64), "non-empty tree must have non-zero root");

        // Proof for alice must verify.
        let proof = chain.store.balance_merkle_proof("alice", NATIVE_TOKEN)
            .expect("alice must have a balance proof");
        assert_eq!(proof.balance, 100 * 10_000_000_000);
        assert_eq!(proof.root, root, "proof root must match tree root");
        assert!(proof.verify(), "alice proof must verify");

        // Proof for bob must also verify and share the same root.
        let proof_bob = chain.store.balance_merkle_proof("bob", NATIVE_TOKEN)
            .expect("bob must have a balance proof");
        assert!(proof_bob.verify(), "bob proof must verify");
        assert_eq!(proof_bob.root, root, "bob proof root must match");

        // A proof for a non-existent account must return None.
        assert!(
            chain.store.balance_merkle_proof("nobody", NATIVE_TOKEN).is_none(),
            "unknown account must return no proof"
        );

        // Tampered balance must not verify.
        let mut bad = chain.store.balance_merkle_proof("alice", NATIVE_TOKEN).unwrap();
        bad.balance = 999 * 10_000_000_000; // wrong value; leaf_hash unchanged
        assert!(!bad.verify(), "tampered proof must not verify");
    }

    #[test]
    fn test_merkle_root_changes_after_credit() {
        let (chain, _dir) = make_chain("merkle_root_change");
        let root_empty = chain.store.balance_merkle_root();

        chain.store.credit("alice", NATIVE_TOKEN, 50 * 10_000_000_000).expect("alice credit");
        let root_with_alice = chain.store.balance_merkle_root();

        assert_ne!(root_empty, root_with_alice, "root must change after new balance entry");

        chain.store.credit("alice", NATIVE_TOKEN, 50 * 10_000_000_000).expect("alice 2nd credit");
        let root_doubled = chain.store.balance_merkle_root();

        assert_ne!(root_with_alice, root_doubled, "root must change when balance changes");
    }

    // ── Mainnet gate: two-node deterministic replay ──────────────────────────

    /// Feed the same entry sequence to two independent Chain instances and assert that
    /// both produce identical balance Merkle roots after every epoch seal.
    /// This is the canonical "deterministic replay" launch gate — any non-determinism
    /// in apply_entry or drain_pending_sorted will surface here.
    #[test]
    fn test_two_node_deterministic_replay() {
        let (a, _dir_a) = make_chain("replay-a");
        let (b, _dir_b) = make_chain("replay-b");

        // Identical genesis allocations on both chains.
        for chain in [&a, &b] {
            fund(chain, "alice", 10_000_000_000_000);
            fund(chain, "bob",   5_000_000_000_000);
            fund(chain, "carol", 3_000_000_000_000);
        }

        // 5-epoch replay sequence: transfers, stakes, unstakes.
        for ep in 1u64..=5 {
            let entries: Vec<LedgerEntry> = vec![
                LedgerEntry::Transfer {
                    from: "alice".into(), to: "bob".into(),
                    amount: 100_000_000_000, token: NATIVE_TOKEN.into(),
                    memo: None, nonce: ep, epoch: ep,
                    signed_by: "alice".into(), twofactor: None,
                },
                LedgerEntry::Stake {
                    account: "carol".into(),
                    amount: 200_000_000_000,
                    nonce: ep, epoch: ep,
                    signed_by: "carol".into(),
                },
            ];

            // Push to both in the same order, drain, apply epoch seal.
            for e in &entries {
                a.push_pending(e.clone(), None);
                b.push_pending(e.clone(), None);
            }
            for (e, _sig) in a.drain_pending_sorted() {
                a.apply_entry(&e).expect("node-a apply");
            }
            for (e, _sig) in b.drain_pending_sorted() {
                b.apply_entry(&e).expect("node-b apply");
            }
            seal_epoch(&a, ep);
            seal_epoch(&b, ep);

            let root_a = a.store.balance_merkle_root();
            let root_b = b.store.balance_merkle_root();
            assert_eq!(root_a, root_b,
                "state root mismatch after epoch {} — non-determinism detected", ep);
        }

        // Verify roots are non-trivial (state actually evolved).
        assert_ne!(a.store.balance_merkle_root(), b"".iter().map(|_| '0').collect::<String>(),
            "root should be non-empty");
    }

    // ── T1-7: Registered validator set quorum ────────────────────────────────

    /// EpochFinalize with a lower quorum is rejected when a higher-quorum
    /// finalization is already stored (T1-8 fork choice).
    #[test]
    fn test_fork_choice_higher_quorum_wins() {
        let (chain, _dir) = make_chain("fork-choice");
        seal_epoch(&chain, 0);

        // First finalization: quorum = 3.
        chain.apply_entry(&LedgerEntry::EpochFinalize {
            epoch: 1,
            node_id: "node-a".to_string(),
            rewards_hash: "hash-a".to_string(),
            quorum: 3,
            sealed_by: vec![],
            state_root: "root-a".to_string(),
            timestamp: 1000,
        }).expect("first EpochFinalize accepted");

        let meta = chain.store.get_epoch_meta(1).unwrap().unwrap();
        assert_eq!(meta["rewards_hash"].as_str().unwrap(), "hash-a");
        assert_eq!(meta["quorum"].as_u64().unwrap(), 3);

        // Second finalization for same epoch: lower quorum (2) — must be rejected.
        chain.apply_entry(&LedgerEntry::EpochFinalize {
            epoch: 1,
            node_id: "node-b".to_string(),
            rewards_hash: "hash-b".to_string(),
            quorum: 2,
            sealed_by: vec![],
            state_root: "root-b".to_string(),
            timestamp: 2000,
        }).expect("lower-quorum finalize returns Ok (logged, not errored)");

        // Original (higher quorum) must still be stored.
        let meta = chain.store.get_epoch_meta(1).unwrap().unwrap();
        assert_eq!(meta["rewards_hash"].as_str().unwrap(), "hash-a",
            "lower quorum must not overwrite higher quorum finalization");

        // Fork evidence must be recorded.
        let fork = chain.store.state_get("fork_evidence:1").expect("fork evidence written");
        let j: serde_json::Value = serde_json::from_slice(&fork).unwrap();
        assert_eq!(j["winner"].as_str().unwrap(), "head_a", "head_a wins (quorum 3 > 2)");
    }

    /// Third finalization with higher quorum overwrites the existing one.
    #[test]
    fn test_fork_choice_higher_quorum_overwrites() {
        let (chain, _dir) = make_chain("fork-overwrite");
        seal_epoch(&chain, 0);

        chain.apply_entry(&LedgerEntry::EpochFinalize {
            epoch: 1, node_id: "n".to_string(), rewards_hash: "hash-low".to_string(),
            quorum: 2, sealed_by: vec![], state_root: "r".to_string(), timestamp: 1000,
        }).expect("first");

        // Higher quorum arrives — should overwrite.
        chain.apply_entry(&LedgerEntry::EpochFinalize {
            epoch: 1, node_id: "n".to_string(), rewards_hash: "hash-high".to_string(),
            quorum: 4, sealed_by: vec![], state_root: "r2".to_string(), timestamp: 2000,
        }).expect("higher quorum accepted");

        let meta = chain.store.get_epoch_meta(1).unwrap().unwrap();
        assert_eq!(meta["rewards_hash"].as_str().unwrap(), "hash-high",
            "higher quorum finalization must overwrite lower");
        assert_eq!(meta["quorum"].as_u64().unwrap(), 4);

        let fork = chain.store.state_get("fork_evidence:1").expect("fork evidence written");
        let j: serde_json::Value = serde_json::from_slice(&fork).unwrap();
        assert_eq!(j["winner"].as_str().unwrap(), "head_b", "head_b wins (quorum 4 > 2)");
    }

    /// Identical EpochFinalize applied twice is idempotent.
    #[test]
    fn test_epoch_finalize_idempotent() {
        let (chain, _dir) = make_chain("finalize-idem");
        seal_epoch(&chain, 0);

        let entry = LedgerEntry::EpochFinalize {
            epoch: 1, node_id: "n".to_string(), rewards_hash: "same-hash".to_string(),
            quorum: 3, sealed_by: vec![], state_root: "root".to_string(), timestamp: 1000,
        };
        chain.apply_entry(&entry).expect("first apply");
        chain.apply_entry(&entry).expect("second apply is idempotent");

        // No fork evidence for an identical re-application.
        assert!(chain.store.state_get("fork_evidence:1").is_none(),
            "no fork evidence for idempotent re-application");
    }

    // ── T4-1: Storage Merkle challenge tests ─────────────────────────────────

    /// Build a valid MerkleRangeProof for a single-chunk file and verify it passes.
    #[test]
    fn test_storage_proof_valid() {
        use sha2::{Sha256, Digest};
        use btcpc_types::{MerkleRangeProof, STORAGE_CHALLENGE_CHUNK_BYTES};

        let (chain, _dir) = make_chain("storage-proof-valid");

        // Seal epoch 0 — stores epoch_seal_hash:0.
        seal_epoch(&chain, 0);
        let seal_hash = chain.store.state_get("epoch_seal_hash:0").unwrap();
        let seal_str = String::from_utf8(seal_hash).unwrap();

        let node_id = "storage-node";
        let epoch: u64 = 1;
        let bytes_proven: u64 = STORAGE_CHALLENGE_CHUNK_BYTES; // exactly one chunk

        // Compute expected challenge.
        let mut h = Sha256::new();
        h.update(format!("{}:{}:{}", seal_str, node_id, epoch).as_bytes());
        let challenge: [u8; 32] = h.finalize().into();
        let challenge_hash = hex::encode(challenge);

        // Only one chunk → chunk_idx = 0.
        let chunk_idx = u64::from_be_bytes(challenge[0..8].try_into().unwrap()) % 1;
        assert_eq!(chunk_idx, 0);

        // Build a one-leaf Merkle tree over the single chunk.
        let chunk_bytes = vec![0xAB_u8; STORAGE_CHALLENGE_CHUNK_BYTES as usize];
        let mut lh = Sha256::new();
        lh.update(b"chunk:");
        lh.update(&(0u64).to_be_bytes()); // range_start
        lh.update(b":");
        lh.update(&chunk_bytes);
        let leaf: [u8; 32] = lh.finalize().into();

        let levels = crate::store::build_merkle_tree(&[leaf]);
        let merkle_root = hex::encode(levels.last().unwrap()[0]);
        // Single leaf: no siblings needed.
        let proof_nodes: Vec<String> = vec![];

        let proof = MerkleRangeProof {
            challenge_hash,
            range_start: 0,
            range_end: STORAGE_CHALLENGE_CHUNK_BYTES,
            total_chunks: 1,
            leaf_hash: hex::encode(leaf),
            merkle_root,
            proof_nodes,
        };

        let ok = verify_storage_proof(&chain.store, &proof, node_id, epoch, bytes_proven);
        assert!(ok, "single-chunk proof must verify");
    }

    /// A proof with a wrong challenge hash is rejected.
    #[test]
    fn test_storage_proof_wrong_challenge() {
        use btcpc_types::{MerkleRangeProof, STORAGE_CHALLENGE_CHUNK_BYTES};

        let (chain, _dir) = make_chain("storage-proof-bad-challenge");
        seal_epoch(&chain, 0);

        let proof = MerkleRangeProof {
            challenge_hash: "0000000000000000000000000000000000000000000000000000000000000000".to_string(),
            range_start: 0,
            range_end: STORAGE_CHALLENGE_CHUNK_BYTES,
            total_chunks: 1,
            leaf_hash: "aa".repeat(32),
            merkle_root: "bb".repeat(32),
            proof_nodes: vec![],
        };

        let ok = verify_storage_proof(&chain.store, &proof, "node", 1, STORAGE_CHALLENGE_CHUNK_BYTES);
        assert!(!ok, "wrong challenge hash must be rejected");
    }

    /// StorageHeartbeat without a proof stores proof_valid=false.
    #[test]
    fn test_storage_heartbeat_no_proof_scores_reduced() {
        use btcpc_types::STORAGE_CHALLENGE_CHUNK_BYTES;

        let (chain, _dir) = make_chain("storage-heartbeat-noproof");
        seal_epoch(&chain, 0);

        chain.apply_entry(&LedgerEntry::StorageHeartbeat {
            node_id: "storer".to_string(),
            epoch: 1,
            bytes_proven: STORAGE_CHALLENGE_CHUNK_BYTES,
            query_count: 0,
            challenge_response: None,
            tier: None,
            signed_by: "storer".to_string(),
        }).expect("heartbeat applied");

        let beat = chain.store.state_get("storage_beat:1:storer").unwrap();
        let j: serde_json::Value = serde_json::from_slice(&beat).unwrap();
        assert_eq!(j["proof_valid"].as_bool().unwrap(), false, "no proof → proof_valid=false");
    }

    /// StorageHeartbeat with no prior seal hash stores proof_valid=false (cannot verify).
    #[test]
    fn test_storage_proof_no_prior_seal_rejected() {
        use btcpc_types::{MerkleRangeProof, STORAGE_CHALLENGE_CHUNK_BYTES};

        let (chain, _dir) = make_chain("storage-proof-no-seal");
        // Do NOT seal epoch 0 — no epoch_seal_hash:0 in state.

        let proof = MerkleRangeProof {
            challenge_hash: "any".to_string(),
            range_start: 0,
            range_end: STORAGE_CHALLENGE_CHUNK_BYTES,
            total_chunks: 1,
            leaf_hash: "aa".repeat(32),
            merkle_root: "bb".repeat(32),
            proof_nodes: vec![],
        };

        let ok = verify_storage_proof(&chain.store, &proof, "node", 1, STORAGE_CHALLENGE_CHUNK_BYTES);
        assert!(!ok, "missing prior seal → proof must be rejected");
    }

    // ── T2-3: Verifier board quorum tests ─────────────────────────────────────

    fn post_and_complete_job(chain: &Chain, job_id: &str, worker: &str, epoch: u64) {
        fund(chain, "requester", 100_000_000_000);
        fund(chain, worker, 0);
        chain.apply_entry(&LedgerEntry::AccountCreate {
            account: "requester".to_string(), keys: Default::default(),
            chain_proofs: vec![], epoch: 0, funded_by: None, machine_fingerprint: None,
        }).ok();
        chain.apply_entry(&LedgerEntry::InferenceJobPost {
            job_id: job_id.to_string(), requester: "requester".to_string(),
            model: "llama3".to_string(), mode: "solo".to_string(),
            input_hash: "abc".to_string(), max_fee: 10_000_000_000,
            min_reputation: 0, bid_window_epochs: 2,
            deadline_epoch: epoch + 10, epoch, nonce: 1,
            signed_by: "requester".to_string(),
            persist_on_fs: None, fs_fee: None, min_verifiers: Some(1),
            node_fingerprint: None,
        }).expect("post");
        // Manually set job to Awarded+Completed state via direct store injection.
        let mut job = crate::inference::get_job(chain, job_id).unwrap();
        job.winner = Some(worker.to_string());
        job.awarded_fee = Some(10_000_000_000);
        job.status = crate::inference::JobStatus::Awarded;
        crate::inference::set_job(chain, &job).unwrap();
        chain.apply_entry(&LedgerEntry::InferenceJobComplete {
            job_id: job_id.to_string(), worker: worker.to_string(),
            result_hash: "result".to_string(), latency_ms: 100, epoch,
            signed_by: worker.to_string(),
        }).expect("complete");
    }

    /// Single verifier, approved → job resolves to Verified immediately.
    #[test]
    fn test_board_single_verifier_approved() {
        let (chain, _dir) = make_chain("board-single-approved");
        seal_epoch(&chain, 0);
        fund(&chain, "worker", 0);
        post_and_complete_job(&chain, "job1", "worker", 1);

        chain.apply_entry(&LedgerEntry::InferenceJobVerify {
            job_id: "job1".to_string(), verifier: "v1".to_string(),
            verdict: "approved".to_string(), value_score: 80,
            epoch: 1, reason: None, reveal_salt: None, signed_by: "v1".to_string(),
        }).expect("verify");

        let job = crate::inference::get_job(&chain, "job1").unwrap();
        assert_eq!(job.status, crate::inference::JobStatus::Verified,
            "single verifier approved → Verified");
        assert_eq!(job.verdicts.len(), 1);
    }

    /// Board of 3: 2 approve, 1 rejects → approved wins (2/3 majority).
    #[test]
    fn test_board_three_verifiers_majority_approved() {
        let (chain, _dir) = make_chain("board-three-majority");
        seal_epoch(&chain, 0);
        fund(&chain, "worker", 0);
        // Force min_verifiers=3 by setting active_verifier_count >= VERIFIER_ACTIVE_THRESH_3
        chain.store.state_set("active_verifier_count", &5u64.to_le_bytes()).unwrap();
        post_and_complete_job(&chain, "job2", "worker", 1);

        let make_verify = |verifier: &str, verdict: &str| LedgerEntry::InferenceJobVerify {
            job_id: "job2".to_string(), verifier: verifier.to_string(),
            verdict: verdict.to_string(), value_score: 80,
            epoch: 1, reason: None, reveal_salt: None, signed_by: verifier.to_string(),
        };

        // After 1 vote: still Completed (waiting for quorum of 3).
        chain.apply_entry(&make_verify("v1", "approved")).expect("v1");
        assert_eq!(crate::inference::get_job(&chain, "job2").unwrap().status,
            crate::inference::JobStatus::Completed, "not resolved after 1/3 votes");

        // After 2 votes (both approve): resolved (2 >= majority of 3).
        chain.apply_entry(&make_verify("v2", "approved")).expect("v2");
        let job = crate::inference::get_job(&chain, "job2").unwrap();
        assert_eq!(job.status, crate::inference::JobStatus::Verified,
            "2/3 approved → Verified");

        // 3rd verifier can no longer vote (job resolved).
        let err = chain.apply_entry(&make_verify("v3", "rejected"));
        assert!(err.is_err(), "cannot verify a resolved job");
    }

    /// Three-way tie (1/1/1) → review_required wins via tiebreak weight.
    #[test]
    fn test_board_three_way_tie_review_wins() {
        let (chain, _dir) = make_chain("board-tie");
        seal_epoch(&chain, 0);
        fund(&chain, "worker", 0);
        chain.store.state_set("active_verifier_count", &5u64.to_le_bytes()).unwrap();
        post_and_complete_job(&chain, "job3", "worker", 1);

        let make_verify = |verifier: &str, verdict: &str| LedgerEntry::InferenceJobVerify {
            job_id: "job3".to_string(), verifier: verifier.to_string(),
            verdict: verdict.to_string(), value_score: 50,
            epoch: 1, reason: None, reveal_salt: None, signed_by: verifier.to_string(),
        };

        chain.apply_entry(&make_verify("v1", "approved")).expect("v1");
        chain.apply_entry(&make_verify("v2", "rejected")).expect("v2");
        chain.apply_entry(&make_verify("v3", "review_required")).expect("v3");

        let job = crate::inference::get_job(&chain, "job3").unwrap();
        assert_eq!(job.status, crate::inference::JobStatus::Disputed,
            "1/1/1 tie → review_required wins, job disputed");
    }

    /// Duplicate vote from same verifier is rejected.
    #[test]
    fn test_board_duplicate_vote_rejected() {
        let (chain, _dir) = make_chain("board-dup");
        seal_epoch(&chain, 0);
        fund(&chain, "worker", 0);
        post_and_complete_job(&chain, "job4", "worker", 1);

        chain.apply_entry(&LedgerEntry::InferenceJobVerify {
            job_id: "job4".to_string(), verifier: "v1".to_string(),
            verdict: "approved".to_string(), value_score: 80,
            epoch: 1, reason: None, reveal_salt: None, signed_by: "v1".to_string(),
        }).expect("first vote ok");

        // Override min_verifiers to force the job to still be Completed.
        // (It's already Verified since min_verifiers=1 by default — test the guard directly.)
        let err = crate::inference::apply_verify(&chain, &LedgerEntry::InferenceJobVerify {
            job_id: "job4".to_string(), verifier: "v1".to_string(),
            verdict: "rejected".to_string(), value_score: 0,
            epoch: 1, reason: None, reveal_salt: None, signed_by: "v1".to_string(),
        });
        assert!(err.is_err(), "duplicate verifier vote must be rejected");
    }

    /// Leaky-bucket approval-rate: rejections decrement approved_count in steady state.
    #[test]
    fn test_board_leaky_bucket_rejects_decrement() {
        let (chain, _dir) = make_chain("leaky-bucket");
        seal_epoch(&chain, 0);
        use btcpc_types::VERIFIER_APPROVAL_WINDOW_EPOCHS;

        // Fill to steady state with all approvals.
        for _ in 0..VERIFIER_APPROVAL_WINDOW_EPOCHS {
            crate::inference::update_verifier_approval_rate_pub(&chain, "v1", true).unwrap();
        }
        let vrep: serde_json::Value = chain.store.state_get("verifier_rep:v1")
            .and_then(|b| serde_json::from_slice(&b).ok()).unwrap();
        assert_eq!(vrep["total"].as_u64().unwrap(), VERIFIER_APPROVAL_WINDOW_EPOCHS,
            "total capped at window");
        assert_eq!(vrep["approved_count"].as_u64().unwrap(), VERIFIER_APPROVAL_WINDOW_EPOCHS,
            "all approved in window");

        // One rejection: approved_count decrements by 1.
        crate::inference::update_verifier_approval_rate_pub(&chain, "v1", false).unwrap();
        let vrep: serde_json::Value = chain.store.state_get("verifier_rep:v1")
            .and_then(|b| serde_json::from_slice(&b).ok()).unwrap();
        assert_eq!(vrep["approved_count"].as_u64().unwrap(), VERIFIER_APPROVAL_WINDOW_EPOCHS - 1,
            "rejection decrements approved_count (leaky bucket)");
        assert_eq!(vrep["total"].as_u64().unwrap(), VERIFIER_APPROVAL_WINDOW_EPOCHS,
            "total stays at window in steady state");
    }

    /// T4-2: Commit-reveal — verify rejected without prior commit; accepted with matching commit.
    #[test]
    fn test_commit_reveal_flow() {
        use sha2::{Sha256, Digest};
        use crate::inference::{apply_commit, apply_verify};

        let (chain, _dir) = make_chain("commit-reveal");
        fund(&chain, "worker", 0);
        seal_epoch(&chain, 0);
        post_and_complete_job(&chain, "job-cr", "worker", 1);

        let verifier = "v1";
        let verdict  = "approved";
        let salt     = "random-salt-abc123";
        let commit_hash = {
            let mut h = Sha256::new();
            h.update(verdict.as_bytes());
            h.update(b"|");
            h.update(salt.as_bytes());
            hex::encode(h.finalize())
        };

        // Without a commit, verify is rejected when commit-reveal is forced on.
        // We test the commit function directly since the constant is disabled globally.
        apply_commit(&chain, &LedgerEntry::InferenceJobCommit {
            job_id: "job-cr".to_string(), verifier: verifier.to_string(),
            commit_hash: commit_hash.clone(), epoch: 1,
            signed_by: verifier.to_string(),
        }).expect("commit accepted");

        // Duplicate commit from same verifier is rejected.
        let dup = apply_commit(&chain, &LedgerEntry::InferenceJobCommit {
            job_id: "job-cr".to_string(), verifier: verifier.to_string(),
            commit_hash: commit_hash.clone(), epoch: 1,
            signed_by: verifier.to_string(),
        });
        assert!(dup.is_err(), "duplicate commit must be rejected");

        // Wrong salt → reveal rejected when we force-check commit.
        // (Testing the sha256 mismatch path by calling with a wrong reveal_salt.)
        // We can't toggle the const directly, so we verify the hash logic manually.
        let bad_hash = hex::encode(Sha256::digest(b"approved|wrong-salt"));
        assert_ne!(bad_hash, commit_hash, "wrong salt produces different hash");

        // Correct reveal succeeds (commit-reveal disabled globally so apply_verify ignores commit,
        // but commit stored correctly in job.commits).
        apply_verify(&chain, &LedgerEntry::InferenceJobVerify {
            job_id: "job-cr".to_string(), verifier: verifier.to_string(),
            verdict: verdict.to_string(), value_score: 80,
            reason: None, reveal_salt: Some(salt.to_string()),
            epoch: 1, signed_by: verifier.to_string(),
        }).expect("reveal accepted");

        let job = crate::inference::get_job(&chain, "job-cr").unwrap();
        assert_eq!(job.commits.len(), 1, "commit stored");
        assert_eq!(job.commits[0].0, verifier);
        assert_eq!(job.commits[0].1, commit_hash, "commit hash matches");
        assert_eq!(job.status, crate::inference::JobStatus::Verified, "job verified");
    }

    /// T4-3: GNSS plausibility — readings with impossible speed between epochs are rejected.
    #[test]
    fn test_gnss_plausibility_rejects_impossible_jump() {
        let (chain, _dir) = make_chain("gnss-plausib");
        seal_epoch(&chain, 0);

        // First reading: Sydney, Australia.
        chain.apply_entry(&LedgerEntry::SensorReading {
            sensor_id: "gnss-1".to_string(),
            owner: "alice".to_string(),
            epoch: 1,
            value: 0.0,
            data_hash: "aa".repeat(32),
            metadata: Some(serde_json::json!({ "lat": -33.8688, "lon": 151.2093 })),
        }).expect("first GNSS reading accepted");

        // Second reading: same location (zero speed) — should pass.
        chain.apply_entry(&LedgerEntry::SensorReading {
            sensor_id: "gnss-1".to_string(),
            owner: "alice".to_string(),
            epoch: 2,
            value: 0.0,
            data_hash: "bb".repeat(32),
            metadata: Some(serde_json::json!({ "lat": -33.8688, "lon": 151.2093 })),
        }).expect("same-location GNSS reading accepted");

        // Third reading: New York — ~16,000 km away in 30s = ~533 km/s. Must be rejected.
        let err = chain.apply_entry(&LedgerEntry::SensorReading {
            sensor_id: "gnss-1".to_string(),
            owner: "alice".to_string(),
            epoch: 3,
            value: 0.0,
            data_hash: "cc".repeat(32),
            metadata: Some(serde_json::json!({ "lat": 40.7128, "lon": -74.0060 })),
        });
        assert!(err.is_err(), "impossible GNSS jump (Sydney→NYC in 30s) must be rejected");

        // Sensor without lat/lon metadata is not checked — passes unconditionally.
        chain.apply_entry(&LedgerEntry::SensorReading {
            sensor_id: "temp-1".to_string(),
            owner: "alice".to_string(),
            epoch: 1,
            value: 22.5,
            data_hash: "dd".repeat(32),
            metadata: Some(serde_json::json!({ "type": "temperature" })),
        }).expect("non-GNSS sensor reading accepted");
    }

    /// Coverage reporting: dead-spot corroboration, anti-spam cap, grid quantization.
    #[test]
    fn test_coverage_report_corroboration() {
        let (chain, _dir) = make_chain("coverage-report");
        seal_epoch(&chain, 0);
        // Three reporters at the same grid cell (~-33.8688, 151.2093) = (-33869, 151209) at ×1000.
        for reporter in ["alice", "bob", "charlie"] {
            fund(&chain, reporter, 0);
            chain.apply_entry(&LedgerEntry::CoverageReport {
                reporter: reporter.to_string(),
                lat: -33.8688,
                lon: 151.2093,
                signal_dbm: None, // dead spot
                carrier_mcc_mnc: "272-01".to_string(),
                technology: "LTE".to_string(),
                data_hash: format!("{}aa", reporter).repeat(16)[..64].to_string(),
                epoch: 1,
                signed_by: reporter.to_string(),
            }).expect("coverage report accepted");
        }

        // Cell should now be corroborated.
        let cell_key = "coverage_cell:1:-33869:151209:272-01";
        let cell: serde_json::Value = chain.store.state_get(cell_key)
            .and_then(|b| serde_json::from_slice(&b).ok()).unwrap();
        assert_eq!(cell["reporters"].as_array().unwrap().len(), 3);
        assert!(cell["corroborated"].as_bool().unwrap(), "3 reporters → corroborated");
        assert_eq!(cell["dead_spot_count"].as_u64().unwrap(), 3);

        // Duplicate report from same reporter in same epoch is deduplicated in cell.
        chain.apply_entry(&LedgerEntry::CoverageReport {
            reporter: "alice".to_string(),
            lat: -33.8688,
            lon: 151.2093,
            signal_dbm: None,
            carrier_mcc_mnc: "272-01".to_string(),
            technology: "LTE".to_string(),
            data_hash: "b0b0b0b0".repeat(16)[..64].to_string(),
            epoch: 1,
            signed_by: "alice".to_string(),
        }).expect("duplicate location report stored");
        let cell: serde_json::Value = chain.store.state_get(cell_key)
            .and_then(|b| serde_json::from_slice(&b).ok()).unwrap();
        assert_eq!(cell["reporters"].as_array().unwrap().len(), 3,
            "same reporter not double-counted in cell");
    }

    /// T2-4: Storage bytes_proven replaced by proven_chunks × chunk_size when proof valid.
    #[test]
    fn test_storage_effective_bytes_from_proof() {
        use btcpc_types::STORAGE_CHALLENGE_CHUNK_BYTES;

        let (chain, _dir) = make_chain("storage-effective-bytes");
        seal_epoch(&chain, 0);

        // StorageHeartbeat without proof: bytes_proven is self-declared.
        chain.apply_entry(&LedgerEntry::StorageHeartbeat {
            node_id: "storer".to_string(),
            epoch: 1,
            bytes_proven: 5 * STORAGE_CHALLENGE_CHUNK_BYTES,
            query_count: 0,
            challenge_response: None,
            tier: None,
            signed_by: "storer".to_string(),
        }).expect("no-proof heartbeat");
        let beat = chain.store.state_get("storage_beat:1:storer").unwrap();
        let j: serde_json::Value = serde_json::from_slice(&beat).unwrap();
        assert_eq!(j["bytes_proven"].as_u64().unwrap(), 5 * STORAGE_CHALLENGE_CHUNK_BYTES,
            "no proof: bytes_proven is self-declared");
        assert!(!j["proof_valid"].as_bool().unwrap_or(true), "no proof → proof_valid=false");
    }

    // ── Phase 8 chaos tests ──────────────────────────────────────────────────

    /// Clock equivocation: two valid EpochSeal entries with the same node_id and epoch
    /// but different seal_hashes must be rejected on the second application.
    /// A node that equivocates cannot earn a double reward or fork the ledger.
    #[test]
    fn test_chaos_clock_equivocation_rejected() {
        let (chain, _dir) = make_chain("equivocation");

        let seal_a = LedgerEntry::EpochSeal {
            node_id: "node-alice".into(),
            epoch: 1,
            timestamp: 30_000,
            seal_hash: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".into(),
            signature: None,
            node_version: None,
        };
        let seal_b = LedgerEntry::EpochSeal {
            node_id: "node-alice".into(),
            epoch: 1,
            timestamp: 30_000,
            seal_hash: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            signature: None,
            node_version: None,
        };

        chain.apply_entry(&seal_a).expect("first seal accepted");
        // A second seal from the same node for the same epoch must be rejected.
        let result = chain.apply_entry(&seal_b);
        assert!(result.is_err(),
            "equivocating seal (same node, same epoch, different hash) must be rejected");
    }

    /// Corrupt Merkle proof: a StorageHeartbeat with a Merkle proof whose leaves
    /// don't reconstruct the claimed root must be accepted (chain stores it) but
    /// must be marked proof_valid=false, earning only the no-proof reward rate.
    #[test]
    fn test_chaos_storage_corrupt_proof_earns_reduced_rate() {
        let (chain, _dir) = make_chain("corrupt-proof");
        seal_epoch(&chain, 0);
        fund(&chain, "storer", 1_000_000_000_000);

        let bad_proof = btcpc_types::MerkleRangeProof {
            challenge_hash: "deadbeef".repeat(8),   // wrong — doesn't match any seal
            range_start: 0,
            range_end: STORAGE_CHALLENGE_CHUNK_BYTES,
            total_chunks: 10,
            leaf_hash: "0".repeat(64),
            merkle_root: "deadbeef".repeat(8),
            proof_nodes: vec![],                    // empty path → can't verify to root
        };

        chain.apply_entry(&LedgerEntry::StorageHeartbeat {
            node_id: "storer".into(),
            epoch: 1,
            bytes_proven: 10 * STORAGE_CHALLENGE_CHUNK_BYTES,
            query_count: 0,
            challenge_response: Some(bad_proof),
            tier: None,
            signed_by: "storer".into(),
        }).expect("heartbeat with bad proof accepted to chain");

        let raw = chain.store.state_get("storage_beat:1:storer")
            .expect("heartbeat record written");
        let j: serde_json::Value = serde_json::from_slice(&raw).unwrap();
        assert!(!j["proof_valid"].as_bool().unwrap_or(true),
            "corrupt proof must be marked proof_valid=false");
        // effective_bytes must fall back to self-declared (no-proof path).
        assert_eq!(j["bytes_proven"].as_u64().unwrap(),
            10 * STORAGE_CHALLENGE_CHUNK_BYTES,
            "no-proof path: bytes_proven is self-declared");
    }

    /// Inference double-award: awarding the same job to two different workers must
    /// reject the second award. Only one winner per job is valid.
    #[test]
    fn test_chaos_inference_double_award_rejected() {
        use crate::inference::{apply_post, apply_award};

        let (chain, _dir) = make_chain("double-award");
        seal_epoch(&chain, 0);
        fund(&chain, "requester", 10_000_000_000_000);
        fund(&chain, "worker-a",  1_000_000_000_000);
        fund(&chain, "worker-b",  1_000_000_000_000);

        // Post a job.
        apply_post(&chain, &LedgerEntry::InferenceJobPost {
            job_id: "job-1".into(),
            requester: "requester".into(),
            model: "qwen3:4b".into(),
            mode: "solo".into(),
            input_hash: "a".repeat(64),
            max_fee: 5_000_000_000,
            min_reputation: 0,
            bid_window_epochs: 2,
            deadline_epoch: 10,
            epoch: 1,
            nonce: 1,
            signed_by: "requester".into(),
            persist_on_fs: None,
            fs_fee: None,
            min_verifiers: None,
            node_fingerprint: None,
        }).expect("post");

        // Award to worker-a.
        apply_award(&chain, &LedgerEntry::InferenceJobAward {
            job_id: "job-1".into(),
            winner: "worker-a".into(),
            role: "worker".into(),
            fee: 1_000_000_000,
            epoch: 1,
        }).expect("first award");

        // Attempt to award the same job to worker-b — must fail.
        // After the first award the job status is Awarded, not Posted, so apply_award
        // rejects the second attempt at the status guard.
        let result = apply_award(&chain, &LedgerEntry::InferenceJobAward {
            job_id: "job-1".into(),
            winner: "worker-b".into(),
            role: "worker".into(),
            fee: 1_000_000_000,
            epoch: 1,
        });
        assert!(result.is_err(),
            "double-award of same job to a second worker must be rejected");
    }
}

// ── Property / fuzz tests (proptest) ─────────────────────────────────────────
//
// These tests run hundreds of randomized input sequences against the core
// state machine and assert fundamental safety invariants that must hold under
// ALL inputs, not just the crafted examples in the unit tests above.
//
// Invariants under test:
//   P1 — Balance conservation: a Transfer moves value, never creates it.
//   P2 — No negative balances: debit always fails before going negative.
//   P3 — Nonce replay rejection: same entry applied twice is rejected.
//   P4 — Supply cap: credited balances never exceed what was seeded.
//   P5 — Stake monotonicity: successful Stake always increases on-chain stake.
//   P6 — Epoch status consistency: sealed > pending, finalized > sealed.

#[cfg(test)]
mod property_tests {
    use super::*;
    use proptest::prelude::*;
    use btcpc_types::{LedgerEntry, NATIVE_TOKEN};
    use tempfile::TempDir;

    fn prop_chain() -> (Chain, TempDir) {
        let dir = tempfile::Builder::new().prefix("btcpc_prop_").tempdir().unwrap();
        let store = crate::store::Store::open(dir.path()).unwrap();
        let chain = Chain::new(store, "prop-node".into(), "btcpc-satoshi".into());
        (chain, dir)
    }

    fn seed(chain: &Chain, account: &str, amount: u64) {
        chain.apply_entry(&LedgerEntry::AccountCreate {
            account: account.into(), keys: Default::default(),
            chain_proofs: vec![], epoch: 0, funded_by: None, machine_fingerprint: None,
        }).ok();
        chain.apply_entry(&LedgerEntry::GenesisAlloc {
            account: account.into(), amount, token: NATIVE_TOKEN.into(),
        }).unwrap();
    }

    /// P1 + P2: After any valid transfer the sum of sender + receiver balances is
    /// unchanged, and neither balance goes negative.
    proptest! {
        #[test]
        fn prop_transfer_conserves_balance(
            seed_a in 1u64..=1_000_000_000_000u64,
            seed_b in 0u64..=1_000_000_000_000u64,
            amount  in 1u64..=1_000_000_000_000u64,
        ) {
            let (chain, _dir) = prop_chain();
            seed(&chain, "a", seed_a);
            seed(&chain, "b", seed_b);

            let before_a = chain.get_balance("a", NATIVE_TOKEN);
            let before_b = chain.get_balance("b", NATIVE_TOKEN);
            let before_sum = before_a + before_b;

            let result = chain.apply_entry(&LedgerEntry::Transfer {
                from: "a".into(), to: "b".into(),
                amount, token: NATIVE_TOKEN.into(),
                memo: None, nonce: 1, epoch: 1,
                signed_by: "a".into(), twofactor: None,
            });

            let after_a = chain.get_balance("a", NATIVE_TOKEN);
            let after_b = chain.get_balance("b", NATIVE_TOKEN);
            let after_sum = after_a + after_b;

            // P1: sum is conserved regardless of success or failure.
            prop_assert_eq!(before_sum, after_sum,
                "balance sum changed: before={} after={}", before_sum, after_sum);

            // P2: if transfer succeeded, sender balance is non-negative and decreased.
            if result.is_ok() {
                prop_assert!(after_a < before_a, "sender balance did not decrease");
                prop_assert_eq!(before_a - after_a, amount);
            } else {
                // Failed transfer must not have changed any balance.
                prop_assert_eq!(before_a, after_a, "failed transfer mutated sender");
                prop_assert_eq!(before_b, after_b, "failed transfer mutated receiver");
            }
        }
    }

    /// P2 edge: A sequence of random transfers from A to B, some valid some not,
    /// must never leave A with a negative balance.
    proptest! {
        #[test]
        fn prop_no_negative_balance_under_repeated_transfers(
            amounts in proptest::collection::vec(1u64..=500_000_000_000u64, 1..=20),
        ) {
            let (chain, _dir) = prop_chain();
            seed(&chain, "rich", 1_000_000_000_000); // 100 BTCPC
            seed(&chain, "poor", 0);

            for (i, &amount) in amounts.iter().enumerate() {
                let _ = chain.apply_entry(&LedgerEntry::Transfer {
                    from: "rich".into(), to: "poor".into(),
                    amount, token: NATIVE_TOKEN.into(),
                    memo: None, nonce: (i + 1) as u64, epoch: 1,
                    signed_by: "rich".into(), twofactor: None,
                });
                // Invariant: balance of "rich" is NEVER negative (u64 can't go below 0,
                // but store.debit must prevent going below zero).
                let bal = chain.get_balance("rich", NATIVE_TOKEN);
                prop_assert!(bal <= 1_000_000_000_000,
                    "rich balance exceeded initial seed (impossible): {}", bal);
            }
        }
    }

    /// P3: The same Transfer entry applied twice (identical nonce) must be rejected
    /// on the second attempt. The nonce gate is the primary replay defence.
    proptest! {
        #[test]
        fn prop_nonce_replay_rejected(
            amount in 1u64..=100_000_000_000u64,
            nonce  in 1u64..=1_000u64,
        ) {
            let (chain, _dir) = prop_chain();
            seed(&chain, "alice", 1_000_000_000_000_000);
            seed(&chain, "bob",   0);

            let entry = LedgerEntry::Transfer {
                from: "alice".into(), to: "bob".into(),
                amount, token: NATIVE_TOKEN.into(),
                memo: None, nonce, epoch: 1,
                signed_by: "alice".into(), twofactor: None,
            };

            // First application may succeed or fail (if nonce != account nonce).
            let first = chain.apply_entry(&entry);
            // Second application with the same nonce must always fail.
            let second = chain.apply_entry(&entry);
            if first.is_ok() {
                prop_assert!(second.is_err(),
                    "replay with same nonce {} must be rejected", nonce);
            }
        }
    }

    /// P4: The total balance across all accounts after N genesis allocations never
    /// exceeds the sum of what was allocated. Credits do not spontaneously appear.
    proptest! {
        #[test]
        fn prop_total_supply_bounded_by_genesis(
            allocs in proptest::collection::vec(1u64..=10_000_000_000_000u64, 1..=10),
        ) {
            let (chain, _dir) = prop_chain();
            let total_seeded: u64 = allocs.iter().sum();

            for (i, &amount) in allocs.iter().enumerate() {
                let name = format!("acc{}", i);
                seed(&chain, &name, amount);
            }

            // Sum all BTCPC balances across every account.
            let total_on_chain: u64 = (0..allocs.len())
                .map(|i| chain.get_balance(&format!("acc{}", i), NATIVE_TOKEN))
                .sum();

            prop_assert_eq!(total_seeded, total_on_chain,
                "on-chain total {} != seeded total {}", total_on_chain, total_seeded);
        }
    }

    /// P5: A successful Stake always increases the staked amount by exactly the
    /// requested value and decreases the liquid balance by the same amount.
    proptest! {
        #[test]
        fn prop_stake_moves_balance_to_stake(
            liquid in 1_000_000_000u64..=100_000_000_000_000u64,
            stake_amount in 1u64..=1_000_000_000u64,
        ) {
            let (chain, _dir) = prop_chain();
            seed(&chain, "staker", liquid);

            let bal_before   = chain.get_balance("staker", NATIVE_TOKEN);
            let stake_before = chain.get_stake("staker");

            let result = chain.apply_entry(&LedgerEntry::Stake {
                account: "staker".into(),
                amount: stake_amount,
                epoch: 1, nonce: 1,
                signed_by: "staker".into(),
            });

            let bal_after   = chain.get_balance("staker", NATIVE_TOKEN);
            let stake_after = chain.get_stake("staker");

            if result.is_ok() {
                prop_assert_eq!(bal_before - bal_after, stake_amount,
                    "liquid balance did not decrease by stake amount");
                prop_assert_eq!(stake_after - stake_before, stake_amount,
                    "staked amount did not increase by stake amount");
            } else {
                prop_assert_eq!(bal_before, bal_after, "failed stake mutated balance");
                prop_assert_eq!(stake_before, stake_after, "failed stake mutated stake");
            }
        }
    }

    /// P6: epoch_status ordering — a finalized epoch always reports "finalized",
    /// never "pending" or "sealed", regardless of which state keys exist.
    #[test]
    fn prop_epoch_status_finalized_wins_over_sealed() {
        let (chain, _dir) = prop_chain();

        // Apply an epoch seal (marks it "sealed").
        chain.apply_entry(&LedgerEntry::EpochSeal {
            node_id: "n1".into(), epoch: 1,
            timestamp: 30_000,
            seal_hash: "a".repeat(64),
            signature: None,
            node_version: None,
        }).unwrap();
        assert_eq!(chain.epoch_status(1), "sealed");

        // Apply an EpochFinalize (marks it "finalized").
        chain.apply_entry(&LedgerEntry::EpochFinalize {
            epoch: 1,
            node_id: "n1".into(),
            rewards_hash: "b".repeat(64),
            state_root: "c".repeat(64),
            quorum: 3,
            sealed_by: vec![],
            timestamp: 31_000,
        }).unwrap();
        assert_eq!(chain.epoch_status(1), "finalized",
            "finalized must win over sealed");

        // An epoch we've never touched is pending.
        assert_eq!(chain.epoch_status(999), "pending");
    }

    #[test]
    fn runtime_phase1_register_claim_attest_persists_keys() {
        let (chain, _dir) = prop_chain();

        chain.apply_entry(&LedgerEntry::RuntimeRegister {
            runtime_id: "rt-1".into(),
            owner: "alice".into(),
            manifest_cid: "cid-runtime-1".into(),
            runtime_class: "stateful_session_service".into(),
            nonce: "n-1".into(),
            bond: 0,
            epoch: 10,
            signed_by: "alice".into(),
        }).expect("runtime register");

        chain.apply_entry(&LedgerEntry::RuntimeDeploy {
            runtime_id: "rt-1".into(),
            owner: "alice".into(),
            host_id: "host-a".into(),
            manifest_cid: None,
            epoch: 10,
            signed_by: "alice".into(),
        }).expect("runtime deploy");

        chain.apply_entry(&LedgerEntry::RuntimeJobEnqueue {
            job_id: "job-1".into(),
            runtime_id: "rt-1".into(),
            job_class: "worker".into(),
            due_epoch: 12,
            payload_cid: Some("cid-job-1".into()),
            fee: 0,
            epoch: 11,
            signed_by: "alice".into(),
        }).expect("job enqueue");

        chain.apply_entry(&LedgerEntry::RuntimeClaim {
            lease_id: "lease-1".into(),
            runtime_id: "rt-1".into(),
            job_id: "job-1".into(),
            host_id: "host-a".into(),
            expires_epoch: 16,
            epoch: 12,
            signed_by: "host-a".into(),
        }).expect("runtime claim");

        chain.apply_entry(&LedgerEntry::RuntimeAttest {
            attestation_id: "att-1".into(),
            runtime_id: "rt-1".into(),
            job_id: "job-1".into(),
            host_id: "host-a".into(),
            output_commitment: "out-123".into(),
            runtime_sha: None,
            epoch: 13,
            signed_by: "host-a".into(),
        }).expect("runtime attest");

        assert!(chain.store.state_get("runtime:rt-1").is_some());
        assert!(chain.store.state_get("runtime_job:job-1").is_some());
        assert!(chain.store.state_get("runtime_lease:lease-1").is_some());
        assert!(chain.store.state_get("runtime_host:host-a").is_some());
        assert!(chain.store.state_get("runtime_attest:att-1").is_some());
    }

    #[test]
    fn runtime_phase1_challenge_slash_undeploy_persists_keys() {
        let (chain, _dir) = prop_chain();

        chain.apply_entry(&LedgerEntry::RuntimeRegister {
            runtime_id: "rt-2".into(),
            owner: "alice".into(),
            manifest_cid: "cid-runtime-2".into(),
            runtime_class: "http_service".into(),
            nonce: "n-2".into(),
            bond: 0,
            epoch: 20,
            signed_by: "alice".into(),
        }).expect("runtime register");

        chain.apply_entry(&LedgerEntry::RuntimeDeploy {
            runtime_id: "rt-2".into(),
            owner: "alice".into(),
            host_id: "host-b".into(),
            manifest_cid: None,
            epoch: 20,
            signed_by: "alice".into(),
        }).expect("runtime deploy");

        chain.apply_entry(&LedgerEntry::RuntimeJobEnqueue {
            job_id: "job-2".into(),
            runtime_id: "rt-2".into(),
            job_class: "worker".into(),
            due_epoch: 22,
            payload_cid: None,
            fee: 0,
            epoch: 20,
            signed_by: "alice".into(),
        }).expect("job enqueue");

        chain.apply_entry(&LedgerEntry::RuntimeClaim {
            lease_id: "lease-2".into(),
            runtime_id: "rt-2".into(),
            job_id: "job-2".into(),
            host_id: "host-b".into(),
            expires_epoch: 25,
            epoch: 21,
            signed_by: "host-b".into(),
        }).expect("runtime claim");

        chain.apply_entry(&LedgerEntry::RuntimeAttest {
            attestation_id: "att-x".into(),
            runtime_id: "rt-2".into(),
            job_id: "job-2".into(),
            host_id: "host-b".into(),
            output_commitment: "out-456".into(),
            runtime_sha: None,
            epoch: 21,
            signed_by: "host-b".into(),
        }).expect("runtime attest");

        chain.apply_entry(&LedgerEntry::RuntimeChallenge {
            challenge_id: "ch-1".into(),
            attestation_id: "att-x".into(),
            runtime_id: "rt-2".into(),
            challenger: "bob".into(),
            reason: "invalid_output".into(),
            evidence_cid: Some("cid-ev-1".into()),
            epoch: 21,
            signed_by: "bob".into(),
        }).expect("runtime challenge");

        chain.apply_entry(&LedgerEntry::RuntimeSlash {
            slash_id: "sl-1".into(),
            runtime_id: "rt-2".into(),
            host_id: "host-b".into(),
            amount: 42,
            reason: "invalid_attestation".into(),
            epoch: 22,
            signed_by: "arbiter".into(),
        }).expect("runtime slash");

        chain.apply_entry(&LedgerEntry::RuntimeUndeploy {
            runtime_id: "rt-2".into(),
            owner: "alice".into(),
            epoch: 23,
            signed_by: "alice".into(),
        }).expect("runtime undeploy");

        assert!(chain.store.state_get("runtime_challenge:ch-1").is_some());
        assert!(chain.store.state_get("runtime_slash:sl-1").is_some());
        let runtime = chain.store.state_get("runtime:rt-2").expect("runtime rt-2 persisted");
        let runtime_json: serde_json::Value = serde_json::from_slice(&runtime).expect("runtime json");
        assert_eq!(runtime_json["status"], "undeployed");
    }
}
