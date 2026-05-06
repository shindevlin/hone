pub mod explorer;
pub mod inference;
pub mod node;
pub mod settings;
pub mod staking;
pub mod wallet;

#[derive(Clone, serde::Serialize, serde::Deserialize, PartialEq, Debug)]
pub enum PaneKind {
    NodeStatus,
    Wallet,
    Explorer,
    Inference,
    Staking,
    Settings,
}

#[derive(Default)]
pub struct FormState {
    // Transfer
    pub transfer_to: String,
    pub transfer_amount: String,
    pub transfer_result: Option<(bool, String)>,

    // Generic stake
    pub stake_amount: String,
    pub stake_result: Option<(bool, String)>,

    // Role stake — backing another node's role
    pub role_stake_node: String,
    pub role_stake_role: String,
    pub role_stake_amount: String,
    pub role_stake_result: Option<(bool, String)>,
    pub role_stake_lookup: Option<serde_json::Value>,  // fetched node+role info

    // Role unstake — picked from existing positions
    pub role_unstake_idx: Option<usize>,
    pub role_unstake_amount: String,
    pub role_unstake_result: Option<(bool, String)>,

    // Operator opt-in
    pub role_opt_role: String,
    pub role_opt_share_bps: String,
    pub role_opt_max_backers: String,
    pub role_opt_result: Option<(bool, String)>,

    // Inference post
    pub job_model: String,
    pub job_input: String,
    pub job_max_fee: String,
    pub job_deadline: String,
    pub job_result: Option<(bool, String)>,
}

pub struct AppData {
    pub node_info: Option<serde_json::Value>,
    pub explorer_status: Option<serde_json::Value>,
    pub blocks: Vec<serde_json::Value>,
    pub balance: Option<u64>,
    pub staked: Option<u64>,
    pub jobs: Vec<serde_json::Value>,
    pub status_msg: String,
    pub node_url: String,
    pub account: Option<String>,
    pub key_file: Option<std::path::PathBuf>,
    pub key_role: Option<crate::app::KeyRole>,
    pub forms: FormState,
    // Role staking data
    pub role_positions: Vec<serde_json::Value>,
    pub role_requirements: Option<serde_json::Value>,
    // Cross-chain / bridge
    pub eth_address: Option<String>,
    pub btc_pubkey: Option<String>,
    pub wbtcpc_balance: Option<u64>,
    pub eth_rpc_url: String,
    pub wbtcpc_contract: String,
}

impl Default for AppData {
    fn default() -> Self {
        Self {
            node_info: None,
            explorer_status: None,
            blocks: Vec::new(),
            balance: None,
            staked: None,
            jobs: Vec::new(),
            status_msg: String::new(),
            node_url: "http://localhost:4242".to_string(),
            account: None,
            key_file: None,
            key_role: None,
            forms: FormState::default(),
            role_positions: Vec::new(),
            role_requirements: None,
            eth_address: None,
            btc_pubkey: None,
            wbtcpc_balance: None,
            eth_rpc_url: String::new(),
            wbtcpc_contract: String::new(),
        }
    }
}

// ── Palette ───────────────────────────────────────────────────────────────────

pub const ORANGE:   egui::Color32 = egui::Color32::from_rgb(247, 147, 26);
pub const GREEN:    egui::Color32 = egui::Color32::from_rgb(74,  222, 128);
pub const BLUE:     egui::Color32 = egui::Color32::from_rgb(96,  165, 250);
pub const YELLOW:   egui::Color32 = egui::Color32::from_rgb(251, 191, 36);
pub const RED:      egui::Color32 = egui::Color32::from_rgb(248, 113, 113);
pub const DIM_TEXT: egui::Color32 = egui::Color32::from_rgb(120, 120, 145);
pub const CARD_BG:  egui::Color32 = egui::Color32::from_rgb(26,  26,  36);
pub const CARD_BORDER: egui::Color32 = egui::Color32::from_rgb(42, 42, 60);

// ── Shared helpers ────────────────────────────────────────────────────────────

/// 1 BTCPC = 10_000_000_000 dreams (10^10)
pub const DREAMS_PER_BTCPC: u64 = 10_000_000_000;

pub fn dreams_to_btcpc(dreams: u64) -> String {
    let whole = dreams / DREAMS_PER_BTCPC;
    let frac  = dreams % DREAMS_PER_BTCPC;
    if frac == 0 {
        format!("{}", whole)
    } else {
        let s = format!("{:010}", frac);
        format!("{}.{}", whole, s.trim_end_matches('0'))
    }
}

pub fn parse_btcpc_input(s: &str) -> Option<u64> {
    s.trim().parse::<f64>().ok()
        .filter(|a| *a >= 0.0)
        .map(|a| (a * DREAMS_PER_BTCPC as f64).round() as u64)
}

