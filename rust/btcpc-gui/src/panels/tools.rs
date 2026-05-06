use super::{
    AppData, dreams_to_btcpc, parse_btcpc_input,
    not_signed_in, active_key_required, section_heading, card,
    ORANGE, GREEN, RED, BLUE, YELLOW, DIM_TEXT,
};
use crate::app::KeyRole;

const SECTIONS: &[(&str, &str)] = &[
    ("AI",       "Inference jobs on the worker network"),
    ("Storage",  "Distributed blob storage"),
    ("Sensors",  "Live sensor data feeds"),
    ("Freeport", "Open trade marketplace"),
    ("LinkGit",  "Git repository registry"),
];

fn nav_strip(ui: &mut egui::Ui, selected: &mut usize) {
    ui.horizontal(|ui| {
        ui.spacing_mut().item_spacing.x = 2.0;
        for (i, (label, _)) in SECTIONS.iter().enumerate() {
            let active = *selected == i;
            let text = egui::RichText::new(*label)
                .size(12.0)
                .strong();
            let btn = egui::Button::new(text)
                .fill(if active { ORANGE } else { egui::Color32::from_rgb(30, 30, 44) })
                .stroke(egui::Stroke::new(
                    1.0,
                    if active { ORANGE } else { egui::Color32::from_rgb(50, 50, 70) },
                ))
                .corner_radius(egui::CornerRadius::same(4));
            if ui.add(btn).clicked() {
                *selected = i;
            }
        }
    });
    ui.add_space(10.0);
}

// ── AI (inference jobs) ───────────────────────────────────────────────────────

fn status_color(status: &str) -> egui::Color32 {
    match status {
        "posted"   => BLUE,
        "assigned" => YELLOW,
        "done"     => GREEN,
        "rejected" => RED,
        _          => DIM_TEXT,
    }
}

fn show_ai(ui: &mut egui::Ui, data: &mut AppData) {
    let account  = data.account.clone();
    let can_spend = data.key_role.as_ref().map(|r| *r == KeyRole::Active).unwrap_or(false);

    section_heading(ui, "Post Inference Job");

    match &account {
        None => { not_signed_in(ui); }
        Some(_) if !can_spend => { active_key_required(ui); }
        Some(acct) => {
            let acct     = acct.clone();
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
                            .hint_text("Enter your prompt…"));
                        ui.end_row();

                        ui.label(egui::RichText::new("Max fee").color(DIM_TEXT).small());
                        ui.horizontal(|ui| {
                            ui.add(egui::TextEdit::singleline(&mut data.forms.job_max_fee)
                                .hint_text("0.01").desired_width(100.0));
                            ui.label(egui::RichText::new("BTCPC").color(ORANGE).size(12.0));
                        });
                        ui.end_row();

                        ui.label(egui::RichText::new("Deadline epoch").color(DIM_TEXT).small());
                        ui.add(egui::TextEdit::singleline(&mut data.forms.job_deadline)
                            .hint_text("e.g. 500").desired_width(100.0));
                        ui.end_row();
                    });

                ui.add_space(6.0);
                let post_btn = egui::Button::new(
                    egui::RichText::new("Post Job").size(13.0).strong()
                ).fill(ORANGE).min_size(egui::vec2(110.0, 30.0));

                if ui.add(post_btn).clicked() {
                    let base     = data.node_url.clone();
                    let model    = data.forms.job_model.clone();
                    let input    = data.forms.job_input.clone();
                    let max_fee  = parse_btcpc_input(&data.forms.job_max_fee);
                    let deadline = data.forms.job_deadline.trim().parse::<u64>().ok();

                    data.forms.job_result = Some(match (key_file, max_fee, deadline) {
                        (None, _, _) => (false, "no key file in session".into()),
                        (_, None, _) => (false, "invalid max fee".into()),
                        (_, _, None) => (false, "invalid deadline epoch".into()),
                        (Some(kf), Some(fee), Some(dl)) => {
                            match crate::sign::submit_post_job(&base, &kf, &acct, &model, &input, fee, dl) {
                                Ok(msg) => (true, msg),
                                Err(e)  => (false, e.to_string()),
                            }
                        }
                    });
                }

                if let Some((ok, ref msg)) = data.forms.job_result {
                    ui.add_space(4.0);
                    ui.colored_label(if ok { GREEN } else { egui::Color32::from_rgb(255, 90, 90) }, msg);
                }
            });
        }
    }

    ui.add_space(12.0);
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

    egui::Frame::none()
        .fill(egui::Color32::from_rgb(20, 20, 30))
        .inner_margin(egui::Margin::symmetric(8_i8, 4_i8))
        .show(ui, |ui| {
            ui.columns(5, |cols| {
                for (c, h) in [0,1,2,3,4].iter().zip(["ID","Model","Status","Fee","Deadline"]) {
                    cols[*c].label(egui::RichText::new(h).size(11.0).color(ORANGE).strong());
                }
            });
        });

    egui::ScrollArea::vertical().max_height(240.0).show(ui, |ui| {
        for (i, job) in data.jobs.iter().enumerate() {
            let bg = if i % 2 == 0 { egui::Color32::from_rgb(22,22,32) } else { egui::Color32::from_rgb(26,26,38) };
            egui::Frame::none().fill(bg).inner_margin(egui::Margin::symmetric(8_i8, 5_i8)).show(ui, |ui| {
                let id     = job.get("id").and_then(|v| v.as_str()).unwrap_or("—");
                let model  = job.get("model").and_then(|v| v.as_str()).unwrap_or("—");
                let status = job.get("status").and_then(|v| v.as_str()).unwrap_or("—");
                let fee    = job.get("max_fee").and_then(|v| v.as_u64())
                    .map(|f| dreams_to_btcpc(f)).unwrap_or_else(|| "—".to_string());
                let epoch  = job.get("deadline_epoch").or_else(|| job.get("epoch"))
                    .and_then(|v| v.as_u64()).map(|e| e.to_string()).unwrap_or_else(|| "—".to_string());

                ui.columns(5, |cols| {
                    cols[0].label(egui::RichText::new(&id[..id.len().min(10)]).size(11.0).monospace().color(BLUE));
                    cols[1].label(egui::RichText::new(model).size(12.0));
                    cols[2].label(egui::RichText::new(status).size(12.0).color(status_color(status)));
                    cols[3].label(egui::RichText::new(&fee).size(12.0));
                    cols[4].label(egui::RichText::new(&epoch).size(12.0).color(DIM_TEXT));
                });
            });
        }
    });
}

