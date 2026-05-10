//! ENS (Ethereum Name Service) resolution for LinkGit identity.
//!
//! Lets users use `name.eth` as their LinkGit owner:
//!   git clone https://git.btcpc.net/git/vitalik.eth/myrepo
//!
//! Resolution order:
//!   1. ENS `btcpc` text record → BTCPC account name (explicit, fastest)
//!   2. ENS addr record → ETH address → `wallet_addr:eth:{addr}` chain index
//!
//! Set `BTCPC_ETH_RPC` to point at a custom Ethereum JSON-RPC endpoint.
//! Default: https://cloudflare-eth.com  (no API key required).
#![allow(dead_code)]

#![allow(dead_code)]
use anyhow::Result;
use sha3::{Keccak256, Digest};
use serde_json::json;

use crate::chain::Chain;

// ── ENS contract addresses (Ethereum mainnet) ─────────────────────────────────

const ENS_REGISTRY: &str = "0x00000000000C2E074eC69A0dFb2997BA6C7d2e1e";
const ETH_RPC_DEFAULT: &str = "https://ethereum.publicnode.com";

fn rpc_url() -> String {
    std::env::var("BTCPC_ETH_RPC").unwrap_or_else(|_| ETH_RPC_DEFAULT.to_owned())
}

// ── Function selectors (EIP-137 / EIP-634) ────────────────────────────────────
//   resolver(bytes32)   → 0x0178b8bf
//   addr(bytes32)       → 0x3b3b57de
//   text(bytes32,string)→ 0x59d1d43c
//   name(bytes32)       → 0x691f3431  (reverse resolver)

// ── Namehash (EIP-137) ────────────────────────────────────────────────────────

/// Compute the ENS namehash for `name` (e.g. "vitalik.eth").
pub fn namehash(name: &str) -> [u8; 32] {
    let mut node = [0u8; 32];
    if name.is_empty() { return node; }
    for label in name.split('.').rev() {
        let label_hash: [u8; 32] = Keccak256::digest(label.as_bytes()).into();
        let mut h = Keccak256::new();
        h.update(node);
        h.update(label_hash);
        node = h.finalize().into();
    }
    node
}

// ── ABI encoding helpers ──────────────────────────────────────────────────────

fn abi_bytes32(selector: [u8; 4], arg: &[u8; 32]) -> Vec<u8> {
    let mut out = selector.to_vec();
    out.extend_from_slice(arg);
    out
}

/// selector + bytes32 + string (dynamic tail)
fn abi_bytes32_string(selector: [u8; 4], arg0: &[u8; 32], arg1: &str) -> Vec<u8> {
    let mut out = selector.to_vec();
    out.extend_from_slice(arg0);                        // arg0: bytes32
    let mut offset = [0u8; 32];                         // arg1 offset = 0x40 (64)
    offset[24..].copy_from_slice(&64u64.to_be_bytes());
    out.extend_from_slice(&offset);
    let s = arg1.as_bytes();
    let mut len_word = [0u8; 32];                       // string length
    len_word[24..].copy_from_slice(&(s.len() as u64).to_be_bytes());
    out.extend_from_slice(&len_word);
    out.extend_from_slice(s);                           // string bytes, padded
    let pad = (32 - s.len() % 32) % 32;
    out.extend(std::iter::repeat(0u8).take(pad));
    out
}

// ── ABI decoding helpers ──────────────────────────────────────────────────────

/// Decode a 32-byte ABI word as an Ethereum address ("0x{hex}").
/// Returns None for zero address.
fn decode_address(bytes: &[u8]) -> Option<String> {
    if bytes.len() < 32 { return None; }
    let addr = &bytes[12..32];
    if addr.iter().all(|b| *b == 0) { return None; }
    Some(format!("0x{}", hex::encode(addr)))
}

/// Decode an ABI-encoded `string` return value.
fn decode_string(bytes: &[u8]) -> Option<String> {
    if bytes.len() < 32 { return None; }
    let offset = u64::from_be_bytes(bytes[24..32].try_into().ok()?) as usize;
    if offset + 32 > bytes.len() { return None; }
    let len = u64::from_be_bytes(bytes[offset + 24..offset + 32].try_into().ok()?) as usize;
    let start = offset + 32;
    if start + len > bytes.len() { return None; }
    let s = String::from_utf8(bytes[start..start + len].to_vec()).ok()?;
    if s.is_empty() { None } else { Some(s) }
}

// ── Ethereum JSON-RPC ─────────────────────────────────────────────────────────

