# HONE Threat Model: Timing, Work, and Consensus

## Scope

This document covers the primary threat vectors against HONE's proof-of-compute consensus, focusing on timing decoupling, clock infrastructure, work proof integrity, verifier behavior, and network-level attacks.

---

## Trust Assumptions

| Component | Trusted? | Notes |
|-----------|----------|-------|
| Clock nodes | Partially | Majority of clock nodes assumed honest; minority compromise tolerated |
| Ollama runtime | Yes (local) | Miner controls their own inference stack |
| Verifier set | Partially | Requires honest supermajority; collusion threshold defined per epoch |
| P2P network | No | Adversarial; any peer may be malicious or eclipsed |
| MongoDB (legacy) | No | Optional cache only; not authoritative |
| Ledger files | Yes (local) | File-based source of truth on each node |

---

## Threat Inventory

### T-01: Timing Decoupling Attack

**Description:** An attacker manipulates local or network clocks to decouple their epoch timestamp from the canonical chain time, enabling them to submit work outside the valid epoch window while appearing on-time to peers.

**Impact:** Work submitted in a stale or future epoch is accepted; attacker earns rewards without doing valid compute in the correct window.

**Likelihood:** Medium. Requires influence over clock nodes or control of a node's local clock.

**Mitigations:**
- Clock nodes publish signed timestamps; divergence beyond threshold triggers rejection.
- Epoch boundary is derived from the median of clock node reports, not local system time.
- Work submissions include the clock node signature used for epoch binding.

---

### T-02: Clock Node Compromise

**Description:** An attacker compromises one or more clock nodes to shift perceived epoch boundaries, enabling early or late work acceptance.

**Impact:** Epoch windows drift; miners on honest time are disadvantaged; attacker can time submissions to avoid fair competition.

**Likelihood:** Low-Medium. Clock nodes are a small, known set.

**Mitigations:**
- Require a supermajority (e.g., 2/3) of clock nodes to agree on epoch boundary.
- Clock nodes rotate or are staked; compromise triggers slashing of clock node stake.
- Nodes that diverge from median by more than 2 seconds are excluded from quorum.

---

### T-03: Work Proof Replay

**Description:** An attacker resubmits a valid work proof from a previous epoch in a new epoch, claiming a second reward for the same compute.

**Impact:** Double-reward; degrades the integrity of per-epoch work accounting.

**Likelihood:** Medium. Easy to attempt; must be caught at verification.

**Mitigations:**
- Work proof includes epoch hash and block height; replay to a different epoch is invalid.
- Verifiers maintain a seen-proofs set per epoch; duplicate submission is rejected.
- Proofs are bound to the submitting account and epoch nonce.

---

### T-04: Verifier Collusion

**Description:** A subset of verifiers collude to approve fraudulent work proofs or reject valid ones, manipulating reward distribution.

**Impact:** Fraudulent miners earn rewards; honest miners are excluded.

**Likelihood:** Low. Requires coordination among staked parties with large slash risk.

**Mitigations:**
- Verifier selection is pseudo-random, weighted by stake, per epoch.
- Slashing applies to verifiers who approve proofs later shown to be invalid.
- Dispute window allows any node to challenge a verified result within N epochs.

---

### T-05: Consensus Hash Collision

**Description:** An attacker engineers a block or work proof whose hash collides with a legitimate one, enabling substitution in the chain state.

**Impact:** Chain integrity violation; malicious state accepted as canonical.

**Likelihood:** Negligible for SHA-256/SHA-3 under current hardware. Theoretical long-term risk.

**Mitigations:**
- Use SHA-256 for all proof-of-work hashes; no truncation.
- Block hashes include the full Merkle root of all epoch work proofs.
- Hash algorithm is a chain parameter; can be upgraded via governance.

---

### T-06: Network Partition Fork

**Description:** A network partition splits the peer set; each partition advances its own chain. On reconnect, conflicting forks exist.

**Impact:** Double-spend window; one partition's transactions are orphaned.

**Likelihood:** Medium on a small, early network with few peers.

**Mitigations:**
- Longest-chain rule with work-weighted tiebreaking.
- Finality tiers: native (immediate), L2 (100 epochs), Ethereum anchor (1000 epochs), Bitcoin anchor (10000 epochs).
- Nodes detect low peer count and enter a reduced-confidence mode, flagging unanchored transactions.

---

## Failure Modes and Recovery

| Threat | Failure Mode | Detection | Recovery |
|--------|-------------|-----------|----------|
| T-01 Timing decoupling | Stale/future work accepted | Epoch hash mismatch on sync | Reject non-matching work; re-sync from peers |
| T-02 Clock node compromise | Epoch boundary drift | Clock node divergence metric | Exclude outlier clock nodes; use remaining quorum |
| T-03 Work proof replay | Double reward | Duplicate proof hash in epoch | Reject duplicate; slash if deliberate replay detected |
| T-04 Verifier collusion | Fraudulent work passes | Dispute challenge succeeds | Slash colluding verifiers; re-verify epoch |
| T-05 Hash collision | Chain state substitution | Hash verification failure | Node rejects block; peer banning |
| T-06 Network partition | Fork on reconnect | Chain height divergence | Longest-chain selection; orphan shorter fork |

---

## Revision History

| Version | Date | Notes |
|---------|------|-------|
| 1.0 | 2026-04-15 | Initial threat model, T-01 through T-06 |
