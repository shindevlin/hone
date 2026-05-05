use super::AppData;

pub fn show(ui: &mut egui::Ui, data: &AppData) {
    ui.heading("Node Status");
    ui.separator();

    let info = data.node_info.as_ref();
    let status = data.explorer_status.as_ref();

    egui::Grid::new("node_grid")
        .striped(true)
        .num_columns(2)
        .spacing([16.0, 4.0])
        .show(ui, |ui| {
            ui.strong("Field");
            ui.strong("Value");
            ui.end_row();

            ui.label("Epoch");
            ui.label(
                info.and_then(|v| v.get("epoch"))
                    .and_then(|v| v.as_u64())
                    .map(|e| e.to_string())
                    .unwrap_or_else(|| "—".to_string()),
            );
            ui.end_row();

            ui.label("Chain ID");
            ui.label(
                info.and_then(|v| v.get("chain_id"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("—"),
            );
            ui.end_row();

            ui.label("Version");
            ui.label(
                info.and_then(|v| v.get("version"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("—"),
            );
            ui.end_row();

            ui.label("Peers");
            ui.label(
                info.and_then(|v| v.get("peer_count"))
                    .and_then(|v| v.as_u64())
                    .map(|e| e.to_string())
                    .unwrap_or_else(|| "—".to_string()),
            );
            ui.end_row();

            ui.label("Block Hash");
            ui.label(
                info.and_then(|v| v.get("block_hash"))
                    .and_then(|v| v.as_str())
                    .unwrap_or("—"),
            );
            ui.end_row();

            ui.label("Block Count");
            ui.label(
                status
                    .and_then(|v| v.get("block_count"))
                    .and_then(|v| v.as_u64())
                    .map(|e| e.to_string())
                    .unwrap_or_else(|| "—".to_string()),
            );
            ui.end_row();

            ui.label("Status");
            let connected = info.is_some();
            if connected {
                ui.colored_label(egui::Color32::GREEN, "Connected");
            } else {
                ui.colored_label(egui::Color32::RED, "Disconnected");
            }
            ui.end_row();
        });

    if !data.status_msg.is_empty() {
        ui.separator();
        ui.colored_label(egui::Color32::YELLOW, &data.status_msg);
    }
}
