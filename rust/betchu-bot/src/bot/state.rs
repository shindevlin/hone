use alloy::primitives::U256;
use serde::{Deserialize, Serialize};

/// Multi-step conversation state stored per chat.
/// Teloxide's `InMemStorage` keeps this alive across messages in the same chat.
#[derive(Clone, Default, Debug, Serialize, Deserialize)]
pub enum BetState {
    /// No active flow — normal command mode
    #[default]
    Idle,

    // ── Create bet flow ──────────────────────────────────────────
    CreateDescription,

    /// After BTCPC/fuzzy matched the user input to a real ESPN game,
    /// show the normalized description and wait for confirm/retype.
    ConfirmGameMatch {
        normalized: String,
        original: String,
        /// Unix timestamp of game start (0 if no game matched)
        start_epoch: u64,
        sport: String,
    },

    CreateCurrency {
        description: String,
        /// Pre-computed absolute deadline (0 = ask user)
        auto_deadline: u64,
    },

    CreateBetType {
        description: String,
        currency: u8,
        auto_deadline: u64,
    },

    CreateDeadline {
        description: String,
        currency: u8,
        bet_type: u8,
    },

    CreateAmount {
        description: String,
        currency: u8,
        bet_type: u8,
        deadline_secs: u64,
    },

    ConfirmCreate {
        description: String,
        currency: u8,
        bet_type: u8,
        deadline_secs: u64,
        amount_wei_str: String, // U256 as string for serde
    },

    // ── Join bet flow ────────────────────────────────────────────
    JoinPoolId,

    JoinSide {
        pool_id: u32,
    },

    JoinAmount {
        pool_id: u32,
        side: u8,
    },

    ConfirmJoin {
        pool_id: u32,
        side: u8,
        amount_wei_str: String,
    },

    // ── Mini app join (amount only) ──────────────────────────────
    MiniAppJoinAmount {
        pool_id: u32,
        side: u8,
        description: String,
        currency: u8,
        bet_type: u8,
    },

    // ── Wallet setup ─────────────────────────────────────────────
    LinkWallet,
}

pub type BetDialogue = teloxide::dispatching::dialogue::Dialogue<
    BetState,
    teloxide::dispatching::dialogue::InMemStorage<BetState>,
>;
pub type BetStorage = teloxide::dispatching::dialogue::InMemStorage<BetState>;
