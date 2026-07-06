use crate::api;
use crate::panels::{AppData, HoneBehavior, PaneKind};

// ── Key role ──────────────────────────────────────────────────────────────────

#[derive(serde::Serialize, serde::Deserialize, Clone, Debug, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum KeyRole {
    Active,
    Posting,
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

// ── Session ───────────────────────────────────────────────────────────────────

#[derive(serde::Serialize, serde::Deserialize, Clone)]
pub struct Session {
    pub account:  String,
    pub key_file: std::path::PathBuf,
    pub node_url: String,
    pub key_role: KeyRole,
}

impl Session {
    #[allow(dead_code)]
    pub fn can_spend_tokens(&self) -> bool {
        self.key_role == KeyRole::Active
    }
}

fn load_session() -> Option<Session> {
    let home = std::env::var("HOME").ok()?;
    let path = std::path::PathBuf::from(home).join(".hone").join("session.json");
    serde_json::from_slice(&std::fs::read(path).ok()?).ok()
}

/// Read chain addresses embedded in a `.wallet.key` JSON file.
/// Returns `(eth_address, btc_pubkey)`.
fn read_chain_addresses(key_file: &std::path::Path) -> (Option<String>, Option<String>) {
    let Ok(content) = std::fs::read_to_string(key_file) else { return (None, None) };
    let Ok(v) = serde_json::from_str::<serde_json::Value>(&content) else { return (None, None) };
    let eth = v.get("ethereum_address").and_then(|x| x.as_str()).map(|s| s.to_string());
    let btc = v.get("bitcoin_pubkey").and_then(|x| x.as_str()).map(|s| s.to_string());
    (eth, btc)
}

/// Query ERC-20 balanceOf via JSON-RPC eth_call.  Returns balance in token base units.
fn query_erc20_balance(rpc_url: &str, contract: &str, address: &str) -> Option<u64> {
    if rpc_url.is_empty() || contract.is_empty() || address.is_empty() { return None; }
    let padded = format!("{:0>64}", address.trim_start_matches("0x"));
    let data   = format!("0x70a08231{}", padded); // balanceOf(address)
    let body   = serde_json::json!({
        "jsonrpc": "2.0", "method": "eth_call",
        "params": [{"to": contract, "data": data}, "latest"],
        "id": 1
    });
    let resp: serde_json::Value = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        .build().ok()?
        .post(rpc_url).json(&body).send().ok()?
        .json().ok()?;
    let hex = resp.get("result").and_then(|r| r.as_str())?.trim_start_matches("0x");
    if hex.is_empty() || hex.chars().all(|c| c == '0') { return Some(0); }
    let s = if hex.len() > 16 { &hex[hex.len() - 16..] } else { hex };
    u64::from_str_radix(s, 16).ok()
}

fn save_session(s: &Session) -> anyhow::Result<()> {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    let dir  = std::path::PathBuf::from(home).join(".hone");
    std::fs::create_dir_all(&dir)?;
    std::fs::write(dir.join("session.json"), serde_json::to_string_pretty(s)?)?;
    Ok(())
}

// ── Login state ───────────────────────────────────────────────────────────────

#[derive(Default)]
pub struct LoginState {
    pub account:  String,
    pub key_file: String,
    pub node_url: String,
    pub error:    Option<String>,
}

fn discover_wallet(account: &str) -> Option<String> {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
    let data_dir = std::env::var("HONE_DATA_DIR").unwrap_or_default();
    let candidates = [
        format!("{}/.hone/{}.wallet.key", home, account),
        format!("{}/.hone/wallet.key", home),
        format!("{}/wallet.key", data_dir),
        format!("{}/.hone/key.json", home),
    ];
    candidates.iter()
        .find(|p| !p.starts_with("/wallet.key") && std::path::Path::new(p.as_str()).exists())
        .cloned()
}

