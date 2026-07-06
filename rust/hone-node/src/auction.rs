//! Name and Freeport auction engine — shared logic for both auction types.

use anyhow::Result;
use serde::{Deserialize, Serialize};

use crate::chain::Chain;
use hone_types::NATIVE_TOKEN;

/// Name length tier for gating auction opens.
/// Returns the minimum bid in hunits based on character length.
pub fn name_min_bid(name: &str) -> u64 {
    match name.len() {
        1        => 10_000_000 * 100_000_000, // 10 M HONE — ultra-rare
        2        =>  1_000_000 * 100_000_000, // 1 M  HONE — premium
        3..=4    =>    100_000 * 100_000_000, // 100k HONE — standard
        5..=9    =>      1_000 * 100_000_000, // 1k   HONE — basic
        _        =>          0,               // 10+  — free to auction
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuctionState {
    pub auction_id: String,
    pub kind: AuctionKind,
    pub opener: String,
    pub min_bid: u64,
    pub end_epoch: u64,
    pub top_bidder: Option<String>,
    pub top_bid: u64,
    /// Previous top bidder whose escrowed funds must be refunded on overbid.
    pub prev_bidder: Option<String>,
    pub prev_bid: u64,
    pub settled: bool,
    pub cancelled: bool,
    pub opened_epoch: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum AuctionKind {
    Name { name: String },
    Freeport { item_type: String, item_id: String },
}

fn auction_key(id: &str) -> String {
    format!("auction:{}", id)
}

pub fn apply_name_open(
    chain: &Chain,
    auction_id: &str,
    name: &str,
    opener: &str,
    min_bid: u64,
    end_epoch: u64,
    epoch: u64,
) -> Result<()> {
    let effective_min = min_bid.max(name_min_bid(name));
    // Ensure name is not already registered or in active auction
    if chain.store.state_get(&format!("name_owner:{}", name)).is_some() {
        anyhow::bail!("name '{}' is already registered", name);
    }
    let state = AuctionState {
        auction_id: auction_id.to_string(),
        kind: AuctionKind::Name { name: name.to_string() },
        opener: opener.to_string(),
        min_bid: effective_min,
        end_epoch,
        top_bidder: None,
        top_bid: 0,
        prev_bidder: None,
        prev_bid: 0,
        settled: false,
        cancelled: false,
        opened_epoch: epoch,
    };
    chain.store.state_set(&auction_key(auction_id), &serde_json::to_vec(&state)?)?;
    chain.store.state_set(&format!("name_auction_active:{}", name), auction_id.as_bytes())?;
    Ok(())
}

pub fn apply_freeport_open(
    chain: &Chain,
    auction_id: &str,
    item_type: &str,
    item_id: &str,
    seller: &str,
    min_bid: u64,
    end_epoch: u64,
    epoch: u64,
) -> Result<()> {
    let state = AuctionState {
        auction_id: auction_id.to_string(),
        kind: AuctionKind::Freeport { item_type: item_type.to_string(), item_id: item_id.to_string() },
        opener: seller.to_string(),
        min_bid,
        end_epoch,
        top_bidder: None,
        top_bid: 0,
        prev_bidder: None,
        prev_bid: 0,
        settled: false,
        cancelled: false,
        opened_epoch: epoch,
    };
    chain.store.state_set(&auction_key(auction_id), &serde_json::to_vec(&state)?)?;
    Ok(())
}

pub fn apply_bid(
    chain: &Chain,
    auction_id: &str,
    bidder: &str,
    amount: u64,
    epoch: u64,
) -> Result<()> {
    let raw = chain.store.state_get(&auction_key(auction_id))
        .ok_or_else(|| anyhow::anyhow!("auction '{}' not found", auction_id))?;
    let mut state: AuctionState = serde_json::from_slice(&raw)?;

    anyhow::ensure!(!state.settled && !state.cancelled, "auction is closed");
    anyhow::ensure!(epoch <= state.end_epoch, "auction has ended");
    anyhow::ensure!(amount > state.top_bid && amount >= state.min_bid,
        "bid {} does not exceed current top bid {} or min bid {}",
        amount, state.top_bid, state.min_bid);

    // Debit bidder
    chain.store.debit(bidder, NATIVE_TOKEN, amount)?;

    // Refund previous top bidder
    if let Some(ref prev) = state.top_bidder.clone() {
        if state.top_bid > 0 {
            chain.store.credit(prev, NATIVE_TOKEN, state.top_bid)?;
        }
    }

    state.prev_bidder = state.top_bidder.clone();
    state.prev_bid = state.top_bid;
    state.top_bidder = Some(bidder.to_string());
    state.top_bid = amount;

    chain.store.state_set(&auction_key(auction_id), &serde_json::to_vec(&state)?)?;
    Ok(())
}

pub fn apply_settle(
    chain: &Chain,
    auction_id: &str,
    epoch: u64,
) -> Result<()> {
    let raw = chain.store.state_get(&auction_key(auction_id))
        .ok_or_else(|| anyhow::anyhow!("auction '{}' not found", auction_id))?;
    let mut state: AuctionState = serde_json::from_slice(&raw)?;

    anyhow::ensure!(!state.settled && !state.cancelled, "auction already closed");
    anyhow::ensure!(epoch > state.end_epoch, "auction has not ended yet (epoch {} <= end {})", epoch, state.end_epoch);

    state.settled = true;

    if let Some(ref winner) = state.top_bidder.clone() {
        // Transfer winning bid to opener
        let _ = chain.store.credit(&state.opener, NATIVE_TOKEN, state.top_bid);

        // Assign the name or item
        match &state.kind {
            AuctionKind::Name { name } => {
                let _ = chain.store.state_set(&format!("name_owner:{}", name), winner.as_bytes());
                let _ = chain.store.state_set(&format!("account_name:{}", winner), name.as_bytes());
                let _ = chain.store.state_delete(&format!("name_auction_active:{}", name));
            }
            AuctionKind::Freeport { item_type, item_id } => {
                let ownership_key = format!("freeport_owner:{}:{}", item_type, item_id);
                let _ = chain.store.state_set(&ownership_key, winner.as_bytes());
            }
        }
    }

    chain.store.state_set(&auction_key(auction_id), &serde_json::to_vec(&state)?)?;
    Ok(())
}

pub fn apply_cancel(
    chain: &Chain,
    auction_id: &str,
    requester: &str,
) -> Result<()> {
    let raw = chain.store.state_get(&auction_key(auction_id))
        .ok_or_else(|| anyhow::anyhow!("auction '{}' not found", auction_id))?;
    let mut state: AuctionState = serde_json::from_slice(&raw)?;

    anyhow::ensure!(state.opener == requester, "only the opener can cancel");
    anyhow::ensure!(!state.settled && !state.cancelled, "auction already closed");
    anyhow::ensure!(state.top_bidder.is_none(), "cannot cancel — bids already received");

    state.cancelled = true;
    chain.store.state_set(&auction_key(auction_id), &serde_json::to_vec(&state)?)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use hone_types::LedgerEntry;
    use tempfile::TempDir;

    fn make_chain() -> (Chain, TempDir) {
        let dir = tempfile::TempDir::new().unwrap();
        let store = crate::store::Store::open(dir.path()).unwrap();
        let chain = Chain::new(
            store, "test".to_string(), "hone-test".to_string(),
        );
        (chain, dir)
    }

    fn fund(chain: &Chain, account: &str, amount: u64) {
        chain.apply_entry(&LedgerEntry::AccountCreate {
            account: account.to_string(),
            keys: Default::default(),
            chain_proofs: vec![],
            epoch: 0,
            funded_by: None,
            machine_fingerprint: None,
        }).ok();
        chain.apply_entry(&LedgerEntry::GenesisAlloc {
            account: account.to_string(),
            amount,
            token: hone_types::NATIVE_TOKEN.to_string(),
        }).unwrap();
    }

    #[test]
    fn name_auction_open_stores_auction() {
        let (chain, _dir) = make_chain();
        apply_name_open(&chain, "auc1", "hello", "alice", 0, 10, 1).unwrap();

        let raw = chain.store.state_get(&auction_key("auc1")).unwrap();
        let state: AuctionState = serde_json::from_slice(&raw).unwrap();
        assert_eq!(state.auction_id, "auc1");
        assert!(!state.settled);
        assert!(!state.cancelled);
        assert_eq!(state.top_bidder, None);
    }

    #[test]
    fn name_auction_bid_updates_high_bid() {
        let (chain, _dir) = make_chain();
        fund(&chain, "bidder", 5_000_000_000_000);

        apply_name_open(&chain, "auc2", "helloworld", "alice", 0, 10, 1).unwrap();
        apply_bid(&chain, "auc2", "bidder", 1_000, 2).unwrap();

        let raw = chain.store.state_get(&auction_key("auc2")).unwrap();
        let state: AuctionState = serde_json::from_slice(&raw).unwrap();
        assert_eq!(state.top_bidder, Some("bidder".to_string()));
        assert_eq!(state.top_bid, 1_000);
    }

    #[test]
    fn name_auction_bid_below_min_fails() {
        let (chain, _dir) = make_chain();
        fund(&chain, "bidder", 5_000_000_000_000);

        // "helloworld" (10+ chars) has name_min_bid=0, but we set min_bid=5_000
        apply_name_open(&chain, "auc3", "helloworld", "alice", 5_000, 10, 1).unwrap();
        // Bid 1_000 < min_bid 5_000 — should fail
        let res = apply_bid(&chain, "auc3", "bidder", 1_000, 2);
        assert!(res.is_err(), "bid below min_bid should fail");
    }

    #[test]
    fn name_auction_settle_awards_winner() {
        let (chain, _dir) = make_chain();
        fund(&chain, "bidder", 50_000_000);
        fund(&chain, "opener", 0);

        apply_name_open(&chain, "auc4", "helloworld", "opener", 0, 5, 1).unwrap();
        apply_bid(&chain, "auc4", "bidder", 10_000_000, 2).unwrap();
        apply_settle(&chain, "auc4", 6).unwrap();

        let raw = chain.store.state_get(&auction_key("auc4")).unwrap();
        let state: AuctionState = serde_json::from_slice(&raw).unwrap();
        assert!(state.settled);

        // Winner should own the name
        let owner = chain.store.state_get("name_owner:helloworld").unwrap();
        assert_eq!(owner, b"bidder");
    }

    #[test]
    fn name_auction_cancel_closes_auction() {
        let (chain, _dir) = make_chain();
        apply_name_open(&chain, "auc5", "helloworld", "opener", 0, 10, 1).unwrap();
        apply_cancel(&chain, "auc5", "opener").unwrap();

        let raw = chain.store.state_get(&auction_key("auc5")).unwrap();
        let state: AuctionState = serde_json::from_slice(&raw).unwrap();
        assert!(state.cancelled);
    }

    #[test]
    fn freeport_open_stores_auction() {
        let (chain, _dir) = make_chain();
        apply_freeport_open(&chain, "fp1", "nft", "item42", "seller", 0, 10, 1).unwrap();

        let raw = chain.store.state_get(&auction_key("fp1")).unwrap();
        let state: AuctionState = serde_json::from_slice(&raw).unwrap();
        assert_eq!(state.auction_id, "fp1");
        assert!(!state.settled);
        assert!(!state.cancelled);
        match &state.kind {
            AuctionKind::Freeport { item_type, item_id } => {
                assert_eq!(item_type, "nft");
                assert_eq!(item_id, "item42");
            }
            _ => panic!("wrong auction kind"),
        }
    }

    #[test]
    fn freeport_bid_updates_high_bid() {
        let (chain, _dir) = make_chain();
        fund(&chain, "buyer", 5_000_000_000_000);

        apply_freeport_open(&chain, "fp2", "nft", "item43", "seller", 0, 10, 1).unwrap();
        apply_bid(&chain, "fp2", "buyer", 2_000_000, 2).unwrap();

        let raw = chain.store.state_get(&auction_key("fp2")).unwrap();
        let state: AuctionState = serde_json::from_slice(&raw).unwrap();
        assert_eq!(state.top_bidder, Some("buyer".to_string()));
        assert_eq!(state.top_bid, 2_000_000);
    }
}
