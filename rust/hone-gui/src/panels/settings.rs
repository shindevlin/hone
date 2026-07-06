use super::{AppData, section_heading, card, DIM_TEXT, GREEN, BLUE, YELLOW, ORANGE};
use crate::app::KeyRole;

pub fn show(ui: &mut egui::Ui, data: &mut AppData) {
    section_heading(ui, "Connection");

    let mut node_url_edit = data.node_url.clone();

    card(ui, |ui| {
        egui::Grid::new("settings_grid")
            .num_columns(2)
            .spacing([12.0, 8.0])
            .show(ui, |ui| {
                ui.label(egui::RichText::new("Node URL").color(DIM_TEXT).small());
                ui.add(egui::TextEdit::singleline(&mut node_url_edit)
                    .desired_width(280.0));
                ui.end_row();
            });

        ui.add_space(6.0);
        if ui.add(
            egui::Button::new(egui::RichText::new("Apply").strong())
                .min_size(egui::vec2(80.0, 26.0))
        ).clicked() {
            let trimmed = node_url_edit.trim().to_owned();
            if !trimmed.is_empty() {
                data.node_url = trimmed;
            }
        }
    });

    ui.add_space(12.0);
    section_heading(ui, "Session");

    card(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 6.0;

        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Account").color(DIM_TEXT).small().size(12.0));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(egui::RichText::new(
                    data.account.as_deref().unwrap_or("Not signed in")
                ).size(12.0).strong());
            });
        });

        ui.separator();

        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Key role").color(DIM_TEXT).small().size(12.0));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(role) = &data.key_role {
                    let (color, label) = match role {
                        KeyRole::Active  => (GREEN,  "active"),
                        KeyRole::Posting => (BLUE,   "posting"),
                        KeyRole::Owner   => (YELLOW, "owner"),
                    };
                    ui.label(egui::RichText::new(label).size(12.0).strong().color(color));
                } else {
                    ui.label(egui::RichText::new("—").size(12.0).color(DIM_TEXT));
                }
            });
        });

        ui.separator();

        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("Key file").color(DIM_TEXT).small().size(12.0));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.label(egui::RichText::new(
                    data.key_file.as_ref()
                        .map(|p| p.file_name()
                            .and_then(|n| n.to_str())
                            .unwrap_or("—")
                            .to_string())
                        .unwrap_or_else(|| "—".to_string())
                ).size(11.0).monospace().color(DIM_TEXT));
            });
        });
    });

    ui.add_space(16.0);
    ui.label(egui::RichText::new(
        "To switch accounts, use File → Sign out."
    ).small().color(DIM_TEXT));

    ui.add_space(16.0);
    section_heading(ui, "Bridge");

    let mut eth_rpc = data.eth_rpc_url.clone();
    let mut contract = data.whone_contract.clone();

    card(ui, |ui| {
        egui::Grid::new("bridge_grid")
            .num_columns(2)
            .spacing([12.0, 8.0])
            .show(ui, |ui| {
                ui.label(egui::RichText::new("ETH RPC URL").color(DIM_TEXT).small());
                ui.add(egui::TextEdit::singleline(&mut eth_rpc)
                    .hint_text("https://eth.llamarpc.com")
                    .desired_width(280.0));
                ui.end_row();

                ui.label(egui::RichText::new("wHONE contract").color(DIM_TEXT).small());
                ui.add(egui::TextEdit::singleline(&mut contract)
                    .hint_text("0x…")
                    .desired_width(280.0));
                ui.end_row();
            });

        ui.add_space(6.0);
        if ui.add(
            egui::Button::new(egui::RichText::new("Apply").strong())
                .min_size(egui::vec2(80.0, 26.0))
        ).clicked() {
            data.eth_rpc_url = eth_rpc.trim().to_owned();
            data.whone_contract = contract.trim().to_owned();
        }
    });

    ui.add_space(4.0);
    ui.label(egui::RichText::new(
        "Enter an Ethereum JSON-RPC URL and the deployed wHONE ERC-20 contract address to display your wHONE balance."
    ).size(10.0).color(DIM_TEXT));
}
