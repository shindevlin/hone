use super::{
    AppData, section_heading, card, not_signed_in, active_key_required,
    DIM_TEXT, GREEN, BLUE, YELLOW, ORANGE, RED,
    parse_hone_input, hunits_to_hone,
};
use crate::app::KeyRole;

pub fn show(ui: &mut egui::Ui, data: &mut AppData) {
    let account = match data.account.clone() {
        Some(a) => a,
        None => { not_signed_in(ui); return; }
    };

    let can_act = data.key_role.as_ref().map(|r| *r == KeyRole::Active).unwrap_or(false);

    // ── Tab bar ───────────────────────────────────────────────────────────────
    ui.horizontal(|ui| {
        for (i, label) in ["Browse", "New Project", "New Task"].iter().enumerate() {
            let selected = data.forms.project_section == i;
            let text = egui::RichText::new(*label).size(12.0);
            let btn = egui::Button::new(if selected { text.strong() } else { text.color(DIM_TEXT) })
                .fill(if selected {
                    egui::Color32::from_rgb(37, 37, 55)
                } else {
                    egui::Color32::TRANSPARENT
                });
            if ui.add(btn).clicked() {
                data.forms.project_section = i;
            }
        }
    });

    ui.add_space(6.0);

    match data.forms.project_section {
        1 => show_create_project(ui, data, &account, can_act),
        2 => show_create_task(ui, data, &account, can_act),
        _ => show_browse(ui, data, &account, can_act),
    }
}

