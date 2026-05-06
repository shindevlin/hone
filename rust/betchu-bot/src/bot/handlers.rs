use crate::{
    AppState,
    bot::state::{BetDialogue, BetState},
    bot::commands::BetchuCommand,
    contract::{eth_to_wei, BetPool},
    oracle::espn::{find_matching_game},
};
use alloy::primitives::U256;
use teloxide::{
    prelude::*,
    types::{
        InlineKeyboardButton, InlineKeyboardMarkup, KeyboardButton, KeyboardMarkup,
        WebAppInfo, ParseMode,
    },
};
use tracing::{info, warn};

type HandlerResult = Result<(), Box<dyn std::error::Error + Send + Sync>>;

/// Duration to add to game start_epoch to get the pool deadline (absolute unix timestamp).
fn sport_game_duration(sport: &str) -> u64 {
    match sport {
        "football_nfl" | "football_college" => 4 * 3600,
        "basketball_nba" | "basketball_college" => 3 * 3600,
        _ => 3 * 3600,
    }
}

// ── Slash command dispatcher ─────────────────────────────────────────────────

pub async fn handle_command(
    bot: Bot,
    dialogue: BetDialogue,
    msg: Message,
    cmd: BetchuCommand,
    state: std::sync::Arc<AppState>,
) -> HandlerResult {
    match cmd {
        BetchuCommand::Start => cmd_start(bot, msg, state).await,
        BetchuCommand::Help => cmd_help(bot, msg).await,
        BetchuCommand::Bets => cmd_bets(bot, msg, state).await,
        BetchuCommand::CreateBet => cmd_create_bet(bot, dialogue, msg).await,
        BetchuCommand::Join(arg) => cmd_join(bot, dialogue, msg, arg, state).await,
        BetchuCommand::Cancel => cmd_cancel(bot, dialogue, msg).await,
        BetchuCommand::Me => cmd_me(bot, msg, state).await,
        BetchuCommand::Wallet(arg) => cmd_wallet(bot, msg, arg, state).await,
        BetchuCommand::App => cmd_app(bot, msg, state).await,
        BetchuCommand::Network => cmd_network(bot, msg, state).await,
        BetchuCommand::Scores => cmd_scores(bot, msg, state).await,
    }
}

// ── /start ───────────────────────────────────────────────────────────────────

async fn cmd_start(bot: Bot, msg: Message, state: std::sync::Arc<AppState>) -> HandlerResult {
    let user = msg.from.as_ref();
    let telegram_id = user.map(|u| u.id.0 as i64).unwrap_or(0);
    let username = user.and_then(|u| u.username.as_deref());

    if let Ok(db) = &state.db {
        let _ = db.get_or_create_user(telegram_id, username).await;
    }

    let app_btn = InlineKeyboardButton::web_app(
        "Open BetChu App",
        WebAppInfo { url: state.config.miniapp_url.parse().unwrap() },
    );

    let keyboard = InlineKeyboardMarkup::new(vec![vec![app_btn]]);

    bot.send_message(
        msg.chat.id,
        "Welcome to BetChu!\n\n\
        Peer-to-peer sports betting on Base Sepolia.\n\n\
        /bets — Browse open pools\n\
        /createbet — Start a new bet\n\
        /join <id> — Join a pool\n\
        /me — Your stats & wallet\n\
        /scores — Latest game scores\n\
        /app — Open the Mini App\n\n\
        Powered by BTCPC P2P inference network.",
    )
    .reply_markup(keyboard)
    .await?;
    Ok(())
}

// ── /help ────────────────────────────────────────────────────────────────────

async fn cmd_help(bot: Bot, msg: Message) -> HandlerResult {
    bot.send_message(
        msg.chat.id,
        "/start — Welcome and getting started\n\
        /bets — Browse open betting pools\n\
        /createbet — Create a new pool\n\
        /join <pool_id> — Join an existing pool\n\
        /cancel — Cancel current action\n\
        /me — Profile, wallet, stats\n\
        /wallet <address> — Link EVM wallet\n\
        /scores — Live ESPN scores\n\
        /network — BTCPC node status\n\
        /app — Open BetChu Mini App\n\n\
        Units: 1 ETH = 10^18 wei | Fees: 1% platform + 0.5% creator",
    )
    .await?;
    Ok(())
}

// ── /bets ────────────────────────────────────────────────────────────────────