// ── Storage ───────────────────────────────────────────────────────────────────

fn show_storage(ui: &mut egui::Ui, data: &AppData) {
    section_heading(ui, "Distributed Storage");
    card(ui, |ui| {
        ui.horizontal(|ui| {
            let dot_color = if data.node_info.as_ref()
                .and_then(|v| v.get("is_storage")).and_then(|v| v.as_bool()).unwrap_or(false)
            { GREEN } else { DIM_TEXT };
            let (r_rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
            ui.painter().circle_filled(r_rect.center(), 5.0, dot_color);
            ui.add_space(4.0);
            ui.label(egui::RichText::new("Storage node").size(12.0).color(egui::Color32::WHITE).strong());
        });
        ui.add_space(4.0);
        ui.label(egui::RichText::new(
            "This node can store and serve blobs for the network. Blob uploads and retrievals \
             are posted as StoragePin / StorageRelease entries on-chain and priced in BTCPC per MB."
        ).size(11.0).color(DIM_TEXT));
        ui.add_space(6.0);
        ui.label(egui::RichText::new("Upload / listing UI — coming soon").size(11.0).color(YELLOW));
    });
}

// ── Sensors ───────────────────────────────────────────────────────────────────

fn show_sensors(ui: &mut egui::Ui, data: &AppData) {
    section_heading(ui, "Sensor Network");
    card(ui, |ui| {
        let is_sensor = data.node_info.as_ref()
            .and_then(|v| v.get("is_sensor")).and_then(|v| v.as_bool()).unwrap_or(false);
        ui.horizontal(|ui| {
            let (r_rect, _) = ui.allocate_exact_size(egui::vec2(10.0, 10.0), egui::Sense::hover());
            ui.painter().circle_filled(r_rect.center(), 5.0, if is_sensor { GREEN } else { DIM_TEXT });
            ui.add_space(4.0);
            ui.label(egui::RichText::new(
                if is_sensor { "Sensor node — submitting data" } else { "Sensor node — not running" }
            ).size(12.0).strong().color(egui::Color32::WHITE));
        });
        ui.add_space(4.0);
        ui.label(egui::RichText::new(
            "Sensor nodes submit signed physical-world data (GPS, temperature, RF, etc.) \
             as SensorReport entries. Verifiers earn BTCPC for cross-validating reports."
        ).size(11.0).color(DIM_TEXT));
        ui.add_space(6.0);
        ui.label(egui::RichText::new("Live feed browser — coming soon").size(11.0).color(YELLOW));
    });
}

// ── Freeport ──────────────────────────────────────────────────────────────────

fn show_freeport(ui: &mut egui::Ui, _data: &AppData) {
    section_heading(ui, "Freeport Marketplace");
    card(ui, |ui| {
        ui.label(egui::RichText::new("Open Trade Marketplace").size(13.0).strong().color(ORANGE));
        ui.add_space(4.0);
        ui.label(egui::RichText::new(
            "Freeport is the on-chain peer-to-peer marketplace. List and purchase goods, \
             services, and data subscriptions priced in native BTCPC. All trades settle \
             through the chain with escrow enforced by the mempool role."
        ).size(11.0).color(DIM_TEXT));
        ui.add_space(6.0);
        ui.label(egui::RichText::new("Listing and browse UI — coming soon").size(11.0).color(YELLOW));
    });
}

// ── LinkGit ───────────────────────────────────────────────────────────────────

fn show_linkgit(ui: &mut egui::Ui, _data: &AppData) {
    section_heading(ui, "LinkGit — Repository Registry");
    card(ui, |ui| {
        ui.label(egui::RichText::new("Git Identity on Chain").size(13.0).strong().color(BLUE));
        ui.add_space(4.0);
        ui.label(egui::RichText::new(
            "LinkGit registers git repository identities on the BTCPC chain. \
             Pin commit hashes, prove authorship, and receive BTCPC tips for open-source \
             contributions — all without leaving your normal git workflow."
        ).size(11.0).color(DIM_TEXT));
        ui.add_space(6.0);
        ui.label(egui::RichText::new("Repository registry UI — coming soon").size(11.0).color(YELLOW));
    });
}

// ── Entry point ───────────────────────────────────────────────────────────────

pub fn show(ui: &mut egui::Ui, data: &mut AppData) {
    let section = data.forms.tools_section;
    nav_strip(ui, &mut data.forms.tools_section);

    match section {
        0 => show_ai(ui, data),
        1 => show_storage(ui, data),
        2 => show_sensors(ui, data),
        3 => show_freeport(ui, data),
        _ => show_linkgit(ui, data),
    }
}
