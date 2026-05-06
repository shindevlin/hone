# Rust Cutover Audit Gates

## What step 2 means

Step 2 means a deep audit of the **consensus boundary**, not a general code quality pass.

Consensus boundary for BTCPC:

- transaction acceptance rules
- deterministic ordering rules
- state transition dispatcher
- replay/finality hydration
- reward allocation and recycle routing
- canonical hashing/state root generation
- contract runtime host API

If any part of this boundary is still JS-authoritative, Rust cutover is incomplete.

## Required audit at every step

Each step must pass three audits before the next step starts:

1. **Invariant audit**
- Validate non-negative balances, stake invariants, and replay rejection of corrupt snapshots.

2. **Determinism audit**
- Same input block stream must produce identical state root and identical output entries.

3. **Authority audit**
- Verify no JS path can override Rust consensus outcomes.

## Step gates

### Gate A — accounting invariants

Scope:

- `src/chain/stateStore.js`
- `src/chain/stateManager.js`
- `src/chain/replay.js`
- `tests/stateBackend.test.js`

Pass criteria:

- negative snapshot rejected
- replay fails fast on invalid balances
- no overdraw path for wallets

### Gate B — Rust core parity

Scope:

- `rust/chain-core/src/lib.rs`
- Rust test suite for transfer/stake/delegate/reward split invariants

Pass criteria:

- Rust logic matches JS expected outputs for canonical test vectors
- overflow-safe arithmetic on all balance transitions

### Gate C — replay authority cutover

Scope:

- JS replay path uses Rust validation outputs
- Rust result is the canonical decision source

Pass criteria:

- disabling JS helper logic does not change replay outcome
- state root and account totals match across 3 nodes

### Gate D — reward authority cutover

Scope:

- reward split and recycle routing come from Rust engine

Pass criteria:

- reward vectors deterministic for the same epoch inputs
- unearned allocation routes to `btcpc_recycle`

### Gate E — contract runtime readiness

Scope:

- Rust WASM runtime (`btcpc-vm`) with restricted host API

Pass criteria:

- no time/network/fs/random host calls
- fuel-metered execution and deterministic trap behavior
- contract call replay determinism across nodes

## Second-engine audit requirement

For each gate:

- run one independent review by a second engine/agent
- record findings with severity and file references
- block progression until critical/high findings are closed

If subagent infrastructure is unavailable, fallback to tool-driven review:

- `mcp__code_review_graph__.detect_changes_tool`
- `mcp__code_review_graph__.get_review_context_tool`
- targeted regression test suite

## Audit record template

- Gate:
- Commit/patch scope:
- Invariant audit result: pass/fail
- Determinism audit result: pass/fail
- Authority audit result: pass/fail
- Second-engine findings:
- Decision: proceed / block