async fn cmd_bets(bot: Bot, msg: Message, state: std::sync::Arc<AppState>) -> HandlerResult {
    bot.send_message(msg.chat.id, "Fetching open pools...").await?;

    let pools = match state.contract.get_active_pools(10).await {
        Ok(p) => p,
        Err(e) => {
            warn!("Failed to fetch pools: {}", e);
            bot.send_message(msg.chat.id, "Could not load pools. Check your connection.").await?;
            return Ok(());
        }
    };

    if pools.is_empty() {
        let app_btn = InlineKeyboardButton::web_app(
            "Open BetChu App",
            WebAppInfo { url: state.config.miniapp_url.parse().unwrap() },
        );
        bot.send_message(msg.chat.id, "No open pools right now. Be the first to create one with /createbet!")
            .reply_markup(InlineKeyboardMarkup::new(vec![vec![app_btn]]))
            .await?;
        return Ok(());
    }

    let mut text = format!("Open Betting Pools ({})\n\n", pools.len());

    for pool in &pools {
        let currency = pool.currency();
        let hours = pool.hours_left();
        let time_str = if hours < 1.0 {
            format!("{:.0}m left", hours * 60.0)
        } else if hours < 24.0 {
            format!("{:.1}h left", hours)
        } else {
            format!("{:.0}d left", hours / 24.0)
        };

        text.push_str(&format!(
            "#{id} {desc}\n\
            Side A: {a:.3} {sym} | Side B: {b:.3} {sym}\n\
            {time} | {type_name}\n\n",
            id = pool.pool_id,
            desc = truncate(&pool.description, 60),
            a = pool.side_a_pool,
            b = pool.side_b_pool,
            sym = currency.symbol,
            time = time_str,
            type_name = pool.bet_type_name(),
        ));
    }

    // Generate AI insight for top pool if BTCPC is configured
    if let Some(btcpc) = &state.btcpc {
        if let Some(top) = pools.first() {
            if let Ok(insight) = btcpc
                .analyze_pool(
                    top.pool_id,
                    &top.description,
                    top.side_a_pool,
                    top.side_b_pool,
                    top.hours_left(),
                )
                .await
            {
                text.push_str(&format!("AI Insight on #{}: {}\n", top.pool_id, insight));
            }
        }
    }

    let app_btn = InlineKeyboardButton::web_app(
        "Open BetChu App",
        WebAppInfo { url: state.config.miniapp_url.parse().unwrap() },
    );

    bot.send_message(msg.chat.id, text)
        .reply_markup(InlineKeyboardMarkup::new(vec![vec![app_btn]]))
        .await?;
    Ok(())
}

// ── /createbet (starts dialogue) ─────────────────────────────────────────────

async fn cmd_create_bet(bot: Bot, dialogue: BetDialogue, msg: Message) -> HandlerResult {
    dialogue.update(BetState::CreateDescription).await?;
    bot.send_message(
        msg.chat.id,
        "Creating a new bet pool.\n\nDescribe what you're betting on (e.g. 'Alabama beats Georgia'):",
    )
    .await?;
    Ok(())
}

// ── /join <pool_id> ──────────────────────────────────────────────────────────

async fn cmd_join(
    bot: Bot,
    dialogue: BetDialogue,
    msg: Message,
    arg: String,
    state: std::sync::Arc<AppState>,
) -> HandlerResult {
    let pool_id: u32 = match arg.trim().parse() {
        Ok(id) => id,
        Err(_) => {
            dialogue.update(BetState::JoinPoolId).await?;
            bot.send_message(msg.chat.id, "Enter the pool ID you want to join:").await?;
            return Ok(());
        }
    };

    prompt_join_side(bot, dialogue, msg.chat.id, pool_id, state).await
}

