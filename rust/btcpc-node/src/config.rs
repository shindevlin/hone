use std::path::PathBuf;
use serde::{Deserialize, Serialize};
use btcpc_types::AccountId;

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
}

impl Config {
    pub fn from_env() -> Self {
        let data_dir = std::env::var("BTCPC_DATA_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| {
                dirs_next::home_dir()
                    .unwrap_or_else(|| PathBuf::from("/tmp"))
                    .join(".btcpc")
            });

        Self {
            node_id: std::env::var("BTCPC_NODE_ID")
                .unwrap_or_else(|_| "node-0".to_string()),
            account: std::env::var("BTCPC_ACCOUNT")
                .unwrap_or_else(|_| "genesis".to_string()),
            data_dir,
            api_port: std::env::var("BTCPC_API_PORT")
                .ok().and_then(|s| s.parse().ok()).unwrap_or(4242),
            p2p_port: std::env::var("BTCPC_P2P_PORT")
                .ok().and_then(|s| s.parse().ok()).unwrap_or(6942),
            bootstrap_peers: std::env::var("BTCPC_BOOTSTRAP_PEERS")
                .unwrap_or_default()
                .split(',')
                .filter(|s| !s.is_empty())
                .map(|s| s.trim().to_string())
                .collect(),
            is_miner: std::env::var("BTCPC_MINER")
                .map(|v| v == "true" || v == "1").unwrap_or(false),
            is_clock: std::env::var("BTCPC_CLOCK")
                .map(|v| v == "true" || v == "1").unwrap_or(false),
            genesis_file: std::env::var("BTCPC_GENESIS_FILE").ok().map(PathBuf::from),
            log_level: std::env::var("BTCPC_LOG_LEVEL")
                .unwrap_or_else(|_| "btcpc_node=info".to_string()),
        }
    }
}