fn try_login(login: &mut LoginState) -> Option<Session> {
    let account  = login.account.trim().to_owned();
    let node_url = if login.node_url.trim().is_empty() {
        "http://localhost:4242".to_owned()
    } else {
        login.node_url.trim().to_owned()
    };

    if account.is_empty() {
        login.error = Some("Account name is required".into());
        return None;
    }

    // Auto-discover key file if blank
    let key_file_str = if login.key_file.trim().is_empty() {
        match discover_wallet(&account) {
            Some(p) => { login.key_file = p.clone(); p }
            None => {
                login.error = Some(format!(
                    "No key file found — tried ~/.hone/{}.wallet.key, wallet.key, key.json",
                    account
                ));
                return None;
            }
        }
    } else {
        login.key_file.trim().to_owned()
    };

    let key_path = std::path::PathBuf::from(&key_file_str);

    // Fetch on-chain keys
    let account_data = match crate::api::get_json(&node_url, &format!("/api/account/{}", account)) {
        Ok(v) => v,
        Err(e) => {
            login.error = Some(format!("Cannot reach node: {}", e));
            return None;
        }
    };

    let on_chain_keys = account_data.get("keys");

    // If no keys registered yet, just verify the file is readable
    if on_chain_keys.is_none() {
        match crate::sign::load_keypair(&key_path, "active") {
            Ok(_) => {
                let session = Session {
                    account, key_file: key_path, node_url,
                    key_role: KeyRole::Active,
                };
                let _ = save_session(&session);
                return Some(session);
            }
            Err(e) => {
                login.error = Some(format!("Cannot read key file: {}", e));
                return None;
            }
        }
    }

    // Public key match: try each role
    let candidates: &[(&str, KeyRole)] = &[
        ("active",  KeyRole::Active),
        ("posting", KeyRole::Posting),
        ("owner",   KeyRole::Owner),
    ];

    for (role_name, role_variant) in candidates {
        let on_chain_pub = on_chain_keys
            .and_then(|k| k.get(*role_name))
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if on_chain_pub.is_empty() { continue; }

        let kp = match crate::sign::load_keypair(&key_path, role_name) {
            Ok(k) => k,
            Err(_) => continue,
        };

        if kp.public_key_hex() == on_chain_pub {
            let session = Session {
                account,
                key_file: key_path,
                node_url,
                key_role: role_variant.clone(),
            };
            let _ = save_session(&session);
            return Some(session);
        }
    }

    login.error = Some("Key file does not match any registered key (tried active, posting, owner)".into());
    None
}

// ── Layout helpers ────────────────────────────────────────────────────────────

fn layout_path() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    std::path::PathBuf::from(home).join(".hone").join("gui-layout.json")
}

fn default_tree() -> egui_tiles::Tree<PaneKind> {
    let mut tiles = egui_tiles::Tiles::default();
    let node      = tiles.insert_pane(PaneKind::NodeStatus);
    let wallet    = tiles.insert_pane(PaneKind::Wallet);
    let staking   = tiles.insert_pane(PaneKind::Staking);
    let explorer  = tiles.insert_pane(PaneKind::Explorer);
    let inference = tiles.insert_pane(PaneKind::Inference);
    let settings  = tiles.insert_pane(PaneKind::Settings);
    let main_tabs = tiles.insert_tab_tile(vec![wallet, staking, inference, explorer, settings]);
    let root      = tiles.insert_horizontal_tile(vec![node, main_tabs]);
    egui_tiles::Tree::new("hone_layout", root, tiles)
}

// ── Theme ─────────────────────────────────────────────────────────────────────