async fn prompt_join_side(
    bot: Bot,
    dialogue: BetDialogue,
    chat_id: ChatId,
    pool_id: u32,
    state: std::sync::Arc<AppState>,
) -> HandlerResult {
    let pool = match state.contract.get_pool(pool_id).await {
        Ok(p) => p,
        Err(e) => {
            bot.send_message(chat_id, format!("Pool #{} not found: {}", pool_id, e)).await?;
            return Ok(());
        }
    };

    if pool.is_resolved || pool.is_cancelled {
        bot.send_message(chat_id, format!("Pool #{} is already closed.", pool_id)).await?;
        return Ok(());
    }

    if pool.hours_left() <= 0.0 {
        bot.send_message(chat_id, format!("Pool #{} deadline has passed.", pool_id)).await?;
        return Ok(());
    }

    dialogue.update(BetState::JoinSide { pool_id }).await?;

    let currency = pool.currency();
    let text = format!(
        "Pool #{pool_id}: {desc}\n\
        Side A: {a:.3} {sym} | Side B: {b:.3} {sym}\n\
        {h:.1}h left\n\n\
        Choose your side:",
        pool_id = pool_id,
        desc = pool.description,
        a = pool.side_a_pool,
        sym = currency.symbol,
        b = pool.side_b_pool,
        h = pool.hours_left(),
    );

    let keyboard = InlineKeyboardMarkup::new(vec![vec![
        InlineKeyboardButton::callback("Side A", format!("side:{}:1", pool_id)),
        InlineKeyboardButton::callback("Side B", format!("side:{}:2", pool_id)),
    ]]);

    bot.send_message(chat_id, text)
        .reply_markup(keyboard)
        .await?;
    Ok(())
}

// ── /cancel ──────────────────────────────────────────────────────────────────

async fn cmd_cancel(bot: Bot, dialogue: BetDialogue, msg: Message) -> HandlerResult {
    dialogue.update(BetState::Idle).await?;
    bot.send_message(msg.chat.id, "Cancelled. Use /bets to browse pools.").await?;
    Ok(())
}

// ── /me ──────────────────────────────────────────────────────────────────────

async fn cmd_me(bot: Bot, msg: Message, state: std::sync::Arc<AppState>) -> HandlerResult {
    let telegram_id = msg.from.as_ref().map(|u| u.id.0 as i64).unwrap_or(0);
    let username = msg.from.as_ref().and_then(|u| u.username.as_deref());

    let user = if let Ok(db) = &state.db {
        match db.get_or_create_user(telegram_id, username).await {
            Ok(u) => Some(u),
            Err(e) => {
                warn!("DB error getting user: {}", e);
                None
            }
        }
    } else {
        None
    };

    let wallet_line = user
        .as_ref()
        .and_then(|u| u.wallet_address.as_deref())
        .map(|w| format!("Wallet: `{}`", w))
        .unwrap_or_else(|| "Wallet: not linked (use /wallet <address>)".to_string());

    let stats_line = if let Some(u) = &user {
        format!(
            "Bets placed: {}\nBets won: {}\nWin rate: {:.0}%",
            u.bets_placed,
            u.bets_won,
            u.win_rate()
        )
    } else {
        String::new()
    };

    let referral = user
        .as_ref()
        .map(|u| format!("Referral code: `{}`", u.referral_code))
        .unwrap_or_default();

    let text = format!(
        "Your Profile\n\n{}\n{}\n\n{}",
        wallet_line, stats_line, referral
    );

    bot.send_message(msg.chat.id, text)
        .parse_mode(ParseMode::MarkdownV2)
        .await
        .ok();
    // Fallback without markdown
    Ok(())
}

// ── /wallet <address> ─────────────────────────────────────────────────────────

async fn cmd_wallet(
    bot: Bot,
    msg: Message,
    arg: String,
    state: std::sync::Arc<AppState>,
) -> HandlerResult {
    let telegram_id = msg.from.as_ref().map(|u| u.id.0 as i64).unwrap_or(0);
    let addr = arg.trim();

    if addr.is_empty() {
        dialogue_ask_wallet(bot, msg).await?;
        return Ok(());
    }

    if !is_valid_evm_address(addr) {
        bot.send_message(msg.chat.id, "Invalid EVM address. It should start with 0x and be 42 characters.").await?;
        return Ok(());
    }

    if let Ok(db) = &state.db {
        match db.link_wallet(telegram_id, addr).await {
            Ok(_) => {
                bot.send_message(
                    msg.chat.id,
                    format!("Wallet linked: {}\n\nYou can now join and create betting pools!", addr),
                )
                .await?;
            }
            Err(e) => {
                warn!("Failed to link wallet: {}", e);
                bot.send_message(msg.chat.id, "Failed to save wallet. Try again.").await?;
            }
        }
    } else {
        bot.send_message(msg.chat.id, "Database unavailable. Cannot save wallet.").await?;
    }

    Ok(())
}

async fn dialogue_ask_wallet(bot: Bot, msg: Message) -> HandlerResult {
    bot.send_message(
        msg.chat.id,
        "Send your EVM wallet address (starts with 0x):",
    )
    .await?;
    Ok(())
}

