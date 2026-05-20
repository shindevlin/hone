use serde::{Deserialize, Serialize};

// ── Ledger entry (wire format matches Node.js ledger) ────────────────────────

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct LedgerEntry {
    #[serde(rename = "type")]
    pub entry_type: String,
    pub from: Option<String>,
    pub to: Option<String>,
    pub token: String,
    pub amount: f64,
    pub epoch: u64,
    pub signature: Option<String>,
    pub signed_by: Option<String>,
    pub memo: Option<String>,
    pub timestamp: u64,
    // Commerce-specific payloads (None for non-commerce entries)
    #[serde(skip_serializing_if = "Option::is_none")]
    pub store_data: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub product_data: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order_data: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reputation_data: Option<serde_json::Value>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub memo_key_data: Option<serde_json::Value>,
}

impl LedgerEntry {
    pub fn new(entry_type: &str, from: &str, epoch: u64) -> Self {
        LedgerEntry {
            entry_type: entry_type.to_string(),
            from: Some(from.to_string()),
            to: None,
            token: "BTCPC".to_string(),
            amount: 0.0,
            epoch,
            signature: None,
            signed_by: None,
            memo: None,
            timestamp: now_ms(),
            store_data: None,
            product_data: None,
            order_data: None,
            reputation_data: None,
            memo_key_data: None,
        }
    }
}

pub fn now_ms() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}

// Genesis timestamp: 2026-04-15T07:00:00.000Z
const GENESIS_MS: u64 = 1776236400000;
const EPOCH_DURATION_MS: u64 = 30_000;

pub fn current_epoch() -> u64 {
    let now = now_ms();
    if now < GENESIS_MS {
        return 0;
    }
    (now - GENESIS_MS) / EPOCH_DURATION_MS
}

// ── In-memory state snapshots ─────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct ShippingAccount {
    pub carrier: String,           // "ups" | "fedex" | "usps" | "dhl" | "pirateship" | "other"
    pub account_id_masked: String, // last 4 chars shown — full id stored only in ledger
    pub default_service: String,   // "ground" | "2day" | "overnight" | etc.
    pub linked_at: u64,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct Store {
    pub seller: String,
    pub name: String,
    pub banner_cid: Option<String>,
    pub description_cid: Option<String>,
    pub categories: Vec<String>,
    pub capacity: u32,
    pub used_capacity: u32,
    pub stake_amount: f64,
    pub status: String, // "active" | "closed"
    pub opened_at: u64,
    pub score: f64,
    #[serde(default)]
    pub shipping_accounts: Vec<ShippingAccount>,
    #[serde(default)]
    pub tor_enabled: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub onion_address: Option<String>,
}

