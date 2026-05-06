use anyhow::Result;
use mongodb::{bson::doc, Client, Collection, Database};
use serde::{Deserialize, Serialize};
use tracing::info;

pub mod models;
pub use models::User;

pub struct DbClient {
    db: Database,
}

impl DbClient {
    pub async fn connect(uri: &str) -> Result<Self> {
        let client = Client::with_uri_str(uri).await?;
        let db = client.database("betchu");
        info!("MongoDB connected: {}", uri);
        Ok(Self { db })
    }

    pub fn users(&self) -> Collection<User> {
        self.db.collection("users")
    }

    pub async fn get_or_create_user(&self, telegram_id: i64, username: Option<&str>) -> Result<User> {
        let col = self.users();
        let filter = doc! { "telegram_id": telegram_id };

        if let Some(user) = col.find_one(filter.clone()).await? {
            return Ok(user);
        }

        let user = User {
            telegram_id,
            username: username.map(|s| s.to_string()),
            wallet_address: None,
            referral_code: generate_referral_code(telegram_id),
            referred_by: None,
            points: 0,
            bets_placed: 0,
            bets_won: 0,
            created_at: chrono::Utc::now(),
        };

        col.insert_one(&user).await?;
        Ok(user)
    }

    pub async fn link_wallet(&self, telegram_id: i64, address: &str) -> Result<()> {
        let col = self.users();
        col.update_one(
            doc! { "telegram_id": telegram_id },
            doc! { "$set": { "wallet_address": address } },
        )
        .await?;
        Ok(())
    }

    pub async fn get_user(&self, telegram_id: i64) -> Result<Option<User>> {
        let col = self.users();
        Ok(col.find_one(doc! { "telegram_id": telegram_id }).await?)
    }
}

fn generate_referral_code(telegram_id: i64) -> String {
    let hash = format!("{:x}", telegram_id * 0x9e37_79b9 + 0x6c62_272e);
    format!("BCH{}", &hash[..6].to_uppercase())
}
