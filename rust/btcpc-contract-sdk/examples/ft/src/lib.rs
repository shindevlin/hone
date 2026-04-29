/*!
# BSP-20 Fungible Token

Reference implementation of the BTCPC Standard Proposal 20 (BSP-20)
fungible token interface. Mirrors ERC-20 semantics on BTCPC.

## Deploy

```bash
btcpc contract deploy ft.wasm \
  --args '{"name":"MyToken","symbol":"MTK","decimals":8,"total_supply":"1000000000000000"}' \
  --account natoshisakamoto
```

## Call

```bash
btcpc contract call BTCPCsc<address> transfer \
  --args '{"to":"alice","amount":"500000000"}' \
  --account natoshisakamoto
```

## View

```bash
btcpc contract view BTCPCsc<address> balance_of \
  --args '{"account":"alice"}'
```
*/

use btcpc_contract_sdk::*;
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};

/// BSP-20 standard interface (all methods are required for compliance).
/// Contracts implementing this must also emit BSP-20 events.
pub trait Bsp20 {
    fn ft_transfer(&mut self, receiver_id: AccountId, amount: String, memo: Option<String>);
    fn ft_transfer_call(&mut self, receiver_id: AccountId, amount: String, memo: Option<String>, msg: String) -> String;
    fn ft_total_supply(&self) -> String;
    fn ft_balance_of(&self, account_id: AccountId) -> String;
    fn ft_metadata(&self) -> FtMetadata;
}

#[derive(BorshSerialize, BorshDeserialize, Serialize, Deserialize, Clone)]
pub struct FtMetadata {
    pub name: String,
    pub symbol: String,
    pub decimals: u8,
    pub icon: Option<String>,
    pub reference: Option<String>,
}

// ── Contract State ───────────────────────────────────────────────────────────

#[btcpc_contract]
pub struct FungibleToken {
    pub metadata: FtMetadata,
    pub total_supply: u128,
    pub accounts: LookupMap<AccountId, u128>,
    pub allowances: LookupMap<(AccountId, AccountId), u128>,
}

// ── Methods ──────────────────────────────────────────────────────────────────

#[btcpc_impl]
impl FungibleToken {
    /// Deploy a new fungible token.
    /// `total_supply` is in the token's smallest unit (considering `decimals`).
    #[init]
    pub fn new(
        name: String,
        symbol: String,
        decimals: u8,
        total_supply: String,
    ) -> Self {
        let supply: u128 = total_supply.parse().unwrap_or(0);
        let owner = env::signer();

        let mut accounts = LookupMap::new(b"a");
        accounts.insert(&owner, &supply);

        let metadata = FtMetadata { name, symbol, decimals, icon: None, reference: None };

        emit!(&event::TransferEvent::new("".to_string(), owner.clone(), supply));

        Self {
            metadata,
            total_supply: supply,
            accounts,
            allowances: LookupMap::new(b"l"),
        }
    }

    /// Transfer `amount` tokens to `receiver_id`.
    #[call]
    pub fn ft_transfer(&mut self, receiver_id: AccountId, amount: String, memo: Option<String>) {
        let sender = env::signer();
        let amount: u128 = amount.parse().unwrap_or(0);
        require!(amount > 0, "Amount must be greater than zero");

        let sender_balance = self.accounts.get(&sender).unwrap_or(0);
        require!(sender_balance >= amount, "Insufficient token balance");

        self.accounts.insert(&sender, &(sender_balance - amount));
        let receiver_balance = self.accounts.get(&receiver_id).unwrap_or(0);
        self.accounts.insert(&receiver_id, &(receiver_balance + amount));

        emit!(&event::TransferEvent::new(sender, receiver_id, amount));
        log!("Transferred {} tokens | memo: {:?}", amount, memo);
    }

    /// Transfer `amount` tokens to `receiver_id` and call `ft_on_transfer` on it.
    #[call]
    pub fn ft_transfer_call(
        &mut self,
        receiver_id: AccountId,
        amount: String,
        memo: Option<String>,
        msg: String,
    ) -> String {
        self.ft_transfer(receiver_id.clone(), amount.clone(), memo);
        // Schedule cross-contract callback
        promise::Promise::new(&receiver_id)
            .call(
                "ft_on_transfer",
                serde_json::json!({
                    "sender_id": env::signer(),
                    "amount": amount,
                    "msg": msg,
                }),
                0,
                promise::gas::CALLBACK,
            );
        amount
    }

    /// Total token supply (as string to avoid JS u128 precision loss).
    #[view]
    pub fn ft_total_supply(&self) -> String {
        self.total_supply.to_string()
    }

    /// Token balance of `account_id`.
    #[view]
    pub fn ft_balance_of(&self, account_id: AccountId) -> String {
        self.accounts.get(&account_id).unwrap_or(0).to_string()
    }

    /// Token metadata.
    #[view]
    pub fn ft_metadata(&self) -> FtMetadata {
        self.metadata.clone()
    }

    // ── ERC-20-style allowance extension ─────────────────────────────────────

    /// Approve `spender` to transfer up to `amount` on your behalf.
    #[call]
    pub fn approve(&mut self, spender: AccountId, amount: String) {
        let owner = env::signer();
        let amount: u128 = amount.parse().unwrap_or(0);
        self.allowances.insert(&(owner, spender), &amount);
    }

    /// Transfer on behalf of `owner` (requires prior approval).
    #[call]
    pub fn transfer_from(&mut self, owner: AccountId, receiver: AccountId, amount: String) {
        let spender = env::signer();
        let amount: u128 = amount.parse().unwrap_or(0);

        let allowance = self.allowances.get(&(owner.clone(), spender.clone())).unwrap_or(0);
        require!(allowance >= amount, "Transfer exceeds allowance");

        let owner_balance = self.accounts.get(&owner).unwrap_or(0);
        require!(owner_balance >= amount, "Insufficient balance");

        self.allowances.insert(&(owner.clone(), spender), &(allowance - amount));
        self.accounts.insert(&owner, &(owner_balance - amount));
        let receiver_balance = self.accounts.get(&receiver).unwrap_or(0);
        self.accounts.insert(&receiver, &(receiver_balance + amount));

        emit!(&event::TransferEvent::new(owner, receiver, amount));
    }

    /// Allowance remaining for `spender` from `owner`.
    #[view]
    pub fn allowance(&self, owner: AccountId, spender: AccountId) -> String {
        self.allowances.get(&(owner, spender)).unwrap_or(0).to_string()
    }
}
