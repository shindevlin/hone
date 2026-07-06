# Codex Review Handoff For Claude

Date: 2026-07-05
Repo: `X:\hone`
Reviewer: Codex

## Codex Follow-Up Fixes Applied

After this report was drafted, Codex applied the low-risk fixes below:

- `tests/installerScripts.test.js` now points at `website/hone-start.bat` and `website/hone-start.ps1`.
- The stale installer assertion now expects `hone-setup` instead of `btcpc-setup`.
- `package.json` Jest config now ignores `.claude` for module and watch crawling, not only test paths.
- `rust/hone-node/src/api.rs` GitHub webhook handling now fails closed when `HONE_GITHUB_WEBHOOK_SECRET` is missing or empty.

Verification after those fixes:

- `npm test -- --runInBand tests/installerScripts.test.js`: passed, 30/30 tests.
- `npm test -- --runInBand --forceExit`: passed, 112 suites passed, 1 skipped, 1737 tests passed, 14 skipped.
- `npm test -- --runInBand` without `--forceExit`: assertions passed, but Jest exited nonzero because an existing P2P test leaves async work/logging after completion.

## Scope

This was a targeted review of the current dirty workspace after a broad rebrand/refactor pass. The code-review-graph MCP tools required by `AGENTS.md` were not exposed in this Codex session, so this report is based on targeted local inspection plus `npm test` and `cargo check` attempts.

High-risk areas checked:

- Rust node API routes in `rust/hone-node/src/api.rs`
- wallet/account/TOTP/inference paths
- GitHub webhook handling
- installer and website scripts
- stale rebrand paths in tests and lockfiles

## Executive Summary

There are four issues that should block launch until fixed:

1. Public streaming inference can debit arbitrary accounts without authentication.
2. TOTP setup/enable/disable can be hijacked for any account.
3. GitHub webhook verification was fail-open when the secret was missing. Fixed by Codex follow-up.
4. Public config mutation endpoints can alter node model selection and OTC sale settings.

The JavaScript suite mostly passes, but one test file is stale after the `btcpc` to `hone` file rename. Rust build verification was blocked by missing local Windows build prerequisites, not by a confirmed Rust compile error.

## Findings

### 1. Critical: Unauthenticated Inference Account Debit

File: `rust/hone-node/src/api.rs`

Relevant locations:

- Route: `/api/inference/stream` at about line 376
- Handler: `post_inference_stream` at about line 8110
- Debit: `s.chain.store.debit(&body.account, NATIVE_TOKEN, fee)` at about line 8131

Problem:

`post_inference_stream` accepts `account` in the JSON body, checks that account balance, then debits that account. There is no bearer token, API key lookup, account signature, local-only guard, or proof that the caller controls `body.account`.

Impact:

Any remote caller can repeatedly charge arbitrary accounts 1000 hunits per request and consume the node's inference backend.

Suggested fix:

- Require the same authenticated identity pattern used by `/v1/chat/completions`.
- Resolve `Authorization: Bearer <api_key>` to an account.
- Remove `account` from the trusted request body, or require it to match the resolved account.
- Add a regression test that a request for another account is rejected and no debit occurs.

### 2. Critical: TOTP Setup Can Be Hijacked

File: `rust/hone-node/src/api.rs`

Relevant locations:

- Routes: `/api/totp/setup`, `/api/totp/enable`, `/api/totp/disable`, `/api/totp/backup-codes` at about lines 369-374
- `post_totp_setup` returns the new secret at about line 8026
- `post_totp_enable` enables pending secret at about lines 8032-8044
- `post_totp_disable` deletes active secret at about lines 8065-8075

Problem:

The TOTP endpoints take `account` from the request body and do not verify account ownership. `setup` returns the newly generated TOTP secret to the caller. An attacker can set up a pending secret for a victim account, enable it using the returned code, and later verify or disable TOTP.

Impact:

2FA can be hijacked or disrupted for any account. This undermines account security and any flow that treats TOTP as an authentication factor.

Suggested fix:

- Require owner-key or active-key signature for setup, enable, disable, and backup code generation.
- Bind the challenge/signature to the specific operation and account.
- Consider not returning the raw secret after setup except in the initial setup response.
- Rate-limit setup/enable attempts per account and IP.
- Add tests covering unauthorized setup, wrong-account setup, and authorized setup.

### 3. High: GitHub Webhook Verification Is Fail-Open

Status: fixed by Codex follow-up.

File: `rust/hone-node/src/api.rs`

Relevant locations:

- `post_github_webhook` at about line 1008
- Secret check at about lines 1014-1031

Problem:

