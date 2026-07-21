# BSP-20: Fungible Token Standard

| Field | Value |
|-------|-------|
| **BSP** | 20 |
| **Title** | Fungible Token Standard |
| **Status** | Final |
| **Category** | Token Standard |
| **Created** | Epoch 1 |
| **Updated** | 2026-04-28 |
| **Equivalent** | ERC-20 (Ethereum), SPL Token (Solana) |

---

## Abstract

BSP-20 defines a standard interface for fungible tokens on the HONE blockchain. A fungible token is one where each unit is interchangeable with any other unit of the same token — every smallest unit of a BSP-20 token has equal value and equal rights.

This standard enables a single, predictable API for token contracts so that wallets, DEXes, explorers, and other on-chain contracts can interact with any BSP-20 token without knowing its implementation details.

---

## Motivation

Without a shared standard, every fungible token would expose a different API. Integrators would need custom code for each token contract. BSP-20 solves this by specifying:

- A mandatory set of callable methods with defined signatures and semantics.
- A mandatory set of on-chain events that off-chain indexers can parse.
- An optional allowance extension that enables the "approve + transfer-from" delegation pattern used by DEXes and protocols.

The interface is modeled on Ethereum's ERC-20 with modifications for HONE's architecture: amounts are stringified `u128` values to avoid JavaScript's 53-bit float precision limit, and transfer-call uses HONE's native cross-contract Promise API rather than a callback return value.

---

## Specification

