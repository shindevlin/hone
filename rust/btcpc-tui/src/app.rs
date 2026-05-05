use crate::api;

// ── Session ───────────────────────────────────────────────────────────────────

/// Which named key role was verified at login.
/// Determines which operations are permitted in this session.
#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum KeyRole {
    /// Full token authority: transfer, stake, unstake, delegate
    Active,
    /// Inference and node operations only; no token spend
    Posting,
    /// Key rotation only
    Owner,
}

impl std::fmt::Display for KeyRole {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            KeyRole::Active  => write!(f, "active"),
            KeyRole::Posting => write!(f, "posting"),
            KeyRole::Owner   => write!(f, "owner"),
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct Session {
    pub account: String,
    pub key_file: std::path::PathBuf,
    pub node_url: String,
    /// The key role that was challenge-verified at login.
    pub key_role: KeyRole,
}

impl Session {
    pub fn can_spend_tokens(&self) -> bool {
        self.key_role == KeyRole::Active
    }
}

pub fn load_session() -> Option<Session> {
    let home = std::env::var("HOME").ok()?;
    let path = std::path::PathBuf::from(home).join(".btcpc").join("session.json");
    let bytes = std::fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

pub fn save_session(s: &Session) -> anyhow::Result<()> {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    let dir = std::path::PathBuf::from(home).join(".btcpc");
    std::fs::create_dir_all(&dir)?;
    let path = dir.join("session.json");
    let json = serde_json::to_string_pretty(s)?;
    std::fs::write(&path, json)?;
    Ok(())
}

// ── Mode / form state ─────────────────────────────────────────────────────────

#[derive(Clone)]
pub struct TransferState {
    pub field: usize, // 0=to, 1=amount, 2=memo
    pub to: String,
    pub amount: String,
    pub memo: String,
}

impl TransferState {
    pub fn new() -> Self {
        Self { field: 0, to: String::new(), amount: String::new(), memo: String::new() }
    }
    pub fn field_count() -> usize { 3 }
    pub fn current_field_mut(&mut self) -> &mut String {
        match self.field {
            0 => &mut self.to,
            1 => &mut self.amount,
            _ => &mut self.memo,
        }
    }
    pub fn field_values(&self) -> Vec<(&'static str, &str)> {
        vec![
            ("To", self.to.as_str()),
            ("Amount (BTCPC)", self.amount.as_str()),
            ("Memo (optional)", self.memo.as_str()),
        ]
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum StakeAction { Add, Remove }

#[derive(Clone)]
pub struct StakeState {
    pub field: usize,
    pub amount: String,
    pub action: StakeAction,
}

impl StakeState {
    pub fn new(action: StakeAction) -> Self {
        Self { field: 0, amount: String::new(), action }
    }
    pub fn field_count() -> usize { 1 }
    pub fn current_field_mut(&mut self) -> &mut String {
        &mut self.amount
    }
    pub fn field_values(&self) -> Vec<(&'static str, &str)> {
        vec![("Amount (BTCPC)", self.amount.as_str())]
    }
}

#[derive(Clone)]
pub struct PostJobState {
    pub field: usize, // 0=model, 1=input, 2=max_fee, 3=deadline
    pub model: String,
    pub input: String,
    pub max_fee: String,
    pub deadline: String,
}

impl PostJobState {
    pub fn new() -> Self {
        Self {
            field: 0,
            model: String::new(),
            input: String::new(),
            max_fee: String::new(),
            deadline: String::new(),
        }
    }
    pub fn field_count() -> usize { 4 }
    pub fn current_field_mut(&mut self) -> &mut String {
        match self.field {
            0 => &mut self.model,
            1 => &mut self.input,
            2 => &mut self.max_fee,
            _ => &mut self.deadline,
        }
    }
    pub fn field_values(&self) -> Vec<(&'static str, &str)> {
        vec![
            ("Model", self.model.as_str()),
            ("Input", self.input.as_str()),
            ("Max Fee (BTCPC)", self.max_fee.as_str()),
            ("Deadline Epoch", self.deadline.as_str()),
        ]
    }
}

#[derive(Clone)]
pub struct LoginState {
    pub field: usize,      // 0=account, 1=key_file, 2=node_url
    pub account: String,
    pub key_file: String,
    pub node_url: String,
    pub error: Option<String>,
}

impl LoginState {
    pub fn new() -> Self {
        let home = std::env::var("HOME").unwrap_or_default();
        Self {
            field: 0,
            account: String::new(),
            key_file: format!("{}/.btcpc/key.json", home),
            node_url: "http://localhost:4242".into(),
            error: None,
        }
    }
    pub fn field_count() -> usize { 3 }
    pub fn current_field_mut(&mut self) -> &mut String {
        match self.field {
            0 => &mut self.account,
            1 => &mut self.key_file,
            _ => &mut self.node_url,
        }
    }
    pub fn fields(&self) -> Vec<(&'static str, &str)> {
        vec![
            ("Account name", self.account.as_str()),
            ("Key file path", self.key_file.as_str()),
            ("Node URL",      self.node_url.as_str()),
        ]
    }
}

#[derive(Clone)]
pub enum Mode {
    Login(LoginState),
    Normal,
    TransferForm(TransferState),
    StakeForm(StakeState),
    PostJobForm(PostJobState),
    Result { msg: String, success: bool },
}

// ── App ───────────────────────────────────────────────────────────────────────

pub struct App {
    pub tab: usize,
    pub session: Option<Session>,
    pub mode: Mode,
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
        let initial_mode = if session.is_none() {
            Mode::Login(LoginState::new())
        } else {
            Mode::Normal
        };
        let mut app = App {
            tab: 0,
            session,
            mode: initial_mode,
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
                    } else if let Some(b) = v.get("dreams").and_then(|b| b.as_u64()) {
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
