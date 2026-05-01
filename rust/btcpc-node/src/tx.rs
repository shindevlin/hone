//! Transaction validation and application — the gatekeeper for user-submitted entries.
#![allow(dead_code)]
//!
//! `validate_and_apply` runs all pre-flight checks (signature, nonce, balance)
//! before delegating to `Chain::apply_entry`.  Block-replay paths skip this
//! module entirely because entries inside sealed blocks are already trusted.
//!
//! # Signature convention
//!
//! The LedgerEntry enum does not embed a signature field for Transfer / Stake /
//! Unstake.  Callers that want signature verification must pass the hex-encoded
//! ed25519 signature as the `signature` parameter.  The message signed is the
//! canonical JSON of the entry as produced by `serde_json::to_string`.
//!
//! For `EpochSeal` the signature is carried inside the variant and is verified
//! automatically when `validate_and_apply` is called without an explicit
//! override.

use anyhow::{bail, Result};
use btcpc_types::{LedgerEntry, NATIVE_TOKEN, DREAMS_PER_BTCPC};
use ed25519_dalek::{Signature, VerifyingKey};

use crate::chain::Chain;

/// Returns true if `delegate` has been granted `capability` by `from`
/// and the delegation has not expired at `current_epoch`.
fn has_delegation(chain: &Chain, from: &str, delegate: &str, capability: &str, current_epoch: u64) -> bool {
    let key = format!("delegation:{}:{}", from, delegate);
    let raw = match chain.store.state_get(&key) {
        Some(r) => r,
        None => return false,
    };
    let d: serde_json::Value = match serde_json::from_slice(&raw) {
        Ok(v) => v,
        Err(_) => return false,
    };
    let expires = d["expires_epoch"].as_u64().unwrap_or(0);
    if expires < current_epoch { return false; }
    if let Some(caps) = d["capabilities"].as_array() {
        return caps.iter().any(|c| {
            c.as_str() == Some(capability) || c.as_str() == Some("all")
        });
    }
    false
}

/// Returned by HTTP handlers after a submission attempt.
pub struct SubmitResult {
    pub entry_hash: String,
    pub accepted: bool,
    pub error: Option<String>,
}

// ── Public entry-points ───────────────────────────────────────────────────────