fn setup_theme(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();

    // Background layers
    let bg_deep   = egui::Color32::from_rgb(13, 13, 18);
    let bg_panel  = egui::Color32::from_rgb(20, 20, 28);
    let bg_widget = egui::Color32::from_rgb(30, 30, 42);
    let bg_hover  = egui::Color32::from_rgb(42, 42, 58);
    let bg_active = egui::Color32::from_rgb(55, 55, 75);

    visuals.extreme_bg_color   = bg_deep;
    visuals.faint_bg_color     = bg_panel;
    visuals.panel_fill         = bg_panel;
    visuals.window_fill        = egui::Color32::from_rgb(24, 24, 34);

    // Accent: bitcoin orange
    visuals.selection.bg_fill          = egui::Color32::from_rgb(247, 147, 26);
    visuals.selection.stroke.color     = egui::Color32::WHITE;
    visuals.hyperlink_color            = egui::Color32::from_rgb(247, 147, 26);

    // Widget states
    visuals.widgets.noninteractive.bg_fill  = bg_panel;
    visuals.widgets.inactive.bg_fill        = bg_widget;
    visuals.widgets.hovered.bg_fill         = bg_hover;
    visuals.widgets.active.bg_fill          = bg_active;
    visuals.widgets.open.bg_fill            = bg_active;

    // Text colors
    let text_primary  = egui::Color32::from_rgb(220, 220, 230);
    let text_dim      = egui::Color32::from_rgb(140, 140, 160);
    visuals.widgets.noninteractive.fg_stroke = egui::Stroke::new(1.0, text_primary);
    visuals.widgets.inactive.fg_stroke       = egui::Stroke::new(1.0, text_primary);
    visuals.widgets.hovered.fg_stroke        = egui::Stroke::new(1.0, egui::Color32::WHITE);
    visuals.widgets.active.fg_stroke         = egui::Stroke::new(1.0, egui::Color32::WHITE);
    visuals.override_text_color              = Some(text_primary);

    // Corner radius (egui 0.34 uses CornerRadius with u8)
    let r4 = egui::CornerRadius::same(4);
    let r6 = egui::CornerRadius::same(6);
    let r8 = egui::CornerRadius::same(8);
    visuals.window_corner_radius                      = r8;
    visuals.menu_corner_radius                        = r6;
    visuals.widgets.noninteractive.corner_radius      = r4;
    visuals.widgets.inactive.corner_radius            = r4;
    visuals.widgets.hovered.corner_radius             = r4;
    visuals.widgets.active.corner_radius              = r4;

    // Separators and strokes
    visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, egui::Color32::from_rgb(45, 45, 62));
    visuals.widgets.inactive.bg_stroke       = egui::Stroke::new(1.0, egui::Color32::from_rgb(55, 55, 75));

    ctx.set_visuals(visuals);

    // Font sizes and spacing
    let mut style = (*ctx.style()).clone();
    use egui::{FontId, FontFamily, TextStyle};
    style.text_styles = [
        (TextStyle::Heading,  FontId::new(18.0, FontFamily::Proportional)),
        (TextStyle::Body,     FontId::new(14.0, FontFamily::Proportional)),
        (TextStyle::Monospace,FontId::new(13.0, FontFamily::Monospace)),
        (TextStyle::Button,   FontId::new(14.0, FontFamily::Proportional)),
        (TextStyle::Small,    FontId::new(11.5, FontFamily::Proportional)),
    ].into();
    style.spacing.item_spacing    = egui::vec2(8.0, 5.0);
    style.spacing.button_padding  = egui::vec2(12.0, 5.0);
    style.spacing.window_margin   = egui::Margin::same(16_i8);
    style.spacing.indent          = 18.0;
    let _ = text_dim; // used implicitly via override_text_color fallback
    ctx.set_style(style);
}

// ── App ───────────────────────────────────────────────────────────────────────

pub struct HoneApp {
    pub tree:         egui_tiles::Tree<PaneKind>,
    pub data:         AppData,
    pub session:      Option<Session>,
    pub login:        LoginState,
    pub last_refresh: std::time::Instant,
    pub layout_path:  std::path::PathBuf,
    pending_add:      Option<PaneKind>,
}

