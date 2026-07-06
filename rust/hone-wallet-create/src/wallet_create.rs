use anyhow::{anyhow, Context, Result};
use hone_sdk::Wallet;
use eframe::egui;
use serde_json::{json, Value};
use std::path::{Path, PathBuf};

const DEFAULT_NODE_URL: &str = "http://localhost:4242";

#[derive(Default)]
pub struct WalletCreateApp {
    account: String,
    wallet_file: String,
    signer_file: String,
    node_url: String,
    mnemonic: String,
    export_signer: bool,
    summary: Option<WalletSummary>,
    status: Option<(bool, String)>,
    register_status: Option<(bool, String)>,
}

struct WalletSummary {
    wallet_path: PathBuf,
    signer_path: Option<PathBuf>,
    role_keys: Vec<(String, String)>,
    addresses: Vec<(String, String, String)>,
}

impl WalletCreateApp {
    pub fn new(cc: &eframe::CreationContext<'_>) -> Self {
        setup_theme(&cc.egui_ctx);
        Self {
            node_url: DEFAULT_NODE_URL.to_string(),
            export_signer: false,
            ..Default::default()
        }
    }

    fn normalized_account(&self) -> String {
        self.account.trim().to_lowercase()
    }

    fn wallet_path_for(&self, account: &str) -> PathBuf {
        if self.wallet_file.trim().is_empty() {
            default_wallet_path(account)
        } else {
            PathBuf::from(self.wallet_file.trim())
        }
    }

    fn signer_path_for(&self, account: &str) -> PathBuf {
        if self.signer_file.trim().is_empty() {
            default_signer_path(account)
        } else {
            PathBuf::from(self.signer_file.trim())
        }
    }

    fn generate_offline_wallet(&mut self) {
        self.status = None;
        self.register_status = None;
        self.summary = None;

        let account = self.normalized_account();
        if account.is_empty() {
            self.status = Some((false, "Account name is required.".into()));
            return;
        }

        let wallet_path = self.wallet_path_for(&account);
        let signer_path = self.signer_path_for(&account);
        if wallet_path.exists() {
            self.status = Some((false, format!("Wallet file already exists: {}", wallet_path.display())));
            return;
        }
        if self.export_signer && signer_path.exists() {
            self.status = Some((false, format!("Signer file already exists: {}", signer_path.display())));
            return;
        }

        match generate_wallet_files(&account, &wallet_path, self.export_signer.then_some(signer_path.as_path())) {
            Ok((mnemonic, summary)) => {
                self.account = account;
                self.wallet_file = summary.wallet_path.display().to_string();
                if let Some(ref p) = summary.signer_path {
                    self.signer_file = p.display().to_string();
                }
                self.mnemonic = mnemonic;
                self.summary = Some(summary);
                self.status = Some((true, "Offline wallet generated. Write down the mnemonic now.".into()));
            }
            Err(e) => self.status = Some((false, e.to_string())),
        }
    }

    fn register_online_name(&mut self) {
        self.register_status = None;
        let account = self.normalized_account();
        if account.is_empty() {
            self.register_status = Some((false, "Account name is required.".into()));
            return;
        }
        if self.mnemonic.trim().is_empty() {
            self.register_status = Some((false, "Mnemonic is required. Generate or paste it first.".into()));
            return;
        }
        let node_url = if self.node_url.trim().is_empty() {
            DEFAULT_NODE_URL.to_string()
        } else {
            self.node_url.trim().trim_end_matches('/').to_string()
        };

        match register_account(&node_url, &account, self.mnemonic.trim()) {
            Ok(msg) => self.register_status = Some((true, msg)),
            Err(e) => self.register_status = Some((false, e.to_string())),
        }
    }
}

impl eframe::App for WalletCreateApp {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        egui::ScrollArea::vertical()
            .auto_shrink([false; 2])
            .show(ui, |ui| self.show(ui));
    }
}

