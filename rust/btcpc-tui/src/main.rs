mod api;
mod app;
mod sign;
mod ui;

use anyhow::Result;
use app::{LoginState, Mode, PostJobState, StakeAction, StakeState, TransferState};
use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{Terminal, backend::CrosstermBackend};
use std::io;

fn main() -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_app(&mut terminal);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;

    if let Err(e) = result {
        eprintln!("Error: {}", e);
    }

    Ok(())
}

fn run_app(terminal: &mut Terminal<CrosstermBackend<io::Stdout>>) -> Result<()> {
    let mut app = app::App::new();

    loop {
        terminal.draw(|f| ui::render(f, &app))?;

        if event::poll(std::time::Duration::from_millis(250))? {
            if let Event::Key(key) = event::read()? {
                match &app.mode.clone() {
                    Mode::Login(_) => {
                        if handle_login_keys(key, &mut app) {
                            break;
                        }
                    }
                    Mode::Normal => {
                        if handle_normal_keys(key, &mut app) {
                            break;
                        }
                    }
                    Mode::TransferForm(_) | Mode::StakeForm(_) | Mode::PostJobForm(_) => {
                        handle_form_keys(key, &mut app);
                    }
                    Mode::Result { .. } => {
                        app.mode = Mode::Normal;
                    }
                }
            }
        }

        // Auto-refresh (only in Normal mode to avoid clobbering form state)
        if matches!(app.mode, Mode::Normal) {
            if app.last_refresh.elapsed() >= app.refresh_interval {
                app.status_msg.clear();
                app.refresh();
            }
        }
    }

    Ok(())
}

/// Returns true if the loop should break (quit).
fn handle_login_keys(key: event::KeyEvent, app: &mut app::App) -> bool {
    match key.code {
        KeyCode::Char('c') if key.modifiers.contains(KeyModifiers::CONTROL) => return true,
        KeyCode::Tab | KeyCode::Down => advance_field(app, 1),
        KeyCode::BackTab | KeyCode::Up => advance_field(app, -1),
        KeyCode::Backspace => pop_current_field(app),
        KeyCode::Enter => submit_login(app),
        KeyCode::Char(c) => {
            if !key.modifiers.contains(KeyModifiers::CONTROL)
                && !key.modifiers.contains(KeyModifiers::ALT)
            {
                push_current_field(app, c);
            }
        }
        _ => {}
    }
    false
}

fn set_login_error(app: &mut app::App, msg: String) {
    if let Mode::Login(s) = &mut app.mode {
        s.error = Some(msg);
    }
}

fn submit_login(app: &mut app::App) {
    let (account, key_file_str, node_url) = match &app.mode {
        Mode::Login(s) => (s.account.trim().to_owned(), s.key_file.trim().to_owned(), s.node_url.trim().to_owned()),
        _ => return,
    };

    if account.is_empty() {
        set_login_error(app, "Account name is required".into());
        return;
    }

    let key_path = std::path::PathBuf::from(&key_file_str);
    let keypair = match btcpc_sdk::KeyPair::from_file(&key_path) {
        Err(e) => {
            set_login_error(app, format!("Cannot read key file: {}", e));
            return;
        }
        Ok(k) => k,
    };

    let node_url = if node_url.is_empty() { "http://localhost:4242".to_owned() } else { node_url };

    // Fetch the account's registered public key from the node and compare.
    // This proves the key file matches the account — without this check anyone
    // could type any account name and point to any key file.
    match api::get_json(&node_url, &format!("/api/account/{}", account)) {
        Err(e) => {
            set_login_error(app, format!("Cannot verify account (node unreachable?): {}", e));
            return;
        }
        Ok(v) => {
            let registered = v
                .get("public_key")
                .and_then(|k| k.as_str())
                .unwrap_or("");
            if registered.is_empty() {
                // Account exists but has no key registered yet — allow login
                // so the user can still register their key via `btcpc key register`
            } else if registered != keypair.public_key_hex() {
                set_login_error(app, "Key does not match the registered key for this account".into());
                return;
            }
        }
    }
    let session = app::Session {
        account: account.clone(),
        key_file: key_path,
        node_url: node_url.clone(),
    };
    if let Err(e) = app::save_session(&session) {
        if let Mode::Login(s) = &mut app.mode {
            s.error = Some(format!("Failed to save session: {}", e));
        }
        return;
    }

    app.session = Some(session);
    app.mode = Mode::Normal;
    app.refresh();
}

