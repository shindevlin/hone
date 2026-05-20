use std::env;

#[allow(dead_code)]
pub struct Config {
    pub node_id: String,
    pub p2p_port: u16,
    pub ipc_socket: String,
    pub relay_urls: Vec<String>,
    pub seed_peers: Vec<String>,
    pub max_peers: usize,
    pub data_dir: String,
}

impl Config {
    pub fn from_env() -> Self {
        let node_id = env::var("BTCPC_NODE_ID")
            .unwrap_or_else(|_| hex::encode(rand_bytes(16)));

        let p2p_port = env::var("BTCPC_P2P_SIDECAR_PORT")
            .or_else(|_| env::var("P2P_PORT"))
            .or_else(|_| env::var("BTCPC_API_P2P_PORT"))
            .unwrap_or_else(|_| "6942".to_string())
            .parse()
            .unwrap_or(6942);

        let ipc_socket = env::var("BTCPC_P2P_IPC_SOCKET")
            .unwrap_or_else(|_| "/tmp/btcpc-p2p.sock".to_string());

        let relay_urls = env::var("BTCPC_RELAY_URLS")
            .map(|s| s.split(',').map(|u| u.trim().to_string()).filter(|u| !u.is_empty()).collect())
            .unwrap_or_else(|_| {
                env::var("BTCPC_RELAY_URL")
                    .map(|u| vec![u])
                    .unwrap_or_else(|_| vec![
                        "wss://btcpc-relay.shindevlin.workers.dev/ws".to_string()
                    ])
            });

        let seed_peers = env::var("BTCPC_SEED_PEERS")
            .map(|s| s.split(',').map(|p| p.trim().to_string()).filter(|p| !p.is_empty()).collect())
            .unwrap_or_default();

        let max_peers = env::var("BTCPC_MAX_PEERS")
            .ok()
            .and_then(|s| s.parse().ok())
            .unwrap_or(50);

        let data_dir = env::var("BTCPC_DATA_DIR")
            .unwrap_or_else(|_| {
                dirs_next::home_dir()
                    .map(|h: std::path::PathBuf| h.join(".btcpc").to_string_lossy().to_string())
                    .unwrap_or_else(|| "/var/lib/btcpc".to_string())
            });

        Self { node_id, p2p_port, ipc_socket, relay_urls, seed_peers, max_peers, data_dir }
    }

    #[allow(dead_code)]
    pub fn bootstrap_peers() -> Vec<&'static str> {
        vec![
            "/dns4/node1.btcpc.network/tcp/6942/wss",
            "/dns4/node2.btcpc.network/tcp/6942/wss",
        ]
    }
}

fn rand_bytes(n: usize) -> Vec<u8> {
    use rand::RngCore;
    let mut bytes = vec![0u8; n];
    rand::thread_rng().fill_bytes(&mut bytes);
    bytes
}
