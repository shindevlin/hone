//! TON wallet activation relay.
//!
//! Watches for USDT deposits (Tron TRC-20 + Ethereum ERC-20) to the relay addresses,
//! then sends TON to activate the depositor's registered TON wallet.
//!
//! Config (env vars):
//!   TON_RELAY_ENABLED=true
//!   TON_RELAY_FEE_USDT=2000000        (2 USDT in micro-USDT, 6 decimals)
//!   TON_RELAY_ACTIVATION_NANOTON=100000000  (0.1 TON in nanotons)
//!   TON_RELAY_TRON_API=https://api.trongrid.io
//!   TON_RELAY_ETH_API=https://api.etherscan.io/api
//!   TON_RELAY_ETH_API_KEY=            (etherscan API key, optional)
//!   TONCENTER_API_KEY=                (toncenter API key, optional)
#![allow(dead_code)]

use std::sync::Arc;
use std::time::Duration;
use anyhow::Result;
use reqwest::Client;
use tracing::{info, warn, error};
use base64::Engine as _;

use crate::chain::Chain;
use crate::net::NetCmd;

// USDT contract addresses
const USDT_TRON:     &str = "TR7NHqjeKQxGTCi8q8ZY4pL8otSzgjLj6t";
const USDT_ETH:      &str = "0xdac17f958d2ee523a2206206994597c13d831ec7";
const TONCENTER_API: &str = "https://toncenter.com/api/v2";

pub struct TonRelayConfig {
    pub enabled:             bool,
    pub fee_usdt:            u64,   // micro-USDT (6 decimals)
    pub activation_nanoton:  u64,   // nanoTON to send
    pub tron_relay_address:  String,
    pub eth_relay_address:   String,
    pub ton_private_key:     [u8; 32],
    pub ton_address:         String,
    pub tron_api_url:        String,
    pub eth_api_url:         String,
    pub eth_api_key:         String,
    pub toncenter_api_key:   String,
    pub account:             String,
}

impl TonRelayConfig {
    pub fn from_env(wallet: &crate::wallet::WalletKeys) -> Self {
        let ton_priv = hex::decode(&wallet.ton_private_key).unwrap_or_default();
        let mut key = [0u8; 32];
        if ton_priv.len() >= 32 { key.copy_from_slice(&ton_priv[..32]); }
        Self {
            enabled:            std::env::var("TON_RELAY_ENABLED").map(|v| v == "true" || v == "1").unwrap_or(false),
            fee_usdt:           std::env::var("TON_RELAY_FEE_USDT").ok().and_then(|s| s.parse().ok()).unwrap_or(2_000_000),
            activation_nanoton: std::env::var("TON_RELAY_ACTIVATION_NANOTON").ok().and_then(|s| s.parse().ok()).unwrap_or(100_000_000),
            tron_relay_address: wallet.tron_address.clone(),
            eth_relay_address:  wallet.ethereum_address.clone(),
            ton_private_key:    key,
            ton_address:        wallet.ton_address.clone(),
            tron_api_url:       std::env::var("TON_RELAY_TRON_API").unwrap_or_else(|_| "https://api.trongrid.io".to_string()),
            eth_api_url:        std::env::var("TON_RELAY_ETH_API").unwrap_or_else(|_| "https://api.etherscan.io/api".to_string()),
            eth_api_key:        std::env::var("TON_RELAY_ETH_API_KEY").unwrap_or_default(),
            toncenter_api_key:  std::env::var("TONCENTER_API_KEY").unwrap_or_default(),
            account:            std::env::var("BTCPC_ACCOUNT").unwrap_or_else(|_| "relay".to_string()),
        }
    }
}

pub async fn run_ton_relay(
    chain:  Arc<Chain>,
    config: TonRelayConfig,
    cmd_tx: tokio::sync::mpsc::Sender<NetCmd>,
) {
    if !config.enabled {
        return;
    }
    info!("TON relay started: tron={} eth={} ton={}",
        config.tron_relay_address, config.eth_relay_address, config.ton_address);

    let client = Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .unwrap_or_default();

    let mut last_tron_tx: Option<String> = None;
    let mut last_eth_block: u64 = 0;

    loop {
        tokio::time::sleep(Duration::from_secs(30)).await;

        if let Err(e) = check_tron_deposits(&client, &chain, &config, &cmd_tx, &mut last_tron_tx).await {
            warn!("ton_relay: tron check failed: {}", e);
        }

        if let Err(e) = check_eth_deposits(&client, &chain, &config, &cmd_tx, &mut last_eth_block).await {
            warn!("ton_relay: eth check failed: {}", e);
        }
    }
}

