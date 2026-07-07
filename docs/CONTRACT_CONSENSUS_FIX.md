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

## Why the Naive Fix Doesn't Work (what the code forced)

The obvious fix — "emit the contract's transfers as `LedgerEntry::Transfer`s into the
pending pool" — is **impossible** in HONE's model, for two verified reasons:

1. **A contract has no active key.** At seal, pending entries apply via
   `tx::validate_and_apply`, and a `Transfer` requires an **active-key signature**
   (`tx.rs:174–176`: `require_key` + `check_signature(..., "active")`). A
   contract-emitted transfer is `signed_by: contract_id` with no signature — it would
   be **rejected at seal.** Contracts are not signing accounts.

2. **System entries can't be gossiped.** The forge-guard: gossip-received
   `is_system_entry` entries are **silently dropped** (`main.rs:924–927`) — they only
   ever originate *locally at seal* on each node, so no one can inject a fake reward
   over the wire. A "`ContractEffect`" system entry that skipped signatures would
   either be droppable (never propagates) or, if accepted from gossip, a **forgeable
   mint/theft primitive** — a worse hole than the one we're closing.

So there is no way to make a contract's balance effect a standalone sealed entry
safely. The effect must be a **deterministic consequence** that *every node derives
for itself*, from a trigger it can verify.

---

## The Decision — Contract Execution Becomes Consensus Replay

**A `ContractCall` becomes a consensus-sealed unit, and every node re-executes the
contract at seal to derive identical balance effects.** This mirrors how `Mine`
already works (gossiped, applied on every node so the reward hash matches).

- `ContractCall` (and `ContractDeploy`) stop being base-layer no-ops
  (`chain.rs ~1112`). They are gossiped, enter the pending pool, are hardline-gated
  and broadcast, and at **seal** each node **executes the WASM** and applies the
  resulting balance effects (deposit + transfers) to its own state.
- Because every node runs the same call against the same pre-seal state in the same
  deterministic order (`drain_pending_sorted`), all nodes compute the **identical**
  balance delta → the state root converges → **no fork.**
- The call itself is authorized the normal way: the **signer** signs the
  `CONTRACT_CALL` message with their key (already verified at submit,
  `api.rs:2125–2138`), so the sealed unit is legitimately attributable — no forgeable
  effect entry, no contract signature needed.

This is the honest, permanent fix. It **necessarily** makes contract **execution**
part of consensus — the coupled decision the earlier draft deferred is now **made:
execution is replicated-deterministic.**

### Consequences (accepted)

- **Determinism is now load-bearing.** Every node must produce byte-identical
  execution results. The §1.2 pins (`cranelift_nan_canonicalization` + Cranelift
  strategy) are no longer optional insurance — they are **required**, and the
  `wasmi`-interpreter swap (verdict §1.1/§1.2) moves from "deferred" to "the correct
  long-term engine" (native deterministic fuel, no per-version codegen drift).
- **Contract storage joins consensus too.** If execution is replayed, storage writes
  (`contract:{id}:{key}`) must be applied identically on every node at seal — storage
  effectively enters consensus alongside balances. (Whether the storage root is folded
  into the state Merkle root is a follow-up; at minimum every node must hold identical
  storage after seal.)
- **Non-determinism is now a consensus fault.** Any host function exposing time,
  randomness, node-local state, or float non-determinism to contract WASM must be
  audited and made deterministic or removed. This is a hard requirement, not a
  nicety.
- **Contracts stay gated/experimental until proven.** Ship this behind a flag; do not
  enable contract token movement on mainnet until the partition test (§1.1) and a
  determinism-replay test both pass.

### The Fix, concretely

1. **Submit path (`post_contract_call`, api.rs):** apply the `peer_count == 0`
   hardline (reject if zero peers), verify the signer signature (already done), then
   `push_pending(ContractCall{...})` + `tx_broadcast` — **do not** call
   `s.contracts.call` synchronously for its balance effects. Return "accepted, pending
   seal."
2. **`ContractCall` at seal (`chain.rs` apply path):** execute the contract
   deterministically and apply deposit + emitted transfers + storage writes to state.
   This runs on **every** node from the sealed pending set, in `drain_pending_sorted`
   order.
3. **`ContractEngine::call` / `execute_call`:** no longer mutates balances via bare
   `store.debit/credit` or `apply_entry` at HTTP time. Execution is invoked from the
   seal path; its output is applied there. `view` (read-only) stays synchronous.
4. **Gossip:** `ContractCall` is a **user** entry (not `is_system_entry`) so it
   propagates and pending-pools like any transfer; the forge-guard doesn't drop it,
   and it carries the signer's signature for attribution.

### What must NOT happen (guardrails)
- Do **not** keep applying contract `Transfer`s or the deposit via bare `apply_entry`
  / `store.debit/credit` at HTTP time.
- Do **not** add a contract-specific bypass of the `peer_count == 0` hardline.
- Do **not** expose non-deterministic host functions to contract WASM once execution
  is replayed.
- Do **not** enable contract token movement on mainnet before the §1.1 partition test
  and a cross-node determinism test pass.

---

## Coupled Decisions — now resolved

This fix answers **both** questions the verdict coupled:

- *Are contract balance effects consensus state?* → **Yes.**
- *Does contract execution become consensus replay on every node?* → **Yes** — forced
  by the two constraints above (no contract key, no gossipable system entry). The only
  safe way to move a contract's value is for every node to re-derive it.

Therefore:

- **§1.2 / wasmi:** determinism is now required, not optional. NaN-canonicalization pin
  is landed; the `wasmi` swap is the correct long-term engine and should be scheduled
  (a ~15% perf cost that now defends an *exposed* divergence, not a hypothetical one).
- **§4 proof-of-inference:** the same "optimistic re-execution vs single-executor +
  fraud proof" question, for inference instead of contracts — still deferred, but now
  has a precedent (contracts) for what "execution = consensus" looks like in HONE.

Recommended sequence: land the NaN pin (done) → implement contract execution as
seal-replay behind a flag → build the §1.1 partition test + a cross-node determinism
test → only then consider enabling contract token movement, and schedule the wasmi
swap for full cross-version determinism.

---

## Verification / Acceptance

- **Determinism test (new):** two independently-built nodes execute the same
  `ContractCall` at seal and reach a **byte-identical state root**. This is the test
  that justifies the whole replay decision — if it can't pass, contracts don't ship.
- **Partition test (task §1.1):** the deterministic partition/rejoin test covers the
  contract path — a zero-peer node's `ContractCall` is rejected at submit, and the
  test **fails if the hardline is bypassed** (as it is today).
- **No bare balance mutation at HTTP time:** grep guard — `store.debit`/`store.credit`/
  `apply_entry` must not move contract balances outside the seal path.
- **Regression:** existing `contracts.rs` behavior (atomic refund semantics,
  storage-after-transfer ordering) preserved under seal-replay.
- **Gated:** contract token movement stays flag-disabled on mainnet until both tests
  pass.
- **Block-0 unchanged:** apply/seal-path logic, not genesis — hash stays `98e3c1b0`.

---

_A contract that moves value moves consensus state. There is no contract key to sign
it and no gossipable entry to carry it — so every node re-executes the call at seal
and derives the same result. Execution is consensus. No exceptions, including this one._
