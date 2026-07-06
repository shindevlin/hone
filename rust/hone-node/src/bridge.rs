//! Cross-chain bridge registry (wHONE).
//!
//! BridgeFund: a custodian deposits ETH/BTC into a registered bridge contract.
//! BridgeWrap: mints wHONE on-chain (1:1 HONE hunits), up to the 4.2M HONE cap.
//! BridgeUnlock: custodian signals that wrapped tokens have been burned on the external chain.
//! BridgeUnwrap: burns on-chain wHONE and queues an unlock in FIFO order.
//!
//! Cap enforcement: total wHONE in circulation never exceeds BRIDGE_CAP_HUNITS.
//! Unlock queue: BridgeUnwrap entries queue up; custodian processes them in order.

use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use tracing::info;

use hone_types::{LedgerEntry, NATIVE_TOKEN};
use crate::chain::Chain;

const BRIDGE_CAP_HUNITS: u64 = 4_200_000 * 100_000_000; // 4.2M HONE in hunits
const WHONE_TOKEN: &str = "wHONE";
const BRIDGE_QUEUE_KEY: &str = "bridge_unlock_queue";
const BRIDGE_SUPPLY_KEY: &str = "bridge_whone_supply";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UnlockRequest {
    pub request_id: String,
    pub recipient_external: String,
    pub amount_hunits: u64,
    pub queued_epoch: u64,
    pub chain: String,
    pub fulfilled: bool,
}

fn supply() -> impl Fn(&Chain) -> u64 {
    |chain: &Chain| {
        chain.store.state_get(BRIDGE_SUPPLY_KEY)
            .and_then(|b| serde_json::from_slice::<u64>(&b).ok())
            .unwrap_or(0)
    }
}

pub fn apply_fund(chain: &Chain, entry: &LedgerEntry) -> Result<()> {
    let LedgerEntry::BridgeFund {
        bridge_id, custodian, amount_hunits, external_tx_hash, chain: ext_chain, epoch, ..
    } = entry else { bail!("wrong entry type") };

    // Idempotency: reject duplicate external tx hashes.
    let dedup_key = format!("bridge_fund_tx:{}", external_tx_hash);
    if chain.store.state_get(&dedup_key).is_some() {
        bail!("external tx '{}' already processed", external_tx_hash);
    }

    let current_supply = supply()(chain);
    if current_supply + amount_hunits > BRIDGE_CAP_HUNITS {
        bail!("bridge cap exceeded: {} + {} > {}", current_supply, amount_hunits, BRIDGE_CAP_HUNITS);
    }

    // Mint wHONE to custodian.
    chain.store.credit(custodian, WHONE_TOKEN, *amount_hunits)?;
    let new_supply = current_supply + amount_hunits;
    chain.store.state_set(BRIDGE_SUPPLY_KEY, &serde_json::to_vec(&new_supply)?)?;
    chain.store.state_set(&dedup_key, &serde_json::to_vec(epoch)?)?;

    // Record bridge fund event.
    let record = serde_json::json!({
        "bridge_id": bridge_id,
        "custodian": custodian,
        "amount_hunits": amount_hunits,
        "external_tx_hash": external_tx_hash,
        "chain": ext_chain,
        "epoch": epoch,
    });
    chain.store.state_set(
        &format!("bridge_fund:{}:{}", epoch, external_tx_hash),
        &serde_json::to_vec(&record)?,
    )?;
    info!("[bridge] funded {} wHONE for '{}' (supply now {})", amount_hunits, custodian, new_supply);
    Ok(())
}

pub fn apply_wrap(chain: &Chain, entry: &LedgerEntry) -> Result<()> {
    let LedgerEntry::BridgeWrap {
        account, amount_hunits, external_address, chain: ext_chain, epoch, ..
    } = entry else { bail!("wrong entry type") };

    let current_supply = supply()(chain);
    if current_supply + amount_hunits > BRIDGE_CAP_HUNITS {
        bail!("bridge cap exceeded");
    }

    // Burn HONE, mint wHONE.
    chain.store.debit(account, NATIVE_TOKEN, *amount_hunits)?;
    chain.store.credit(account, WHONE_TOKEN, *amount_hunits)?;
    let new_supply = current_supply + amount_hunits;
    chain.store.state_set(BRIDGE_SUPPLY_KEY, &serde_json::to_vec(&new_supply)?)?;

    let record = serde_json::json!({
        "account": account,
        "amount_hunits": amount_hunits,
        "external_address": external_address,
        "chain": ext_chain,
        "epoch": epoch,
    });
    chain.store.state_set(
        &format!("bridge_wrap:{}:{}", account, epoch),
        &serde_json::to_vec(&record)?,
    )?;
    info!("[bridge] {} wrapped {} HONE → wHONE (→ {})", account, amount_hunits, external_address);
    Ok(())
}