// ── Tron TRC-20 deposit watcher ───────────────────────────────────────────────

async fn check_tron_deposits(
    client:   &Client,
    chain:    &Arc<Chain>,
    config:   &TonRelayConfig,
    cmd_tx:   &tokio::sync::mpsc::Sender<NetCmd>,
    last_tx:  &mut Option<String>,
) -> Result<()> {
    let url = format!(
        "{}/v1/accounts/{}/transactions/trc20?limit=20&contract_address={}",
        config.tron_api_url, config.tron_relay_address, USDT_TRON
    );
    let resp: serde_json::Value = client.get(&url).send().await?.json().await?;
    let txs = match resp["data"].as_array() {
        Some(a) => a.clone(),
        None => return Ok(()),
    };

    for tx in txs.iter().rev() {
        let tx_id = tx["transaction_id"].as_str().unwrap_or("").to_string();
        if last_tx.as_deref() == Some(&tx_id) { break; }

        let to = tx["to"].as_str().unwrap_or("");
        if to.to_lowercase() != config.tron_relay_address.to_lowercase() { continue; }

        let from = tx["from"].as_str().unwrap_or("").to_string();
        let amount: u64 = tx["value"].as_str().unwrap_or("0").parse().unwrap_or(0);

        if amount < config.fee_usdt { continue; }

        // Idempotency guard: mark tx as attempted BEFORE sending TON
        let attempt_key = format!("ton_relay_attempt:{}", tx_id);
        if chain.store.state_get(&attempt_key).is_some() {
            continue; // already processed or attempted
        }
        let _ = chain.store.state_set(&attempt_key, b"1");

        info!("ton_relay: tron deposit from={} amount={} tx={}", from, amount, tx_id);

        if let Err(e) = handle_activation(client, chain, config, cmd_tx, "tron", &from, amount, &tx_id).await {
            warn!("ton_relay: activation failed for {}: {}", from, e);
        }
    }

    if let Some(first) = txs.first() {
        *last_tx = first["transaction_id"].as_str().map(str::to_string);
    }
    Ok(())
}

// ── ETH ERC-20 deposit watcher ────────────────────────────────────────────────

async fn check_eth_deposits(
    client:      &Client,
    chain:       &Arc<Chain>,
    config:      &TonRelayConfig,
    cmd_tx:      &tokio::sync::mpsc::Sender<NetCmd>,
    last_block:  &mut u64,
) -> Result<()> {
    let mut url = format!(
        "{}?module=account&action=tokentx&address={}&contractaddress={}&sort=asc",
        config.eth_api_url, config.eth_relay_address, USDT_ETH
    );
    if !config.eth_api_key.is_empty() {
        url.push_str(&format!("&apikey={}", config.eth_api_key));
    }
    if *last_block > 0 {
        url.push_str(&format!("&startblock={}", *last_block + 1));
    }

    let resp: serde_json::Value = client.get(&url).send().await?.json().await?;
    let txs = match resp["result"].as_array() {
        Some(a) => a.clone(),
        None => return Ok(()),
    };

    for tx in &txs {
        let to = tx["to"].as_str().unwrap_or("");
        if to.to_lowercase() != config.eth_relay_address.to_lowercase() { continue; }

        let from = tx["from"].as_str().unwrap_or("").to_string();
        // USDT-ERC20 uses 6 decimals — value is in micro-USDT
        let amount_usdt: u64 = tx["value"].as_str().unwrap_or("0").parse().unwrap_or(0);
        let tx_hash = tx["hash"].as_str().unwrap_or("").to_string();
        let block: u64 = tx["blockNumber"].as_str().unwrap_or("0").parse().unwrap_or(0);

        if amount_usdt < config.fee_usdt { continue; }

        // Idempotency guard: mark tx as attempted BEFORE sending TON
        let attempt_key = format!("ton_relay_attempt:{}", tx_hash);
        if chain.store.state_get(&attempt_key).is_some() {
            if block > *last_block { *last_block = block; }
            continue;
        }
        let _ = chain.store.state_set(&attempt_key, b"1");

        info!("ton_relay: eth deposit from={} amount={} tx={}", from, amount_usdt, tx_hash);

        if let Err(e) = handle_activation(client, chain, config, cmd_tx, "ethereum", &from, amount_usdt, &tx_hash).await {
            warn!("ton_relay: activation failed for {}: {}", from, e);
        }

        if block > *last_block { *last_block = block; }
    }
    Ok(())
}