impl WalletCreateApp {
    fn show(&mut self, ui: &mut egui::Ui) {
        ui.add_space(18.0);
        ui.horizontal(|ui| {
            ui.add_space(18.0);
            ui.vertical(|ui| {
                ui.label(egui::RichText::new("HONE Wallet Create").size(34.0).strong().color(ORANGE));
                ui.label(egui::RichText::new("Standalone offline-first account and wallet creation.").size(14.0).color(MUTED));
            });
        });

        ui.add_space(22.0);
        ui.columns(2, |cols| {
            cols[0].vertical(|ui| self.show_generate_card(ui));
            cols[1].vertical(|ui| self.show_register_card(ui));
        });

        ui.add_space(16.0);
        self.show_summary(ui);
    }

    fn show_generate_card(&mut self, ui: &mut egui::Ui) {
        card(ui, |ui| {
            heading(ui, "1. Generate offline wallet");
            ui.label(egui::RichText::new("No network is required. The public wallet file can be shared; the mnemonic must stay offline/private.").small().color(MUTED));
            ui.add_space(12.0);

            labeled_text(ui, "Account name", &mut self.account, "example: alice");
            labeled_text(ui, "Public wallet file", &mut self.wallet_file, "blank = ~/.hone/{account}.wallet.key");

            ui.add_space(4.0);
            ui.checkbox(&mut self.export_signer, "Also export current-node signer file (unencrypted, chmod 600)");
            if self.export_signer {
                labeled_text(ui, "Signer file", &mut self.signer_file, "blank = ~/.hone/{account}.signer.key");
            }

            ui.add_space(12.0);
            if ui.add(primary_button("Generate wallet files")).clicked() {
                self.generate_offline_wallet();
            }

            if let Some((ok, msg)) = &self.status {
                status_line(ui, *ok, msg);
            }
        });
    }

    fn show_register_card(&mut self, ui: &mut egui::Ui) {
        card(ui, |ui| {
            heading(ui, "2. Register account name");
            ui.label(egui::RichText::new("Optional online step. This claims the HONE account name on a node using owner-key signatures.").small().color(MUTED));
            ui.add_space(12.0);

            labeled_text(ui, "Node URL", &mut self.node_url, DEFAULT_NODE_URL);
            ui.add_space(4.0);
            ui.label(egui::RichText::new("Mnemonic").small().color(MUTED));
            ui.add(egui::TextEdit::multiline(&mut self.mnemonic)
                .desired_rows(4)
                .desired_width(ui.available_width())
                .font(egui::TextStyle::Monospace)
                .hint_text("Generate offline, or paste mnemonic to restore/register."));

            ui.add_space(8.0);
            ui.horizontal(|ui| {
                if ui.button("Copy mnemonic").clicked() {
                    ui.ctx().copy_text(self.mnemonic.clone());
                }
                if ui.button("Clear mnemonic from screen").clicked() {
                    self.mnemonic.clear();
                }
            });

            ui.add_space(12.0);
            if ui.add(primary_button("Register account on node")).clicked() {
                self.register_online_name();
            }

            if let Some((ok, msg)) = &self.register_status {
                status_line(ui, *ok, msg);
            }
        });
    }

    fn show_summary(&self, ui: &mut egui::Ui) {
        card(ui, |ui| {
            heading(ui, "Generated wallet details");
            if let Some(summary) = &self.summary {
                key_value(ui, "Wallet file", &summary.wallet_path.display().to_string());
                key_value(ui, "Signer file", summary.signer_path.as_ref()
                    .map(|p| p.display().to_string())
                    .as_deref()
                    .unwrap_or("not exported"));

                ui.add_space(12.0);
                ui.label(egui::RichText::new("HONE role public keys").strong().color(TEXT));
                for (role, key) in &summary.role_keys {
                    key_value(ui, role, key);
                }

                ui.add_space(12.0);
                ui.label(egui::RichText::new("Derived public addresses").strong().color(TEXT));
                for (chain, address, path) in &summary.addresses {
                    key_value(ui, chain, &format!("{}  ({})", address, path));
                }
            } else {
                ui.label(egui::RichText::new("No wallet generated in this session yet.").color(MUTED));
            }
        });
    }
}

