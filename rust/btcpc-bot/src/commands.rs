use std::sync::Arc;

use teloxide::{prelude::*, utils::command::BotCommands};

use crate::api::{dreams_to_btcpc, BtcpcClient};

#[derive(BotCommands, Clone, Debug)]
#[command(rename_rule = "lowercase", description = "BTCPC Bot commands:")]
pub enum Command {
    #[command(description = "Welcome message and command list")]
    Start,
    #[command(description = "Check BTCPC balance — /balance [account]")]
    Balance(String),
    #[command(description = "Check staked amount — /stake [account]")]
    Stake(String),
    #[command(description = "Current epoch number and info")]
    Epoch,
    #[command(description = "Block info — /block [epoch] (omit for latest)")]
    Block(String),
    #[command(description = "Network status: latest epoch, node health")]
    Network,
    #[command(description = "Unsigned transfer info — /transfer <to> <amount>")]
    Transfer(String),
    #[command(description = "Claim testnet tokens from the faucet")]
    Faucet,
    #[command(description = "Full command reference")]
    Help,
}

pub struct HandlerState {
    pub api: Arc<BtcpcClient>,
    pub default_account: Option<String>,
}

pub async fn handle_command(
    bot: Bot,
    msg: Message,
    cmd: Command,
    state: Arc<HandlerState>,
) -> ResponseResult<()> {
    let text = match cmd {
        Command::Start => handle_start(),
        Command::Help => handle_help(),
        Command::Balance(arg) => handle_balance(arg, &state).await,
        Command::Stake(arg) => handle_stake(arg, &state).await,
        Command::Epoch => handle_epoch(&state).await,
        Command::Block(arg) => handle_block(arg, &state).await,
        Command::Network => handle_network(&state).await,
        Command::Transfer(arg) => handle_transfer(arg),
        Command::Faucet => handle_faucet(&state).await,
    };

    bot.send_message(msg.chat.id, text).await?;
    Ok(())
}

fn handle_start() -> String {
    "Welcome to the BTCPC Bot!\n\n\
    BTCPC is a proof-of-inference blockchain. Use the commands below to interact:\n\n\
    /balance [account] — Check your BTCPC balance\n\
    /stake [account] — Check staked amount\n\
    /epoch — Current epoch info\n\
    /block [epoch] — Block details\n\
    /network — Network status\n\
    /transfer <to> <amount> — Transfer info\n\
    /faucet — Claim testnet tokens\n\
    /help — Full command reference\n\n\
    Set BTCPC_ACCOUNT env var to use a default account for balance/stake/faucet."
        .to_string()
}

fn handle_help() -> String {
    "/start — Welcome message and command list\n\
    /balance [account] — BTCPC balance (uses BTCPC_ACCOUNT if no arg)\n\
    /stake [account] — Staked amount (uses BTCPC_ACCOUNT if no arg)\n\
    /epoch — Current epoch number and timestamp\n\
    /block [epoch] — Block info (latest if epoch omitted)\n\
    /network — Latest epoch, node health\n\
    /transfer <to> <amount> — Unsigned transfer info (explains signing)\n\
    /faucet — Claim testnet BTCPC for the linked account\n\
    /help — This message\n\n\
    Units: 1 BTCPC = 10,000,000,000 dreams"
        .to_string()
}

async fn handle_balance(arg: String, state: &Arc<HandlerState>) -> String {
    let account = resolve_account(arg.trim(), &state.default_account);
    let account = match account {
        Some(a) => a,
        None => return "No account specified. Provide an account or set BTCPC_ACCOUNT.".to_string(),
    };

    match state.api.get_balance(&account).await {
        Ok(resp) => {
            let btcpc = match resp.dreams {
                Some(d) => dreams_to_btcpc(d),
                None => resp.balance.unwrap_or(0.0) / 10_000_000_000.0,
            };
            let token = resp.token.unwrap_or_else(|| "BTCPC".to_string());
            format!(
                "Balance for {}:\n{:.4} {}\n({} dreams)",
                account,
                btcpc,
                token,
                resp.dreams.unwrap_or(0)
            )
        }
        Err(e) => offline_or_error(&e.to_string()),
    }
}

async fn handle_stake(arg: String, state: &Arc<HandlerState>) -> String {
    let account = resolve_account(arg.trim(), &state.default_account);
    let account = match account {
        Some(a) => a,
        None => return "No account specified. Provide an account or set BTCPC_ACCOUNT.".to_string(),
    };

    match state.api.get_stake(&account).await {
        Ok(resp) => {
            let btcpc = match resp.dreams {
                Some(d) => dreams_to_btcpc(d),
                None => resp.stake.unwrap_or(0.0) / 10_000_000_000.0,
            };
            format!(
                "Stake for {}:\n{:.4} BTCPC\n({} dreams)",
                account,
                btcpc,
                resp.dreams.unwrap_or(0)
            )
        }
        Err(e) => offline_or_error(&e.to_string()),
    }
}