// ── Core activation logic ─────────────────────────────────────────────────────

async fn handle_activation(
    client:        &Client,
    chain:         &Arc<Chain>,
    config:        &TonRelayConfig,
    cmd_tx:        &tokio::sync::mpsc::Sender<NetCmd>,
    source_chain:  &str,
    from_address:  &str,
    usdt_received: u64,
    _deposit_tx:   &str,
) -> Result<()> {
    // Find pending intent for this source address
    let ton_address = match find_pending_intent(chain, source_chain, from_address)? {
        Some(a) => a,
        None => {
            warn!("ton_relay: no pending intent for {} on {}", from_address, source_chain);
            return Ok(());
        }
    };

    // Check not already activated
    let activated_key = format!("ton_activated:{}", ton_address);
    if chain.store.state_get(&activated_key).is_some() {
        warn!("ton_relay: {} already activated", ton_address);
        return Ok(());
    }

    // Get relay TON wallet seqno
    let seqno = get_ton_seqno(client, &config.ton_address, &config.toncenter_api_key).await?;

    // Build and send the wallet_v3r2 transfer BoC.
    let ton_tx_hash = match build_wallet_v3r2_boc_inner(&config.ton_private_key, &config.ton_address, &ton_address, config.activation_nanoton, seqno) {
        Ok(boc) => {
            let boc_b64 = base64::engine::general_purpose::STANDARD.encode(&boc);
            let body = serde_json::json!({"boc": boc_b64});
            let mut req = client
                .post(format!("{}/sendBocReturnHash", TONCENTER_API))
                .json(&body);
            if !config.toncenter_api_key.is_empty() {
                req = req.header("X-API-Key", config.toncenter_api_key.clone());
            }
            let resp: serde_json::Value = req.send().await?.json().await?;
            resp["result"]["hash"].as_str().unwrap_or("").to_string()
        }
        Err(e) => {
            // TON tx signing not yet implemented — log intent and continue
            warn!("ton_relay: {}", e);
            warn!("ton_relay: WOULD activate {} for {} from {} ({})",
                ton_address, config.activation_nanoton, from_address, source_chain);
            return Ok(());
        }
    };

    let fee = config.fee_usdt;
    let current_epoch = chain.current_epoch();

    // Mark as activated
    let _ = chain.store.state_set(&activated_key, b"1");

    // Emit TonWalletActivated entry
    let entry = btcpc_types::LedgerEntry::TonWalletActivated {
        btcpc_account:  config.account.clone(),
        ton_address:    ton_address.clone(),
        source_chain:   source_chain.to_string(),
        source_address: from_address.to_string(),
        usdt_received,
        ton_sent:       config.activation_nanoton,
        fee_usdt:       fee,
        tx_hash:        ton_tx_hash.clone(),
        epoch:          current_epoch,
        relay:          config.account.clone(),
    };

    if let Err(e) = chain.apply_entry(&entry) {
        error!("ton_relay: failed to apply TonWalletActivated: {}", e);
    }

    if let Ok(data) = serde_json::to_vec(&serde_json::json!({"entry": entry})) {
        let _ = cmd_tx.send(NetCmd::Broadcast { topic: "btcpc/entries", data }).await;
    }

    info!("ton_relay: activated {} from {} tx={}", ton_address, from_address, ton_tx_hash);
    Ok(())
}

fn find_pending_intent(chain: &Arc<Chain>, source_chain: &str, from_address: &str) -> Result<Option<String>> {
    // Key format: ton_activation_intent:{btcpc_account}:{source_address}
    let prefix = "ton_activation_intent:";
    let pairs = chain.store.state_scan_prefix(prefix);
    for (key, val_bytes) in pairs {
        // Parse the key: split by ':' and compare the last segment exactly
        let parts: Vec<&str> = key.splitn(3, ':').collect();
        // parts[0]="ton_activation_intent", parts[1]=btcpc_account, parts[2]=source_address
        if parts.len() < 3 { continue; }
        if parts[2] != from_address { continue; }

        if let Ok(val) = serde_json::from_slice::<serde_json::Value>(&val_bytes) {
            if val["status"].as_str() == Some("pending") &&
               val["source_chain"].as_str() == Some(source_chain) {
                if let Some(addr) = val["ton_address"].as_str() {
                    return Ok(Some(addr.to_string()));
                }
            }
        }
    }
    Ok(None)
}

