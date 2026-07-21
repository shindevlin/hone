# Security Audit Notes

Branch: `security-hardening-audit`

## Scope
- Review the full repo for exposed HTTP surfaces, SSRF primitives, auth gaps, and secret handling.
- Harden the highest-risk paths first.

## Findings Addressed
- Public tool execution endpoint was callable without auth.
- Browser clock heartbeats could contribute to reward accounting without a verified JWT.
- Saved MCP URLs could be used as backend fetch targets with only weak hostname filtering.
- JWT secrets were being written into the repo-local `.env`.

## Changes Applied
- Added `src/services/urlSafety.js` for shared public-URL validation.
- Locked `POST /api/tools/call`, `POST /api/tools/register`, `POST /api/tools/mcp/start`, and `POST /api/tools/mcp/stop` to loopback-only requests.
- Required a verified JWT before browser clock activity is credited.
- Replaced `.env` JWT persistence with `~/.hone/jwt_secret`.
- Added tests for URL safety and reserved username validation.

## Notes
- The repository has 122 Jest test files.
- A full coverage run is long-running in this workspace; targeted security tests pass.
- Root `npx jest --runInBand` now passes cleanly after excluding the nested `camofox-browser/` workspace from the main Jest ignore list.
- `npm test` still depends on a locally installed `jest` binary in this workspace; verification here used `npx jest --runInBand` instead.
- Consensus redesign target: clocks only seal time, workers only submit work, verifiers review and finalize work, reviewers arbitrate disputes, and settlement must be based on a frozen request/submission snapshot rather than the live epoch view.
- Review economics target: challenges should require payment, review fees should be separate from inference fees, upheld challenges should refund inference cost but not review cost, denied challenges should forfeit both, and reviewer selection should be weighted against stake concentration with anti-sybil slashing.
- In progress: `src/services/inferenceMarket.js` now has a review/challenge/finality lifecycle, `src/routes/inferenceMarketRoutes.js` exposes review/challenge/finalize endpoints, and `src/services/marketplaceSweep.js` performs finality sweeps after the challenge window closes.
- In progress: review fees are escrowed at job open, challenge bonds are escrowed only when a challenge is filed, and self-review is rejected in the service layer.
- In progress: human review committees now record individual votes, slash only dissenters for that event, and increment reviewer reputation based on agreement versus the winning vote.
- In progress: cross-chain finality announcements now publish finalized inference work batches after the challenge window closes, so the finality epoch is the external trust boundary rather than the request epoch.
- Verified subset:
  - `tests/toolRoutes.security.test.js`
  - `tests/publicRoutes.security.test.js`
  - `tests/urlSafety.test.js`
  - `tests/accountManagerUsername.test.js`
- `tests/publicRoutes.security.test.js` needed a mocked `setInterval` and mocked rate limiter to avoid Jest open-handle warnings from module-level startup code.
- Existing unrelated worktree edits were present before this branch and were left untouched.
