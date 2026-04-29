//! Decentralized peer registry — queries Hive and TON for bootstrap multiaddrs.
//!
//! Startup sequence:
//!   1. RocksDB peer store  (instant, no network)
//!   2. Cloudflare DNS seeds (config default, fast)
//!   3. Hive registry        (this module)
//!   4. TON registry         (this module, stub until contract deployed)
//!
//! Both queries run concurrently and are best-effort — any failure returns an
//! empty list so the node starts regardless.
//!
//! # Hive setup
//! Create a Hive account named `btcpc-nodes`.  Set its `json_metadata` to:
//!   {"btcpc_peers": ["/dns4/node1.btcpc.net/tcp/6942", ...]}
//! Update it whenever the node list changes (free transaction on Hive).
//!
//! # TON setup (future)
//! Deploy a contract with a `get_peers()` get-method that returns a bytes slice
//! containing a JSON array of multiaddr strings.  Set TON_REGISTRY_CONTRACT below.

use anyhow::Result;
use reqwest::Client;
use tracing::{info, warn};

// ── Hive ──────────────────────────────────────────────────────────────────────

const HIVE_API: &str = "https://api.hive.blog";

/// Hive account whose json_metadata holds the peer list.
/// json_metadata format: {"btcpc_peers": ["/dns4/host/tcp/port", ...]}
pub const HIVE_REGISTRY_ACCOUNT: &str = "btcpc";

async fn fetch_hive_peers(client: &Client) -> Result<Vec<String>> {
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "condenser_api.get_accounts",
        "params": [[HIVE_REGISTRY_ACCOUNT]],
        "id": 1,
    });

    let resp = client
        .post(HIVE_API)
        .json(&body)
        .send()
        .await?
        .json::<serde_json::Value>()
        .await?;

    let peers = resp
        .pointer("/result/0/json_metadata")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
        .and_then(|v| {
            v.get("btcpc_peers")
                .and_then(|p| serde_json::from_value::<Vec<String>>(p.clone()).ok())
        })
        .unwrap_or_default();

    Ok(peers)
}

// ── TON ───────────────────────────────────────────────────────────────────────

const TON_CENTER_API: &str = "https://toncenter.com/api/v2";

/// TON contract address for the peer registry.
/// Contract must expose: get_peers() → bytes (UTF-8 JSON array of multiaddr strings)
/// Leave empty until the contract is deployed.
pub const TON_REGISTRY_CONTRACT: &str = "";

async fn fetch_ton_peers(client: &Client) -> Result<Vec<String>> {
    if TON_REGISTRY_CONTRACT.is_empty() {
        return Ok(vec![]);
    }

    let body = serde_json::json!({
        "address": TON_REGISTRY_CONTRACT,
        "method": "get_peers",
        "stack": [],
    });

    let resp = client
        .post(format!("{}/runGetMethod", TON_CENTER_API))
        .json(&body)
        .send()
        .await?
        .json::<serde_json::Value>()
        .await?;

    // Contract returns peers as a raw UTF-8 JSON bytes slice in stack[0][1].
    let hex_bytes = resp
        .pointer("/result/stack/0/1")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("unexpected TON response format"))?;

    let bytes = hex::decode(hex_bytes)?;
    let peers: Vec<String> = serde_json::from_slice(&bytes)?;
    Ok(peers)
}

// ── Bitcoin Ordinals ──────────────────────────────────────────────────────────
//
// Inscribe a JSON array of multiaddr strings from BTC_REGISTRY_WALLET.
// Example inscription content:
//   ["/dns4/node1.btcpc.net/tcp/6942", "/dns4/node2.btcpc.net/tcp/6942"]
//
// To update: inscribe a new JSON file from the same wallet.  The node always
// reads the most recent application/json inscription from that address.
//
// Reads are free via the Hiro API.  Writes cost one Bitcoin transaction
// (~$5–50 depending on fee market) — suitable for a slow-moving bootstrap list.

const HIRO_API: &str = "https://api.hiro.so/ordinals/v1";

/// Bitcoin wallet that publishes the peer list via Ordinals inscriptions.
/// Leave empty until the wallet is set up — the query is skipped gracefully.
pub const BTC_REGISTRY_WALLET: &str = "";

async fn fetch_btc_peers(client: &Client) -> Result<Vec<String>> {
    if BTC_REGISTRY_WALLET.is_empty() {
        return Ok(vec![]);
    }

    // Find the most recent application/json inscription from the registry wallet.
    let list_resp = client
        .get(format!("{}/inscriptions", HIRO_API))
        .query(&[
            ("address", BTC_REGISTRY_WALLET),
            ("mime_type", "application/json"),
            ("order", "desc"),
            ("limit", "1"),
        ])
        .send()
        .await?
        .json::<serde_json::Value>()
        .await?;

    let inscription_id = list_resp
        .pointer("/results/0/id")
        .and_then(|v| v.as_str())
        .ok_or_else(|| anyhow::anyhow!("no inscriptions found for BTC registry wallet"))?
        .to_owned();

    // Fetch the raw inscription content.
    let content = client
        .get(format!("{}/inscriptions/{}/content", HIRO_API, inscription_id))
        .send()
        .await?
        .bytes()
        .await?;

    let peers: Vec<String> = serde_json::from_slice(&content)?;
    Ok(peers)
}

// ── Public entry point ────────────────────────────────────────────────────────

/// Query Hive, TON, and Bitcoin Ordinals concurrently.
/// Returns merged, deduplicated list.  Never fails — any error is logged and
/// that source returns empty so the node starts regardless.
pub async fn fetch_all_peers() -> Vec<String> {
    let client = match Client::builder()
        .timeout(std::time::Duration::from_secs(10))
        .build()
    {
        Ok(c) => c,
        Err(e) => {
            warn!("discovery: failed to build HTTP client: {}", e);
            return vec![];
        }
    };

    let (hive_result, ton_result, btc_result) = tokio::join!(
        fetch_hive_peers(&client),
        fetch_ton_peers(&client),
        fetch_btc_peers(&client),
    );

    let mut peers: Vec<String> = Vec::new();

    match hive_result {
        Ok(p) if !p.is_empty() => {
            info!("discovery: Hive registry returned {} peers", p.len());
            peers.extend(p);
        }
        Ok(_) => info!("discovery: Hive registry empty (account not yet configured)"),
        Err(e) => warn!("discovery: Hive query failed: {}", e),
    }

    match ton_result {
        Ok(p) if !p.is_empty() => {
            info!("discovery: TON registry returned {} peers", p.len());
            peers.extend(p);
        }
        Ok(_) => {}
        Err(e) => warn!("discovery: TON query failed: {}", e),
    }

    match btc_result {
        Ok(p) if !p.is_empty() => {
            info!("discovery: BTC Ordinals registry returned {} peers", p.len());
            peers.extend(p);
        }
        Ok(_) => {}
        Err(e) => warn!("discovery: BTC Ordinals query failed: {}", e),
    }

    peers.sort();
    peers.dedup();
    peers
}
