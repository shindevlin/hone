# Committee lottery voting: safety and liveness review (design only, 2026-11-13)

**Status: design only. Nothing is built.** Building waits for the snapshot, the e61bd096 rollout and Josh's OK.

**Approved plan (Josh, 2026-11-12)**
- **Pool:** grouchly, beastly, nebra, ferryvps, the Pixel and the moto.
- **Eligibility:** a device must have signed a seal or vote within the last N epochs, read from finalized data.
- **Committee:** for each epoch, an odd committee of 3 is drawn deterministically, seeded by the previous certificate.
- **Certify:** 2 of the 3, with at least 2 operators where possible.
- **Fallback:** if there is no certificate within K s, round r+1 draws a new committee with seed `H(prev cert ‖ epoch ‖ r)`.
- **Phones:** a light vote mode for when they are on the committee.

## 1. The safety problem with "2 of a 3-member committee"

**Why today's rule is safe.** Two certificates for the same epoch can never both exist when every pair of certifying quorums shares at least one honest voter. A voter that shares two quorums is locked: it won't sign a second, conflicting certificate. Today every epoch uses the same 3 voters with quorum 2, and any two 2-of-3 sets overlap.

**Why the committee rule breaks that.** With rotating committees drawn from 6 devices, the committees of rounds r and r+1 can be disjoint, for example {grouchly, beastly, nebra} and then {ferryvps, Pixel, moto}. Their 2-of-3 quorums then share nobody, and lock rules only bind a voter to itself. So this can happen:
- Round r certifies set X. Its certificate is delayed: a partition, or a slow phone link. That is exactly the 08:00Z LAN outage case.
- After K s, round r+1's committee certifies a different set Y.
- Both are valid quorum certificates for the same epoch.
- The next epoch's committees may build on X on one side and Y on the other, which is a fork.

The timeout does not prevent this. It is only safe if every message arrives within K s, and a partition breaks exactly that assumption. **"Two rounds can never certify different sets" does not hold for disjoint committees.**

**What safety requires (quorum intersection):** any two quorums that can certify the same epoch, in any round, must share at least one voter. With n eligible voters and quorum q, that means q + q > n, so q > n/2 of the **whole eligible set**, not of a committee. With 6 eligible that is 4 votes, whoever the committee is.

## 2. Proposed safe variant (same goals, kept safe)

| Piece | Rule |
|---|---|
| Eligible set E(T) | Devices with a seal or vote recorded in **certified** data in [T−L−N, T−L] (L = finality depth, e.g. 4). Every node computes the same set from the same certified chain. |
| Quorum | More than \|E(T)\|/2 votes, with at least 2 distinct operators (as today). |
| Committee lottery | Picks the **proposers and leader order** for the epoch (seed = previous certificate digest ‖ epoch ‖ r). It also caps message load: non-committee devices only vote. It does not lower the quorum. |
| Rounds | As now: the union grows, prev stays locked, and the round increases after K s. Votes in any round count toward the same quorum, so two rounds can't both certify. |
| Phones (light mode) | Sign votes over a certificate digest prepared by a full node (prop union, prev, content hash), with no pool or bodies of their own. They need a full node they trust for content, and they verify only the digest chain and their own lock. Their seal and vote activity makes them eligible. |
| Operator identities | One voter id per device, `owner#tag` (for example shindevlin#vps), so beastly and ferryvps can both be shindevlin voters. The operator is the part before `#`; the 2-operator minimum is unchanged. Clock registration needs a key per device instead of per account (`clock_reg:{id}`). Gated. |

**Two consequences to accept:**
- **Offline devices fall out automatically, but only through certified data.** If a device stops signing, it leaves E after N epochs, so phones that don't send don't count toward the quorum.
  - But E is computed from certified data. If more than half of E vanishes at once (a stall), nothing new is certified and E never shrinks.
  - Getting out of that needs those voters to come back, or a governance reconfiguration signed by the council.
  - **Any rule that lets the remaining voters certify without a majority of the current set can be used by both sides of a partition**, and that is a fork.
- **A LAN outage stalls whenever LAN devices are half or more of E** (they would be most of the time). The catch-up fix (e61bd096) recovers automatically within minutes when the LAN returns.
  - Making the off-LAN side (ferryvps + phones) live on its own would need it to hold a majority. Then the LAN side alone could never certify, which is worse day to day.

## 3. Liveness math

Model: each device is up independently. LAN servers 0.99, ferryvps 0.995, phones 0.5 / 0.7 / 0.9 (doze and cellular). Correlated LAN outages are listed separately.

| Voter set | Quorum | P(live), phones 0.5 | phones 0.7 | phones 0.9 | During a LAN outage |
|---|---|---|---|---|---|
| Today: 3 LAN servers | 2 | 0.9997 | 0.9997 | 0.9997 | stall (recovers after) |
| + ferryvps (4) | 3 | 0.9996 | 0.9996 | 0.9996 | stall |
| All 6, fixed set | 4 | 0.9911 | 0.9967 | 0.9996 | stall (ferryvps + 2 phones = 3 < 4) |
| All 6, active-set eligibility (sec. 2) | > \|E\|/2 | ≈0.9996 (offline phones drop out of E) | ≈0.9996 | ≈0.9996 | stall unless E is mostly off-LAN |
| 3-committee, 2-of-3, rotating (sec. 1) | 2 of 3 | ≈1.0, eventually | ≈1.0 | ≈1.0 | **live, but unsafe**: both sides can certify |

**Answers to the questions**
- **Does a fixed set of unreliable phone voters hurt liveness?** Yes. It raises the quorum, and offline phones then count against it: from 0.9997 down to 0.9911 with phones at 50%. Activity-based eligibility removes that harm.
- **Does it help in a LAN outage?** No, not without letting the LAN side lose its own liveness. The real protection against LAN outages is the catch-up fix: the outage costs minutes, not a permanent stall.
- **Weighted votes:** give the off-LAN side enough weight to stay live in a LAN outage, and the LAN side can't certify alone. When phones doze, ferryvps going down then also stalls. So weighting moves the stall around without removing it.

## 4. Gates and tests (before any build)
- **Gates:**
  - `VOTER_IDS_EPOCH`: device voter ids `owner#tag` and a clock key per device.
  - `ACTIVE_SET_EPOCH`: E(T) from certified data, quorum > |E|/2.
  - `COMMITTEE_EPOCH`: proposer lottery and the light vote mode for phones.
- **Lab tests:**
  - phones offline: they leave E after N epochs and liveness holds;
  - LAN outage: stall, then catch-up after it;
  - a selected proposer drops: the next round's proposer takes over;
  - restart mid-round: restored votes keep their lock;
  - partition with each side holding half or less of E: no side certifies;
  - **fork test:** with disjoint committees and delayed delivery, assert that no two certificates exist for one epoch. This test must fail for the original 2-of-3-committee rule and pass for section 2.
- **Prerequisites:** ferryvps needs about 2 GB more RAM, or n8n/java moved off. Today it has 1 vCPU, 2 GB RAM, about 390 MB available and swap 675/953 MB used.

## 5. Recommendation
1. Build section 2, not the 2-of-3-committee quorum.
2. Order: device voter ids, then ferryvps as the 4th voter (quorum 3), then active-set eligibility, then the proposer lottery and the phone light mode.
3. Phones become voters only after the light mode has soaked in the lab.