async fn handle_epoch(state: &Arc<HandlerState>) -> String {
    match state.api.get_latest().await {
        Ok(resp) => {
            let epoch = resp.current_epoch.or(resp.epoch).unwrap_or(0);
            let hash = resp.hash.unwrap_or_else(|| "unknown".to_string());
            format!("Current epoch: {}\nLatest hash: {}", epoch, hash)
        }
        Err(e) => offline_or_error(&e.to_string()),
    }
}

async fn handle_block(arg: String, state: &Arc<HandlerState>) -> String {
    let epoch_opt: Option<u64> = arg.trim().parse().ok();

    let epoch = match epoch_opt {
        Some(e) => e,
        None => match state.api.get_latest().await {
            Ok(resp) => resp.current_epoch.or(resp.epoch).unwrap_or(0),
            Err(e) => return offline_or_error(&e.to_string()),
        },
    };

    match state.api.get_block(epoch).await {
        Ok(resp) => {
            let hash = resp.hash.unwrap_or_else(|| "unknown".to_string());
            let epoch_num = resp.epoch.unwrap_or(epoch);
            let header_str = resp
                .header
                .map(|h| format!("\nHeader: {}", h))
                .unwrap_or_default();
            let payload_str = resp
                .payload
                .map(|p| {
                    let s = p.to_string();
                    if s.len() > 200 {
                        format!("\nPayload: {}...", &s[..200])
                    } else {
                        format!("\nPayload: {}", s)
                    }
                })
                .unwrap_or_default();
            format!(
                "Block #{}\nHash: {}{}{}",
                epoch_num, hash, header_str, payload_str
            )
        }
        Err(e) => offline_or_error(&e.to_string()),
    }
}

async fn handle_network(state: &Arc<HandlerState>) -> String {
    let latest_result = state.api.get_latest().await;
    let health_result = state.api.get_health().await;

    let epoch_line = match latest_result {
        Ok(resp) => {
            let epoch = resp.current_epoch.or(resp.epoch).unwrap_or(0);
            format!("Latest epoch: {}", epoch)
        }
        Err(e) => format!("Latest epoch: unavailable ({})", e),
    };

    let health_line = match health_result {
        Ok(resp) => {
            let status = resp.status.unwrap_or_else(|| "unknown".to_string());
            format!("Node health: {}", status)
        }
        Err(_) => "Node health: offline".to_string(),
    };

    format!("BTCPC Network\n{}\n{}", epoch_line, health_line)
}

fn handle_transfer(arg: String) -> String {
    let parts: Vec<&str> = arg.trim().splitn(2, ' ').collect();
    if parts.len() < 2 || parts[0].is_empty() || parts[1].is_empty() {
        return "Usage: /transfer <to_account> <amount>\nExample: /transfer alice 1.5".to_string();
    }
    let to = parts[0];
    let amount_str = parts[1];
    let amount: f64 = match amount_str.trim().parse() {
        Ok(a) => a,
        Err(_) => return format!("Invalid amount: {}", amount_str),
    };
    let dreams = (amount * 10_000_000_000.0) as u64;

    format!(
        "Transfer details (unsigned):\n\
        To: {}\n\
        Amount: {:.4} BTCPC ({} dreams)\n\n\
        BTCPC transactions must be signed with your private key.\n\
        Use the btcpc-cli or wallet app to submit signed transactions to the node.\n\
        API endpoint: POST /api/tx",
        to, amount, dreams
    )
}

async fn handle_faucet(state: &Arc<HandlerState>) -> String {
    let account = match &state.default_account {
        Some(a) => a.clone(),
        None => {
            return "No default account configured. Set BTCPC_ACCOUNT to use /faucet.".to_string()
        }
    };

    match state.api.claim_faucet(&account).await {
        Ok(resp) => {
            let success = resp.success.unwrap_or(false);
            let msg = resp
                .message
                .unwrap_or_else(|| if success { "Claimed!".to_string() } else { "Failed.".to_string() });
            if success {
                let btcpc = match resp.dreams {
                    Some(d) => dreams_to_btcpc(d),
                    None => resp.amount.unwrap_or(0.0),
                };
                format!(
                    "Faucet claimed for {}!\nReceived: {:.4} BTCPC\n{}",
                    account, btcpc, msg
                )
            } else {
                format!("Faucet claim failed for {}: {}", account, msg)
            }
        }
        Err(e) => offline_or_error(&e.to_string()),
    }
}

fn resolve_account(arg: &str, default: &Option<String>) -> Option<String> {
    if !arg.is_empty() {
        Some(arg.to_string())
    } else {
        default.clone()
    }
}

fn offline_or_error(msg: &str) -> String {
    if msg.contains("Node offline") || msg.contains("connection") || msg.contains("connect") {
        "Node offline, try again shortly.".to_string()
    } else {
        format!("Error: {}", msg)
    }
}
