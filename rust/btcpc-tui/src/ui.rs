use ratatui::{
    Frame,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Clear, Paragraph, Row, Table, Tabs},
};

use crate::app::{App, Mode, StakeAction};

// ── Conversion helpers ────────────────────────────────────────────────────────

fn dreams_to_btcpc(dreams: u64) -> String {
    let whole = dreams / 10_000_000_000;
    let frac = dreams % 10_000_000_000;
    format!("{}.{:010}", whole, frac)
}

fn truncate(s: &str, n: usize) -> &str {
    if s.len() <= n { s } else { &s[..n] }
}

// ── Top-level render ──────────────────────────────────────────────────────────

pub fn render(f: &mut Frame, app: &App) {
    let size = f.area();

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(3), // tab bar
            Constraint::Min(1),    // content
            Constraint::Length(3), // footer
        ])
        .split(size);

    render_tabs(f, app, chunks[0]);
    render_body(f, app, chunks[1]);
    render_footer(f, app, chunks[2]);
}

// ── Tabs bar ──────────────────────────────────────────────────────────────────

fn render_tabs(f: &mut Frame, app: &App, area: Rect) {
    let titles = vec![
        Line::from("[1] Node"),
        Line::from("[2] Wallet"),
        Line::from("[3] Explorer"),
        Line::from("[4] Inference"),
    ];
    let tabs = Tabs::new(titles)
        .block(Block::default().borders(Borders::ALL).title("BTCPC"))
        .select(app.tab)
        .style(Style::default().fg(Color::White))
        .highlight_style(
            Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD),
        );
    f.render_widget(tabs, area);
}

// ── Body dispatcher ───────────────────────────────────────────────────────────

fn render_body(f: &mut Frame, app: &App, area: Rect) {
    // Always render the underlying tab first, then overlay form/result if needed
    match app.tab {
        0 => render_node_tab(f, app, area),
        1 => render_wallet_tab(f, app, area),
        2 => render_explorer_tab(f, app, area),
        3 => render_inference_tab(f, app, area),
        _ => {}
    }

    match &app.mode {
        Mode::Normal => {}
        Mode::TransferForm(state) => render_transfer_form(f, area, state),
        Mode::StakeForm(state) => render_stake_form(f, area, state),
        Mode::PostJobForm(state) => render_post_job_form(f, area, state),
        Mode::Result { msg, success } => render_result(f, area, msg, *success),
    }
}

// ── Node tab ──────────────────────────────────────────────────────────────────

fn render_node_tab(f: &mut Frame, app: &App, area: Rect) {
    let info = app.node_info.as_ref();

    let epoch = info
        .and_then(|v| v.get("epoch")).and_then(|v| v.as_u64())
        .map(|e| e.to_string()).unwrap_or_else(|| "—".to_string());
    let peer_count = info
        .and_then(|v| v.get("peer_count")).and_then(|v| v.as_u64())
        .map(|e| e.to_string()).unwrap_or_else(|| "—".to_string());
    let chain_id = info
        .and_then(|v| v.get("chain_id")).and_then(|v| v.as_str())
        .unwrap_or("—").to_string();
    let version = info
        .and_then(|v| v.get("version")).and_then(|v| v.as_str())
        .unwrap_or("—").to_string();
    let block_hash = info
        .and_then(|v| v.get("block_hash")).and_then(|v| v.as_str())
        .unwrap_or("—").to_string();

    let rows = vec![
        Row::new(vec![Cell::from("Epoch"), Cell::from(epoch)]),
        Row::new(vec![Cell::from("Peers"), Cell::from(peer_count)]),
        Row::new(vec![Cell::from("Chain ID"), Cell::from(chain_id)]),
        Row::new(vec![Cell::from("Version"), Cell::from(version)]),
        Row::new(vec![Cell::from("Block Hash"), Cell::from(block_hash)]),
    ];

    let table = Table::new(rows, [Constraint::Length(12), Constraint::Min(20)])
        .block(Block::default().borders(Borders::ALL).title("Node Status"))
        .header(
            Row::new(vec!["Field", "Value"])
                .style(Style::default().add_modifier(Modifier::BOLD)),
        )
        .highlight_style(Style::default().fg(Color::Yellow));

    f.render_widget(table, area);
}

// ── Wallet tab ────────────────────────────────────────────────────────────────

