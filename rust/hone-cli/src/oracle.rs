use anyhow::Result;
use hone_sdk::KeyPair;
use serde_json::{json, Value};
use std::path::Path;

use crate::api::ApiClient;
use crate::helpers::{next_nonce, node_chain_id, print_resp, resolve_key_file, sign_entry};

pub fn cmd_oracle_create(
    creator: &str, feed_id: &str, description: &str, asset_pair: &str,
    min_reporters: u32, key_file: Option<&Path>,
) -> Result<()> {
    let api = ApiClient::new();
    let kp = KeyPair::from_file(&resolve_key_file(key_file)?)?;
    let nonce = next_nonce(&api, creator)?;
    let chain_id = node_chain_id(&api)?;
    let sig = sign_entry(&kp, &chain_id, "OracleFeedCreate", json!({
        "creator": creator, "feed_id": feed_id, "description": description,
        "asset_pair": asset_pair, "min_reporters": min_reporters, "nonce": nonce,
    }));
    let resp: Value = api.post("/api/oracle/feed/create", &json!({
        "creator": creator, "feed_id": feed_id, "description": description,
        "asset_pair": asset_pair, "min_reporters": min_reporters,
        "nonce": nonce, "signed_by": creator, "signature": sig,
    }))?;
    print_resp(&resp);
    Ok(())
}

pub fn cmd_oracle_report(
    feed_id: &str, reporter: &str, value: &str,
    commit_hash: Option<String>, key_file: Option<&Path>,
) -> Result<()> {
    let api = ApiClient::new();
    let kp = KeyPair::from_file(&resolve_key_file(key_file)?)?;
    let nonce = next_nonce(&api, reporter)?;
    let chain_id = node_chain_id(&api)?;
    let sig = sign_entry(&kp, &chain_id, "OracleReport", json!({
        "feed_id": feed_id, "reporter": reporter, "value": value, "nonce": nonce,
    }));
    let mut body = json!({
        "reporter": reporter, "value": value,
        "nonce": nonce, "signed_by": reporter, "signature": sig,
    });
    if let Some(c) = commit_hash { body["commit_hash"] = c.into(); }
    let resp: Value = api.post(&format!("/api/oracle/feed/{}/report", feed_id), &body)?;
    print_resp(&resp);
    Ok(())
}

pub fn cmd_oracle_finalize(
    feed_id: &str, finalizer: &str, reveal_value: Option<String>,
    reveal_salt: Option<String>, key_file: Option<&Path>,
) -> Result<()> {
    let api = ApiClient::new();
    let kp = KeyPair::from_file(&resolve_key_file(key_file)?)?;
    let nonce = next_nonce(&api, finalizer)?;
    let chain_id = node_chain_id(&api)?;
    let sig = sign_entry(&kp, &chain_id, "OracleFeedFinalize", json!({
        "feed_id": feed_id, "finalizer": finalizer, "nonce": nonce,
    }));
    let mut body = json!({
        "finalizer": finalizer, "nonce": nonce, "signed_by": finalizer, "signature": sig,
    });
    if let Some(v) = reveal_value { body["reveal_value"] = v.into(); }
    if let Some(s) = reveal_salt  { body["reveal_salt"]  = s.into(); }
    let resp: Value = api.post(&format!("/api/oracle/feed/{}/finalize", feed_id), &body)?;
    print_resp(&resp);
    Ok(())
}

pub fn cmd_oracle_get(feed_id: &str) -> Result<()> {
    let api = ApiClient::new();
    let resp: Value = api.get(&format!("/api/oracle/feed/{}", feed_id))?;
    print_resp(&resp);
    Ok(())
}

pub fn cmd_oracle_list() -> Result<()> {
    let api = ApiClient::new();
    let resp: Value = api.get("/api/oracle/feeds")?;
    print_resp(&resp);
    Ok(())
}

pub fn cmd_oracle_price(pair: &str) -> Result<()> {
    let api = ApiClient::new();
    let resp: Value = api.get(&format!("/api/oracle/price/{}", pair))?;
    print_resp(&resp);
    Ok(())
}

pub fn cmd_oracle_reputation(reporter: &str) -> Result<()> {
    let api = ApiClient::new();
    let resp: Value = api.get(&format!("/api/oracle/reputation/{}", reporter))?;
    print_resp(&resp);
    Ok(())
}
