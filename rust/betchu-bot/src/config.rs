use anyhow::{anyhow, Result};

#[derive(Debug, Clone)]
pub struct Config {
    // Telegram
    pub telegram_bot_token: String,
    pub miniapp_url: String,

    // EVM / Base Sepolia
    pub rpc_url: String,
    pub private_key: String,
    pub contract_owner: String,
    pub bet_contract_address: String,
    pub player_registry_address: String,
    pub loyalty_token_address: String,

    // BTCPC inference
    pub btcpc_api_url: String,
    pub btcpc_project_key: String,

    // MongoDB
    pub mongodb_uri: String,

    // BTCPC service identity
    pub btcpc_account: String,
    pub service_port: u16,

    // Admin
    pub admin_chat_id: Option<i64>,
    pub referral_internal_key: String,
}

impl Config {
    pub fn from_env() -> Result<Self> {
        // Load .envbtcpc first so BTCPC credentials override any parent-process env.
        // Then load .env for betchu-specific vars.
        let _ = dotenvy::from_filename(".envbtcpc");
        let _ = dotenvy::dotenv();

        Ok(Config {
            telegram_bot_token: require("TELEGRAM_BOT_TOKEN")?,
            miniapp_url: optional("MINIAPP_URL", "https://betchu-miniapp.pages.dev"),

            rpc_url: optional("RPC_URL", "https://sepolia.base.org"),
            private_key: optional("PRIVATE_KEY", ""),
            contract_owner: optional("CONTRACT_OWNER", ""),
            bet_contract_address: require("BET_CONTRACT_ADDRESS")?,
            player_registry_address: optional("PLAYER_REGISTRY_ADDRESS", ""),
            loyalty_token_address: optional("LOYALTY_TOKEN_ADDRESS", ""),

            btcpc_api_url: optional("BTCPC_API_URL", "https://btcpc.net"),
            btcpc_project_key: optional("BTCPC_PROJECT_KEY", ""),

            mongodb_uri: optional("MONGODB_URI", "mongodb://localhost:27018/betchu"),

            btcpc_account: optional("BTCPC_HIVE_ACCOUNT", "betchu-bot"),
            service_port: std::env::var("SERVICE_PORT")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(8765),

            admin_chat_id: std::env::var("ADMIN_CHAT_ID")
                .ok()
                .and_then(|s| s.parse().ok()),

            referral_internal_key: optional("REFERRAL_INTERNAL_KEY", ""),
        })
    }
}

fn require(key: &str) -> Result<String> {
    std::env::var(key).map_err(|_| anyhow!("Missing required env var: {}", key))
}

fn optional(key: &str, default: &str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}