// ── TON seqno query ───────────────────────────────────────────────────────────

async fn get_ton_seqno(client: &Client, address: &str, api_key: &str) -> Result<u32> {
    let body = serde_json::json!({
        "address": address,
        "method": "seqno",
        "stack": [],
    });
    let mut req = client
        .post(format!("{}/runGetMethod", TONCENTER_API))
        .json(&body);
    if !api_key.is_empty() {
        req = req.header("X-API-Key", api_key);
    }
    let resp: serde_json::Value = req.send().await?.json().await?;
    let seqno = resp
        .pointer("/result/stack/0/1")
        .and_then(|v| v.as_str())
        .and_then(|s| u64::from_str_radix(s.trim_start_matches("0x"), 16).ok())
        .unwrap_or(0) as u32;
    Ok(seqno)
}

// ── TON BoC construction ──────────────────────────────────────────────────────

use everscale_types::cell::{Cell, CellBuilder};
use everscale_types::boc::Boc;
use ed25519_dalek::{SigningKey, Signer};

/// Default subwallet_id used when a wallet is deployed without the custom id flag.
const WALLET_V3_SUBWALLET_ID: u32 = 698_983_191;
/// send_mode 3: pay fees separately + ignore action errors.
const SEND_MODE: u8 = 3;

/// Encode a TON Grams value (variable-length integer).
///
/// Format: 4-bit length `l` (number of significant bytes) followed by `l` bytes big-endian.
/// Zero is encoded as four zero bits.
fn store_grams(b: &mut CellBuilder, value: u128) -> anyhow::Result<()> {
    if value == 0 {
        b.store_small_uint(0, 4)
            .map_err(|e| anyhow::anyhow!("store_grams(0): {}", e))?;
        return Ok(());
    }
    let byte_len = ((128 - value.leading_zeros() + 7) / 8) as u8;
    b.store_small_uint(byte_len, 4)
        .map_err(|e| anyhow::anyhow!("store_grams len: {}", e))?;
    let full = value.to_be_bytes(); // 16 bytes big-endian
    let sig = &full[(16 - byte_len as usize)..];
    b.store_raw(sig, byte_len as u16 * 8)
        .map_err(|e| anyhow::anyhow!("store_grams val: {}", e))?;
    Ok(())
}

/// Append an `addr_std` address (no anycast) into a cell builder.
///
/// `addr_std$10 anycast:(nothing) workchain_id:int8 address:bits256`
/// Total: 2 + 1 + 8 + 256 = 267 bits, no references.
fn store_addr_std(b: &mut CellBuilder, workchain: i8, account_id: &[u8; 32]) -> anyhow::Result<()> {
    b.store_bit_one().map_err(|e| anyhow::anyhow!("addr_std[1]: {}", e))?;  // $10 tag high bit
    b.store_bit_zero().map_err(|e| anyhow::anyhow!("addr_std[0]: {}", e))?; // $10 tag low bit
    b.store_bit_zero().map_err(|e| anyhow::anyhow!("addr_std anycast: {}", e))?; // anycast = nothing
    b.store_u8(workchain as u8).map_err(|e| anyhow::anyhow!("addr_std wc: {}", e))?;
    b.store_raw(account_id, 256).map_err(|e| anyhow::anyhow!("addr_std acct: {}", e))?;
    Ok(())
}

