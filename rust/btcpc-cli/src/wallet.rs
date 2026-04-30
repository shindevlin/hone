use anyhow::{anyhow, Result};
use btcpc_sdk::{Wallet, WalletFile};
use colored::Colorize;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

use crate::api::ApiClient;

pub fn cmd_wallet_create(account: &str, output: Option<&Path>) -> Result<()> {
    let wallet_path = resolve_wallet_path(output)?;

    if wallet_path.exists() {
        return Err(anyhow!(
            "wallet file already exists at {}; use --output to specify a different path",
            wallet_path.display()
        ));
    }

    // Generate ephemeral wallet from a fresh 12-word mnemonic.
    let wallet = Wallet::generate(account)?;
    let kp = wallet.btcpc_keypair()?;
    let pubkey = kp.public_key_hex();
    let addresses = wallet.chain_addresses();

    // ── Show mnemonic ONCE ────────────────────────────────────────────────────
    println!();
    println!("{}", "━━━ WRITE DOWN YOUR MNEMONIC — stored nowhere, shown once ━━━".yellow().bold());
    println!();
    println!("  {}", wallet.mnemonic.to_string().bold());
    println!();
    println!("{}", "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━".yellow().bold());
    println!();
    println!("{} {}", "Account:".bold(), account);
    println!("{} {}", "BTCPC pubkey:".bold(), pubkey);
    println!();
    println!("{}", "Derived addresses (published on-chain):".bold());
    for (chain, addr, path) in &addresses {
        println!("  {:10} {} ({})", chain, addr, path);
    }
    println!();

    // Save identity file (public keys / addresses only).
    wallet.save_to_file(&wallet_path)?;
    println!("{} {}", "Identity file saved (public keys only):".bold(), wallet_path.display());
    println!();

    // ── Submit AccountCreate ──────────────────────────────────────────────────
    let api = ApiClient::new();
    let create_resp: Value = api.post("/api/entry", &json!({
        "type": "ACCOUNT_CREATE",
        "account": account,
        "keys": {
            "owner":   pubkey,
            "active":  pubkey,
            "posting": pubkey,
        },
        "epoch": 0,
    }))?;

    if !create_resp.get("accepted").and_then(|v| v.as_bool()).unwrap_or(false) {
        let err = create_resp.get("error").and_then(|v| v.as_str()).unwrap_or("unknown");
        return Err(anyhow!("AccountCreate rejected: {}", err));
    }
    println!("{}", "AccountCreate accepted.".green());

    // ── Submit WalletFamilyPublish ────────────────────────────────────────────
    let latest: Value = api.get("/api/latest")?;
    let epoch = latest["current_epoch"].as_u64().unwrap_or(0);

    let chain_entries: Vec<Value> = addresses
        .iter()
        .map(|(chain, addr, dpath)| {
            let msg = format!("btcpc-family:{}:{}:{}", account, chain, addr);
            json!({
                "chain": chain,
                "address": addr,
                "derivation_path": dpath,
                "signature": kp.sign_entry_json(&msg),
            })
        })
        .collect();

    let publish_entry = json!({
        "type": "WALLET_FAMILY_PUBLISH",
        "account": account,
        "chains": chain_entries,
        "epoch": epoch,
        "nonce": 0,
        "signed_by": account,
    });
    let sig = kp.sign_entry_json(&serde_json::to_string(&publish_entry)?);

    let publish_resp: Value = api.post("/api/entry", &json!({
        "entry": publish_entry,
        "signature": sig,
    }))?;

    if publish_resp.get("accepted").and_then(|v| v.as_bool()).unwrap_or(false) {
        println!("{}", "WalletFamilyPublish accepted — all addresses linked on-chain permanently.".green());
    } else {
        let err = publish_resp.get("error").and_then(|v| v.as_str()).unwrap_or("unknown");
        eprintln!(
            "{} WalletFamilyPublish failed: {} — re-run `wallet publish --mnemonic \"...\"` to retry",
            "warning:".yellow(), err
        );
    }

    Ok(())
}