// ── /app ─────────────────────────────────────────────────────────────────────

async fn cmd_app(bot: Bot, msg: Message, state: std::sync::Arc<AppState>) -> HandlerResult {
    let app_btn = InlineKeyboardButton::web_app(
        "Open BetChu App",
        WebAppInfo { url: state.config.miniapp_url.parse().unwrap() },
    );
    bot.send_message(msg.chat.id, "Open the BetChu Mini App:")
        .reply_markup(InlineKeyboardMarkup::new(vec![vec![app_btn]]))
        .await?;
    Ok(())
}

// ── /network ─────────────────────────────────────────────────────────────────

async fn cmd_network(bot: Bot, msg: Message, state: std::sync::Arc<AppState>) -> HandlerResult {
    let health = match state
        .btcpc_service
        .as_ref()
    {
        Some(svc) => {
            // Try a quick balance/health check on the BTCPC API
            "Checking..."
        }
        None => "BTCPC node not configured",
    };

    bot.send_message(
        msg.chat.id,
        format!(
            "BTCPC Network Status\n\nAPI: {}\nChain: Base Sepolia\nContract: {}\n\nPowered by BTCPC P2P inference",
            state.config.btcpc_api_url,
            &state.config.bet_contract_address[..8],
        ),
    )
    .await?;
    Ok(())
}

// ── /scores ──────────────────────────────────────────────────────────────────

async fn cmd_scores(bot: Bot, msg: Message, state: std::sync::Arc<AppState>) -> HandlerResult {
    let games = match state.espn.get_finished_games().await {
        Ok(g) => g,
        Err(e) => {
            warn!("ESPN error: {}", e);
            bot.send_message(msg.chat.id, "Could not fetch scores right now.").await?;
            return Ok(());
        }
    };

    if games.is_empty() {
        bot.send_message(msg.chat.id, "No finished games found. Check back later.").await?;
        return Ok(());
    }

    let mut text = String::from("Recent Scores\n\n");
    for g in games.iter().take(10) {
        let score = match (g.home_score, g.away_score) {
            (Some(h), Some(a)) => format!("{}-{}", h, a),
            _ => "N/A".to_string(),
        };
        text.push_str(&format!(
            "{} vs {} — {}\n",
            g.home_team, g.away_team, score
        ));
    }

    bot.send_message(msg.chat.id, text).await?;
    Ok(())
}

// ── Dialogue / multi-step input handler ─────────────────────────────────────