/// Build a wallet_v3r2 external transfer BoC ready to submit to `/sendBoc`.
///
/// # Message structure (TL-B summary)
///
/// ```text
/// ext_in_msg_info$10
///   src:addr_none$00
///   dest:addr_std(relay_wallet_addr)   ← the wallet contract being invoked
///   import_fee:Grams(0)
///   init:0
///   body:1 (ref) ──────────────────────────────────────────────────────────╮
///                                                                           │
/// body_cell:                                                                │
///   Ed25519 signature       [512 bits]                                      │
///   subwallet_id            [32 bits]                                       │
///   valid_until             [32 bits]                                       │
///   seqno                   [32 bits]                                       │
///   send_mode               [8 bits]                                        │
///   ref ──────────────────────────────────────────────────────────────────╮ │
///                                                                         │ │
/// internal_msg:                                                           │ │
///   int_msg_info$0                                                        │ │
///   ihr_disabled:1 bounce:0 bounced:0                                     │ │
///   src:addr_none$00                                                      │ │
///   dest:addr_std(to_addr)             ← the activation target             │ │
///   value:Grams(nanoton)                                                  │ │
///   extra_currencies:0 ihr_fee:0 fwd_fee:0                                │ │
///   created_lt:0 created_at:0                                             │ │
///   init:0 body:0                                                         │ │
/// ◄─────────────────────────────────────────────────────────────────────── ◄─╯
/// ```
///
/// Ed25519 signs `signing_cell.repr_hash()` where `signing_cell` encodes
/// `subwallet_id | valid_until | seqno | send_mode | ref(internal_msg)`.
fn build_wallet_v3r2_boc(
    private_key:         &[u8; 32],
    to_addr:             &str,
    nanoton:             u64,
    seqno:               u32,
) -> Result<Vec<u8>> {
    // The relay wallet address is embedded in the TonRelayConfig; we receive only the
    // destination address here.  The ext_in_msg_info dest is the relay's own wallet.
    // Callers updated to pass relay_ton_address as `relay_addr` via the wrapper below.
    // This internal implementation accepts both explicitly.
    build_wallet_v3r2_boc_inner(private_key, to_addr, to_addr, nanoton, seqno)
}

