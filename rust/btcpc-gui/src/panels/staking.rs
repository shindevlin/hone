use super::AppData;

fn dreams_to_btcpc(dreams: u64) -> String {
    format!("{:.8}", dreams as f64 / 100_000_000.0)
}

fn parse_btcpc_input(s: &str) -> Option<u64> {
    s.trim().parse::<f64>().ok().map(|a| (a * 10_000_000_000.0) as u64)
}

pub fn show(ui: &mut egui::Ui, data: &mut AppData) {
    ui.heading("Staking");
    ui.separator();

    let account = match data.account.clone() {
        Some(a) => a,
        None => {
            ui.label("Not logged in. Run: btcpc login --account <name>");
            return;
        }
    };

    egui::Grid::new("staking_info_grid")
        .num_columns(2)
        .spacing([16.0, 4.0])
        .show(ui, |ui| {
            ui.strong("Account");
            ui.label(&account);
            ui.end_row();

            ui.strong("Staked");
            ui.label(
                data.staked
                    .map(|s| format!("{} BTCPC", dreams_to_btcpc(s)))
                    .unwrap_or_else(|| "—".to_string()),
            );
            ui.end_row();
        });

    ui.separator();

    egui::Grid::new("stake_form")
        .num_columns(2)
        .spacing([8.0, 4.0])
        .show(ui, |ui| {
            ui.label("Amount:");
            ui.horizontal(|ui| {
                ui.text_edit_singleline(&mut data.forms.stake_amount);
                ui.label("BTCPC");
            });
            ui.end_row();
        });

    ui.horizontal(|ui| {
        let add_clicked = ui.button("Add Stake").clicked();
        let remove_clicked = ui.button("Remove Stake").clicked();

        if add_clicked || remove_clicked {
            let base = data.node_url.clone();
            let acct = account.clone();
            let amount_str = data.forms.stake_amount.clone();
            let key_file = data.key_file.clone();
            let add = add_clicked;

            match (key_file, parse_btcpc_input(&amount_str)) {
                (_, None) => {
                    data.forms.stake_result = Some((false, "invalid amount".to_string()));
                }
                (None, _) => {
                    data.forms.stake_result = Some((false, "no key file in session".to_string()));
                }
                (Some(kf), Some(dreams)) => {
                    match crate::sign::submit_stake(&base, &kf, &acct, dreams, add) {
                        Ok(msg) => data.forms.stake_result = Some((true, msg)),
                        Err(e)  => data.forms.stake_result = Some((false, e.to_string())),
                    }
                }
            }
        }
    });

    if let Some((ok, ref msg)) = data.forms.stake_result {
        let color = if ok { egui::Color32::GREEN } else { egui::Color32::RED };
        ui.colored_label(color, msg);
    }
}
