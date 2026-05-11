use anyhow::Result;
use btcpc_sdk::KeyPair;
use serde_json::{json, Value};
use std::path::Path;

use crate::api::ApiClient;
use crate::helpers::{next_nonce, node_chain_id, print_resp, resolve_key_file, sign_entry};

pub fn cmd_slash_submit(
    reporter: &str, accused: &str, violation: &str, evidence: &str,
    key_file: Option<&Path>,
) -> Result<()> {
    let api = ApiClient::new();
    let kp = KeyPair::from_file(&resolve_key_file(key_file)?)?;
    let nonce = next_nonce(&api, reporter)?;
    let chain_id = node_chain_id(&api)?;
    let sig = sign_entry(&kp, &chain_id, "SlashValidator", json!({
        "reporter": reporter, "accused": accused,
        "violation": violation, "evidence": evidence, "nonce": nonce,
    }));
    let resp: Value = api.post("/api/slash/submit", &json!({
        "reporter": reporter, "accused": accused,
        "violation": violation, "evidence": evidence,
        "nonce": nonce, "signed_by": reporter, "signature": sig,
    }))?;
    print_resp(&resp);
    Ok(())
}

pub fn cmd_slash_appeal(
    slash_id: &str, panelist: &str, overturn: bool, key_file: Option<&Path>,
) -> Result<()> {
    let api = ApiClient::new();
    let kp = KeyPair::from_file(&resolve_key_file(key_file)?)?;
    let nonce = next_nonce(&api, panelist)?;
    let chain_id = node_chain_id(&api)?;
    let sig = sign_entry(&kp, &chain_id, "SlashAppeal", json!({
        "slash_id": slash_id, "panelist": panelist, "overturn": overturn, "nonce": nonce,
    }));
    let resp: Value = api.post("/api/slash/appeal", &json!({
        "slash_id": slash_id, "panelist": panelist, "overturn": overturn,
        "nonce": nonce, "signed_by": panelist, "signature": sig,
    }))?;
    print_resp(&resp);
    Ok(())
}

pub fn cmd_slash_get(slash_id: &str) -> Result<()> {
    let api = ApiClient::new();
    let resp: Value = api.get(&format!("/api/slash/{}", slash_id))?;
    print_resp(&resp);
    Ok(())
}

pub fn cmd_slash_list() -> Result<()> {
    let api = ApiClient::new();
    let resp: Value = api.get("/api/slash/list")?;
    print_resp(&resp);
    Ok(())
}