pub fn format_ts_ms(ts_ms: u64) -> String {
    let secs = ts_ms / 1000;
    let dt = chrono::DateTime::from_timestamp(secs as i64, 0);
    dt.map(|d| d.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_else(|| "—".to_string())
}

/// Section heading with orange left accent bar.
pub fn section_heading(ui: &mut egui::Ui, text: &str) {
    ui.add_space(4.0);
    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(3.0, 18.0), egui::Sense::hover());
        ui.painter().rect_filled(rect, egui::CornerRadius::same(2), ORANGE);
        ui.add_space(6.0);
        ui.label(egui::RichText::new(text).size(13.0).strong()
            .color(egui::Color32::from_rgb(210, 210, 225)));
    });
    ui.add_space(4.0);
}

/// Framed card container.
pub fn card<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::none()
        .fill(CARD_BG)
        .stroke(egui::Stroke::new(1.0, CARD_BORDER))
        .corner_radius(egui::CornerRadius::same(6))
        .inner_margin(egui::Margin::same(12_i8))
        .show(ui, add)
        .inner
}

pub fn not_signed_in(ui: &mut egui::Ui) {
    ui.add_space(24.0);
    ui.vertical_centered(|ui| {
        ui.label(egui::RichText::new("Not signed in").size(15.0).color(DIM_TEXT));
        ui.add_space(6.0);
        ui.label(egui::RichText::new("Use  File → Sign in  to authenticate.")
            .small().color(DIM_TEXT));
    });
}

pub fn active_key_required(ui: &mut egui::Ui) {
    ui.add_space(8.0);
    card(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("⚠").color(YELLOW).size(15.0));
            ui.add_space(4.0);
            ui.vertical(|ui| {
                ui.label(egui::RichText::new("Active key required").strong().color(YELLOW));
                ui.label(egui::RichText::new("Sign out and sign back in with your active key to use this feature.")
                    .small().color(DIM_TEXT));
            });
        });
    });
}

// ─────────────────────────────────────────────────────────────────────────────

pub struct BtcpcBehavior<'a> {
    pub data: &'a mut AppData,
}

impl<'a> egui_tiles::Behavior<PaneKind> for BtcpcBehavior<'a> {
    fn tab_title_for_pane(&mut self, pane: &PaneKind) -> egui::WidgetText {
        match pane {
            PaneKind::NodeStatus => "Node".into(),
            PaneKind::Wallet     => "Wallet".into(),
            PaneKind::Explorer   => "Explorer".into(),
            PaneKind::Inference  => "Inference".into(),
            PaneKind::Staking    => "Staking".into(),
            PaneKind::Settings   => "Settings".into(),
        }
    }

    fn pane_ui(
        &mut self,
        ui: &mut egui::Ui,
        _tile_id: egui_tiles::TileId,
        pane: &mut PaneKind,
    ) -> egui_tiles::UiResponse {
        egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .show(ui, |ui| {
                ui.add_space(8.0);
                match pane {
                    PaneKind::NodeStatus => node::show(ui, self.data),
                    PaneKind::Wallet     => wallet::show(ui, self.data),
                    PaneKind::Explorer   => explorer::show(ui, self.data),
                    PaneKind::Inference  => inference::show(ui, self.data),
                    PaneKind::Staking    => staking::show(ui, self.data),
                    PaneKind::Settings   => settings::show(ui, self.data),
                }
                ui.add_space(16.0);
            });
        egui_tiles::UiResponse::None
    }

    fn tab_bar_height(&self, _style: &egui::Style) -> f32 { 32.0 }
    fn gap_width(&self, _style: &egui::Style) -> f32 { 2.0 }

    fn tab_bar_color(&self, _visuals: &egui::Visuals) -> egui::Color32 {
        egui::Color32::from_rgb(15, 15, 22)
    }

    fn tab_bg_color(
        &self,
        _visuals: &egui::Visuals,
        _tiles: &egui_tiles::Tiles<PaneKind>,
        _tile_id: egui_tiles::TileId,
        state: &egui_tiles::TabState,
    ) -> egui::Color32 {
        if state.active {
            egui::Color32::from_rgb(26, 26, 36)
        } else {
            egui::Color32::TRANSPARENT
        }
    }

    fn tab_text_color(
        &self,
        _visuals: &egui::Visuals,
        _tiles: &egui_tiles::Tiles<PaneKind>,
        _tile_id: egui_tiles::TileId,
        state: &egui_tiles::TabState,
    ) -> egui::Color32 {
        if state.active {
            ORANGE
        } else {
            egui::Color32::from_rgb(130, 130, 155)
        }
    }

    fn tab_outline_stroke(
        &self,
        _visuals: &egui::Visuals,
        _tiles: &egui_tiles::Tiles<PaneKind>,
        _tile_id: egui_tiles::TileId,
        state: &egui_tiles::TabState,
    ) -> egui::Stroke {
        if state.active {
            egui::Stroke::new(1.5, ORANGE)
        } else {
            egui::Stroke::NONE
        }
    }

    fn tab_bar_hline_stroke(&self, _visuals: &egui::Visuals) -> egui::Stroke {
        egui::Stroke::new(1.0, egui::Color32::from_rgb(38, 38, 55))
    }

    fn simplification_options(&self) -> egui_tiles::SimplificationOptions {
        egui_tiles::SimplificationOptions {
            all_panes_must_have_tabs: true,
            ..Default::default()
        }
    }
}
