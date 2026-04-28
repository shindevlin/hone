# Contributing to BTCPC

Bitcoin Proof of Compute — every token earned, every machine welcome.

## Quick start

```bash
git clone https://github.com/shindevlin/btcpc.git
cd btcpc
npm install
npm test                    # run the full test suite (1100+ tests)
node bin/btcpc-all          # start all roles (api + miner + clock + storage)
```

## Running tests

```bash
npx jest                              # all tests
npx jest tests/ledger.test.js         # single file
npx jest --testPathPatterns='bridge'   # pattern match
```

All tests must pass before submitting a PR. Current count: 1,151+.

## Commit format

We use atomic, versioned commits:

```
v2.X.Y-phase: one-line summary

Longer description of what changed and why.
Reference the architectural rule or feedback memory if applicable.
```

Sub-phases use alpha/beta/gamma/delta suffixes:
- `v2.15-alpha: sensor registry primitive`
- `v2.15-beta: HTTP routes for sensors`
- `self-heal: model verification auto-pull`

## Architecture

- `src/chain/` — blockchain core (stateStore, replay, blockStore, SMT)
- `src/services/` — business logic (ledger, escrow, commerce, bridge, IoT)
- `src/routes/` — HTTP API (Express routers)
- `src/p2p/` — WebSocket peer-to-peer network
- `src/mining/` — miner daemon, reward distribution, model verification
- `src/wallet/` — BIP-39 key management, account creation
- `bin/` — CLI tools (btcpc-mine, btcpc-clock, btcpc-all, btcpc-cli)
- `website/` — btcpc.net static site + install scripts
- `tests/` — Jest test suite
- `docs/` — whitepaper, tokenomics, plans

## Hard rules (never break these)

1. **No burn, ever.** All fees flow to `btcpc_recycle`. See docs/TOKENOMICS.md §5.
2. **Storage is never slashed.** Pay for delivery, not for absence.
3. **42M supply, 10 decimals.** Every BTCPC token (native + user-issued).
4. **No fixed BTCPC promises.** Always % of stream / fraction of pool.
5. **Self-heal, never ask.** Every fail path must auto-repair. Non-technical users can't run commands.

## Security

Report vulnerabilities to shindevlin@proton.me. Do NOT file public issues for security bugs.

The P2P protocol requires cryptographic signatures on messages (v2.16.1+). Set `BTCPC_REQUIRE_SIGNATURES=true` for strict enforcement.

## License

AGPL-3.0-or-later. See [LICENSE](LICENSE).
