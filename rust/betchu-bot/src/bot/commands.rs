use teloxide::{prelude::*, utils::command::BotCommands};

#[derive(BotCommands, Clone, Debug)]
#[command(rename_rule = "lowercase", description = "BetChu commands:")]
pub enum BetchuCommand {
    #[command(description = "Welcome message and getting started guide")]
    Start,
    #[command(description = "Show all commands")]
    Help,
    #[command(description = "Browse open betting pools")]
    Bets,
    #[command(description = "Create a new betting pool")]
    CreateBet,
    #[command(description = "Join a betting pool — /join <pool_id>")]
    Join(String),
    #[command(description = "Cancel current action")]
    Cancel,
    #[command(description = "Your profile, stats, and wallet")]
    Me,
    #[command(description = "Link your wallet address — /wallet <address>")]
    Wallet(String),
    #[command(description = "Open the BetChu Mini App")]
    App,
    #[command(description = "Current HONE network status")]
    Network,
    #[command(description = "Latest college football/basketball scores")]
    Scores,
}