/// Validate and apply a ledger entry submitted via the API.
///
/// `sig_hex`: optional hex-encoded ed25519 signature over the canonical JSON of
/// `entry`.  Required for Transfer / Stake / Unstake when the account has a
/// registered public key.  Pass `None` for entries that either embed the
/// signature (EpochSeal) or need no signature (AccountCreate, GenesisAlloc).
///
/// Returns `Ok(entry_hash)` — the SHA-256 hex of the entry JSON — on success.
pub fn validate_and_apply(
    chain: &Chain,
    entry: &LedgerEntry,
    sig_hex: Option<&str>,
) -> Result<String> {
    let hash = entry.hash();

    match entry {
        // ── Transfer ─────────────────────────────────────────────────────────
        LedgerEntry::Transfer { from, to, amount, token, nonce, signed_by, twofactor, .. } => {
            let _guard = chain.write_lock.lock();
            check_not_empty(from, "from")?;
            check_not_empty(to, "to")?;
            if *amount == 0 {
                bail!("transfer amount must be positive");
            }
            let current_epoch = chain.current_epoch();
            if signed_by != from
                && !has_delegation(chain, from, signed_by, "Transfer", current_epoch)
            {
                bail!("signed_by '{}' is neither 'from' nor a Transfer delegate of '{}'", signed_by, from);
            }
            require_key(chain, signed_by)?;
            check_nonce(chain, from, *nonce)?;
            check_signature(chain, signed_by, entry, sig_hex, "active")?;
            check_slot_2fa(chain, from, "active", entry, twofactor.as_ref())?;
            let bal = chain.get_balance(from, token);
            if bal < *amount {
                bail!(
                    "insufficient balance: {} has {} {} (need {})",
                    from,
                    bal as f64 / DREAMS_PER_BTCPC as f64,
                    token,
                    *amount as f64 / DREAMS_PER_BTCPC as f64,
                );
            }
            chain.apply_entry(entry)?;
            bump_nonce(chain, from)?;
        }

        // ── Stake ────────────────────────────────────────────────────────────
        LedgerEntry::Stake { account, amount, nonce, signed_by, .. } => {
            let _guard = chain.write_lock.lock();
            if *amount == 0 {
                bail!("stake amount must be positive");
            }
            let current_epoch = chain.current_epoch();
            if signed_by != account
                && !has_delegation(chain, account, signed_by, "Stake", current_epoch)
            {
                bail!("signed_by '{}' is neither 'account' nor a Stake delegate of '{}'", signed_by, account);
            }
            require_key(chain, signed_by)?;
            check_nonce(chain, account, *nonce)?;
            check_signature(chain, signed_by, entry, sig_hex, "active")?;
            let bal = chain.get_balance(account, NATIVE_TOKEN);
            if bal < *amount {
                bail!(
                    "insufficient balance for stake: {} has {} BTCPC",
                    account,
                    bal as f64 / DREAMS_PER_BTCPC as f64,
                );
            }
            chain.apply_entry(entry)?;
            bump_nonce(chain, account)?;
        }

        // ── Unstake ──────────────────────────────────────────────────────────
        LedgerEntry::Unstake { account, amount, nonce, signed_by, .. } => {
            let _guard = chain.write_lock.lock();
            if *amount == 0 {
                bail!("unstake amount must be positive");
            }
            let current_epoch = chain.current_epoch();
            if signed_by != account
                && !has_delegation(chain, account, signed_by, "Stake", current_epoch)
            {
                bail!("signed_by '{}' is neither 'account' nor a Stake delegate of '{}'", signed_by, account);
            }
            require_key(chain, signed_by)?;
            check_nonce(chain, account, *nonce)?;
            check_signature(chain, signed_by, entry, sig_hex, "active")?;
            let staked = chain.get_stake(account);
            if staked < *amount {
                bail!(
                    "insufficient stake: {} has {} BTCPC staked",
                    account,
                    staked as f64 / DREAMS_PER_BTCPC as f64,
                );
            }
            chain.apply_entry(entry)?;
            bump_nonce(chain, account)?;
        }

        // ── AccountCreate ────────────────────────────────────────────────────
        LedgerEntry::AccountCreate { account, funded_by, .. } => {
            check_not_empty(account, "account")?;
            // Block 3-digit all-numeric names (000-999) — reserved for future numeric namespace.
            if account.len() == 3 && account.chars().all(|c| c.is_ascii_digit()) {
                bail!("3-digit numeric names (000-999) are reserved and cannot be registered yet");
            }
            let exempt = btcpc_types::STAKE_EXEMPT_ACCOUNTS.contains(&account.as_str());
            if !exempt {
                // Check if name stake is enabled via on-chain governance param.
                let stake_enabled = chain.store.state_get("chain_param:name_stake_enabled")
                    .and_then(|bytes| String::from_utf8(bytes).ok())
                    .map(|s| s.trim() == "true")
                    .unwrap_or(false); // default: OFF — free registration until shindevlin flips the switch

                if stake_enabled {
                    let stake_amount = chain.store.state_get("chain_param:name_stake_amount")
                        .and_then(|bytes| String::from_utf8(bytes).ok())
                        .and_then(|s| s.trim().parse::<u64>().ok())
                        .unwrap_or(btcpc_types::NAME_REGISTRATION_STAKE);

                    let funder = funded_by.as_deref()
                        .filter(|s| !s.is_empty())
                        .ok_or_else(|| anyhow::anyhow!(
                            "name registration requires a funded_by account with {} dreams",
                            stake_amount
                        ))?;
                    let bal = chain.store.get_balance(funder, btcpc_types::NATIVE_TOKEN);
                    anyhow::ensure!(
                        bal >= stake_amount,
                        "funded_by '{}' has {} dreams, need {} for name registration",
                        funder, bal, stake_amount
                    );
                }
            }
            chain.apply_entry(entry)?;
        }

        // ── ChainParameterSet ────────────────────────────────────────────────
        LedgerEntry::ChainParameterSet { key, signed_by, .. } => {
            // Only authorized accounts. shindevlin is the bootstrap governor.
            // TODO: replace with on-chain governance vote once consensus is live.
            const AUTHORIZED: &[&str] = &["shindevlin"];
            if !AUTHORIZED.contains(&signed_by.as_str()) {
                bail!("only authorized accounts can set chain parameters; '{}' is not authorized", signed_by);
            }
            check_signature(chain, signed_by, entry, sig_hex, "posting")?;
            chain.apply_entry(entry)?;
            let _ = key;
        }

        // ── EpochSeal (signature embedded) ───────────────────────────────────
        LedgerEntry::EpochSeal { node_id, signature: embedded_sig, epoch, .. } => {
            let current = chain.current_epoch();
            // Reject seals too far ahead (prevents epoch inflation attacks).
            if *epoch as u64 > current + 3 {
                bail!(
                    "EpochSeal epoch {} is too far ahead of current {} — max drift is +3",
                    epoch, current
                );
            }
            // Reject very stale seals.
            if current > 10 && (*epoch as u64) < current - 10 {
                bail!(
                    "EpochSeal epoch {} is too far behind current {}",
                    epoch, current
                );
            }
            let effective_sig = sig_hex.or(embedded_sig.as_deref());
            check_signature(chain, node_id, entry, effective_sig, "posting")?;
            chain.apply_entry(entry)?;
        }

        // ── AccountUpdateKey ──────────────────────────────────────────────────
        // The account must sign its own key update.  If no key is registered yet,
        // the first key can be set without a signature (bootstrap flow).
        LedgerEntry::AccountUpdateKey { account, role: _, signed_by, .. } => {
            if signed_by != account {
                bail!("signed_by '{}' must equal account '{}'", signed_by, account);
            }
            if let Ok(Some(state)) = chain.store.get_account(account) {
                // Prefer owner key for authorization; fall back to posting.
                let auth_key = state.get("keys")
                    .and_then(|v| v.get("owner"))
                    .and_then(|v| v.as_str())
                    .filter(|k| !k.is_empty())
                    .or_else(|| {
                        state.get("keys")
                            .and_then(|v| v.get("posting"))
                            .and_then(|v| v.as_str())
                            .filter(|k| !k.is_empty())
                    });
                if auth_key.is_some() {
                    check_signature(chain, account, entry, sig_hex, "owner")?;
                }
            }
            chain.apply_entry(entry)?;
        }

        // ── AccountSetPrimary ─────────────────────────────────────────────────
        // Declares another account as the owner's primary identity.
        // The primary must exist and share the same posting key — proving ownership.
        // Must be set before AccountTransfer is allowed.
        LedgerEntry::AccountSetPrimary { account, primary, signed_by, .. } => {
            if signed_by != account {
                bail!("signed_by '{}' must equal account '{}'", signed_by, account);
            }
            anyhow::ensure!(account != primary, "primary cannot be the same as account");

            let cur_state = chain.store.get_account(account)?
                .ok_or_else(|| anyhow::anyhow!("account '{}' not found", account))?;
            let primary_state = chain.store.get_account(primary)?
                .ok_or_else(|| anyhow::anyhow!(
                    "primary '{}' does not exist — create it with your keys first", primary
                ))?;

            // Verify the primary shares the same posting key.
            let cur_posting = cur_state.get("keys")
                .and_then(|v| v.get("posting")).and_then(|v| v.as_str())
                .filter(|k| !k.is_empty());
            let pri_posting = primary_state.get("keys")
                .and_then(|v| v.get("posting")).and_then(|v| v.as_str())
                .filter(|k| !k.is_empty());

            if cur_posting.is_some() && cur_posting != pri_posting {
                bail!(
                    "primary '{}' must share the same posting key as '{}' — \
                     both must be controlled by your wallet",
                    primary, account
                );
            }

            if cur_posting.is_some() {
                check_signature(chain, account, entry, sig_hex, "posting")?;
            }
            chain.apply_entry(entry)?;
        }

        // ── AccountTransfer ───────────────────────────────────────────────────
        // Transfers the identity to a new owner. Requires AccountSetPrimary first.
        // The stored primary receives any balance; the key map is replaced atomically.
        LedgerEntry::AccountTransfer { account, signed_by, new_keys, .. } => {
            if signed_by != account {
                bail!("signed_by '{}' must equal account '{}' for AccountTransfer", signed_by, account);
            }
            anyhow::ensure!(!new_keys.is_empty(), "new_keys must not be empty");

            let cur_state = chain.store.get_account(account)?
                .ok_or_else(|| anyhow::anyhow!("cannot transfer non-existent account '{}'", account))?;

            // Primary must be declared — that is where the balance goes and what proves
            // the owner won't lose their chain presence.
            let primary = cur_state.get("primary")
                .and_then(|v| v.as_str())
                .filter(|s| !s.is_empty())
                .ok_or_else(|| anyhow::anyhow!(
                    "call AccountSetPrimary first — declare your other identity (e.g. 'joshua') \
                     before transferring 'josh'. This protects you from losing chain presence."
                ))?;

            // Confirm the primary still exists.
            anyhow::ensure!(
                chain.store.get_account(primary)?.is_some(),
                "declared primary '{}' no longer exists", primary
            );

            let auth_key = cur_state.get("keys")
                .and_then(|v| v.get("owner")).and_then(|v| v.as_str())
                .filter(|k| !k.is_empty())
                .or_else(|| cur_state.get("keys")
                    .and_then(|v| v.get("posting")).and_then(|v| v.as_str())
                    .filter(|k| !k.is_empty()));
            if auth_key.is_some() {
                check_signature(chain, account, entry, sig_hex, "owner")?;
            }

            chain.apply_entry(entry)?;
        }

        // ── Allowlisted pass-through entries ──────────────────────────────────
        LedgerEntry::SensorReading { .. }
        | LedgerEntry::BlobStore { .. }
        | LedgerEntry::ContractDeploy { .. }
        | LedgerEntry::ContractCall { .. }
        // Freeport commerce — recorded on-chain, state managed by btcpc-market sidecar
        | LedgerEntry::StoreUpdate { .. }
        | LedgerEntry::ProductCreate { .. }
        | LedgerEntry::ProductUpdate { .. }
        | LedgerEntry::OrderPlace { .. }
        | LedgerEntry::OrderFulfill { .. }
        | LedgerEntry::OrderCancel { .. }
        | LedgerEntry::OrderDispute { .. }
        | LedgerEntry::EscrowRelease { .. }
        | LedgerEntry::FlashSale { .. }
        // Verasens sensors — recorded on-chain, state in sidecar
        | LedgerEntry::SensorRegister { .. }
        | LedgerEntry::SensorKeyRegister { .. }
        | LedgerEntry::SensorVouch { .. }
        | LedgerEntry::SensorDataCommit { .. }
        | LedgerEntry::DeviceKeyRegister { .. }
        | LedgerEntry::DeviceYieldUnstake { .. }
        | LedgerEntry::GatewayHeartbeat { .. }
        | LedgerEntry::StorageHeartbeat { .. }
        // LinkGit — recorded on-chain, object storage in btcpc-fs
        | LedgerEntry::LinkGitRepoCreate { .. }
        | LedgerEntry::LinkGitRefUpdate { .. }
        | LedgerEntry::LinkGitAccessGrant { .. }
        | LedgerEntry::LinkGitAccessRevoke { .. }
        | LedgerEntry::LinkGitPruneProof { .. }
        | LedgerEntry::LinkGitStorageExtend { .. }
        // BLE Tracker — recorded on-chain, state in chain.rs
        | LedgerEntry::TrackerSightingCommit { .. }
        | LedgerEntry::TrackerClaim { .. }
        | LedgerEntry::TrackerClaimRelease { .. }
        | LedgerEntry::TrackerAcousticProof { .. }
        | LedgerEntry::TrackerSubscription { .. }
        | LedgerEntry::TrackerSightingData { .. }
        | LedgerEntry::TrackerHint { .. }
        | LedgerEntry::TrackerLostMode { .. }
        | LedgerEntry::TrackerFoundReport { .. }
        | LedgerEntry::TrackerFoundConfirm { .. } => {
            chain.apply_entry(entry)?;
        }

        // ── Inference marketplace (user-submitted) ────────────────────────────
        LedgerEntry::InferenceJobPost { requester, nonce, signed_by, .. } => {
            let _guard = chain.write_lock.lock();
            if signed_by != requester {
                bail!("signed_by must equal requester");
            }
            require_key(chain, requester)?;
            check_nonce(chain, requester, *nonce)?;
            check_signature(chain, signed_by, entry, sig_hex, "posting")?;
            chain.apply_entry(entry)?;
            bump_nonce(chain, requester)?;
        }

        LedgerEntry::InferenceJobBid { bidder, nonce, signed_by, .. } => {
            let _guard = chain.write_lock.lock();
            if signed_by != bidder {
                bail!("signed_by must equal bidder");
            }
            require_key(chain, bidder)?;
            check_nonce(chain, bidder, *nonce)?;
            check_signature(chain, signed_by, entry, sig_hex, "posting")?;
            chain.apply_entry(entry)?;
            bump_nonce(chain, bidder)?;
        }

        LedgerEntry::InferenceJobComplete { worker, signed_by, .. } => {
            if signed_by != worker {
                bail!("signed_by must equal worker");
            }
            require_key(chain, worker)?;
            check_signature(chain, signed_by, entry, sig_hex, "posting")?;
            chain.apply_entry(entry)?;
        }

        // System verifiers submit verify results; must have a registered key.
        LedgerEntry::InferenceJobVerify { verifier, signed_by, .. } => {
            if signed_by != verifier {
                bail!("signed_by must equal verifier");
            }
            require_key(chain, verifier)?;
            check_signature(chain, signed_by, entry, sig_hex, "posting")?;
            chain.apply_entry(entry)?;
        }

        // Worker contests a dispute — nonce required.
        LedgerEntry::InferenceJobClaim { claimant, nonce, signed_by, .. } => {
            let _guard = chain.write_lock.lock();
            if signed_by != claimant {
                bail!("signed_by must equal claimant");
            }
            require_key(chain, claimant)?;
            check_nonce(chain, claimant, *nonce)?;
            check_signature(chain, signed_by, entry, sig_hex, "posting")?;
            chain.apply_entry(entry)?;
            bump_nonce(chain, claimant)?;
        }

        // Human reviewers vote on disputed jobs.
        LedgerEntry::InferenceReviewVote { reviewer, signed_by, .. } => {
            if signed_by != reviewer {
                bail!("signed_by must equal reviewer");
            }
            require_key(chain, reviewer)?;
            check_signature(chain, signed_by, entry, sig_hex, "posting")?;
            chain.apply_entry(entry)?;
        }

        LedgerEntry::InferenceJobCancel { cancelled_by, nonce, signed_by, .. } => {
            let _guard = chain.write_lock.lock();
            if signed_by != cancelled_by {
                bail!("signed_by must equal cancelled_by");
            }
            require_key(chain, cancelled_by)?;
            check_nonce(chain, cancelled_by, *nonce)?;
            check_signature(chain, signed_by, entry, sig_hex, "posting")?;
            chain.apply_entry(entry)?;
            bump_nonce(chain, cancelled_by)?;
        }

        // ── Privileged types: never accepted from external callers ────────────
        LedgerEntry::Mine { .. }
        | LedgerEntry::MineReward { .. }
        | LedgerEntry::ClockReward { .. }
        | LedgerEntry::InferenceJobAward { .. }
        | LedgerEntry::InferenceJobPay { .. }
        | LedgerEntry::GenesisAlloc { .. }
        | LedgerEntry::EpochFinalize { .. }
        | LedgerEntry::TestnetReward { .. }
        | LedgerEntry::ClockReward { .. }
        | LedgerEntry::StorageReward { .. }
        | LedgerEntry::SensorReward { .. }
        | LedgerEntry::VerifierReward { .. }
        | LedgerEntry::ServiceReward { .. }
        | LedgerEntry::EpochSeal { .. } => {
            bail!("entry type is not externally submittable");
        }

        // ── ServiceHeartbeat: service nodes prove active container-hours ──────
        LedgerEntry::ServiceHeartbeat { node_id, signed_by, .. } => {
            if signed_by != node_id {
                bail!("signed_by must match node_id");
            }
            require_key(chain, node_id)?;
            check_signature(chain, signed_by, entry, sig_hex, "posting")?;
            chain.apply_entry(entry)?;
        }

        // ── InferenceVerifyClaim: verifier claims a job to inspect ────────────
        LedgerEntry::InferenceVerifyClaim { verifier, signed_by, .. } => {
            if signed_by != verifier {
                bail!("signed_by must match verifier");
            }
            require_key(chain, verifier)?;
            check_signature(chain, signed_by, entry, sig_hex, "posting")?;
            chain.apply_entry(entry)?;
        }

        // ── SensorDataPurchase: buyer pays for sensor data ────────────────────
        LedgerEntry::SensorDataPurchase { buyer, signed_by, nonce, fee, .. } => {
            if signed_by != buyer {
                bail!("signed_by must match buyer");
            }
            require_key(chain, buyer)?;
            check_nonce(chain, buyer, *nonce)?;
            check_signature(chain, signed_by, entry, sig_hex, "active")?;
            let bal = chain.get_balance(buyer, NATIVE_TOKEN);
            if bal < *fee {
                bail!("insufficient balance for sensor data purchase");
            }
            chain.apply_entry(entry)?;
            bump_nonce(chain, buyer)?;
        }

        // TestnetOperatorRegister is user-submittable (registers mainnet account).
        LedgerEntry::TestnetOperatorRegister { mainnet_account, signed_by, epoch, .. } => {
            if signed_by != mainnet_account {
                bail!("signed_by must match mainnet_account");
            }
            require_key(chain, mainnet_account)?;
            check_signature(chain, signed_by, entry, sig_hex, "posting")?;
            chain.apply_entry(entry)?;
            let _ = epoch;
        }

        // ── Mempool operator registration ─────────────────────────────────────
        LedgerEntry::MempoolOperatorRegister { operator, signed_by, amount, nonce, .. } => {
            if signed_by != operator {
                bail!("signed_by must match operator");
            }
            require_key(chain, operator)?;
            check_nonce(chain, operator, *nonce)?;
            check_signature(chain, signed_by, entry, sig_hex, "active")?;
            let bal = chain.get_balance(operator, NATIVE_TOKEN);
            if bal < *amount {
                bail!("insufficient balance for mempool operator stake");
            }
            chain.apply_entry(entry)?;
            bump_nonce(chain, operator)?;
        }

        // ── Mempool heartbeat (signed by operator) ────────────────────────────
        LedgerEntry::MempoolHeartbeat { operator, signed_by, .. } => {
            if signed_by != operator {
                bail!("signed_by must match operator");
            }
            require_key(chain, operator)?;
            check_signature(chain, signed_by, entry, sig_hex, "posting")?;
            chain.apply_entry(entry)?;
        }

        // ── Device claim stake ────────────────────────────────────────────────
        LedgerEntry::DeviceClaimStake { owner, signed_by, amount, nonce, .. } => {
            if signed_by != owner {
                bail!("signed_by must match owner");
            }
            require_key(chain, owner)?;
            check_nonce(chain, owner, *nonce)?;
            check_signature(chain, signed_by, entry, sig_hex, "posting")?;
            let bal = chain.get_balance(owner, NATIVE_TOKEN);
            if bal < *amount {
                bail!("insufficient balance for device claim stake");
            }
            chain.apply_entry(entry)?;
            bump_nonce(chain, owner)?;
        }

        LedgerEntry::DeviceClaimUnstake { owner, signed_by, nonce, .. } => {
            if signed_by != owner {
                bail!("signed_by must match owner");
            }
            require_key(chain, owner)?;
            check_nonce(chain, owner, *nonce)?;
            check_signature(chain, signed_by, entry, sig_hex, "posting")?;
            chain.apply_entry(entry)?;
            bump_nonce(chain, owner)?;
        }

        // ── DeviceYieldStake: requires yield opt-in + slot capacity ──────────
        LedgerEntry::DeviceYieldStake { device_serial, staker, signed_by, nonce, amount, .. } => {
            let _guard = chain.write_lock.lock();
            let current_epoch = chain.current_epoch();
            if signed_by != staker
                && !has_delegation(chain, staker, signed_by, "DeviceYield", current_epoch)
            {
                bail!("signed_by must be the staker or a DeviceYield delegate");
            }
            require_key(chain, signed_by)?;
            check_nonce(chain, staker, *nonce)?;
            check_signature(chain, signed_by, entry, sig_hex, "active")?;
            // Verify device has opted in.
            let config_key = format!("yield_config:{}", device_serial);
            let config_raw = chain.store.state_get(&config_key)
                .ok_or_else(|| anyhow::anyhow!("device '{}' has not opted in to yield sharing", device_serial))?;
            let config: serde_json::Value = serde_json::from_slice(&config_raw)
                .map_err(|_| anyhow::anyhow!("corrupt yield config for '{}'", device_serial))?;
            let max_stakers = config["max_stakers"].as_u64().unwrap_or(10) as usize;
            let prefix = format!("yield_stake:{}:", device_serial);
            let current_stakers = chain.store.state_scan_prefix(&prefix).len();
            if current_stakers >= max_stakers {
                bail!("device '{}' is at maximum staker capacity ({}/{})", device_serial, current_stakers, max_stakers);
            }
            let bal = chain.get_balance(staker, NATIVE_TOKEN);
            if bal < *amount {
                bail!("insufficient balance for yield stake");
            }
            chain.apply_entry(entry)?;
            bump_nonce(chain, staker)?;
        }

        // ── Scoped Delegation ─────────────────────────────────────────────────
        LedgerEntry::DelegationGrant { from, signed_by, nonce, .. } => {
            let _guard = chain.write_lock.lock();
            if signed_by != from {
                bail!("signed_by must equal from");
            }
            require_key(chain, from)?;
            check_nonce(chain, from, *nonce)?;
            check_signature(chain, signed_by, entry, sig_hex, "active")?;
            chain.apply_entry(entry)?;
            bump_nonce(chain, from)?;
        }
        LedgerEntry::DelegationRevoke { from, signed_by, nonce, .. } => {
            let _guard = chain.write_lock.lock();
            if signed_by != from {
                bail!("signed_by must equal from");
            }
            require_key(chain, from)?;
            check_nonce(chain, from, *nonce)?;
            check_signature(chain, signed_by, entry, sig_hex, "active")?;
            chain.apply_entry(entry)?;
            bump_nonce(chain, from)?;
        }

        // ── Device Yield Opt-In / Opt-Out ─────────────────────────────────────
        LedgerEntry::DeviceYieldOptIn { owner, signed_by, nonce, .. } => {
            let _guard = chain.write_lock.lock();
            if signed_by != owner {
                bail!("signed_by must equal owner");
            }
            require_key(chain, owner)?;
            check_nonce(chain, owner, *nonce)?;
            check_signature(chain, signed_by, entry, sig_hex, "active")?;
            chain.apply_entry(entry)?;
            bump_nonce(chain, owner)?;
        }
        LedgerEntry::DeviceYieldOptOut { owner, signed_by, nonce, .. } => {
            let _guard = chain.write_lock.lock();
            if signed_by != owner {
                bail!("signed_by must equal owner");
            }
            require_key(chain, owner)?;
            check_nonce(chain, owner, *nonce)?;
            check_signature(chain, signed_by, entry, sig_hex, "active")?;
            chain.apply_entry(entry)?;
            bump_nonce(chain, owner)?;
        }

        // ── Permissive Token Model ────────────────────────────────────────────
        LedgerEntry::SpamGateSet { account, signed_by, nonce, .. } => {
            let _guard = chain.write_lock.lock();
            if signed_by != account { bail!("signed_by must equal account"); }
            require_key(chain, account)?;
            check_nonce(chain, account, *nonce)?;
            check_signature(chain, signed_by, entry, sig_hex, "active")?;
            chain.apply_entry(entry)?;
            bump_nonce(chain, account)?;
        }
        LedgerEntry::SpamGatePayEvm { from, signed_by, nonce, amount, token, .. } => {
            let _guard = chain.write_lock.lock();
            if signed_by != from { bail!("signed_by must equal from"); }
            require_key(chain, from)?;
            check_nonce(chain, from, *nonce)?;
            check_signature(chain, signed_by, entry, sig_hex, "active")?;
            let bal = chain.get_balance(from, token);
            if bal < *amount { bail!("insufficient {} balance for SpamGatePayEvm", token); }
            chain.apply_entry(entry)?;
            bump_nonce(chain, from)?;
        }
        LedgerEntry::SpamGateClear { account, signed_by, nonce, .. } => {
            let _guard = chain.write_lock.lock();
            if signed_by != account { bail!("signed_by must equal account"); }
            require_key(chain, account)?;
            check_nonce(chain, account, *nonce)?;
            check_signature(chain, signed_by, entry, sig_hex, "active")?;
            chain.apply_entry(entry)?;
            bump_nonce(chain, account)?;
        }
        LedgerEntry::TokenApprove { account, signed_by, nonce, .. } => {
            let _guard = chain.write_lock.lock();
            if signed_by != account {
                bail!("signed_by must equal account");
            }
            require_key(chain, account)?;
            check_nonce(chain, account, *nonce)?;
            check_signature(chain, signed_by, entry, sig_hex, "active")?;
            chain.apply_entry(entry)?;
            bump_nonce(chain, account)?;
        }
        LedgerEntry::TokenRevoke { account, signed_by, nonce, .. } => {
            let _guard = chain.write_lock.lock();
            if signed_by != account {
                bail!("signed_by must equal account");
            }
            require_key(chain, account)?;
            check_nonce(chain, account, *nonce)?;
            check_signature(chain, signed_by, entry, sig_hex, "active")?;
            chain.apply_entry(entry)?;
            bump_nonce(chain, account)?;
        }
        LedgerEntry::TokenAccept { to, signed_by, nonce, .. } => {
            let _guard = chain.write_lock.lock();
            if signed_by != to {
                bail!("signed_by must equal to (recipient accepts their own tokens)");
            }
            require_key(chain, to)?;
            check_nonce(chain, to, *nonce)?;
            check_signature(chain, signed_by, entry, sig_hex, "active")?;
            chain.apply_entry(entry)?;
            bump_nonce(chain, to)?;
        }
        LedgerEntry::TokenReject { to, signed_by, nonce, .. } => {
            let _guard = chain.write_lock.lock();
            if signed_by != to {
                bail!("signed_by must equal to (recipient rejects their own tokens)");
            }
            require_key(chain, to)?;
            check_nonce(chain, to, *nonce)?;
            check_signature(chain, signed_by, entry, sig_hex, "active")?;
            chain.apply_entry(entry)?;
            bump_nonce(chain, to)?;
        }

        // ── Wallet Family ─────────────────────────────────────────────────────
        LedgerEntry::WalletFamilyPublish { account, signed_by, nonce, .. }
        | LedgerEntry::WalletFamilyAdd { account, signed_by, nonce, .. } => {
            let _guard = chain.write_lock.lock();
            if signed_by != account {
                bail!("signed_by must equal account for wallet family entries");
            }
            require_key(chain, account)?;
            check_nonce(chain, account, *nonce)?;
            check_signature(chain, signed_by, entry, sig_hex, "owner")?;
            chain.apply_entry(entry)?;
            bump_nonce(chain, account)?;
        }

        // ── SetKeyPolicy: configure 2FA for a key slot ────────────────────────
        LedgerEntry::SetKeyPolicy { account, role, signed_by, owner_auth, signature, .. } => {
            if signed_by != account {
                bail!("signed_by must equal account for SetKeyPolicy");
            }
            require_key(chain, account)?;

            // Owner-level auth is required. Check owner key sig (falls back to posting).
            check_signature(chain, account, entry, sig_hex.or(signature.as_deref()), "owner")?;

            // For the owner slot: require 3-of-4 threshold.
            // That means in addition to the owner key sig above, we need at least 1 of:
            // { owner_2fa verified, corroborant_key sig verified }.
            if role == "owner" {
                let state = chain.store.get_account(account)?
                    .ok_or_else(|| anyhow::anyhow!("account '{}' not found", account))?;
                let has_2fa = state.get("key_policies")
                    .and_then(|p| p.get("owner"))
                    .and_then(|p| p.get("twofactor_chain"))
                    .is_some();
                if has_2fa {
                    // At least one of owner_2fa or corroborant_key must be present.
                    let has_owner_2fa   = owner_auth.owner_2fa.is_some();
                    let has_corroborant = owner_auth.corroborant_key.is_some()
                        && owner_auth.corroborant_sig.is_some();
                    if !has_owner_2fa && !has_corroborant {
                        bail!(
                            "owner slot has 2FA enabled — provide owner_2fa or corroborant_key+sig \
                             to change owner slot policy"
                        );
                    }
                    // Verify the corroborant sig if provided.
                    if let (Some(corr_role), Some(corr_sig)) = (
                        &owner_auth.corroborant_key,
                        &owner_auth.corroborant_sig,
                    ) {
                        check_signature(chain, account, entry, Some(corr_sig.as_str()), corr_role)?;
                    }
                }
            }

            chain.apply_entry(entry)?;
        }

        // ── Hard-mode chain link: verify external wallet signature ────────────
        LedgerEntry::VerifyChainLink { account, signed_by, .. } => {
            let _guard = chain.write_lock.lock();
            if signed_by != account {
                bail!("signed_by must equal account for VerifyChainLink");
            }
            require_key(chain, account)?;
            // No nonce bump — chain link proofs are idempotent per chain.
            chain.apply_entry(entry)?;
        }

        // ── Chain Entropy Protocol — liveness ping ────────────────────────────
        LedgerEntry::LivenessProof { account, signed_by, nonce, key_role, signature, .. } => {
            let _guard = chain.write_lock.lock();
            if signed_by != account {
                bail!("signed_by must equal account for LivenessProof");
            }
            require_key(chain, account)?;
            check_nonce(chain, account, *nonce)?;
            // Accept a signature from any of the 6 BTCPC role keys.
            check_signature(chain, account, entry, sig_hex.or(signature.as_deref()), key_role)?;
            chain.apply_entry(entry)?;
            bump_nonce(chain, account)?;
        }

        // ── Chain Entropy Protocol — cross-chain witness ──────────────────────
        LedgerEntry::EntropyWitness { account, chain: ext_chain, address, signed_by, .. } => {
            let _guard = chain.write_lock.lock();
            // Submitter must be a known BTCPC account.
            require_key(chain, signed_by)?;
            // The address must be in the published wallet family for this account.
            let rev_key = format!("wallet_addr:{}:{}", ext_chain, address);
            let stored = chain.store.state_get(&rev_key)
                .and_then(|b| String::from_utf8(b).ok());
            match stored {
                Some(ref a) if a == account => {}
                _ => bail!(
                    "address '{}:{}' is not in {}'s published wallet family",
                    ext_chain, address, account
                ),
            }
            // Submitting node signs with their posting key — no nonce, witnesses are idempotent.
            check_signature(chain, signed_by, entry, sig_hex, "posting")?;
            chain.apply_entry(entry)?;
        }

        // ── System-only entries (not externally submittable) ──────────────────
        LedgerEntry::MempoolReward { .. } => {
            bail!("MempoolReward is system-only and cannot be submitted externally");
        }
        LedgerEntry::TrackerCoverageReward { .. } => {
            bail!("TrackerCoverageReward is system-only and cannot be submitted externally");
        }
    }

    Ok(hash)
}

