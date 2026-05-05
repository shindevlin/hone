use super::AppData;

fn dreams_to_btcpc(dreams: u64) -> String {
    format!("{:.8}", dreams as f64 / 100_000_000.0)
}

pub fn show(ui: &mut egui::Ui, data: &AppData) {
    ui.heading("Explorer");
    ui.separator();

    // Summary row
    if let Some(ref status) = data.explorer_status {
        egui::Grid::new("explorer_summary")
            .num_columns(2)
            .spacing([16.0, 4.0])
            .show(ui, |ui| {
                ui.strong("Total Supply");
                ui.label(
                    status
                        .get("total_supply")
                        .and_then(|v| v.as_u64())
                        .map(|s| format!("{} BTCPC", dreams_to_btcpc(s)))
                        .unwrap_or_else(|| "—".to_string()),
                );
                ui.end_row();

                ui.strong("Circulating");
                ui.label(
                    status
                        .get("circulating_supply")
                        .and_then(|v| v.as_u64())
                        .map(|s| format!("{} BTCPC", dreams_to_btcpc(s)))
                        .unwrap_or_else(|| "—".to_string()),
                );
                ui.end_row();

                ui.strong("Staked Total");
                ui.label(
                    status
                        .get("staked_total")
                        .and_then(|v| v.as_u64())
                        .map(|s| format!("{} BTCPC", dreams_to_btcpc(s)))
                        .unwrap_or_else(|| "—".to_string()),
                );
                ui.end_row();
            });
        ui.separator();
    }

    ui.strong("Recent Blocks");
    egui::ScrollArea::vertical().show(ui, |ui| {
        egui::Grid::new("blocks_grid")
            .striped(true)
            .num_columns(4)
            .spacing([16.0, 4.0])
            .show(ui, |ui| {
                ui.strong("Epoch");
                ui.strong("Hash");
                ui.strong("Entries");
                ui.strong("Timestamp");
                ui.end_row();

                for block in &data.blocks {
                    ui.label(
                        block
                            .get("epoch")
                            .and_then(|v| v.as_u64())
                            .map(|e| e.to_string())
                            .unwrap_or_else(|| "—".to_string()),
                    );

                    let hash = block
                        .get("hash")
                        .and_then(|v| v.as_str())
                        .unwrap_or("—");
                    let hash_short = if hash.len() > 16 { &hash[..16] } else { hash };
                    ui.monospace(hash_short);

                    ui.label(
                        block
                            .get("entry_count")
                            .and_then(|v| v.as_u64())
                            .map(|e| e.to_string())
                            .unwrap_or_else(|| "—".to_string()),
                    );

                    ui.label(
                        block
                            .get("timestamp_ms")
                            .and_then(|v| v.as_u64())
                            .map(|t| t.to_string())
                            .unwrap_or_else(|| "—".to_string()),
                    );
                    ui.end_row();
                }
            });
    });
}