The key words "MUST", "MUST NOT", "REQUIRED", "SHALL", "SHOULD", "MAY" follow [RFC 2119](https://www.rfc-editor.org/rfc/rfc2119).

### Types

```rust
pub type AccountId = String;

pub struct FtMetadata {
    pub name:      String,         // Human-readable token name
    pub symbol:    String,         // Uppercase ticker symbol (e.g. "MTK")
    pub decimals:  u8,             // Decimal places (8 recommended for HONE compatibility)
    pub icon:      Option<String>, // Data URL (e.g. "data:image/svg+xml,...")
    pub reference: Option<String>, // HTTPS URL to a JSON metadata document
}
```

### Required Methods

All BSP-20-compliant contracts MUST implement the following methods. Methods marked `#[call]` modify state and cost EB. Methods marked `#[view]` are read-only and may be called off-chain for free.

---

#### `ft_transfer`

```rust
#[call]
pub fn ft_transfer(
    &mut self,
    receiver_id: AccountId,
    amount:      String,     // Stringified u128, in the token's smallest unit
    memo:        Option<String>,
)
```

Transfer `amount` tokens from the transaction signer to `receiver_id`.

**Requirements:**
- MUST decrease the signer's balance by `amount`.
- MUST increase `receiver_id`'s balance by `amount`.
- MUST panic if the signer's balance is less than `amount`.
- MUST panic if `amount` is zero.
- MUST panic if `receiver_id == signer`.
- MUST emit a `Transfer` event.
- `memo` MAY be logged but MUST NOT affect state.

---

#### `ft_transfer_call`

```rust
#[call]
pub fn ft_transfer_call(
    &mut self,
    receiver_id: AccountId,
    amount:      String,
    memo:        Option<String>,
    msg:         String,
) -> String
```

Transfer `amount` tokens to `receiver_id` and schedule a cross-contract call to `receiver_id.ft_on_transfer(sender_id, amount, msg)`.

This method enables protocols (AMMs, lending markets, staking contracts) to receive tokens and act on them atomically within a transaction.

**Requirements:**
- MUST execute a full `ft_transfer` first (all transfer requirements apply).
- MUST schedule a `Promise::new(receiver_id).call("ft_on_transfer", ...)` with the following arguments:
  - `sender_id`: the original transaction signer
  - `amount`: the transferred amount (string)
  - `msg`: the caller-supplied `msg`
- MUST return the transferred `amount` as a string.
- Phase 2: If `ft_on_transfer` returns a non-zero refund amount, the contract SHOULD refund that amount back to the sender via a callback.

The `ft_on_transfer` interface on the receiving contract:

```rust
pub fn ft_on_transfer(
    &mut self,
    sender_id: AccountId,
    amount:    String,
    msg:       String,
) -> String  // Unused refund amount (Phase 2)
```

---

#### `ft_total_supply`

```rust
#[view]
pub fn ft_total_supply(&self) -> String
```

Returns the total token supply as a stringified `u128`. The total supply MUST NOT exceed `u128::MAX`.

**Requirements:**
- MUST return the sum of all outstanding token balances.
- MAY differ from the initial supply if the contract supports minting or burning.

---

#### `ft_balance_of`

```rust
#[view]
pub fn ft_balance_of(&self, account_id: AccountId) -> String
```

Returns the token balance of `account_id` as a stringified `u128`.

**Requirements:**
- MUST return `"0"` for accounts that have never held this token.
- MUST NOT panic for any `account_id`.

---

#### `ft_metadata`

```rust
#[view]
pub fn ft_metadata(&self) -> FtMetadata
```

Returns the token's metadata.

**Requirements:**
- `name` and `symbol` MUST be non-empty strings.
- `decimals` MUST accurately reflect the number of decimal places. 10 is RECOMMENDED for native HONE compatibility (1 HONE = 10^10 dreams).
- The metadata SHOULD be immutable after deployment.

---

### Optional Allowance Extension

Implementing the allowance extension is RECOMMENDED for any token that may be used in DeFi protocols. Contracts that implement it MUST implement all three methods.

#### `approve`

```rust
#[call]
pub fn approve(&mut self, spender: AccountId, amount: String)
```

Grant `spender` permission to transfer up to `amount` tokens from the signer's balance using `transfer_from`.

**Requirements:**
- MUST set the allowance for `(signer, spender)` to `amount`.
- Setting `amount` to `"0"` MUST revoke the allowance.
- MUST emit an `Approval` event.
- Calling `approve` again MUST overwrite the previous allowance (not add to it).

---

#### `transfer_from`

```rust
#[call]
pub fn transfer_from(
    &mut self,
    owner:    AccountId,
    receiver: AccountId,
    amount:   String,
)
```

Transfer `amount` tokens from `owner` to `receiver` using the signer's pre-approved allowance.

**Requirements:**
- MUST panic if `allowance(owner, signer)` < `amount`.
- MUST decrease `allowance(owner, signer)` by `amount`.
- MUST decrease `owner`'s balance by `amount`.
- MUST increase `receiver`'s balance by `amount`.
- MUST panic if `owner`'s balance < `amount`.
- MUST emit a `Transfer` event with `from = owner`.

---

#### `allowance`

```rust
#[view]
pub fn allowance(&self, owner: AccountId, spender: AccountId) -> String
```

Returns the remaining allowance that `owner` has granted to `spender`, as a stringified `u128`.

**Requirements:**
- MUST return `"0"` if no allowance has been set.
- MUST NOT panic for any `(owner, spender)` pair.

---

### Required Events

BSP-20 contracts MUST emit JSON events in the following format. Events are emitted via `event::emit_event(&payload)` and are indexed by the chain explorer.

#### Transfer Event

Emitted whenever tokens move from one account to another, including minting (from = `""`) and burning (to = `""`).

```json
{
  "standard": "bsp-20",
  "version":  "1.0.0",
  "event":    "ft_transfer",
  "from":     "alice",
  "to":       "bob",
  "amount":   1000000000
}
```

| Field | Type | Description |
|-------|------|-------------|
| `standard` | `string` | Always `"bsp-20"` |
| `version` | `string` | Always `"1.0.0"` |
| `event` | `string` | Always `"ft_transfer"` |
| `from` | `string` | Sender account. Empty string `""` for mint events |
| `to` | `string` | Receiver account. Empty string `""` for burn events |
| `amount` | `number` | Token amount in the smallest unit (u128) |

#### Approval Event

Emitted whenever an allowance is set via `approve`.

```json
{
  "standard": "bsp-20",
  "version":  "1.0.0",
  "event":    "ft_approve",
  "owner":    "alice",
  "spender":  "dex.contract",
  "amount":   "5000000000"
}
```

| Field | Type | Description |
|-------|------|-------------|
| `standard` | `string` | Always `"bsp-20"` |
| `version` | `string` | Always `"1.0.0"` |
| `event` | `string` | Always `"ft_approve"` |
| `owner` | `string` | Token owner granting the allowance |
| `spender` | `string` | Account approved to spend |
| `amount` | `string` | New allowance (stringified u128) |

---

### Error Conditions

Implementations MUST panic (reverting all state changes) in the following cases:

| Condition | Required Panic Message |
|-----------|----------------------|
| `ft_transfer` with zero amount | `"amount must be greater than zero"` |
| `ft_transfer` with insufficient balance | `"insufficient token balance"` |
| `ft_transfer` sender == receiver | `"sender and receiver must differ"` |
| `transfer_from` with insufficient allowance | `"transfer amount exceeds allowance"` |
| `transfer_from` with insufficient balance | `"insufficient balance"` |

The exact panic message wording is RECOMMENDED but not REQUIRED. Wallets and explorers parse error messages for user display; consistent wording improves UX.

---

## Rationale

### Amounts as Strings

`u128` values cannot be represented precisely in JSON numbers (JavaScript's `Number` type is a 64-bit float, giving only 53 bits of integer precision). Stringifying balances avoids silent precision loss in browsers and JavaScript-based wallets. Implementations MUST accept and return stringified amounts for all balance fields.

### Separate ft_transfer and ft_transfer_call

`ft_transfer_call` exists because HONE's execution model (WASM, synchronous per call, async cross-contract via Promise) requires that transfer notification be an explicit cross-contract call rather than an inline callback. Separating the two methods makes the gas cost transparent to callers.

### Optional Allowance Extension

Allowances add two storage writes per approval (one for the allowance value). On-chain programs that do not require delegation (e.g., a simple airdrop contract) should not be penalized with mandatory allowance storage. The allowance extension is therefore optional, though RECOMMENDED.

---

## Implementation Reference

The canonical BSP-20 implementation is in [`examples/ft/src/lib.rs`](../examples/ft/src/lib.rs).

To build and test it:

```bash
# Build the WASM
cargo build -p ft --target wasm32-unknown-unknown --release

# The binary will be at:
# target/wasm32-unknown-unknown/release/ft.wasm

# Deploy to chain
hone contract deploy target/wasm32-unknown-unknown/release/ft.wasm \
  --args '{"name":"TestToken","symbol":"TST","decimals":8,"total_supply":"10000000000000000"}' \
  --account myaccount
```

---

## Changelog

| Date | Change |
|------|--------|
| 2026-04-28 | Initial final specification |
