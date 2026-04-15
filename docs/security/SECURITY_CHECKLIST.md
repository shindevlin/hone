# BTCPC Pre-Mainnet Security Checklist

## Overview

This checklist tracks security requirements across three tiers. Items must be resolved in tier order: Critical items gate mainnet launch, High Priority items gate testnet expansion, and Medium Priority items gate production readiness.

---

## Tier 1: Critical — Must Fix Before Mainnet

These issues represent attack surfaces that could compromise chain integrity, enable double-rewards, or allow stake theft. Mainnet launch is blocked until all Critical items are checked.

### Consensus and Work Integrity

- [ ] **T-03 / S-03**: Work proof replay is rejected at the verifier layer; duplicate proof hash detection is live and tested across epoch boundaries.
- [ ] **T-01**: Epoch timestamp binding is enforced; work submitted with a mismatched epoch hash is rejected.
- [ ] **T-04**: Verifier selection is pseudo-random and stake-weighted; colluding verifier sets are detectable via dispute challenge.
- [ ] **S-01**: Inactivity deduction logic is implemented and tested; sustained epoch abstention by staked miners triggers warnings then stake deductions.
- [ ] **S-04**: Slash rates are set per the recommended slashing matrix; rates are chain parameters adjustable via governance.

### P2P Security

- [ ] **P2P-01**: All P2P connections use TLS (WSS); plaintext WebSocket is disabled on all production nodes.
- [ ] **P2P-02**: Replay window is reduced to 30 seconds (one epoch); per-session nonces are enforced.
- [ ] **P2P-03**: Peer identity handshake is implemented; staked peers sign connection challenges with their on-chain key.
- [ ] **P2P-05**: Eclipse resistance is implemented; minimum outbound peer diversity by AS is enforced.

### Slashing Infrastructure

- [ ] **S-02**: Slashing evidence gossip is independent of block proposals; evidence included within N epochs of first broadcast.
- [ ] **S-03**: Replay slash rate is set to 10x epoch reward; evidence window is 100 epochs.
- [ ] All slashed stake routes to `btcpc_recycle`; no burn paths exist anywhere in the codebase.

---

## Tier 2: High Priority — Pre-Testnet Expansion

These issues should be resolved before opening the testnet to external validators and miners. They are not immediate mainnet blockers but would create exploitable conditions under load.

### Clock Infrastructure

- [ ] **T-02**: Clock node quorum requires 2/3 supermajority; single clock node cannot shift epoch boundary.
- [ ] **T-02**: Clock nodes that diverge more than 2 seconds from median are excluded from quorum automatically.
- [ ] **T-02**: Clock node compromise triggers stake slash; clock nodes are registered on-chain with a stake requirement.

### Sybil Resistance

- [ ] **S-05**: Minimum stake per verifier registration is enforced (recommended: 100 BTCPC).
- [ ] **S-05**: Per-account selection weight cap is implemented; aggregate Sybil stake does not gain linear selection advantage.

### Network Health

- [ ] **P2P-04**: Seen-message deduplication cache is live; gossip fan-out cap is enforced (recommended: 8 peers).
- [ ] **P2P-04**: Per-peer inbound rate limits are implemented.

### Finality

- [ ] **T-06**: Finality tier logic is implemented: native, L2 (100 epochs), Ethereum anchor (1000 epochs), Bitcoin anchor (10000 epochs).
- [ ] **T-06**: Nodes in low-peer-count state enter reduced-confidence mode and flag unanchored transactions.

---

## Tier 3: Medium Priority — Pre-Production

These items improve robustness and operational security but do not represent acute attack vectors on a well-monitored testnet.

### Governance and Upgradability

- [ ] Slash rates are verified to be chain parameters; a governance proposal flow exists to adjust them.
- [ ] Hash algorithm used for block and proof hashes is a chain parameter with an upgrade path.
- [ ] Clock node set membership is managed on-chain with a defined rotation schedule.

### Monitoring and Alerting

- [ ] Epoch divergence metrics are exported and monitored; alerts fire if clock node quorum degrades.
- [ ] Slashing event log is queryable via the explorer (port 4242).
- [ ] Peer diversity metrics (unique AS count) are visible in node status output.
- [ ] Replay attempt rate is tracked per epoch; spikes trigger automated alerting.

### Operational Security

- [ ] All node operator keys are stored in environment variables or a secrets manager; no keys in git.
- [ ] `.env` files are in `.gitignore` and verified absent from all commits in repo history.
- [ ] Telegram bot tokens are confirmed absent from git history.
- [ ] Bootstrap node list does not contain IP addresses that resolve to a single operator.

---

## Documentation Requirements

- [ ] `THREAT_MODEL_TIMING_WORK.md` is complete and reviewed.
- [ ] `SLASHING_LOGIC_REVIEW.md` is complete and reviewed.
- [ ] `P2P_AUTH_ANALYSIS.md` is complete and reviewed.
- [ ] This checklist is linked from the main `README.md` or `docs/` index.
- [ ] Each Critical item links to the relevant issue or PR where the fix was implemented.

---

## Launch Gates

| Gate | Condition |
|------|-----------|
| Internal testnet | Tier 1 checklist 100% complete |
| External testnet (open validators) | Tier 1 + Tier 2 complete |
| Mainnet launch | Tier 1 + Tier 2 + Tier 3 complete; external audit report received |

---

## Revision History

| Version | Date | Notes |
|---------|------|-------|
| 1.0 | 2026-04-15 | Initial pre-mainnet checklist, three tiers |
