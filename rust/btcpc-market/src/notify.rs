use crate::config::Config;
use std::sync::Arc;

pub async fn order_placed(
    cfg: Arc<Config>,
    order_id: String,
    product_id: String,
    buyer: String,
    seller: String,
    total: f64,
    quantity: u32,
) {
    let (token, chat_id) = match (&cfg.telegram_bot_token, &cfg.telegram_chat_id) {
        (Some(t), Some(c)) => (t.clone(), c.clone()),
        _ => return,
    };

    let text = format!(
        "🛒 <b>New order</b> #{order_id}\n\
         📦 Product: <code>{product_id}</code>\n\
         👤 Buyer: <b>{buyer}</b>\n\
         🏪 Seller: <b>{seller}</b>\n\
         💰 Total: <b>{total} BTCPC</b>  ×{quantity}\n\
         ⏱ Fulfill within 40 hours to avoid auto-cancel"
    );

    let url = format!("https://api.telegram.org/bot{token}/sendMessage");
    let client = reqwest::Client::new();
    if let Err(e) = client
        .post(&url)
        .json(&serde_json::json!({
            "chat_id": chat_id,
            "text": text,
            "parse_mode": "HTML",
        }))
        .send()
        .await
    {
        tracing::warn!("Telegram notify failed: {e}");
    }
}