fn render_wallet_tab(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(3)])
        .split(area);

    let content = if let Some(ref session) = app.session {
        let balance_str = app.wallet_balance
            .map(dreams_to_btcpc)
            .unwrap_or_else(|| "—".to_string());
        let staked_str = app.wallet_staked
            .map(dreams_to_btcpc)
            .unwrap_or_else(|| "—".to_string());
        format!(
            "Account:  {}\nBalance:  {} BTCPC\nStaked:   {} BTCPC",
            session.account, balance_str, staked_str
        )
    } else {
        "Not logged in.\n\nRun: btcpc login --account <name>".to_string()
    };

    let paragraph = Paragraph::new(content)
        .block(Block::default().borders(Borders::ALL).title("Wallet"))
        .style(Style::default().fg(Color::White));
    f.render_widget(paragraph, chunks[0]);

    // Help text (only in Normal mode)
    if matches!(app.mode, Mode::Normal) && app.session.is_some() {
        let help = Paragraph::new(Line::from(vec![
            Span::styled("t", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            Span::raw(" transfer   "),
            Span::styled("a", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            Span::raw(" add stake   "),
            Span::styled("x", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
            Span::raw(" remove stake"),
        ]))
        .block(Block::default().borders(Borders::ALL));
        f.render_widget(help, chunks[1]);
    } else if matches!(app.mode, Mode::Normal) {
        let help = Paragraph::new("Not logged in — run: btcpc login --account <name>")
            .block(Block::default().borders(Borders::ALL))
            .style(Style::default().fg(Color::DarkGray));
        f.render_widget(help, chunks[1]);
    }
}

// ── Explorer tab ──────────────────────────────────────────────────────────────

fn render_explorer_tab(f: &mut Frame, app: &App, area: Rect) {
    let rows: Vec<Row> = app.blocks.iter().map(|b| {
        let epoch = b.get("epoch").and_then(|v| v.as_u64())
            .map(|e| e.to_string()).unwrap_or_else(|| "—".to_string());
        let hash = b.get("hash").and_then(|v| v.as_str())
            .map(|h| truncate(h, 16).to_string()).unwrap_or_else(|| "—".to_string());
        let entries = b.get("entry_count").and_then(|v| v.as_u64())
            .map(|e| e.to_string()).unwrap_or_else(|| "—".to_string());
        let ts = b.get("timestamp_ms").and_then(|v| v.as_u64())
            .map(|t| t.to_string()).unwrap_or_else(|| "—".to_string());
        Row::new(vec![epoch, hash, entries, ts])
    }).collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(8),
            Constraint::Length(18),
            Constraint::Length(8),
            Constraint::Min(14),
        ],
    )
    .block(Block::default().borders(Borders::ALL).title("Recent Blocks"))
    .header(
        Row::new(vec!["Epoch", "Hash", "Entries", "Timestamp"])
            .style(Style::default().add_modifier(Modifier::BOLD)),
    );

    f.render_widget(table, area);
}

// ── Inference tab ─────────────────────────────────────────────────────────────

fn render_inference_tab(f: &mut Frame, app: &App, area: Rect) {
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints([Constraint::Min(1), Constraint::Length(3)])
        .split(area);

    let rows: Vec<Row> = app.jobs.iter().map(|j| {
        let id = j.get("job_id").or_else(|| j.get("id"))
            .and_then(|v| v.as_str())
            .map(|s| truncate(s, 12).to_string())
            .unwrap_or_else(|| "—".to_string());
        let model = j.get("model").and_then(|v| v.as_str())
            .unwrap_or("—").to_string();
        let status = j.get("status").and_then(|v| v.as_str())
            .unwrap_or("—").to_string();
        let fee = j.get("max_fee").and_then(|v| v.as_u64())
            .map(dreams_to_btcpc).unwrap_or_else(|| "—".to_string());
        let epoch = j.get("epoch").or_else(|| j.get("deadline_epoch"))
            .and_then(|v| v.as_u64())
            .map(|e| e.to_string()).unwrap_or_else(|| "—".to_string());
        Row::new(vec![id, model, status, fee, epoch])
    }).collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(14),
            Constraint::Min(20),
            Constraint::Length(10),
            Constraint::Length(14),
            Constraint::Length(8),
        ],
    )
    .block(Block::default().borders(Borders::ALL).title("Inference Jobs"))
    .header(
        Row::new(vec!["ID", "Model", "Status", "Fee BTCPC", "Epoch"])
            .style(Style::default().add_modifier(Modifier::BOLD)),
    );

    f.render_widget(table, chunks[0]);

    // Help text
    if matches!(app.mode, Mode::Normal) {
        let help_text = if app.session.is_some() {
            Line::from(vec![
                Span::styled("n", Style::default().fg(Color::Yellow).add_modifier(Modifier::BOLD)),
                Span::raw(" post new job"),
            ])
        } else {
            Line::from(Span::styled(
                "Not logged in — run: btcpc login --account <name>",
                Style::default().fg(Color::DarkGray),
            ))
        };
        let help = Paragraph::new(help_text)
            .block(Block::default().borders(Borders::ALL));
        f.render_widget(help, chunks[1]);
    }
}

// ── Footer ────────────────────────────────────────────────────────────────────

fn render_footer(f: &mut Frame, app: &App, area: Rect) {
    let account = app.session.as_ref().map(|s| s.account.as_str()).unwrap_or("none");
    let url = app.node_url();
    let status = if app.status_msg.is_empty() {
        String::new()
    } else {
        format!(" | {}", app.status_msg)
    };
    let text = format!(
        " q quit  r refresh  1-4 tabs | Node: {} | Account: {}{}",
        url, account, status
    );
    let paragraph = Paragraph::new(Line::from(vec![Span::styled(
        text,
        Style::default().fg(Color::DarkGray),
    )]))
    .block(Block::default().borders(Borders::ALL));
    f.render_widget(paragraph, area);
}

