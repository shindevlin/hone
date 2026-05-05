use crate::api;
use crate::panels::{AppData, BtcpcBehavior, PaneKind};

#[derive(serde::Deserialize, Clone)]
pub struct Session {
    pub account: String,
    #[allow(dead_code)]
    pub key_file: std::path::PathBuf,
    pub node_url: String,
}

fn load_session() -> Option<Session> {
    let home = std::env::var("HOME").ok()?;
    let path = std::path::PathBuf::from(home).join(".btcpc").join("session.json");
    let bytes = std::fs::read(path).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn layout_path() -> std::path::PathBuf {
    let home = std::env::var("HOME").unwrap_or_else(|_| ".".to_string());
    std::path::PathBuf::from(home)
        .join(".btcpc")
        .join("gui-layout.json")
}

fn default_tree() -> egui_tiles::Tree<PaneKind> {
    let mut tiles = egui_tiles::Tiles::default();

    let node = tiles.insert_pane(PaneKind::NodeStatus);
    let staking = tiles.insert_pane(PaneKind::Staking);
    let wallet = tiles.insert_pane(PaneKind::Wallet);
    let explorer = tiles.insert_pane(PaneKind::Explorer);
    let inference = tiles.insert_pane(PaneKind::Inference);

    let left = tiles.insert_vertical_tile(vec![node, staking]);
    let right = tiles.insert_vertical_tile(vec![wallet, explorer, inference]);
    let root = tiles.insert_horizontal_tile(vec![left, right]);

    egui_tiles::Tree::new("btcpc_layout", root, tiles)
}

pub struct BtcpcApp {
    pub tree: egui_tiles::Tree<PaneKind>,
    pub data: AppData,
    #[allow(dead_code)]
    pub session: Option<Session>,
    pub last_refresh: std::time::Instant,
    pub layout_path: std::path::PathBuf,
    /// Queue of panes to add on next frame
    pending_add: Option<PaneKind>,
}

impl BtcpcApp {
    pub fn new(_cc: &eframe::CreationContext) -> Self {
        let session = load_session();
        let lp = layout_path();

        let tree = std::fs::read_to_string(&lp)
            .ok()
            .and_then(|s| serde_json::from_str::<egui_tiles::Tree<PaneKind>>(&s).ok())
            .unwrap_or_else(default_tree);

        let node_url = std::env::var("BTCPC_API_URL")
            .ok()
            .filter(|s| !s.trim().is_empty())
            .or_else(|| session.as_ref().map(|s| s.node_url.clone()))
            .unwrap_or_else(|| "http://localhost:4242".to_string());

        let account = session.as_ref().map(|s| s.account.clone());
        let key_file = session.as_ref().map(|s| s.key_file.clone());

        let mut app = BtcpcApp {
            tree,
            data: AppData {
                node_url,
                account,
                key_file,
                ..Default::default()
            },
            session,
            last_refresh: std::time::Instant::now()
                .checked_sub(std::time::Duration::from_secs(20))
                .unwrap_or(std::time::Instant::now()),
            layout_path: lp,
            pending_add: None,
        };
        app.refresh();
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
                    if let Some(b) = v.get("balance").and_then(|b| b.as_u64()) {
                        self.data.balance = Some(b);
                    }
                }
                Err(e) => self.data.status_msg = format!("balance: {}", e),
            }
            match api::get_json(&base, &format!("/api/account/{}", account)) {
                Ok(v) => {
                    if let Some(s) = v.get("staked").and_then(|s| s.as_u64()) {
                        self.data.staked = Some(s);
                    }
                }
                Err(e) => self.data.status_msg = format!("account: {}", e),
            }
        }

        self.last_refresh = std::time::Instant::now();
    }

    fn add_pane_to_tree(&mut self, kind: PaneKind) {
        let pane_id = self.tree.tiles.insert_pane(kind);
        // Wrap in a tab container and add to root
        let tab_id = self.tree.tiles.insert_tab_tile(vec![pane_id]);
        if let Some(root) = self.tree.root {
            // Try to add to root as a horizontal sibling
            if let Some(egui_tiles::Tile::Container(container)) =
                self.tree.tiles.get_mut(root)
            {
                container.add_child(tab_id);
            } else {
                // Root is a pane; create new horizontal with both
                let new_root =
                    self.tree.tiles.insert_horizontal_tile(vec![root, tab_id]);
                self.tree.root = Some(new_root);
            }
        } else {
            self.tree.root = Some(pane_id);
        }
    }
}

impl eframe::App for BtcpcApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        // Process pending add
        if let Some(kind) = self.pending_add.take() {
            self.add_pane_to_tree(kind);
        }

        // Auto-refresh every 10 seconds
        if self.last_refresh.elapsed() >= std::time::Duration::from_secs(10) {
            self.data.status_msg.clear();
            self.refresh();
            ui.ctx().request_repaint();
        }

        // Menu bar
        egui::MenuBar::new().ui(ui, |ui| {
            ui.menu_button("File", |ui| {
                if ui.button("Quit").clicked() {
                    ui.ctx().send_viewport_cmd(egui::ViewportCommand::Close);
                }
            });
            ui.menu_button("Panels", |ui| {
                if ui.button("Node Status").clicked() {
                    self.pending_add = Some(PaneKind::NodeStatus);
                    ui.close();
                }
                if ui.button("Wallet").clicked() {
                    self.pending_add = Some(PaneKind::Wallet);
                    ui.close();
                }
                if ui.button("Explorer").clicked() {
                    self.pending_add = Some(PaneKind::Explorer);
                    ui.close();
                }
                if ui.button("Inference").clicked() {
                    self.pending_add = Some(PaneKind::Inference);
                    ui.close();
                }
                if ui.button("Staking").clicked() {
                    self.pending_add = Some(PaneKind::Staking);
                    ui.close();
                }
                if ui.button("Settings").clicked() {
                    self.pending_add = Some(PaneKind::Settings);
                    ui.close();
                }
            });
            if ui.button("Refresh").clicked() {
                self.data.status_msg.clear();
                self.refresh();
            }
            ui.separator();
            if !self.data.status_msg.is_empty() {
                ui.colored_label(egui::Color32::YELLOW, &self.data.status_msg);
            }
        });

        // Central tile tree
        let mut behavior = BtcpcBehavior {
            data: &mut self.data,
        };
        self.tree.ui(&mut behavior, ui);
    }

    fn on_exit(&mut self, _gl: Option<&glow::Context>) {
        self.save_layout();
    }
}
