# Website API Audit — Rust Node Rewiring

**Date**: 2026-05-05  
**Audited against**: Rust node API routes in `rust/hone-node/src/api.rs`  
**Node base**: `http://localhost:4242`

---

## Summary

| Page | Status | Notes |
|------|--------|-------|
| `explorer.html` + `explorer.js` | ✅ Fixed | All fetch calls rewritten to Rust routes |
| `dashboard.html` | ✅ Fixed | `/public/network` and `/public/leaderboard` rewritten |
| `app.html` (wallet/balance reads) | ✅ Fixed | Balance and history reads rewritten |
| `app.html` (node tab) | ✅ Fixed | Network stats and epoch calls rewritten |
| `app.html` (send/transfer) | ❌ Blocked | Requires ed25519 signing — cannot fix without signing flow |
| `app.html` (auth/login) | ❌ Blocked | No JWT login in Rust node — architecture mismatch |
| `node.html` | ✅ Working | Already uses `/health` correctly |

---

## Per-Page Breakdown

### `explorer.html` + `explorer.js`

**Status**: ✅ Fixed completely.

| Old call | New call | Notes |
|----------|----------|-------|
| `GET /status` | `GET /api/explorer/status` | Shape adapted: `chain_height`, `epoch_ms`, `active_nodes_last_100`, `circulating_hone` |
| `GET /blocks?limit=N` | `GET /api/explorer/blocks?limit=N` | Shape adapted: `timestamp_ms` (not `timestamp`), no `reward` field |
| `GET /activity?limit=N` | `GET /api/explorer/activity?limit=N` | Shape compatible: `entries[]` array |
| `GET /block/:n` | `GET /api/block/:n` | Shape adapted: entries in `payload.ledger_entries` |
| `GET /account/:name` | `GET /api/account/:name` | Shape differs significantly — adapted rendering |
| `GET /account/:name/history` | `GET /api/account/:name/history` | Returns `entries` not `history`; uses `_epoch` field |
| `GET /accounts` | `GET /api/accounts` | Returns `balances.HONE` as integer dreams, not float |
| `GET /api/sensors` | No equivalent | Graceful empty state already present |
| `GET /api/sensors/:id` | `GET /api/sensor/:id` | URL fixed (no plural) |

**Key shape differences from old API**:
- `chain_height` instead of `chain_height` (same)
- `epoch_ms` instead of `epoch_time_ms`
- `active_nodes_last_100` instead of `active_nodes`
- `circulating_hone` (float) instead of `circulating_supply`
- `max_supply_hone` instead of `max_supply`
- No `current_reward_per_epoch` field exists in Rust node — removed from stats
- Account history uses `_epoch` field (set by scanner) not `epoch`
- `get_all_accounts` returns `balances.HONE` as raw integer dreams (divide by 10,000,000,000 for HONE)

---

### `dashboard.html`

**Status**: ✅ Fixed.

| Old call | New call | Notes |
|----------|----------|-------|
| `GET /public/network` (×2) | `GET /api/explorer/status` | Field mapping: `epoch→current_epoch`, `nodes→active_nodes_last_100`, `clocks→clock_nodes` |
| `GET /public/leaderboard` | Derived from `GET /api/accounts` | No dedicated leaderboard endpoint; sorted by balance client-side |

---

### `node.html`

**Status**: ✅ Already working.

Uses `GET /health` at `http://localhost:4242/health` and `http://localhost:4243/health` for node detection. `/health` exists in the Rust node. No changes needed.

---

### `app.html`

**Status**: 🔄 Partially fixed.

#### Fixed calls

| Old call | New call | Location |
|----------|----------|----------|
| `GET /api/wallet/balance` (×5) | `GET /api/balance/:account` | Lines 1781, 2530, 5086, 5305, 5355 |
| `GET /api/wallet/transactions` | `GET /api/account/:account/history?limit=20` | Line 5322 |
| `GET /public/network` (×3) | `GET /api/explorer/status` | Lines 1762, 2483, 5173 |
| `GET /node/epoch/current` | `GET /api/latest` | Lines 1789, 2507 |
| `GET /node/epoch/:n` (loop) | `GET /api/explorer/blocks?limit=8` | Line 2557 |

**Shape note on balance**: The old `/api/wallet/balance` returned `{ success, balance: { HONE: float }, address, delegated_balance }`. The Rust `/api/balance/:account` returns `{ account, balance: float, dreams: int, token: "HONE" }`. All rendering code adapted accordingly. No JWT required for balance reads.

**Shape note on history**: The old `/api/wallet/transactions` returned `{ transactions: [{type: "receive"/"send", counterpart, amount}] }`. The Rust `/api/account/:account/history` returns `{ account, count, entries: [{type (LedgerEntry variant), from, to, account, amount, memo, _epoch, _role}] }`. The receive/send detection was rewritten to use `to === acct` logic.

#### Blocked — Needs Signing Flow