/// Returns true if the loop should break (quit).
fn handle_normal_keys(key: event::KeyEvent, app: &mut app::App) -> bool {
    match (key.code, key.modifiers) {
        (KeyCode::Char('q'), _)
        | (KeyCode::Esc, _)
        | (KeyCode::Char('c'), KeyModifiers::CONTROL) => {
            return true;
        }
        (KeyCode::Char('r'), _) => {
            app.status_msg.clear();
            app.refresh();
        }
        (KeyCode::Char('1'), _) => app.tab = 0,
        (KeyCode::Char('2'), _) => app.tab = 1,
        (KeyCode::Char('3'), _) => app.tab = 2,
        (KeyCode::Char('4'), _) => app.tab = 3,

        // Wallet tab actions (only when logged in)
        (KeyCode::Char('t'), _) if app.tab == 1 => {
            if app.session.is_some() {
                app.mode = Mode::TransferForm(TransferState::new());
            }
        }
        (KeyCode::Char('a'), _) if app.tab == 1 => {
            if app.session.is_some() {
                app.mode = Mode::StakeForm(StakeState::new(StakeAction::Add));
            }
        }
        (KeyCode::Char('x'), _) if app.tab == 1 => {
            if app.session.is_some() {
                app.mode = Mode::StakeForm(StakeState::new(StakeAction::Remove));
            }
        }

        // Inference tab actions (only when logged in)
        (KeyCode::Char('n'), _) if app.tab == 3 => {
            if app.session.is_some() {
                app.mode = Mode::PostJobForm(PostJobState::new());
            }
        }

        _ => {}
    }
    false
}

fn handle_form_keys(key: event::KeyEvent, app: &mut app::App) {
    match key.code {
        KeyCode::Esc => {
            app.mode = Mode::Normal;
        }

        KeyCode::Tab | KeyCode::Down => {
            advance_field(app, 1);
        }
        KeyCode::BackTab | KeyCode::Up => {
            advance_field(app, -1);
        }

        KeyCode::Backspace => {
            pop_current_field(app);
        }

        KeyCode::Enter => {
            submit_form(app);
        }

        KeyCode::Char(c) => {
            // Filter to printable chars only
            if !key.modifiers.contains(KeyModifiers::CONTROL)
                && !key.modifiers.contains(KeyModifiers::ALT)
            {
                push_current_field(app, c);
            }
        }

        _ => {}
    }
}

fn advance_field(app: &mut app::App, delta: i32) {
    match &mut app.mode {
        Mode::Login(s) => {
            let count = LoginState::field_count() as i32;
            s.field = ((s.field as i32 + delta).rem_euclid(count)) as usize;
        }
        Mode::TransferForm(s) => {
            let count = TransferState::field_count() as i32;
            s.field = ((s.field as i32 + delta).rem_euclid(count)) as usize;
        }
        Mode::StakeForm(s) => {
            let count = StakeState::field_count() as i32;
            s.field = ((s.field as i32 + delta).rem_euclid(count)) as usize;
        }
        Mode::PostJobForm(s) => {
            let count = PostJobState::field_count() as i32;
            s.field = ((s.field as i32 + delta).rem_euclid(count)) as usize;
        }
        _ => {}
    }
}

