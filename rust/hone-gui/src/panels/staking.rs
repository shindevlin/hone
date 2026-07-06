use super::{
    AppData, hunits_to_hone, parse_hone_input,
    not_signed_in, active_key_required, section_heading, card,
    ORANGE, BLUE, GREEN, RED, DIM_TEXT, YELLOW,
};
use crate::app::KeyRole;

const ROLES: &[&str] = &["clock", "miner", "storage", "sensor", "service", "verifier", "mempool"];

pub fn show(ui: &mut egui::Ui, data: &mut AppData) {
    let account = match data.account.clone() {
        Some(a) => a,
        None => { not_signed_in(ui); return; }
    };

    let can_spend = data.key_role.as_ref().map(|r| *r == KeyRole::Active).unwrap_or(false);

    // ── Summary card ──────────────────────────────────────────────────────────
    card(ui, |ui| {
        let balance     = data.balance.unwrap_or(0);
        let staked      = data.staked.unwrap_or(0);
        let role_backed: u64 = data.role_positions.iter()
            .filter_map(|p| p.get("amount").and_then(|a| a.as_u64()))
            .sum();
        let total = balance + staked + role_backed;

        if total > 0 {
            let r_staked = staked as f32 / total as f32;
            let r_backed = role_backed as f32 / total as f32;
            let avail_w  = ui.available_width();
            let bar_h    = 6.0_f32;
            let (bar_rect, _) = ui.allocate_exact_size(egui::vec2(avail_w, bar_h), egui::Sense::hover());
            ui.painter().rect_filled(bar_rect, egui::CornerRadius::same(3), egui::Color32::from_rgb(35, 35, 50));
            if r_staked > 0.0 {
                let r = egui::Rect::from_min_size(bar_rect.min, egui::vec2(bar_rect.width() * r_staked, bar_h));
                ui.painter().rect_filled(r, egui::CornerRadius::same(3), BLUE);
            }
            if r_backed > 0.0 {
                let x = bar_rect.width() * r_staked;
                let r = egui::Rect::from_min_size(
                    bar_rect.min + egui::vec2(x, 0.0),
                    egui::vec2(bar_rect.width() * r_backed, bar_h),
                );
                ui.painter().rect_filled(r, egui::CornerRadius::same(3), ORANGE);
            }
            ui.add_space(8.0);
        }

        ui.columns(3, |cols| {
            cols[0].vertical_centered(|ui| {
                ui.label(egui::RichText::new(
                    data.balance.map(|b| format!("{} HONE", hunits_to_hone(b)))
                        .unwrap_or_else(|| "—".to_string())
                ).size(14.0).strong().color(egui::Color32::WHITE));
                ui.label(egui::RichText::new("Available").size(11.0).color(DIM_TEXT));
            });
            cols[1].vertical_centered(|ui| {
                ui.label(egui::RichText::new(
                    data.staked.map(|s| format!("{} HONE", hunits_to_hone(s)))
                        .unwrap_or_else(|| "—".to_string())
                ).size(14.0).strong().color(BLUE));
                ui.label(egui::RichText::new("Staked").size(11.0).color(DIM_TEXT));
            });
            cols[2].vertical_centered(|ui| {
                let role_backed: u64 = data.role_positions.iter()
                    .filter_map(|p| p.get("amount").and_then(|a| a.as_u64()))
                    .sum();
                ui.label(egui::RichText::new(
                    format!("{} HONE", hunits_to_hone(role_backed))
                ).size(14.0).strong().color(ORANGE));
                ui.label(egui::RichText::new("Backing").size(11.0).color(DIM_TEXT));
            });
        });

        ui.add_space(6.0);
        ui.separator();
        ui.add_space(2.0);
        ui.label(egui::RichText::new(&account).size(11.0).color(DIM_TEXT));
    });

    ui.add_space(12.0);

    // ── Staking Requirements ──────────────────────────────────────────────────
    if let Some(req) = data.role_requirements.clone() {
        section_heading(ui, "Staking Requirements");
        card(ui, |ui| {
            if let Some(map) = req.get("requirements").and_then(|r| r.as_object()) {
                egui::Grid::new("req_grid")
                    .num_columns(2)
                    .spacing([20.0, 4.0])
                    .show(ui, |ui| {
                        for (role, min_val) in map {
                            let min_hunits = min_val.as_u64().unwrap_or(0);
                            ui.label(egui::RichText::new(role).color(ORANGE).small().strong());
                            ui.label(egui::RichText::new(
                                format!("{} HONE min", hunits_to_hone(min_hunits))
                            ).small().color(DIM_TEXT));
                            ui.end_row();
                        }
                    });
            }
            let slope    = req.get("loyalty_slope_bps").and_then(|v| v.as_u64()).unwrap_or(0);
            let cap_bps  = req.get("stake_increase_cap_bps").and_then(|v| v.as_u64()).unwrap_or(0);
            if slope > 0 || cap_bps > 0 {
                ui.add_space(6.0);
                ui.separator();
                ui.add_space(4.0);
            }
            if slope > 0 {
                ui.label(egui::RichText::new(
                    format!("Loyalty: existing operators cover {}% of each increase (others pay 100%)", slope / 100)
                ).small().color(DIM_TEXT));
            }
            if cap_bps > 0 {
                ui.label(egui::RichText::new(
                    format!("Governance cap: minimum may rise at most {}% per action ({}% of a doubling)", cap_bps / 100, cap_bps / 100)
                ).small().color(DIM_TEXT));
            }
        });
        ui.add_space(12.0);
    }

    if !can_spend {
        active_key_required(ui);
    } else {
        // ── Back a Node ───────────────────────────────────────────────────────
        section_heading(ui, "Back a Node");
        card(ui, |ui| {
            egui::Grid::new("role_stake_form")
                .num_columns(2)
                .spacing([8.0, 6.0])
                .show(ui, |ui| {
                    ui.label(egui::RichText::new("Node").color(DIM_TEXT).small());
                    ui.vertical(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut data.forms.role_stake_node)
                            .hint_text("account name")
                            .desired_width(200.0));
                        ui.label(egui::RichText::new("Type your own account name (or 'self') to stake your node")
                            .size(10.0).color(DIM_TEXT));
                    });
                    ui.end_row();

                    ui.label(egui::RichText::new("Role").color(DIM_TEXT).small());
                    let cur_role = data.forms.role_stake_role.clone();
                    egui::ComboBox::from_id_salt("role_stake_combo")
                        .selected_text(if cur_role.is_empty() { "select role" } else { &cur_role })
                        .width(200.0)
                        .show_ui(ui, |ui| {
                            for role in ROLES {
                                let selected = data.forms.role_stake_role == *role;
                                if ui.selectable_label(selected, *role).clicked() {
                                    data.forms.role_stake_role = role.to_string();
                                    data.forms.role_stake_lookup = None;
                                }
                            }
                        });
                    ui.end_row();

                    ui.label(egui::RichText::new("Amount").color(DIM_TEXT).small());
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut data.forms.role_stake_amount)
                            .hint_text("0.0")
                            .desired_width(140.0));
                        ui.label(egui::RichText::new("HONE").color(ORANGE).size(12.0));
                        // Show minimum for selected role
                        if !data.forms.role_stake_role.is_empty() {
                            if let Some(ref req) = data.role_requirements {
                                let min_d = req.get("requirements")
                                    .and_then(|r| r.get(&data.forms.role_stake_role))
                                    .and_then(|v| v.as_u64())
                                    .unwrap_or(0);
                                if min_d > 0 {
                                    ui.label(egui::RichText::new(
                                        format!("(min {})", hunits_to_hone(min_d))
                                    ).size(11.0).color(DIM_TEXT));
                                }
                            }
                        }
                    });
                    ui.end_row();
                });

            ui.add_space(4.0);
            ui.horizontal(|ui| {
                if ui.small_button("Check node").clicked()
                    && !data.forms.role_stake_node.is_empty()
                    && !data.forms.role_stake_role.is_empty()
                {
                    let url = format!(
                        "/api/staking/node/{}/{}",
                        data.forms.role_stake_node, data.forms.role_stake_role
                    );
                    data.forms.role_stake_lookup = Some(match crate::api::get_json(&data.node_url, &url) {
                        Ok(v) => v,
                        Err(e) => serde_json::json!({ "error": e.to_string() }),
                    });
                }

                if let Some(ref info) = data.forms.role_stake_lookup {
                    if let Some(err) = info.get("error").and_then(|e| e.as_str()) {
                        ui.label(egui::RichText::new(err).small().color(RED));
                    } else {
                        let opted_in  = info.get("opted_in").and_then(|v| v.as_bool()).unwrap_or(false);
                        let share_bps = info.get("backer_share_bps").and_then(|v| v.as_u64()).unwrap_or(0);
                        let cur_b     = info.get("current_backers").and_then(|v| v.as_u64()).unwrap_or(0);
                        let max_b     = info.get("max_backers").and_then(|v| v.as_u64()).unwrap_or(0);
                        if opted_in {
                            ui.label(egui::RichText::new(
                                format!("Opted in — {}% share, {}/{} backers", share_bps / 100, cur_b, max_b)
                            ).small().color(GREEN));
                        } else {
                            ui.label(egui::RichText::new("Not accepting backers for this role").small().color(YELLOW));
                        }
                    }
                }
            });

            ui.add_space(6.0);
            if ui.add(
                egui::Button::new(egui::RichText::new("Stake").strong())
                    .fill(egui::Color32::from_rgb(37, 99, 235))
                    .min_size(egui::vec2(90.0, 28.0))
            ).clicked() {
                let base     = data.node_url.clone();
                let key_file = data.key_file.clone();
                let raw_node = data.forms.role_stake_node.trim().to_string();
                let node     = if raw_node == "self" { account.clone() } else { raw_node };
                let role     = data.forms.role_stake_role.clone();
                data.forms.role_stake_result = Some(
                    if node.is_empty() {
                        (false, "enter a node account".into())
                    } else if role.is_empty() {
                        (false, "select a role".into())
                    } else {
                        match (key_file, parse_hone_input(&data.forms.role_stake_amount)) {
                            (None, _)          => (false, "no key file".into()),
                            (_, None)          => (false, "invalid amount".into()),
                            (Some(kf), Some(d)) => {
                                match crate::sign::submit_role_stake(&base, &kf, &account, &node, &role, d, true) {
                                    Ok(msg) => (true, msg),
                                    Err(e)  => (false, e.to_string()),
                                }
                            }
                        }
                    }
                );
            }

            if let Some((ok, ref msg)) = data.forms.role_stake_result {
                ui.add_space(4.0);
                ui.colored_label(if ok { GREEN } else { egui::Color32::from_rgb(255, 90, 90) }, msg);
            }
        });

        ui.add_space(12.0);

        // ── My Backing Positions ──────────────────────────────────────────────
        if !data.role_positions.is_empty() {
            section_heading(ui, "My Positions");

            let positions = data.role_positions.clone();
            let mut unstake_action: Option<(String, String, u64)> = None;

            for (idx, pos) in positions.iter().enumerate() {
                let node_name = pos.get("node").and_then(|v| v.as_str()).unwrap_or("?").to_string();
                let role_name = pos.get("role").and_then(|v| v.as_str()).unwrap_or("?").to_string();
                let amount    = pos.get("amount").and_then(|v| v.as_u64()).unwrap_or(0);
                let is_selected = data.forms.role_unstake_idx == Some(idx);

                card(ui, |ui| {
                    ui.horizontal(|ui| {
                        ui.vertical(|ui| {
                            ui.label(egui::RichText::new(&node_name).strong().color(egui::Color32::WHITE));
                            ui.label(egui::RichText::new(
                                format!("{} — {} HONE", role_name, hunits_to_hone(amount))
                            ).small().color(DIM_TEXT));
                        });

                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if is_selected {
                                if ui.add(
                                    egui::Button::new(egui::RichText::new("Unstake").strong())
                                        .fill(egui::Color32::from_rgb(180, 40, 40))
                                        .min_size(egui::vec2(70.0, 24.0))
                                ).clicked() {
                                    match parse_hone_input(&data.forms.role_unstake_amount) {
                                        Some(d) => unstake_action = Some((node_name.clone(), role_name.clone(), d)),
                                        None    => data.forms.role_unstake_result = Some((false, "invalid amount".into())),
                                    }
                                }
                                ui.add_space(4.0);
                                ui.add(egui::TextEdit::singleline(&mut data.forms.role_unstake_amount)
                                    .hint_text("0.0")
                                    .desired_width(80.0));
                                ui.label(egui::RichText::new("HONE").color(ORANGE).size(11.0));
                            } else {
                                if ui.add(
                                    egui::Button::new("Unstake")
                                        .fill(egui::Color32::from_rgb(55, 55, 75))
                                        .min_size(egui::vec2(70.0, 24.0))
                                ).clicked() {
                                    data.forms.role_unstake_idx    = Some(idx);
                                    data.forms.role_unstake_amount = String::new();
                                    data.forms.role_unstake_result = None;
                                }
                            }
                        });
                    });
                });
                ui.add_space(4.0);
            }

            if let Some((node, role, hunits)) = unstake_action {
                let base     = data.node_url.clone();
                let key_file = data.key_file.clone();
                data.forms.role_unstake_result = Some(match key_file {
                    None     => (false, "no key file".into()),
                    Some(kf) => match crate::sign::submit_role_stake(&base, &kf, &account, &node, &role, hunits, false) {
                        Ok(msg) => { data.forms.role_unstake_idx = None; (true, msg) }
                        Err(e)  => (false, e.to_string()),
                    },
                });
            }

            if let Some((ok, ref msg)) = data.forms.role_unstake_result {
                ui.add_space(2.0);
                ui.colored_label(if ok { GREEN } else { egui::Color32::from_rgb(255, 90, 90) }, msg);
            }

            ui.add_space(12.0);
        }

        // ── Validator Stake / Unstake ─────────────────────────────────────────
        section_heading(ui, "Validator Stake");
        card(ui, |ui| {
            egui::Grid::new("stake_form")
                .num_columns(2)
                .spacing([8.0, 8.0])
                .show(ui, |ui| {
                    ui.label(egui::RichText::new("Amount").color(DIM_TEXT).small());
                    ui.horizontal(|ui| {
                        ui.add(egui::TextEdit::singleline(&mut data.forms.stake_amount)
                            .hint_text("0.0")
                            .desired_width(140.0));
                        ui.label(egui::RichText::new("HONE").color(ORANGE).size(12.0));
                    });
                    ui.end_row();
                });

            ui.add_space(6.0);
            ui.horizontal(|ui| {
                let add_clicked = ui.add(
                    egui::Button::new(egui::RichText::new("Stake").strong())
                        .fill(egui::Color32::from_rgb(37, 99, 235))
                        .min_size(egui::vec2(90.0, 28.0))
                ).clicked();
                ui.add_space(6.0);
                let remove_clicked = ui.add(
                    egui::Button::new(egui::RichText::new("Unstake").strong())
                        .fill(egui::Color32::from_rgb(55, 55, 75))
                        .min_size(egui::vec2(90.0, 28.0))
                ).clicked();

                if add_clicked || remove_clicked {
                    let base     = data.node_url.clone();
                    let key_file = data.key_file.clone();
                    data.forms.stake_result = Some(
                        match (key_file, parse_hone_input(&data.forms.stake_amount)) {
                            (None, _)          => (false, "no key file".into()),
                            (_, None)          => (false, "invalid amount".into()),
                            (Some(kf), Some(d)) => {
                                match crate::sign::submit_stake(&base, &kf, &account, d, add_clicked) {
                                    Ok(msg) => (true, msg),
                                    Err(e)  => (false, e.to_string()),
                                }
                            }
                        }
                    );
                }
            });

            if let Some((ok, ref msg)) = data.forms.stake_result {
                ui.add_space(4.0);
                ui.colored_label(if ok { GREEN } else { egui::Color32::from_rgb(255, 90, 90) }, msg);
            }
        });
    }

    // ── Operator Settings ─────────────────────────────────────────────────────
    // Visible to all signed-in users (posting key needed at submit time)
    ui.add_space(12.0);
    section_heading(ui, "Operator Settings");
    card(ui, |ui| {
        ui.label(egui::RichText::new(
            "Opt your node into a role to accept backers and share epoch rewards with them."
        ).small().color(DIM_TEXT));
        ui.add_space(8.0);

        egui::Grid::new("opt_in_form")
            .num_columns(2)
            .spacing([8.0, 6.0])
            .show(ui, |ui| {
                ui.label(egui::RichText::new("Role").color(DIM_TEXT).small());
                let cur_opt_role = data.forms.role_opt_role.clone();
                egui::ComboBox::from_id_salt("opt_role_combo")
                    .selected_text(if cur_opt_role.is_empty() { "select role" } else { &cur_opt_role })
                    .width(200.0)
                    .show_ui(ui, |ui| {
                        for role in ROLES {
                            let selected = data.forms.role_opt_role == *role;
                            if ui.selectable_label(selected, *role).clicked() {
                                data.forms.role_opt_role = role.to_string();
                            }
                        }
                    });
                ui.end_row();

                ui.label(egui::RichText::new("Backer share").color(DIM_TEXT).small());
                ui.horizontal(|ui| {
                    ui.add(egui::TextEdit::singleline(&mut data.forms.role_opt_share_bps)
                        .hint_text("e.g. 1000")
                        .desired_width(80.0));
                    ui.label(egui::RichText::new("bps (100 bps = 1%)").small().color(DIM_TEXT));
                });
                ui.end_row();

                ui.label(egui::RichText::new("Max backers").color(DIM_TEXT).small());
                ui.add(egui::TextEdit::singleline(&mut data.forms.role_opt_max_backers)
                    .hint_text("e.g. 10")
                    .desired_width(80.0));
                ui.end_row();
            });

        ui.add_space(6.0);
        ui.horizontal(|ui| {
            let in_clicked = ui.add(
                egui::Button::new(egui::RichText::new("Opt In").strong())
                    .fill(egui::Color32::from_rgb(37, 99, 235))
                    .min_size(egui::vec2(90.0, 28.0))
            ).clicked();
            ui.add_space(6.0);
            let out_clicked = ui.add(
                egui::Button::new(egui::RichText::new("Opt Out").strong())
                    .fill(egui::Color32::from_rgb(55, 55, 75))
                    .min_size(egui::vec2(90.0, 28.0))
            ).clicked();

            if in_clicked || out_clicked {
                let base     = data.node_url.clone();
                let key_file = data.key_file.clone();
                let role     = data.forms.role_opt_role.clone();

                data.forms.role_opt_result = Some(if role.is_empty() {
                    (false, "select a role".into())
                } else if let Some(kf) = key_file {
                    if in_clicked {
                        let bps   = data.forms.role_opt_share_bps.trim().parse::<u16>().unwrap_or(0);
                        let max_b = data.forms.role_opt_max_backers.trim().parse::<u8>().unwrap_or(10);
                        match crate::sign::submit_role_opt_in(&base, &kf, &account, &role, bps, max_b) {
                            Ok(msg) => (true, msg),
                            Err(e)  => (false, e.to_string()),
                        }
                    } else {
                        match crate::sign::submit_role_opt_out(&base, &kf, &account, &role) {
                            Ok(msg) => (true, msg),
                            Err(e)  => (false, e.to_string()),
                        }
                    }
                } else {
                    (false, "no key file".into())
                });
            }
        });

        if let Some((ok, ref msg)) = data.forms.role_opt_result {
            ui.add_space(4.0);
            ui.colored_label(if ok { GREEN } else { egui::Color32::from_rgb(255, 90, 90) }, msg);
        }
    });
}
