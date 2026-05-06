use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, Tabs},
};

use crate::app::{App, Mode, StakeAction, LoginState, STAKE_ROLES};

// ── Helpers ───────────────────────────────────────────────────────────────────

fn btcpc(dreams: u64) -> String {
    let whole = dreams / 10_000_000_000;
    let frac  = dreams % 10_000_000_000;
    if frac == 0 {
        format!("{}", whole)
    } else {
        let s = format!("{:010}", frac);
        format!("{}.{}", whole, s.trim_end_matches('0'))
    }
}

fn truncate(s: &str, n: usize) -> &str {
    if s.len() <= n { s } else { &s[..n] }
}

fn req_min(req: &Option<serde_json::Value>, role: &str) -> Option<u64> {
    let r = req.as_ref()?;
    // Try requirements.{role} first, then {role}.dreams
    r.get("requirements").and_then(|m| m.get(role)).and_then(|v| v.as_u64())
        .or_else(|| r.get(role).and_then(|m| m.get("dreams")).and_then(|v| v.as_u64()))
}

// ── Top-level render ──────────────────────────────────────────────────────────

pub fn render(f: &mut Frame, app: &App) {
    if let Mode::Login(ref s) = app.mode {
        render_login(f, s);
        return;
    }

    let size = f.area();
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // tab bar
            Constraint::Length(1), // node status banner
            Constraint::Min(1),    // content
            Constraint::Length(1), // footer
        ])
        .split(size);

    render_tabs(f, app, chunks[0]);
    render_node_banner(f, app, chunks[1]);
    render_body(f, app, chunks[2]);
    render_footer(f, app, chunks[3]);
}

// ── Login ─────────────────────────────────────────────────────────────────────

fn render_login(f: &mut Frame, s: &LoginState) {
    let size = f.area();
    let box_w = 62u16.min(size.width.saturating_sub(4));
    let box_h = 14u16.min(size.height.saturating_sub(4));
    let x = (size.width.saturating_sub(box_w)) / 2;
    let y = (size.height.saturating_sub(box_h)) / 2;
    let area = Rect::new(x, y, box_w, box_h);
    f.render_widget(Clear, area);
    let block = Block::default()
        .borders(Borders::ALL)
        .title(" BTCPC — Sign In ")
        .title_alignment(Alignment::Center)
        .style(Style::default().fg(Color::Yellow));
    f.render_widget(block, area);
    let inner = Rect::new(area.x + 2, area.y + 1, area.width.saturating_sub(4), area.height.saturating_sub(2));
    let field_areas = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(2),
            Constraint::Length(2),
            Constraint::Length(2),
            Constraint::Length(1),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .split(inner);
    for (i, (label, value)) in s.fields().iter().enumerate() {
        let focused = s.field == i;
        let display = if focused { format!("{}: {}_", label, value) } else { format!("{}: {}", label, value) };
        let style = if focused { Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD) } else { Style::default().fg(Color::White) };
        f.render_widget(Paragraph::new(display).style(style), field_areas[i]);
    }
    f.render_widget(
        Paragraph::new("Tab / ↓↑ navigate  Enter sign in  Ctrl-C quit")
            .style(Style::default().fg(Color::DarkGray)),
        field_areas[4],
    );
    if let Some(ref err) = s.error {
        f.render_widget(
            Paragraph::new(err.as_str()).style(Style::default().fg(Color::Red).add_modifier(Modifier::BOLD)),
            field_areas[5],
        );
    }
}

// ── Tabs ──────────────────────────────────────────────────────────────────────

fn render_tabs(f: &mut Frame, app: &App, area: Rect) {
    let titles = vec![
        Line::from("[1] Wallet"),
        Line::from("[2] Staking"),
        Line::from("[3] Explorer"),
        Line::from("[4] Inference"),
    ];
    let tabs = Tabs::new(titles)
        .block(Block::default().borders(Borders::ALL).title(" BTCPC "))
        .select(app.tab)
        .style(Style::default().fg(Color::DarkGray))
        .highlight_style(Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD));
    f.render_widget(tabs, area);
}

