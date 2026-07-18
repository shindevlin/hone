# HONE Go-Live Checklist

Use this checklist before expanding the public testnet surface or asking new users to rely on it.

## Public Surface
- [ ] `https://honemesh.net/testnet` loads
- [ ] `GET /health` returns `OK` on repeated reads
- [ ] `GET /public/machine-status` reports truth-bearing status on repeated reads
- [ ] `GET /public/network` reports connected peers and truth-bearing status on repeated reads
- [ ] `GET /api/node/list` returns node identities with publishable P2P addresses on repeated reads
- [ ] `GET /v1/models` works from a remote client, even after a transient failure
- [ ] `GET /v1/pricing` works from a remote client, even after a transient failure
- [ ] `POST /v1/chat/completions` returns non-empty content after retry

## Smoke Runs
- [ ] `npm run smoke:testnet` passes against the public testnet using retry/quorum checks
- [ ] One run includes an API key so authenticated inference paths are covered
- [ ] One run without an API key still proves the public health pages are reachable
- [ ] Smoke checks tolerate transient 5xx, reconnect, and peer-churn failures as long as quorum eventually succeeds

## Operational Gates
- [ ] At least two real nodes are connected
- [ ] No chain-truth path depends on localhost
- [ ] Node announcements carry a real advertised P2P address
- [ ] Storage host summaries remain redacted
- [ ] Testnet smoke tests are run after each public-facing deployment
- [ ] Public health only counts as green when retry/quorum checks succeed, not on a single lucky read