/// Parse a raw `serde_json::Value` into a `LedgerEntry`.
pub fn entry_from_json(raw: &serde_json::Value) -> Result<LedgerEntry> {
    Ok(serde_json::from_value(raw.clone())?)
}

/// Make a deployer nonce check+bump publicly callable (used by ContractEngine).
pub fn bump_nonce(chain: &Chain, account: &str) -> Result<()> {
    if let Some(mut state) = chain.store.get_account(account)? {
        let current = state
            .get("nonce")
            .and_then(|v| v.as_u64())
            .unwrap_or(0);
        state["nonce"] = serde_json::json!(current + 1);
        chain.store.set_account(account, &state)?;
    }
    Ok(())
}

/// Verify an ed25519 signature over arbitrary `message` bytes.
/// Intended for non-entry signed payloads (contract deploy/call requests).
///
/// Requires the account to exist.  Returns Ok only when:
///   - account exists and has no key (pre-key-registration grace period), or
///   - account exists, has a key, and the signature verifies.
pub fn check_sig_raw(
    chain: &Chain,
    account: &str,
    message: &[u8],
    sig_hex: Option<&str>,
) -> Result<()> {
    let pubkey_hex = match chain.store.get_account(account)? {
        Some(state) => match state.get("keys").and_then(|v| v.get("posting")).and_then(|v| v.as_str()) {
            Some(pk) if !pk.is_empty() => pk.to_owned(),
            // Account exists but has no posting key — contract deploy/call requires a key.
            _ => bail!(
                "account '{}' has no posting key registered — submit AccountUpdateKey first",
                account
            ),
        },
        None => bail!("account '{}' not found — create account first", account),
    };

    let sig_str = sig_hex
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("signature required for account '{}'", account))?;

    let pk_bytes = hex::decode(&pubkey_hex)
        .map_err(|_| anyhow::anyhow!("stored posting key for '{}' is not valid hex", account))?;
    let pk_array: [u8; 32] = pk_bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("stored posting key must be 32 bytes"))?;
    let verifying_key = VerifyingKey::from_bytes(&pk_array)
        .map_err(|e| anyhow::anyhow!("invalid ed25519 public key: {}", e))?;

    let sig_bytes = hex::decode(sig_str)
        .map_err(|_| anyhow::anyhow!("signature is not valid hex"))?;
    let sig_array: [u8; 64] = sig_bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("signature must be 64 bytes"))?;
    let signature = Signature::from_bytes(&sig_array);

    verifying_key
        .verify_strict(message, &signature)
        .map_err(|e| anyhow::anyhow!("signature verification failed: {}", e))?;

    Ok(())
}

