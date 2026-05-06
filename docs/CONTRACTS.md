# BTCPC WASM Contract API

BTCPC supports user-deployed WebAssembly contracts via the `btcpc-contract-runtime` crate
(Wasmtime). Contracts coexist with native protocol entry types (D12).

---

## Entry Types

### ContractDeploy

Deploy a compiled WASM module to the chain.

```json
{
  "type": "ContractDeploy",
  "deployer": "alice",
  "wasm_hash": "sha256hex",
  "wasm_bytes": "base64_encoded_wasm",
  "init_args": "base64_encoded_args",
  "epoch": 42,
  "signed_by": "alice"
}
```

| Field | Description |
|-------|-------------|
| `deployer` | Account that deploys and owns the contract |
| `wasm_hash` | SHA-256 of the raw WASM bytes (hex). Node verifies before storing. |
| `wasm_bytes` | Base64-encoded compiled WASM module |
| `init_args` | Base64-encoded arguments passed to the `init()` export on first call |

**On success:** Contract address = `sha256(deployer + ":" + epoch + ":" + wasm_hash)` (first 20 bytes, hex).
This address is deterministic and reproducible from public data.

**Fee:** `ENTRY_WEIGHT_BULK` (10) × base fee. Contracts are expensive because they occupy
permanent on-chain storage.

---

### ContractCall

Call a deployed contract.

```json
{
  "type": "ContractCall",
  "caller": "alice",
  "contract": "0xdeadbeef...",
  "method": "transfer",
  "args": "base64_encoded_args",
  "value": 0,
  "epoch": 43,
  "nonce": 1,
  "signed_by": "alice"
}
```

| Field | Description |
|-------|-------------|
| `contract` | Contract address (20-byte hex from deploy) |
| `method` | Export function name to call (UTF-8, ≤ 64 chars) |
| `args` | Base64-encoded call arguments (contract-defined encoding) |
| `value` | Native BTCPC (in dreams) attached to the call |

**Fee:** `ENTRY_WEIGHT_HEAVY` (5) × base fee.

---

## Gas Model

WASM execution is metered using Wasmtime's fuel mechanism.

| Limit | Value | Notes |
|-------|-------|-------|
| Gas per instruction | 1 fuel unit | Calibrated against secp256k1 sign = ~50k units |
| Gas limit per call | `500_000` | Configurable via `chain_param:contract_gas_limit` |
| Gas price | 0 (currently) | Fee is charged per-entry, not per-gas |
| Execution timeout | 100ms wall-clock | Hard limit; call is aborted |

Gas is not currently metered per-unit in dreams — the flat `ENTRY_WEIGHT_HEAVY` fee covers
execution cost. This will be revisited when contracts see heavy usage.

---

## Storage Model

Contracts have access to a scoped key-value store via host functions.

### Host Functions

| Export | Signature | Description |
|--------|-----------|-------------|
| `btcpc_get(key_ptr, key_len) → (ptr, len)` | read | Read contract storage |
| `btcpc_set(key_ptr, key_len, val_ptr, val_len)` | write | Write contract storage |
| `btcpc_delete(key_ptr, key_len)` | write | Delete a storage key |
| `btcpc_balance(account_ptr, account_len) → u64` | read | Read BTCPC balance |
| `btcpc_transfer(from_ptr, from_len, to_ptr, to_len, amount: u64) → i32` | write | Transfer BTCPC |
| `btcpc_emit(data_ptr, data_len)` | write | Emit a contract event |
| `btcpc_caller(out_ptr)` | read | Write calling account name (≤ 64 bytes) to out_ptr |

Storage keys are namespaced by contract address. Contracts cannot read each other's storage
(no cross-contract storage calls). Cross-contract *calls* via `ContractCall` entries
are allowed.

### Storage Pricing

| Operation | Cost |
|-----------|------|
| Read | Free |
| Write (new key) | 100 dreams per byte |
| Write (existing key) | 10 dreams per byte |
| Delete | 0 (storage reclaimed, no refund in v1) |

Storage cost is deducted from the `caller`'s balance at execution time. If the caller
has insufficient balance, the call reverts.

---

## Upgrade Path

Contracts are **immutable by default** — the `wasm_bytes` stored at deploy cannot be changed.
To upgrade, deploy a new contract and migrate state.

**Upgrade-in-place** (governance-approved): The contract owner can call
`chain_param:contract_upgrade:{contract_address}` to propose a new WASM hash.
The proposal enters the standard 2-epoch timelock (T1-6). After the timelock,
the next `ContractCall` to that address will use the new bytecode.

This is off by default. It must be explicitly opted into at deploy time via an
`upgradeable: true` flag in `ContractDeploy`.

---

## Governance Controls

| Parameter | Key | Default | Effect |
|-----------|-----|---------|--------|
| Gas limit per call | `contract_gas_limit` | `500000` | Higher = more complex contracts allowed |
| Max WASM size | `contract_max_bytes` | `1_000_000` (1MB) | Larger = more code fits |
| Contracts enabled | `contracts_enabled` | `true` | Kill switch |

All parameters use the standard `ChainParameterSet` entry with 2-epoch timelock.

---

## Security Notes

- WASM execution is sandboxed by Wasmtime. Contracts cannot access the filesystem,
  network, or any OS resources.
- Host functions are the only interface between contracts and chain state.
  All host function calls are audited through the `btcpc-contract-runtime` crate.
- `btcpc_transfer` is rate-limited to one cross-account transfer per contract call.
  This prevents re-entrancy patterns.
- The contract address is deterministic and collision-resistant (SHA-256 based).
  There is no constructor-less `CREATE2`-style address prediction.
