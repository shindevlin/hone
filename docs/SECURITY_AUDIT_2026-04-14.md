# BTCPC Security Audit — 2026-04-14 Pre-Genesis

Scope: fast pre-genesis audit of dependency advisories, obvious secret leaks,
JWT/session handling, genesis timestamp drift, public routes, and website globe
runtime behavior.

## Fixed In Working Tree

- Critical dependency advisory removed:
  - `@ton/ton@16.2.3` bundled `axios@1.14.0`, affected by critical SSRF advisories.
  - Pinned `@ton/ton` to `16.2.2`, which dedupes to root `axios@1.15.0`.
- Moderate dependency advisory removed:
  - `follow-redirects@1.15.11` leaked auth headers across cross-domain redirects.
  - Updated to `follow-redirects@1.16.0`.
- JWT persistence bug fixed:
  - Server self-heal wrote `BTCPC_JWT_SECRET` to `.env` but only read `JWT_SECRET` on restart.
  - `src/index.js` now accepts `BTCPC_JWT_SECRET` as an alias and writes both names on self-heal.
  - Public/bot token signing no longer uses weak hardcoded fallback secrets.
  - Auth middleware verifies against `JWT_SECRET || BTCPC_JWT_SECRET`.
- Website globe privacy hardening:
  - `website/globe.html` now maps accounts to broad metro buckets and deterministic 15-50km sector placement.
  - Users physically next door should not render as neighboring points.

## Current Audit Output

`npm audit --omit=dev --json` after fixes:

- Critical: 0
- High: 0
- Moderate: 0
- Low: 4

Remaining low-severity advisories are all secp256k1/elliptic chain:

- `secp256k1` via `elliptic`
- `@hiveio/dhive` via `secp256k1`
- `hdkey` via `secp256k1`
- `elliptic` risky implementation advisory

No non-breaking fix is currently available for that chain. Do not force-downgrade
`hdkey` to `0.6.0` before genesis without testing wallet derivation thoroughly.

## Commands Run

- `npm audit --omit=dev --json`
- `npm ls @ton/ton axios follow-redirects`
- `rg` secret scan for private keys, mnemonics, JWT/API key patterns, Mongo URIs
- `rg` genesis/epoch scan for `1776236400000`, `30000`, and `300000`
- `npx jest tests/authMiddlewareSecretStore.test.js tests/securityFixes.test.js tests/botApiRedesign.test.js --runInBand`
- `node --check src/index.js`
- `node --check src/routes/publicRoutes.js`
- `node --check src/routes/botRoutes.js`
- `node --check src/middlewares/auth.js`
- Extracted `website/globe.html` module script and ran `node --check` against it
- Rendered local preview: `PORT=14243 node website/serve.js`, screenshot to `/tmp/btcpc-website-globe.png`
- `git diff --check`

## Notes For Next LLM

- Do not change `src/services/epochManager.js` genesis timestamp. It is correctly set to `1776236400000`.
- Package metadata mismatch was fixed in the working tree: `package.json`, `package-lock.json`, and lock root now report `3.0.87`.
- Future version bumps must update `package.json` and `package-lock.json` together.
- `bin/btcpc-chain-monitor` and several timeout constants use `300000` for monitor/backoff/timeouts, not epoch length.
- `src/chain/authorityRotation.js` still has a stale comment saying default epoch length is 300000 ms; comment only.
- Public signup/login rely on password + secretStore and rate limits from `src/index.js`; focused tests passed.
- `/api/bot/export-mnemonic` is deliberately disabled and returns 403.
