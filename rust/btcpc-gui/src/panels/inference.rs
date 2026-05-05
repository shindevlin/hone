use super::AppData;

fn dreams_to_btcpc(dreams: u64) -> String {
    format!("{:.8}", dreams as f64 / 100_000_000.0)
}

pub fn show(ui: &mut egui::Ui, data: &AppData) {
    ui.heading("Inference Jobs");
    ui.separator();

    egui::ScrollArea::vertical().show(ui, |ui| {
        egui::Grid::new("jobs_grid")
            .striped(true)
            .num_columns(4)
            .spacing([16.0, 4.0])
            .show(ui, |ui| {
                ui.strong("ID");
                ui.strong("Model");
                ui.strong("Status");
                ui.strong("Fee (BTCPC)");
                ui.end_row();

                for job in &data.jobs {
                    let id = job
                        .get("id")
                        .and_then(|v| v.as_str())
                        .unwrap_or("—");
                    let id_short = if id.len() > 12 { &id[..12] } else { id };
                    ui.monospace(id_short);

                    ui.label(
                        job.get("model")
                            .and_then(|v| v.as_str())
                            .unwrap_or("—"),
                    );

                    ui.label(
                        job.get("status")
                            .and_then(|v| v.as_str())
                            .unwrap_or("—"),
                    );

                    ui.label(
                        job.get("max_fee")
                            .and_then(|v| v.as_u64())
                            .map(dreams_to_btcpc)
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
