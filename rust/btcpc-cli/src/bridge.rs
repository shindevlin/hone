use anyhow::Result;
use btcpc_sdk::KeyPair;
use serde_json::{json, Value};
use std::path::Path;

use crate::api::ApiClient;
use crate::helpers::{next_nonce, node_chain_id, print_resp, resolve_key_file, sign_entry};

pub fn cmd_bridge_fund(
    bridge_id: &str, custodian: &str, amount_dreams: u64,
    external_tx_hash: &str, chain: &str, key_file: Option<&Path>,
) -> Result<()> {
    let api = ApiClient::new();
    let kp = KeyPair::from_file(&resolve_key_file(key_file)?)?;
    let nonce = next_nonce(&api, custodian)?;
    let chain_id = node_chain_id(&api)?;
    let sig = sign_entry(&kp, &chain_id, "BridgeFund", json!({
        "bridge_id": bridge_id, "custodian": custodian,
        "amount_dreams": amount_dreams, "external_tx_hash": external_tx_hash,
        "chain": chain, "nonce": nonce,
    }));
    let resp: Value = api.post("/api/bridge/fund", &json!({
        "bridge_id": bridge_id, "custodian": custodian,
        "amount_dreams": amount_dreams, "external_tx_hash": external_tx_hash,
        "chain": chain, "nonce": nonce, "signed_by": custodian, "signature": sig,
    }))?;
    print_resp(&resp);
    Ok(())
}

pub fn cmd_bridge_wrap(
    account: &str, amount_dreams: u64, external_address: &str,
    chain: &str, key_file: Option<&Path>,
) -> Result<()> {
    let api = ApiClient::new();
    let kp = KeyPair::from_file(&resolve_key_file(key_file)?)?;
    let nonce = next_nonce(&api, account)?;
    let chain_id = node_chain_id(&api)?;
    let sig = sign_entry(&kp, &chain_id, "BridgeWrap", json!({
        "account": account, "amount_dreams": amount_dreams,
        "external_address": external_address, "chain": chain, "nonce": nonce,
    }));
    let resp: Value = api.post("/api/bridge/wrap", &json!({
        "account": account, "amount_dreams": amount_dreams,
        "external_address": external_address, "chain": chain,
        "nonce": nonce, "signed_by": account, "signature": sig,
    }))?;
    print_resp(&resp);
    Ok(())
}

pub fn cmd_bridge_unwrap(
    account: &str, amount_dreams: u64, recipient_external: &str,
    chain: &str, key_file: Option<&Path>,
) -> Result<()> {
    let api = ApiClient::new();
    let kp = KeyPair::from_file(&resolve_key_file(key_file)?)?;
    let nonce = next_nonce(&api, account)?;
    let chain_id = node_chain_id(&api)?;
    let sig = sign_entry(&kp, &chain_id, "BridgeUnwrap", json!({
        "account": account, "amount_dreams": amount_dreams,
        "recipient_external": recipient_external, "chain": chain, "nonce": nonce,
    }));
    let resp: Value = api.post("/api/bridge/unwrap", &json!({
        "account": account, "amount_dreams": amount_dreams,
        "recipient_external": recipient_external, "chain": chain,
        "nonce": nonce, "signed_by": account, "signature": sig,
    }))?;
    print_resp(&resp);
    Ok(())
}

pub fn cmd_bridge_unlock(
    request_id: &str, custodian: &str, external_tx_hash: &str,
    key_file: Option<&Path>,
) -> Result<()> {
    let api = ApiClient::new();
    let kp = KeyPair::from_file(&resolve_key_file(key_file)?)?;
    let nonce = next_nonce(&api, custodian)?;
    let chain_id = node_chain_id(&api)?;
    let sig = sign_entry(&kp, &chain_id, "BridgeUnlock", json!({
        "request_id": request_id, "custodian": custodian,
        "external_tx_hash": external_tx_hash, "nonce": nonce,
    }));
    let resp: Value = api.post("/api/bridge/unlock", &json!({
        "request_id": request_id, "custodian": custodian,
        "external_tx_hash": external_tx_hash,
        "nonce": nonce, "signed_by": custodian, "signature": sig,
    }))?;
    print_resp(&resp);
    Ok(())
}

pub fn cmd_bridge_status() -> Result<()> {
    let api = ApiClient::new();
    let resp: Value = api.get("/api/bridge/status")?;
    print_resp(&resp);
    Ok(())
}

pub fn cmd_bridge_queue() -> Result<()> {
    let api = ApiClient::new();
    let resp: Value = api.get("/api/bridge/queue")?;
    print_resp(&resp);
    Ok(())
}
