use super::{AppData, format_ts_ms, card, GREEN, RED, DIM_TEXT, ORANGE, YELLOW};

fn pill(ui: &mut egui::Ui, label: &str, value: &str, color: egui::Color32) {
    ui.horizontal(|ui| {
        ui.label(egui::RichText::new(label).size(11.0).color(DIM_TEXT));
        ui.label(egui::RichText::new(value).size(11.0).strong().color(color).monospace());
    });
}

pub fn show(ui: &mut egui::Ui, data: &mut AppData) {
    let info   = data.node_info.as_ref();
    let status = data.explorer_status.as_ref();
    let connected = info.is_some();

    // ── Compact status strip ──────────────────────────────────────────────────
    card(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            // Dot
            let dot_color = if connected { GREEN } else { RED };
            let r = 5.0_f32;
            let (rect, _) = ui.allocate_exact_size(egui::vec2(r * 2.0, r * 2.0), egui::Sense::hover());
            ui.painter().circle_filled(rect.center(), r, dot_color);
            ui.add_space(4.0);

            let epoch = info.and_then(|v| v.get("epoch")).and_then(|v| v.as_u64())
                .map(|e| e.to_string()).unwrap_or_else(|| "—".into());
            let peers = info.and_then(|v| v.get("peer_count")).and_then(|v| v.as_u64())
                .map(|e| e.to_string()).unwrap_or_else(|| "—".into());
            let chain = info.and_then(|v| v.get("chain_id")).and_then(|v| v.as_str()).unwrap_or("—");
            let version = info.and_then(|v| v.get("version")).and_then(|v| v.as_str()).unwrap_or("—");

            ui.label(egui::RichText::new(format!("Epoch {}", epoch)).size(12.0).strong().color(egui::Color32::WHITE));
            ui.add_space(12.0);
            pill(ui, "Peers", &peers, egui::Color32::WHITE);
            ui.add_space(12.0);
            pill(ui, "Chain", chain, ORANGE);
            ui.add_space(12.0);
            pill(ui, "v", version, DIM_TEXT);
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(egui::RichText::new(&data.node_url).size(10.0).color(DIM_TEXT));
            });
        });
    });

    ui.add_space(8.0);

    // ── Roles this node is running ────────────────────────────────────────────
    card(ui, |ui| {
        ui.horizontal_wrapped(|ui| {
            let is_clock  = info.and_then(|v| v.get("is_clock")).and_then(|v| v.as_bool()).unwrap_or(false);
            let is_miner  = info.and_then(|v| v.get("is_miner")).and_then(|v| v.as_bool()).unwrap_or(false);
            let is_sensor = info.and_then(|v| v.get("is_sensor")).and_then(|v| v.as_bool()).unwrap_or(false);
            let is_worker = info.and_then(|v| v.get("is_worker")).and_then(|v| v.as_bool()).unwrap_or(false);
            let account   = info.and_then(|v| v.get("account")).and_then(|v| v.as_str()).unwrap_or("—");

            ui.label(egui::RichText::new(account).size(12.0).strong().color(egui::Color32::WHITE));
            ui.add_space(12.0);

            for (active, label) in [(is_clock, "CLOCK"), (is_miner, "MINER"), (is_sensor, "SENSOR"), (is_worker, "WORKER")] {
                let color = if active { GREEN } else { DIM_TEXT };
                ui.label(egui::RichText::new(label).size(11.0).color(color).monospace());
                ui.add_space(8.0);
            }
        });
    });

    ui.add_space(8.0);

    // ── Supply ────────────────────────────────────────────────────────────────
    card(ui, |ui| {
        let fmt = |v: Option<&serde_json::Value>| {
            v.and_then(|v| v.as_u64())
                .map(|d| format!("{:.2} HONE", d as f64 / 10_000_000_000.0))
                .unwrap_or_else(|| "—".into())
        };
        ui.horizontal_wrapped(|ui| {
            pill(ui, "Circulating", &fmt(status.and_then(|s| s.get("circulating_supply"))), YELLOW);
            ui.add_space(16.0);
            pill(ui, "Staked", &fmt(status.and_then(|s| s.get("staked_total"))), ORANGE);
            ui.add_space(16.0);
            pill(ui, "Cap", "42,000,000 HONE", DIM_TEXT);
        });
    });

    ui.add_space(8.0);

    // ── Last block hash (truncated) ───────────────────────────────────────────
    let hash = info.and_then(|v| v.get("block_hash")).and_then(|v| v.as_str()).unwrap_or("");
    let ts   = info.and_then(|v| v.get("latest_block_ts")).and_then(|v| v.as_u64()).map(format_ts_ms).unwrap_or_else(|| "—".into());
    if !hash.is_empty() {
        ui.label(egui::RichText::new(format!("{} · {}", &hash[..hash.len().min(20)], ts)).size(11.0).color(DIM_TEXT).monospace());
    }

    if !data.status_msg.is_empty() {
        ui.add_space(6.0);
        ui.colored_label(egui::Color32::from_rgb(255, 200, 60), &data.status_msg);
    }
}
