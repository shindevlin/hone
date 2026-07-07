# Contract Balance Effects Are Consensus State (§0 Fork Fix)

**Version 0.1 — July 2026**
**Status: launch-blocker. Spec first (consensus-critical), then implement, then partition-test.**

---

## The Bug (verified)

A contract call that moves tokens mutates this node's consensus balance state
**locally, without broadcast and without the no-peers hardline** — a silent fork.

Verified in the current tree:

- `POST /api/contract/call` → `s.contracts.call(...)` (`api.rs:2140`).
- `ContractEngine::call` (`contracts.rs`) mutates balances three ways, all via the
  **store directly** or a **bare `apply_entry`**:
  - deposit debit/credit: `store.debit(signer)` + `store.credit(contract_id)`
    (`contracts.rs:138–144`);
  - each emitted transfer: builds `LedgerEntry::Transfer` and calls
    `self.chain.apply_entry(&entry)` (`contracts.rs:188`);
  - refund-on-failure debits/credits (`contracts.rs:161–164, 190–192`).
- **Contrast:** every user-facing mutation (`/api/transfer`, `/api/stake`, …) goes
  through `apply_and_broadcast` (`api.rs`), which begins with the hardline:

  > `if peer_count == 0 { reject }` — *"zero peers = silent fork … Do not remove,
  > soften, or add an offline bypass to this check. **No exceptions for any entry
  > type.**"* (`api.rs:2021–2024`)

The consensus state root **is** the balance Merkle root. So a contract transfer
advances the originating node's state root while the network never sees the entry —
the exact failure the hardline exists to prevent, entering through a path the
hardline does not guard. A node with zero peers can run a contract that moves tokens
and silently diverge.

This is the class of bug that broke prior launches by another name. It is a
**launch gate**, same tier as the clock-bootstrap deadlock.

---

## The Decision

**Contract-emitted balance effects ARE first-class consensus entries.** They must
flow through the *same* path as every other value mutation: subject to the no-peers
hardline, broadcast to the network, and sealed in an epoch. A contract's token
movements are not private node-local bookkeeping; they change the balance state root,
so the network must see and seal them like any Transfer.

(Contract *storage* — `contract:{id}:{key}` in CF_META — is a separate question; it
is **not** in the balance Merkle root today (single-executor, replay is a no-op —
`chain.rs ~1112`). This fix is scoped to **balance** effects, which *are* in the
state root. Whether storage/execution itself becomes replicated-deterministic is the
coupled §1.2/wasmi + §4 decision, deferred — see "Coupled Decisions.")

---

## The Fix

`ContractEngine::call` must not mutate balances outside the guarded path. Concretely:

1. **No bare balance mutation for consensus effects.** Replace the direct
   `store.debit`/`store.credit` for the **deposit** and the bare `apply_entry` for
   **transfers** with real ledger entries that go through validation + the no-peers
   gate + broadcast + seal.

2. **Model the effects as entries.** A contract call produces a set of balance
   effects: the deposit (signer → contract), and zero or more transfers
   (contract → recipient). Emit these as `LedgerEntry::Transfer`s (or a dedicated
   `ContractEffect` entry batch) so they are:
   - **validated** (`validate_and_apply` / `tx::` rules),
   - **gated** by `peer_count == 0` → reject (the hardline, applied uniformly),
   - **broadcast** via `tx_broadcast` for peers to re-verify,
   - **sealed** in an epoch (drain_pending_sorted → apply in sha256 order), so all
     nodes converge on identical post-state.

3. **Atomicity across the call.** Today `call()` is atomic on one node (refund on any
   failed transfer, storage written only after all transfers succeed). Under the fix,
   the effect set must remain all-or-nothing at seal time: either the whole contract
   call's balance effects seal together or none do. Options to preserve in the spec's
   implementation:
   - emit the effects as one **atomic batch entry** the seal applies transactionally, or
   - keep execution local but only **stage** effects, committing them exclusively
     through the sealed path (no store mutation until seal).

4. **Determinism of the effect set.** The emitted effects must be a pure function of
   (contract state, call args, epoch) so every node that re-applies at seal computes
   the identical balance delta. Execution that yields the effect set stays
   single-executor for now (§ coupled decision); only the **balance delta** it
   produces must be deterministic and sealed.

### What must NOT happen (guardrails)
- Do **not** keep applying contract `Transfer`s via bare `apply_entry`.
- Do **not** add a contract-specific bypass of the `peer_count == 0` hardline.
- Do **not** mutate `store` balances for the deposit before the effect is on the
  sealed path.
- A zero-peer node calling a token-moving contract must be **rejected**, exactly as a
  zero-peer `/api/transfer` is.

---

## Coupled Decisions (from the architect verdict §0/§1.2/§4)

This fix answers one question — *"are contract balance effects consensus state?"* —
**yes.** It deliberately does **not** answer the larger coupled one: *"does contract
execution (the WASM run itself) become consensus replay on every node?"* That governs:

- **§1.2 / wasmi:** codegen determinism only matters if every node re-executes the
  WASM on seal. Until that decision, take the free `cranelift_nan_canonicalization`
  pin (separate task) and defer the wasmi swap.
- **§4 proof-of-inference:** the same "optimistic re-execution vs single-executor +
  fraud proof" question, for inference instead of contracts.

The recommended sequence (verdict §3 method note): land THIS balance-effects fix +
the NaN one-liner, build the partition test that proves the hardline now covers the
contract path, then take the execution-replay decision as its own spec.

---

## Verification / Acceptance

- **Partition test (task §1.1):** the deterministic partition/rejoin test must cover
  the contract path — a zero-peer node calling a token-moving contract is rejected,
  and the test **fails if the hardline is bypassed** (as it is today).
- **Regression:** existing `contracts.rs` tests (atomic refund, storage-after-transfer)
  still pass under the sealed-effect model.
- **No new bare balance mutation:** grep guard — `store.debit`/`store.credit`/
  `apply_entry` must not appear for consensus balance effects in `contracts.rs`
  outside the sealed path.
- **Block-0 unchanged:** this is apply/seal-path logic, not genesis — hash stays
  `98e3c1b0`.

---

_A contract that moves value moves consensus state. It goes through the same gate,
broadcast, and seal as every other Transfer — no exceptions, including this one._
