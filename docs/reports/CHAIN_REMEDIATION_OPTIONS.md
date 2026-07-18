# HONE Chain Remediation Options

Date: 2026-04-28

This note explains how to recover from the negative spendable balance issue found in the replayed chain state for `natoshisakamoto`.

The key fact is this:

- the bug is in persisted chain state / finality hydration
- it is not, by itself, evidence that the protocol needs a new genesis

## Recommendation

Start with the least disruptive fix:

1. Enforce a non-negative spendable balance invariant during replay and finality hydration.
2. Reject or regenerate malformed finality snapshots.
3. Rebuild state from the last known safe checkpoint.

Only escalate to a hard reset or consensus fork if the bad balance is proven to come from historical canonical blocks and cannot be repaired by snapshot or replay validation.

If the project chooses a new genesis relaunch, then the old chain becomes historical record only and the new chain starts with zero balances for all accounts.

## Option 1: No-Genesis Fix

### When to use it

Use this when the negative balance exists only in:

- finality snapshots
- replay caches
- hydrated in-memory state

and the underlying canonical block history is still valid.

### What it means

- Add replay-time validation that fails on negative spendable balances for non-system accounts.
- Invalidate the malformed snapshot.
- Replay from the last safe checkpoint or from genesis if no safe checkpoint exists.
- Regenerate a clean finality snapshot from valid state.

### Why this is preferred

- Lowest blast radius
- Preserves chain history
- Avoids forcing wallets, miners, and tools onto a new chain identity
- Keeps continuity for docs, explorers, and integrations

### Risk

- If the bad state is also encoded in the canonical block stream, this fix only detects the problem and does not repair the source.

## Option 2: Hard Reset

### When to use it

Use this when:

- the snapshot is corrupt
- the chain can be replayed from a known-good point
- but the current persisted state is too polluted to trust

### What it means

- Stop trusting the current finality snapshot.
- Delete or quarantine the corrupted snapshot artifacts.
- Rebuild state from an earlier checkpoint or from genesis.
- Publish a fresh finality checkpoint after validation.

### Why this is heavier

- It can discard a lot of cached state.
- It may require nodes to resync.
- Operationally disruptive, but still less severe than a protocol fork.

### Risk

- If the underlying block history is wrong, a hard reset just replays the same bad logic.

## Option 3: Consensus Fork / Protocol Migration

### When to use it

Use this only if the negative balance is not a snapshot bug, but a consequence of canonical protocol rules or a consensus-critical ledger path.

### What it means

- Introduce a protocol-level state fix at a specific activation epoch.
- Nodes must upgrade to the new rules.
- The chain state is migrated under consensus.

### Why this is the most expensive option

- Requires coordinated node upgrades
- May split the network if adoption is uneven
- Needs explicit activation rules and migration tests

### Risk

- This is the right answer only when the bug is baked into consensus.
- It should not be used just to hide a bad snapshot.

## Decision Rule

Use this order:

1. If the negative balance only exists after finality hydration or replay, choose the no-genesis fix.
2. If the snapshot itself is corrupt, choose a hard reset.
3. If the canonical block rules produce the bad state, choose a consensus fork.

## Current Assessment

Based on the observed state:

- `natoshisakamoto` replays to a negative spendable HONE balance
- the latest finality snapshot already contains that negative balance
- the account is not a system account

That points first to a snapshot/replay integrity failure, not an automatic need for a brand-new genesis.

## Operational Note

Whatever path is chosen, the following guardrails should be part of the fix:

- fail closed on negative spendable balances
- log the first offending epoch and account
- add tests that replay the chain and assert non-negative balances for all non-system accounts
- add a migration note for explorers and wallet clients so they can distinguish spendable balance from total earnings
