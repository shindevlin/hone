use std::path::PathBuf;
use serde::{Deserialize, Serialize};
use hone_types::{AccountId, MAINNET_CHAIN_ID};

/// Cloudflare DNS seeds — update the A record when a machine changes, no code change needed.
/// Add more entries here as the network grows.
/// Public seeds every non-isolated node dials, appended to HONE_BOOTSTRAP_PEERS.
/// `/dnsaddr/bootstrap.honemesh.net` reads `_dnsaddr.bootstrap.honemesh.net` TXT records
/// (libp2p dnsaddr), so seeds are added or removed in DNS with no release. The /dns4 line
/// is the ferryvps seed itself, so a node finds the mesh even before the TXT seed exists.
/// HONE_NO_DEFAULT_BOOTSTRAP=1 drops them (lab / private test networks).
pub const DEFAULT_BOOTSTRAP_PEERS: &[&str] = &[
    "/dnsaddr/bootstrap.honemesh.net",
    // QUIC first: a UDP connection to the seed gives the node its real UDP mapping, the
    // most reliable hole punch. TCP is the fallback where UDP is blocked.
    "/dns4/honemesh.net/udp/6942/quic-v1/p2p/12D3KooWQnJPfAYbktWXv1zFoGCQAz4v7qWthLTdrD7ey5JPDCUt",
    "/dns4/honemesh.net/tcp/6942/p2p/12D3KooWQnJPfAYbktWXv1zFoGCQAz4v7qWthLTdrD7ey5JPDCUt",
];

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Config {
    pub node_id: String,
    pub account: AccountId,
    pub data_dir: PathBuf,
    pub api_port: u16,
    pub p2p_port: u16,
    pub bootstrap_peers: Vec<String>,
    pub is_miner: bool,
    pub is_clock: bool,
    pub genesis_file: Option<PathBuf>,
    pub log_level: String,
    pub chain_id: String,
    /// Unix millisecond timestamp for the genesis block.
    /// MUST be identical on every node — all nodes derive the same genesis hash from it.
    /// Set via HONE_GENESIS_TIMESTAMP env var.
    /// Default: 1783191600000 = 2026-07-04 19:00:00 UTC = 12:00 noon PDT (Los Angeles).
    pub genesis_timestamp: Option<u64>,
    /// Wallet secret: a 12-word BIP-39 mnemonic or 64-char hex (treated as 32-byte
    /// BIP-39 entropy, NOT a raw ed25519 seed). The wallet derives the posting role
    /// key at SLIP-10 m/44'/6942'/2'/0', which both identifies the account on-chain
    /// and signs clock seals — so the seal signer matches the registered identity.
    /// Set via HONE_POSTING_KEY env var.
    pub posting_key: Option<String>,
    /// Isolated mode (HONE_ISOLATED=true): the node makes NO outbound calls to
    /// public discovery — no honemesh.net registry fetch/announce, no Hive/TON
    /// registry, and no fallback to the Cloudflare DNS seed peers. It only dials
    /// the peers explicitly listed in HONE_BOOTSTRAP_PEERS (plus its cached peer
    /// store). Used to run a self-contained N-clock consensus test that cannot be
    /// contaminated by live-network peers, so state_root convergence between the
    /// test nodes can be asserted. NOT for production nodes.
    pub isolated: bool,
}

impl Config {
    pub fn from_env() -> Self {
        let data_dir = std::env::var("HONE_DATA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                dirs_next::home_dir()
                    .unwrap_or_else(|| PathBuf::from("/tmp"))
                    .join(".hone")
            });

        let isolated = std::env::var("HONE_ISOLATED")
            .map(|v| v == "true" || v == "1").unwrap_or(false);

        Self {
            node_id: std::env::var("HONE_NODE_ID")
                .unwrap_or_else(|_| std::env::var("HONE_ACCOUNT").unwrap_or_else(|_| "node-0".to_string())),
            account: std::env::var("HONE_ACCOUNT")
                .unwrap_or_else(|_| "genesis".to_string()),
            data_dir,
            api_port: std::env::var("HONE_API_PORT")
                .ok().and_then(|s| s.parse().ok()).unwrap_or(4242),
            p2p_port: std::env::var("HONE_P2P_PORT")
                .ok().and_then(|s| s.parse().ok()).unwrap_or(6942),
            bootstrap_peers: {
                let raw = std::env::var("HONE_BOOTSTRAP_PEERS").unwrap_or_default();
                let mut peers: Vec<String> = if raw.trim().is_empty() {
                    Vec::new()
                } else {
                    raw.split(',')
                        .filter(|s| !s.is_empty())
                        .map(|s| s.trim().to_string())
                        .inspect(|addr| {
                            if !addr.starts_with('/') {
                                eprintln!(
                                    "[config] HONE_BOOTSTRAP_PEERS: '{}' is not a valid multiaddr \
                                     (must start with '/'; e.g. /dns4/host/tcp/6942)",
                                    addr
                                );
                            }
                        })
                        .collect()
                };
                // Isolated mode dials only the listed peers. Every other node always
                // appends the public seeds, so it finds the mesh from anywhere.
                let no_default = matches!(std::env::var("HONE_NO_DEFAULT_BOOTSTRAP").as_deref(), Ok("1") | Ok("true"));
                if !isolated && !no_default {
                    for d in DEFAULT_BOOTSTRAP_PEERS {
                        if !peers.iter().any(|p| p == d) { peers.push(d.to_string()); }
                    }
                }
                peers
            },
            is_miner: std::env::var("HONE_MINER")
                .map(|v| v == "true" || v == "1").unwrap_or(false),
            is_clock: std::env::var("HONE_CLOCK")
                .map(|v| v == "true" || v == "1").unwrap_or(false),
            genesis_file: std::env::var("HONE_GENESIS_FILE").ok().map(PathBuf::from),
            log_level: std::env::var("HONE_LOG_LEVEL")
                .unwrap_or_else(|_| "hone_node=info".to_string()),
            chain_id: std::env::var("HONE_CHAIN_ID")
                .unwrap_or_else(|_| MAINNET_CHAIN_ID.to_string()),
            genesis_timestamp: Some(
                std::env::var("HONE_GENESIS_TIMESTAMP")
                    .ok()
                    .and_then(|s| s.parse().ok())
                    .unwrap_or(1783191600000u64)
            ),
            posting_key: std::env::var("HONE_POSTING_KEY").ok(),
            isolated,
        }
    }
}
