# BTCPC Smart Contract Developer Guide

> **Version 1.0** — SDK `btcpc-contract-sdk = "1.0"` — Chain epoch model, wasmtime sandbox

---

## Table of Contents

1. [Introduction](#1-introduction)
2. [Getting Started](#2-getting-started)
3. [Contract Structure](#3-contract-structure)
4. [Storage](#4-storage)
5. [Environment API](#5-environment-api)
6. [Events](#6-events)
7. [Cross-Contract Calls](#7-cross-contract-calls)
8. [Gas and EB](#8-gas-and-eb)
9. [BSP Standards](#9-bsp-standards)
10. [Security](#10-security)
11. [Full Example: Fungible Token](#11-full-example-fungible-token)

---

## 1. Introduction

### What are BTCPC smart contracts?

BTCPC smart contracts are programs that run directly on the BTCPC blockchain. They are written in **Rust**, compiled to **WebAssembly (WASM)**, deployed on-chain, and called by submitting signed transactions. Once deployed, a contract's code is immutable and its execution is deterministic.

Contracts on BTCPC are first-class citizens of the network — they can hold BTCPC balances, own on-chain assets, emit indexed events, and call other contracts.

### WASM Execution Model

Every contract is compiled to the `wasm32-unknown-unknown` target and stored as a WASM binary in the chain state. When a method is called, the BTCPC node:

1. Loads the contract WASM into a fresh **wasmtime** sandbox.
2. Injects host functions (the Environment API) as WASM imports.
3. Calls the single exported entry point: `__btcpc_dispatch()`.
4. The dispatch function reads the method name and JSON arguments from host registers, calls the correct Rust method, writes mutated state back to storage, and returns any result value.
5. All state writes are atomic — a panic reverts everything.

The sandbox has no access to the filesystem, network, or system clock. All external information comes through the Environment API.

### State Model

Each contract owns an isolated key-value store. The SDK's collection types (`LookupMap`, `UnorderedMap`, `Vector`, `LazyOption`) provide ergonomic access backed by this store. The contract struct itself is serialized to Borsh and stored under the reserved key `__state`.

State persists across calls. There is no garbage collection — storage you write, you pay for, and it remains until you explicitly delete it.

### Gas Model

Every operation in a contract call consumes **EB (Epoch Bandwidth)** — BTCPC's unit of computational cost.

| Unit | Equivalent |
|------|-----------|
| 1 EB | 1,000,000 WASM fuel units |
| 1 fuel unit | 1 WASM instruction (approximately) |

Gas is prepaid by the caller and deducted from their account. If execution exhausts the gas limit, the call panics and all state changes are reverted. The caller is still charged for gas used up to the panic point.

---

## 2. Getting Started

### Prerequisites

- Rust stable (1.75+)
- The `wasm32-unknown-unknown` compilation target
- The BTCPC CLI (for deployment)

```bash
# Install Rust
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# Add the WASM target
rustup target add wasm32-unknown-unknown
```

### Create a New Contract Project

```bash
cargo new --lib my-contract
cd my-contract
```

Edit `Cargo.toml`:

```toml
[package]
name = "my-contract"
version = "0.1.0"
edition = "2021"

[lib]
crate-type = ["cdylib"]  # Required: compile to a dynamic library for WASM

[dependencies]
btcpc-contract-sdk = "1.0"
borsh = { version = "1", features = ["derive"] }
serde = { version = "1", features = ["derive"] }
serde_json = "1"

[profile.release]
opt-level = "z"        # Optimize for size
lto = true
codegen-units = 1
panic = "abort"        # Smaller WASM — panics abort rather than unwind
```

### Hello World: A Counter Contract

```rust
use btcpc_contract_sdk::*;

#[btcpc_contract]
pub struct Counter {
    count: u64,
    owner: AccountId,
}

#[btcpc_impl]
impl Counter {
    /// Deploy a new counter. Called once at deploy time.
    #[init]
    pub fn new(start: u64) -> Self {
        Self {
            count: start,
            owner: env::signer(),
        }
    }

    /// Increment the counter by 1.
    #[call]
    pub fn increment(&mut self) {
        self.count = self.count.saturating_add(1);
        log!("Counter is now {}", self.count);
    }

    /// Reset the counter to zero (owner only).
    #[call]
    pub fn reset(&mut self) {
        require!(env::signer() == self.owner, "only owner can reset");
        self.count = 0;
    }

    /// Read the current count without modifying state.
    #[view]
    pub fn get(&self) -> u64 {
        self.count
    }
}
```

### Build

```bash
cargo build --target wasm32-unknown-unknown --release
```

The compiled WASM is at `target/wasm32-unknown-unknown/release/my_contract.wasm`.

### Deploy via API

```bash
btcpc contract deploy my_contract.wasm \
  --args '{"start": 0}' \
  --account myaccount
```

On success, the CLI prints the contract address (`BTCPCsc…`). Save it — you will need it to call the contract.

### Call and View

```bash
# State-mutating call
btcpc contract call BTCPCsc<address> increment --account myaccount

# Read-only view (free, no gas)
btcpc contract view BTCPCsc<address> get
```

---

## 3. Contract Structure

### `#[btcpc_contract]` — The State Struct

Place this attribute on the struct that holds all persistent contract state. Under the hood it derives `BorshSerialize` and `BorshDeserialize` and generates `__load()` / `__save()` helpers that read and write the entire struct from the `__state` storage key.

```rust
#[btcpc_contract]
pub struct MyContract {
    owner: AccountId,
    value: u64,
    balances: LookupMap<AccountId, u128>,
}
```

Every field must implement `BorshSerialize + BorshDeserialize`. All SDK collection types (`LookupMap`, `UnorderedMap`, `Vector`, `LazyOption`) satisfy this requirement.

### `#[btcpc_impl]` — The Dispatch Table

Place this attribute on the `impl` block that defines your contract's callable methods. It generates the `__btcpc_dispatch()` WASM entry point that routes incoming calls to the correct Rust method.

```rust
#[btcpc_impl]
impl MyContract {
    // methods go here
}
```

There must be exactly one `#[btcpc_impl]` block per contract.

### `#[init]` — Constructor

The `#[init]` method is called exactly once when the contract is deployed. It must return `Self`. Arguments are passed as a JSON object.

```rust
#[init]
pub fn new(owner: AccountId, initial_value: u64) -> Self {
    Self {
        owner,
        value: initial_value,
        balances: LookupMap::new(b"b"),
    }
}
```

Calling a non-init method before `#[init]` has run will panic: `"Contract state not initialized. Call init first."`.

### `#[call]` — State-Mutating Method

`#[call]` methods take `&mut self` and may modify contract state. After a call method returns, the mutated state is saved back to storage automatically.

```rust
#[call]
pub fn set_value(&mut self, new_value: u64) {
    self.value = new_value;
}
```

Arguments are passed as a JSON object by the caller. Return values (if any) are JSON-serialized and returned to the caller.

### `#[view]` — Read-Only Method

`#[view]` methods take `&self` and cannot modify state. They are cheaper to execute (5 EB base cost vs 500,000 fuel for a call). Views are free for the reader when called as an off-chain query.

```rust
#[view]
pub fn get_value(&self) -> u64 {
    self.value
}
```

The runtime will reject any attempt to call `env::storage_write` from inside a view method.

### `#[private]` — Internal Callbacks

`#[private]` methods can only be called by the contract itself (i.e., `env::predecessor() == env::current_contract()`). This is used for cross-contract callback handlers.

```rust
#[private]
#[call]
pub fn on_transfer_complete(&mut self, result: serde_json::Value) {
    // handle the result of a cross-contract call
}
```

The `#[private]` macro inserts an automatic predecessor check; any external caller will receive a panic.

---

## 4. Storage

All storage collections use **Borsh serialization** for both keys and values. Borsh is a deterministic binary format — the same Rust value always produces the same bytes, making it safe for hashing and storage keys.

### Storage Costs

| Operation | Fuel Cost |
|-----------|-----------|
| Read (hit) | 50,000 fuel |
| Read (miss) | 5,000 fuel |
| Write (new key) | 100,000 fuel |
| Write (existing key) | 100,000 fuel |
| Remove | 40,000 fuel |

Minimize writes by batching state changes within a single call.

### Choosing the Right Collection

| Collection | Iteration | Gas | Use When |
|-----------|-----------|-----|----------|
| `LookupMap<K, V>` | No | Lowest | Pure key lookups (balances, allowances) |
| `UnorderedMap<K, V>` | Yes (keys snapshot) | Higher | Enumerable registries |
| `Vector<V>` | Yes (index range) | Low | Append-only ordered sequences |
| `LazyOption<V>` | N/A | Lowest | Single optional config or metadata value |

### `LookupMap<K, V>`

A hash map backed by the host KV store. Keys are not enumerable. O(1) read and write.

```rust
// Initialize with a unique prefix (prevents key collisions with other collections)
let mut balances: LookupMap<AccountId, u128> = LookupMap::new(b"bal");

// Write
balances.insert(&"alice".to_string(), &1_000_000u128);

// Read
let balance = balances.get(&"alice".to_string()).unwrap_or(0);

// Check existence without reading the value
if balances.contains_key(&"alice".to_string()) { ... }

// Remove
balances.remove(&"alice".to_string());
```

**Prefix uniqueness:** Every collection must have a unique prefix within a contract. If two collections share a prefix, their keys will collide. Convention: use short byte literals — `b"a"`, `b"b"`, etc.

### `UnorderedMap<K, V>`

Extends `LookupMap` with a key index that enables enumeration. Internally maintains a `Vector<K>` of inserted keys alongside the value map.

```rust
let mut registry: UnorderedMap<AccountId, TokenInfo> = UnorderedMap::new(b"reg");

registry.insert(&"alice".to_string(), &token_info);

// Enumerate all keys
let all_keys: Vec<AccountId> = registry.keys_as_vec();

let len = registry.len();
```

**Warning:** Iterating over a very large `UnorderedMap` in a single call can exhaust gas. For unbounded registries, use pagination: store a `start_index` parameter and process a fixed batch size per call.

### `Vector<V>`

An append-only indexed sequence. Supports random access by index.

```rust
let mut log: Vector<String> = Vector::new(b"log");

log.push(&"first event".to_string());
log.push(&"second event".to_string());

let entry = log.get(0); // Some("first event")
let len = log.len();    // 2
let empty = log.is_empty(); // false
```

### `LazyOption<V>`

A single optional value loaded lazily from storage. Ideal for contract-wide metadata or config that is not always needed.

```rust
let mut config: LazyOption<ContractConfig> = LazyOption::new(b"cfg", None);

// Write
config.set(&ContractConfig { paused: false, fee_bps: 30 });

// Read (one storage read per call)
let cfg = config.get().unwrap_or_default();

// Remove and return
let old_cfg = config.take();
```

### Storage Key Design

All collections are namespaced by their prefix. To avoid collisions in complex contracts, define an enum for your prefixes:

```rust
use borsh::BorshSerialize;

#[derive(BorshSerialize)]
pub enum StorageKey {
    Balances,
    Allowances,
    TokenIndex,
    Metadata,
}

// Usage
let balances: LookupMap<AccountId, u128> = LookupMap::new(StorageKey::Balances);
```

---

## 5. Environment API

The `env` module exposes the BTCPC runtime's host functions. On-chain these are `extern "C"` imports; in tests, the mock implementation is used automatically.

### Full Reference

| Function | Signature | Description | Return Type |
|----------|-----------|-------------|-------------|
| `env::signer()` | `() -> AccountId` | The account that signed the originating transaction | `AccountId` (`String`) |
| `env::predecessor()` | `() -> AccountId` | The immediate caller (same as signer unless called from another contract) | `AccountId` |
| `env::current_contract()` | `() -> AccountId` | This contract's own address (`BTCPCsc…`) | `AccountId` |
| `env::epoch()` | `() -> Epoch` | Current chain epoch number. One epoch = 30 seconds | `u64` |
| `env::balance_of(account)` | `(&AccountId) -> Balance` | On-chain BTCPC balance of any account, in dreams | `u128` |
| `env::transfer(to, amount)` | `(&AccountId, Balance) -> bool` | Transfer dreams from this contract's balance to `to`. Panics if insufficient funds | `bool` |
| `env::log(msg)` | `(&str)` | Emit a log string. Visible in transaction receipts; not stored in state | `()` |
| `env::panic_str(msg)` | `(&str) -> !` | Abort execution. All state changes are rolled back | `!` (never) |
| `env::storage_write(key, val)` | `(&[u8], &[u8])` | Raw KV write (prefer collection types) | `()` |
| `env::storage_read(key)` | `(&[u8]) -> Option<Vec<u8>>` | Raw KV read | `Option<Vec<u8>>` |
| `env::storage_remove(key)` | `(&[u8]) -> bool` | Remove a key. Returns true if the key existed | `bool` |
| `env::storage_has_key(key)` | `(&[u8]) -> bool` | Check key existence without reading the value | `bool` |

### Identity Functions

`env::signer()` vs `env::predecessor()`: these differ only in cross-contract call scenarios.

- **`signer`**: The externally-owned account that signed the original transaction. Constant throughout the entire call chain.
- **`predecessor`**: The contract or account that directly invoked this method. Changes with each hop in a cross-contract call.

For most access-control checks, use `env::signer()`. For contracts that act as middleware or routers, `env::predecessor()` is more appropriate.

```rust
// Access control: check the original human signer
require!(env::signer() == self.owner, "unauthorized");

// Check direct caller (e.g., only callable via a specific proxy contract)
require!(env::predecessor() == PROXY_CONTRACT_ID, "use the proxy");
```

### Balance and Transfer

`Balance` is denominated in **dreams** (1 BTCPC = 10,000,000,000 dreams). The constant `DREAMS_PER_BTCPC` is provided for convenience.

```rust
use btcpc_contract_sdk::types::DREAMS_PER_BTCPC;

let my_balance = env::balance_of(&env::current_contract());
let caller_balance = env::balance_of(&env::signer());

// Send 0.5 BTCPC to the caller
env::transfer(&env::signer(), DREAMS_PER_BTCPC / 2);
```

### Macros

The SDK provides three convenience macros:

```rust
// require! — panics with a message if condition is false (rolls back state)
require!(amount > 0, "amount must be positive");
require!(self.active, format!("contract is paused since epoch {}", self.paused_at));

// log! — format-string log (visible in receipts)
log!("Transfer: {} -> {} | amount: {}", from, to, amount);

// emit! — emit a structured event (indexed, queryable)
emit!(&MyEvent { from, to, amount });
```

---

## 6. Events

Events are structured, JSON-serialized records emitted during contract execution. They are included in block receipts, indexed by the BTCPC explorer, and queryable via the API. Events do **not** affect contract state and do not cost storage.

### Emitting Events

```rust
use btcpc_contract_sdk::event;
use serde::Serialize;

#[derive(Serialize)]
struct PriceUpdate {
    standard: &'static str,
    version: &'static str,
    event:    &'static str,
    asset:    String,
    price:    u64,
    epoch:    u64,
}

// In a #[call] method:
event::emit_event(&PriceUpdate {
    standard: "custom",
    version:  "1.0.0",
    event:    "price_update",
    asset:    "BTC".to_string(),
    price:    6_500_000_000,
    epoch:    env::epoch(),
});
```

Or use the `emit!` macro:

```rust
emit!(&PriceUpdate { ... });
```

### Event Format

Events are emitted as raw JSON bytes. The convention for standard events is:

```json
{
  "standard": "bsp-20",
  "version":  "1.0.0",
  "event":    "ft_transfer",
  "from":     "alice",
  "to":       "bob",
  "amount":   1000000
}
```

### Built-in Standard Events

The SDK provides pre-built event structs in `btcpc_contract_sdk::event`:

#### `TransferEvent` (BSP-20)

```rust
use btcpc_contract_sdk::event::TransferEvent;

emit!(&TransferEvent::new(
    sender.clone(),
    receiver.clone(),
    amount,
));
```

Serializes to:
```json
{
  "standard": "bsp-20",
  "version":  "1.0.0",
  "event":    "ft_transfer",
  "from":     "alice",
  "to":       "bob",
  "amount":   500000000
}
```

#### `NftMintEvent` (BSP-721)

```rust
use btcpc_contract_sdk::event::NftMintEvent;

emit!(&NftMintEvent::new(owner.clone(), token_id.clone()));
```

Serializes to:
```json
{
  "standard": "bsp-721",
  "version":  "1.0.0",
  "event":    "nft_mint",
  "owner":    "alice",
  "token_id": "token-001"
}
```

### Querying Events

```bash
btcpc events --contract BTCPCsc<address> --event ft_transfer --from-epoch 3200
```

---

## 7. Cross-Contract Calls

Cross-contract calls allow a contract to schedule a method call on another contract within the same transaction. The callee executes asynchronously after the caller's method returns.

### `Promise::new(...).call(...)`

```rust
use btcpc_contract_sdk::promise::{self, Promise};

// Inside a #[call] method:
Promise::new("BTCPCsc<other-contract>")
    .call(
        "ft_on_transfer",
        serde_json::json!({
            "sender_id": env::signer(),
            "amount":    amount.to_string(),
            "msg":       msg,
        }),
        0,                         // attached deposit (dreams)
        promise::gas::CALLBACK,    // gas to attach (10 TGAS)
    );
```

### Gas Constants

The `promise::gas` module provides suggested gas values:

| Constant | Value | Use Case |
|----------|-------|----------|
| `gas::BASIC` | 5 TGAS (5 × 10^9) | Simple read calls |
| `gas::TRANSFER` | 10 TGAS | Token transfer callbacks |
| `gas::COMPLEX` | 50 TGAS | Multi-step logic |
| `gas::CALLBACK` | 10 TGAS | General callbacks |

### Phase 2 Note

The full cross-contract callback result API (`promise_then`, `promise_and`, `promise_result`) is a **Phase 2 feature**. The `promise_create` host function is live; `promise_then` and the `#[callback]` result-handling machinery are stubs that return 0. Do not build production logic that depends on the callback result until Phase 2 is released.

### Callback Pattern (Phase 2 Preview)

```rust
// This pattern will work fully in Phase 2:
#[private]
#[call]
pub fn on_ft_transfer_complete(&mut self, succeeded: bool) {
    if !succeeded {
        // revert internal accounting
        self.pending_transfers.remove(&env::signer());
    }
}
```

---

## 8. Gas and EB

### Units

| Term | Definition |
|------|-----------|
| **EB (Epoch Bandwidth)** | BTCPC's human-readable gas unit. 1 EB = 1,000,000 fuel units |
| **Fuel unit** | One unit of wasmtime metered WASM execution |
| **TGAS** | Tera-gas (10^12 fuel units) — used in the `promise::gas` constants |

The `Gas` type alias is `u64` and represents raw fuel units throughout the SDK. The relationship is:

```
1 EB = 1,000,000 fuel = 0.000001 TGAS
```

### Cost Table

| Operation | Fuel Cost | EB Cost |
|-----------|-----------|---------|
| Base call cost | 500,000 | 0.5 EB |
| Base view cost | 5,000,000 | 5 EB |
| Storage read (hit) | 50,000 | 0.05 EB |
| Storage read (miss) | 5,000 | 0.005 EB |
| Storage write | 100,000 | 0.1 EB |
| Storage remove | 40,000 | 0.04 EB |
| Emit event | 10,000 | 0.01 EB |
| Contract deploy (base) | 500,000,000 | 500 EB |
| Contract deploy (per 1 KB of WASM) | 1,000,000 | 1 EB |
| Maximum gas per call | `MAX_GAS` = 300,000,000,000 | 300,000 EB |

### Checking Gas Remaining

```rust
// Not yet exposed in the high-level API; available via the raw env::sys functions
// on wasm32. In practice, keep calls well under the 300,000 EB limit.
```

### Minimizing Gas

1. Prefer `LookupMap` over `UnorderedMap` when you do not need iteration.
2. Batch storage writes — accumulate changes in memory, write once.
3. Avoid unbounded loops. Process data in paginated batches across multiple calls.
4. Use `#[view]` for read-only methods. Views have a lower base cost and can be called off-chain for free.
5. Optimize WASM binary size: use `opt-level = "z"`, `lto = true`, and `panic = "abort"` in `Cargo.toml`.

---

## 9. BSP Standards

**BTCPC Standard Proposals (BSPs)** define shared interfaces for common contract patterns — analogous to Ethereum's ERCs or Solana's SPL.

Implementing a BSP standard makes your contract interoperable with wallets, explorers, DEXes, and other infrastructure that recognizes these interfaces.

### BSP-20: Fungible Token

Equivalent to ERC-20. Defines a standard interface for divisible, interchangeable tokens.

**Required methods:**
- `ft_transfer(receiver_id, amount, memo)` — transfer tokens
- `ft_transfer_call(receiver_id, amount, memo, msg)` — transfer + notify receiver contract
- `ft_total_supply() -> String` — total token supply (stringified u128)
- `ft_balance_of(account_id) -> String` — balance of an account
- `ft_metadata() -> FtMetadata` — name, symbol, decimals, icon, reference

**Allowance extension (optional but recommended):**
- `approve(spender, amount)` — grant a spending allowance
- `transfer_from(owner, receiver, amount)` — spend on behalf of owner
- `allowance(owner, spender) -> String` — query remaining allowance

**Required events:**
- `Transfer { standard: "bsp-20", event: "ft_transfer", from, to, amount }`
- `Approval { standard: "bsp-20", event: "ft_approve", owner, spender, amount }`

Full specification: [standards/BSP-20.md](standards/BSP-20.md)
Reference implementation: [examples/ft](examples/ft/src/lib.rs)

### BSP-721: Non-Fungible Token

Equivalent to ERC-721. Defines a standard interface for unique, indivisible tokens.

**Required methods:**
- `nft_mint(token_id, receiver, metadata, transferable, soulbound)` — create a new token
- `nft_transfer(token_id, receiver, memo)` — transfer ownership
- `nft_approve(token_id, approved)` — approve a single account for one token
- `nft_approve_all(operator, approve)` — approve/revoke an operator for all tokens
- `nft_token(token_id) -> Option<Token>` — token details
- `nft_tokens_for_owner(account) -> Vec<String>` — tokens owned by an account
- `nft_total_supply() -> u64` — total minted tokens
- `nft_metadata() -> Value` — collection-level metadata
- `nft_is_approved(token_id, account) -> bool` — check approval status

**Required events:**
- `NftMint { standard: "bsp-721", event: "nft_mint", owner, token_id }`
- `NftTransfer { standard: "bsp-721", event: "nft_transfer", token_id, old_owner, new_owner }`

Full specification: [standards/BSP-721.md](standards/BSP-721.md)
Reference implementation: [examples/nft](examples/nft/src/lib.rs)

### BSP-10: Multi-Token (Coming)

Equivalent to ERC-1155. A single contract managing multiple fungible and non-fungible token types. Allows batched transfers across token IDs, reducing gas costs for complex operations.

### BSP-1: Governance (Coming)

Equivalent to ERC-20 governance extensions (votes, delegation, on-chain proposals). Extends BSP-20 with checkpointed balances and a proposal/voting lifecycle.

---

## 10. Security

### Access Control

Always verify the caller before sensitive state changes. Use `env::signer()` for the human account that originated the transaction.

```rust
// Pattern 1: Owner-only
require!(env::signer() == self.owner, "unauthorized: owner only");

// Pattern 2: assert_eq! (panics with a default message on failure)
assert_eq!(env::signer(), self.owner, "unauthorized");

// Pattern 3: Allowlist
require!(self.admins.contains_key(&env::signer()), "not an admin");
```

### Integer Overflow

Do not use raw `+` or `-` for token balances. Use `saturating_add` / `saturating_sub` or check explicitly.

```rust
// Unsafe — can panic in debug, wrap in release
let new_balance = balance + amount;

// Safe — saturates at u128::MAX rather than overflowing
let new_balance = balance.saturating_add(amount);

// Safe — explicit underflow check
require!(balance >= amount, "insufficient balance");
let new_balance = balance - amount;
```

The reference BSP-20 implementation uses explicit underflow checks via `require!` before all subtractions.

### Reentrancy

Reentrancy attacks are **not possible** in BTCPC. WASM execution is synchronous — a contract method runs to completion before any cross-contract call is dispatched. There is no way for a callee to re-enter the caller mid-execution.

Cross-contract calls via `Promise` are scheduled and execute in a subsequent step, after all state changes from the current call have been committed. Design callback handlers (`#[private]`) defensively, but traditional reentrancy guards are unnecessary.

### Validating Deposits

If your contract accepts native BTCPC transfers alongside calls, always validate the deposit amount explicitly:

```rust
// Example: payable deposit gate
let deposit = env::balance_of(&env::current_contract());
// Note: prefer an explicit deposit parameter; native deposit tracking is application-specific.
require!(deposit >= required_fee, "insufficient deposit");
```

### Avoiding Unbounded Loops

Never loop over an unbounded collection in a single call. An attacker can force a legitimate call to exhaust gas by bloating the collection.

```rust
// Dangerous: loops over potentially millions of entries
for key in self.registry.keys_as_vec() { ... }

// Safe: paginated with caller-supplied limit
#[call]
pub fn process_batch(&mut self, start: u64, limit: u64) {
    let end = start.saturating_add(limit).min(self.queue.len());
    for i in start..end {
        if let Some(item) = self.queue.get(i) { self.process(item); }
    }
}
```

### Pausing

Implement a pause mechanism for critical contracts:

```rust
#[call]
pub fn pause(&mut self) {
    require!(env::signer() == self.owner, "unauthorized");
    self.paused = true;
}

// Guard at the top of sensitive methods:
require!(!self.paused, "contract is paused");
```

---

## 11. Full Example: Fungible Token

The following is the complete BSP-20 reference implementation with all methods documented inline. This contract is production-grade and passes the BSP-20 compliance test suite.

```rust
use btcpc_contract_sdk::*;
use borsh::{BorshDeserialize, BorshSerialize};
use serde::{Deserialize, Serialize};

// ── Types ────────────────────────────────────────────────────────────────────

/// BSP-20 token metadata. Returned by ft_metadata().
#[derive(BorshSerialize, BorshDeserialize, Serialize, Deserialize, Clone)]
pub struct FtMetadata {
    pub name:      String,        // Human-readable token name, e.g. "MyToken"
    pub symbol:    String,        // Ticker symbol, e.g. "MTK"
    pub decimals:  u8,            // Decimal places. 10 matches BTCPC dreams precision (1 BTCPC = 10^10 dreams).
    pub icon:      Option<String>,     // Data URL for a token icon (optional)
    pub reference: Option<String>,     // URL to a JSON metadata reference (optional)
}

// ── State ────────────────────────────────────────────────────────────────────

#[btcpc_contract]
pub struct FungibleToken {
    pub metadata:     FtMetadata,
    pub total_supply: u128,
    /// token balance per account. Amounts are in the token's smallest unit.
    pub accounts:     LookupMap<AccountId, u128>,
    /// (owner, spender) -> approved amount
    pub allowances:   LookupMap<(AccountId, AccountId), u128>,
}

// ── Methods ──────────────────────────────────────────────────────────────────

#[btcpc_impl]
impl FungibleToken {
    /// Deploy the token. Called once via CONTRACT_DEPLOY.
    ///
    /// `total_supply` is in the token's smallest unit. For a token with 8 decimals,
    /// 1 full token = 100_000_000.
    ///
    /// The entire supply is minted to the deployer's account.
    #[init]
    pub fn new(
        name:         String,
        symbol:       String,
        decimals:     u8,
        total_supply: String,
    ) -> Self {
        let supply: u128 = total_supply.parse().unwrap_or(0);
        let owner = env::signer();

        let mut accounts = LookupMap::new(b"a");
        accounts.insert(&owner, &supply);

        let metadata = FtMetadata {
            name, symbol, decimals, icon: None, reference: None
        };

        // Emit a mint event (from="" signals a mint)
        emit!(&event::TransferEvent::new("".to_string(), owner, supply));

        Self {
            metadata,
            total_supply: supply,
            accounts,
            allowances: LookupMap::new(b"l"),
        }
    }

    /// Transfer `amount` tokens from the signer to `receiver_id`.
    ///
    /// `amount` is a stringified u128 to avoid JSON precision loss.
    /// `memo` is optional and included in the log; it is not stored.
    #[call]
    pub fn ft_transfer(
        &mut self,
        receiver_id: AccountId,
        amount:      String,
        memo:        Option<String>,
    ) {
        let sender = env::signer();
        let amount: u128 = amount.parse().unwrap_or(0);

        require!(amount > 0, "amount must be greater than zero");
        require!(sender != receiver_id, "sender and receiver must differ");

        let sender_balance = self.accounts.get(&sender).unwrap_or(0);
        require!(sender_balance >= amount, "insufficient token balance");

        // Update balances
        self.accounts.insert(&sender, &(sender_balance - amount));
        let receiver_balance = self.accounts.get(&receiver_id).unwrap_or(0);
        self.accounts.insert(&receiver_id, &(receiver_balance.saturating_add(amount)));

        emit!(&event::TransferEvent::new(sender, receiver_id, amount));
        log!("ft_transfer: {} tokens | memo: {:?}", amount, memo);
    }

    /// Transfer tokens to a contract and notify it via ft_on_transfer.
    ///
    /// The receiver contract must implement:
    ///   fn ft_on_transfer(sender_id: AccountId, amount: String, msg: String) -> String
    ///
    /// Phase 2: the return value from ft_on_transfer can trigger a refund via callback.
    #[call]
    pub fn ft_transfer_call(
        &mut self,
        receiver_id: AccountId,
        amount:      String,
        memo:        Option<String>,
        msg:         String,
    ) -> String {
        // Execute the transfer first
        self.ft_transfer(receiver_id.clone(), amount.clone(), memo);

        // Schedule the cross-contract notification
        promise::Promise::new(&receiver_id)
            .call(
                "ft_on_transfer",
                serde_json::json!({
                    "sender_id": env::signer(),
                    "amount":    amount.clone(),
                    "msg":       msg,
                }),
                0,
                promise::gas::CALLBACK,
            );

        amount
    }

    /// Returns the total token supply as a string (avoids JS u128 precision loss).
    #[view]
    pub fn ft_total_supply(&self) -> String {
        self.total_supply.to_string()
    }

    /// Returns the token balance of `account_id` as a string.
    /// Returns "0" if the account has never received tokens.
    #[view]
    pub fn ft_balance_of(&self, account_id: AccountId) -> String {
        self.accounts.get(&account_id).unwrap_or(0).to_string()
    }

    /// Returns the token metadata (name, symbol, decimals, icon, reference).
    #[view]
    pub fn ft_metadata(&self) -> FtMetadata {
        self.metadata.clone()
    }

    // ── Allowance Extension ───────────────────────────────────────────────────

    /// Approve `spender` to transfer up to `amount` tokens on your behalf.
    ///
    /// Setting `amount` to "0" revokes the allowance.
    #[call]
    pub fn approve(&mut self, spender: AccountId, amount: String) {
        let owner = env::signer();
        let amount: u128 = amount.parse().unwrap_or(0);
        self.allowances.insert(&(owner.clone(), spender.clone()), &amount);
        // Emit an approval event
        event::emit_event(&serde_json::json!({
            "standard": "bsp-20",
            "version":  "1.0.0",
            "event":    "ft_approve",
            "owner":    owner,
            "spender":  spender,
            "amount":   amount.to_string(),
        }));
    }

    /// Transfer tokens from `owner` to `receiver` using a pre-approved allowance.
    ///
    /// The caller (signer) must have been approved by `owner` via `approve`.
    #[call]
    pub fn transfer_from(
        &mut self,
        owner:    AccountId,
        receiver: AccountId,
        amount:   String,
    ) {
        let spender = env::signer();
        let amount: u128 = amount.parse().unwrap_or(0);
        require!(amount > 0, "amount must be greater than zero");

        // Check and consume allowance
        let allowance = self.allowances
            .get(&(owner.clone(), spender.clone()))
            .unwrap_or(0);
        require!(allowance >= amount, "transfer amount exceeds allowance");

        let owner_balance = self.accounts.get(&owner).unwrap_or(0);
        require!(owner_balance >= amount, "insufficient balance");

        // Deduct allowance
        self.allowances.insert(
            &(owner.clone(), spender),
            &(allowance - amount),
        );

        // Move tokens
        self.accounts.insert(&owner, &(owner_balance - amount));
        let receiver_balance = self.accounts.get(&receiver).unwrap_or(0);
        self.accounts.insert(&receiver, &(receiver_balance.saturating_add(amount)));

        emit!(&event::TransferEvent::new(owner, receiver, amount));
    }

    /// Returns the remaining allowance that `spender` has from `owner`.
    #[view]
    pub fn allowance(&self, owner: AccountId, spender: AccountId) -> String {
        self.allowances
            .get(&(owner, spender))
            .unwrap_or(0)
            .to_string()
    }
}
```

### Deploying and Calling the Token

```bash
# Deploy with 1 billion tokens at 8 decimals
btcpc contract deploy ft.wasm \
  --args '{"name":"MyToken","symbol":"MTK","decimals":8,"total_supply":"100000000000000000"}' \
  --account natoshisakamoto

# Transfer 10 tokens (10 * 10^8 = 1_000_000_000)
btcpc contract call BTCPCsc<address> ft_transfer \
  --args '{"receiver_id":"alice","amount":"1000000000","memo":null}' \
  --account natoshisakamoto

# Check balance
btcpc contract view BTCPCsc<address> ft_balance_of \
  --args '{"account_id":"alice"}'
```

---

*BTCPC Contract SDK — MIT OR Apache-2.0 — [github.com/btcpc-network/btcpc-contract-sdk](https://github.com/btcpc-network/btcpc-contract-sdk)*