async fn eth_call(to: &str, calldata: &[u8]) -> Result<Vec<u8>> {
    let client = reqwest::Client::new();
    let hex_data = format!("0x{}", hex::encode(calldata));
    let body = json!({
        "jsonrpc": "2.0",
        "method":  "eth_call",
        "params":  [{"to": to, "data": hex_data}, "latest"],
        "id":      1
    });
    let resp = client
        .post(rpc_url())
        .json(&body)
        .timeout(std::time::Duration::from_secs(6))
        .send().await?
        .json::<serde_json::Value>().await?;
    if let Some(err) = resp.get("error") {
        anyhow::bail!("eth_call error: {}", err);
    }
    let hex = resp["result"].as_str().unwrap_or("0x");
    Ok(hex::decode(hex.trim_start_matches("0x"))?)
}

// ── ENS resolution ────────────────────────────────────────────────────────────

/// Resolve the resolver contract for a name node.
async fn get_resolver(node: &[u8; 32]) -> Option<String> {
    let cd = abi_bytes32([0x01, 0x78, 0xb8, 0xbf], node);
    let result = eth_call(ENS_REGISTRY, &cd).await.ok()?;
    decode_address(&result)
}

/// Resolve the `text` record for `key` on `name` (e.g. key = "btcpc").
pub async fn resolve_text(name: &str, key: &str) -> Option<String> {
    let node = namehash(name);
    let resolver = get_resolver(&node).await?;
    let cd = abi_bytes32_string([0x59, 0xd1, 0xd4, 0x3c], &node, key);
    let result = eth_call(&resolver, &cd).await.ok()?;
    decode_string(&result)
}

/// Resolve the ETH address (`addr` record) for `name`.
pub async fn resolve_addr(name: &str) -> Option<String> {
    let node = namehash(name);
    let resolver = get_resolver(&node).await?;
    let cd = abi_bytes32([0x3b, 0x3b, 0x57, 0xde], &node);
    let result = eth_call(&resolver, &cd).await.ok()?;
    decode_address(&result)
}

/// Reverse-resolve an ETH address to its primary ENS name.
pub async fn reverse_resolve(eth_addr: &str) -> Option<String> {
    let addr = eth_addr.trim_start_matches("0x").to_lowercase();
    let reverse_name = format!("{}.addr.reverse", addr);
    let node = namehash(&reverse_name);
    let resolver = get_resolver(&node).await?;
    let cd = abi_bytes32([0x69, 0x1f, 0x34, 0x31], &node);
    let result = eth_call(&resolver, &cd).await.ok()?;
    decode_string(&result)
}

// ── BTCPC ↔ ENS bridge ────────────────────────────────────────────────────────

/// Given an ENS name, find the linked BTCPC account.
///
/// Resolution order:
///   1. `btcpc` text record → account name (user sets this on their ENS name)
///   2. addr record → ETH address → `wallet_addr:eth:{addr}` chain reverse index
///      (populated by WalletFamilyPublish on BTCPC)
pub async fn btcpc_account_for_ens(chain: &Chain, ens_name: &str) -> Option<String> {
    // Fast path: "btcpc" text record directly names the account.
    if let Some(account) = resolve_text(ens_name, "btcpc").await {
        let account = account.trim().to_lowercase();
        if chain.store.state_get(&format!("account:{}", account)).is_some() {
            return Some(account);
        }
    }

    // Fallback: addr record → wallet family reverse index.
    let eth_addr = resolve_addr(ens_name).await?;
    let addr_lower = eth_addr.to_lowercase();

    // Try "eth" and "ethereum" as chain identifiers (both are used in the wild).
    for chain_key in &["eth", "ethereum"] {
        let rev_key = format!("wallet_addr:{}:{}", chain_key, addr_lower);
        if let Some(bytes) = chain.store.state_get(&rev_key) {
            if let Ok(account) = String::from_utf8(bytes) {
                return Some(account);
            }
        }
        // Also try with checksum address (mixed case).
        let rev_key2 = format!("wallet_addr:{}:{}", chain_key, eth_addr);
        if let Some(bytes) = chain.store.state_get(&rev_key2) {
            if let Ok(account) = String::from_utf8(bytes) {
                return Some(account);
            }
        }
    }

    None
}

/// Returns `true` if `name` looks like an ENS name (ends with `.eth` or contains `.`).
pub fn is_ens_name(name: &str) -> bool {
    name.contains('.') && !name.contains(':') && !name.contains('/')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn namehash_vitalik_eth() {
        // Known ENS namehash for "vitalik.eth" from ethers.js / reference impl
        let h = namehash("vitalik.eth");
        assert_eq!(
            hex::encode(h),
            "ee6c4522aab0003e8d14cd40a6af439055fd2577951148c14b6cea9a53475835"
        );
    }

    #[test]
    fn namehash_empty() {
        assert_eq!(namehash(""), [0u8; 32]);
    }

    #[test]
    fn namehash_eth() {
        let h = namehash("eth");
        assert_eq!(
            hex::encode(h),
            "93cdeb708b7545dc668eb9280176169d1c33cfd8ed6f04690a0bcc88a93fc4ae"
        );
    }
}
