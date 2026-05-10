//! System contract bootstrap — deploys built-in contracts on first node startup.
//!
//! Currently deployed:
//!   - BTCPC Registry (btcpc-registry) — on-chain key-value store
//!
//! Each contract is deployed exactly once: the contract_id is stored at
//! `system_contract:{name}` in the chain state. If that key exists, the
//! contract is already deployed and this module is a no-op.

use std::sync::Arc;
use anyhow::Result;
use base64::{Engine as _, engine::general_purpose::STANDARD as BASE64};
use tracing::info;

use crate::chain::Chain;
use crate::contracts::ContractEngine;

/// WASM bytecode for the on-chain registry contract.
/// Built from `btcpc-contract-sdk/examples/registry` targeting wasm32-unknown-unknown.
static REGISTRY_WASM: &[u8] = include_bytes!("../contracts/registry.wasm");

const REGISTRY_DEPLOYER: &str = "shindevlin";
const REGISTRY_STATE_KEY: &str = "system_contract:registry";

/// Deploy built-in system contracts if they have not been deployed yet.
/// Safe to call on every startup — idempotent.
pub fn ensure_system_contracts(chain: &Arc<Chain>, contracts: &ContractEngine) {
    if let Err(e) = deploy_registry_if_needed(chain, contracts) {
        tracing::warn!("[system_contracts] registry deploy failed: {}", e);
    }
}

fn deploy_registry_if_needed(chain: &Arc<Chain>, contracts: &ContractEngine) -> Result<()> {
    // Already deployed?
    if chain.store.state_get(REGISTRY_STATE_KEY).is_some() {
        return Ok(());
    }

    // Ensure deployer account exists (it should — shindevlin is in genesis).
    if chain.store.get_account(REGISTRY_DEPLOYER)?.is_none() {
        // Not in genesis yet — skip silently; will retry on next restart.
        return Ok(());
    }

    let deployer_nonce = chain.store.get_account(REGISTRY_DEPLOYER)?
        .and_then(|s| s.get("nonce").and_then(|v| v.as_u64()))
        .unwrap_or(0);
    let nonce = deployer_nonce + 1;

    let wasm_b64 = BASE64.encode(REGISTRY_WASM);
    let epoch = chain.current_epoch();

    let contract_id = contracts.deploy(
        REGISTRY_DEPLOYER,
        &wasm_b64,
        Some("new".to_owned()),
        Some(serde_json::json!({})),
        300_000_000_000,
        epoch,
        nonce,
    )?;

    // Record the contract_id so we skip deployment on future restarts.
    chain.store.state_set(REGISTRY_STATE_KEY, contract_id.as_bytes())?;

    info!(
        "[system_contracts] registry deployed at {} (epoch {})",
        contract_id, epoch
    );

    Ok(())
}

/// Return the deployed registry contract_id, or None if not yet deployed.
pub fn registry_contract_id(chain: &Arc<Chain>) -> Option<String> {
    chain.store.state_get(REGISTRY_STATE_KEY)
        .and_then(|b| String::from_utf8(b).ok())
}