fn generate_wallet_files(account: &str, wallet_path: &Path, signer_path: Option<&Path>) -> Result<(String, WalletSummary)> {
    let wallet = Wallet::generate(account)?;
    wallet.save_to_file(wallet_path)
        .with_context(|| format!("failed to save wallet file {}", wallet_path.display()))?;

    let signer_path = if let Some(path) = signer_path {
        save_signer_file(&wallet, path)
            .with_context(|| format!("failed to save signer file {}", path.display()))?;
        Some(path.to_path_buf())
    } else {
        None
    };

    let role_map = wallet.hone_role_public_keys()?;
    let role_keys = ["owner", "active", "posting", "memo", "hide", "seek"]
        .into_iter()
        .filter_map(|role| role_map.get(role).map(|key| (role.to_string(), key.clone())))
        .collect();

    let summary = WalletSummary {
        wallet_path: wallet_path.to_path_buf(),
        signer_path,
        role_keys,
        addresses: wallet.chain_addresses(),
    };
    Ok((wallet.mnemonic.to_string(), summary))
}

fn save_signer_file(wallet: &Wallet, path: &Path) -> Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let owner = wallet.hone_role_keypair("owner")?;
    let active = wallet.hone_role_keypair("active")?;
    let posting = wallet.hone_role_keypair("posting")?;
    let signer = json!({
        "version": 3,
        "format": "hone-local-signer",
        "account": wallet.account,
        "hone_owner_private_key": owner.private_key_hex(),
        "hone_owner_public_key": owner.public_key_hex(),
        "hone_active_private_key": active.private_key_hex(),
        "hone_active_public_key": active.public_key_hex(),
        "hone_posting_private_key": posting.private_key_hex(),
        "hone_posting_public_key": posting.public_key_hex(),
        "hone_private_key": posting.private_key_hex(),
        "hone_public_key_hex": posting.public_key_hex(),
    });
    std::fs::write(path, serde_json::to_string_pretty(&signer)?)?;

    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o600))?;
    }

    Ok(())
}

fn register_account(base: &str, account: &str, mnemonic: &str) -> Result<String> {
    let wallet = Wallet::from_phrase(mnemonic, account)?;
    let owner = wallet.hone_role_keypair("owner")?;
    let role_keys = wallet.hone_role_public_keys()?;
    let chain_id = get_chain_id(base)?;

    let create_sig = sign_account_create(&owner, &chain_id, account, &role_keys)?;
    let create_resp = post_json(base, "/api/account/create", &json!({
        "account": account,
        "keys": role_keys,
        "signature": create_sig,
    }))?;
    if !accepted(&create_resp) {
        return Err(anyhow!("AccountCreate rejected: {}", response_error(&create_resp)));
    }

    let nonce = get_nonce(base, account)?;
    let epoch = get_epoch(base);
    let chains: Vec<Value> = wallet.chain_addresses()
        .into_iter()
        .map(|(chain, address, derivation_path)| {
            let msg = format!("hone-family:{}:{}:{}", account, chain, address);
            json!({
                "address": address,
                "chain": chain,
                "derivation_path": derivation_path,
                "signature": owner.sign_entry_json(&msg),
            })
        })
        .collect();
    let family_sig = sign_wallet_family(&owner, &chain_id, account, &chains, nonce)?;
    let publish_resp = post_json(base, "/api/wallet/family/publish", &json!({
        "account": account,
        "chains": chains,
        "epoch": epoch,
        "nonce": nonce,
        "signed_by": account,
        "signature": family_sig,
    }))?;
    if !accepted(&publish_resp) {
        return Err(anyhow!("WalletFamilyPublish rejected: {}", response_error(&publish_resp)));
    }

    let create_hash = create_resp.get("hash").and_then(|v| v.as_str()).unwrap_or("accepted");
    let family_hash = publish_resp.get("hash").and_then(|v| v.as_str()).unwrap_or("accepted");
    Ok(format!(
        "Registered. account hash: {}; wallet-family hash: {}",
        &create_hash[..create_hash.len().min(16)],
        &family_hash[..family_hash.len().min(16)]
    ))
}

