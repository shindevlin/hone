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
        LedgerEntry::Transfer { from, to, amount, token, nonce, signed_by, .. } => {
            let _guard = chain.write_lock.lock();
            check_not_empty(from, "from")?;
            check_not_empty(to, "to")?;
            if *amount == 0 {
                bail!("transfer amount must be positive");
            }
            // Signer must be the sender — prevents spending another account's funds.
            if signed_by != from {
                bail!("signed_by '{}' must equal from '{}'", signed_by, from);
            }
            // Account must have a registered public key — no keyless spending.
            require_key(chain, from)?;
            check_nonce(chain, from, *nonce)?;
            check_signature(chain, signed_by, entry, sig_hex)?;
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
            if signed_by != account {
                bail!("signed_by '{}' must equal account '{}'", signed_by, account);
            }
            require_key(chain, account)?;
            check_nonce(chain, account, *nonce)?;
            check_signature(chain, signed_by, entry, sig_hex)?;
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
            if signed_by != account {
                bail!("signed_by '{}' must equal account '{}'", signed_by, account);
            }
            require_key(chain, account)?;
            check_nonce(chain, account, *nonce)?;
            check_signature(chain, signed_by, entry, sig_hex)?;
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
        LedgerEntry::AccountCreate { account, .. } => {
            check_not_empty(account, "account")?;
            chain.apply_entry(entry)?;
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
            check_signature(chain, node_id, entry, effective_sig)?;
            chain.apply_entry(entry)?;
        }

        // ── Allowlisted pass-through entries ──────────────────────────────────
        LedgerEntry::AccountUpdateKey { .. }
        | LedgerEntry::SensorReading { .. }
        | LedgerEntry::BlobStore { .. }
        | LedgerEntry::InferenceJob { .. }
        | LedgerEntry::ContractDeploy { .. }
        | LedgerEntry::ContractCall { .. } => {
            chain.apply_entry(entry)?;
        }

        // ── Privileged types: never accepted from external callers ────────────
        LedgerEntry::Mine { .. }
        | LedgerEntry::MineReward { .. }
        | LedgerEntry::GenesisAlloc { .. }
        | LedgerEntry::EpochFinalize { .. } => {
            bail!("entry type is not externally submittable");
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
/// Same key lookup as `check_signature`; intended for non-entry signed payloads
/// (contract deploy/call requests).
pub fn check_sig_raw(
    chain: &Chain,
    account: &str,
    message: &[u8],
    sig_hex: Option<&str>,
) -> Result<()> {
    let pubkey_hex = match chain.store.get_account(account)? {
        Some(state) => match state.get("public_key").and_then(|v| v.as_str()) {
            Some(pk) if !pk.is_empty() => pk.to_owned(),
            _ => return Ok(()),
        },
        None => return Ok(()),
    };

    let sig_str = sig_hex
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("signature required for account '{}'", account))?;

    let pk_bytes = hex::decode(&pubkey_hex)
        .map_err(|_| anyhow::anyhow!("stored public_key for '{}' is not valid hex", account))?;
    let pk_array: [u8; 32] = pk_bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("stored public_key must be 32 bytes"))?;
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

/// Reject spend attempts from accounts that exist but have no registered key.
/// An account without a key is watch-only; no signature can be verified.
fn require_key(chain: &Chain, account: &str) -> Result<()> {
    if let Ok(Some(state)) = chain.store.get_account(account) {
        let has_key = state
            .get("public_key")
            .and_then(|v| v.as_str())
            .map(|k| !k.is_empty())
            .unwrap_or(false);
        if !has_key {
            bail!(
                "account '{}' has no public key registered — submit AccountUpdateKey first",
                account
            );
        }
    }
    Ok(())
}

/// Verify an ed25519 signature over the canonical JSON of `entry`.
///
/// `signed_by`: the account whose registered public key should be used.
/// `sig_hex`:   the signature to verify; if `None` and the account has a
///              registered key the call **fails** (missing required signature).
///
/// The call is a no-op (returns `Ok`) when the account has no registered key —
/// this permits the new-account flow where `AccountCreate` is the first op.
fn check_signature(
    chain: &Chain,
    signed_by: &str,
    entry: &LedgerEntry,
    sig_hex: Option<&str>,
) -> Result<()> {
    let pubkey_hex = match chain.store.get_account(signed_by)? {
        Some(state) => match state.get("public_key").and_then(|v| v.as_str()) {
            Some(pk) if !pk.is_empty() => pk.to_owned(),
            _ => return Ok(()), // key not set yet — skip
        },
        None => return Ok(()), // new account — skip
    };

    // From here on the account has a registered key; signature is mandatory.
    let sig_str = sig_hex
        .filter(|s| !s.is_empty())
        .ok_or_else(|| anyhow::anyhow!("signature required for account '{}'", signed_by))?;

    let pk_bytes = hex::decode(&pubkey_hex)
        .map_err(|_| anyhow::anyhow!("stored public_key for '{}' is not valid hex", signed_by))?;
    let pk_array: [u8; 32] = pk_bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("stored public_key must be 32 bytes"))?;
    let verifying_key = VerifyingKey::from_bytes(&pk_array)
        .map_err(|e| anyhow::anyhow!("invalid ed25519 public key: {}", e))?;

    let sig_bytes = hex::decode(sig_str)
        .map_err(|_| anyhow::anyhow!("signature is not valid hex"))?;
    let sig_array: [u8; 64] = sig_bytes
        .try_into()
        .map_err(|_| anyhow::anyhow!("signature must be 64 bytes"))?;
    let signature = Signature::from_bytes(&sig_array);

    // Message = canonical JSON of the entry (same bytes used by entry.hash()).
    let message = serde_json::to_string(entry)?;

    verifying_key
        .verify_strict(message.as_bytes(), &signature)
        .map_err(|e| anyhow::anyhow!("signature verification failed: {}", e))?;

    Ok(())
}