Webhook HMAC verification runs only if `HONE_GITHUB_WEBHOOK_SECRET` exists and is non-empty. If the env var is absent or empty, the function continues and processes the webhook.

Impact:

A misconfigured public node can accept forged GitHub webhook events. The handler can create `gh_` accounts and gossip entries based on attacker-supplied payloads.

Suggested fix:

- Fail closed unless the secret is configured.
- Return `503` or `401` when `HONE_GITHUB_WEBHOOK_SECRET` is missing/empty.
- Compare signatures in constant time.
- Require `X-Hub-Signature-256` to be present.
- Add tests for missing secret, empty secret, bad signature, and valid signature.

### 4. High: Public Config Mutation Endpoints

File: `rust/hone-node/src/api.rs`

Relevant locations:

- `/api/node/config` route at about line 318
- `patch_node_config` at about lines 1469-1481
- `/api/purchase/config` route at about line 333
- `post_purchase_config` at about lines 6715-6730

Problem:

`PATCH /api/node/config` changes the active model with no authentication. `POST /api/purchase/config` updates sale config including `enabled`, `price_usd`, `eth_address`, and `operator` with no authentication.

Impact:

An attacker can alter model routing on a public node. More seriously, if OTC purchase routes are exposed, an attacker can change the receiving address or sale price.

Suggested fix:

- Gate node-local config changes behind local-only access or admin auth.
- Gate purchase config behind governance/admin signature.
- Keep GET public if needed, but never POST/PATCH without auth.
- Add tests that anonymous POST/PATCH requests are rejected.

### 5. Medium: Stale Installer Test Paths After Rebrand

Status: fixed by Codex follow-up.

File: `tests/installerScripts.test.js`

Relevant locations:

- `const BAT = path.join(WEBSITE, 'btcpc-start.bat')`
- `const PS1 = path.join(WEBSITE, 'btcpc-start.ps1')`

Problem:

The website now has:

- `website/hone-start.bat`
- `website/hone-start.ps1`

But the test still expects the old `btcpc-start.*` names.

Impact:

`npm test -- --runInBand` fails 28 tests in `installerScripts.test.js`.

Suggested fix:

- Update the constants to `hone-start.bat` and `hone-start.ps1`.
- Update assertion text that references `btcpc-setup` if the installer command was renamed.

### 6. Medium: Jest Crawls Claude Worktrees

Status: fixed by Codex follow-up.

File: `package.json`

Relevant location:

- Jest config `testPathIgnorePatterns` around line 86

Problem:

The test run showed haste module naming collisions from `.claude/worktrees/.../package.json`. `testPathIgnorePatterns` excludes tests there, but Jest still crawls the directory for module names.

Impact:

Local test runs are noisy and can fail before meaningful test output, depending on worktree contents.

Suggested fix:

Add Jest ignore settings:

```json
"modulePathIgnorePatterns": [
  "<rootDir>/\\.claude/"
],
"watchPathIgnorePatterns": [
  "<rootDir>/\\.claude/"
]
```

Keep the existing `testPathIgnorePatterns` entry too.

## Verification Notes

### JavaScript

Command:

```bash
npm test -- --runInBand
```

Result:

- 111 suites passed
- 1 suite failed
- 1709 tests passed
- 28 tests failed
- 14 skipped

The failures are from `tests/installerScripts.test.js` looking for `website/btcpc-start.bat`.

Jest also emitted haste module collision warnings from `.claude/worktrees`.

### Rust

Command:

```bash
cargo check --workspace
```

Result:

The build did not get far enough to validate the source. It was blocked by local Windows prerequisites:

- `librocksdb-sys` could not find `libclang`
- `openssl-sys` could not find `perl` for vendored OpenSSL build

This is an environment blocker, not a confirmed Rust source failure.

## Suggested Fix Order

1. Lock down `/api/inference/stream` auth and account debit.
2. Lock down all TOTP mutation endpoints with account signatures.
3. Make GitHub webhooks fail closed.
4. Add auth/local-only guards for `/api/node/config` and `/api/purchase/config`.
5. Fix stale installer test filenames.
6. Add Jest module/watch ignore patterns for `.claude`.
7. Re-run `npm test -- --runInBand`.
8. Re-run Rust checks in an environment with `libclang` and `perl` installed, or in the existing Linux CI image.

## Notes For Claude

This repo currently has a very broad dirty tree. Most inspected Rust diff appears to be rebrand/import churn from `honemesh_types` to `hone_types` and `HoneMesh` to `HONE`. The risks above are not cosmetic rebrand misses; they are behavioral security holes in exposed API routes.
