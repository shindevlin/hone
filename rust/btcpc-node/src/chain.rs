//! Chain state machine — applies ledger entries and advances chain state.

use std::sync::Arc;
use anyhow::{Context, Result};
use parking_lot::{Mutex, RwLock};
use sha2::{Sha256, Digest as Sha256Digest};
use tracing::{info, warn};
use btcpc_types::{AccountId, LedgerEntry, NATIVE_TOKEN, CLOCK_REWARD_DREAMS, era, RECYCLE_ERA, RECYCLE_FUND_ACCOUNT, TESTNET_FUND_ACCOUNT, DEVICE_CLAIM_OVERBID_NUM, DEVICE_CLAIM_OVERBID_DENOM, OVERCLAIM_STAKER_SHARE_BPS};

use crate::inference;
use crate::store::Store;

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
        other => anyhow::bail!("unsupported sig_type '{}' — supported: eth_personal_sign", other),
    }
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
        }
    }

    /// Queue a user entry for application at the next epoch seal.
    pub fn push_pending(&self, entry: LedgerEntry, sig: Option<String>) {
        self.pending.lock().push((entry, sig));
    }

    /// Drain and sort the pending pool, returning entries in deterministic hash order.
    /// All nodes that received the same gossip will sort identically, ensuring consistent
    /// "first claim wins" across the whole network regardless of gossip arrival timing.
    pub fn drain_pending_sorted(&self) -> Vec<(LedgerEntry, Option<String>)> {
        let mut pool = self.pending.lock();
        pool.sort_by_cached_key(|(e, _)| {
            let bytes = serde_json::to_vec(e).unwrap_or_default();
            let mut h = Sha256::new();
            h.update(&bytes);
            h.finalize().to_vec()
        });
        pool.drain(..).collect()
    }

    pub fn current_epoch(&self) -> u64 {
        *self.current_epoch.read()
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
        use std::collections::BTreeMap;
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
            LedgerEntry::InferenceJobPay { worker, verifier_payments, .. } => {
                touched.push((worker.clone(), "inference_fee"));
                for (v, _) in verifier_payments {
                    touched.push((v.clone(), "verifier_fee"));
                }
            }
            LedgerEntry::Stake { account, .. } => touched.push((account.clone(), "stake")),
            LedgerEntry::Unstake { account, .. } => touched.push((account.clone(), "unstake")),
            _ => {}
        }

        if touched.is_empty() { return; }

        // Derive entry type from the first key of the JSON object.
        let entry_json = serde_json::to_value(entry).unwrap_or_default();
        let entry_type = entry_json.as_object()
            .and_then(|m| m.keys().next())
            .map(String::as_str)
            .unwrap_or("unknown");

        let record = serde_json::to_vec(&serde_json::json!({
            "epoch": epoch,
            "type": entry_type,
            "entry": entry,
        })).unwrap_or_default();

        for (account, role) in touched {
            let key = format!("txhist:{}:{:016x}:{}:{}", account, epoch, role, key_prefix);
            let _ = self.store.state_set(&key, &record);
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

            LedgerEntry::AccountCreate { account, keys, chain_proofs, epoch, funded_by } => {
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

                let state = serde_json::json!({
                    "account_id": account,
                    "created_epoch": epoch,
                    "keys": keys,
                    "chain_proofs": proofs_json,
                    "nonce": 0,
                    "stake": 0,
                    "name_stake_locked": btcpc_types::NAME_REGISTRATION_STAKE,
                });
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
                self.store.set_stake(account, current_stake - amount)?;
                self.store.credit(account, NATIVE_TOKEN, *amount)?;
            }

            LedgerEntry::Mine { miner, epoch, model, input_tokens, output_tokens, tool_calls, hw_tier, compute_proof } => {
                self.touch_alive(miner, *epoch);
                self.ensure_account(miner, *epoch)?;
                let key = format!("mine:{}:{}", epoch, miner);
                let _ = self.store.state_set(&key,
                    &serde_json::to_vec(&serde_json::json!({
                        "miner": miner, "epoch": epoch,
                        "model": model, "input_tokens": input_tokens,
                        "output_tokens": output_tokens, "tool_calls": tool_calls,
                        "hw_tier": hw_tier, "compute_proof": compute_proof,
                    })).unwrap_or_default());
            }

            LedgerEntry::MineReward { miner, amount, epoch } => {
                self.ensure_account(miner, *epoch)?;
                if era(*epoch) >= RECYCLE_ERA {
                    // Era 5+: transfer from recycle fund rather than minting.
                    self.store.debit(RECYCLE_FUND_ACCOUNT, NATIVE_TOKEN, *amount)
                        .map_err(|e| anyhow::anyhow!("recycle fund insufficient for era-5 reward: {}", e))?;
                }
                self.store.credit(miner, NATIVE_TOKEN, *amount)?;
                info!("mine reward: {} dreams → {} (epoch {})", amount, miner, epoch);
            }

            LedgerEntry::EpochSeal { node_id: _, epoch, .. } => {
                let ep = *epoch as u64;
                let mut current = self.current_epoch.write();
                if ep > *current {
                    *current = ep;
                    // Persist so restarts recover the correct epoch even if no block was produced.
                    let _ = self.store.set_meta("current_epoch", &ep.to_le_bytes());
                }
            }

            LedgerEntry::EpochFinalize { epoch, rewards_hash, quorum, state_root, timestamp, .. } => {
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

            LedgerEntry::SensorReading { .. }
            | LedgerEntry::BlobStore { .. }
            | LedgerEntry::ContractDeploy { .. }
            | LedgerEntry::ContractCall { .. } => {
                // Accepted, no balance mutations needed at base layer
            }

            // Track storage heartbeats so the clock can compute storage rewards at seal time.
            LedgerEntry::StorageHeartbeat { node_id, epoch, bytes_proven, query_count, .. } => {
                let key = format!("storage_beat:{}:{}", epoch, node_id);
                let _ = self.store.state_set(&key,
                    &serde_json::to_vec(&serde_json::json!({
                        "node_id": node_id, "epoch": epoch,
                        "bytes_proven": bytes_proven, "query_count": query_count,
                    })).unwrap_or_default());
            }

            // Track sensor commits per sensor_id (not per owner) so reward scoring
            // can apply type-aware sensor_score() per individual sensor.
            LedgerEntry::SensorDataCommit { sensor_id, owner, epoch, reading_count, sensor_type, .. } => {
                let key = format!("sensor_commit:{}:{}", epoch, sensor_id);
                let _ = self.store.state_set(&key,
                    &serde_json::to_vec(&serde_json::json!({
                        "sensor_id": sensor_id, "owner": owner, "epoch": epoch,
                        "reading_count": reading_count, "sensor_type": sensor_type,
                    })).unwrap_or_default());
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
            }
            LedgerEntry::LinkGitRefUpdate { repo_id, ref_name, commit_hash, .. } => {
                let key = format!("linkgit:repo:{}", repo_id);
                if let Some(bytes) = self.store.get_meta(&key) {
                    if let Ok(mut repo) = serde_json::from_slice::<serde_json::Value>(&bytes) {
                        repo["refs"][ref_name] = serde_json::json!(commit_hash);
                        self.store.set_meta(&key, repo.to_string().as_bytes())?;
                    }
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
                let _ = CLOCK_REWARD_DREAMS; // used by caller to size per-node amounts
                self.store.credit(node_id, NATIVE_TOKEN, *amount)?;
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
                self.store.credit(owner, NATIVE_TOKEN, stake_amount)?;
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
                    rec["status"]         = serde_json::json!("AcousticVerified");
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

        }

        self.index_tx_history(entry);
        Ok(())
    }

    /// Apply a batch of entries from a block payload.
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
}