// ── Node status banner (compact 1 line) ───────────────────────────────────────

fn render_node_banner(f: &mut Frame, app: &App, area: Rect) {
    let info = app.node_info.as_ref();
    let epoch     = info.and_then(|v| v.get("epoch")).and_then(|v| v.as_u64()).map(|e| e.to_string()).unwrap_or_else(|| "—".into());
    let peers     = info.and_then(|v| v.get("peer_count")).and_then(|v| v.as_u64()).map(|e| e.to_string()).unwrap_or_else(|| "—".into());
    let chain_id  = info.and_then(|v| v.get("chain_id")).and_then(|v| v.as_str()).unwrap_or("—");
    let version   = info.and_then(|v| v.get("version")).and_then(|v| v.as_str()).unwrap_or("—");
    let account   = app.session.as_ref().map(|s| format!("  @{} ({})", s.account, s.key_role)).unwrap_or_default();
    let (dot, dot_color) = if info.is_some() { ("●", Color::Green) } else { ("○", Color::DarkGray) };

    let line = Line::from(vec![
        Span::styled(format!(" {} ", dot), Style::default().fg(dot_color)),
        Span::styled(format!("Epoch {}  Peers {}  {}  v{}", epoch, peers, chain_id, version), Style::default().fg(Color::DarkGray)),
        Span::styled(account, Style::default().fg(Color::Yellow)),
    ]);
    f.render_widget(Paragraph::new(line), area);
}

// ── Body dispatcher ───────────────────────────────────────────────────────────

fn render_body(f: &mut Frame, app: &App, area: Rect) {
    match app.tab {
        0 => render_wallet_tab(f, app, area),
        1 => render_staking_tab(f, app, area),
        2 => render_explorer_tab(f, app, area),
        3 => render_inference_tab(f, app, area),
        _ => {}
    }

    match &app.mode {
        Mode::Login(_) | Mode::Normal => {}
        Mode::TransferForm(state)  => render_transfer_form(f, area, state),
        Mode::StakeForm(state)     => render_stake_form(f, area, state),
        Mode::RoleStakeForm(state) => render_role_stake_form(f, area, state, &app.staking_requirements),
        Mode::PostJobForm(state)   => render_post_job_form(f, area, state),
        Mode::Result { msg, success } => render_result(f, area, msg, *success),
    }
}

// ── Wallet tab ────────────────────────────────────────────────────────────────

