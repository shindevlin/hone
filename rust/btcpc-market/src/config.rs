use std::env;

pub struct Config {
    pub port: u16,
    pub data_dir: String,
    pub jwt_secret: String,
    pub telegram_bot_token: Option<String>,
    pub telegram_chat_id: Option<String>,
}

impl Config {
    pub fn from_env() -> Self {
        let port = env::var("BTCPC_MARKET_PORT")
            .or_else(|_| env::var("PORT"))
            .unwrap_or_else(|_| "7042".to_string())
            .parse()
            .unwrap_or(7042);

        let data_dir = env::var("BTCPC_DATA_DIR").unwrap_or_else(|_| {
            dirs_next::home_dir()
                .map(|h| h.join(".btcpc").to_string_lossy().to_string())
                .unwrap_or_else(|| "/var/lib/btcpc".to_string())
        });

        let jwt_secret = env::var("BTCPC_JWT_SECRET")
            .or_else(|_| env::var("JWT_SECRET"))
            .unwrap_or_else(|_| "btcpc-dev-secret".to_string());

        let telegram_bot_token = env::var("BTCPC_TELEGRAM_BOT_TOKEN").ok();
        let telegram_chat_id = env::var("BTCPC_TELEGRAM_CHAT_ID").ok();

        Self { port, data_dir, jwt_secret, telegram_bot_token, telegram_chat_id }
    }

    pub fn pending_entries_path(&self) -> std::path::PathBuf {
        std::path::Path::new(&self.data_dir).join("pending-entries.jsonl")
    }
}