fn show_browse(ui: &mut egui::Ui, data: &mut AppData, account: &str, can_act: bool) {
    ui.horizontal(|ui| {
        section_heading(ui, "Projects");
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.add(
                egui::Button::new(egui::RichText::new("Refresh").size(11.0))
                    .min_size(egui::vec2(70.0, 22.0))
            ).clicked() {
                if let Ok(v) = crate::api::get_json(&data.node_url, "/api/projects") {
                    data.forms.projects_list = v.get("projects")
                        .and_then(|a| a.as_array())
                        .cloned()
                        .unwrap_or_default();
                    data.forms.project_selected = None;
                    data.forms.project_tasks.clear();
                }
            }
        });
    });

    if data.forms.projects_list.is_empty() {
        ui.add_space(8.0);
        ui.label(egui::RichText::new("No projects found. Click Refresh or create one.")
            .size(11.0).color(DIM_TEXT));
        return;
    }

    // Project list
    let mut select_idx: Option<usize> = None;
    for (i, proj) in data.forms.projects_list.clone().iter().enumerate() {
        let pid  = proj.get("project_id").and_then(|v| v.as_str()).unwrap_or("?");
        let name = proj.get("name").and_then(|v| v.as_str()).unwrap_or("?");
        let creator = proj.get("creator").and_then(|v| v.as_str()).unwrap_or("?");
        let n_collabs = proj.get("collaborators").and_then(|v| v.as_array()).map(|a| a.len()).unwrap_or(0);
        let selected = data.forms.project_selected == Some(i);

        ui.add_space(3.0);
        card(ui, |ui| {
            let resp = ui.add(egui::Label::new(
                egui::RichText::new(name)
                    .size(13.0)
                    .strong()
                    .color(if selected { ORANGE } else { egui::Color32::WHITE })
            ).sense(egui::Sense::click()));
            if resp.clicked() { select_idx = Some(i); }

            ui.horizontal(|ui| {
                ui.label(egui::RichText::new(pid).size(10.0).monospace().color(DIM_TEXT));
                ui.add_space(8.0);
                ui.label(egui::RichText::new(format!("by {}", creator)).size(10.0).color(DIM_TEXT));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(egui::RichText::new(format!("{} member(s)", n_collabs + 1))
                        .size(10.0).color(DIM_TEXT));
                });
            });
        });
    }

    if let Some(i) = select_idx {
        data.forms.project_selected = Some(i);
        data.forms.task_action_result = None;
        let pid = data.forms.projects_list[i]
            .get("project_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
        if let Ok(v) = crate::api::get_json(&data.node_url, &format!("/api/projects/{}/tasks", pid)) {
            data.forms.project_tasks = v.get("tasks")
                .and_then(|a| a.as_array())
                .cloned()
                .unwrap_or_default();
        }
    }

    // Task list for selected project
    if let Some(idx) = data.forms.project_selected {
        let proj = &data.forms.projects_list[idx];
        let pid = proj.get("project_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let proj_creator = proj.get("creator").and_then(|v| v.as_str()).unwrap_or("").to_string();
        let is_lead = account == proj_creator;

        ui.add_space(8.0);
        section_heading(ui, "Tasks");

        if data.forms.project_tasks.is_empty() {
            ui.label(egui::RichText::new("No tasks yet.").size(11.0).color(DIM_TEXT));
        }

        for task in data.forms.project_tasks.clone().iter() {
            let tid    = task.get("task_id").and_then(|v| v.as_str()).unwrap_or("?");
            let title  = task.get("title").and_then(|v| v.as_str()).unwrap_or("?");
            let ttype  = task.get("task_type").and_then(|v| v.as_str()).unwrap_or("partial");
            let status = task.get("status").and_then(|v| v.as_str()).unwrap_or("?");
            let bounty = task.get("bounty").and_then(|v| v.as_u64()).unwrap_or(0);
            let claimant = task.get("claimant").and_then(|v| v.as_str()).unwrap_or("");
            let sub_link = task.get("submission_link").and_then(|v| v.as_str()).unwrap_or("");

            let status_color = match status {
                "approved" => GREEN,
                "submitted" => BLUE,
                "claimed"  => YELLOW,
                _          => DIM_TEXT,
            };
            let type_color = if ttype == "full" { BLUE } else { ORANGE };

            ui.add_space(4.0);
            card(ui, |ui| {
                ui.horizontal(|ui| {
                    ui.label(egui::RichText::new(title).size(12.0).strong().color(egui::Color32::WHITE));
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(egui::RichText::new(status).size(10.0).color(status_color));
                        ui.add_space(6.0);
                        ui.label(egui::RichText::new(ttype).size(10.0).color(type_color));
                    });
                });

                if bounty > 0 {
                    ui.label(egui::RichText::new(format!("Bounty: {} HONE", hunits_to_hone(bounty)))
                        .size(11.0).color(ORANGE));
                }
                if !claimant.is_empty() {
                    ui.label(egui::RichText::new(format!("Claimed by: {}", claimant))
                        .size(10.0).color(DIM_TEXT));
                }
                if !sub_link.is_empty() {
                    ui.label(egui::RichText::new(format!("Submission: {}", sub_link))
                        .size(10.0).color(BLUE));
                }

                if can_act {
                    ui.add_space(4.0);
                    ui.horizontal(|ui| {
                        let key_file = data.key_file.clone();

                        // Claim button (open tasks only, not already claimed by me)
                        if status == "open" {
                            if ui.add(
                                egui::Button::new(egui::RichText::new("Claim").size(11.0))
                                    .min_size(egui::vec2(55.0, 20.0))
                            ).clicked() {
                                if let Some(kf) = &key_file {
                                    match crate::sign::submit_task_claim(
                                        &data.node_url, kf, account, &pid, tid
                                    ) {
                                        Ok(m) => data.forms.task_action_result = Some((true, m)),
                                        Err(e) => data.forms.task_action_result = Some((false, e.to_string())),
                                    }
                                }
                            }
                        }

                        // Submit button (claimed by me, not yet submitted)
                        if status == "claimed" && claimant == account {
                            ui.add(egui::TextEdit::singleline(&mut data.forms.task_link)
                                .hint_text("link to work")
                                .desired_width(140.0));
                            if ui.add(
                                egui::Button::new(egui::RichText::new("Submit").size(11.0))
                                    .min_size(egui::vec2(55.0, 20.0))
                            ).clicked() {
                                let link = data.forms.task_link.trim().to_owned();
                                if let Some(kf) = &key_file {
                                    match crate::sign::submit_task_submit(
                                        &data.node_url, kf, account, &pid, tid,
                                        &link, None
                                    ) {
                                        Ok(m) => data.forms.task_action_result = Some((true, m)),
                                        Err(e) => data.forms.task_action_result = Some((false, e.to_string())),
                                    }
                                }
                            }
                        }

                        // Approve button (project lead only, submitted tasks)
                        if is_lead && status == "submitted" {
                            if ui.add(
                                egui::Button::new(egui::RichText::new("Approve").size(11.0).strong())
                                    .fill(egui::Color32::from_rgb(37, 99, 235))
                                    .min_size(egui::vec2(65.0, 20.0))
                            ).clicked() {
                                let recipient = claimant.to_owned();
                                if let Some(kf) = &key_file {
                                    match crate::sign::submit_task_approve(
                                        &data.node_url, kf, account, &pid, tid,
                                        &recipient, None
                                    ) {
                                        Ok(m) => data.forms.task_action_result = Some((true, m)),
                                        Err(e) => data.forms.task_action_result = Some((false, e.to_string())),
                                    }
                                }
                            }
                        }
                    });
                }
            });
        }

        if let Some((ok, msg)) = &data.forms.task_action_result {
            ui.add_space(4.0);
            ui.label(egui::RichText::new(msg).size(11.0)
                .color(if *ok { GREEN } else { RED }));
        }
    }
}