fn render_wallet_tab(f: &mut Frame, app: &App, area: Rect) {
    if app.session.is_none() {
        let p = Paragraph::new("Not logged in.\n\nSign in to see your wallet.")
            .block(Block::default().borders(Borders::ALL).title(" Wallet "))
            .style(Style::default().fg(Color::DarkGray))
            .alignment(Alignment::Center);
        f.render_widget(p, area);
        return;
    }
    let session = app.session.as_ref().unwrap();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(6), // balance card
            Constraint::Min(1),    // actions
        ])
        .split(area);

    // ── Balance card ─────────────────────────────────────────────────────────
    let bal = app.wallet_balance.map(btcpc).unwrap_or_else(|| "—".into());
    let staked = app.wallet_staked.map(btcpc).unwrap_or_else(|| "—".into());

    // Build key role line
    let can_spend = session.can_spend_tokens();
    let role_line = Line::from(vec![
        Span::styled("  Key: ", Style::default().fg(Color::DarkGray)),
        Span::styled(
            session.key_role.to_string(),
            Style::default().fg(if can_spend { Color::Green } else { Color::Yellow }),
        ),
        Span::styled(
            if can_spend { "  (full token access)" } else { "  (token actions locked — use active key)" },
            Style::default().fg(Color::DarkGray),
        ),
    ]);

    let eth_line = {
        let eth_str = match &app.eth_address {
            Some(addr) => {
                let short = if addr.len() > 14 {
                    format!("{}…{}", &addr[..8], &addr[addr.len()-6..])
                } else { addr.clone() };
                short
            }
            None => "—".into(),
        };
        Line::from(vec![
            Span::styled("  ETH  ", Style::default().fg(Color::DarkGray)),
            Span::styled(eth_str, Style::default().fg(Color::Blue).add_modifier(Modifier::BOLD)),
            Span::styled("  (wBTCPC perma-tied wallet)", Style::default().fg(Color::DarkGray)),
        ])
    };

    let content = vec![
        Line::from(vec![
            Span::styled("  Account  ", Style::default().fg(Color::DarkGray)),
            Span::styled(&session.account, Style::default().fg(Color::White).add_modifier(Modifier::BOLD)),
            Span::raw("     "),
            Span::styled("Balance  ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{} BTCPC", bal), Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            Span::raw("     "),
            Span::styled("Staked  ", Style::default().fg(Color::DarkGray)),
            Span::styled(format!("{} BTCPC", staked), Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
        ]),
        Line::from(""),
        role_line,
        Line::from(""),
        eth_line,
    ];

    f.render_widget(
        Paragraph::new(content).block(Block::default().borders(Borders::ALL).title(" Wallet ")),
        chunks[0],
    );

    // ── Actions ──────────────────────────────────────────────────────────────
    let action_lines = if can_spend {
        vec![
            Line::from(""),
            Line::from(vec![
                Span::styled("  [T]", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
                Span::raw("  Transfer BTCPC to another account"),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("  [A]", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
                Span::raw("  Add validator stake"),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("  [X]", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
                Span::raw("  Remove validator stake"),
            ]),
            Line::from(""),
            Line::from(vec![
                Span::styled("  [2]", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                Span::raw("  Switch to Staking tab — stake tokens to back specific node roles"),
            ]),
        ]
    } else {
        vec![
            Line::from(""),
            Line::from(Span::styled("  Active key required for token operations.", Style::default().fg(Color::DarkGray))),
            Line::from(""),
            Line::from(vec![
                Span::styled("  [2]", Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)),
                Span::raw("  Switch to Staking tab"),
            ]),
        ]
    };

    f.render_widget(
        Paragraph::new(action_lines).block(Block::default().borders(Borders::ALL).title(" Actions ")),
        chunks[1],
    );
}

// ── Staking tab ───────────────────────────────────────────────────────────────

fn render_staking_tab(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(4),  // role minimums
            Constraint::Min(1),     // my positions
            Constraint::Length(5),  // actions
        ])
        .split(area);

    // ── Role minimums ─────────────────────────────────────────────────────────
    let roles = [("clock", "miner", "storage", "sensor")];
    let req = &app.staking_requirements;
    let mins: Vec<String> = ["clock", "miner", "storage", "sensor"].iter().map(|r| {
        let v = req_min(req, r).map(btcpc).unwrap_or_else(|| "?".into());
        format!("{}  {} BTCPC", r, v)
    }).collect();

    let min_line = Line::from(vec![
        Span::raw("  "),
        Span::styled(&mins[0], Style::default().fg(Color::White)),
        Span::raw("    "),
        Span::styled(&mins[1], Style::default().fg(Color::White)),
        Span::raw("    "),
        Span::styled(&mins[2], Style::default().fg(Color::White)),
        Span::raw("    "),
        Span::styled(&mins[3], Style::default().fg(Color::White)),
    ]);
    let slope = req.as_ref()
        .and_then(|r| r.get("loyalty_slope_bps")).and_then(|v| v.as_u64())
        .unwrap_or(5000);
    let slope_line = Line::from(vec![
        Span::raw("  "),
        Span::styled(
            format!("Loyalty slope: {}% — existing operators pay {}% of any minimum increase", slope / 100, slope / 100),
            Style::default().fg(Color::DarkGray),
        ),
    ]);

    f.render_widget(
        Paragraph::new(vec![Line::from(""), min_line, slope_line])
            .block(Block::default().borders(Borders::ALL).title(" Role Minimums ")),
        chunks[0],
    );

    // ── My positions ──────────────────────────────────────────────────────────
    if app.role_positions.is_empty() {
        f.render_widget(
            Paragraph::new("\n  No role positions yet. Press [S] to stake a role.")
                .block(Block::default().borders(Borders::ALL).title(" My Positions "))
                .style(Style::default().fg(Color::DarkGray)),
            chunks[1],
        );
    } else {
        let rows: Vec<Row> = app.role_positions.iter().map(|p| {
            let node   = p.get("node").and_then(|v| v.as_str()).unwrap_or("?");
            let role   = p.get("role").and_then(|v| v.as_str()).unwrap_or("?");
            let amount = p.get("amount").and_then(|v| v.as_u64()).map(btcpc).unwrap_or_else(|| "?".into());
            Row::new(vec![
                Cell::from(node).style(Style::default().fg(Color::White)),
                Cell::from(role).style(Style::default().fg(Color::Cyan)),
                Cell::from(format!("{} BTCPC", amount)).style(Style::default().fg(Color::Yellow)),
            ])
        }).collect();

        let table = Table::new(
            rows,
            [Constraint::Min(18), Constraint::Length(12), Constraint::Min(16)],
        )
        .block(Block::default().borders(Borders::ALL).title(" My Positions "))
        .header(Row::new(vec!["Node", "Role", "Staked"]).style(Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD)));

        f.render_widget(table, chunks[1]);
    }

    // ── Actions ───────────────────────────────────────────────────────────────
    let can_spend = app.session.as_ref().map(|s| s.can_spend_tokens()).unwrap_or(false);
    let action_lines = if app.session.is_none() {
        vec![Line::from(Span::styled("  Sign in to stake roles.", Style::default().fg(Color::DarkGray)))]
    } else if can_spend {
        vec![
            Line::from(vec![
                Span::styled("  [S]", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
                Span::raw("  Stake — back a node's role with your BTCPC"),
            ]),
            Line::from(vec![
                Span::styled("  [U]", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
                Span::raw("  Unstake — reclaim BTCPC from a role position"),
            ]),
        ]
    } else {
        vec![Line::from(Span::styled("  Active key required for staking.", Style::default().fg(Color::DarkGray)))]
    };

    f.render_widget(
        Paragraph::new(action_lines).block(Block::default().borders(Borders::ALL).title(" Actions ")),
        chunks[2],
    );
}

// ── Explorer tab ──────────────────────────────────────────────────────────────

fn render_explorer_tab(f: &mut Frame, app: &App, area: Rect) {
    let rows: Vec<Row> = app.blocks.iter().map(|b| {
        let epoch   = b.get("epoch").and_then(|v| v.as_u64()).map(|e| e.to_string()).unwrap_or_else(|| "—".into());
        let hash    = b.get("hash").and_then(|v| v.as_str()).map(|h| truncate(h, 16).to_string()).unwrap_or_else(|| "—".into());
        let entries = b.get("entry_count").and_then(|v| v.as_u64()).map(|e| e.to_string()).unwrap_or_else(|| "—".into());
        let ts      = b.get("timestamp_ms").and_then(|v| v.as_u64()).map(|t| t.to_string()).unwrap_or_else(|| "—".into());
        Row::new(vec![epoch, hash, entries, ts])
    }).collect();

    let table = Table::new(rows, [Constraint::Length(8), Constraint::Length(18), Constraint::Length(8), Constraint::Min(14)])
        .block(Block::default().borders(Borders::ALL).title(" Recent Blocks "))
        .header(Row::new(vec!["Epoch", "Hash", "Entries", "Timestamp ms"]).style(Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD)));

    f.render_widget(table, area);
}

// ── Inference tab ─────────────────────────────────────────────────────────────

fn render_inference_tab(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(3)])
        .split(area);

    let rows: Vec<Row> = app.jobs.iter().map(|j| {
        let id     = j.get("job_id").or_else(|| j.get("id")).and_then(|v| v.as_str()).map(|s| truncate(s, 12).to_string()).unwrap_or_else(|| "—".into());
        let model  = j.get("model").and_then(|v| v.as_str()).unwrap_or("—").to_string();
        let status = j.get("status").and_then(|v| v.as_str()).unwrap_or("—").to_string();
        let fee    = j.get("max_fee").and_then(|v| v.as_u64()).map(btcpc).unwrap_or_else(|| "—".into());
        let epoch  = j.get("epoch").or_else(|| j.get("deadline_epoch")).and_then(|v| v.as_u64()).map(|e| e.to_string()).unwrap_or_else(|| "—".into());
        Row::new(vec![id, model, status, fee, epoch])
    }).collect();

    let table = Table::new(rows, [Constraint::Length(14), Constraint::Min(20), Constraint::Length(10), Constraint::Length(14), Constraint::Length(8)])
        .block(Block::default().borders(Borders::ALL).title(" Inference Jobs "))
        .header(Row::new(vec!["ID", "Model", "Status", "Fee BTCPC", "Epoch"]).style(Style::default().fg(Color::DarkGray).add_modifier(Modifier::BOLD)));

    f.render_widget(table, chunks[0]);

    let can_spend = app.session.as_ref().map(|s| s.can_spend_tokens()).unwrap_or(false);
    let hint = if can_spend {
        Line::from(vec![
            Span::styled("  [N]", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            Span::raw("  Post a new inference job"),
        ])
    } else {
        Line::from(Span::styled("  [N] Post job  (active key required)", Style::default().fg(Color::DarkGray)))
    };
    f.render_widget(Paragraph::new(hint).block(Block::default().borders(Borders::ALL)), chunks[1]);
}

