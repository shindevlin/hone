mod api;
mod app;
mod sign;
mod ui;

use anyhow::Result;
use app::{LoginState, Mode, PostJobState, RoleStakeState, StakeAction, StakeState, TransferState};
use crossterm::{
    event::{self, Event, KeyCode, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
    event::{EnableBracketedPaste, DisableBracketedPaste},
};
use ratatui::{Terminal, backend::CrosstermBackend};
use std::io;

fn main() -> Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableBracketedPaste)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let result = run_app(&mut terminal);

    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen, DisableBracketedPaste)?;
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
            match event::read()? {
                Event::Paste(s) => paste_current_field(&mut app, &s),
                Event::Key(key) => {
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
                        Mode::TransferForm(_) | Mode::StakeForm(_) | Mode::RoleStakeForm(_) | Mode::PostJobForm(_) => {
                            handle_form_keys(key, &mut app);
                        }
                        Mode::Result { .. } => {
                            app.mode = Mode::Normal;
                        }
                    }
                }
                _ => {}
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

    // If key file was left blank, try common locations derived from the account name
    let key_file_str = if key_file_str.is_empty() {
        let home = std::env::var("HOME").unwrap_or_else(|_| ".".into());
        let data_dir = std::env::var("HONE_DATA_DIR").unwrap_or_default();
        let candidates: Vec<String> = [
            format!("{}/.hone/{}.wallet.key", home, account),
            format!("{}/.hone/wallet.key", home),
            format!("{}/wallet.key", data_dir),
            format!("{}/.hone/key.json", home),
        ]
        .into_iter()
        .filter(|p| !p.starts_with('/') || !p.trim_start_matches('/').is_empty())
        .collect();
        match candidates.iter().find(|p| !p.is_empty() && std::path::Path::new(p.as_str()).exists()) {
            Some(p) => {
                // Show the user what was found
                if let Mode::Login(s) = &mut app.mode {
                    s.key_file = p.clone();
                }
                p.clone()
            }
            None => {
                set_login_error(app, format!(
                    "No key file found — tried ~/.hone/{}.wallet.key, wallet.key, key.json",
                    account
                ));
                return;
            }
        }
    } else {
        key_file_str
    };

    let key_path = std::path::PathBuf::from(&key_file_str);
    let node_url = if node_url.is_empty() { "http://localhost:4242".to_owned() } else { node_url };

    // Fetch on-chain keys first so we can figure out which role this file covers.
    let account_data = match api::get_json(&node_url, &format!("/api/account/{}", account)) {
        Err(e) => {
            set_login_error(app, format!("Cannot reach node to verify account: {}", e));
            return;
        }
        Ok(v) => v,
    };

    let on_chain_keys = account_data.get("keys");

    // If account has no registered keys yet, allow login so they can register.
    let key_role = if on_chain_keys.is_none() {
        match sign::load_keypair(&key_path, "active") {
            Ok(_) => app::KeyRole::Active,
            Err(e) => {
                set_login_error(app, format!("Cannot read key file: {}", e));
                return;
            }
        }
    } else {
        // Build a fresh timestamp challenge.
        let ts = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let challenge = format!("hone-auth:{}:{}", account, ts);

        use ed25519_dalek::{Signature, Verifier, VerifyingKey};

        // Try each role: load the keypair for that role from the file, sign the
        // challenge, and verify it against the on-chain key for that role.
        // First match wins; active is tried first.
        let candidates: &[(&str, app::KeyRole)] = &[
            ("active",  app::KeyRole::Active),
            ("posting", app::KeyRole::Posting),
            ("owner",   app::KeyRole::Owner),
        ];

        let mut matched_role: Option<app::KeyRole> = None;
        for (role_name, role_variant) in candidates {
            let on_chain_pubkey = on_chain_keys
                .and_then(|k| k.get(*role_name))
                .and_then(|v| v.as_str())
                .unwrap_or("");
            if on_chain_pubkey.is_empty() { continue; }

            let kp = match sign::load_keypair(&key_path, role_name) {
                Ok(k) => k,
                Err(_) => continue,
            };

            let sig_hex = kp.sign_bytes(challenge.as_bytes());
            let ok = (|| -> Option<()> {
                let pk: [u8; 32] = hex::decode(on_chain_pubkey).ok()?.try_into().ok()?;
                let vk = VerifyingKey::from_bytes(&pk).ok()?;
                let sb: [u8; 64] = hex::decode(&sig_hex).ok()?.try_into().ok()?;
                let sig = Signature::from_bytes(&sb);
                vk.verify(challenge.as_bytes(), &sig).ok()?;
                Some(())
            })().is_some();

            if ok {
                matched_role = Some(role_variant.clone());
                break;
            }
        }

        match matched_role {
            Some(r) => r,
            None => {
                set_login_error(app,
                    "Key file does not match any registered key for this account (tried active, posting, owner)".into());
                return;
            }
        }
    };
    let session = app::Session {
        account: account.clone(),
        key_file: key_path,
        node_url: node_url.clone(),
        key_role: key_role.clone(),
    };
    if let Err(e) = app::save_session(&session) {
        if let Mode::Login(s) = &mut app.mode {
            s.error = Some(format!("Failed to save session: {}", e));
        }
        return;
    }

    let role_str = key_role.to_string();
    app.eth_address = app::read_eth_address(&session.key_file);
    app.session = Some(session);
    app.mode = Mode::Normal;
    app.refresh();
    app.status_msg = format!("Signed in as {} ({})", account, role_str);
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
        // Tab 0=Wallet, 1=Staking, 2=Explorer, 3=Inference
        (KeyCode::Char('1'), _) => app.tab = 0,
        (KeyCode::Char('2'), _) => app.tab = 1,
        (KeyCode::Char('3'), _) => app.tab = 2,
        (KeyCode::Char('4'), _) => app.tab = 3,

        // Wallet tab actions — require active key
        (KeyCode::Char('t'), _) if app.tab == 0 => {
            match &app.session {
                Some(s) if s.can_spend_tokens() => app.mode = Mode::TransferForm(TransferState::new()),
                Some(_) => app.status_msg = "Transfer requires the active key".into(),
                None => {}
            }
        }
        (KeyCode::Char('a'), _) if app.tab == 0 => {
            match &app.session {
                Some(s) if s.can_spend_tokens() => app.mode = Mode::StakeForm(StakeState::new(StakeAction::Add)),
                Some(_) => app.status_msg = "Staking requires the active key".into(),
                None => {}
            }
        }
        (KeyCode::Char('x'), _) if app.tab == 0 => {
            match &app.session {
                Some(s) if s.can_spend_tokens() => app.mode = Mode::StakeForm(StakeState::new(StakeAction::Remove)),
                Some(_) => app.status_msg = "Unstaking requires the active key".into(),
                None => {}
            }
        }

        // Staking tab actions — require active key
        (KeyCode::Char('s'), _) if app.tab == 1 => {
            match &app.session {
                Some(s) if s.can_spend_tokens() => app.mode = Mode::RoleStakeForm(RoleStakeState::new(true)),
                Some(_) => app.status_msg = "Role staking requires the active key".into(),
                None => {}
            }
        }
        (KeyCode::Char('u'), _) if app.tab == 1 => {
            match &app.session {
                Some(s) if s.can_spend_tokens() => app.mode = Mode::RoleStakeForm(RoleStakeState::new(false)),
                Some(_) => app.status_msg = "Role unstaking requires the active key".into(),
                None => {}
            }
        }

        // Inference tab actions — require active key
        (KeyCode::Char('n'), _) if app.tab == 3 => {
            match &app.session {
                Some(s) if s.can_spend_tokens() => app.mode = Mode::PostJobForm(PostJobState::new()),
                Some(_) => app.status_msg = "Posting inference jobs requires the active key".into(),
                None => {}
            }
        }

        _ => {}
    }
    false
}

