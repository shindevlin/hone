use clap::Parser;

#[derive(Parser, Debug, Clone)]
#[command(about = "BTCPC BLE tracker scanner daemon")]
pub struct Config {
    /// URL of the local btcpc-node API.
    #[arg(long, env = "BTCPC_NODE_URL", default_value = "http://127.0.0.1:3001")]
    pub node_url: String,

    /// BTCPC account ID that will sign sighting commits and claims.
    #[arg(long, env = "BTCPC_ACCOUNT")]
    pub account: String,

    /// ed25519 private key hex (posting key).
    #[arg(long, env = "BTCPC_POSTING_KEY")]
    pub posting_key: String,

    /// Human-readable observer identifier, e.g. "pi-nebra/ble".
    /// Embedded in sighting commits so the network can credit this node.
    #[arg(long, env = "BTCPC_OBSERVER_ID")]
    pub observer_id: String,

    /// Epoch duration in seconds (must match the chain).
    #[arg(long, env = "BTCPC_EPOCH_SECS", default_value_t = 30)]
    pub epoch_secs: u64,

    /// Port for the local challenge HTTP API (acoustic challenge requests arrive here).
    #[arg(long, env = "BTCPC_TRACKER_CHALLENGE_PORT", default_value_t = 3210)]
    pub challenge_port: u16,

    /// Minimum RSSI (dBm) to record a sighting. Filters out ghost signals.
    #[arg(long, env = "BTCPC_TRACKER_MIN_RSSI", default_value_t = -90)]
    pub min_rssi: i16,

    /// Path for the local sled DB (co-location log).
    #[arg(long, env = "BTCPC_TRACKER_DB", default_value = "/var/lib/btcpc/tracker.db")]
    pub db_path: String,
}