// ── Footer (minimal) ─────────────────────────────────────────────────────────

fn render_footer(f: &mut Frame, app: &App, area: Rect) {
    let status = if app.status_msg.is_empty() { String::new() } else { format!("  │  {}", app.status_msg) };
    let text = format!(" [Q] quit  [R] refresh  [1-4] tabs{}", status);
    f.render_widget(
        Paragraph::new(Span::styled(text, Style::default().fg(Color::DarkGray))),
        area,
    );
}

// ── Form helpers ──────────────────────────────────────────────────────────────

fn centered_rect(percent_x: u16, min_height: u16, area: Rect) -> Rect {
    let popup_width = (area.width as u32 * percent_x as u32 / 100) as u16;
    let x = area.x + (area.width.saturating_sub(popup_width)) / 2;
    let y = area.y + (area.height.saturating_sub(min_height)) / 2;
    Rect {
        x,
        y: y.min(area.y + area.height.saturating_sub(min_height)),
        width: popup_width.min(area.width),
        height: min_height.min(area.height),
    }
}

fn render_form_fields(f: &mut Frame, title: &str, fields: &[(&str, &str)], focused: usize, hint: &str, area: Rect) {
    let height = (fields.len() as u16 * 3) + 4;
    let popup = centered_rect(70, height, area);
    f.render_widget(Clear, popup);
    let outer_block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(title, Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)));
    f.render_widget(outer_block.clone(), popup);
    let inner = outer_block.inner(popup);
    let mut constraints: Vec<Constraint> = fields.iter().map(|_| Constraint::Length(3)).collect();
    constraints.push(Constraint::Min(1));
    let field_chunks = Layout::default().direction(Direction::Vertical).constraints(constraints).split(inner);
    for (i, (label, value)) in fields.iter().enumerate() {
        let is_focused = i == focused;
        let display = if is_focused { format!("{}_", value) } else { value.to_string() };
        let border_style = if is_focused { Style::default().fg(Color::Yellow) } else { Style::default().fg(Color::DarkGray) };
        let value_style  = if is_focused { Style::default().fg(Color::Yellow) } else { Style::default().fg(Color::White) };
        let widget = Paragraph::new(Span::styled(display, value_style))
            .block(Block::default().borders(Borders::ALL).border_style(border_style).title(*label));
        if i < field_chunks.len().saturating_sub(1) {
            f.render_widget(widget, field_chunks[i]);
        }
    }
    if let Some(hint_area) = field_chunks.last() {
        f.render_widget(
            Paragraph::new(Span::styled(hint, Style::default().fg(Color::DarkGray))).alignment(Alignment::Center),
            *hint_area,
        );
    }
}

