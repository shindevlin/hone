use super::AppData;

fn dreams_to_btcpc(dreams: u64) -> String {
    format!("{:.8}", dreams as f64 / 100_000_000.0)
}

pub fn show(ui: &mut egui::Ui, data: &AppData) {
    ui.heading("Wallet");
    ui.separator();

    if let Some(ref account) = data.account {
        egui::Grid::new("wallet_grid")
            .striped(true)
            .num_columns(2)
            .spacing([16.0, 4.0])
            .show(ui, |ui| {
                ui.strong("Account");
                ui.label(account);
                ui.end_row();

                ui.strong("Balance");
                ui.label(
                    data.balance
                        .map(|b| format!("{} BTCPC", dreams_to_btcpc(b)))
                        .unwrap_or_else(|| "—".to_string()),
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
    } else {
        ui.label("Not logged in.");
        ui.label("");
        ui.monospace("Run: btcpc login --account <name>");
    }
}
