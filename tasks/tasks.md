# HONE Execution Tracker

Snapshot date: 2026-04-08

This tracker replaces the stale URSNode-era task list. It reflects the codebase as it exists in this repository on April 8, 2026.

## Status legend

- [ ] Planned
- [>] In progress
- [x] Done
- [!] Blocked / needs decision

## Current state summary

- [x] Core API server, MongoDB boot, health endpoint, security middleware, and route wiring exist
- [x] HONE account creation, wallet derivation, chain wallet creation, and JWT auth exist
- [x] Wallet, faucet, staking, delegation, node registration, recovery, project billing, inference, and bot routes exist
- [x] P2P network, epoch manager, miner, clock-node support, permanent ledger, and explorer are implemented
- [x] Cross-chain claim and wrapped token groundwork exists
- [>] Automated testing is being added, but baseline coverage is still missing across most modules
- [>] Cleanup is needed where older URS terminology and legacy auth assumptions still remain
- [!] Local worktree has in-flight edits in mining/inference files that should be preserved during cleanup

## Immediate priorities

### P0. Correctness and trust

- [>] Fix auth consistency so newly registered users can log in reliably
  - Success criterion: registration and login use compatible password verification
- [ ] Normalize identifier handling for login
  - Success criterion: users can authenticate with username or email consistently
- [ ] Audit remaining legacy terminology and broken assumptions
  - Success criterion: user-facing HONE flows no longer expose old URS wording or mismatched token names
- [ ] Review dirty local changes before any mining/inference refactor
  - Success criterion: current local work is not overwritten or silently regressed

### P1. Testing baseline

- [>] Add unit tests for authentication flows
  - Success criterion: register/login behavior is covered for current and legacy password formats
- [ ] Add unit tests for wallet controller behavior
  - Success criterion: duplicate wallet, insufficient balance, self-transfer, and success paths are covered
- [ ] Add unit tests for ledger/pricing utilities
  - Success criterion: core accounting and pricing logic have deterministic tests with mocks
- [ ] Add CI-friendly test command documentation
  - Success criterion: contributors can run tests locally without guessing setup

### P2. Project hygiene

- [x] Replace stale task tracker with codebase-accurate tracker
- [ ] Maintain a current state document tied to the roadmap
  - Success criterion: roadmap progress, implemented features, and gaps are easy to audit
- [ ] Update README to reflect actual test/operational state
  - Success criterion: README does not imply stronger validation or launch readiness than exists

## Roadmap progress vs code

### Phase 0

- [x] Mostly implemented in repository code
- [ ] Needs verification and cleanup rather than broad greenfield implementation

### Phase 1

- [ ] Partially started, not complete
  - Gaps: 3-miner consensus per model, work proof gossip/mempool, variable block sizing, streaming inference, broader signature support

### Phase 2

- [>] Started but not production-complete
  - Existing groundwork: wrapped token contract, claim manager, SDK packaging work
  - Gaps: deployment, watchers, bridge operations, liquidity, multisig transition

### Phase 3

- [>] Started but incomplete
  - Existing groundwork: encrypted inference endpoints, silicon attestation research/code
  - Gaps: MPC sharding, pricing/productization, integrated production flow

### Phase 4

- [>] Started but incomplete
  - Existing groundwork: explorer/dashboard
  - Gaps: light-client proofs, governance, end-to-end integration tests, onboarding/ops maturity

## Known gaps

- [ ] No meaningful automated test suite yet
- [ ] No trustworthy CI pipeline in repo
- [ ] Stale and contradictory documentation remains
- [ ] Login/auth code had legacy hashing mismatch and still needs broader audit
- [ ] Operational hardening items in roadmap remain open
- [ ] Secret-history scrubbing is still explicitly pending in roadmap

## Next 10 tasks

1. [>] Finish auth cleanup and cover it with automated tests.
2. [>] Add wallet controller tests for core transfer safety paths.
3. [ ] Add ledger service tests for transfer validation and wallet cache updates.
4. [ ] Add pricing tests for load-based pricing and automatic bids.
5. [ ] Sweep user-facing API/controller strings for URS leftovers and naming drift.
6. [ ] Create a small CI workflow that runs unit tests on every push/PR.
7. [ ] Write a focused integration smoke test plan for miner, clock node, and inference API.
8. [ ] Reconcile README, INSTALL, and ROADMAP wording with actual implementation status.
9. [ ] Audit pending roadmap items related to updater, bots, and webhook deployment.
10. [ ] Decide whether generated `data/blocks/` should be versioned, fixture-scoped, or ignored.
