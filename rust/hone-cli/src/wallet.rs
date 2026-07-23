use anyhow::{anyhow, Context, Result};
use hone_sdk::{KeyPair, Wallet, WalletFile};
use colored::Colorize;
use serde_json::{json, Value};
use std::io::IsTerminal;
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

    // ── Secret-exposure gate ──────────────────────────────────────────────────
    // Creating a wallet mints a 12-word mnemonic that grants FULL owner+spend
    // authority. It must be shown to a human ONCE and to no one else. If stdout or
    // stdin is not an interactive terminal, the stream is being captured (a pipe,
    // a log file, or an automation/agent harness) — refuse rather than leak the
    // seed into whatever is reading it. Vaults are meant to be created OFFLINE with
    // the standalone air-gapped keytool; a node or agent must never see the seed.
    // (See the vault/agent two-key model — the agent only ever holds a bounded key.)
    if !std::io::stdout().is_terminal() || !std::io::stdin().is_terminal() {
        return Err(anyhow!(
            "refusing to create a wallet on a non-interactive stream — the 12-word \
             mnemonic would be exposed to whatever is capturing stdout (a pipe, a log, \
             or an automation agent). Create your vault OFFLINE with the standalone \
             air-gapped keytool, then bring only the bounded agent key online. If you \
             are a human, run `hone wallet create` directly in a terminal — not through \
             a pipe, a redirect, or an agent."
        ));
    }

    // Generate ephemeral wallet from a fresh 12-word mnemonic.
    let wallet = Wallet::generate(account)?;
    let owner_kp = wallet.hone_role_keypair("owner")?;
    let posting_kp = wallet.hone_role_keypair("posting")?;
    let pubkey = posting_kp.public_key_hex();
    let role_keys = wallet.hone_role_public_keys()?;
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
    println!("{} {}", "HONE posting pubkey:".bold(), pubkey);
    println!("{}", "HONE role keys:".bold());
    for role in ["owner", "active", "posting", "memo", "hide", "seek"] {
        if let Some(pk) = role_keys.get(role) {
            println!("  {:8} {}", role, pk);
        }
    }
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
    let chain_id = node_chain_id(&api)?;
    let create_sig = sign_account_create(&owner_kp, &chain_id, account, &role_keys)?;
    let create_resp: Value = api.post("/api/account/create", &json!({
        "account": account,
        "keys": role_keys,
        "signature": create_sig,
    }))?;

    if !create_resp.get("accepted").and_then(|v| v.as_bool()).unwrap_or(false) {
        let err = create_resp.get("error").and_then(|v| v.as_str()).unwrap_or("unknown");
        return Err(anyhow!("AccountCreate rejected: {}", err));
    }
    println!("{}", "AccountCreate accepted.".green());

    // ── Submit WalletFamilyPublish ────────────────────────────────────────────
    let latest: Value = api.get("/api/latest")?;
    let epoch = latest["current_epoch"].as_u64().unwrap_or(0);
    let nonce = next_nonce(&api, account)?;

    let chain_entries: Vec<Value> = addresses
        .iter()
        .map(|(chain, addr, dpath)| {
            let msg = format!("hone-family:{}:{}:{}", account, chain, addr);
            json!({
                "chain": chain,
                "address": addr,
                "derivation_path": dpath,
                "signature": owner_kp.sign_entry_json(&msg),
            })
        })
        .collect();

    let publish_entry = json!({
        "type": "WALLET_FAMILY_PUBLISH",
        "account": account,
        "chains": chain_entries,
        "epoch": epoch,
        "nonce": nonce,
        "signed_by": account,
    });
    let sig = sign_wallet_family(
        &owner_kp,
        &chain_id,
        "WALLET_FAMILY_PUBLISH",
        account,
        &publish_entry["chains"],
        nonce,
    )?;

    let publish_resp: Value = api.post("/api/wallet/family/publish", &json!({
        "account": account,
        "chains": publish_entry["chains"].clone(),
        "epoch": epoch,
        "nonce": nonce,
        "signed_by": account,
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
    println!("{} {}", "HONE posting pubkey:".bold(), wf.hone_public_key_hex);
    println!("{} {}", "File:".bold(), path.display());
    if !wf.hone_role_public_keys.is_empty() {
        println!();
        println!("{}", "HONE role keys:".bold());
        let mut roles: Vec<_> = wf.hone_role_public_keys.iter().collect();
        roles.sort_by_key(|(role, _)| role.as_str());
        for (role, pk) in roles {
            println!("  {:8} {}", role, pk);
        }
    }
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
/// or the HONE_MNEMONIC env var) to derive the signing key.
pub fn cmd_wallet_publish(wallet_file: Option<&Path>, mnemonic: &str) -> Result<()> {
    let path = resolve_wallet_path(wallet_file)?;
    let raw = std::fs::read_to_string(&path)?;
    let wf: WalletFile = serde_json::from_str(&raw)?;
    let account = &wf.account;

    // Derive signing key from mnemonic (private key never stored).
    let wallet = Wallet::from_phrase(mnemonic, account)?;
    let owner_kp = role_or_legacy_signer(&wallet, &wf, "owner")?;

    // Build addresses from stored public data (no private keys needed for addresses).
    let mut addresses: Vec<(String, String, String)> = wf.chain_addresses
        .iter()
        .map(|(chain, addr)| {
            let dpath = match chain.as_str() {
                "evm"     => "m/44'/60'/0'/0/0",
                "solana"  => "m/44'/501'/0'/0'",
                "bitcoin" => "m/44'/0'/0'/0/0",
                "hone"   => hone_sdk::paths::HONE_POSTING_STR,
                _         => hone_sdk::paths::HONE_POSTING_STR,
            };
            (chain.clone(), addr.clone(), dpath.to_string())
        })
        .collect();
    // Also include the HONE pubkey itself.
    addresses.push(("hone".into(), wf.hone_public_key_hex.clone(), hone_sdk::paths::HONE_POSTING_STR.into()));

    let api = ApiClient::new();
    let chain_id = node_chain_id(&api)?;
    let latest: Value = api.get("/api/latest")?;
    let epoch = latest["current_epoch"].as_u64().unwrap_or(0);
    let nonce = next_nonce(&api, account)?;

    let chain_entries: Vec<Value> = addresses
        .iter()
        .map(|(chain, addr, dpath)| {
            let msg = format!("hone-family:{}:{}:{}", account, chain, addr);
            json!({
                "chain": chain,
                "address": addr,
                "derivation_path": dpath,
                "signature": owner_kp.sign_entry_json(&msg),
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
    let sig = sign_wallet_family(
        &owner_kp,
        &chain_id,
        "WALLET_FAMILY_ADD",
        account,
        &publish_entry["chains"],
        nonce,
    )?;

    let resp: Value = api.post("/api/wallet/family/add", &json!({
        "account": account,
        "chains": publish_entry["chains"].clone(),
        "epoch": epoch,
        "nonce": nonce,
        "signed_by": account,
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

/// Generate a random API key, register it on-chain, and write it to `.hone/wallet.env`.
///
/// Requires the mnemonic to sign the `AccountApiKeySet` entry. Pass via `--mnemonic`
/// or the `HONE_MNEMONIC` environment variable.
pub fn cmd_wallet_api_key_gen(
    wallet_file: Option<&Path>,
    mnemonic: &str,
    output: Option<&Path>,
) -> Result<()> {
    use rand::RngCore;

    let wf_path = resolve_wallet_path(wallet_file)?;
    let raw = std::fs::read_to_string(&wf_path)
        .with_context(|| format!("cannot read wallet file at {}", wf_path.display()))?;
    let wf: WalletFile = serde_json::from_str(&raw)
        .with_context(|| "wallet file is not valid JSON")?;
    let account = &wf.account;

    // Derive signing key from mnemonic.
    let wallet = hone_sdk::Wallet::from_phrase(mnemonic, account)?;
    let active_kp = role_or_legacy_signer(&wallet, &wf, "active")?;

    // Generate 32 random bytes → 64-char hex API key.
    let mut raw_key = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut raw_key);
    let api_key = hex::encode(raw_key);

    let api = ApiClient::new();
    let chain_id = node_chain_id(&api)?;
    let latest: Value = api.get("/api/latest")?;
    let epoch = latest["current_epoch"].as_u64().unwrap_or(0);
    let nonce = next_nonce(&api, account)?;

    let sig = sign_account_api_key_set(&active_kp, &chain_id, account, &api_key, nonce)?;

    let resp: Value = api.post("/api/account/api-key", &json!({
        "account": account,
        "api_key": api_key,
        "epoch": epoch,
        "nonce": nonce,
        "signed_by": account,
        "signature": sig,
    }))?;

    if !resp.get("accepted").and_then(|v| v.as_bool()).unwrap_or(false) {
        let err = resp.get("error").and_then(|v| v.as_str()).unwrap_or("unknown");
        return Err(anyhow!("AccountApiKeySet rejected: {}", err));
    }
    println!("{}", "AccountApiKeySet accepted — API key registered on-chain.".green());

    // Write/update .hone/wallet.env.
    let out_path = output
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| std::env::current_dir().unwrap().join(".hone").join("wallet.env"));

    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let content = format!(
        "# HONE wallet — generated by `hone wallet api-key-gen`\n\
         # Add this file to .gitignore — it contains your API key\n\
         \n\
         HONE_ACCOUNT={}\n\
         HONE_API_KEY={}\n\
         # HONE_API_URL=https://honemesh.net\n",
        account, api_key,
    );
    std::fs::write(&out_path, &content)?;

    println!("{} {}", "Written:".green().bold(), out_path.display());
    println!();
    println!("{} {}", "Account:".bold(), account);
    println!("{} {}...", "API key:".bold(), &api_key[..16]);
    println!();
    println!("Add to .gitignore:  echo '.hone/wallet.env' >> .gitignore");

    Ok(())
}

/// Write `.hone/wallet.env` in the current directory (or `output` path).
/// Reads the account name from the wallet identity file — no mnemonic needed.
/// Note: this sets HONE_API_KEY to the account name as a placeholder.
/// Run `hone wallet api-key-gen` afterward to register a secure random API key.
pub fn cmd_wallet_env(wallet_file: Option<&Path>, output: Option<&Path>) -> Result<()> {
    let wf_path = resolve_wallet_path(wallet_file)?;
    let raw = std::fs::read_to_string(&wf_path)
        .with_context(|| format!("cannot read wallet file at {}", wf_path.display()))?;
    let wf: WalletFile = serde_json::from_str(&raw)
        .with_context(|| "wallet file is not valid JSON")?;

    let out_path = output
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| std::env::current_dir().unwrap().join(".hone").join("wallet.env"));

    if let Some(parent) = out_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    if out_path.exists() {
        eprintln!(
            "{} {} already exists — overwriting",
            "warning:".yellow(),
            out_path.display()
        );
    }

    let content = format!(
        "# HONE wallet — generated by `hone wallet env`\n\
         # Add this file to .gitignore\n\
         # Run `hone wallet api-key-gen --mnemonic \"...\"` to set a secure API key\n\
         \n\
         HONE_ACCOUNT={}\n\
         HONE_API_KEY=\n\
         # HONE_API_URL=https://honemesh.net\n",
        wf.account,
    );
    std::fs::write(&out_path, &content)?;

    println!("{} {}", "Created:".green().bold(), out_path.display());
    println!();
    println!("{} {}", "Account:".bold(), wf.account);
    println!();
    println!("Next steps:");
    println!("  echo '.hone/wallet.env' >> .gitignore");
    println!("  hone faucet claim {}   # get testnet tokens", wf.account);
    println!("  hone wallet api-key-gen --mnemonic \"...\"   # register a secure API key");

    Ok(())
}

fn resolve_wallet_path(input: Option<&Path>) -> Result<PathBuf> {
    if let Some(p) = input {
        return Ok(p.to_path_buf());
    }
    let home = std::env::var("HOME")
        .map_err(|_| anyhow!("HOME not set; pass --output / --wallet-file explicitly"))?;
    Ok(PathBuf::from(home).join(".hone").join("wallet.json"))
}

fn node_chain_id(api: &ApiClient) -> Result<String> {
    let info: Value = api.get("/api/node/info")?;
    info.get("chain_id")
        .and_then(|v| v.as_str())
        .map(str::to_owned)
        .ok_or_else(|| anyhow!("node info missing chain_id"))
}

fn next_nonce(api: &ApiClient, account: &str) -> Result<u64> {
    let account_data: Value = api.get(&format!("/api/account/{}", account))?;
    let current = account_data
        .get("nonce")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| anyhow!("account '{}' has no nonce field", account))?;
    current
        .checked_add(1)
        .ok_or_else(|| anyhow!("nonce overflow for account '{}'", account))
}

fn sign_account_create(
    owner_kp: &KeyPair,
    chain_id: &str,
    account: &str,
    keys: &std::collections::HashMap<String, String>,
) -> Result<String> {
    let sorted_keys: std::collections::BTreeMap<_, _> = keys.iter().collect();
    let msg = serde_json::to_string(&json!({
        "chain_id": chain_id,
        "type": "ACCOUNT_CREATE",
        "account": account,
        "keys": sorted_keys,
        "chain_proofs": [],
        "funded_by": null,
    }))?;
    Ok(owner_kp.sign_entry_json(&msg))
}

fn sign_wallet_family(
    keypair: &KeyPair,
    chain_id: &str,
    entry_type: &str,
    account: &str,
    chains: &Value,
    nonce: u64,
) -> Result<String> {
    let msg = serde_json::to_string(&json!({
        "chain_id": chain_id,
        "type": entry_type,
        "account": account,
        "chains": chains,
        "nonce": nonce,
    }))?;
    Ok(keypair.sign_entry_json(&msg))
}

fn sign_account_api_key_set(
    keypair: &KeyPair,
    chain_id: &str,
    account: &str,
    api_key: &str,
    nonce: u64,
) -> Result<String> {
    let msg = serde_json::to_string(&json!({
        "chain_id": chain_id,
        "type": "ACCOUNT_API_KEY_SET",
        "account": account,
        "api_key": api_key,
        "nonce": nonce,
    }))?;
    Ok(keypair.sign_entry_json(&msg))
}

fn role_or_legacy_signer(wallet: &Wallet, wf: &WalletFile, role: &str) -> Result<KeyPair> {
    if let Some(expected) = wf.hone_role_public_keys.get(role) {
        let kp = wallet.hone_role_keypair(role)?;
        if kp.public_key_hex() == *expected {
            return Ok(kp);
        }
        return Err(anyhow!(
            "mnemonic doesn't match this wallet — derived {} pubkey {} but file has {}",
            role,
            kp.public_key_hex(),
            expected
        ));
    }

    let canonical = wallet.hone_role_keypair(role)?;
    if canonical.public_key_hex() == wf.hone_public_key_hex {
        return Ok(canonical);
    }

    let legacy = wallet.legacy_hone_keypair()?;
    if legacy.public_key_hex() == wf.hone_public_key_hex {
        return Ok(legacy);
    }

    Err(anyhow!(
        "mnemonic doesn't match this wallet — neither canonical {} key nor legacy key matches {}",
        role,
        wf.hone_public_key_hex
    ))
}