pub async fn handle_dialogue_text(
    bot: Bot,
    dialogue: BetDialogue,
    msg: Message,
    state: std::sync::Arc<AppState>,
) -> HandlerResult {
    let text = match msg.text() {
        Some(t) => t.to_string(),
        None => return Ok(()),
    };
    let chat_id = msg.chat.id;

    match dialogue.get_or_default().await? {
        BetState::Idle => {
            // No active flow — ignore non-command messages
        }

        // ── Create bet flow ──────────────────────────────────────────────
        BetState::CreateDescription => {
            let raw = text.trim().to_string();
            if raw.len() < 5 {
                bot.send_message(chat_id, "Description too short. Try something like 'Alabama vs Georgia':").await?;
                return Ok(());
            }

            bot.send_message(chat_id, "Looking up the game...").await?;

            // 1. Fetch live ESPN games
            let games = state.espn.get_games().await.unwrap_or_default();

            // 2. Try BTCPC semantic match first; fall back to fuzzy string match
            // Returns (normalized_desc, start_epoch, sport) or None
            let matched: Option<(String, u64, String)> = if let Some(btcpc) = &state.btcpc {
                match btcpc.match_game_description(&raw, &games).await {
                    Ok(Some(idx)) => {
                        let g = &games[idx];
                        Some((g.normalized_description(), g.start_epoch, g.sport.clone()))
                    }
                    Ok(None) => find_matching_game(&raw, &games)
                        .map(|g| (g.normalized_description(), g.start_epoch, g.sport.clone())),
                    Err(e) => {
                        warn!("BTCPC game match failed, falling back to fuzzy: {}", e);
                        find_matching_game(&raw, &games)
                            .map(|g| (g.normalized_description(), g.start_epoch, g.sport.clone()))
                    }
                }
            } else {
                find_matching_game(&raw, &games)
                    .map(|g| (g.normalized_description(), g.start_epoch, g.sport.clone()))
            };

            match matched {
                Some((normalized, start_epoch, sport)) if normalized != raw => {
                    let duration_secs = sport_game_duration(&sport);
                    let hours = duration_secs / 3600;

                    dialogue.update(BetState::ConfirmGameMatch {
                        normalized: normalized.clone(),
                        original: raw.clone(),
                        start_epoch,
                        sport: sport.clone(),
                    }).await?;
                    let kb = InlineKeyboardMarkup::new(vec![
                        vec![InlineKeyboardButton::callback(
                            "Yes, use this", "gameconfirm:yes",
                        )],
                        vec![InlineKeyboardButton::callback(
                            "No, use my original", "gameconfirm:no",
                        )],
                        vec![InlineKeyboardButton::callback(
                            "Retype description", "gameconfirm:retype",
                        )],
                    ]);

                    bot.send_message(
                        chat_id,
                        format!(
                            "Found a matching game:\n\n{}\n\nDeadline auto-set to {} hours after kickoff.\n\nUse this as your bet description?",
                            normalized, hours
                        ),
                    )
                    .reply_markup(kb)
                    .await?;
                }
                _ => {
                    // No match — warn, manual deadline will be shown
                    let kb = InlineKeyboardMarkup::new(vec![vec![
                        InlineKeyboardButton::callback("Continue anyway", "gameconfirm:custom"),
                        InlineKeyboardButton::callback("Retype", "gameconfirm:retype"),
                    ]]);

                    bot.send_message(
                        chat_id,
                        format!(
                            "No scheduled game found for \"{}\".\n\n\
                            The pool won't be auto-resolved by the oracle. \
                            Continue with this description?",
                            raw
                        ),
                    )
                    .reply_markup(kb)
                    .await?;

                    dialogue.update(BetState::ConfirmGameMatch {
                        normalized: raw.clone(),
                        original: raw,
                        start_epoch: 0,
                        sport: String::new(),
                    }).await?;
                }
            }
        }

        BetState::ConfirmGameMatch { .. } => {
            // User typed text instead of pressing a button — re-prompt
            bot.send_message(chat_id, "Please use the buttons above to confirm or retype.").await?;
        }

        BetState::CreateAmount { description, currency, bet_type, deadline_secs } => {
            let amount: f64 = match text.trim().parse() {
                Ok(a) if a > 0.0 => a,
                _ => {
                    bot.send_message(chat_id, "Enter a valid amount (e.g. 0.1):").await?;
                    return Ok(());
                }
            };

            let amount_wei = eth_to_wei(amount);
            let currency_sym = if currency == 1 { "ETH" } else { "USDC" };
            let bet_type_name = if bet_type == 1 { "MATCHED" } else { "POOL" };

            dialogue.update(BetState::ConfirmCreate {
                description: description.clone(),
                currency,
                bet_type,
                deadline_secs,
                amount_wei_str: amount_wei.to_string(),
            }).await?;

            let kb = InlineKeyboardMarkup::new(vec![vec![
                InlineKeyboardButton::callback("Confirm", "create:confirm"),
                InlineKeyboardButton::callback("Cancel", "create:cancel"),
            ]]);
            bot.send_message(
                chat_id,
                format!(
                    "Confirm new betting pool:\n\n\
                    Description: {}\n\
                    Currency: {}\n\
                    Type: {}\n\
                    Initial stake: {:.4} {}\n\n\
                    Proceed?",
                    description, currency_sym, bet_type_name, amount, currency_sym
                ),
            )
            .reply_markup(kb)
            .await?;
        }

        BetState::JoinPoolId => {
            let pool_id: u32 = match text.trim().parse() {
                Ok(id) => id,
                Err(_) => {
                    bot.send_message(chat_id, "Enter a valid pool ID number:").await?;
                    return Ok(());
                }
            };
            prompt_join_side(bot, dialogue, chat_id, pool_id, state).await?;
        }

        BetState::JoinAmount { pool_id, side } => {
            let amount: f64 = match text.trim().parse() {
                Ok(a) if a > 0.0 => a,
                _ => {
                    bot.send_message(chat_id, "Enter a valid amount (e.g. 0.5):").await?;
                    return Ok(());
                }
            };

            let amount_wei = eth_to_wei(amount);
            let side_name = if side == 1 { "Side A" } else { "Side B" };

            dialogue.update(BetState::ConfirmJoin {
                pool_id,
                side,
                amount_wei_str: amount_wei.to_string(),
            }).await?;

            let kb = InlineKeyboardMarkup::new(vec![vec![
                InlineKeyboardButton::callback("Confirm", "join:confirm"),
                InlineKeyboardButton::callback("Cancel", "join:cancel"),
            ]]);
            bot.send_message(
                chat_id,
                format!(
                    "Confirm join:\nPool #{}\n{}: {:.4} ETH\n\nProceed?",
                    pool_id, side_name, amount
                ),
            )
            .reply_markup(kb)
            .await?;
        }

        BetState::MiniAppJoinAmount { pool_id, side, description, currency, bet_type } => {
            let amount: f64 = match text.trim().parse() {
                Ok(a) if a > 0.0 => a,
                _ => {
                    bot.send_message(chat_id, "Enter a valid amount:").await?;
                    return Ok(());
                }
            };

            let amount_wei = eth_to_wei(amount);
            let side_name = if side == 1 { "Side A" } else { "Side B" };
            let currency_sym = if currency == 1 { "ETH" } else { "USDC" };

            dialogue.update(BetState::ConfirmJoin {
                pool_id,
                side,
                amount_wei_str: amount_wei.to_string(),
            }).await?;

            let kb = InlineKeyboardMarkup::new(vec![vec![
                InlineKeyboardButton::callback("Confirm", "join:confirm"),
                InlineKeyboardButton::callback("Cancel", "join:cancel"),
            ]]);
            bot.send_message(
                chat_id,
                format!(
                    "Join pool #{}: {}\n{}: {:.4} {}\n\nConfirm?",
                    pool_id, description, side_name, amount, currency_sym
                ),
            )
            .reply_markup(kb)
            .await?;
        }

        BetState::LinkWallet => {
            let addr = text.trim();
            if is_valid_evm_address(addr) {
                let telegram_id = msg.from.as_ref().map(|u| u.id.0 as i64).unwrap_or(0);
                if let Ok(db) = &state.db {
                    let _ = db.link_wallet(telegram_id, addr).await;
                }
                dialogue.update(BetState::Idle).await?;
                bot.send_message(chat_id, format!("Wallet linked: {}", addr)).await?;
            } else {
                bot.send_message(chat_id, "Invalid address. Must be 0x... (42 chars):").await?;
            }
        }

        _ => {}
    }

    Ok(())
}

