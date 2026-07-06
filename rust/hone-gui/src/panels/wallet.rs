use super::{
    AppData, hunits_to_hone, parse_hone_input,
    not_signed_in, active_key_required, section_heading, card,
    GREEN, BLUE, YELLOW, DIM_TEXT, ORANGE,
};
use crate::app::KeyRole;

fn truncate_addr(s: &str, n: usize) -> String {
    if s.len() <= n * 2 + 2 { s.to_string() }
    else { format!("{}…{}", &s[..n + 2], &s[s.len() - n..]) }
}

pub fn show(ui: &mut egui::Ui, data: &mut AppData) {
    let account = match data.account.clone() {
        Some(a) => a,
        None => { not_signed_in(ui); return; }
    };

    let can_spend = data.key_role.as_ref().map(|r| *r == KeyRole::Active).unwrap_or(false);

    // ── Hero balance card ─────────────────────────────────────────────────────
    card(ui, |ui| {
        ui.vertical_centered(|ui| {
            let balance_str = data.balance
                .map(|b| hunits_to_hone(b))
                .unwrap_or_else(|| "—".to_string());

            ui.label(egui::RichText::new(&balance_str)
                .size(36.0).strong().color(egui::Color32::WHITE));
            ui.label(egui::RichText::new("HONE")
                .size(13.0).color(ORANGE).strong());

            ui.add_space(6.0);

            ui.horizontal(|ui| {
                ui.spacing_mut().item_spacing.x = 16.0;
                ui.vertical_centered(|ui| {
                    ui.label(egui::RichText::new("Staked").size(11.0).color(DIM_TEXT));
                    ui.label(egui::RichText::new(
                        data.staked.map(|s| format!("{} HONE", hunits_to_hone(s)))
                            .unwrap_or_else(|| "—".to_string())
                    ).size(13.0).color(BLUE));
                });
            });
        });

        ui.add_space(8.0);
        ui.separator();
        ui.add_space(4.0);

        // Account + role row
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new(&account).size(12.0).color(DIM_TEXT));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if let Some(role) = &data.key_role {
                    let (color, label) = match role {
                        KeyRole::Active  => (GREEN,  "● active"),
                        KeyRole::Posting => (BLUE,   "● posting"),
                        KeyRole::Owner   => (YELLOW, "● owner"),
                    };
                    ui.label(egui::RichText::new(label).size(11.0).color(color));
                }
            });
        });
    });

    ui.add_space(12.0);

    // ── Transfer form ─────────────────────────────────────────────────────────
    section_heading(ui, "Send Transfer");

    if !can_spend {
        active_key_required(ui);
    } else {
        card(ui, |ui| {
            egui::Grid::new("transfer_form")
                .num_columns(2)
                .spacing([8.0, 8.0])
                .show(ui, |ui| {
                    ui.label(egui::RichText::new("To").color(DIM_TEXT).small());
                    ui.add(egui::TextEdit::singleline(&mut data.forms.transfer_to)
                        .hint_text("recipient account")
                        .desired_width(220.0));
                    ui.end_row();

                    ui.label(egui::RichText::new("Amount").color(DIM_TEXT).small());
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut data.forms.transfer_amount)
                            .hint_text("0.0")
                            .desired_width(140.0));
                        ui.label(egui::RichText::new("HONE").color(ORANGE).size(12.0));
                    });
                    ui.end_row();
                });

            ui.add_space(6.0);

            let send_btn = egui::Button::new(
                egui::RichText::new("Send Transfer").size(13.0).strong()
            ).fill(ORANGE).min_size(egui::vec2(130.0, 28.0));

            if ui.add(send_btn).clicked() {
                let base = data.node_url.clone();
                let from = account.clone();
                let to   = data.forms.transfer_to.trim().to_owned();
                let key_file = data.key_file.clone();

                if to.is_empty() {
                    data.forms.transfer_result = Some((false, "recipient is required".into()));
                } else {
                    match (key_file, parse_hone_input(&data.forms.transfer_amount)) {
                        (_, None) => {
                            data.forms.transfer_result = Some((false, "invalid amount".into()));
                        }
                        (None, _) => {
                            data.forms.transfer_result = Some((false, "no key file in session".into()));
                        }
                        (Some(kf), Some(hunits)) => {
                            match crate::sign::submit_transfer(&base, &kf, &from, &to, hunits) {
                                Ok(msg) => data.forms.transfer_result = Some((true, msg)),
                                Err(e)  => data.forms.transfer_result = Some((false, e.to_string())),
                            }
                        }
                    }
                }
            }

            if let Some((ok, ref msg)) = data.forms.transfer_result {
                ui.add_space(4.0);
                let color = if ok { GREEN } else { egui::Color32::from_rgb(255, 90, 90) };
                ui.colored_label(color, msg);
            }
        });
    }

    // ── Cross-chain wallets ───────────────────────────────────────────────────
    ui.add_space(12.0);
    section_heading(ui, "Cross-chain Wallets");

    card(ui, |ui| {
        ui.spacing_mut().item_spacing.y = 6.0;

        // ETH / wHONE
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("ETH").size(11.0).color(DIM_TEXT).monospace().strong());
            ui.add_space(8.0);
            if let Some(ref addr) = data.eth_address {
                let display = truncate_addr(addr, 6);
                let resp = ui.add(
                    egui::Label::new(
                        egui::RichText::new(&display).size(11.0).monospace().color(BLUE)
                    ).sense(egui::Sense::click())
                );
                if resp.clicked() {
                    ui.ctx().copy_text(addr.clone());
                }
                if resp.hovered() {
                    resp.on_hover_text(addr.as_str());
                }
                ui.add_space(8.0);
                // wHONE balance
                let bal_str = match data.whone_balance {
                    Some(b) => format!("{} wHONE", hunits_to_hone(b)),
                    None if data.whone_contract.is_empty() => "configure contract →".into(),
                    None => "—".into(),
                };
                ui.label(egui::RichText::new(bal_str).size(11.0).color(ORANGE));
            } else {
                ui.label(egui::RichText::new("no wallet key file").size(11.0).color(DIM_TEXT));
            }
        });

        ui.separator();

        // BTC pubkey (abbreviated)
        ui.horizontal(|ui| {
            ui.label(egui::RichText::new("BTC").size(11.0).color(DIM_TEXT).monospace().strong());
            ui.add_space(8.0);
            if let Some(ref pk) = data.btc_pubkey {
                let display = truncate_addr(pk, 6);
                ui.label(egui::RichText::new(display).size(11.0).monospace().color(DIM_TEXT))
                  .on_hover_text(pk.as_str());
            } else {
                ui.label(egui::RichText::new("—").size(11.0).color(DIM_TEXT));
            }
        });

        ui.add_space(4.0);
        ui.label(egui::RichText::new(
            "Derived from your BIP39 mnemonic.  Configure wHONE contract in Settings → Bridge."
        ).size(10.0).color(DIM_TEXT));
    });
}