fn show_create_project(ui: &mut egui::Ui, data: &mut AppData, account: &str, can_act: bool) {
    section_heading(ui, "Create Project");

    if !can_act {
        active_key_required(ui);
        return;
    }

    card(ui, |ui| {
        egui::Grid::new("new_project_grid")
            .num_columns(2)
            .spacing([12.0, 8.0])
            .show(ui, |ui| {
                ui.label(egui::RichText::new("Project ID").color(DIM_TEXT).small());
                ui.add(egui::TextEdit::singleline(&mut data.forms.new_project_id)
                    .hint_text("my-project (unique slug)")
                    .desired_width(220.0));
                ui.end_row();

                ui.label(egui::RichText::new("Name").color(DIM_TEXT).small());
                ui.add(egui::TextEdit::singleline(&mut data.forms.new_project_name)
                    .hint_text("My Project")
                    .desired_width(220.0));
                ui.end_row();

                ui.label(egui::RichText::new("Description").color(DIM_TEXT).small());
                ui.add(egui::TextEdit::multiline(&mut data.forms.new_project_description)
                    .desired_width(220.0)
                    .desired_rows(3));
                ui.end_row();

                ui.label(egui::RichText::new("Repo (optional)").color(DIM_TEXT).small());
                ui.add(egui::TextEdit::singleline(&mut data.forms.new_project_repo)
                    .hint_text("owner/repo or URL")
                    .desired_width(220.0));
                ui.end_row();
            });

        ui.add_space(6.0);

        if ui.add(
            egui::Button::new(egui::RichText::new("Create Project").strong().size(12.0))
                .fill(ORANGE)
                .min_size(egui::vec2(130.0, 26.0))
        ).clicked() {
            let pid = data.forms.new_project_id.trim().to_owned();
            let name = data.forms.new_project_name.trim().to_owned();
            let desc = data.forms.new_project_description.trim().to_owned();
            let repo_raw = data.forms.new_project_repo.trim().to_owned();
            let repo = if repo_raw.is_empty() { None } else { Some(repo_raw.as_str()) };

            if pid.is_empty() || name.is_empty() {
                data.forms.new_project_result = Some((false, "project ID and name required".into()));
            } else if let Some(kf) = data.key_file.clone() {
                match crate::sign::submit_project_create(
                    &data.node_url, &kf, account, &pid, &name, &desc, repo
                ) {
                    Ok(m) => {
                        data.forms.new_project_result = Some((true, m));
                        data.forms.new_project_id.clear();
                        data.forms.new_project_name.clear();
                        data.forms.new_project_description.clear();
                        data.forms.new_project_repo.clear();
                    }
                    Err(e) => data.forms.new_project_result = Some((false, e.to_string())),
                }
            } else {
                data.forms.new_project_result = Some((false, "no key file loaded".into()));
            }
        }

        if let Some((ok, msg)) = &data.forms.new_project_result {
            ui.add_space(4.0);
            ui.label(egui::RichText::new(msg).size(11.0)
                .color(if *ok { GREEN } else { RED }));
        }
    });
}

