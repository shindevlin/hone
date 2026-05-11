use anyhow::Result;
use btcpc_sdk::KeyPair;
use serde_json::{json, Value};
use std::path::Path;

use crate::api::ApiClient;
use crate::helpers::{next_nonce, node_chain_id, print_resp, resolve_key_file, sign_entry};

pub fn cmd_private_auth_enroll(
    account: &str, approvers: Vec<String>, threshold: u32,
    key_file: Option<&Path>,
) -> Result<()> {
    let api = ApiClient::new();
    let kp = KeyPair::from_file(&resolve_key_file(key_file)?)?;
    let nonce = next_nonce(&api, account)?;
    let chain_id = node_chain_id(&api)?;
    let sig = sign_entry(&kp, &chain_id, "PrivateAuthEnroll", json!({
        "account": account, "approvers": approvers, "threshold": threshold, "nonce": nonce,
    }));
    let resp: Value = api.post("/api/private-auth/enroll", &json!({
        "account": account, "approvers": approvers, "threshold": threshold,
        "nonce": nonce, "signed_by": account, "signature": sig,
    }))?;
    print_resp(&resp);
    Ok(())
}

pub fn cmd_private_auth_approve(
    tx_hash: &str, approver: &str, approved: bool,
    key_file: Option<&Path>,
) -> Result<()> {
    let api = ApiClient::new();
    let kp = KeyPair::from_file(&resolve_key_file(key_file)?)?;
    let nonce = next_nonce(&api, approver)?;
    let chain_id = node_chain_id(&api)?;
    let sig = sign_entry(&kp, &chain_id, "PrivateAuthApprove", json!({
        "tx_hash": tx_hash, "approver": approver, "approved": approved, "nonce": nonce,
    }));
    let resp: Value = api.post("/api/private-auth/approve", &json!({
        "tx_hash": tx_hash, "approver": approver, "approved": approved,
        "nonce": nonce, "signed_by": approver, "signature": sig,
    }))?;
    print_resp(&resp);
    Ok(())
}

pub fn cmd_private_auth_status(tx_hash: &str) -> Result<()> {
    let api = ApiClient::new();
    let resp: Value = api.get(&format!("/api/private-auth/status/{}", tx_hash))?;
    print_resp(&resp);
    Ok(())
}
