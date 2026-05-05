use super::AppData;

fn dreams_to_btcpc(dreams: u64) -> String {
    format!("{:.8}", dreams as f64 / 100_000_000.0)
}

fn parse_btcpc_input(s: &str) -> Option<u64> {
    s.trim().parse::<f64>().ok().map(|a| (a * 10_000_000_000.0) as u64)
}

pub fn show(ui: &mut egui::Ui, data: &mut AppData) {
    ui.heading("Inference Jobs");
    ui.separator();

    // ── Post New Job ──────────────────────────────────────────────────────────
    egui::CollapsingHeader::new("Post New Job")
        .default_open(true)
        .show(ui, |ui| {
            let account = data.account.clone();
            let key_file = data.key_file.clone();

            if account.is_none() {
                ui.label("Not logged in. Run: btcpc login --account <name>");
            } else {
                egui::Grid::new("job_post_form")
                    .num_columns(2)
                    .spacing([8.0, 4.0])
                    .show(ui, |ui| {
                        ui.label("Model:");
                        ui.text_edit_singleline(&mut data.forms.job_model);
                        ui.end_row();

                        ui.label("Input text:");
                        ui.text_edit_multiline(&mut data.forms.job_input);
                        ui.end_row();

                        ui.label("Max Fee (BTCPC):");
                        ui.text_edit_singleline(&mut data.forms.job_max_fee);
                        ui.end_row();

                        ui.label("Deadline Epoch:");
                        ui.text_edit_singleline(&mut data.forms.job_deadline);
                        ui.end_row();
                    });

                if ui.button("Post Job").clicked() {
                    let base = data.node_url.clone();
                    let acct = account.unwrap();
                    let model = data.forms.job_model.clone();
                    let input = data.forms.job_input.clone();
                    let max_fee_str = data.forms.job_max_fee.clone();
                    let deadline_str = data.forms.job_deadline.clone();

                    let max_fee = parse_btcpc_input(&max_fee_str);
                    let deadline = deadline_str.trim().parse::<u64>().ok();

                    match (key_file, max_fee, deadline) {
                        (None, _, _) => {
                            data.forms.job_result = Some((false, "no key file in session".to_string()));
                        }
                        (_, None, _) => {
                            data.forms.job_result = Some((false, "invalid max fee".to_string()));
                        }
                        (_, _, None) => {
                            data.forms.job_result = Some((false, "invalid deadline epoch".to_string()));
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
                    let color = if ok { egui::Color32::GREEN } else { egui::Color32::RED };
                    ui.colored_label(color, msg);
                }
            }
        });

    ui.separator();

    // ── Open Jobs table ───────────────────────────────────────────────────────
    ui.strong("Open Jobs");
    egui::ScrollArea::vertical().show(ui, |ui| {
        egui::Grid::new("jobs_grid")
            .striped(true)
            .num_columns(5)
            .spacing([16.0, 4.0])
            .show(ui, |ui| {
                ui.strong("ID");
                ui.strong("Model");
                ui.strong("Status");
                ui.strong("Fee (BTCPC)");
                ui.strong("Epoch");
                ui.end_row();

                for job in &data.jobs {
                    let id = job.get("id").and_then(|v| v.as_str()).unwrap_or("—");
                    let id_short = if id.len() > 12 { &id[..12] } else { id };
                    ui.monospace(id_short);

                    ui.label(job.get("model").and_then(|v| v.as_str()).unwrap_or("—"));
                    ui.label(job.get("status").and_then(|v| v.as_str()).unwrap_or("—"));
                    ui.label(
                        job.get("max_fee")
                            .and_then(|v| v.as_u64())
                            .map(dreams_to_btcpc)
                            .unwrap_or_else(|| "—".to_string()),
                    );
                    ui.label(
                        job.get("deadline_epoch")
                            .or_else(|| job.get("epoch"))
                            .and_then(|v| v.as_u64())
                            .map(|e| e.to_string())
                            .unwrap_or_else(|| "—".to_string()),
                    );
                    ui.end_row();
                }
            });

        if data.jobs.is_empty() {
            ui.label("No posted jobs.");
        }
    });
}
