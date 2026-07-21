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
//! Use the @hone account.  Set its `posting_json_metadata` to:
//!   {"hone_peers": [...mainnet...], "hone_testnet_peers": [...testnet...]}
//! Run scripts/hive-register-peers.py to publish or update peer lists.
//!
//! # TON setup (future)
//! Deploy a contract with a `get_peers()` get-method that returns a bytes slice
//! containing a JSON array of multiaddr strings.  Set TON_REGISTRY_CONTRACT below.

use anyhow::Result;
use reqwest::Client;
use tracing::{info, warn};
use hone_types::TESTNET_CHAIN_ID;

// ── Hive ──────────────────────────────────────────────────────────────────────

const HIVE_API: &str = "https://api.hive.blog";

/// Single Hive account that holds peer lists for all chains.
/// json_metadata format:
///   { "hone_peers": [...mainnet multiaddrs...], "hone_testnet_peers": [...testnet...] }
pub const HIVE_REGISTRY_ACCOUNT: &str = "hone";

/// json_metadata key for the peer list, keyed by chain_id.
pub fn hive_peers_key(chain_id: &str) -> &'static str {
    if chain_id == TESTNET_CHAIN_ID {
        "hone_testnet_peers"
    } else {
        "hone_peers"
    }
}

async fn fetch_hive_peers(client: &Client, chain_id: &str) -> Result<Vec<String>> {
    let body = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "condenser_api.get_accounts",
        "params": [[HIVE_REGISTRY_ACCOUNT]],
        "id": 1,
    });
    let peers_key = hive_peers_key(chain_id);

    let resp = client
        .post(HIVE_API)
        .json(&body)
        .send()
        .await?
        .json::<serde_json::Value>()
        .await?;

    // posting_json_metadata is writable with a posting key (account_update2).
    // json_metadata requires the active key — we use posting for the peer registry.
    let peers = resp
        .pointer("/result/0/posting_json_metadata")
        .and_then(|v| v.as_str())
        .filter(|s| !s.is_empty())
        .and_then(|s| serde_json::from_str::<serde_json::Value>(s).ok())
        .and_then(|v| {
            v.get(peers_key)
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
//   ["/dns4/node1.honemesh.net/tcp/6942", "/dns4/node2.honemesh.net/tcp/6942"]
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

// ── Hive self-announce ────────────────────────────────────────────────────────
//
// If HONE_HIVE_POSTING_KEY (WIF) is set, the node posts a custom_json to the
// Hive registry account announcing its multiaddr.  This is a best-effort
// fire-and-forget — failures are logged but never fatal.
//
// The Hive custom_json operation is broadcast via the condenser_api.
// We use a pre-built unsigned transaction body and POST it to a Hive relay that
// handles signing on behalf of the hone registry account.
// For operator-signed publishing, set HONE_HIVE_POSTING_KEY.
//
// Hive custom_json ID: "hone_peer_announce"
// json payload: {"multiaddr": "/ip4/...", "chain_id": "hone", "node_id": "..."}

/// Announce this node's multiaddr to the Hive peer registry.
/// Set HONE_ANNOUNCE_ADDR to the multiaddr to publish.
/// Set HONE_HIVE_POSTING_KEY to a WIF posting key for the registry account.
/// Both are optional — if absent, announcement is skipped silently.
pub async fn announce_to_hive(chain_id: &str, node_id: &str) {
    let multiaddr = match std::env::var("HONE_ANNOUNCE_ADDR") {
        Ok(v) if !v.is_empty() => v,
        _ => return, // no address to announce
    };
    let posting_key_wif = match std::env::var("HONE_HIVE_POSTING_KEY") {
        Ok(v) if !v.is_empty() => v,
        _ => {
            info!("discovery: HONE_HIVE_POSTING_KEY not set — skipping Hive announcement");
            return;
        }
    };

    // Both mainnet and testnet publish to the same "hone" Hive account.
    // The custom_json id encodes the chain: "hone_peer_announce" or "hone_satoshi_peer_announce".
    let announce_id = if chain_id == TESTNET_CHAIN_ID {
        "hone_satoshi_peer_announce"
    } else {
        "hone_peer_announce"
    };
    let payload = serde_json::json!({
        "multiaddr": multiaddr,
        "chain_id": chain_id,
        "node_id": node_id,
    });

    let client = match Client::builder().timeout(std::time::Duration::from_secs(15)).build() {
        Ok(c) => c,
        Err(e) => { warn!("discovery: announce: HTTP client error: {}", e); return; }
    };

    // Fetch reference block for transaction expiry.
    let props_body = serde_json::json!({
        "jsonrpc": "2.0", "method": "condenser_api.get_dynamic_global_properties", "id": 1
    });
    let props_resp = match client.post(HIVE_API).json(&props_body).send().await {
        Ok(r) => r,
        Err(e) => { warn!("discovery: announce: props fetch failed: {}", e); return; }
    };
    let props = match props_resp.json::<serde_json::Value>().await {
        Ok(v) => v,
        Err(e) => { warn!("discovery: announce: props parse failed: {}", e); return; }
    };

    let head_block_number = props.pointer("/result/head_block_number")
        .and_then(|v| v.as_u64()).unwrap_or(0) as u32;
    let head_block_id = props.pointer("/result/head_block_id")
        .and_then(|v| v.as_str()).unwrap_or("").to_string();

    // Hive ref_block_num = last 16 bits of head_block_number
    let ref_block_num = (head_block_number & 0xFFFF) as u16;
    // ref_block_prefix = bytes 4–7 of head_block_id (little-endian u32)
    let ref_block_prefix = if head_block_id.len() >= 16 {
        u32::from_le_bytes(
            hex::decode(&head_block_id[8..16]).unwrap_or_default()
                .try_into().unwrap_or([0u8; 4])
        )
    } else { 0 };

    let expiration = chrono::Utc::now() + chrono::Duration::seconds(600);
    let expiration_str = expiration.format("%Y-%m-%dT%H:%M:%S").to_string();

    let op = serde_json::json!([
        "custom_json",
        {
            "required_auths": [],
            "required_posting_auths": [HIVE_REGISTRY_ACCOUNT],
            "id": announce_id,
            "json": serde_json::to_string(&payload).unwrap_or_default(),
        }
    ]);

    // Sign the transaction using the posting key WIF.
    match sign_hive_tx(ref_block_num, ref_block_prefix, &expiration_str, op, &posting_key_wif) {
        Ok(signed_tx) => {
            let broadcast_body = serde_json::json!({
                "jsonrpc": "2.0",
                "method": "condenser_api.broadcast_transaction",
                "params": [signed_tx],
                "id": 1,
            });
            match client.post(HIVE_API).json(&broadcast_body).send().await {
                Ok(resp) => {
                    let status = resp.status();
                    let body = resp.text().await.unwrap_or_default();
                    if status.is_success() && !body.contains("\"error\"") {
                        info!("discovery: announced {} to Hive ({} chain)", multiaddr, chain_id);
                    } else {
                        warn!("discovery: Hive announce failed: {} {}", status, &body[..body.len().min(200)]);
                    }
                }
                Err(e) => warn!("discovery: Hive broadcast failed: {}", e),
            }
        }
        Err(e) => warn!("discovery: Hive tx signing failed: {}", e),
    }
}

/// Sign a Hive transaction containing a single operation.
/// Returns a JSON value suitable for condenser_api.broadcast_transaction.
fn sign_hive_tx(
    ref_block_num: u16,
    ref_block_prefix: u32,
    expiration: &str,
    op: serde_json::Value,
    posting_key_wif: &str,
) -> Result<serde_json::Value> {
    use secp256k1::{Secp256k1, Message, SecretKey};
    use sha2::{Sha256, Digest};

    // Hive uses secp256k1 ECDSA with SHA-256. The message to sign is
    // SHA-256(chain_id_bytes || serialized_tx_bytes).
    // Chain ID for Hive mainnet (BST bytes):
    let hive_chain_id = hex::decode(
        "beeab0de00000000000000000000000000000000000000000000000000000000"
    )?;

    // Minimal binary serialization of the transaction.
    // Format: ref_block_num(2 LE) + ref_block_prefix(4 LE) + expiration(4 LE as unix u32)
    //       + 0x01 (op count) + op_type(2 LE) + op_fields + 0x00 (extensions)
    // We use the condenser_api "legacy" format which expects JSON-level signing
    // via the digest approach.
    //
    // For simplicity, use the transaction digest approach:
    // sign SHA-256(SHA-256(chain_id || tx_bytes))
    let exp_unix = chrono::DateTime::parse_from_str(
        &format!("{} +0000", expiration), "%Y-%m-%dT%H:%M:%S %z"
    ).unwrap_or_default().timestamp() as u32;

    let mut tx_bytes: Vec<u8> = Vec::new();
    tx_bytes.extend_from_slice(&ref_block_num.to_le_bytes());
    tx_bytes.extend_from_slice(&ref_block_prefix.to_le_bytes());
    tx_bytes.extend_from_slice(&exp_unix.to_le_bytes());
    // For custom_json op type = 18 (0x12)
    tx_bytes.push(1); // 1 operation
    tx_bytes.extend_from_slice(&[18u8, 0u8]); // op type 18 LE u16
    // Encode custom_json fields (simplified — required_auths varint 0, required_posting_auths varint 1 + account, id string, json string)
    if let (Some(auths), Some(posting_auths), Some(id), Some(json)) = (
        op[1].get("required_auths"),
        op[1].get("required_posting_auths"),
        op[1].get("id").and_then(|v| v.as_str()),
        op[1].get("json").and_then(|v| v.as_str()),
    ) {
        let posting_auth_list: Vec<String> = serde_json::from_value(posting_auths.clone()).unwrap_or_default();
        // required_auths: varint 0
        tx_bytes.push(auths.as_array().map(|a| a.len()).unwrap_or(0) as u8);
        // required_posting_auths: varint + each account as length-prefixed string
        tx_bytes.push(posting_auth_list.len() as u8);
        for acct in &posting_auth_list {
            tx_bytes.push(acct.len() as u8);
            tx_bytes.extend_from_slice(acct.as_bytes());
        }
        // id: length-prefixed string
        tx_bytes.push(id.len() as u8);
        tx_bytes.extend_from_slice(id.as_bytes());
        // json: u16 length + bytes (Hive uses u16 for json field length)
        let json_bytes = json.as_bytes();
        tx_bytes.extend_from_slice(&(json_bytes.len() as u16).to_le_bytes());
        tx_bytes.extend_from_slice(json_bytes);
    }
    tx_bytes.push(0); // extensions: 0

    // Digest: SHA-256(chain_id || tx_bytes)
    let mut h = Sha256::new();
    h.update(&hive_chain_id);
    h.update(&tx_bytes);
    let digest = h.finalize();

    // Decode WIF private key
    let wif_bytes = bs58::decode(posting_key_wif).into_vec()
        .map_err(|e| anyhow::anyhow!("WIF decode failed: {}", e))?;
    // WIF: 1 byte version + 32 bytes key + 1 byte compression flag + 4 bytes checksum
    if wif_bytes.len() < 33 {
        anyhow::bail!("WIF too short");
    }
    let key_bytes = &wif_bytes[1..33];

    let secp = Secp256k1::new();
    let sk = SecretKey::from_slice(key_bytes)?;
    let msg = Message::from_digest(digest.into());
    let (recid, sig_bytes) = secp.sign_ecdsa_recoverable(&msg, &sk).serialize_compact();

    // Hive signature format: 1 byte (31 + recid) + 64 bytes sig
    let mut sig = Vec::with_capacity(65);
    sig.push(31 + recid.to_i32() as u8);
    sig.extend_from_slice(&sig_bytes);
    let sig_hex = hex::encode(&sig);

    Ok(serde_json::json!({
        "ref_block_num": ref_block_num,
        "ref_block_prefix": ref_block_prefix,
        "expiration": expiration,
        "operations": [op],
        "extensions": [],
        "signatures": [sig_hex],
    }))
}

// ── honemesh.net bootstrap API ───────────────────────────────────────────────────
//
// The genesis node at honemesh.net hosts a lightweight peer registry via its HTTP API.
// Nodes announce themselves on startup and fetch the list at boot.
//
// GET  https://honemesh.net/api/peers/bootstrap?chain_id=hone  → multiaddr list
// POST https://honemesh.net/api/peers/bootstrap                   → register self

const HONE_NET_API: &str = "https://honemesh.net";

async fn fetch_hone_net_peers(client: &Client, chain_id: &str) -> Result<Vec<String>> {
    let resp = client
        .get(format!("{}/api/peers/bootstrap", HONE_NET_API))
        .query(&[("chain_id", chain_id)])
        .send()
        .await?
        .json::<serde_json::Value>()
        .await?;

    let peers = resp
        .get("peers")
        .and_then(|v| serde_json::from_value::<Vec<String>>(v.clone()).ok())
        .unwrap_or_default();

    Ok(peers)
}

/// Announce an explicit multiaddr to honemesh.net (called from net.rs with peer_id built in).
pub async fn announce_to_hone_net_addr(multiaddr: &str, chain_id: &str, node_id: &str) {
    let client = match Client::builder().timeout(std::time::Duration::from_secs(10)).build() {
        Ok(c) => c,
        Err(e) => { warn!("discovery: honemesh.net announce: client error: {}", e); return; }
    };
    let payload = serde_json::json!({
        "multiaddr": multiaddr,
        "chain_id": chain_id,
        "node_id": node_id,
    });
    match client.post(format!("{}/api/peers/bootstrap", HONE_NET_API)).json(&payload).send().await {
        Ok(resp) if resp.status().is_success() =>
            info!("discovery: registered {} in peer registry", multiaddr),
        Ok(resp) => warn!("discovery: peer registry returned {}", resp.status()),
        Err(e)   => warn!("discovery: peer registry unreachable: {}", e),
    }
}

/// Announce this node's public multiaddr to honemesh.net.
/// Reads HONE_ANNOUNCE_ADDR for the multiaddr to publish.
pub async fn announce_to_hone_net(chain_id: &str, node_id: &str) {
    let multiaddr = match std::env::var("HONE_ANNOUNCE_ADDR") {
        Ok(v) if !v.is_empty() => v,
        _ => return,
    };

    let client = match Client::builder().timeout(std::time::Duration::from_secs(10)).build() {
        Ok(c) => c,
        Err(e) => { warn!("discovery: honemesh.net announce: client error: {}", e); return; }
    };

    let payload = serde_json::json!({
        "multiaddr": multiaddr,
        "chain_id": chain_id,
        "node_id": node_id,
    });

    match client
        .post(format!("{}/api/peers/bootstrap", HONE_NET_API))
        .json(&payload)
        .send()
        .await
    {
        Ok(resp) if resp.status().is_success() => {
            info!("discovery: announced {} to honemesh.net ({} chain)", multiaddr, chain_id);
        }
        Ok(resp) => warn!("discovery: honemesh.net announce returned {}", resp.status()),
        Err(e) => warn!("discovery: honemesh.net announce failed: {}", e),
    }
}

// ── Public entry point ────────────────────────────────────────────────────────

/// Query Hive, TON, Bitcoin Ordinals, and honemesh.net concurrently.
/// Returns merged, deduplicated list.  Never fails — any error is logged and
/// that source returns empty so the node starts regardless.
pub async fn fetch_all_peers(chain_id: &str) -> Vec<String> {
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

    let (hive_result, ton_result, btc_result, hone_net_result) = tokio::join!(
        fetch_hive_peers(&client, chain_id),
        fetch_ton_peers(&client),
        fetch_btc_peers(&client),
        fetch_hone_net_peers(&client, chain_id),
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

    match hone_net_result {
        Ok(p) if !p.is_empty() => {
            info!("discovery: honemesh.net registry returned {} peers", p.len());
            peers.extend(p);
        }
        Ok(_) => {}
        Err(e) => warn!("discovery: honemesh.net query failed (offline?): {}", e),
    }

    peers.sort();
    peers.dedup();
    peers
}
