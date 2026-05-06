pub mod commands;
pub mod handlers;
pub mod state;

use crate::{AppState, bot::commands::BetchuCommand, bot::handlers::*, bot::state::BetStorage};
use std::sync::Arc;
use teloxide::{
    dispatching::{
        dialogue::{self, InMemStorage},
        UpdateFilterExt, UpdateHandler,
    },
    prelude::*,
    types::Update,
};

pub fn schema() -> UpdateHandler<Box<dyn std::error::Error + Send + Sync + 'static>> {
    let command_handler = teloxide::filter_command::<BetchuCommand, _>()
        .endpoint(handle_command);

    let message_handler = Update::filter_message()
        .branch(
            // Route web_app_data messages from the Mini App
            dptree::filter(|msg: Message| msg.web_app_data().is_some())
                .endpoint(handle_webapp_data),
        )
        .branch(command_handler)
        .branch(
            // Route all other text messages to the dialogue handler
            dptree::filter(|msg: Message| msg.text().is_some())
                .endpoint(handle_dialogue_text),
        );

    let callback_handler =
        Update::filter_callback_query().endpoint(handle_callback);

    dialogue::enter::<Update, InMemStorage<state::BetState>, state::BetState, _>()
        .branch(message_handler)
        .branch(callback_handler)
}

pub async fn run(bot: Bot, state: Arc<AppState>) {
    Dispatcher::builder(bot, schema())
        .dependencies(dptree::deps![InMemStorage::<state::BetState>::new(), state])
        .enable_ctrlc_handler()
        .build()
        .dispatch()
        .await;
}
