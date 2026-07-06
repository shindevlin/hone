//! BSP-721: Non-Fungible Token standard for HONE
//!
//! Each NFT is identified by a (collection, token_id) pair.
//! Supports: mint, transfer, approve, approve_all, revoke, metadata, royalties.

use hone_contract_sdk::*;
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};

#[derive(BorshSerialize, BorshDeserialize, Serialize, Deserialize, Clone, Debug)]
pub struct TokenMetadata {
    pub title: String,
    pub description: Option<String>,
    pub media_url: Option<String>,
    pub attributes: Option<String>,
    pub royalty_percent: u8, // 0-100 (basis points × 100, max 10%)
}

#[derive(BorshSerialize, BorshDeserialize, Serialize, Deserialize, Clone, Debug)]
pub struct Token {
    pub token_id: String,
    pub owner: AccountId,
    pub metadata: TokenMetadata,
    pub minted_epoch: Epoch,
    pub approved: Option<AccountId>,
    pub transferable: bool,
    pub soulbound: bool,
}

#[hone_contract]
pub struct NftCollection {
    pub name: String,
    pub symbol: String,
    pub owner: AccountId,
    pub base_uri: Option<String>,
    pub tokens: LookupMap<String, Token>,        // token_id → Token
    pub owner_tokens: LookupMap<String, Vec<String>>, // owner → [token_id, ...]
    pub approve_all: LookupMap<String, Vec<AccountId>>, // owner → [operator, ...]
    pub total_supply: u64,
}

#[hone_impl]
impl NftCollection {
    /// Initialize the NFT collection.
    #[init]
    pub fn new(name: String, symbol: String, base_uri: Option<String>) -> Self {
        Self {
            name,
            symbol,
            owner: env::signer(),
            base_uri,
            tokens: LookupMap::new(b"t"),
            owner_tokens: LookupMap::new(b"o"),
            approve_all: LookupMap::new(b"a"),
            total_supply: 0,
        }
    }

    /// Mint a new NFT to `receiver`.
    #[call]
    pub fn nft_mint(
        &mut self,
        token_id: String,
        receiver: AccountId,
        metadata: TokenMetadata,
        transferable: bool,
        soulbound: bool,
    ) {
        assert_eq!(env::signer(), self.owner, "only collection owner can mint");
        assert!(metadata.royalty_percent <= 10, "royalty max 10%");
        assert!(self.tokens.get(&token_id).is_none(), "token_id already exists");

        let epoch = env::epoch();
        let token = Token {
            token_id: token_id.clone(),
            owner: receiver.clone(),
            metadata,
            minted_epoch: epoch,
            approved: None,
            transferable,
            soulbound,
        };

        self.tokens.insert(&token_id, &token);

        let mut owner_list = self.owner_tokens.get(&receiver).unwrap_or_default();
        owner_list.push(token_id.clone());
        self.owner_tokens.insert(&receiver, &owner_list);

        self.total_supply += 1;

        event::emit_event(&serde_json::json!({
            "standard": "bsp721",
            "event": "nft_mint",
            "data": { "token_id": token_id, "owner": receiver, "epoch": epoch }
        }));
    }

    /// Transfer an NFT to a new owner.
    #[call]
    pub fn nft_transfer(&mut self, token_id: String, receiver: AccountId, memo: Option<String>) {
        let mut token = self.tokens.get(&token_id).expect("token not found");
        assert!(!token.soulbound, "token is soulbound");
        assert!(token.transferable, "token is non-transferable");

        let signer = env::signer();
        let is_owner = signer == token.owner;
        let is_approved = token.approved.as_deref() == Some(&signer);
        let is_operator = self.approve_all
            .get(&token.owner)
            .map(|ops: Vec<AccountId>| ops.contains(&signer))
            .unwrap_or(false);
        assert!(is_owner || is_approved || is_operator, "not authorized");

        let old_owner = token.owner.clone();
        token.owner = receiver.clone();
        token.approved = None;
        self.tokens.insert(&token_id, &token);

        // Update owner index
        let mut old_list = self.owner_tokens.get(&old_owner).unwrap_or_default();
        old_list.retain(|t| t != &token_id);
        self.owner_tokens.insert(&old_owner, &old_list);

        let mut new_list = self.owner_tokens.get(&receiver).unwrap_or_default();
        new_list.push(token_id.clone());
        self.owner_tokens.insert(&receiver, &new_list);

        event::emit_event(&serde_json::json!({
            "standard": "bsp721",
            "event": "nft_transfer",
            "data": {
                "token_id": token_id,
                "old_owner": old_owner,
                "new_owner": receiver,
                "memo": memo
            }
        }));
    }

    /// Approve an account to transfer a specific token.
    #[call]
    pub fn nft_approve(&mut self, token_id: String, approved: Option<AccountId>) {
        let mut token = self.tokens.get(&token_id).expect("token not found");
        assert_eq!(env::signer(), token.owner, "only token owner");
        token.approved = approved.clone();
        self.tokens.insert(&token_id, &token);
    }

    /// Approve an operator for all tokens owned by caller.
    #[call]
    pub fn nft_approve_all(&mut self, operator: AccountId, approve: bool) {
        let signer = env::signer();
        let mut ops = self.approve_all.get(&signer).unwrap_or_default();
        if approve {
            if !ops.contains(&operator) { ops.push(operator); }
        } else {
            ops.retain(|o| o != &operator);
        }
        self.approve_all.insert(&signer, &ops);
    }

    // ── Views ────────────────────────────────────────────────────────────────

    #[view]
    pub fn nft_token(&self, token_id: String) -> Option<Token> {
        self.tokens.get(&token_id)
    }

    #[view]
    pub fn nft_tokens_for_owner(&self, account: AccountId) -> Vec<String> {
        self.owner_tokens.get(&account).unwrap_or_default()
    }

    #[view]
    pub fn nft_total_supply(&self) -> u64 {
        self.total_supply
    }

    #[view]
    pub fn nft_metadata(&self) -> serde_json::Value {
        serde_json::json!({
            "name": self.name,
            "symbol": self.symbol,
            "base_uri": self.base_uri,
            "total_supply": self.total_supply
        })
    }

    #[view]
    pub fn nft_is_approved(&self, token_id: String, account: AccountId) -> bool {
        let token = match self.tokens.get(&token_id) { Some(t) => t, None => return false };
        if token.approved.as_deref() == Some(&account) { return true; }
        self.approve_all
            .get(&token.owner)
            .map(|ops: Vec<AccountId>| ops.contains(&account))
            .unwrap_or(false)
    }
}
