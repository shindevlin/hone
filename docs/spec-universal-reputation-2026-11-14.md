# HONE universal reputation (Josh, 2026-11-14)

## Josh's direction
- "A device that is collecting other data is building reputation."
- "I want a universal reputation system built": one system for every user and every device, across everything they do.
- Early network: single-device evidence must still count. Missing confirmation is only held against a device when other devices were there to confirm.

## 0. What exists today (to be replaced)
| Piece | Where | Problem |
|---|---|---|
| Inference reputation (starts 5000) | `inference.rs get_reputation/set_reputation` | AI jobs only |
| Oracle reputation | `api.rs get_oracle_reputation` | oracle only |
| Clock uptime reputation (Phase 8) | `chain.rs` ~5071 | measures registration, not behaviour |
| `sensor_strikes:{id}` | `api.rs` ~4486 | **written in the API handler by whichever node receives the reading: not consensus, can diverge between nodes** |
| `decide_strikes:{node}` | `decide_jury.rs` | voting checks only |
| Trust score, open-device parity, trust tiers A/B/C | design docs only | never built |

## 1. Subjects
- **Device:** any registered key that signs data or work: Flipper, phone seat, puck, nebra, Max, clock node, storage node, inference worker, oracle reporter. Keyed by the registered device public key (owner@pubkey).
- **Owner:** the account that owns devices (josh, shindevlin, ...).
- Public readings keep their daily-rotating tokens. Reputation lives on the registered device key, which the chain already knows because it signs the commits. No new linkage is published.

## 2. Domains and score
- One score per domain, plus a composite:
  `data` (sensor/observation), `alert`, `storage`, `compute` (inference), `consensus` (clock/vote/setcert), `oracle`, `relay`.
- Each domain keeps two evidence counters, fixed-point integers (x1000) for determinism:
  `good` and `bad`, plus a prior.
- **Score** (0-10,000) = lower confidence bound of good/(good+bad):
  `score = 10000 * (good + P_g) / (good + bad + P_g + P_b) - uncertainty(good+bad)`
  so a new device with little evidence scores low-middle, and confidence grows with evidence. Integer math only; the exact formula is in the code and tests.
- **Composite** = evidence-weighted average over the domains the subject actually works in. A domain with no evidence doesn't drag it down.
- **Bad weighs more than good:** a contradiction costs about 3x a confirmation; a proven violation resets the domain to zero evidence with a heavy bad count.

## 3. Events (all from decided sets, applied at decide_by; never in API handlers)
| Event | Domain | Effect |
|---|---|---|
| Reading accepted as useful (novel/changed) | data | +small, diminishing per window |
| Reading later confirmed by an independent owner (rep >= threshold) | data | +medium |
| Contradicted by independent owners | data | -large |
| Self-consistent across the owner's own radios (e.g. Flipper + own phone) | data / alert | +small (evidence quality, **not** confirmation) |
| Persistent threat later re-seen by anyone | alert | +medium (deferred) |
| Alert with capable other-owner devices present that saw nothing | alert | -medium |
| Alert with nobody else capable present | alert | 0 (no penalty) |
| Physical challenge passed / failed | data | +medium / -large |
| Random audit passed / failed | any | +medium / -very large |
| Storage proof passed / strike | storage | +small / -medium |
| Inference job completed+accepted / failed | compute | +/- (migrates current logic) |
| Seal in quorum, certificate signed on time / missed spot check | consensus | +small / -medium |
| Oracle report within band / outside | oracle | +/- |
| Relayed entry that was certified | relay | +small |
| Teleport (impossible travel) | data | -large, window voided |
| Proven violation | all of that device | reset + slash (existing rules) |

## 4. Time
- **Decay:** every 2,880 epochs (1 day) both counters x 0.98 (half-life ~34 days). Old good behaviour fades; so does old bad behaviour.
- **Quiet devices** keep their score but lose confidence as evidence decays.

## 5. Starting reputation (Josh's clarification, 2026-11-14: overrides the first draft)
- **Reputation is a network of useful work.** It is earned ONLY from proven useful work: accepted useful data, independent confirmations, passed challenges/audits, certified seals and relays, proven storage, accepted jobs. Nothing is earned for existing, registration, uptime or stake alone.
- **Owner trust is the main source of a new device's trust.** A new device of an established owner (e.g. josh, shindevlin) starts at about the owner's score: its prior is the owner's score carried as at most 20 evidence units (`INHERIT_CAP`; fewer if the owner has less evidence). That is low confidence, so bad events pull it down fast. Josh's second Flipper does not start from zero.
- **New / unknown owner, open firmware:** neutral-low prior (score 3,500); the device has to earn it (replaces the 30-day parity rule with evidence).
- **Locked/attested hardware:** higher prior (score 6,500), never below it (hook ready; attestation not on chain yet).
- New devices count toward their owner only after 2,880 epochs.

## 6. Owner reputation
- Evidence-weighted average of the owner's device composites, capped by the best device's confidence.
- A proven violation on any device hits the owner (shared -large) and the inheritance for future devices.
- Anti-sybil: many cheap devices don't raise the owner score faster than evidence accrues; confirmations only count from **other owners** with reputation >= threshold; one owner's devices never confirm each other.

