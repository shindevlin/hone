# Rust-Primary Chain Migration Plan

## Goal

Make Rust the authoritative implementation for all chain-critical logic:

- ledger accounting
- replay and finality hydration
- reward calculation
- recycle routing
- balance invariants
- state commitment generation

Node.js may remain for UI, CLI, and peripheral services, but it should not be the source of truth for chain state.

## Non-negotiable invariants

- New genesis starts every wallet at zero.
- No wallet balance may ever go negative.
- Unearned or unallocatable value routes to `hone_recycle`.
- Valid user balances are never trimmed.
- Replay must fail fast on corrupt state instead of normalizing it.
- A node joining at later epochs must be able to trust balances as live chain state, not UI cache.

## Why this is necessary

The current replay path is able to hydrate invalid state from persisted snapshots. That means chain accounting is not yet a hard invariant; it is partly a convention enforced by the current JS runtime. The migration needs to move those rules into Rust so they are compiled, tested, and owned by the chain engine itself.

## Phase 1: Freeze the accounting contract

- Keep the newly added no-negative-balance checks in the existing runtime.
- Add regression tests for corrupt finality snapshots.
- Document the exact meaning of `hone_recycle`, reward pools, and zero-balance genesis.

## Phase 2: Create a Rust chain-core crate

Build a Rust crate that owns:

- balance arithmetic
- account state transitions
- recycle routing
- epoch reward allocation
- replay validation

The crate should expose a small deterministic API and be covered by unit tests before integration.

## Phase 3: Port replay and reward logic

Move these paths out of JS:

- finality snapshot decoding
- ledger entry application
- reward splitting
- balance integrity assertions
- state root generation

At this stage, Rust should be the canonical path for block replay.

## Phase 4: Remove chain-side authority from Node

Retain Node only for:

- API gateway
- CLI
- explorer
- documentation tooling
- optional non-chain UX helpers

The chain process itself should no longer depend on a JS sidecar for core accounting.

## Phase 5: Update external surfaces

Once Rust chain-core is live:

- rewrite the whitepaper sections describing accounting
- update README launch text
- update the public website launch copy
- clearly describe zero genesis and recycle behavior
- clearly state that balances are on-chain and immutable except by chain rules

## Implementation order

1. Harden current runtime invariants.
2. Add regression tests for negative state.
3. Introduce Rust chain-core crate.
4. Port replay and reward logic.
5. Remove JS authority from chain-critical paths.
6. Update docs and web copy.