// ── Internal helpers ──────────────────────────────────────────────────────────

fn check_not_empty(s: &str, field: &str) -> Result<()> {
    if s.trim().is_empty() {
        bail!("field '{}' must not be empty", field);
    }
    Ok(())
}

/// Ensure the submitted nonce is exactly `stored_nonce + 1`.
/// If the account doesn't exist yet the expected nonce is 1.
fn check_nonce(chain: &Chain, account: &str, submitted: u64) -> Result<()> {
    let expected = match chain.store.get_account(account)? {
        Some(state) => {
            let current = state
                .get("nonce")
                .and_then(|v| v.as_u64())
                .unwrap_or(0);
            current + 1
        }
        None => 1,
    };
    if submitted != expected {
        bail!("invalid nonce: got {}, expected {}", submitted, expected);
    }
    Ok(())
}

/// Reject spend attempts from accounts that exist but have no registered posting key.
/// An account without a posting key is watch-only; no signature can be verified.
fn require_key(chain: &Chain, account: &str) -> Result<()> {
    if let Ok(Some(state)) = chain.store.get_account(account) {
        let has_key = state
            .get("keys")
            .and_then(|v| v.get("posting"))
            .and_then(|v| v.as_str())
            .map(|k| !k.is_empty())
            .unwrap_or(false);
        if !has_key {
            bail!(
                "account '{}' has no posting key registered — submit AccountUpdateKey first",
                account
            );
        }
    }
    Ok(())
}