impl HoneApp {
    pub fn new(cc: &eframe::CreationContext) -> Self {
        setup_theme(&cc.egui_ctx);
        let session = load_session();
        let lp = layout_path();

        let tree = std::fs::read_to_string(&lp)
            .ok()
            .and_then(|s| serde_json::from_str::<egui_tiles::Tree<PaneKind>>(&s).ok())
            .unwrap_or_else(default_tree);

        let node_url = std::env::var("HONE_API_URL")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| session.as_ref().map(|s| s.node_url.clone()))
            .unwrap_or_else(|| "http://localhost:4242".to_string());

        let login = LoginState {
            node_url: node_url.clone(),
            key_file: String::new(),
            ..Default::default()
        };

        let account  = session.as_ref().map(|s| s.account.clone());
        let key_file = session.as_ref().map(|s| s.key_file.clone());
        let key_role = session.as_ref().map(|s| s.key_role.clone());

        let (eth_address, btc_pubkey) = key_file.as_ref()
            .map(|p| read_chain_addresses(p))
            .unwrap_or((None, None));

        let mut app = HoneApp {
            tree,
            data: AppData { node_url, account, key_file, key_role, eth_address, btc_pubkey, ..Default::default() },
            session,
            login,
            last_refresh: std::time::Instant::now()
                .checked_sub(std::time::Duration::from_secs(20))
                .unwrap_or(std::time::Instant::now()),
            layout_path: lp,
            pending_add: None,
        };
        if app.session.is_some() {
            app.refresh();
        }
        app
    }

    pub fn save_layout(&self) {
        if let Ok(json) = serde_json::to_string_pretty(&self.tree) {
            if let Some(parent) = self.layout_path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let _ = std::fs::write(&self.layout_path, json);
        }
    }

    pub fn refresh(&mut self) {
        let base = self.data.node_url.clone();

        match api::get_json(&base, "/api/node/info") {
            Ok(v) => self.data.node_info = Some(v),
            Err(e) => self.data.status_msg = format!("node/info: {}", e),
        }
        match api::get_json(&base, "/api/explorer/status") {
            Ok(v) => self.data.explorer_status = Some(v),
            Err(e) => self.data.status_msg = format!("explorer/status: {}", e),
        }
        match api::get_json(&base, "/api/explorer/blocks?limit=20") {
            Ok(v) => {
                if let Some(arr) = v.get("blocks").and_then(|b| b.as_array()) {
                    self.data.blocks = arr.clone();
                }
            }
            Err(e) => self.data.status_msg = format!("blocks: {}", e),
        }
        match api::get_json(&base, "/api/task/jobs?status=posted") {
            Ok(v) => {
                if let Some(arr) = v.get("jobs").and_then(|b| b.as_array()) {
                    self.data.jobs = arr.clone();
                }
            }
            Err(e) => self.data.status_msg = format!("jobs: {}", e),
        }
        if let Some(ref account) = self.data.account.clone() {
            match api::get_json(&base, &format!("/api/balance/{}", account)) {
                Ok(v) => {
                    self.data.balance = v.get("balance").and_then(|b| b.as_u64())
                        .or_else(|| v.get("hunits").and_then(|b| b.as_u64()));
                }
                Err(e) => self.data.status_msg = format!("balance: {}", e),
            }
            match api::get_json(&base, &format!("/api/account/{}", account)) {
                Ok(v) => {
                    self.data.staked = v.get("staked").and_then(|s| s.as_u64());
                }
                Err(e) => self.data.status_msg = format!("account: {}", e),
            }
            if let Ok(v) = api::get_json(&base, &format!("/api/staking/account/{}/positions", account)) {
                if let Some(arr) = v.get("positions").and_then(|a| a.as_array()) {
                    self.data.role_positions = arr.clone();
                }
            }
        }
        if let Ok(v) = api::get_json(&base, "/api/staking/requirements") {
            self.data.role_requirements = Some(v);
        }
        // wHONE balance via Ethereum JSON-RPC
        if let Some(ref eth_addr) = self.data.eth_address.clone() {
            let rpc  = self.data.eth_rpc_url.clone();
            let cont = self.data.whone_contract.clone();
            if !rpc.is_empty() && !cont.is_empty() {
                self.data.whone_balance = query_erc20_balance(&rpc, &cont, eth_addr);
            }
        }
        self.last_refresh = std::time::Instant::now();
    }

    fn add_pane_to_tree(&mut self, kind: PaneKind) {
        let pane_id = self.tree.tiles.insert_pane(kind);
        let tab_id  = self.tree.tiles.insert_tab_tile(vec![pane_id]);
        if let Some(root) = self.tree.root {
            if let Some(egui_tiles::Tile::Container(container)) =
                self.tree.tiles.get_mut(root)
            {
                container.add_child(tab_id);
            } else {
                let new_root = self.tree.tiles.insert_horizontal_tile(vec![root, tab_id]);
                self.tree.root = Some(new_root);
            }
        } else {
            self.tree.root = Some(pane_id);
        }
    }

    fn show_login_modal(&mut self, ctx: &egui::Context) {
        let orange = egui::Color32::from_rgb(247, 147, 26);
        egui::Window::new("hone_login")
            .title_bar(false)
            .collapsible(false)
            .resizable(false)
            .anchor(egui::Align2::CENTER_CENTER, egui::Vec2::ZERO)
            .fixed_size([420.0, 300.0])
            .show(ctx, |ui| {
                ui.add_space(12.0);

                // Branding header
                ui.vertical_centered(|ui| {
                    ui.label(egui::RichText::new("₿ HONE")
                        .size(28.0)
                        .color(orange)
                        .strong());
                    ui.add_space(2.0);
                    ui.label(egui::RichText::new("Sign in to your node")
                        .size(13.0)
                        .color(egui::Color32::from_rgb(150, 150, 170)));
                });

                ui.add_space(18.0);
                ui.separator();
                ui.add_space(12.0);

                egui::Grid::new("login_grid")
                    .num_columns(2)
                    .spacing([12.0, 10.0])
                    .show(ui, |ui| {
                        ui.label("Account");
                        let resp = ui.add(egui::TextEdit::singleline(&mut self.login.account)
                            .hint_text("shindevlin")
                            .desired_width(280.0));
                        if resp.gained_focus() { self.login.error = None; }
                        ui.end_row();

                        ui.label("Key file");
                        ui.add(egui::TextEdit::singleline(&mut self.login.key_file)
                            .hint_text("blank = auto-discover ~/.hone/{account}.wallet.key")
                            .desired_width(280.0));
                        ui.end_row();

                        ui.label("Node URL");
                        ui.add(egui::TextEdit::singleline(&mut self.login.node_url)
                            .hint_text("http://localhost:4242")
                            .desired_width(280.0));
                        ui.end_row();
                    });

                ui.add_space(12.0);

                if let Some(ref err) = self.login.error.clone() {
                    ui.colored_label(egui::Color32::from_rgb(255, 90, 90), err);
                    ui.add_space(6.0);
                }

                let enter_pressed = ui.input(|i| i.key_pressed(egui::Key::Enter));
                let sign_in = ui.add_sized(
                    [ui.available_width(), 32.0],
                    egui::Button::new(
                        egui::RichText::new("Sign in").size(14.0).strong()
                    )
                );

                if sign_in.clicked() || enter_pressed {
                    self.login.error = None;
                    if let Some(session) = try_login(&mut self.login) {
                        let (eth_address, btc_pubkey) = read_chain_addresses(&session.key_file);
                        self.data.account   = Some(session.account.clone());
                        self.data.key_file  = Some(session.key_file.clone());
                        self.data.key_role  = Some(session.key_role.clone());
                        self.data.node_url  = session.node_url.clone();
                        self.data.eth_address = eth_address;
                        self.data.btc_pubkey  = btc_pubkey;
                        self.session = Some(session);
                        self.refresh();
                    }
                }

                ui.add_space(8.0);
                ui.separator();
                ui.add_space(4.0);
                ui.label(egui::RichText::new(
                    "Key file stays local. Only the signature is sent to the node."
                ).small().color(egui::Color32::from_rgb(110, 110, 130)));
            });
    }
}

