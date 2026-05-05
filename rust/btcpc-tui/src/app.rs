use crate::api;

#[derive(serde::Deserialize, Clone)]
pub struct Session {
    pub account: String,
    #[allow(dead_code)]
    pub key_file: std::path::PathBuf,
    pub node_url: String,
}

pub fn load_session() -> Option<Session> {
    let home = std::env::var("HOME").ok()?;
    let path = std::path::PathBuf::from(home).join(".btcpc").join("session.json");
    let bytes = std::fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

pub struct App {
    pub tab: usize,
    pub session: Option<Session>,
    pub node_info: Option<serde_json::Value>,
    pub explorer_status: Option<serde_json::Value>,
    pub blocks: Vec<serde_json::Value>,
    pub wallet_balance: Option<u64>,
    pub wallet_staked: Option<u64>,
    pub jobs: Vec<serde_json::Value>,
    pub last_refresh: std::time::Instant,
    pub refresh_interval: std::time::Duration,
    pub status_msg: String,
}

impl App {
    pub fn new() -> Self {
        let session = load_session();
        let mut app = App {
            tab: 0,
            session,
            node_info: None,
            explorer_status: None,
            blocks: Vec::new(),
            wallet_balance: None,
            wallet_staked: None,
            jobs: Vec::new(),
            last_refresh: std::time::Instant::now()
                .checked_sub(std::time::Duration::from_secs(10))
                .unwrap_or(std::time::Instant::now()),
            refresh_interval: std::time::Duration::from_secs(5),
            status_msg: String::new(),
        };
        app.refresh();
        app
    }

    pub fn node_url(&self) -> String {
        std::env::var("BTCPC_API_URL")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| self.session.as_ref().map(|s| s.node_url.clone()))
            .unwrap_or_else(|| "http://localhost:4242".to_string())
    }

    pub fn refresh(&mut self) {
        let base = self.node_url();

        match api::get_json(&base, "/api/node/info") {
            Ok(v) => self.node_info = Some(v),
            Err(e) => self.status_msg = format!("node/info: {}", e),
        }

        match api::get_json(&base, "/api/explorer/status") {
            Ok(v) => self.explorer_status = Some(v),
            Err(e) => self.status_msg = format!("explorer/status: {}", e),
        }

        match api::get_json(&base, "/api/explorer/blocks?limit=20") {
            Ok(v) => {
                if let Some(arr) = v.get("blocks").and_then(|b| b.as_array()) {
                    self.blocks = arr.clone();
                }
            }
            Err(e) => self.status_msg = format!("blocks: {}", e),
        }

        match api::get_json(&base, "/api/task/jobs?status=posted") {
            Ok(v) => {
                if let Some(arr) = v.get("jobs").and_then(|b| b.as_array()) {
                    self.jobs = arr.clone();
                }
            }
            Err(e) => self.status_msg = format!("jobs: {}", e),
        }

        if let Some(ref session) = self.session.clone() {
            let account = session.account.clone();
            match api::get_json(&base, &format!("/api/balance/{}", account)) {
                Ok(v) => {
                    if let Some(b) = v.get("balance").and_then(|b| b.as_u64()) {
                        self.wallet_balance = Some(b);
                    }
                }
                Err(e) => self.status_msg = format!("balance: {}", e),
            }
            match api::get_json(&base, &format!("/api/account/{}", account)) {
                Ok(v) => {
                    if let Some(s) = v.get("staked").and_then(|s| s.as_u64()) {
                        self.wallet_staked = Some(s);
                    }
                }
                Err(e) => self.status_msg = format!("account: {}", e),
            }
        }

        self.last_refresh = std::time::Instant::now();
    }
}