## 6a. Violations and stake (Josh, 2026-11-14)
- **Kinds of violation:** equivocation (two different signed statements for the same slot: a clock double-sign, or two different device commits at the same device seq), fabrication, replay. Only these are slashable; a rejected stale commit alone is not.
- **Violation also kills the stake.** A PROVEN violation (today: clock double-sign evidence; later: fabrication/replay proven by audit hooks) slashes 100% of every `role_stake` backing that device, any role and any staker, to the recycle fund. This overrides the 10%-per-incident cap for proven violation only; ordinary strikes keep the existing slash rules. Supply is conserved (recycle receives exactly what was removed). The owner's OTHER devices are not slashed; they take the reputation hit only.
- **Violation is per device unless proven account-wide.** Account-wide needs proof:
  1. violation evidence signed by the owner's account key itself (not just a device key): account-wide on its own (hook ready; no such evidence type yet), or
  2. coordinated: proven violations on at least `max(ACCOUNT_VIOLATION_MIN_DEVICES = 2, ceil(ACCOUNT_VIOLATION_SHARE = 10% of the owner's ACTIVE devices))` of its devices within `ACCOUNT_VIOLATION_WINDOW = 20,160` epochs (7 days). An owner with thousands of devices needs proportionally more. Active device = registered and with at least one useful-work event within the last 20,160 epochs, so registering idle devices never raises the threshold.
- **Account-wide violation:** 100% of every stake the account placed (any device, any role) goes to recycle, and the owner's reputation is reset: an account penalty holds the owner score, and so every new device's inheritance, at the violation floor (0) while the penalty is at least half its size (about 34 days of decay).
- Below the threshold each violation stays per device.

## 6b. Sync before submit (all devices)
Nothing signs before it has looked at the chain.
- **Nodes** (local behaviour, not gated): on start, restart, or after a gap of more than a few epochs a node is observe-only: it signs nothing (seals, setcert proposals/votes, decide commits/reveals, clock claims) until it has synced to the finalized head. It then raises its local "highest signed" floor per signing type to the max of its local record and what the chain shows signed by its own key (covers lost or restored state, e.g. snapshot swaps). Doppelganger wait: it listens 2 epochs; any signature by its own key that it did not make this run means another instance runs with the same key, so it refuses to sign and logs loudly. Only then does it sign, never at or below the floor. The floor is persisted next to `setcert_voted`.
- **Devices** (chain rule behind REPUTATION_EPOCH): every device that commits data (SensorDataCommit, SensorSummary: Flipper heads, phone seats, pucks) has a committed head in state, `{seq, head_hash, epoch}`, served at `GET /api/device/:key/head`. Before submitting, a device fetches its head and reconciles its local log: its next commit carries `seq > head.seq` and `prev_head == head.head_hash` (a skipped seq is a declared gap, still hash-linked; the first commit has no prev_head). A commit that does not extend the head is rejected (not slashed) with an error telling the device to resync. If the chain head is not in the device's local history, the device has been cloned or restored: it stops and alerts the owner.
- **Equivocation for devices** is two different signed commits for the same device seq, both seen on chain or gossip: that is the provable violation. A rejected stale commit alone is not.

## 7. What reputation controls
- **Reward weight:** reward x (0.25 ... 1.0) by domain score; unconfirmed single-device data/alerts paid by reputation, not withheld.
- **Late window and weighting:** tier (A/B/C) x reputation.
- **Audit rate:** inversely proportional to score (low rep audited often).
- **Gates:** inference bid `min_reputation` uses compute score; committee lottery eligibility can use consensus score (later, separate gate).

## 8. Migration
- Inference and oracle reputation become the compute and oracle domains: current values seed the counters at activation, but never with more good evidence than the old record proved (completed jobs, in-band reports).
- Clock uptime (Phase 8) is NOT seeded: it measures registration/uptime, not proven work. The consensus domain starts from certified seals after activation.
- `sensor_strikes` moves out of the API handler into decided-set processing (fixes the divergence) and becomes data-domain bad events.
- From the gate the sensor commit handler writes no chain state at all: `sensor_net` (the /24 diversity guard) and the informational 2-of-N status move to a node-local admission cache, and the decided `SensorDataCommit` records `sensor_readings` (submitter, batch, count; the signed entry carries no value, so value-based checks are the decided usefulness rules).
- `decide_strikes` become consensus-domain bad events.

## 9. Chain
- Gate `REPUTATION_EPOCH = u64::MAX` until Josh's OK. Before it, behaviour stays byte-identical.
- State: `rep:{subject}:{domain}` -> {good, bad, prior, last_decay_epoch}; `rep_owner:{account}` derived.
- API: `GET /api/reputation/:subject` (domains, composite, confidence, recent events); `GET /api/reputation/owner/:account`.
- App: owner sees each of their devices' reputation and why it moved.

## 10. Tests
- Deterministic across nodes (same decided sets -> same state), integer math only.
- Gate off -> byte-identical state.
- Decay, inheritance caps, owner cap, same-owner confirmations ignored, absent-witness = no penalty, violation reset.
- Migration seeds match current inference/oracle/clock values.

## 11. Next: attack alerts on top
Signed Flipper alerts (deauth, jamming, evil twin, rogue AP, skimmer) -> nearby HONE phones over BLE (flag + GATT pull) and pushed to the same area. Phones verify the device key against the on-chain registry offline; owner sees "Your Flipper", others "Verified HONE device". Opt-in. Reward via the alert domain above.