// ── Callback query handler ────────────────────────────────────────────────────

pub async fn handle_callback(
    bot: Bot,
    dialogue: BetDialogue,
    q: CallbackQuery,
    state: std::sync::Arc<AppState>,
) -> HandlerResult {
    bot.answer_callback_query(&q.id).await?;

    let data = match &q.data {
        Some(d) => d.clone(),
        None => return Ok(()),
    };

    let chat_id = match q.message.as_ref().and_then(|m| Some(m.chat().id)) {
        Some(id) => id,
        None => return Ok(()),
    };

    let parts: Vec<&str> = data.splitn(3, ':').collect();

    match parts.as_slice() {
        // Game description confirm / reject
        ["gameconfirm", action] => {
            if let BetState::ConfirmGameMatch { normalized, original, start_epoch, sport } = dialogue.get_or_default().await? {
                let description = match *action {
                    "yes" => normalized,
                    "no" | "custom" => original,
                    "retype" => {
                        dialogue.update(BetState::CreateDescription).await?;
                        bot.send_message(chat_id, "OK, describe the bet again:").await?;
                        return Ok(());
                    }
                    _ => original,
                };

                // Auto-deadline: absolute unix timestamp of game end
                // 0 means manual picker will be shown
                let auto_deadline = if start_epoch > 0 {
                    start_epoch + sport_game_duration(&sport)
                } else {
                    0
                };

                dialogue.update(BetState::CreateCurrency { description: description.clone(), auto_deadline }).await?;
                let kb = InlineKeyboardMarkup::new(vec![vec![
                    InlineKeyboardButton::callback("ETH (Base)", "currency:1"),
                    InlineKeyboardButton::callback("USDC", "currency:2"),
                ]]);
                bot.send_message(
                    chat_id,
                    format!("Bet: \"{}\"\n\nChoose the betting currency:", description),
                )
                .reply_markup(kb)
                .await?;
            }
        }

        // Side selection for joining
        ["side", pool_id_str, side_str] => {
            let pool_id: u32 = pool_id_str.parse().unwrap_or(0);
            let side: u8 = side_str.parse().unwrap_or(1);
            dialogue.update(BetState::JoinAmount { pool_id, side }).await?;
            let side_name = if side == 1 { "Side A" } else { "Side B" };
            bot.send_message(chat_id, format!("You chose {}. Enter your bet amount in ETH:", side_name)).await?;
        }

        // Currency selection during create
        ["currency", currency_str] => {
            let currency: u8 = currency_str.parse().unwrap_or(1);
            if let BetState::CreateCurrency { description, auto_deadline } = dialogue.get_or_default().await? {
                dialogue.update(BetState::CreateBetType { description, currency, auto_deadline }).await?;
                let kb = InlineKeyboardMarkup::new(vec![vec![
                    InlineKeyboardButton::callback("Pool (winner takes all)", "bettype:2"),
                    InlineKeyboardButton::callback("Matched (equal sides)", "bettype:1"),
                ]]);
                bot.send_message(chat_id, "Choose bet type:")
                    .reply_markup(kb)
                    .await?;
            }
        }

        // Bet type selection during create
        ["bettype", bt_str] => {
            let bet_type: u8 = bt_str.parse().unwrap_or(2);
            if let BetState::CreateBetType { description, currency, auto_deadline } = dialogue.get_or_default().await? {
                if auto_deadline > 0 {
                    // Deadline is auto-computed from game start — skip the picker
                    dialogue.update(BetState::CreateAmount {
                        description,
                        currency,
                        bet_type,
                        deadline_secs: auto_deadline,
                    }).await?;
                    bot.send_message(chat_id, "Enter your initial stake amount (e.g. 0.01):").await?;
                } else {
                    dialogue.update(BetState::CreateDeadline { description, currency, bet_type }).await?;
                    let now = std::time::SystemTime::now()
                        .duration_since(std::time::UNIX_EPOCH)
                        .unwrap_or_default()
                        .as_secs();
                    let kb = InlineKeyboardMarkup::new(vec![vec![
                        InlineKeyboardButton::callback("1 hour", &format!("deadline:{}", now + 3600)),
                        InlineKeyboardButton::callback("6 hours", &format!("deadline:{}", now + 21600)),
                        InlineKeyboardButton::callback("24 hours", &format!("deadline:{}", now + 86400)),
                        InlineKeyboardButton::callback("1 week", &format!("deadline:{}", now + 604800)),
                    ]]);
                    bot.send_message(chat_id, "How long should this pool be open?")
                        .reply_markup(kb)
                        .await?;
                }
            }
        }

        // Deadline selection during create (manual — no game matched)
        ["deadline", secs_str] => {
            let deadline_secs: u64 = secs_str.parse().unwrap_or(0);
            if deadline_secs == 0 {
                return Ok(());
            }
            if let BetState::CreateDeadline { description, currency, bet_type } = dialogue.get_or_default().await? {
                dialogue.update(BetState::CreateAmount { description, currency, bet_type, deadline_secs }).await?;
                bot.send_message(chat_id, "Enter your initial stake amount (e.g. 0.01):").await?;
            }
        }

        // Create confirm/cancel
        ["create", "confirm"] => {
            if let BetState::ConfirmCreate { description, currency, bet_type, deadline_secs, amount_wei_str } =
                dialogue.get_or_default().await?
            {
                dialogue.update(BetState::Idle).await?;
                let amount_wei: U256 = amount_wei_str.parse().unwrap_or(U256::ZERO);

                match state.contract.create_bet_pool(&description, deadline_secs, currency, bet_type, amount_wei).await {
                    Ok(pool_id) => {
                        info!("Created pool #{} by chat {}", pool_id, chat_id);
                        bot.send_message(
                            chat_id,
                            format!("Betting pool #{} created!\n\nDescription: {}\nShare with friends to get bets on both sides.", pool_id, description),
                        )
                        .await?;
                    }
                    Err(e) => {
                        warn!("Failed to create pool: {}", e);
                        bot.send_message(chat_id, format!("Transaction failed: {}", e)).await?;
                    }
                }
            }
        }

        ["create", "cancel"] => {
            dialogue.update(BetState::Idle).await?;
            bot.send_message(chat_id, "Cancelled.").await?;
        }

        // Join confirm/cancel
        ["join", "confirm"] => {
            if let BetState::ConfirmJoin { pool_id, side, amount_wei_str } =
                dialogue.get_or_default().await?
            {
                dialogue.update(BetState::Idle).await?;
                let amount_wei: U256 = amount_wei_str.parse().unwrap_or(U256::ZERO);

                match state.contract.join_bet_pool(pool_id, side, amount_wei).await {
                    Ok(tx_hash) => {
                        info!("Joined pool #{} side {} tx {}", pool_id, side, tx_hash);
                        bot.send_message(
                            chat_id,
                            format!("Joined pool #{}! Transaction: {}", pool_id, &tx_hash[..12]),
                        )
                        .await?;
                    }
                    Err(e) => {
                        warn!("Failed to join pool: {}", e);
                        bot.send_message(chat_id, format!("Transaction failed: {}", e)).await?;
                    }
                }
            }
        }

        ["join", "cancel"] => {
            dialogue.update(BetState::Idle).await?;
            bot.send_message(chat_id, "Cancelled.").await?;
        }

        _ => {}
    }

    Ok(())
}

