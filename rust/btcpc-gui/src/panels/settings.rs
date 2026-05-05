use super::AppData;

pub fn show(ui: &mut egui::Ui, data: &mut AppData) {
    ui.heading("Settings");
    ui.separator();

    egui::Grid::new("settings_grid")
        .num_columns(2)
        .spacing([16.0, 4.0])
        .show(ui, |ui| {
            ui.strong("Node URL");
            ui.text_edit_singleline(&mut data.node_url);
            ui.end_row();

            ui.strong("Account");
            ui.label(data.account.as_deref().unwrap_or("none (run btcpc login)"));
            ui.end_row();
        });

    ui.add_space(8.0);
    if ui.button("Apply").clicked() {
        // node_url is already updated by the text_edit widget
    }
}
