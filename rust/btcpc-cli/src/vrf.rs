use anyhow::Result;
use btcpc_sdk::KeyPair;
use serde_json::{json, Value};
use std::path::Path;

use crate::api::ApiClient;
use crate::helpers::{next_nonce, node_chain_id, print_resp, resolve_key_file, sign_entry};

pub fn cmd_vrf_commit(
    clock_node: &str, commit_hash: &str, key_file: Option<&Path>,
) -> Result<()> {
    let api = ApiClient::new();
    let kp = KeyPair::from_file(&resolve_key_file(key_file)?)?;
    let nonce = next_nonce(&api, clock_node)?;
    let chain_id = node_chain_id(&api)?;
    let sig = sign_entry(&kp, &chain_id, "VrfCommit", json!({
        "clock_node": clock_node, "commit_hash": commit_hash, "nonce": nonce,
    }));
    let resp: Value = api.post("/api/vrf/commit", &json!({
        "clock_node": clock_node, "commit_hash": commit_hash,
        "nonce": nonce, "signed_by": clock_node, "signature": sig,
    }))?;
    print_resp(&resp);
    Ok(())
}

pub fn cmd_vrf_reveal(
    clock_node: &str, reveal_value: &str, salt: &str, key_file: Option<&Path>,
) -> Result<()> {
    let api = ApiClient::new();
    let kp = KeyPair::from_file(&resolve_key_file(key_file)?)?;
    let nonce = next_nonce(&api, clock_node)?;
    let chain_id = node_chain_id(&api)?;
    let sig = sign_entry(&kp, &chain_id, "VrfReveal", json!({
        "clock_node": clock_node, "reveal_value": reveal_value, "salt": salt, "nonce": nonce,
    }));
    let resp: Value = api.post("/api/vrf/reveal", &json!({
        "clock_node": clock_node, "reveal_value": reveal_value, "salt": salt,
        "nonce": nonce, "signed_by": clock_node, "signature": sig,
    }))?;
    print_resp(&resp);
    Ok(())
}

pub fn cmd_vrf_beacon() -> Result<()> {
    let api = ApiClient::new();
    let resp: Value = api.get("/api/vrf/beacon")?;
    print_resp(&resp);
    Ok(())
}

pub fn cmd_vrf_round(epoch: u64) -> Result<()> {
    let api = ApiClient::new();
    let resp: Value = api.get(&format!("/api/vrf/round/{}", epoch))?;
    print_resp(&resp);
    Ok(())
}
