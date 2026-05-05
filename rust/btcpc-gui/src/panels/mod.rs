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
        }
    }
}

pub struct BtcpcBehavior<'a> {
    pub data: &'a mut AppData,
}

impl<'a> egui_tiles::Behavior<PaneKind> for BtcpcBehavior<'a> {
    fn tab_title_for_pane(&mut self, pane: &PaneKind) -> egui::WidgetText {
        match pane {
            PaneKind::NodeStatus => "Node Status".into(),
            PaneKind::Wallet => "Wallet".into(),
            PaneKind::Explorer => "Explorer".into(),
            PaneKind::Inference => "Inference".into(),
            PaneKind::Staking => "Staking".into(),
            PaneKind::Settings => "Settings".into(),
        }
    }

    fn pane_ui(
        &mut self,
        ui: &mut egui::Ui,
        _tile_id: egui_tiles::TileId,
        pane: &mut PaneKind,
    ) -> egui_tiles::UiResponse {
        match pane {
            PaneKind::NodeStatus => node::show(ui, self.data),
            PaneKind::Wallet => wallet::show(ui, self.data),
            PaneKind::Explorer => explorer::show(ui, self.data),
            PaneKind::Inference => inference::show(ui, self.data),
            PaneKind::Staking => staking::show(ui, self.data),
            PaneKind::Settings => settings::show(ui, self.data),
        }
        egui_tiles::UiResponse::None
    }

    fn simplification_options(&self) -> egui_tiles::SimplificationOptions {
        egui_tiles::SimplificationOptions {
            all_panes_must_have_tabs: true,
            ..Default::default()
        }
    }
}
