use super::{AppData, section_heading, card, ORANGE, GREEN, BLUE, YELLOW, RED, DIM_TEXT};

/// Agent Spending Key panel.
///
/// Click-driven, no CLI: creates a standalone HONE account for an AI agent.
/// The user funds it by sending HONE to the account; the agent holds only this
/// account's key and can spend nothing beyond its balance. The vault is never
/// touched.
pub fn show(ui: &mut egui::Ui, data: &mut AppData) {
    ui.add_space(4.0);
    section_heading(ui, "Agent Spending Key");

    card(ui, |ui| {
        ui.label(
            egui::RichText::new(
                "Create a separate account for an AI agent. You fund it by sending HONE to it, \
                 and the agent can only ever spend that balance. Your vault keys are never \
                 involved — top it up with only what you're willing to let the agent spend.",
            )
            .size(12.0)
            .color(DIM_TEXT),
        );
        ui.add_space(10.0);

        egui::Grid::new("agent_form")
            .num_columns(2)
            .spacing([8.0, 8.0])
            .show(ui, |ui| {
                ui.label(egui::RichText::new("Agent name").color(DIM_TEXT).small());
                ui.add(
                    egui::TextEdit::singleline(&mut data.forms.agent_name)
                        .hint_text("e.g. myagent")
                        .desired_width(220.0),
                );
                ui.end_row();
            });

        ui.add_space(8.0);

        let btn = egui::Button::new(
            egui::RichText::new("Create agent spending key").size(13.0).strong(),
        )
        .fill(ORANGE)
        .min_size(egui::vec2(220.0, 30.0));

        if ui.add(btn).clicked() {
            let name = data.forms.agent_name.trim().to_string();
            if name.is_empty() {
                data.forms.agent_result = Some((false, "enter an agent name".into()));
            } else {
                match crate::sign::create_agent_key(&name) {
                    Ok((pubkey, path)) => {
                        data.forms.agent_pubkey = Some(pubkey);
                        data.forms.agent_keyfile = Some(path);
                        data.forms.agent_result =
                            Some((true, format!("Created agent account '{name}'")));
                    }
                    Err(e) => {
                        data.forms.agent_pubkey = None;
                        data.forms.agent_keyfile = None;
                        data.forms.agent_result = Some((false, e.to_string()));
                    }
                }
            }
        }

        if let Some((ok, ref msg)) = data.forms.agent_result {
            ui.add_space(6.0);
            ui.colored_label(if ok { GREEN } else { RED }, msg);
        }
    });

    // Result: how to fund the newly created agent.
    if let (Some(pubkey), Some(path)) =
        (data.forms.agent_pubkey.clone(), data.forms.agent_keyfile.clone())
    {
        let name = data.forms.agent_name.trim().to_string();
        ui.add_space(12.0);
        section_heading(ui, "Fund this agent");
        card(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Account").size(11.0).color(DIM_TEXT));
                ui.add_space(6.0);
                let resp = ui.add(
                    egui::Label::new(
                        egui::RichText::new(&name).size(13.0).monospace().color(ORANGE).strong(),
                    )
                    .sense(egui::Sense::click()),
                );
                if resp.clicked() {
                    ui.ctx().copy_text(name.clone());
                }
                resp.on_hover_text("click to copy — send HONE here to fund the agent");
            });

            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(egui::RichText::new("Public key").size(11.0).color(DIM_TEXT));
                ui.add_space(6.0);
                let short = if pubkey.len() > 20 {
                    format!("{}…{}", &pubkey[..10], &pubkey[pubkey.len() - 8..])
                } else {
                    pubkey.clone()
                };
                let resp = ui.add(
                    egui::Label::new(
                        egui::RichText::new(short).size(11.0).monospace().color(BLUE),
                    )
                    .sense(egui::Sense::click()),
                );
                if resp.clicked() {
                    ui.ctx().copy_text(pubkey.clone());
                }
                resp.on_hover_text(pubkey.as_str());
            });

            ui.add_space(8.0);
            ui.label(
                egui::RichText::new(format!("Agent key file saved to:\n{path}"))
                    .size(10.0)
                    .color(DIM_TEXT),
            );
            ui.add_space(6.0);
            ui.label(
                egui::RichText::new(
                    "Send HONE to this account to fund it. Point your agent at the saved key \
                     file — it can spend up to the balance and no more. Keep that file only on \
                     the agent's machine; back it up like any hot wallet.",
                )
                .size(11.0)
                .color(YELLOW),
            );
        });
    }
}