// ── web_app_data handler (from Mini App) ────────────────────────────────────

pub async fn handle_webapp_data(
    bot: Bot,
    dialogue: BetDialogue,
    msg: Message,
    state: std::sync::Arc<AppState>,
) -> HandlerResult {
    let data = match msg.web_app_data() {
        Some(d) => d.data.clone(),
        None => return Ok(()),
    };

    let payload: serde_json::Value = match serde_json::from_str(&data) {
        Ok(v) => v,
        Err(_) => {
            bot.send_message(msg.chat.id, "Could not read app data.").await?;
            return Ok(());
        }
    };

    let action = payload["action"].as_str().unwrap_or("");

    match action {
        "join_bet" => {
            let pool_id = payload["poolId"].as_u64().unwrap_or(0) as u32;
            let side_str = payload["side"].as_str().unwrap_or("A");
            let side: u8 = if side_str == "A" { 1 } else { 2 };

            let pool = match state.contract.get_pool(pool_id).await {
                Ok(p) => p,
                Err(e) => {
                    bot.send_message(msg.chat.id, format!("Pool not found: {}", e)).await?;
                    return Ok(());
                }
            };

            if pool.is_resolved || pool.is_cancelled || pool.hours_left() <= 0.0 {
                bot.send_message(msg.chat.id, "This pool is no longer accepting bets.").await?;
                return Ok(());
            }

            dialogue.update(BetState::MiniAppJoinAmount {
                pool_id,
                side,
                description: pool.description.clone(),
                currency: pool.betting_currency,
                bet_type: pool.bet_type,
            }).await?;

            let side_name = if side == 1 { "Side A" } else { "Side B" };
            let currency_sym = pool.currency().symbol;

            bot.send_message(
                msg.chat.id,
                format!(
                    "Joining pool #{} — {}\n{}: Enter amount in {}:",
                    pool_id, pool.description, side_name, currency_sym
                ),
            )
            .await?;
        }

        "create_bet" => {
            let description = payload["description"].as_str().unwrap_or("").trim();
            if description.len() < 5 {
                bot.send_message(msg.chat.id, "Description too short.").await?;
                return Ok(());
            }

            let hours: f64 = payload["hoursUntilDeadline"].as_f64().unwrap_or(24.0);
            if !(1.0..=8760.0).contains(&hours) {
                bot.send_message(msg.chat.id, "Invalid deadline (1–8760 hours).").await?;
                return Ok(());
            }

            let currency_str = payload["currency"].as_str().unwrap_or("ETH");
            let bet_type_str = payload["betType"].as_str().unwrap_or("POOL");

            let currency: u8 = if currency_str == "USDC" { 2 } else { 1 };
            let bet_type: u8 = if bet_type_str == "MATCHED" { 1 } else { 2 };
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_secs();
            let deadline_secs = now + (hours * 3600.0) as u64;

            dialogue.update(BetState::CreateAmount {
                description: description.to_string(),
                currency,
                bet_type,
                deadline_secs,
            }).await?;

            bot.send_message(msg.chat.id, "Enter your initial stake amount:").await?;
        }

        _ => {
            bot.send_message(msg.chat.id, "Unknown action from Mini App.").await?;
        }
    }

    Ok(())
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn truncate(s: &str, max_len: usize) -> &str {
    if s.len() <= max_len {
        s
    } else {
        &s[..max_len]
    }
}

fn is_valid_evm_address(addr: &str) -> bool {
    addr.starts_with("0x") && addr.len() == 42 && addr[2..].chars().all(|c| c.is_ascii_hexdigit())
}
