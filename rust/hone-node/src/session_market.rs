//! Session context marketplace.
//!
//! Sellers list an on-chain session context (rolling conversation summary + metadata).
//! Buyers purchase it, gaining access to the session key to continue the conversation.
//! Sessions include a rolling summary that stays within the context window (≤ 6k tokens).
#![allow(dead_code)]

#![allow(dead_code)]
use anyhow::{bail, Result};
use serde::{Deserialize, Serialize};
use tracing::info;

use hone_types::{LedgerEntry, NATIVE_TOKEN, RECYCLE_FUND_ACCOUNT};
use crate::chain::Chain;

const MARKETPLACE_FEE_BPS: u64 = 200; // 2% to recycle
const MAX_SUMMARY_BYTES: usize = 24_000; // ~6k tokens
const SESSION_EXPIRY_EPOCHS: u64 = 720; // ~6 hours at 30s epochs

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionListing {
    pub listing_id: String,
    pub seller: String,
    pub price_hunits: u64,
    pub model: String,
    pub summary_hash: String, // SHA-256 of summary (summary stored off-chain)
    pub turn_count: u32,
    pub created_epoch: u64,
    pub expires_epoch: u64,
    pub status: ListingStatus,
    pub buyer: Option<String>,
    pub session_key_encrypted: Option<String>, // encrypted with buyer's pubkey after purchase
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ListingStatus {
    Active,
    Sold,
    Cancelled,
    Expired,
}

fn listing_key(listing_id: &str) -> String { format!("session_listing:{}", listing_id) }

pub fn apply_create(chain: &Chain, entry: &LedgerEntry) -> Result<()> {
    let LedgerEntry::SessionListingCreate {
        listing_id, seller, price_hunits, model, summary_hash, turn_count, epoch, ..
    } = entry else { bail!("wrong entry type") };

    if chain.store.state_get(&listing_key(listing_id)).is_some() {
        bail!("listing '{}' already exists", listing_id);
    }

    let listing = SessionListing {
        listing_id: listing_id.clone(),
        seller: seller.clone(),
        price_hunits: *price_hunits,
        model: model.clone(),
        summary_hash: summary_hash.clone(),
        turn_count: *turn_count,
        created_epoch: *epoch,
        expires_epoch: epoch + SESSION_EXPIRY_EPOCHS,
        status: ListingStatus::Active,
        buyer: None,
        session_key_encrypted: None,
    };
    chain.store.state_set(&listing_key(listing_id), &serde_json::to_vec(&listing)?)?;
    info!("[session-market] listing '{}' created by '{}' ({}d)", listing_id, seller, price_hunits);
    Ok(())
}

pub fn apply_buy(chain: &Chain, entry: &LedgerEntry) -> Result<()> {
    let LedgerEntry::SessionListingBuy {
        listing_id, buyer, epoch, ..
    } = entry else { bail!("wrong entry type") };

    let raw = chain.store.state_get(&listing_key(listing_id))
        .ok_or_else(|| anyhow::anyhow!("listing '{}' not found", listing_id))?;
    let mut listing: SessionListing = serde_json::from_slice(&raw)?;

    anyhow::ensure!(listing.status == ListingStatus::Active, "listing '{}' is not active", listing_id);
    anyhow::ensure!(*epoch <= listing.expires_epoch, "listing '{}' has expired", listing_id);
    anyhow::ensure!(buyer != &listing.seller, "cannot buy your own listing");

    chain.store.debit(buyer, NATIVE_TOKEN, listing.price_hunits)?;
    let fee = listing.price_hunits * MARKETPLACE_FEE_BPS / 10_000;
    let seller_cut = listing.price_hunits.saturating_sub(fee);
    chain.store.credit(&listing.seller, NATIVE_TOKEN, seller_cut)?;
    if fee > 0 { chain.store.credit(RECYCLE_FUND_ACCOUNT, NATIVE_TOKEN, fee)?; }

    listing.status = ListingStatus::Sold;
    listing.buyer = Some(buyer.clone());
    chain.store.state_set(&listing_key(listing_id), &serde_json::to_vec(&listing)?)?;
    info!("[session-market] '{}' bought listing '{}' for {}d", buyer, listing_id, listing.price_hunits);
    Ok(())
}

pub fn apply_cancel(chain: &Chain, entry: &LedgerEntry) -> Result<()> {
    let LedgerEntry::SessionListingCancel {
        listing_id, seller, ..
    } = entry else { bail!("wrong entry type") };

    let raw = chain.store.state_get(&listing_key(listing_id))
        .ok_or_else(|| anyhow::anyhow!("listing '{}' not found", listing_id))?;
    let mut listing: SessionListing = serde_json::from_slice(&raw)?;

    anyhow::ensure!(&listing.seller == seller, "only the seller can cancel");
    anyhow::ensure!(listing.status == ListingStatus::Active, "listing is not active");

    listing.status = ListingStatus::Cancelled;
    chain.store.state_set(&listing_key(listing_id), &serde_json::to_vec(&listing)?)?;
    Ok(())
}

/// Sweep expired listings at epoch seal.
pub fn sweep_expired(chain: &Chain, epoch: u64) {
    for (key, raw) in chain.store.state_scan_prefix("session_listing:") {
        let Ok(mut listing) = serde_json::from_slice::<SessionListing>(&raw) else { continue };
        if listing.status != ListingStatus::Active { continue; }
        if epoch <= listing.expires_epoch { continue; }
        listing.status = ListingStatus::Expired;
        let _ = chain.store.state_set(&key, &serde_json::to_vec(&listing).unwrap_or_default());
    }
}