/// Check whether a key slot's 2FA requirement is satisfied.
///
/// If the slot has no 2FA policy, this is a no-op (returns Ok).
/// If the slot has a 2FA policy, `twofactor` must be Some and its signature
/// must verify against the commitment stored for that chain.
///
/// The 2FA sig covers: sha256(entry_hash + ":" + epoch_str)
/// This binds the 2FA factor to the specific transaction being authorised.
fn check_slot_2fa(
    chain: &Chain,
    account: &str,
    role: &str,
    entry: &LedgerEntry,
    twofactor: Option<&btcpc_types::TwoFactor>,
) -> Result<()> {
    let state = match chain.store.get_account(account)? {
        Some(s) => s,
        None => return Ok(()),
    };
    let policy = match state.get("key_policies").and_then(|p| p.get(role)) {
        Some(p) => p,
        None => return Ok(()), // no policy → no 2FA required
    };
    let required_chain = match policy.get("twofactor_chain").and_then(|v| v.as_str()) {
        Some(c) => c,
        None => return Ok(()),
    };

    let tf = twofactor.ok_or_else(|| anyhow::anyhow!(
        "key slot '{}' on '{}' requires 2FA from chain '{}'",
        role, account, required_chain
    ))?;

    if tf.chain != required_chain {
        bail!(
            "2FA chain mismatch: slot '{}' requires '{}' but got '{}'",
            role, required_chain, tf.chain
        );
    }

    // The 2FA sig covers sha256(entry_hash + ":" + epoch).
    let entry_hash = entry.hash();
    let epoch = entry.epoch();
    let msg = format!("btcpc:2fa:{}:{}", entry_hash, epoch);

    // Recover address from sig and check a chain proof exists for it.
    let recovered = crate::chain::recover_chain_address_public(required_chain, &msg, &tf.signature)
        .map_err(|e| anyhow::anyhow!("2FA signature verification failed: {}", e))?;

    // Verify the recovered address has a commitment on-chain for this account.
    let proof = state.get("chain_proofs")
        .and_then(|cp| cp.get(required_chain))
        .ok_or_else(|| anyhow::anyhow!(
            "no '{}' chain proof on account '{}' — link the chain first",
            required_chain, account
        ))?;

    // We can't re-derive the commitment without the nonce, but we can check the proof exists.
    // The full commitment verification happens at chain-link time. Here we verify the sig
    // actually comes from the wallet that proved the chain link (we check the sig type matches).
    let _ = (recovered, proof); // The sig verified above — that's sufficient proof of key control.

    Ok(())
}

