use anyhow::Result;
use btcpc_sdk::KeyPair;
use colored::Colorize;
use serde_json::{json, Value};
use std::path::Path;

use crate::api::ApiClient;
use crate::helpers::{next_nonce, node_chain_id, print_resp, resolve_key_file, sign_entry};

// ── Name Auction ──────────────────────────────────────────────────────────────

pub fn cmd_name_open(account: &str, name: &str, duration_epochs: u64, key_file: Option<&Path>) -> Result<()> {
    let api = ApiClient::new();
    let kp = KeyPair::from_file(&resolve_key_file(key_file)?)?;
    let nonce = next_nonce(&api, account)?;
    let chain_id = node_chain_id(&api)?;
    let sig = sign_entry(&kp, &chain_id, "NAME_AUCTION_OPEN",
        json!({ "account": account, "name": name, "duration_epochs": duration_epochs, "nonce": nonce }));
    let resp: Value = api.post("/api/auction/name/open", &json!({
        "account": account, "name": name, "duration_epochs": duration_epochs,
        "nonce": nonce, "signature": sig,
    }))?;
    println!("{}", "Auction opened.".green().bold());
    print_resp(&resp);
    Ok(())
}

pub fn cmd_name_bid(account: &str, auction_id: &str, amount: u64, key_file: Option<&Path>) -> Result<()> {
    let api = ApiClient::new();
    let kp = KeyPair::from_file(&resolve_key_file(key_file)?)?;
    let nonce = next_nonce(&api, account)?;
    let chain_id = node_chain_id(&api)?;
    let sig = sign_entry(&kp, &chain_id, "NAME_AUCTION_BID",
        json!({ "account": account, "auction_id": auction_id, "amount": amount, "nonce": nonce }));
    let resp: Value = api.post("/api/auction/bid", &json!({
        "account": account, "auction_id": auction_id, "amount": amount,
        "nonce": nonce, "signature": sig,
    }))?;
    println!("{}", "Bid submitted.".green().bold());
    print_resp(&resp);
    Ok(())
}

pub fn cmd_name_settle(account: &str, auction_id: &str, key_file: Option<&Path>) -> Result<()> {
    let api = ApiClient::new();
    let kp = KeyPair::from_file(&resolve_key_file(key_file)?)?;
    let nonce = next_nonce(&api, account)?;
    let chain_id = node_chain_id(&api)?;
    let sig = sign_entry(&kp, &chain_id, "NAME_AUCTION_SETTLE",
        json!({ "account": account, "auction_id": auction_id, "nonce": nonce }));
    let resp: Value = api.post("/api/auction/settle", &json!({
        "account": account, "auction_id": auction_id,
        "nonce": nonce, "signature": sig,
    }))?;
    println!("{}", "Auction settled.".green().bold());
    print_resp(&resp);
    Ok(())
}

pub fn cmd_name_cancel(account: &str, auction_id: &str, key_file: Option<&Path>) -> Result<()> {
    let api = ApiClient::new();
    let kp = KeyPair::from_file(&resolve_key_file(key_file)?)?;
    let nonce = next_nonce(&api, account)?;
    let chain_id = node_chain_id(&api)?;
    let sig = sign_entry(&kp, &chain_id, "NAME_AUCTION_CANCEL",
        json!({ "account": account, "auction_id": auction_id, "nonce": nonce }));
    let resp: Value = api.post("/api/auction/cancel", &json!({
        "account": account, "auction_id": auction_id,
        "nonce": nonce, "signature": sig,
    }))?;
    println!("{}", "Auction cancelled.".green().bold());
    print_resp(&resp);
    Ok(())
}

pub fn cmd_auction_get(auction_id: &str) -> Result<()> {
    let api = ApiClient::new();
    let resp: Value = api.get(&format!("/api/auction/{}", auction_id))?;
    println!("{}", serde_json::to_string_pretty(&resp)?);
    Ok(())
}

// ── Freeport Auction ──────────────────────────────────────────────────────────

pub fn cmd_freeport_open(
    account: &str, item_id: &str, item_type: &str,
    duration_epochs: u64, key_file: Option<&Path>,
) -> Result<()> {
    let api = ApiClient::new();
    let kp = KeyPair::from_file(&resolve_key_file(key_file)?)?;
    let nonce = next_nonce(&api, account)?;
    let chain_id = node_chain_id(&api)?;
    let sig = sign_entry(&kp, &chain_id, "FREEPORT_AUCTION_OPEN",
        json!({ "account": account, "item_id": item_id, "item_type": item_type,
                 "duration_epochs": duration_epochs, "nonce": nonce }));
    let resp: Value = api.post("/api/freeport/auction/open", &json!({
        "account": account, "item_id": item_id, "item_type": item_type,
        "duration_epochs": duration_epochs, "nonce": nonce, "signature": sig,
    }))?;
    println!("{}", "Freeport auction opened.".green().bold());
    print_resp(&resp);
    Ok(())
}

pub fn cmd_freeport_bid(
    account: &str, auction_id: &str, amount: u64, key_file: Option<&Path>,
) -> Result<()> {
    let api = ApiClient::new();
    let kp = KeyPair::from_file(&resolve_key_file(key_file)?)?;
    let nonce = next_nonce(&api, account)?;
    let chain_id = node_chain_id(&api)?;
    let sig = sign_entry(&kp, &chain_id, "FREEPORT_AUCTION_BID",
        json!({ "account": account, "auction_id": auction_id, "amount": amount, "nonce": nonce }));
    let resp: Value = api.post("/api/freeport/auction/bid", &json!({
        "account": account, "auction_id": auction_id, "amount": amount,
        "nonce": nonce, "signature": sig,
    }))?;
    println!("{}", "Freeport bid submitted.".green().bold());
    print_resp(&resp);
    Ok(())
}

pub fn cmd_freeport_settle(
    account: &str, auction_id: &str, key_file: Option<&Path>,
) -> Result<()> {
    let api = ApiClient::new();
    let kp = KeyPair::from_file(&resolve_key_file(key_file)?)?;
    let nonce = next_nonce(&api, account)?;
    let chain_id = node_chain_id(&api)?;
    let sig = sign_entry(&kp, &chain_id, "FREEPORT_AUCTION_SETTLE",
        json!({ "account": account, "auction_id": auction_id, "nonce": nonce }));
    let resp: Value = api.post("/api/freeport/auction/settle", &json!({
        "account": account, "auction_id": auction_id,
        "nonce": nonce, "signature": sig,
    }))?;
    println!("{}", "Freeport auction settled.".green().bold());
    print_resp(&resp);
    Ok(())
}

pub fn cmd_freeport_get(auction_id: &str) -> Result<()> {
    let api = ApiClient::new();
    let resp: Value = api.get(&format!("/api/freeport/auction/{}", auction_id))?;
    println!("{}", serde_json::to_string_pretty(&resp)?);
    Ok(())
}
