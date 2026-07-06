use super::{AppData, card, section_heading, GREEN, DIM_TEXT, ORANGE, YELLOW, hunits_to_hone, not_signed_in};

pub fn show(ui: &mut egui::Ui, data: &mut AppData) {
    if data.account.is_none() {
        not_signed_in(ui);
        return;
    }

    let is_miner  = data.node_info.as_ref().and_then(|v| v.get("is_miner")).and_then(|v| v.as_bool()).unwrap_or(false);
    let is_worker = data.node_info.as_ref().and_then(|v| v.get("is_worker")).and_then(|v| v.as_bool()).unwrap_or(false);
    let active    = is_miner || is_worker;

    // ── Status ────────────────────────────────────────────────────────────────
    card(ui, |ui| {
        ui.horizontal(|ui| {
            let dot = if active { GREEN } else { DIM_TEXT };
            let (r, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
            ui.painter().circle_filled(r.center(), 5.0, dot);
            ui.add_space(4.0);
            if active {
                let mut roles = Vec::new();
                if is_miner  { roles.push("AI Miner"); }
                if is_worker { roles.push("Worker"); }
                ui.label(egui::RichText::new(roles.join(" + ")).strong().color(GREEN));
            } else {
                ui.label(egui::RichText::new("No active worker role").strong().color(DIM_TEXT));
            }
        });

        if let Some(model) = data.node_info.as_ref().and_then(|v| v.get("model")).and_then(|v| v.as_str()) {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Model").size(11.0).color(DIM_TEXT));
                ui.add_space(4.0);
                ui.label(egui::RichText::new(model).size(11.0).monospace().color(egui::Color32::WHITE));
            });
        }
    });

    ui.add_space(8.0);

    // ── Posting key ───────────────────────────────────────────────────────────
    section_heading(ui, "Posting Key");
    card(ui, |ui| {
        ui.label(egui::RichText::new(
            "Worker submissions are signed with the posting key. Leave blank to use your login key file."
        ).size(11.0).color(DIM_TEXT));
        ui.add_space(6.0);

        ui.horizontal(|ui| {
            let w = ui.available_width() - 72.0;
            ui.add(egui::TextEdit::singleline(&mut data.forms.posting_key_file)
                .hint_text("auto (uses login key)")
                .desired_width(w));
            if ui.add(
                egui::Button::new(egui::RichText::new("Browse").size(11.0))
                    .min_size(egui::vec2(64.0, 22.0))
            ).clicked() {
                let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
                let start = format!("{}/.hone", home);
                let result = std::process::Command::new("zenity")
                    .args(["--file-selection", "--title=Select posting key",
                           &format!("--filename={}/", start)])
                    .output()
                    .or_else(|_| std::process::Command::new("kdialog")
                        .args(["--getopenfilename", &start, "*.key *.json"])
                        .output());
                if let Ok(out) = result {
                    let p = String::from_utf8_lossy(&out.stdout).trim().to_string();
                    if !p.is_empty() { data.forms.posting_key_file = p; }
                }
            }
        });

        let effective = if data.forms.posting_key_file.trim().is_empty() {
            data.key_file.as_ref()
                .map(|p| p.display().to_string())
                .unwrap_or_else(|| "—".into())
        } else {
            data.forms.posting_key_file.trim().to_string()
        };
        ui.add_space(3.0);
        ui.label(egui::RichText::new(format!("Active: {}", effective))
            .size(10.0).color(DIM_TEXT).monospace());
    });

    ui.add_space(8.0);

    // ── Recent rewards ────────────────────────────────────────────────────────
    section_heading(ui, "Recent Rewards");

    let my_account = data.account.clone().unwrap_or_default();
    let rewards: Vec<_> = data.blocks.iter()
        .flat_map(|b| {
            b.get("entries").and_then(|e| e.as_array()).cloned().unwrap_or_default()
        })
        .filter(|e| {
            let t = e.get("type").and_then(|t| t.as_str()).unwrap_or("");
            matches!(t, "MineReward" | "MINE_REWARD")
        })
        .collect();

    if rewards.is_empty() {
        card(ui, |ui| {
            ui.label(egui::RichText::new("No mine rewards in recent blocks").size(12.0).color(DIM_TEXT));
        });
    } else {
        card(ui, |ui| {
            for reward in rewards.iter().take(12) {
                let miner  = reward.get("miner")
                    .or_else(|| reward.get("account"))
                    .and_then(|v| v.as_str()).unwrap_or("?");
                let amount = reward.get("amount").and_then(|v| v.as_u64()).unwrap_or(0);
                let epoch  = reward.get("epoch").and_then(|v| v.as_u64()).unwrap_or(0);
                let is_me  = miner == my_account;

                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(format!("E{:>6}", epoch))
                        .size(11.0).color(DIM_TEXT).monospace());
                    ui.add_space(6.0);
                    ui.label(egui::RichText::new(miner)
                        .size(11.0).monospace()
                        .color(if is_me { GREEN } else { egui::Color32::from_rgb(180, 180, 200) }));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(egui::RichText::new(format!("{} HONE", hunits_to_hone(amount)))
                            .size(11.0).color(if is_me { YELLOW } else { DIM_TEXT }).monospace());
                    });
                });
                ui.add_space(2.0);
            }
        });
    }

    ui.add_space(8.0);

    // ── Registration hint ─────────────────────────────────────────────────────
    if !active {
        section_heading(ui, "Activate Worker");
        card(ui, |ui| {
            ui.label(egui::RichText::new(
                "Stake HONE to your own miner or worker role to start earning rewards."
            ).size(11.0).color(DIM_TEXT));
            ui.add_space(4.0);
            ui.label(egui::RichText::new(
                "→  Staking → Back a Node → enter your account name → role: miner"
            ).size(11.0).color(ORANGE));
        });
    }
}