| Call | Reason |
|------|--------|
| `POST /api/wallet/transfer` (lines 1865, 5455) | Old API accepted `{ toAddress, amount, memo, password }` with JWT. Rust `POST /api/transfer` requires `{ from, to, amount, token, signed_by, nonce, signature }` where signature is an ed25519 signature over the entry. The password/JWT approach is incompatible. This is blocked until a client-side ed25519 signing flow is implemented. |
| `POST /api/staking/stake` / `POST /api/staking/unstake` (via `/api/staking/:action`) | Same signing requirement — Rust `POST /api/stake` and `POST /api/unstake` need ed25519 signature. |
| `GET /api/staking/info` (line 5519) | No `/api/staking/info` endpoint in Rust node. Per-account stake is at `GET /api/stake/:account`. Field names differ (`staked_amount` → `stake` as float HONE). |

#### Out-of-scope (no equivalent in Rust node — noted but not fixed)

| Old call | Status |
|----------|--------|
| `POST /public/login` | No JWT auth in Rust node. The Rust node uses key-based auth. The entire login flow needs architectural rework. |
| `POST /public/signup` | No account creation via username/password. Rust uses `POST /api/account/create` with a key map. |
| `POST /public/change-password` | No password concept in Rust node. |
| `GET /public/machine-status` | No equivalent. The node dashboard serves node info via `GET /api/node/info` and `GET /api/node/models`. |
| `GET /public/my-devices` | No equivalent. |
| `POST /public/device-heartbeat` | No equivalent. |
| `POST /public/model-request` | No equivalent (task marketplace uses `/api/task/*`). |
| `GET /public/model-demand` | No equivalent. |
| `POST /api/delegation/delegate` | No delegation endpoint in Rust node. |
| `GET /api/models/registry` | No model registry endpoint. |
| `POST /api/sensor-data/quote` | No sensor data marketplace endpoint. |
| `POST /api/sensor-data/query` | No sensor data marketplace endpoint. |
| `/v1/chat/completions` | Ollama passthrough — not a node route. |

---

## What Was Actually Fixed

### Files modified

1. **`website/explorer.js`** — Complete rewrite of all fetch calls:
   - Home stats: `/status` → `/api/explorer/status`
   - Recent blocks: `/blocks?limit=10` → `/api/explorer/blocks?limit=10`
   - Activity feed: `/activity?limit=20` → `/api/explorer/activity?limit=20`
   - Block detail: `/block/:n` → `/api/block/:n` (adapted to read `payload.ledger_entries`)
   - Account detail: `/account/:name` → `/api/account/:name` (adapted to Rust account shape)
   - Account history: `/account/:name/history` → `/api/account/:name/history` (adapted field names)
   - All accounts: `/accounts` → `/api/accounts` (adapted dreams→HONE conversion)
   - Search: `/account/:name` → `/api/account/:name`
   - Sensor detail: `/api/sensors/:id` → `/api/sensor/:id` (removed plural)

2. **`website/dashboard.html`** — Rewired network stats and leaderboard:
   - `/public/network` → `/api/explorer/status`
   - `/public/leaderboard` → derived from `/api/accounts` (sorted client-side)
   - Recent blocks section now uses `/api/explorer/blocks?limit=10`

3. **`website/app.html`** — Wallet and node section reads:
   - All `/api/wallet/balance` calls → `/api/balance/:account` (JWT removed, account-name based)
   - `/api/wallet/transactions` → `/api/account/:account/history`
   - `/public/network` calls → `/api/explorer/status`
   - `/node/epoch/current` → `/api/latest`
   - `/node/epoch/:n` loop → `/api/explorer/blocks?limit=8`
   - Models fetch rewired to Ollama local API (`localhost:11434/api/tags`)

---

## Remaining Work

1. **Client-side ed25519 signing**: The transfer, stake, and unstake flows are completely blocked. The Rust node requires `{ from, to, amount, token, signed_by, nonce, signature }` where `signature` is a valid ed25519 signature. A signing library (e.g. `@noble/ed25519`) needs to be integrated into app.html, and the key derivation flow from the mnemonic needs to be wired up.

2. **Login/auth rework**: The `/public/login` → JWT flow has no equivalent. The Rust account model uses key-based ownership. The app's "enter username + password" onboarding flow needs to be redesigned around:
   - `POST /api/account/create` (submit pubkeys)
   - Client-side key derivation from mnemonic
   - Local key storage (never sending private keys to node)

3. **`/api/staking/info` → `/api/stake/:account`**: The staking tab reads per-user stake info. The Rust endpoint returns `{ account, stake (float), dreams }` — different from what the UI expects. The staking tab needs adaptation.

4. **`/public/machine-status`**: Used to show local machine info in the home and node panels. No equivalent in Rust node. The closest is `GET /api/node/info` but it has a different shape.

5. **Sensor data market**: `/api/sensor-data/quote` and `/api/sensor-data/query` have no equivalents. The sensor marketplace is not yet implemented in the Rust node.
