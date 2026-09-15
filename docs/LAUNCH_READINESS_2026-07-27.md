## HONE Launch Readiness — 2026-07-27

Measured against `docs/GO_LIVE_CHECKLIST.md`. Evidence from the live node on Beastly
(`hone-node` v1.2.2, HTTP API on `127.0.0.1:4242`) and remote reachability checks.

### Headline

**The application works. There is no live network.** The node runs solo: `peer_count: 0`,
`truth_bearing: false`, and `/api/node/list` shows only the `genesis` node. The public
testnet surface is effectively down. The blocker to launch is **infrastructure/deployment,
not code.**

### What is GREEN (local, single node)

- `GET /health` -> `{"status":"ok"}` (200), stable on repeated reads
- `GET /v1/models` -> 200 (serves `llama2-uncensored.gguf`)
- `GET /v1/pricing` -> 200 (hunits pricing present)
- `GET /api/node/list` -> 200 (returns the genesis node)
- `POST /v1/chat/completions` -> 401 auth-gated (endpoint live; needs a Bearer account to verify output)
- Storage host summary is redacted (`hw_summary: machine:1ad7abad`)
- Node is live and ticking (epoch advancing ~65914 -> 65919)

### What is RED (network + public surface) — the launch blockers

1. **No public bootstrap nodes.** `bootstrap1.honemesh.net` / `bootstrap2.honemesh.net`
   have **no DNS A record**; ports 6942/6943 are closed/unreachable. Nothing to peer with.
2. **Public HTTPS surface not served.** `honemesh.net` (162.255.119.201) and `hone.net`
   (149.255.58.96) resolve to parking IPs; `honemesh.net/testnet` and `/health` time out
   from a remote client. No public node behind the domain + TLS.
3. **Solo node, no quorum.** Beastly has 0 peers, so `truth_bearing` is false. The
   checklist requires **>=2 real nodes connected** and **no chain-truth on localhost**.
4. **Keyed inference path unverified.** `POST /v1/chat/completions` needs one run with a
   real account/API key (Josh has accounts; cannot be created here).

### Go-live checklist status

- Public Surface: local endpoints green; every **public** read is red (surface down).
- Smoke Runs: cannot pass — `smoke-testnet` targets `https://honemesh.net` / `https://hone.net`, not serving.
- Operational Gates: redaction green; >=2 nodes, no-localhost-truth, advertised P2P address all red.

Net: the single-node app is healthy; **~0 of the public/network go-live gates are green.**

### Path to public testnet (ranked)

1. Stand up >=1-2 reachable bootstrap nodes on a public address (VPS, or Grouchly with a
   routable IP + open 6942/6943). Add the `bootstrap1/2.honemesh.net` A records.
   *(Needs Josh: hosting + DNS access.)*
2. Point `honemesh.net` (and/or `hone.net`) at a node and serve TLS for `/health`, `/v1/*`.
   `scripts/hone-testnet.service` already exists as a starting unit. *(Needs Josh: DNS/TLS.)*
3. Peer Beastly to the public node(s) so `peer_count >= 1` and `truth_bearing` flips true;
   confirm >=2 nodes in `/api/node/list` with advertised P2P addresses.
4. Run `scripts/smoke-testnet.sh https://honemesh.net` — one keyed, one unkeyed — under
   retry/quorum until green.
5. **Human-only, separate step:** genesis / chain-id / mainnet cutover. Not unblocked by the above.

### What I can drive vs what needs Josh

- **I can:** write/verify the systemd + bootstrap config, wire peering, harden the retry
  paths, and run the smoke suite once a reachable node + DNS exist.
- **Needs Josh:** a public host (VPS or routable IP), DNS/TLS for the domains, an API key
  for the keyed smoke, and the genesis/mainnet decision.
