use super::{
    AppData, dreams_to_btcpc, parse_btcpc_input,
    not_signed_in, active_key_required, section_heading, card,
    ORANGE, GREEN, RED, DIM_TEXT,
};
use crate::app::KeyRole;

fn status_color(status: &str) -> egui::Color32 {
    match status {
        "posted"   => egui::Color32::from_rgb(96, 165, 250),
        "assigned" => egui::Color32::from_rgb(251, 191, 36),
        "done"     => GREEN,
        "rejected" => RED,
        _          => DIM_TEXT,
    }
}

pub fn show(ui: &mut egui::Ui, data: &mut AppData) {
    let account = data.account.clone();
    let can_spend = data.key_role.as_ref().map(|r| *r == KeyRole::Active).unwrap_or(false);

    // ── Post New Job ──────────────────────────────────────────────────────────
    section_heading(ui, "Post Inference Job");

    match &account {
        None => { not_signed_in(ui); }
        Some(_) if !can_spend => { active_key_required(ui); }
        Some(acct) => {
            let acct = acct.clone();
            let key_file = data.key_file.clone();

            card(ui, |ui| {
                egui::Grid::new("job_post_form")
                    .num_columns(2)
                    .spacing([8.0, 8.0])
                    .show(ui, |ui| {
                        ui.label(egui::RichText::new("Model").color(DIM_TEXT).small());
                        ui.add(egui::TextEdit::singleline(&mut data.forms.job_model)
                            .hint_text("llama3, mistral, …")
                            .desired_width(220.0));
                        ui.end_row();

                        ui.label(egui::RichText::new("Prompt").color(DIM_TEXT).small());
                        ui.add(egui::TextEdit::multiline(&mut data.forms.job_input)
                            .desired_rows(4)
                            .desired_width(280.0)
                            .hint_text("Enter your prompt here…"));
                        ui.end_row();

                        ui.label(egui::RichText::new("Max fee").color(DIM_TEXT).small());
                        ui.horizontal(|ui| {
                            ui.add(egui::TextEdit::singleline(&mut data.forms.job_max_fee)
                                .hint_text("0.01")
                                .desired_width(100.0));
                            ui.label(egui::RichText::new("BTCPC").color(ORANGE).size(12.0));
                        });
                        ui.end_row();

                        ui.label(egui::RichText::new("Deadline epoch").color(DIM_TEXT).small());
                        ui.add(egui::TextEdit::singleline(&mut data.forms.job_deadline)
                            .hint_text("e.g. 500")
                            .desired_width(100.0));
                        ui.end_row();
                    });

                ui.add_space(6.0);

                let post_btn = egui::Button::new(
                    egui::RichText::new("Post Job").size(13.0).strong()
                ).fill(ORANGE).min_size(egui::vec2(110.0, 28.0));

                if ui.add(post_btn).clicked() {
                    let base = data.node_url.clone();
                    let model = data.forms.job_model.clone();
                    let input = data.forms.job_input.clone();
                    let max_fee = parse_btcpc_input(&data.forms.job_max_fee);
                    let deadline = data.forms.job_deadline.trim().parse::<u64>().ok();

                    match (key_file, max_fee, deadline) {
                        (None, _, _) => {
                            data.forms.job_result = Some((false, "no key file in session".into()));
                        }
                        (_, None, _) => {
                            data.forms.job_result = Some((false, "invalid max fee".into()));
                        }
                        (_, _, None) => {
                            data.forms.job_result = Some((false, "invalid deadline epoch".into()));
                        }
                        (Some(kf), Some(fee), Some(dl)) => {
                            match crate::sign::submit_post_job(&base, &kf, &acct, &model, &input, fee, dl) {
                                Ok(msg) => data.forms.job_result = Some((true, msg)),
                                Err(e)  => data.forms.job_result = Some((false, e.to_string())),
                            }
                        }
                    }
                }

                if let Some((ok, ref msg)) = data.forms.job_result {
                    ui.add_space(4.0);
                    let color = if ok { GREEN } else { egui::Color32::from_rgb(255, 90, 90) };
                    ui.colored_label(color, msg);
                }
            });
        }
    }

    ui.add_space(12.0);

    // ── Jobs table ────────────────────────────────────────────────────────────
    section_heading(ui, "Open Jobs");

    if data.jobs.is_empty() {
        card(ui, |ui| {
            ui.vertical_centered(|ui| {
                ui.add_space(6.0);
                ui.label(egui::RichText::new("No open inference jobs").color(DIM_TEXT).size(12.0));
                ui.add_space(6.0);
            });
        });
        return;
    }

    // Header
    egui::Frame::none()
        .fill(egui::Color32::from_rgb(20, 20, 30))
        .inner_margin(egui::Margin::symmetric(8_i8, 4_i8))
        .show(ui, |ui| {
            ui.columns(5, |cols| {
                for (c, h) in [0, 1, 2, 3, 4].iter().zip(["ID", "Model", "Status", "Max Fee", "Deadline"]) {
                    cols[*c].label(egui::RichText::new(h).size(11.0).color(ORANGE).strong());
                }
            });
        });

    egui::ScrollArea::vertical()
        .max_height(300.0)
        .show(ui, |ui| {
            for (i, job) in data.jobs.iter().enumerate() {
                let bg = if i % 2 == 0 {
                    egui::Color32::from_rgb(22, 22, 32)
                } else {
                    egui::Color32::from_rgb(26, 26, 38)
                };

                egui::Frame::none()
                    .fill(bg)
                    .inner_margin(egui::Margin::symmetric(8_i8, 5_i8))
                    .show(ui, |ui| {
                        let id = job.get("id").and_then(|v| v.as_str()).unwrap_or("—");
                        let id_short = if id.len() > 10 { &id[..10] } else { id };
                        let model  = job.get("model").and_then(|v| v.as_str()).unwrap_or("—");
                        let status = job.get("status").and_then(|v| v.as_str()).unwrap_or("—");
                        let fee    = job.get("max_fee").and_then(|v| v.as_u64())
                            .map(|f| format!("{:.4}", f as f64 / 100_000_000.0))
                            .unwrap_or_else(|| "—".to_string());
                        let epoch  = job.get("deadline_epoch").or_else(|| job.get("epoch"))
                            .and_then(|v| v.as_u64())
                            .map(|e| e.to_string())
                            .unwrap_or_else(|| "—".to_string());

                        ui.columns(5, |cols| {
                            cols[0].label(egui::RichText::new(id_short).size(11.0).monospace()
                                .color(egui::Color32::from_rgb(140, 180, 255)));
                            cols[1].label(egui::RichText::new(model).size(12.0));
                            cols[2].label(egui::RichText::new(status).size(12.0)
                                .color(status_color(status)));
                            cols[3].label(egui::RichText::new(&fee).size(12.0));
                            cols[4].label(egui::RichText::new(&epoch).size(12.0).color(DIM_TEXT));
                        });
                    });
            }
        });
}
