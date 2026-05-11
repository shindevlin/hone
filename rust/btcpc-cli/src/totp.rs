use anyhow::Result;
use btcpc_sdk::KeyPair;
use serde_json::{json, Value};
use std::path::Path;

use crate::api::ApiClient;
use crate::helpers::{next_nonce, node_chain_id, print_resp, resolve_key_file, sign_entry};

pub fn cmd_totp_setup(account: &str, key_file: Option<&Path>) -> Result<()> {
    let api = ApiClient::new();
    let kp = KeyPair::from_file(&resolve_key_file(key_file)?)?;
    let nonce = next_nonce(&api, account)?;
    let chain_id = node_chain_id(&api)?;
    let sig = sign_entry(&kp, &chain_id, "TotpSetup", json!({
        "account": account, "nonce": nonce,
    }));
    let resp: Value = api.post("/api/totp/setup", &json!({
        "account": account, "nonce": nonce, "signed_by": account, "signature": sig,
    }))?;
    print_resp(&resp);
    Ok(())
}

pub fn cmd_totp_enable(account: &str, code: &str, key_file: Option<&Path>) -> Result<()> {
    let api = ApiClient::new();
    let kp = KeyPair::from_file(&resolve_key_file(key_file)?)?;
    let nonce = next_nonce(&api, account)?;
    let chain_id = node_chain_id(&api)?;
    let sig = sign_entry(&kp, &chain_id, "TotpEnable", json!({
        "account": account, "code": code, "nonce": nonce,
    }));
    let resp: Value = api.post("/api/totp/enable", &json!({
        "account": account, "code": code,
        "nonce": nonce, "signed_by": account, "signature": sig,
    }))?;
    print_resp(&resp);
    Ok(())
}

pub fn cmd_totp_verify(account: &str, code: &str) -> Result<()> {
    let api = ApiClient::new();
    let resp: Value = api.post("/api/totp/verify", &json!({
        "account": account, "code": code,
    }))?;
    print_resp(&resp);
    Ok(())
}

pub fn cmd_totp_disable(account: &str, code: &str, key_file: Option<&Path>) -> Result<()> {
    let api = ApiClient::new();
    let kp = KeyPair::from_file(&resolve_key_file(key_file)?)?;
    let nonce = next_nonce(&api, account)?;
    let chain_id = node_chain_id(&api)?;
    let sig = sign_entry(&kp, &chain_id, "TotpDisable", json!({
        "account": account, "code": code, "nonce": nonce,
    }));
    let resp: Value = api.post("/api/totp/disable", &json!({
        "account": account, "code": code,
        "nonce": nonce, "signed_by": account, "signature": sig,
    }))?;
    print_resp(&resp);
    Ok(())
}

pub fn cmd_totp_backup_codes(account: &str, key_file: Option<&Path>) -> Result<()> {
    let api = ApiClient::new();
    let kp = KeyPair::from_file(&resolve_key_file(key_file)?)?;
    let nonce = next_nonce(&api, account)?;
    let chain_id = node_chain_id(&api)?;
    let sig = sign_entry(&kp, &chain_id, "TotpBackupCodes", json!({
        "account": account, "nonce": nonce,
    }));
    let resp: Value = api.post("/api/totp/backup-codes", &json!({
        "account": account, "nonce": nonce, "signed_by": account, "signature": sig,
    }))?;
    print_resp(&resp);
    Ok(())
}