#[derive(Serialize, Deserialize, Clone, Debug, Default)]
pub struct ProductQA {
    pub qa_id: String,
    pub asker: String,
    pub question: String,
    pub answer: Option<String>,
    pub asked_epoch: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub answered_epoch: Option<u64>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Product {
    pub product_id: String,
    pub seller: String,
    pub title: String,
    pub description: Option<String>,
    pub price: f64,
    pub token: String,
    pub image_cid: Option<String>,
    pub inventory: Option<u32>,
    pub categories: Vec<String>,
    pub status: String, // "active" | "delisted"
    pub created_epoch: u64,
    #[serde(default)]
    pub auto_deliver: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub delivery_cid: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sale_price: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sale_ends_epoch: Option<u64>,
    #[serde(default)]
    pub questions: Vec<ProductQA>,
    // PWYW / pricing
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min_price: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub suggested_price: Option<f64>,
    #[serde(default)]
    pub tip_enabled: bool,
    // Condition
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub condition_notes: Option<String>,
    // Early Access
    #[serde(skip_serializing_if = "Option::is_none")]
    pub expected_delivery_epoch: Option<u64>,
    // Tech specs
    #[serde(skip_serializing_if = "Option::is_none")]
    pub specs: Option<serde_json::Value>,
    // Additional images
    #[serde(default)]
    pub image_cids: Vec<String>,
    // Listing bump
    #[serde(skip_serializing_if = "Option::is_none")]
    pub bumped_until_epoch: Option<u64>,
}

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Order {
    pub order_id: String,
    pub buyer: String,
    pub seller: String,
    pub product_id: String,
    pub quantity: u32,
    pub unit_price: f64,
    pub total: f64,
    pub token: String,
    pub escrow_id: Option<String>,
    pub escrow_locked: bool,
    pub escrow_amount: f64,
    pub status: String, // "pending" | "fulfilled" | "delivered" | "cancelled" | "disputed"
    pub fulfillment_cid: Option<String>,
    pub placed_epoch: u64,
    pub fulfill_deadline_epoch: u64, // placed_epoch + 4800
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shipping_address: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub carrier: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tracking_number: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shipping_service: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub shipping_note: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub tip_amount: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub license_key: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub fulfilled_epoch: Option<u64>,
}

// ── Request bodies ────────────────────────────────────────────────────────────

#[derive(Deserialize)]
pub struct OpenStoreRequest {
    pub name: Option<String>,
    pub banner_cid: Option<String>,
    pub description_cid: Option<String>,
    pub categories: Option<Vec<String>>,
    pub initial_capacity: Option<u32>,
    pub stake_paid_usd: Option<f64>,
}

#[derive(Deserialize)]
pub struct UpdateStoreRequest {
    pub name: Option<String>,
    pub banner_cid: Option<String>,
    pub description_cid: Option<String>,
    pub categories: Option<Vec<String>>,
}

#[derive(Deserialize)]
pub struct CreateProductRequest {
    pub slug: String,
    pub title: String,
    pub description: Option<String>,
    pub price: f64,
    pub token: Option<String>,
    pub image_cid: Option<String>,
    pub inventory: Option<u32>,
    pub categories: Option<Vec<String>>,
    pub auto_deliver: Option<bool>,
    pub delivery_cid: Option<String>,
    pub sale_price: Option<f64>,
    pub sale_ends_epoch: Option<u64>,
    pub min_price: Option<f64>,
    pub suggested_price: Option<f64>,
    pub tip_enabled: Option<bool>,
    pub condition: Option<String>,
    pub condition_notes: Option<String>,
    pub expected_delivery_epoch: Option<u64>,
    pub specs: Option<serde_json::Value>,
    pub image_cids: Option<Vec<String>>,
}

#[derive(Deserialize)]
pub struct UpdateProductRequest {
    pub title: Option<String>,
    pub description: Option<String>,
    pub price: Option<f64>,
    pub image_cid: Option<String>,
    pub inventory: Option<i64>,
    pub auto_deliver: Option<bool>,
    pub delivery_cid: Option<String>,
    pub sale_price: Option<f64>,
    pub sale_ends_epoch: Option<u64>,
    pub min_price: Option<f64>,
    pub suggested_price: Option<f64>,
    pub tip_enabled: Option<bool>,
    pub condition: Option<String>,
    pub condition_notes: Option<String>,
    pub expected_delivery_epoch: Option<u64>,
    pub specs: Option<serde_json::Value>,
    pub image_cids: Option<Vec<String>>,
    pub bumped_until_epoch: Option<u64>,
}

#[derive(Deserialize)]
pub struct PlaceOrderRequest {
    pub product_id: String,
    pub quantity: Option<u32>,
    pub token: Option<String>,
    pub shipping_address: Option<String>,
    pub tip_amount: Option<f64>,
}

#[derive(Deserialize)]
pub struct AskQuestionRequest {
    pub question: String,
}

#[derive(Deserialize)]
pub struct AnswerQuestionRequest {
    pub answer: String,
}

#[derive(Deserialize)]
pub struct FulfillOrderRequest {
    pub fulfillment_cid: Option<String>,
    pub carrier: Option<String>,
    pub tracking_number: Option<String>,
    pub shipping_service: Option<String>,
    pub shipping_note: Option<String>,
}

#[derive(Deserialize)]
pub struct LinkShippingRequest {
    pub carrier: String,
    pub account_id: String,
    pub default_service: Option<String>,
}

#[derive(Deserialize)]
pub struct DisputeOrderRequest {
    pub memo: Option<String>,
}

#[derive(Deserialize)]
pub struct ReputationVoteRequest {
    pub target_type: String,
    pub target_id: String,
    pub vote: i8,
    pub weight: Option<u8>,
    pub memo: Option<String>,
}

#[derive(Deserialize)]
pub struct CapacityQuoteRequest {
    pub current_capacity: Option<u32>,
    pub units: u32,
}

#[derive(Deserialize)]
pub struct ImportUrlRequest {
    pub url: String,
}

#[derive(Deserialize)]
pub struct PaginationQuery {
    pub limit: Option<usize>,
    pub offset: Option<usize>,
    pub seller: Option<String>,
    pub q: Option<String>,
    pub category: Option<String>,
    pub sort: Option<String>,  // "newest" | "price_asc" | "price_desc" | "bumped_first"
}

// ── Offer ────────────────────────────────────────────────────────────────────

#[derive(Serialize, Deserialize, Clone, Debug)]
pub struct Offer {
    pub offer_id: String,
    pub product_id: String,
    pub buyer: String,
    pub seller: String,
    pub amount: f64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub counter_amount: Option<f64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub message: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub counter_message: Option<String>,
    pub status: String, // "pending"|"countered"|"accepted"|"rejected"|"expired"
    pub created_epoch: u64,
    pub expires_epoch: u64,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub order_id: Option<String>,
}

#[derive(Deserialize)]
pub struct PlaceOfferRequest {
    pub product_id: String,
    pub amount: f64,
    pub message: Option<String>,
}

#[derive(Deserialize)]
pub struct CounterOfferRequest {
    pub amount: f64,
    pub message: Option<String>,
}

#[derive(Deserialize)]
pub struct BumpProductRequest {
    pub epochs: u64,
}

#[derive(Deserialize)]
pub struct ReputationReplyRequest {
    pub reply: String,
}

#[derive(Deserialize)]
pub struct RegisterMemoKeyRequest {
    pub memo_pubkey: String,
}