fn pop_current_field(app: &mut app::App) {
    match &mut app.mode {
        Mode::Login(s)        => { s.current_field_mut().pop(); }
        Mode::TransferForm(s) => { s.current_field_mut().pop(); }
        Mode::StakeForm(s)    => { s.current_field_mut().pop(); }
        Mode::PostJobForm(s)  => { s.current_field_mut().pop(); }
        _ => {}
    }
}

fn push_current_field(app: &mut app::App, c: char) {
    match &mut app.mode {
        Mode::Login(s)        => { s.current_field_mut().push(c); }
        Mode::TransferForm(s) => { s.current_field_mut().push(c); }
        Mode::StakeForm(s)    => { s.current_field_mut().push(c); }
        Mode::PostJobForm(s)  => { s.current_field_mut().push(c); }
        _ => {}
    }
}

/// Parse a BTCPC decimal string to dreams (u64). 1 BTCPC = 10^10 dreams.
fn parse_btcpc_amount(s: &str) -> Result<u64, String> {
    let trimmed = s.trim();
    if trimmed.is_empty() {
        return Err("amount is empty".to_string());
    }
    match trimmed.parse::<f64>() {
        Ok(f) if f < 0.0 => Err("amount must be positive".to_string()),
        Ok(f) => Ok((f * 10_000_000_000.0) as u64),
        Err(_) => Err(format!("invalid amount: '{}'", trimmed)),
    }
}

fn submit_form(app: &mut app::App) {
    let session = match app.session.clone() {
        Some(s) => s,
        None => {
            app.mode = Mode::Result {
                msg: "Not logged in".to_string(),
                success: false,
            };
            return;
        }
    };

    let base = app.node_url();

    match app.mode.clone() {
        Mode::TransferForm(state) => {
            let amount_dreams = match parse_btcpc_amount(&state.amount) {
                Ok(a) => a,
                Err(e) => {
                    app.mode = Mode::Result { msg: e, success: false };
                    return;
                }
            };
            let result = sign::submit_transfer(
                &base,
                session.key_file.as_path(),
                &session.account,
                &state.to,
                amount_dreams,
                &state.memo,
            );
            match result {
                Ok(msg) => {
                    app.mode = Mode::Result { msg, success: true };
                    app.refresh();
                }
                Err(e) => {
                    app.mode = Mode::Result { msg: e.to_string(), success: false };
                }
            }
        }

        Mode::StakeForm(state) => {
            let amount_dreams = match parse_btcpc_amount(&state.amount) {
                Ok(a) => a,
                Err(e) => {
                    app.mode = Mode::Result { msg: e, success: false };
                    return;
                }
            };
            let add = state.action == StakeAction::Add;
            let result = sign::submit_stake(
                &base,
                session.key_file.as_path(),
                &session.account,
                amount_dreams,
                add,
            );
            match result {
                Ok(msg) => {
                    app.mode = Mode::Result { msg, success: true };
                    app.refresh();
                }
                Err(e) => {
                    app.mode = Mode::Result { msg: e.to_string(), success: false };
                }
            }
        }

        Mode::PostJobForm(state) => {
            let max_fee = match parse_btcpc_amount(&state.max_fee) {
                Ok(a) => a,
                Err(e) => {
                    app.mode = Mode::Result { msg: e, success: false };
                    return;
                }
            };
            let deadline: u64 = match state.deadline.trim().parse() {
                Ok(d) => d,
                Err(_) => {
                    app.mode = Mode::Result {
                        msg: format!("invalid deadline epoch: '{}'", state.deadline),
                        success: false,
                    };
                    return;
                }
            };
            let result = sign::submit_post_job(
                &base,
                session.key_file.as_path(),
                &session.account,
                &state.model,
                &state.input,
                max_fee,
                deadline,
            );
            match result {
                Ok(msg) => {
                    app.mode = Mode::Result { msg, success: true };
                    app.refresh();
                }
                Err(e) => {
                    app.mode = Mode::Result { msg: e.to_string(), success: false };
                }
            }
        }

        _ => {}
    }
}