/// Show the public identity file — no mnemonic or private key required.
pub fn cmd_wallet_show(wallet_file: Option<&Path>) -> Result<()> {
    let path = resolve_wallet_path(wallet_file)?;
    let raw = std::fs::read_to_string(&path)?;
    let wf: WalletFile = serde_json::from_str(&raw)?;

    println!("{} {}", "Account:".bold(), wf.account);
    println!("{} {}", "BTCPC pubkey:".bold(), wf.btcpc_public_key_hex);
    println!("{} {}", "File:".bold(), path.display());
    println!();
    println!("{}", "Linked chain addresses (public):".bold());
    let mut addrs: Vec<_> = wf.chain_addresses.iter().collect();
    addrs.sort_by_key(|(k, _)| k.as_str());
    for (chain, addr) in addrs {
        println!("  {:10} {}", chain, addr);
    }
    Ok(())
}

/// Re-publish addresses on-chain.  Requires the mnemonic (passed via --mnemonic
/// or the BTCPC_MNEMONIC env var) to derive the signing key.
pub fn cmd_wallet_publish(wallet_file: Option<&Path>, mnemonic: &str) -> Result<()> {
    let path = resolve_wallet_path(wallet_file)?;
    let raw = std::fs::read_to_string(&path)?;
    let wf: WalletFile = serde_json::from_str(&raw)?;
    let account = &wf.account;

    // Derive signing key from mnemonic (private key never stored).
    let wallet = Wallet::from_phrase(mnemonic, account)?;
    let kp = wallet.btcpc_keypair()?;

    // Verify pubkey matches identity file.
    if kp.public_key_hex() != wf.btcpc_public_key_hex {
        return Err(anyhow!(
            "mnemonic doesn't match this wallet — derived pubkey {} but file has {}",
            kp.public_key_hex(), wf.btcpc_public_key_hex
        ));
    }

    // Build addresses from stored public data (no private keys needed for addresses).
    let mut addresses: Vec<(String, String, String)> = wf.chain_addresses
        .iter()
        .map(|(chain, addr)| {
            let dpath = match chain.as_str() {
                "evm"     => "m/44'/60'/0'/0/0",
                "solana"  => "m/44'/501'/0'/0'",
                "bitcoin" => "m/44'/0'/0'/0/0",
                _         => "m/44'/2301'/0'/0'/0'",
            };
            (chain.clone(), addr.clone(), dpath.to_string())
        })
        .collect();
    // Also include the BTCPC pubkey itself.
    addresses.push(("btcpc".into(), wf.btcpc_public_key_hex.clone(), "m/44'/2301'/0'/0'/0'".into()));

    let api = ApiClient::new();
    let latest: Value = api.get("/api/latest")?;
    let epoch = latest["current_epoch"].as_u64().unwrap_or(0);
    let nonce: u64 = api.get::<Value>(&format!("/api/account/{}", account))
        .ok()
        .and_then(|v| v["nonce"].as_u64())
        .unwrap_or(0);

    let chain_entries: Vec<Value> = addresses
        .iter()
        .map(|(chain, addr, dpath)| {
            let msg = format!("btcpc-family:{}:{}:{}", account, chain, addr);
            json!({
                "chain": chain,
                "address": addr,
                "derivation_path": dpath,
                "signature": kp.sign_entry_json(&msg),
            })
        })
        .collect();

    let publish_entry = json!({
        "type": "WALLET_FAMILY_ADD",
        "account": account,
        "chains": chain_entries,
        "epoch": epoch,
        "nonce": nonce,
        "signed_by": account,
    });
    let sig = kp.sign_entry_json(&serde_json::to_string(&publish_entry)?);

    let resp: Value = api.post("/api/entry", &json!({
        "entry": publish_entry,
        "signature": sig,
    }))?;

    if resp.get("accepted").and_then(|v| v.as_bool()).unwrap_or(false) {
        println!("{}", "WalletFamilyAdd accepted.".green());
        for (chain, addr, _) in &addresses {
            println!("  {:10} {}", chain, addr);
        }
    } else {
        let err = resp.get("error").and_then(|v| v.as_str()).unwrap_or("unknown");
        return Err(anyhow!("WalletFamilyAdd rejected: {}", err));
    }

    Ok(())
}

fn resolve_wallet_path(input: Option<&Path>) -> Result<PathBuf> {
    if let Some(p) = input {
        return Ok(p.to_path_buf());
    }
    let home = std::env::var("HOME")
        .map_err(|_| anyhow!("HOME not set; pass --output / --wallet-file explicitly"))?;
    Ok(PathBuf::from(home).join(".btcpc").join("wallet.json"))
}