fn sign_account_create(
    keypair: &hone_sdk::KeyPair,
    chain_id: &str,
    account: &str,
    role_keys: &std::collections::HashMap<String, String>,
) -> Result<String> {
    let sorted_keys: std::collections::BTreeMap<_, _> = role_keys.iter().collect();
    let msg = serde_json::to_string(&json!({
        "account": account,
        "chain_id": chain_id,
        "chain_proofs": [],
        "funded_by": null,
        "keys": sorted_keys,
        "type": "ACCOUNT_CREATE",
    }))?;
    Ok(keypair.sign_entry_json(&msg))
}

fn sign_wallet_family(
    keypair: &hone_sdk::KeyPair,
    chain_id: &str,
    account: &str,
    chains: &[Value],
    nonce: u64,
) -> Result<String> {
    let msg = serde_json::to_string(&json!({
        "account": account,
        "chain_id": chain_id,
        "chains": chains,
        "nonce": nonce,
        "type": "WALLET_FAMILY_PUBLISH",
    }))?;
    Ok(keypair.sign_entry_json(&msg))
}

fn get_chain_id(base: &str) -> Result<String> {
    let v = get_json(base, "/api/node/info")?;
    v.get("chain_id")
        .and_then(|s| s.as_str())
        .map(str::to_owned)
        .ok_or_else(|| anyhow!("/api/node/info missing chain_id"))
}

fn get_epoch(base: &str) -> u64 {
    get_json(base, "/api/node/info").ok()
        .and_then(|v| v.get("epoch").and_then(|e| e.as_u64()))
        .or_else(|| get_json(base, "/api/latest").ok()
            .and_then(|v| v.get("current_epoch").and_then(|e| e.as_u64())))
        .unwrap_or(0)
}

fn get_nonce(base: &str, account: &str) -> Result<u64> {
    let v = get_json(base, &format!("/api/account/{}", account))?;
    v.get("nonce")
        .and_then(|n| n.as_u64())
        .map(|n| n + 1)
        .ok_or_else(|| anyhow!("account '{}' not found or missing nonce after AccountCreate", account))
}

fn get_json(base: &str, path: &str) -> Result<Value> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        .build()?;
    let resp = client.get(format!("{}{}", base, path)).send()?;
    let status = resp.status();
    let value: Value = resp.json()?;
    if !status.is_success() {
        return Err(anyhow!("HTTP {}: {}", status, response_error(&value)));
    }
    Ok(value)
}

fn post_json(base: &str, path: &str, body: &Value) -> Result<Value> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(12))
        .build()?;
    let resp = client.post(format!("{}{}", base, path)).json(body).send()?;
    let status = resp.status();
    let value: Value = resp.json()?;
    if !status.is_success() {
        return Err(anyhow!("HTTP {}: {}", status, response_error(&value)));
    }
    Ok(value)
}

fn accepted(v: &Value) -> bool {
    v.get("accepted").and_then(|v| v.as_bool()).unwrap_or(false)
        || v.get("ok").and_then(|v| v.as_bool()).unwrap_or(false)
}

fn response_error(v: &Value) -> String {
    v.get("error")
        .or_else(|| v.get("message"))
        .and_then(|v| v.as_str())
        .unwrap_or("unknown")
        .to_string()
}

fn default_wallet_path(account: &str) -> PathBuf {
    home_hone_dir().join(format!("{}.wallet.key", account))
}