// ── Form rendering helpers ────────────────────────────────────────────────────

fn centered_rect(percent_x: u16, min_height: u16, area: Rect) -> Rect {
    let popup_width = (area.width as u32 * percent_x as u32 / 100) as u16;
    let x = area.x + (area.width.saturating_sub(popup_width)) / 2;
    let popup_height = min_height;
    let y = area.y + (area.height.saturating_sub(popup_height)) / 2;
    Rect {
        x,
        y: y.min(area.y + area.height.saturating_sub(popup_height)),
        width: popup_width.min(area.width),
        height: popup_height.min(area.height),
    }
}

fn render_form_fields(
    f: &mut Frame,
    title: &str,
    fields: &[(&str, &str)],
    focused: usize,
    hint: &str,
    area: Rect,
) {
    // +4: title border top/bottom + hint line + blank line
    let height = (fields.len() as u16 * 3) + 4;
    let popup = centered_rect(70, height, area);

    f.render_widget(Clear, popup);

    let outer_block = Block::default()
        .borders(Borders::ALL)
        .title(Span::styled(title, Style::default().fg(Color::Cyan).add_modifier(Modifier::BOLD)));
    f.render_widget(outer_block.clone(), popup);

    let inner = outer_block.inner(popup);

    // Split inner area: field rows + hint
    let mut constraints: Vec<Constraint> = fields.iter().map(|_| Constraint::Length(3)).collect();
    constraints.push(Constraint::Min(1)); // hint
    let field_chunks = Layout::default()
        .direction(Direction::Vertical)
        .constraints(constraints)
        .split(inner);

    for (i, (label, value)) in fields.iter().enumerate() {
        let is_focused = i == focused;
        let display_value = if is_focused {
            format!("{}_", value)
        } else {
            value.to_string()
        };
        let border_style = if is_focused {
            Style::default().fg(Color::Yellow)
        } else {
            Style::default().fg(Color::DarkGray)
        };
        let value_style = if is_focused {
            Style::default().fg(Color::Yellow)
        } else {
            Style::default().fg(Color::White)
        };
        let field_widget = Paragraph::new(Span::styled(display_value, value_style))
            .block(
                Block::default()
                    .borders(Borders::ALL)
                    .border_style(border_style)
                    .title(*label),
            );
        if i < field_chunks.len().saturating_sub(1) {
            f.render_widget(field_widget, field_chunks[i]);
        }
    }

    // Hint line
    if let Some(hint_area) = field_chunks.last() {
        let hint_widget = Paragraph::new(Line::from(vec![Span::styled(
            hint,
            Style::default().fg(Color::DarkGray),
        )]))
        .alignment(Alignment::Center);
        f.render_widget(hint_widget, *hint_area);
    }
}

// ── Individual form renders ───────────────────────────────────────────────────

fn render_transfer_form(f: &mut Frame, area: Rect, state: &crate::app::TransferState) {
    let fields = state.field_values();
    render_form_fields(
        f,
        " Transfer BTCPC ",
        &fields,
        state.field,
        "Tab next field   Enter submit   Esc cancel",
        area,
    );
}

fn render_stake_form(f: &mut Frame, area: Rect, state: &crate::app::StakeState) {
    let title = if state.action == StakeAction::Add {
        " Add Stake "
    } else {
        " Remove Stake "
    };
    let fields = state.field_values();
    render_form_fields(
        f,
        title,
        &fields,
        state.field,
        "Enter submit   Esc cancel",
        area,
    );
}

fn render_post_job_form(f: &mut Frame, area: Rect, state: &crate::app::PostJobState) {
    let fields = state.field_values();
    render_form_fields(
        f,
        " Post Inference Job ",
        &fields,
        state.field,
        "Tab next field   Enter submit   Esc cancel",
        area,
    );
}

// ── Result overlay ────────────────────────────────────────────────────────────

fn render_result(f: &mut Frame, area: Rect, msg: &str, success: bool) {
    let popup = centered_rect(60, 7, area);
    f.render_widget(Clear, popup);

    let color = if success { Color::Green } else { Color::Red };
    let title = if success { " Success " } else { " Error " };

    let block = Block::default()
        .borders(Borders::ALL)
        .border_style(Style::default().fg(color))
        .title(Span::styled(title, Style::default().fg(color).add_modifier(Modifier::BOLD)));

    let inner = block.inner(popup);
    f.render_widget(block, popup);

    let lines = vec![
        Line::from(Span::styled(msg, Style::default().fg(color))),
        Line::from(""),
        Line::from(Span::styled(
            "Press any key to continue",
            Style::default().fg(Color::DarkGray),
        )),
    ];
    let para = Paragraph::new(lines).alignment(Alignment::Center);
    f.render_widget(para, inner);
}
