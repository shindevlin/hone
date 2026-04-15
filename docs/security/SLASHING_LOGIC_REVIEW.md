# BTCPC Slashing Logic Security Review

## Scope

This document reviews the slashing conditions, rates, and edge cases in BTCPC's proof-of-compute consensus. Slashing applies to miners, verifiers, and staked commerce/storage participants. Storage hosts are explicitly excluded from slashing for absence (they are paid for delivery, not penalized for going offline).

---

## Slashing Principles

- Slashing is punitive, not operational: nodes are slashed for provable misbehavior, not for being offline.
- Fees are never burned. All slashed stake routes to `btcpc_recycle` for redistribution.
- Slashing evidence must be on-chain and disputable within a defined challenge window.
- Rates are calibrated to make attacks economically irrational without making honest errors catastrophic.

---

## Issue Inventory

### S-01: Empty Epoch Collusion

**Description:** A group of miners submit no work in an epoch, collectively suppressing block production to game the reward schedule or deny service to a target.

**Impact:** Epoch skipped or thinly populated; reward pool diluted; potential liveness failure.

**Current Gap:** No penalty for coordinated epoch abstention. A single miner not submitting work is treated as absence, not misbehavior.

**Recommendation:**
- Track per-miner submission rates over a rolling window (e.g., 10 epochs).
- If a staked miner's submission rate falls below a configurable threshold (e.g., 20%) without an announced maintenance window, apply a small inactivity deduction from stake.
- Do not slash for a single missed epoch; only trigger after sustained absence.

---

### S-02: Slashing Evidence Censorship

**Description:** A validator or block proposer omits slashing evidence from a block, preventing a slashable event from being recorded on-chain.

**Impact:** Misbehaving nodes escape punishment; slashing is rendered ineffective.

**Current Gap:** No mechanism forces inclusion of pending slashing evidence within a deadline.

**Recommendation:**
- Slashing evidence submitted to the P2P layer is gossiped independently of block proposals.
- If evidence is not included in a block within N epochs of first broadcast, any node may rebroadcast and include it.
- Block proposers who demonstrably exclude valid evidence (provable via gossip timestamps) are themselves subject to a censorship slash.

---

### S-03: Work Proof Replay (Slashing Angle)

**Description:** A miner replays a valid work proof from a previous epoch. If not caught, they claim double rewards. If caught, the slashing rate must be sufficient to deter the attempt.

**Impact:** Slash deterrent is only effective if the penalty exceeds the expected double-reward gain.

**Current Gap:** Slash rate for replay not yet formally specified relative to expected reward.

**Recommendation:**
- Slash rate for confirmed replay: 10x the epoch reward that would have been earned.
- Replay is provable by comparing proof hashes across epochs; no subjective judgment required.
- Evidence window: valid for 100 epochs after the replayed proof's original epoch.

---

### S-04: Uncalibrated Slash Rates

**Description:** Slash rates set too low are ignored by well-capitalized attackers. Rates set too high create chilling effects on honest participants who make configuration mistakes.

**Impact:** Either the slash regime fails as a deterrent, or it drives away honest miners through fear of misconfiguration penalties.

**Recommendation:**
- Maintain a tiered slash schedule (see Slashing Matrix below).
- Distinguish provable malice (high slash) from configuration error (low slash, warning first).
- All slash rates are chain parameters, adjustable via governance without a hard fork.

---

### S-05: Hardware/Sybil Attacks on Verifier Selection

**Description:** An attacker registers many low-stake accounts to bias verifier selection, increasing their probability of colluding verifier sets.

**Impact:** Attacker gains outsized verifier influence without proportional stake risk.

**Current Gap:** Verifier selection weighted by stake, but minimum stake threshold not enforced per verifier slot.

**Recommendation:**
- Enforce a minimum stake per verifier registration (e.g., 100 BTCPC).
- Apply a Sybil resistance factor: stake per account, not aggregate stake, determines selection weight up to a cap.
- Accounts sharing an IP or signing key pattern are flagged for manual review; automatic soft-cap on selection probability.

---

## Recommended Slashing Matrix

| Offense | Evidence Required | Slash Rate | Routes To | Notes |
|---------|------------------|------------|-----------|-------|
| Work proof replay | Matching proof hashes in two epochs | 10x epoch reward | btcpc_recycle | Objective; no dispute needed |
| Fraudulent work approval (verifier) | Dispute challenge + re-verification | 25% of verifier stake | btcpc_recycle | Challenge window: 10 epochs |
| Slashing evidence censorship (proposer) | Gossip timestamp vs. block timestamp | 5% of proposer stake | btcpc_recycle | Requires gossip timestamp infra |
| Sustained inactivity (staked miner) | Rolling 10-epoch submission rate | 0.5% per epoch below threshold | btcpc_recycle | Warn before slashing |
| Sybil verifier registration | Key/IP clustering analysis | Registration stake forfeited | btcpc_recycle | Governance-triggered |
| Double-sign block proposal | Two signed blocks at same height | 50% of proposer stake | btcpc_recycle | Highest severity |

---

## Exclusions

- **Storage hosts**: Not slashed for going offline. Paid per delivery; absence means no payment, not punishment. See memory note: "Storage is never slashed."
- **Clock nodes**: Slashed for provable timestamp manipulation, not for clock drift within tolerance.

---

## Revision History

| Version | Date | Notes |
|---------|------|-------|
| 1.0 | 2026-04-15 | Initial slashing logic review, S-01 through S-05 |