fn handle_form_keys(key: event::KeyEvent, app: &mut app::App) {
    // ←/→ cycle role when on field 1 of RoleStakeForm
    if matches!(key.code, KeyCode::Left | KeyCode::Right) {
        if let Mode::RoleStakeForm(ref mut s) = app.mode {
            if s.field == 1 {
                s.cycle_role(key.code == KeyCode::Right);
                return;
            }
        }
    }

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
        Mode::RoleStakeForm(s) => {
            let count = RoleStakeState::field_count() as i32;
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
        Mode::Login(s)          => { s.current_field_mut().pop(); }
        Mode::TransferForm(s)   => { s.current_field_mut().pop(); }
        Mode::StakeForm(s)      => { s.current_field_mut().pop(); }
        Mode::RoleStakeForm(s)  => { if let Some(f) = s.current_text_field_mut() { f.pop(); } }
        Mode::PostJobForm(s)    => { s.current_field_mut().pop(); }
        _ => {}
    }
}

fn push_current_field(app: &mut app::App, c: char) {
    match &mut app.mode {
        Mode::Login(s)          => { s.current_field_mut().push(c); }
        Mode::TransferForm(s)   => { s.current_field_mut().push(c); }
        Mode::StakeForm(s)      => { s.current_field_mut().push(c); }
        Mode::RoleStakeForm(s)  => { if let Some(f) = s.current_text_field_mut() { f.push(c); } }
        Mode::PostJobForm(s)    => { s.current_field_mut().push(c); }
        _ => {}
    }
}

fn paste_current_field(app: &mut app::App, text: &str) {
    match &mut app.mode {
        Mode::Login(s)          => s.current_field_mut().push_str(text),
        Mode::TransferForm(s)   => s.current_field_mut().push_str(text),
        Mode::StakeForm(s)      => s.current_field_mut().push_str(text),
        Mode::RoleStakeForm(s)  => { if let Some(f) = s.current_text_field_mut() { f.push_str(text); } }
        Mode::PostJobForm(s)    => s.current_field_mut().push_str(text),
        _ => {}
    }
}

/// Parse a HONE decimal string to hunits (u64). 1 HONE = 10^10 hunits.
fn parse_hone_amount(s: &str) -> Result<u64, String> {
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
            let amount_hunits = match parse_hone_amount(&state.amount) {
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
                amount_hunits,
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
            let amount_hunits = match parse_hone_amount(&state.amount) {
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
                amount_hunits,
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

        Mode::RoleStakeForm(state) => {
            if state.node.trim().is_empty() {
                app.mode = Mode::Result { msg: "Node account is required".into(), success: false };
                return;
            }
            let node = {
                let t = state.node.trim();
                if t == "self" { session.account.as_str() } else { t }
            };
            let amount_hunits = match parse_hone_amount(&state.amount) {
                Ok(a) => a,
                Err(e) => { app.mode = Mode::Result { msg: e, success: false }; return; }
            };
            let result = sign::submit_role_stake(
                &base,
                session.key_file.as_path(),
                &session.account,
                node,
                state.role(),
                amount_hunits,
                state.add,
            );
            match result {
                Ok(msg) => { app.mode = Mode::Result { msg, success: true }; app.refresh(); }
                Err(e)  => { app.mode = Mode::Result { msg: e.to_string(), success: false }; }
            }
        }

        Mode::PostJobForm(state) => {
            let max_fee = match parse_hone_amount(&state.max_fee) {
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