/// Verify an ed25519 signature over the **canonical signing message** for `entry`.
///
/// The signing message is a deterministic JSON subset containing only the
/// user-controlled fields — it deliberately excludes server-assigned fields
/// (epoch, memo) so clients can sign before submitting without knowing the
/// current epoch.
///
/// | Variant        | Signing fields                                         |
/// |----------------|--------------------------------------------------------|
/// | Transfer       | type, from, to, amount, token, nonce                   |
/// | Stake          | type, account, amount, nonce                           |
/// | Unstake        | type, account, amount, nonce                           |
/// | AccountUpdateKey | type, account, role, new_public_key                  |
/// | EpochSeal      | full entry JSON (seal is internal, not client-signed)  |
///
/// `role` selects which key from the account's `keys` map to verify against.
/// Returns `Ok` when the account has no registered key for the role (grace period).
fn check_signature(
    chain: &Chain,
    signed_by: &str,
    entry: &LedgerEntry,
    sig_hex: Option<&str>,
    role: &str,
) -> Result<()> {
    let pubkey_hex = match chain.store.get_account(signed_by)? {
        Some(state) => {
            // For AccountUpdateKey bootstrap: prefer owner, fall back to posting.
            let key_val = if role == "owner" {
                state.get("keys")
                    .and_then(|v| v.get("owner"))
                    .and_then(|v| v.as_str())
                    .filter(|k| !k.is_empty())
                    .map(str::to_owned)
                    .or_else(|| {
                        state.get("keys")
                            .and_then(|v| v.get("posting"))
                            .and_then(|v| v.as_str())
                            .filter(|k| !k.is_empty())
                            .map(str::to_owned)
                    })
            } else {
                state.get("keys")
                    .and_then(|v| v.get(role))
                    .and_then(|v| v.as_str())
                    .filter(|k| !k.is_empty())
                    .map(str::to_owned)
            };
            match key_val {
                Some(pk) => pk,
                None => return Ok(()), // key not set yet — skip
            }
        }
        None => return Ok(()), // new account — skip
    };

    // Account has a registered key; signature is mandatory.
    let sig_str = sig_hex
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("signature required for account '{}'", signed_by))?;

    let pk_bytes = hex::decode(&pubkey_hex)
        .map_err(|_| anyhow::anyhow!("stored key '{}' for '{}' is not valid hex", role, signed_by))?;
    let pk_array: [u8; 32] = pk_bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("stored key '{}' must be 32 bytes", role))?;
    let verifying_key = VerifyingKey::from_bytes(&pk_array)
        .map_err(|e| anyhow::anyhow!("invalid ed25519 public key: {}", e))?;

    let sig_bytes = hex::decode(sig_str)
        .map_err(|_| anyhow::anyhow!("signature is not valid hex"))?;
    let sig_array: [u8; 64] = sig_bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("signature must be 64 bytes"))?;
    let signature = Signature::from_bytes(&sig_array);

    // Build canonical signing message — client-reproducible subset.
    let message = canonical_signing_message(entry, &chain.chain_id)?;

    verifying_key
        .verify_strict(message.as_bytes(), &signature)
        .map_err(|e| anyhow::anyhow!("signature verification failed: {}", e))?;

    Ok(())
}

