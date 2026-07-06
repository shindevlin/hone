//! Structured event emission (indexed, queryable off-chain).

#[cfg(target_arch = "wasm32")]
extern crate alloc;
#[cfg(target_arch = "wasm32")]
use alloc::string::String;

use serde::Serialize;

/// Marker trait for serializable events.
pub trait Event: Serialize {}
impl<T: Serialize> Event for T {}

/// Emit a structured event. Events are included in block receipts and
/// indexed by the explorer. They do NOT affect contract state.
///
/// # Example
/// ```rust
/// #[derive(Serialize)]
/// struct Transfer { from: String, to: String, amount: u128 }
///
/// emit_event(&Transfer { from: sender, to: receiver, amount });
/// ```
pub fn emit_event<T: Serialize>(event: &T) {
    let json = serde_json::to_string(event).unwrap_or_default();
    let bytes = json.as_bytes();
    crate::env::emit_event(bytes);
}

/// Standard BSP-20 transfer event.
#[derive(Serialize)]
pub struct TransferEvent {
    pub standard: &'static str,
    pub version: &'static str,
    pub event: &'static str,
    pub from: String,
    pub to: String,
    pub amount: u128,
}

impl TransferEvent {
    pub fn new(from: String, to: String, amount: u128) -> Self {
        Self { standard: "bsp-20", version: "1.0.0", event: "ft_transfer", from, to, amount }
    }
}

/// Standard BSP-721 mint event.
#[derive(Serialize)]
pub struct NftMintEvent {
    pub standard: &'static str,
    pub version: &'static str,
    pub event: &'static str,
    pub owner: String,
    pub token_id: String,
}

impl NftMintEvent {
    pub fn new(owner: String, token_id: String) -> Self {
        Self { standard: "bsp-721", version: "1.0.0", event: "nft_mint", owner, token_id }
    }
}
