# Contributing to HONE

Bitcoin Proof of Compute — every token earned, every machine welcome.

## Quick start

```bash
git clone https://github.com/shindevlin/hone.git
cd hone
npm install
npm test                    # run the full test suite (1100+ tests)
node bin/hone-all          # start all roles (api + miner + clock + storage)
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
- `bin/` — CLI tools (hone-mine, hone-clock, hone-all, hone-cli)
- `website/` — hone.net static site + install scripts
- `tests/` — Jest test suite
- `docs/` — whitepaper, tokenomics, plans

## Hard rules (never break these)

1. **No burn, ever.** All fees flow to `hone_recycle`. See docs/TOKENOMICS.md §5.
2. **Storage is never slashed.** Pay for delivery, not for absence.
3. **42M supply, 10 decimals.** Every HONE token (native + user-issued).
4. **No fixed HONE promises.** Always % of stream / fraction of pool.
5. **Self-heal, never ask.** Every fail path must auto-repair. Non-technical users can't run commands.

## Security

Report vulnerabilities to shindevlin@proton.me. Do NOT file public issues for security bugs.

The P2P protocol requires cryptographic signatures on messages (v2.16.1+). Set `HONE_REQUIRE_SIGNATURES=true` for strict enforcement.

## License and contributions

HONE is licensed under the Usufruct License (UFL) 3.4, Operational Scope:
Decentralized (`LicenseRef-UFL-3.4-D`). See [LICENSE](LICENSE).

Contributions are governed by Section 4 of that license. A Contribution is any
fix, change, or addition you submit, by pull request, patch, message, or any
other means, whether or not it is accepted. By submitting one you grant the
Licensor a perpetual, worldwide, irrevocable, royalty-free, non-exclusive
license to use, reproduce, modify, distribute, sublicense, and relicense it,
under the UFL or any other terms, including any patent rights you hold that it
would infringe. Nothing is owed to you for it, and you keep ownership. Submit
only what you have the right to license: if an employer, client, or anyone else
has rights in your work, you need their authority first.

This is a summary. Section 4 of [LICENSE](LICENSE) governs.

### Contributor acknowledgement (CLA)

HONE records each contributor's agreement to Section 4 before merging:

1. Open a pull request. Patches sent any other way are not considered.
2. The CLA check comments on your pull request. Post the sign comment it shows
   (see [CLA.md](CLA.md)). You sign once; it covers your later pull requests.
   The record (GitHub username and numeric ID, time, pull request) is kept in
   `signatures/cla.json` on the public `cla-signatures` branch.
3. Sign your commits (`git config commit.gpgsign true`, with a GPG, SSH, or
   S/MIME key added to your GitHub account). Unsigned commits are not merged.
4. If an employer or client has rights in your work, get their authority before
   you submit it (Section 4).

A pull request opened by an automated agent is submitted by whoever runs that
agent, and that person or organization signs. [CLA.md](CLA.md) adds nothing to
Section 4 and changes nothing in it.

Third-party code in this repository keeps its own license; see [NOTICE](NOTICE).
