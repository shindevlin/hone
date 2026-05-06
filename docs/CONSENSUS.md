# BTCPC Consensus — Fork Choice, Finality, Quorum, Slashing

> Phase 2 reference — implemented in v0.5.0

---

## Overview

BTCPC uses a clock-based BFT consensus where registered clock nodes collectively
seal epochs. The epoch IS the block. Each sealed epoch advances chain state;
a finalized epoch is one where reward consensus has been reached.

---

## Clock Node Registration

Clock nodes must register on-chain before their seals count toward quorum.

**Entry:** `ClockNodeRegister { node_id, stake, epoch, signature }`

- Stake is locked from the node's balance. Must meet the dynamic minimum
  `chain_param:clock_min_stake` (default 100 BTCPC = 10,000,000,000 dreams).
- Registration stored in sled at `clock_reg:{node_id}`.
- Slashed nodes (stake == 0) are excluded from the registered set.
- The registered set is refreshed in `ClockConsensus` at each epoch seal via
  `clock::registered_clock_nodes(store)`.

---

## Quorum

When the registered set is non-empty, quorum requires **>51% of registered nodes**
to produce inlier seals with matching seal hashes.

When the registered set is empty (bootstrap), the `BTCPC_BOOTSTRAP_ISOLATION=true`
rule applies: shindevlin's seal is accepted as the sole quorum.

| Condition | Quorum denominator |
|---|---|
| `registered_clocks.is_empty()` OR `BTCPC_BOOTSTRAP_ISOLATION=true` | bootstrap master |
| Registered set non-empty | `registered_clocks.len()` |

Quorum threshold: `ceil(denominator × 0.51)`, minimum 1.

**Future:** BFT-style 2/3 majority and governance flag to switch; planned for Phase 3.

---

## Bootstrap Master

During bootstrap or when no registered nodes exist yet, `shindevlin`'s seal is always
accepted as the single quorum. This allows the network to launch without a chicken-and-egg
problem where you can't register clock nodes because no blocks are being produced.

To enable bootstrap isolation explicitly: `BTCPC_BOOTSTRAP_ISOLATION=true`.

---

## Epoch Entropy

After each epoch seal, entropy is computed as:

```
entropy = SHA-256( XOR of all winning seal hashes )
```

Stored in sled at `epoch_entropy:{epoch}`. Used for pseudorandom selection
protocols in later phases (validator sampling, storage proof targets, etc.).

Implementation: `clock::compute_epoch_entropy(seal_hashes)` in `clock.rs`.

---

## Fork Choice

Fork choice rule (current):
- Accept the highest epoch for which a quorum-threshold signature set from the
  registered clock node set was collected.
- Outlier filtering: seals deviating more than `OUTLIER_EPOCH_TOLERANCE = 2`
  epochs (60 seconds) from the median timestamp are excluded from quorum.
- Tie-breaking: most common `seal_hash` among inliers wins.

**Future Phase 3:** Explicit fork-choice gossip message; longest quorum-certified
chain wins on sync.

---

## State Root

`EpochFinalize.state_root` = `Store::full_state_hash()` — a deterministic SHA-256
over all balances and stakes (sorted by key). Two honest nodes that applied the
same entries in the same epoch must produce identical state roots.

Set in `main.rs` finalized-epoch handler:
```rust
let state_root = chain_ref.store.full_state_hash();
```

---

## Double-Sign Slashing

A clock node that signs two conflicting seals in the same epoch can be slashed by
any submitter who provides evidence.

**Entry:** `ClockDoubleSignEvidence { submitter, offender, epoch, seal_hash_a, seal_hash_b, signature }`

Requirements:
- `seal_hash_a != seal_hash_b` (identical hashes prove nothing)
- Evidence per (offender, epoch) can only be applied once (replay guard: `dbl_slash:{offender}:{epoch}`)

**D7 slash distribution:**
| Recipient | Share |
|---|---|
| `submitter` (bounty) | 10% of offender's registered stake |
| `__legal__` | 10% |
| `__recycle_fund__` | remainder (~80%) |

Offender's `clock_reg:{node_id}` stake is zeroed and they are excluded from future quorum.

---

## Finality

An epoch is *finalized* when `>51%` of clock nodes have proposed the same
`rewards_hash` in a `RewardProposal` gossip message. This triggers an `EpochFinalize`
entry to be applied on-chain with:
- `rewards_hash` — hash of all reward inputs for that epoch
- `state_root` — SHA-256 over sorted balances + stakes
- `quorum` — number of agreeing proposals

**Key invariant:** Two honest nodes that have applied the same entries for epoch N
must produce the same `rewards_hash` AND `state_root`. Divergence indicates a
non-determinism bug.

---

## Upgrade Path

- **Phase 2** (current): registered set quorum, bootstrap master, entropy, state root, double-sign slash
- **Phase 3**: explicit fork-choice sync; 2/3 BFT threshold via governance flag
- **Phase 5**: VDF-based slot assignment; slash for missed slots

See `docs/ROADMAP.md` for the full phase plan.
