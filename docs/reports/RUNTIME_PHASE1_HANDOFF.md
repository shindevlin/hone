# Runtime Phase 1 Handoff (Rust-only)

## Scope Completed
Implemented decentralized runtime **Phase 1** in Rust only:
- New runtime ledger entry variants in `hone-types`.
- Chain apply/persist handling in `hone-node`.
- Tx validation/signature path support for new runtime entries.

### Files in scope
- `rust/hone-node/crates/hone-types/src/entry.rs`
- `rust/hone-node/src/chain.rs`
- `rust/hone-node/src/tx.rs`
- `docs/PLAN_DECENTRALIZED_RUNTIMES_V1.md`
- `.planning/phases/99-decentralized-runtimes/PLAN.md`

## Runtime Entry Types Added
- `RuntimeRegister`
- `RuntimeDeploy`
- `RuntimeUndeploy`
- `RuntimeJobEnqueue`
- `RuntimeClaim`
- `RuntimeAttest`
- `RuntimeChallenge`
- `RuntimeSlash`

These are wired into:
- `LedgerEntry::epoch()`
- `entry_weight(...)`
- tx validation match in `src/tx.rs`
- chain apply path in `src/chain.rs`

## State Keys Written (Chain Store)
All runtime state is persisted via `state_set` (CF_META keyspace) with keys:
- `runtime:{runtime_id}`
- `runtime_host:{host_id}`
- `runtime_job:{job_id}`
- `runtime_lease:{lease_id}`
- `runtime_attest:{attestation_id}`
- `runtime_challenge:{challenge_id}`
- `runtime_slash:{slash_id}`

## Verification Run
- Passed:
  - `cargo test -p hone-types --manifest-path rust/hone-node/Cargo.toml`
  - `cargo check -p hone-node --manifest-path rust/hone-node/Cargo.toml`
- Known repo condition (pre-existing, unrelated):
  - full `cargo test -p hone-node` currently hits unrelated compile/test issues in existing test paths.

## Explicitly Not Completed
- Runtime manifest schema and compatibility rules.
- Host capability registry + freshness/expiry semantics.
- Lease arbitration/conflict resolution policy beyond base persistence.
- Challenge adjudication logic.
- Slash economics/distribution wiring.
- API routes for runtime control surface.

## Next Work Item (Phase 2)
Implement Rust runtime manifest + host capability model and enforce deploy gating:
- manifest registration/update entries
- host capability register/heartbeat entries
- deploy-time capability checks
- API endpoints for runtime + host discovery
- tests for stale capability rejection and incompatible deploy failure