fn default_signer_path(account: &str) -> PathBuf {
    home_hone_dir().join(format!("{}.signer.key", account))
}

fn home_hone_dir() -> PathBuf {
    PathBuf::from(std::env::var("HOME").unwrap_or_else(|_| ".".into())).join(".hone")
}

fn setup_theme(ctx: &egui::Context) {
    let mut visuals = egui::Visuals::dark();
    visuals.panel_fill = BG;
    visuals.window_fill = SURFACE;
    visuals.extreme_bg_color = BG;
    visuals.faint_bg_color = CARD;
    visuals.selection.bg_fill = ORANGE;
    visuals.hyperlink_color = ORANGE;
    visuals.widgets.inactive.bg_fill = FIELD;
    visuals.widgets.hovered.bg_fill = FIELD_HOVER;
    visuals.widgets.active.bg_fill = FIELD_HOVER;
    visuals.widgets.noninteractive.bg_stroke = egui::Stroke::new(1.0, BORDER);
    visuals.widgets.inactive.bg_stroke = egui::Stroke::new(1.0, BORDER);
    visuals.override_text_color = Some(TEXT);
    ctx.set_visuals(visuals);
}

fn card<R>(ui: &mut egui::Ui, add: impl FnOnce(&mut egui::Ui) -> R) -> R {
    egui::Frame::new()
        .fill(CARD)
        .stroke(egui::Stroke::new(1.0, BORDER))
        .corner_radius(egui::CornerRadius::same(14))
        .inner_margin(egui::Margin::same(18_i8))
        .show(ui, add)
        .inner
}

fn heading(ui: &mut egui::Ui, text: &str) {
    ui.label(egui::RichText::new(text).size(18.0).strong().color(TEXT));
}

fn labeled_text(ui: &mut egui::Ui, label: &str, value: &mut String, hint: &str) {
    ui.label(egui::RichText::new(label).small().color(MUTED));
    ui.add(egui::TextEdit::singleline(value)
        .desired_width(ui.available_width())
        .hint_text(hint));
    ui.add_space(8.0);
}

fn key_value(ui: &mut egui::Ui, key: &str, value: &str) {
    ui.horizontal_wrapped(|ui| {
        ui.label(egui::RichText::new(format!("{}:", key)).monospace().small().color(MUTED));
        ui.label(egui::RichText::new(value).monospace().small().color(TEXT));
    });
}

fn status_line(ui: &mut egui::Ui, ok: bool, msg: &str) {
    ui.add_space(8.0);
    let color = if ok { GREEN } else { RED };
    ui.label(egui::RichText::new(msg).color(color).small());
}

fn primary_button(text: &str) -> egui::Button<'_> {
    egui::Button::new(egui::RichText::new(text).strong())
        .fill(ORANGE)
        .min_size(egui::vec2(190.0, 34.0))
}

const BG: egui::Color32 = egui::Color32::from_rgb(9, 11, 13);
const SURFACE: egui::Color32 = egui::Color32::from_rgb(16, 19, 22);
const CARD: egui::Color32 = egui::Color32::from_rgb(20, 24, 28);
const FIELD: egui::Color32 = egui::Color32::from_rgb(28, 33, 38);
const FIELD_HOVER: egui::Color32 = egui::Color32::from_rgb(38, 44, 50);
const BORDER: egui::Color32 = egui::Color32::from_rgb(48, 55, 62);
const TEXT: egui::Color32 = egui::Color32::from_rgb(226, 232, 240);
const MUTED: egui::Color32 = egui::Color32::from_rgb(136, 148, 162);
const ORANGE: egui::Color32 = egui::Color32::from_rgb(247, 147, 26);
const GREEN: egui::Color32 = egui::Color32::from_rgb(74, 222, 128);
const RED: egui::Color32 = egui::Color32::from_rgb(248, 113, 113);