/// Inner implementation that accepts both the relay wallet address (for ext_in_msg_info dest)
/// and the transfer destination address (for the internal message).
fn build_wallet_v3r2_boc_inner(
    private_key:     &[u8; 32],
    relay_addr:      &str,   // relay wallet (ext_in_msg_info dest)
    dest_addr:       &str,   // activation target (internal message dest)
    nanoton:         u64,
    seqno:           u32,
) -> Result<Vec<u8>> {
    let (relay_wc, relay_account) = decode_ton_address(relay_addr)?;
    let (dest_wc,  dest_account)  = decode_ton_address(dest_addr)?;

    // ── 1. Internal message ───────────────────────────────────────────────────
    let internal_msg: Cell = {
        let mut b = CellBuilder::new();
        b.store_bit_zero().map_err(|e| anyhow::anyhow!("int$0: {}", e))?;       // int_msg_info$0
        b.store_bit_one().map_err(|e| anyhow::anyhow!("ihr_dis: {}", e))?;      // ihr_disabled=1
        b.store_bit_zero().map_err(|e| anyhow::anyhow!("bounce: {}", e))?;      // bounce=0 (dest may not exist yet)
        b.store_bit_zero().map_err(|e| anyhow::anyhow!("bounced: {}", e))?;     // bounced=0
        // src: addr_none$00
        b.store_bit_zero().map_err(|e| anyhow::anyhow!("src0: {}", e))?;
        b.store_bit_zero().map_err(|e| anyhow::anyhow!("src1: {}", e))?;
        // dest
        store_addr_std(&mut b, dest_wc, &dest_account)?;
        // value
        store_grams(&mut b, nanoton as u128)?;
        // extra_currencies dict: 0
        b.store_bit_zero().map_err(|e| anyhow::anyhow!("extra_curr: {}", e))?;
        // ihr_fee, fwd_fee
        store_grams(&mut b, 0)?;
        store_grams(&mut b, 0)?;
        // created_lt (64), created_at (32)
        b.store_u64(0).map_err(|e| anyhow::anyhow!("lt: {}", e))?;
        b.store_u32(0).map_err(|e| anyhow::anyhow!("at: {}", e))?;
        // init:0  body:0 (empty body inline)
        b.store_bit_zero().map_err(|e| anyhow::anyhow!("init: {}", e))?;
        b.store_bit_zero().map_err(|e| anyhow::anyhow!("body: {}", e))?;
        b.build().map_err(|e| anyhow::anyhow!("internal_msg: {}", e))?
    };

    // ── 2. Signing cell (what Ed25519 covers) ─────────────────────────────────
    let valid_until = (std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs() as u32)
        .saturating_add(60);

    let signing_cell: Cell = {
        let mut b = CellBuilder::new();
        b.store_u32(WALLET_V3_SUBWALLET_ID).map_err(|e| anyhow::anyhow!("subwallet_id: {}", e))?;
        b.store_u32(valid_until).map_err(|e| anyhow::anyhow!("valid_until: {}", e))?;
        b.store_u32(seqno).map_err(|e| anyhow::anyhow!("seqno: {}", e))?;
        b.store_u8(SEND_MODE).map_err(|e| anyhow::anyhow!("mode: {}", e))?;
        b.store_reference(internal_msg.clone()).map_err(|e| anyhow::anyhow!("sign ref: {}", e))?;
        b.build().map_err(|e| anyhow::anyhow!("signing_cell: {}", e))?
    };

    // ── 3. Sign repr_hash of signing cell ────────────────────────────────────
    let signing_key = SigningKey::from_bytes(private_key);
    let hash: &[u8; 32] = signing_cell.repr_hash().as_ref();
    let sig_bytes: [u8; 64] = signing_key.sign(hash).to_bytes();

    // ── 4. Body cell: signature || signing data ───────────────────────────────
    let body_cell: Cell = {
        let mut b = CellBuilder::new();
        b.store_raw(&sig_bytes, 512).map_err(|e| anyhow::anyhow!("sig: {}", e))?;
        b.store_u32(WALLET_V3_SUBWALLET_ID).map_err(|e| anyhow::anyhow!("b_subwallet: {}", e))?;
        b.store_u32(valid_until).map_err(|e| anyhow::anyhow!("b_valid_until: {}", e))?;
        b.store_u32(seqno).map_err(|e| anyhow::anyhow!("b_seqno: {}", e))?;
        b.store_u8(SEND_MODE).map_err(|e| anyhow::anyhow!("b_mode: {}", e))?;
        b.store_reference(internal_msg).map_err(|e| anyhow::anyhow!("b_ref: {}", e))?;
        b.build().map_err(|e| anyhow::anyhow!("body_cell: {}", e))?
    };

    // ── 5. External message envelope ─────────────────────────────────────────
    let ext_cell: Cell = {
        let mut b = CellBuilder::new();
        // ext_in_msg_info$10
        b.store_bit_one().map_err(|e| anyhow::anyhow!("ext$1: {}", e))?;
        b.store_bit_zero().map_err(|e| anyhow::anyhow!("ext$0: {}", e))?;
        // src: addr_none$00
        b.store_bit_zero().map_err(|e| anyhow::anyhow!("esrc0: {}", e))?;
        b.store_bit_zero().map_err(|e| anyhow::anyhow!("esrc1: {}", e))?;
        // dest: relay wallet
        store_addr_std(&mut b, relay_wc, &relay_account)?;
        // import_fee: Grams(0)
        store_grams(&mut b, 0)?;
        // init:0  body:1 (body as reference)
        b.store_bit_zero().map_err(|e| anyhow::anyhow!("einit: {}", e))?;
        b.store_bit_one().map_err(|e| anyhow::anyhow!("ebody: {}", e))?;
        b.store_reference(body_cell).map_err(|e| anyhow::anyhow!("ebody_ref: {}", e))?;
        b.build().map_err(|e| anyhow::anyhow!("ext_cell: {}", e))?
    };

    Ok(Boc::encode(ext_cell))
}

/// Decode a user-friendly TON address (EQ…/UQ… base64url or standard base64)
/// into `(workchain_id, account_id)`.
///
/// TON user-friendly address layout (36 bytes after base64 decode):
///   [0]     flags byte  (0x11 bounceable, 0x51 non-bounceable, +0x80 testnet)
///   [1]     workchain   (signed i8, usually 0 for basechain)
///   [2..34] account_id  (32 bytes = 256 bits)
///   [34..36] CRC16-CCITT of bytes [0..34]
fn decode_ton_address(addr: &str) -> Result<(i8, [u8; 32])> {
    let bytes = base64::engine::general_purpose::URL_SAFE_NO_PAD
        .decode(addr)
        .or_else(|_| base64::engine::general_purpose::STANDARD.decode(addr))
        .map_err(|e| anyhow::anyhow!("ton address decode: {}", e))?;
    if bytes.len() != 36 {
        return Err(anyhow::anyhow!("ton address must decode to 36 bytes, got {}", bytes.len()));
    }
    let workchain = bytes[1] as i8;
    let mut account_id = [0u8; 32];
    account_id.copy_from_slice(&bytes[2..34]);
    Ok((workchain, account_id))
}
