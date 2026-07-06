use std::path::PathBuf;
use serde::{Deserialize, Serialize};
use hone_types::{AccountId, MAINNET_CHAIN_ID};

/// Cloudflare DNS seeds — update the A record when a machine changes, no code change needed.
/// Add more entries here as the network grows.
pub const DEFAULT_BOOTSTRAP_PEERS: &[&str] = &[
    "/dns4/bootstrap1.honemesh.net/tcp/6942",
    "/dns4/bootstrap2.honemesh.net/tcp/6942",
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
    /// Hex-encoded 32-byte ed25519 seed for signing clock seals.
    /// If set, node_id is derived from the public key so it is self-authenticating.
    /// Set via HONE_POSTING_KEY env var.
    pub posting_key: Option<String>,
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
                let peers: Vec<String> = if raw.trim().is_empty() {
                    // Default Cloudflare DNS seeds — updated via DNS A record, no code change needed.
                    DEFAULT_BOOTSTRAP_PEERS.iter().map(|s| s.to_string()).collect()
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
        }
    }
}