impl eframe::App for HoneApp {
    fn logic(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if let Some(kind) = self.pending_add.take() {
            self.add_pane_to_tree(kind);
        }
        if self.last_refresh.elapsed() >= std::time::Duration::from_secs(10) && self.session.is_some() {
            self.data.status_msg.clear();
            self.refresh();
            ctx.request_repaint();
        }
    }

    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        let ctx = ui.ctx().clone();

        egui::TopBottomPanel::top("menu")
            .frame(egui::Frame::none()
                .fill(egui::Color32::from_rgb(13, 13, 18))
                .inner_margin(egui::Margin::symmetric(8_i8, 4_i8)))
            .show(&ctx, |ui| {
                ui.horizontal(|ui| {
                    let orange = egui::Color32::from_rgb(247, 147, 26);
                    ui.label(egui::RichText::new("₿ HONE").color(orange).strong());
                    ui.separator();

                    ui.menu_button("File", |ui| {
                        if ui.button("Sign out").clicked() {
                            self.session = None;
                            self.data.account  = None;
                            self.data.key_file = None;
                            self.data.key_role = None;
                            self.login = LoginState {
                                node_url: self.data.node_url.clone(),
                                ..Default::default()
                            };
                            let _ = std::fs::remove_file(
                                std::path::PathBuf::from(
                                    std::env::var("HOME").unwrap_or_else(|_| ".".into())
                                ).join(".hone").join("session.json")
                            );
                            ui.close_menu();
                        }
                        ui.separator();
                        if ui.button("Quit").clicked() {
                            ctx.send_viewport_cmd(egui::ViewportCommand::Close);
                        }
                    });

                    ui.menu_button("Panels", |ui| {
                        for (label, kind) in [
                            ("Node Status", PaneKind::NodeStatus),
                            ("Wallet",      PaneKind::Wallet),
                            ("Explorer",    PaneKind::Explorer),
                            ("Inference",   PaneKind::Inference),
                            ("Staking",     PaneKind::Staking),
                            ("Settings",    PaneKind::Settings),
                        ] {
                            if ui.button(label).clicked() {
                                self.pending_add = Some(kind);
                                ui.close_menu();
                            }
                        }
                    });

                    if ui.small_button("⟳ Refresh").clicked() {
                        self.data.status_msg.clear();
                        self.refresh();
                    }

                    // Push session badge to right
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        match &self.session {
                            Some(s) => {
                                let (role_color, role_icon) = match s.key_role {
                                    KeyRole::Active  => (egui::Color32::from_rgb(74, 222, 128),  "🟢"),
                                    KeyRole::Posting => (egui::Color32::from_rgb(96, 165, 250),  "🔵"),
                                    KeyRole::Owner   => (egui::Color32::from_rgb(251, 191, 36),  "🟡"),
                                };
                                ui.colored_label(role_color,
                                    format!("{} {} · {}", role_icon, s.account, s.key_role));
                            }
                            None => {
                                ui.colored_label(egui::Color32::from_rgb(100, 100, 120), "Not signed in");
                            }
                        }
                        if !self.data.status_msg.is_empty() {
                            ui.separator();
                            ui.colored_label(egui::Color32::from_rgb(255, 200, 60), &self.data.status_msg);
                        }
                    });
                });
            });

        egui::CentralPanel::default()
            .frame(egui::Frame::none()
                .fill(egui::Color32::from_rgb(13, 13, 18))
                .inner_margin(egui::Margin::same(0_i8)))
            .show(&ctx, |ui| {
                let mut behavior = HoneBehavior { data: &mut self.data };
                self.tree.ui(&mut behavior, ui);
            });

        if self.session.is_none() {
            self.show_login_modal(&ctx);
        }
    }

    fn on_exit(&mut self, _gl: Option<&glow::Context>) {
        self.save_layout();
    }
}
