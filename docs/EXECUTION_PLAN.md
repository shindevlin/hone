# BTCPC Execution Plan

Plan date: 2026-04-08

This plan is built from the repository state, `docs/ROADMAP.md`, recent git history, and currently implemented modules. It is intentionally separate from the older GSD workflow files, which are not present in this repo and should not be treated as the source of truth.

## Objectives

1. Stabilize correctness in already-implemented core flows.
2. Add automated tests around the highest-risk live behavior.
3. Clean up naming, docs, and project tracking so execution can proceed without false signals.
4. Move the repo from "implemented but weakly verified" to "measurably stable baseline."

## Phase A. Correctness cleanup

### Scope

- Authentication path consistency
- Legacy terminology cleanup in active API/controller surfaces
- Small correctness fixes that are isolated from in-flight mining/inference edits

### Deliverables

- Registration and login use compatible password verification
- Login accepts username or email
- Legacy password hashes can still be read and upgraded on successful login
- User-facing token terminology consistently says BTCPC where applicable

### Exit criteria

- Core auth tests pass
- No new coupling introduced to modified mining/inference files

## Phase B. Automated test baseline

### Scope

- Unit tests first, with mocks around DB and network dependencies
- Focus on high-value modules already in production paths

### Initial test targets

1. `src/controllers/authController.js`
2. `src/controllers/walletController.js`
3. `src/services/ledger.js`
4. `src/services/pricing.js`

### Exit criteria

- `npm test` runs successfully in a clean local environment
- Critical validation paths have deterministic unit coverage
- New tests are fast enough for CI use

## Phase C. Project hygiene

### Scope

- Replace stale tracker
- Add a current-state execution plan
- Align docs with implemented reality

### Deliverables

- Accurate task tracker in `tasks/tasks.md`
- Execution plan in `docs/EXECUTION_PLAN.md`
- Follow-up state doc or README updates that reflect current validation limits

### Exit criteria

- A new contributor can tell what is built, what is missing, and what to do next without reverse-engineering the repo

## Phase D. Follow-on hardening

These are the next recommended workstreams after the baseline lands.

1. CI workflow for unit tests
2. Integration smoke tests for API + MongoDB
3. Miner/clock-node scenario tests
4. Operational hardening for updater, bot service management, and webhook deployment
5. Documentation and secret-history remediation from roadmap pending items

## Risks

- Mining and inference files already have local uncommitted edits; avoid broad refactors there until they are reviewed.
- The repo contains generated chain data under `data/blocks/`, which may complicate fixture/test strategy.
- Some later-phase features exist in partial form, so documentation must distinguish "implemented," "started," and "production-ready."

## Definition of done for this cleanup pass

- Tracker replaced
- Plan added
- Auth consistency fixed
- Initial automated tests added and passing
- Final report identifies the remaining gaps still outside this pass