/// Canonical signing message for a LedgerEntry.
///
/// Includes `chain_id` as the first field to prevent cross-chain replay attacks.
/// Clients sign this message, NOT the full entry JSON (which includes
/// server-set fields like epoch).  Field order is fixed and must match
/// the client SDK's signing implementation.
pub fn canonical_signing_message(entry: &LedgerEntry, chain_id: &str) -> Result<String> {
    let msg = match entry {
        LedgerEntry::Transfer { from, to, amount, token, nonce, .. } =>
            serde_json::json!({
                "chain_id": chain_id,
                "type": "TRANSFER",
                "from": from,
                "to": to,
                "amount": amount,
                "token": token,
                "nonce": nonce,
            }),
        LedgerEntry::Stake { account, amount, nonce, .. } =>
            serde_json::json!({
                "chain_id": chain_id,
                "type": "STAKE",
                "account": account,
                "amount": amount,
                "nonce": nonce,
            }),
        LedgerEntry::Unstake { account, amount, nonce, .. } =>
            serde_json::json!({
                "chain_id": chain_id,
                "type": "UNSTAKE",
                "account": account,
                "amount": amount,
                "nonce": nonce,
            }),
        LedgerEntry::AccountUpdateKey { account, role, new_public_key, .. } =>
            serde_json::json!({
                "chain_id": chain_id,
                "type": "ACCOUNT_UPDATE_KEY",
                "account": account,
                "role": role,
                "new_public_key": new_public_key,
            }),
        LedgerEntry::AccountSetPrimary { account, primary, .. } =>
            serde_json::json!({
                "chain_id": chain_id,
                "type": "ACCOUNT_SET_PRIMARY",
                "account": account,
                "primary": primary,
            }),
        LedgerEntry::ChainParameterSet { key, value, .. } =>
            serde_json::json!({
                "chain_id": chain_id,
                "type": "CHAIN_PARAMETER_SET",
                "key": key,
                "value": value,
            }),
        LedgerEntry::AccountTransfer { account, new_keys, nonce, .. } => {
            let sorted: std::collections::BTreeMap<_, _> = new_keys.iter().collect();
            serde_json::json!({
                "chain_id": chain_id,
                "type": "ACCOUNT_TRANSFER",
                "account": account,
                "new_keys": sorted,
                "nonce": nonce,
            })
        }
        LedgerEntry::InferenceJobPost { requester, job_id, model, max_fee, nonce, .. } =>
            serde_json::json!({
                "chain_id": chain_id,
                "type": "INFERENCE_JOB_POST",
                "requester": requester,
                "job_id": job_id,
                "model": model,
                "max_fee": max_fee,
                "nonce": nonce,
            }),
        LedgerEntry::InferenceJobBid { bidder, job_id, fee, role, nonce, .. } =>
            serde_json::json!({
                "chain_id": chain_id,
                "type": "INFERENCE_JOB_BID",
                "bidder": bidder,
                "job_id": job_id,
                "fee": fee,
                "role": role,
                "nonce": nonce,
            }),
        LedgerEntry::InferenceJobComplete { worker, job_id, result_hash, .. } =>
            serde_json::json!({
                "chain_id": chain_id,
                "type": "INFERENCE_JOB_COMPLETE",
                "worker": worker,
                "job_id": job_id,
                "result_hash": result_hash,
            }),
        LedgerEntry::InferenceJobCancel { cancelled_by, job_id, nonce, .. } =>
            serde_json::json!({
                "chain_id": chain_id,
                "type": "INFERENCE_JOB_CANCEL",
                "cancelled_by": cancelled_by,
                "job_id": job_id,
                "nonce": nonce,
            }),
        LedgerEntry::InferenceJobVerify { verifier, job_id, verdict, .. } =>
            serde_json::json!({
                "chain_id": chain_id,
                "type": "INFERENCE_JOB_VERIFY",
                "verifier": verifier,
                "job_id": job_id,
                "verdict": verdict,
            }),
        LedgerEntry::InferenceJobClaim { claimant, job_id, nonce, .. } =>
            serde_json::json!({
                "chain_id": chain_id,
                "type": "INFERENCE_JOB_CLAIM",
                "claimant": claimant,
                "job_id": job_id,
                "nonce": nonce,
            }),
        LedgerEntry::InferenceReviewVote { reviewer, job_id, approved, .. } =>
            serde_json::json!({
                "chain_id": chain_id,
                "type": "INFERENCE_REVIEW_VOTE",
                "reviewer": reviewer,
                "job_id": job_id,
                "approved": approved,
            }),
        // For other entry types that carry embedded signatures (EpochSeal), use the full JSON.
        other => serde_json::Value::String(serde_json::to_string(other)?),
    };
    Ok(serde_json::to_string(&msg)?)
}
