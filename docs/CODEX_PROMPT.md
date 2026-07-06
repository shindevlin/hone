# Codex Work Prompt — BTCPC v3.0.87

You are working on the BTCPC blockchain codebase at ~/repos/btcpc (GitHub: shindevlin/btcpc, private). Version 3.0.87. Node.js, optional MongoDB, WebSocket P2P.

The main Claude engine is building cross-chain claims, wBTCPC contracts, token creation, and tokenomics dashboard. You handle everything else below.

## YOUR TASKS

### 1. Solana/Bitcoin/TON Sign-to-Link

Follow the EVM pattern in `src/services/chainLink.js`. Add signature verification for:

**Solana:** ed25519 signature verification
- User signs the challenge message with Phantom/Solflare
- Use `ed25519-hd-key` (already in dependencies) or Node.js `crypto.verify` with ed25519
- Recover the public key, derive the Solana address, match against claimed address

**Bitcoin:** secp256k1 message signing (BIP-322 or legacy Bitcoin signed message)
- User signs with Sparrow/Electrum/Ledger
- Bitcoin message format: `\x18Bitcoin Signed Message:\n` + length + message
- Recover pubkey, derive bech32 address, match

**TON:** ed25519 signature
- Similar to Solana but with TON address derivation

Add each to `chainLink.js` with a new case in `verifyAndLink()`. Add bot commands `/link-sol`, `/link-btc`, `/link-ton` to the bot at `~/repos/btcpcbot/index.js`.

**IMPORTANT:** Mark these as HIGHLY DISCOURAGED in the UI — warn users that linking non-EVM wallets is experimental and they should prefer EVM linking. The bot message should say: "EVM linking is recommended. Non-EVM chain linking is experimental."

### 2. Public btcpcscan

The explorer runs at localhost:4242. Set up a permanent Cloudflare tunnel:

```bash
cloudflared tunnel create btcpcscan
cloudflared tunnel route dns btcpcscan scan.honemesh.network
```

If `scan.honemesh.network` DNS isn't available, use the auto-generated trycloudflare.com URL and document it.

Create a systemd service: `systemd/btcpcscan-tunnel.service` that keeps the tunnel alive.

### 3. Test Coverage

Expand tests in `tests/` directory. Current: 4 suites, 20 tests. Add:

- `tests/finalizationConsensus.test.js` — proposal submission, consensus resolution, duplicate rejection, single-miner fallback
- `tests/escrow.test.js` — lock, release, refund, sweep stale, releaseForJob
- `tests/chainLink.test.js` — EVM challenge generation, signature recovery, address matching
- `tests/resourceManager.test.js` — mode calculation, idle detection mock, CPU/GPU caps
- `tests/mempool.test.js` — submit, duplicate rejection, nonce reuse, pending debit tracking

Mock MongoDB with jest mocks (see existing tests for pattern). Run: `npm test -- --runInBand`

### 4. Documentation Site

Build a simple hosted docs site from the existing markdown files in `docs/`:

Option A: Use `docsify` (zero-build, serves markdown directly)
```bash
npm install -g docsify-cli
docsify init docs
docsify serve docs
```

Option B: Add a `/docs` route to the explorer that renders markdown files

Add `npm run docs` script to package.json.

### 5. Telegram Mini-App Wallet

Build a Telegram WebApp (Mini App) for wallet management. This runs inside Telegram as an embedded web page.

Create: `src/telegram-webapp/` directory with:
- `index.html` — single-page wallet app
- Uses Telegram WebApp SDK: `https://telegram.org/js/telegram-web-app.js`
- Features: balance display, send tokens, transaction history, heartbeat button, linked wallets
- Calls the existing `/api/bot/*` endpoints
- Dark theme matching btcpcscan (Bitcoin orange accent)

Register the Mini App with the bot — add a menu button that opens the webapp URL.

The webapp should be served from the explorer (add route `/webapp` to `src/explorer/server.js`).

### 6. Multi-Model Inference Routing

Currently all inference goes to one model. Build smart routing:

In `src/inference/handler.js`:
- If request specifies a model → use that model
- If request says "auto" → pick based on prompt complexity (partially exists)
- NEW: if a model is busy/slow, route to the next best available model
- NEW: track model response times and success rates per miner
- NEW: prefer models with higher work_value (bigger models earn more)

Add a model stats tracker that records per-model: avg response time, success rate, tokens/sec.

## IMPORTANT RULES

- Git author: Shin Devlin <shindevlin@proton.me>
- NO Claude attribution in commits
- Bump version for each commit only when the user wants a commit. Always update `package.json` and `package-lock.json` together, and verify both files plus `package-lock.json.packages[""].version` match.
- Don't modify: src/mining/miner.js, src/chain/finalizationConsensus.js, src/services/escrow.js, src/services/ledger.js (main engine is working on these)
- Don't restart running processes
- Read CLAUDE.md for full project context
- Run `npm test -- --runInBand` after changes to verify nothing breaks