pub fn apply_unwrap(chain: &Chain, entry: &LedgerEntry) -> Result<()> {
    let LedgerEntry::BridgeUnwrap {
        account, amount_hunits, recipient_external, chain: ext_chain, epoch, ..
    } = entry else { bail!("wrong entry type") };

    // Burn wHONE from account.
    chain.store.debit(account, WHONE_TOKEN, *amount_hunits)?;
    let new_supply = supply()(chain).saturating_sub(*amount_hunits);
    chain.store.state_set(BRIDGE_SUPPLY_KEY, &serde_json::to_vec(&new_supply)?)?;

    // Queue unlock request (FIFO).
    let request_id = {
        use sha2::{Digest, Sha256};
        hex::encode(Sha256::digest(
            format!("unwrap:{}:{}:{}", account, epoch, recipient_external).as_bytes()
        ))
    };
    let req = UnlockRequest {
        request_id: request_id.clone(),
        recipient_external: recipient_external.clone(),
        amount_hunits: *amount_hunits,
        queued_epoch: *epoch,
        chain: ext_chain.clone(),
        fulfilled: false,
    };

    // Append to queue.
    let mut queue: Vec<UnlockRequest> = chain.store.state_get(BRIDGE_QUEUE_KEY)
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    queue.push(req);
    chain.store.state_set(BRIDGE_QUEUE_KEY, &serde_json::to_vec(&queue)?)?;

    info!("[bridge] {} unwrapped {} wHONE → {} (request {})",
        account, amount_hunits, recipient_external, &request_id[..12]);
    Ok(())
}

