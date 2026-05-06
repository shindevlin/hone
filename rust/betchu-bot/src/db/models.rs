use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct User {
    pub telegram_id: i64,
    pub username: Option<String>,
    pub wallet_address: Option<String>,
    pub referral_code: String,
    pub referred_by: Option<String>,
    pub points: u64,
    pub bets_placed: u32,
    pub bets_won: u32,
    pub created_at: DateTime<Utc>,
}

impl User {
    pub fn display_name(&self) -> String {
        self.username
            .as_deref()
            .map(|u| format!("@{}", u))
            .unwrap_or_else(|| format!("User#{}", self.telegram_id))
    }

    pub fn has_wallet(&self) -> bool {
        self.wallet_address.as_ref().map(|w| !w.is_empty()).unwrap_or(false)
    }

    pub fn win_rate(&self) -> f64 {
        if self.bets_placed == 0 {
            return 0.0;
        }
        self.bets_won as f64 / self.bets_placed as f64 * 100.0
    }
}
