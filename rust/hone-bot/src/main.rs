mod api;
mod commands;

use std::sync::Arc;

use anyhow::Result;
use teloxide::prelude::*;
use tracing::info;

use api::HoneClient;
use commands::{handle_command, Command, HandlerState};

#[tokio::main]
async fn main() -> Result<()> {
    // UFL 3.8 Decentralized acceptance gate (UFL Section 9): nothing runs
    // until the license is accepted. See docs/LICENSE_ACCEPTANCE.md.
    hone_license::require_acceptance_or_exit(&hone_license::default_data_dir());

    tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info")),
        )
        .init();

    let bot_token = std::env::var("HONE_BOT_TOKEN")
        .expect("HONE_BOT_TOKEN environment variable must be set");

    let api_url = std::env::var("HONE_API_URL")
        .unwrap_or_else(|_| "http://localhost:4242".to_string());

    let default_account = std::env::var("HONE_ACCOUNT").ok();

    info!("Starting HONE bot");
    info!("API URL: {}", api_url);
    if let Some(ref acc) = default_account {
        info!("Default account: {}", acc);
    }

    let bot = Bot::new(bot_token);

    let state = Arc::new(HandlerState {
        api: Arc::new(HoneClient::new(api_url)),
        default_account,
    });

    let handler = Update::filter_message()
        .filter_command::<Command>()
        .endpoint({
            let state = Arc::clone(&state);
            move |bot: Bot, msg: Message, cmd: Command| {
                let state = Arc::clone(&state);
                async move { handle_command(bot, msg, cmd, state).await }
            }
        });

    Dispatcher::builder(bot, handler)
        .enable_ctrlc_handler()
        .build()
        .dispatch()
        .await;

    Ok(())
}
