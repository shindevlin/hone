use ratatui::{
    Frame,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, Tabs},
};

use crate::app::App;

fn dreams_to_btcpc(dreams: u64) -> String {
    format!("{:.8}", dreams as f64 / 100_000_000.0)
}

fn truncate(s: &str, n: usize) -> &str {
    if s.len() <= n {
        s
    } else {
        &s[..n]
    }
}

pub fn render(f: &mut Frame, app: &App) {
    let size = f.area();

    // Overall vertical layout: tabs bar, content, footer
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
            Style::default()
                .fg(Color::Yellow)
                .add_modifier(Modifier::BOLD),
        );
    f.render_widget(tabs, area);
}

fn render_body(f: &mut Frame, app: &App, area: Rect) {
    match app.tab {
        0 => render_node_tab(f, app, area),
        1 => render_wallet_tab(f, app, area),
        2 => render_explorer_tab(f, app, area),
        3 => render_inference_tab(f, app, area),
        _ => {}
    }
}

fn render_node_tab(f: &mut Frame, app: &App, area: Rect) {
    let info = app.node_info.as_ref();

    let epoch = info
        .and_then(|v| v.get("epoch"))
        .and_then(|v| v.as_u64())
        .map(|e| e.to_string())
        .unwrap_or_else(|| "—".to_string());

    let peer_count = info
        .and_then(|v| v.get("peer_count"))
        .and_then(|v| v.as_u64())
        .map(|e| e.to_string())
        .unwrap_or_else(|| "—".to_string());

    let chain_id = info
        .and_then(|v| v.get("chain_id"))
        .and_then(|v| v.as_str())
        .unwrap_or("—")
        .to_string();

    let version = info
        .and_then(|v| v.get("version"))
        .and_then(|v| v.as_str())
        .unwrap_or("—")
        .to_string();

    let block_hash = info
        .and_then(|v| v.get("block_hash"))
        .and_then(|v| v.as_str())
        .unwrap_or("—")
        .to_string();

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

fn render_wallet_tab(f: &mut Frame, app: &App, area: Rect) {
    let content = if let Some(ref session) = app.session {
        let balance_str = app
            .wallet_balance
            .map(dreams_to_btcpc)
            .unwrap_or_else(|| "—".to_string());
        let staked_str = app
            .wallet_staked
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
    f.render_widget(paragraph, area);
}

fn render_explorer_tab(f: &mut Frame, app: &App, area: Rect) {
    let rows: Vec<Row> = app
        .blocks
        .iter()
        .map(|b| {
            let epoch = b
                .get("epoch")
                .and_then(|v| v.as_u64())
                .map(|e| e.to_string())
                .unwrap_or_else(|| "—".to_string());
            let hash = b
                .get("hash")
                .and_then(|v| v.as_str())
                .map(|h| truncate(h, 16).to_string())
                .unwrap_or_else(|| "—".to_string());
            let entries = b
                .get("entry_count")
                .and_then(|v| v.as_u64())
                .map(|e| e.to_string())
                .unwrap_or_else(|| "—".to_string());
            let ts = b
                .get("timestamp_ms")
                .and_then(|v| v.as_u64())
                .map(|t| t.to_string())
                .unwrap_or_else(|| "—".to_string());
            Row::new(vec![epoch, hash, entries, ts])
        })
        .collect();

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

fn render_inference_tab(f: &mut Frame, app: &App, area: Rect) {
    let rows: Vec<Row> = app
        .jobs
        .iter()
        .map(|j| {
            let id = j
                .get("id")
                .and_then(|v| v.as_str())
                .map(|s| truncate(s, 12).to_string())
                .unwrap_or_else(|| "—".to_string());
            let model = j
                .get("model")
                .and_then(|v| v.as_str())
                .unwrap_or("—")
                .to_string();
            let status = j
                .get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("—")
                .to_string();
            let fee = j
                .get("max_fee")
                .and_then(|v| v.as_u64())
                .map(dreams_to_btcpc)
                .unwrap_or_else(|| "—".to_string());
            Row::new(vec![id, model, status, fee])
        })
        .collect();

    let table = Table::new(
        rows,
        [
            Constraint::Length(14),
            Constraint::Min(20),
            Constraint::Length(10),
            Constraint::Length(14),
        ],
    )
    .block(Block::default().borders(Borders::ALL).title("Inference Jobs"))
    .header(
        Row::new(vec!["ID", "Model", "Status", "Fee (BTCPC)"])
            .style(Style::default().add_modifier(Modifier::BOLD)),
    );

    f.render_widget(table, area);
}

fn render_footer(f: &mut Frame, app: &App, area: Rect) {
    let account = app
        .session
        .as_ref()
        .map(|s| s.account.as_str())
        .unwrap_or("none");
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