pub fn apply_unlock(chain: &Chain, entry: &LedgerEntry) -> Result<()> {
    let LedgerEntry::BridgeUnlock {
        request_id, custodian, external_tx_hash, epoch, ..
    } = entry else { bail!("wrong entry type") };

    // Mark request as fulfilled in queue.
    let mut queue: Vec<UnlockRequest> = chain.store.state_get(BRIDGE_QUEUE_KEY)
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();

    let req = queue.iter_mut()
        .find(|r| &r.request_id == request_id)
        .ok_or_else(|| anyhow::anyhow!("unlock request '{}' not found", request_id))?;

    if req.fulfilled {
        bail!("request '{}' already fulfilled", request_id);
    }
    req.fulfilled = true;
    chain.store.state_set(BRIDGE_QUEUE_KEY, &serde_json::to_vec(&queue)?)?;

    // Record fulfillment.
    let record = serde_json::json!({
        "request_id": request_id,
        "custodian": custodian,
        "external_tx_hash": external_tx_hash,
        "epoch": epoch,
    });
    chain.store.state_set(
        &format!("bridge_unlock:{}:{}", request_id, epoch),
        &serde_json::to_vec(&record)?,
    )?;
    info!("[bridge] unlock {} fulfilled by '{}' (ext_tx: {})",
        &request_id[..12], custodian, external_tx_hash);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use hone_types::LedgerEntry;
    use tempfile::TempDir;

    fn make_chain() -> (Chain, TempDir) {
        let dir = tempfile::TempDir::new().unwrap();
        let store = crate::store::Store::open(dir.path()).unwrap();
        let chain = Chain::new(
            store, "test".to_string(), "hone-test".to_string(),
        );
        (chain, dir)
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
            token: hone_types::NATIVE_TOKEN.to_string(),
        }).unwrap();
    }

    fn fund_entry(bridge_id: &str, custodian: &str, amount: u64, ext_tx: &str) -> LedgerEntry {
        LedgerEntry::BridgeFund {
            bridge_id: bridge_id.to_string(),
            custodian: custodian.to_string(),
            amount_hunits: amount,
            external_tx_hash: ext_tx.to_string(),
            chain: "ethereum".to_string(),
            epoch: 1,
            nonce: 1,
            signed_by: custodian.to_string(),
            signature: None,
        }
    }

    #[test]
    fn fund_stores_custody_record() {
        let (chain, _dir) = make_chain();

        apply_fund(&chain, &fund_entry("bridge1", "custodian1", 1_000_000, "0xabc123")).unwrap();

        // wHONE should be credited to custodian
        let balance = chain.get_balance("custodian1", WHONE_TOKEN);
        assert_eq!(balance, 1_000_000, "custodian should receive wHONE");

        // Fund record should be stored
        let rec = chain.store.state_get(&format!("bridge_fund:1:0xabc123")).unwrap();
        let j: serde_json::Value = serde_json::from_slice(&rec).unwrap();
        assert_eq!(j["amount_hunits"], 1_000_000);
    }

    #[test]
    fn wrap_mints_bridged_token() {
        let (chain, _dir) = make_chain();
        fund(&chain, "user", 10_000_000);

        let wrap = LedgerEntry::BridgeWrap {
            account: "user".to_string(),
            amount_hunits: 5_000_000,
            external_address: "0xdeadbeef".to_string(),
            chain: "ethereum".to_string(),
            epoch: 1,
            nonce: 1,
            signed_by: "user".to_string(),
            signature: None,
        };
        apply_wrap(&chain, &wrap).unwrap();

        // HONE burned, wHONE credited
        assert_eq!(chain.get_balance("user", NATIVE_TOKEN), 5_000_000);
        assert_eq!(chain.get_balance("user", WHONE_TOKEN), 5_000_000);
    }

    #[test]
    fn fund_unknown_custodian_succeeds() {
        // apply_fund does NOT validate custodian existence; it simply credits wHONE
        let (chain, _dir) = make_chain();
        let result = apply_fund(&chain, &fund_entry("bridge2", "unknown_custodian", 500_000, "0xfeed01"));
        assert!(result.is_ok(), "fund should succeed for unknown custodian");
        assert_eq!(chain.get_balance("unknown_custodian", WHONE_TOKEN), 500_000);
    }

    #[test]
    fn bridge_status_reflects_state() {
        let (chain, _dir) = make_chain();

        // Initial state: zero supply, no pending
        let s = status(&chain);
        assert_eq!(s["whone_supply_hunits"], 0);
        assert_eq!(s["pending_unlock_count"], 0);

        // Fund some wHONE
        apply_fund(&chain, &fund_entry("bridge3", "custodian3", 2_000_000, "0xdeadcafe")).unwrap();

        // Unwrap to queue a pending unlock
        fund(&chain, "holder", 0);
        // Give holder wHONE by funding then wrapping (or just fund directly then unwrap)
        chain.store.credit("holder", WHONE_TOKEN, 2_000_000).unwrap();
        // Adjust supply to reflect this
        chain.store.state_set(BRIDGE_SUPPLY_KEY, &serde_json::to_vec(&4_000_000u64).unwrap()).unwrap();

        let unwrap = LedgerEntry::BridgeUnwrap {
            account: "holder".to_string(),
            amount_hunits: 1_000_000,
            recipient_external: "0xrecipient".to_string(),
            chain: "ethereum".to_string(),
            epoch: 2,
            nonce: 1,
            signed_by: "holder".to_string(),
            signature: None,
        };
        apply_unwrap(&chain, &unwrap).unwrap();

        let s = status(&chain);
        assert_eq!(s["pending_unlock_count"], 1);
        assert!(s["whone_supply_hunits"].as_u64().unwrap() > 0);
    }
}

/// Get current wHONE supply and pending unlock queue.
pub fn status(chain: &Chain) -> serde_json::Value {
    let supply_hunits = supply()(chain);
    let queue: Vec<UnlockRequest> = chain.store.state_get(BRIDGE_QUEUE_KEY)
        .and_then(|b| serde_json::from_slice(&b).ok())
        .unwrap_or_default();
    let pending = queue.iter().filter(|r| !r.fulfilled).count();
    serde_json::json!({
        "whone_supply_hunits": supply_hunits,
        "cap_hunits": BRIDGE_CAP_HUNITS,
        "pending_unlock_count": pending,
        "utilization_bps": supply_hunits * 10_000 / BRIDGE_CAP_HUNITS.max(1),
    })
}
