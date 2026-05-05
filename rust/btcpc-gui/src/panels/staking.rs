use super::AppData;

fn dreams_to_btcpc(dreams: u64) -> String {
    format!("{:.8}", dreams as f64 / 100_000_000.0)
}

pub fn show(ui: &mut egui::Ui, data: &AppData) {
    ui.heading("Staking");
    ui.separator();

    egui::Grid::new("staking_grid")
        .num_columns(2)
        .spacing([16.0, 4.0])
        .show(ui, |ui| {
            ui.strong("Account");
            ui.label(
                data.account
                    .as_deref()
                    .unwrap_or("—"),
            );
            ui.end_row();

            ui.strong("Staked");
            ui.label(
                data.staked
                    .map(|s| format!("{} BTCPC", dreams_to_btcpc(s)))
                    .unwrap_or_else(|| "—".to_string()),
            );
            ui.end_row();
        });
}