fn render_transfer_form(f: &mut Frame, area: Rect, state: &crate::app::TransferState) {
    render_form_fields(f, " Transfer BTCPC ", &state.field_values(), state.field,
        "Tab next field   Enter submit   Esc cancel", area);
}

fn render_stake_form(f: &mut Frame, area: Rect, state: &crate::app::StakeState) {
    let title = if state.action == StakeAction::Add { " Add Validator Stake " } else { " Remove Validator Stake " };
    render_form_fields(f, title, &state.field_values(), state.field, "Enter submit   Esc cancel", area);
}

fn render_role_stake_form(
    f: &mut Frame,
    area: Rect,
    state: &crate::app::RoleStakeState,
    req: &Option<serde_json::Value>,
) {
    let title = if state.add { " Stake a Role " } else { " Unstake a Role " };
    let role_display = format!("{}  (← → to change)", state.role());
    let fields: &[(&str, &str)] = &[
        ("Node account", state.node.as_str()),
        ("Role",         &role_display),
        ("Amount (BTCPC)", state.amount.as_str()),
    ];

    // Build hint with minimum for selected role
    let min_str = req_min(req, state.role())
        .map(|d| format!("Min for {}: {} BTCPC   ", state.role(), btcpc(d)))
        .unwrap_or_default();
    let hint = format!("{}Tab next field   Enter submit   Esc cancel", min_str);

    // For the role field, don't show the cursor
    let display_fields: Vec<(&str, String)> = fields.iter().enumerate().map(|(i, (lbl, val))| {
        if i == 1 {
            (*lbl, format!("◀  {}  ▶", state.role()))
        } else {
            (*lbl, val.to_string())
        }
    }).collect();
    let display_refs: Vec<(&str, &str)> = display_fields.iter().map(|(l, v)| (*l, v.as_str())).collect();

    render_form_fields(f, title, &display_refs, state.field, &hint, area);
}

fn render_post_job_form(f: &mut Frame, area: Rect, state: &crate::app::PostJobState) {
    render_form_fields(f, " Post Inference Job ", &state.field_values(), state.field,
        "Tab next field   Enter submit   Esc cancel", area);
}

fn render_result(f: &mut Frame, area: Rect, msg: &str, success: bool) {
    let popup = centered_rect(60, 7, area);
    f.render_widget(Clear, popup);
    let color = if success { Color::Green } else { Color::Red };
    let title = if success { " Done " } else { " Error " };
    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(color))
        .title(Span::styled(title, Style::default().fg(color).add_modifier(Modifier::BOLD)));
    let inner = block.inner(popup);
    f.render_widget(block, popup);
    let lines = vec![
        Line::from(Span::styled(msg, Style::default().fg(color))),
        Line::from(""),
        Line::from(Span::styled("Press any key to continue", Style::default().fg(Color::DarkGray))),
    ];
    f.render_widget(Paragraph::new(lines).alignment(Alignment::Center), inner);
}