fn show_create_task(ui: &mut egui::Ui, data: &mut AppData, account: &str, can_act: bool) {
    section_heading(ui, "Post Task");

    if !can_act {
        active_key_required(ui);
        return;
    }

    card(ui, |ui| {
        egui::Grid::new("new_task_grid")
            .num_columns(2)
            .spacing([12.0, 8.0])
            .show(ui, |ui| {
                ui.label(egui::RichText::new("Project ID").color(DIM_TEXT).small());
                ui.add(egui::TextEdit::singleline(&mut data.forms.new_project_id)
                    .hint_text("project slug")
                    .desired_width(180.0));
                ui.end_row();

                ui.label(egui::RichText::new("Task ID").color(DIM_TEXT).small());
                ui.add(egui::TextEdit::singleline(&mut data.forms.new_task_id)
                    .hint_text("task-slug")
                    .desired_width(180.0));
                ui.end_row();

                ui.label(egui::RichText::new("Title").color(DIM_TEXT).small());
                ui.add(egui::TextEdit::singleline(&mut data.forms.new_task_title)
                    .hint_text("Fix the bug")
                    .desired_width(220.0));
                ui.end_row();

                ui.label(egui::RichText::new("Description").color(DIM_TEXT).small());
                ui.add(egui::TextEdit::multiline(&mut data.forms.new_task_description)
                    .desired_width(220.0)
                    .desired_rows(3));
                ui.end_row();

                ui.label(egui::RichText::new("Type").color(DIM_TEXT).small());
                ui.horizontal(|ui| {
                    let is_partial = data.forms.new_task_type != "full";
                    if ui.add(egui::RadioButton::new(is_partial, "partial (bounty)")).clicked() {
                        data.forms.new_task_type = "partial".into();
                    }
                    if ui.add(egui::RadioButton::new(!is_partial, "full (collaborator)")).clicked() {
                        data.forms.new_task_type = "full".into();
                    }
                });
                ui.end_row();

                ui.label(egui::RichText::new("Bounty (HONE)").color(DIM_TEXT).small());
                ui.add(egui::TextEdit::singleline(&mut data.forms.new_task_bounty)
                    .hint_text("0.0")
                    .desired_width(120.0));
                ui.end_row();
            });

        ui.add_space(4.0);
        ui.label(egui::RichText::new(
            "partial: fixed bounty paid on approval.  full: approving adds contributor as project collaborator."
        ).size(10.0).color(DIM_TEXT));

        ui.add_space(6.0);

        if ui.add(
            egui::Button::new(egui::RichText::new("Post Task").strong().size(12.0))
                .fill(egui::Color32::from_rgb(37, 99, 235))
                .min_size(egui::vec2(100.0, 26.0))
        ).clicked() {
            let pid   = data.forms.new_project_id.trim().to_owned();
            let tid   = data.forms.new_task_id.trim().to_owned();
            let title = data.forms.new_task_title.trim().to_owned();
            let desc  = data.forms.new_task_description.trim().to_owned();
            let ttype = if data.forms.new_task_type.is_empty() { "partial".to_owned() }
                        else { data.forms.new_task_type.clone() };
            let bounty = parse_hone_input(&data.forms.new_task_bounty).unwrap_or(0);

            if pid.is_empty() || tid.is_empty() || title.is_empty() {
                data.forms.new_task_result = Some((false, "project ID, task ID, and title required".into()));
            } else if let Some(kf) = data.key_file.clone() {
                match crate::sign::submit_project_task(
                    &data.node_url, &kf, account, &pid, &tid, &title, &desc, &ttype, bounty
                ) {
                    Ok(m) => {
                        data.forms.new_task_result = Some((true, m));
                        data.forms.new_task_id.clear();
                        data.forms.new_task_title.clear();
                        data.forms.new_task_description.clear();
                        data.forms.new_task_bounty.clear();
                    }
                    Err(e) => data.forms.new_task_result = Some((false, e.to_string())),
                }
            } else {
                data.forms.new_task_result = Some((false, "no key file loaded".into()));
            }
        }

        if let Some((ok, msg)) = &data.forms.new_task_result {
            ui.add_space(4.0);
            ui.label(egui::RichText::new(msg).size(11.0)
                .color(if *ok { GREEN } else { RED }));
        }
    });
}
