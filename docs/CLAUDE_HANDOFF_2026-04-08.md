# Claude Handoff Notes

Date: 2026-04-08

Purpose: persistent notes for the next Claude/Codex session so repository state does not need to be reverse-engineered again.

## Why this pass happened

The repo had a large mismatch between actual implementation and project tracking:

- `tasks/tasks.md` described an older URSNode-era scaffold state and claimed major modules were missing.
- There was no local `.planning`, `PROJECT.md`, or `PLAN.md` GSD state to rely on.
- The codebase itself already contains implemented BTCPC server, ledger, mining, inference, explorer, and cross-chain groundwork.

This pass replaced stale planning material with repo-accurate documents, fixed a real auth bug, and added the first real automated test baseline plus CI.

## Files added

- [docs/EXECUTION_PLAN.md](/home/ubuntclaw/repos/btcpc/docs/EXECUTION_PLAN.md)
- [docs/PROJECT_STATE.md](/home/ubuntclaw/repos/btcpc/docs/PROJECT_STATE.md)
- [docs/CLAUDE_HANDOFF_2026-04-08.md](/home/ubuntclaw/repos/btcpc/docs/CLAUDE_HANDOFF_2026-04-08.md)
- [tests/authController.test.js](/home/ubuntclaw/repos/btcpc/tests/authController.test.js)
- [tests/walletController.test.js](/home/ubuntclaw/repos/btcpc/tests/walletController.test.js)
- [tests/ledger.test.js](/home/ubuntclaw/repos/btcpc/tests/ledger.test.js)
- [tests/pricing.test.js](/home/ubuntclaw/repos/btcpc/tests/pricing.test.js)
- [.github/workflows/test.yml](/home/ubuntclaw/repos/btcpc/.github/workflows/test.yml)

## Files changed in this pass

- [src/controllers/authController.js](/home/ubuntclaw/repos/btcpc/src/controllers/authController.js)
- [src/wallet/accountManager.js](/home/ubuntclaw/repos/btcpc/src/wallet/accountManager.js)
- [src/controllers/walletController.js](/home/ubuntclaw/repos/btcpc/src/controllers/walletController.js)
- [tasks/tasks.md](/home/ubuntclaw/repos/btcpc/tasks/tasks.md)
- [README.md](/home/ubuntclaw/repos/btcpc/README.md)

## Important fixes made

### 1. Registration/login password mismatch fixed

Before this pass:

- `createAccount()` in `src/wallet/accountManager.js` stored passwords as raw SHA-256 hex.
- `loginUser()` in `src/controllers/authController.js` verified passwords with `bcrypt.compareSync`.

Effect:

- freshly registered users would not log in correctly through the normal controller path.

What changed:

- New accounts now store bcrypt hashes.
- password changes also write bcrypt hashes.
- login now accepts either `username` or `email`.
- legacy SHA-256 hashes are still accepted and automatically upgraded to bcrypt on successful login.

Why this matters:

- It fixes a real correctness bug without breaking previously created accounts.

### 2. Legacy terminology cleanup

- Replaced visible `URS` token wording in `walletController` with `BTCPC` wording in the touched user-facing messages/comments.

This is not a full repo-wide terminology sweep. More cleanup is still needed.

### 3. Repo-accurate project tracking added

- Rebuilt `tasks/tasks.md` to reflect actual BTCPC state rather than the old scaffold list.
- Added `docs/PROJECT_STATE.md` for a high-level “what exists / what is missing” snapshot.
- Added `docs/EXECUTION_PLAN.md` for next-step execution planning.

## Tests added

### `tests/authController.test.js`

Covers:

- registration response shape
- login with username + bcrypt password
- legacy SHA-256 password acceptance and upgrade
- invalid credential rejection
- helper behavior for legacy-hash recognition

### `tests/walletController.test.js`

Covers:

- duplicate wallet rejection
- missing wallet balance lookup
- insufficient balance rejection
- self-transfer rejection
- successful ledger-backed transfer path
- transaction history fetch behavior

### `tests/ledger.test.js`

Covers:

- self-transfer validation in ledger service
- successful transfer ledger write + wallet cache updates
- balance computation from ledger aggregates
- pending-entry flush behavior
- current-epoch lookup behavior

### `tests/pricing.test.js`

Covers:

- empty finalized-epoch window => zero network load
- current pricing from load + model weight
- deterministic cost calculation
- auto-bid behavior with miner count and block reward coverage

## CI added

- Added GitHub Actions workflow: `.github/workflows/test.yml`
- Runs on push to `main` and on pull requests
- Uses Node 20
- Runs `npm ci` and `npm test -- --runInBand`

## README changes

Added:

- a `Project Status` section that states the repo is implemented but still being verified/cleaned up
- a `Testing` section with the test command and current scope
- links to the new state/plan/tracker docs

Goal:

- avoid implying broader validation maturity than the repo currently has

## Existing local changes intentionally not touched

These files were already dirty and appear to be active work:

- [src/inference/handler.js](/home/ubuntclaw/repos/btcpc/src/inference/handler.js)
- [src/mining/miner.js](/home/ubuntclaw/repos/btcpc/src/mining/miner.js)
- [src/models/Transaction.js](/home/ubuntclaw/repos/btcpc/src/models/Transaction.js)

They were intentionally not normalized/refactored during this pass.

Reason:

- avoid overwriting or entangling in-flight mining/inference work while doing cleanup and tests

## Known untracked or unrelated worktree items

At the time of this pass, the worktree also included:

- [data/blocks](/home/ubuntclaw/repos/btcpc/data/blocks)
- [infra_test.js](/home/ubuntclaw/repos/btcpc/infra_test.js)
- [telegram-bot/package-lock.json](/home/ubuntclaw/repos/btcpc/telegram-bot/package-lock.json)

These were not modified.

## Verification performed

Command run:

```bash
npm test -- --runInBand
```

Result at end of pass:

- 4 test suites passed
- all tests passed

If a later session sees failures, first check whether local dirty changes altered module contracts after this baseline.

## Recommended next tasks

1. Expand tests around `src/inference/api.js`, `src/services/escrow.js`, and `src/services/modelVerifier.js`.
2. Add integration smoke tests for API boot with mocked MongoDB or test containers.
3. Perform a broader terminology sweep for legacy `URS` references.
4. Reconcile `CLAUDE.md`, `README.md`, and roadmap language with current launch/verification reality.
5. Decide whether generated `data/blocks/` belongs in version control, fixtures, or `.gitignore`.
6. Review the dirty mining/inference edits before any wider refactor.

## Cautions for next session

- Do not revert the dirty mining/inference files unless the user explicitly asks.
- Preserve the new auth backward-compatibility path unless there is a migration plan for legacy SHA-256 users.
- Treat `tasks/tasks.md` as the current execution tracker unless a newer dated tracker replaces it.
- Do not assume roadmap “pending” means “unstarted”; several later-phase items already have partial groundwork in code.
