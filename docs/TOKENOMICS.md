# HONE Tokenomics

Author: Shin Devlin
Last updated: 2026-04-10 (v2.10.0 baseline, v2.10.1+ decisions included)
Status: canonical

This document is the source of truth for HONE's economic model. Code should reference constants defined here. Marketing should quote positioning from here. Changes to this document require a version bump and a clear reason.

---

## 1. One-paragraph summary

HONE is a sovereign blockchain where value is captured from verifiable economic activity — inference, storage, compute hosting, commerce, sensor data — and recycled back to the people doing real work. Fixed supply of 42,000,000 HONE. **No burn, ever.** Every token that leaves circulation via fees or slashing flows to a recycle wallet that is steadily drained back into block rewards over time. One token, many capabilities, perpetual circulation. Earned by work, not destroyed by code.

---

## 2. The positioning: "No Burn, All Recycle"

> *"Bitcoin is the 'digital gold' chain. Ethereum is the 'burn fees for scarcity' chain. HONE is the 'no burn, all recycle' chain — 42 million tokens, forever, in perpetual circulation, earned by doing real work."*

HONE does not destroy tokens as a monetary policy tool. Every fee, every slashed bond, every unclaimed escrow flows to the `hone_recycle` system account, and from there back into the active block reward pool. This is the central economic commitment of the project and it cannot be changed without a hard fork.

### Why not burn?

Burn is economically lazy and ethically dubious. Burning fees:

1. **Transfers wealth invisibly to existing holders.** Every destroyed token makes every remaining token marginally more valuable. Late participants pay more for the same network access. It's a wealth transfer disguised as a fee.
2. **Destroys economic work.** The fee was paid by someone doing something productive. Destroying it is literally throwing value in a fire.
3. **Is a lazy answer to the "what do we do with the counter-inflation pressure" question.** Burn is what projects do when they haven't thought hard about velocity and sustainability.

### Why recycle?

Recycling fees:

1. **Maintains velocity.** Tokens that flow into recycle flow back out via block rewards. No idle reserves, no dead capital.
2. **Is honest.** Fees pay for work. Recycled fees pay for the next round of work. The money is in motion, not destroyed.
3. **Keeps 42M real, not theoretical.** The cap is a live, circulating number — not "42M minus whatever we torched last quarter."
4. **Rewards participants, not speculators.** New miners earn the same tokens old miners earned. Nobody gets a scarcity premium from network destruction.

### How the recycle wallet works

- **Address**: `hone_recycle` (system account, already exists in `stateStore._isSystemAccount`)
- **Multi-token**: accepts HONE, wrapped stables, arbitrary TOKEN_CREATE tokens — anything paid as a fee flows here
- **Drain rule**: at each epoch, if `recycle.balance > next_block_reward`, a portion of the next block reward is paid *from* the recycle wallet instead of being emitted fresh. This reduces fresh emission pressure proportional to recycle flow.
- **Emission precedence**: fresh emission runs first (from the fixed schedule). Recycle drain is supplemental, on top of scheduled emission, up to a cap of 100% of the block reward.
- **Cross-token recycling**: recycled non-HONE tokens (wrapped stables, project tokens, etc.) are held in the recycle wallet and gradually redistributed via special entries. Their eventual path back to circulation is configurable per token but always on chain.
- **Smart-contract-paid tokens**: if a smart contract pays into the recycle wallet, those tokens stay recoverable — any chain participant can eventually earn them back via block rewards, dispute resolution payouts, or bounty claims. Nothing is ever permanently locked.

### The NO BURN commitment

This is a hard architectural promise: **HONE will never implement a burn mechanism, ever, under any circumstances.** Any proposal to add burning to HONE is a proposal to fork HONE into a different chain. The No Burn commitment is as foundational as the 42M supply cap.

## 2.5 The conditional payout rule

**HONE cannot make fixed-amount promises it might not be able to cash.** Any HONE-denominated reward in the protocol must be one of:

**(a)** a **percentage of a specific known revenue stream** (e.g., "10% of this order's platform fee")

**(b)** a **fraction of a measurable pool at the moment of payout** (e.g., "0.1% of current recycle wallet balance")

**(c)** a **share of block rewards within a defined role allocation** (e.g., "pro-rata within the storage_hosts 10% of block rewards")

Fixed-amount promises ("you earn 50 HONE for X") are **prohibited in the protocol layer**. They can only exist in the application layer where the funding source is explicitly known and bounded. This rule protects the chain from over-committing and keeps all economics honest about current state, not aspirational future state.

The emission schedule is exempt from this rule because it has a known funded source: the 42M supply cap. Emission is a fixed schedule against a bounded reserve, not an open-ended promise.

---

## 3. Supply

### Total supply: 42,000,000 HONE

- **Fixed forever.** No inflation beyond the emission schedule. No governance-adjustable issuance.
- **No pre-mine.** Genesis block held zero tokens allocated to founders; the project started at zero and earns forward.
- **No founder allocation, no VC allocation, no pre-launch distribution.** Every token in existence has been (or will be) earned through work on the live chain.

### Denomination

- **1 HONE = 10,000,000,000 dreams** (10^10, higher precision than Bitcoin's 10^8 satoshis)
- Dreams are the base unit for internal calculations; HONE is the display unit.
- All on-chain amounts are rounded to 10 decimal places via `stateStore._round()`.
- **All tokens on HONE — native and user-created — use 10 decimals.** See Section 15 (Token standards) for the full rule.
- 10 decimals future-proofs the chain: even at $100,000 per HONE, the smallest unit (1 dream) is worth $0.00001, still fine for sub-penny microtransactions in commerce and sensor markets.

### Emission schedule

- **Epoch duration**: 5 minutes
- **Epochs per day**: 288
- **Epochs per year**: ~105,120
- **Genesis reward**: 243.06 HONE/epoch (year 1 baseline)
- **Halving schedule**: reward roughly halves every 4 years, following the existing `src/services/emissionSchedule.js`
- **Total emission timeline**: ~30 years to exhaust the 42M cap
- **Post-emission**: rewards come entirely from the recycle wallet + fee market. Since recycling maintains velocity, block rewards remain meaningful long after emission ends.

### Cumulative emission by year (approximate)

| Year | Reward/epoch | Yearly emission | Cumulative |
|---|---|---|---|
| 1 | 243.06 HONE | 25,560,000 | 25,560,000 |
| 2 | 121.53 | 12,780,000 | 38,340,000 |
| 3 | 60.77 | 6,390,000 | 44,730,000 *(capped at 42M)* |

Emission halts when cumulative reaches 42M. After that, block rewards come from the recycle wallet + fee flows only.

---

## 4. Block rewards: how they flow to people doing work

HONE has role-based reward splits. The split evolves as new capabilities launch:

### Current split (v2.10 and earlier)

```
85% → miners        (run inference via Ollama, generate useful work)
10% → verifiers     (validate other miners' work via quorum panels)
 5% → clocks        (maintain chain heartbeat, coordinate block proposals)
```

### Proposed split from v2.11 onward (when HONE-FS launches)

```
75% → miners
10% → verifiers
 5% → clocks
10% → storage_hosts (serve HONE-FS blobs, respond to challenges)
```

The 10-point shift from miners to storage_hosts reflects the reality that storage is the bootstrap role for the entire commerce + compute stack. Without storage hosts, v2.11-v2.14 do not work. Without HONE-FS, there is no HONE commerce at scale. Storage is a peer role, not a second-class one.

### Proposed split from v2.13 onward (when SERVICE_DEPLOY launches)

```
65% → miners
10% → verifiers
 5% → clocks
10% → storage_hosts
10% → service_hosts  (run stateless compute workloads)
```

### Proposed split from v2.15 onward (when LoRa sensor mesh launches)

```
60% → miners
10% → verifiers
 5% → clocks
10% → storage_hosts
10% → service_hosts
 5% → sensor_bridges (run LoRa gateways, relay sensor feeds)
```

### Why these splits and not others

- **Miners stay majority** through v2.15 because inference is still the largest economic activity and the most capital-intensive (GPU hardware).
- **Verifiers hold at 10%** because the verifier role scales with all other roles (more storage = more challenges; more compute = more audits; more inference = more validation). Verifiers do not specialize; they check everything.
- **Clocks hold at 5%** because clock duty is low-compute, low-cost, and high-leverage — you don't need many clocks, and each one does little work, but their coordination is critical. A small share fairly compensates this.
- **Storage hosts get 10%** at launch because:
  - Storage is the bootstrap for all downstream capabilities
  - Capital cost (disk + bandwidth) is real and ongoing
  - Helium miner reuse needs a meaningful incentive to overcome switching cost
  - Aligning with verifier share (10%) signals parity
- **Service hosts get 10%** by the same reasoning — capital-intensive, bootstraps downstream value
- **Sensor bridges get 5%** because the role is low-cost (you already own a LoRa gateway) and high-volume (many sensors per gateway). Smaller per-bridge share, but more bridges.

### Within each role, reward distribution

Each role's allocation is distributed among active participants according to work done:

- **Miners**: pro-rata by work_value (tokens × model parameter billions), honest commitments only
- **Verifiers**: pro-rata by verification count, matching consensus
- **Clocks**: equal split among active clocks with heartbeats in the last 3 epochs
- **Storage hosts** (v2.11+): pro-rata by `committed_gb × rep_score × uptime_factor + bandwidth_served_gb × rate`
- **Service hosts** (v2.13+): pro-rata by `active_session_count × uptime_factor × rep_score`
- **Sensor bridges** (v2.15+): pro-rata by `valid_readings_relayed × rep_score`

Reputation gating is real: a role participant below a minimum reputation threshold earns zero for that epoch. This prevents sybil attacks where thousands of low-quality nodes dilute honest participants.

---

## 5. Fees: how value flows into the recycle pool

Every capability has its own fee structure. All fees flow to `hone_recycle` or directly to work-performing nodes (never burned). Fee percentages are conservative by design — we win on volume and alignment, not on taking a large cut of each transaction.

### Commerce fees (v2.10+)

#### Store opening via bonding curve
- Paid in wrapped stables (wUSDC/wUSDT/wDAI)
- Formula: `cost = 1.00 + 0.05 × capacity_slots` (linear bonding curve, defined in `src/services/stakeBondingCurve.js`)
- **Split**:
  - 50% → `hone_recycle` (recycled back to block rewards)
  - 50% → protocol treasury (funds development, audits, bounties — managed by genesis operator initially, transitions to stake-weighted governance later)
- Stake: 1 HONE collateral locked per capacity slot (refundable on store close minus any slashing)

#### Platform fee on orders (v2.10.1+)
- 1% of order total, deducted from seller payout
- **Split**:
  - 0.5% → `hone_recycle`
  - 0.4% → store stakers (pro-rata by stake — includes the seller's own stake plus any delegated stakes)
  - 0.1% → reputation bonus pool (paid to buyers whose reviews match final dispute outcomes)
- Deliberately undercuts Stripe (2.9% + $0.30), Shopify (2%), Amazon (8-15%)

#### Token creation fees (v2.10, existing)
- Micro (≤1M supply): 21 HONE
- Standard (≤42M): 42 HONE
- Mega (≤1B): 84 HONE
- Custom (unbounded): 168 HONE
- **Split**: 100% → `hone_recycle`

#### NFT collection creation fee (v2.10, existing)
- 10 HONE per collection
- **Split**: 100% → `hone_recycle`

#### Project registration fees (v2.10, existing — for API integrations)
- Current: free (covered by inference-consumption fees)
- Future consideration: small registration fee to discourage squatting, split 100% → recycle

### Storage fees (v2.11+)

#### Blob storage commitment
- Seller pays `payment_hone` per BLOB_STORE_COMMIT to storage hosts over the commitment period
- **Split**:
  - 90% → storage hosts (pro-rata by commitment share, released over time via escrow)
  - 9% → `hone_recycle`
  - 1% → reputation bonus pool (paid to hosts with highest challenge success rate)

#### Bandwidth fees (per-GB served)
- Seller pays per BLOB_SERVE_PROOF recording actual bytes served
- **Split**:
  - 95% → the serving host (direct payment)
  - 5% → `hone_recycle`

#### Slashing on failed challenges
- Storage host fails a verifier spot-check → their stake is slashed
- **Split**: 
  - 50% → honest hosts who passed the same challenge (redistributes the failure)
  - 50% → `hone_recycle`

### Compute fees (v2.13+)

#### Session payment
- Buyer pays per session, released pro-rata over session duration
- **Split**:
  - 90% → service host
  - 9% → `hone_recycle`
  - 1% → reputation bonus pool

#### Slashing on uptime failure
- Host fails heartbeat or challenge during an active session
- Session escrow fully refunded to buyer; host stake slashed proportional to session remaining
- **Split**:
  - 50% → refund to session buyer
  - 50% → `hone_recycle`

### Sensor fees (v2.15+)

#### Sensor subscription
- Subscriber locks escrow for monthly subscription
- Released pro-rata as valid readings arrive
- **Split**:
  - 70% → gateway operator (LoRa bridge)
  - 25% → sensor owner (registered SENSOR_REGISTER)
  - 5% → `hone_recycle`

#### Sensor registration fee
- 0.1 HONE per sensor
- **Split**: 100% → `hone_recycle`

### Universal fee routing principle

Every fee, across every capability, follows this pattern:

1. The work-doer gets the largest share (typically 70-95%)
2. `hone_recycle` gets the next-largest share (typically 5-10%)
3. Reputation bonus pool gets a small residual (typically 1%)

**Nothing is ever burned.** Every token is recoverable somehow, either immediately (to the work-doer) or eventually (via block rewards drawn from recycle).

---

## 6. Stake requirements by capability

Stakes are collateral — locked HONE that is refundable on clean exit and slashable on failure. They are NOT consumed. This creates persistent demand that scales with network participation without requiring perpetual fee extraction.

| Capability | Stake formula | Typical value | Slashing conditions |
|---|---|---|---|
| Mining node | Fixed | 100 HONE | Forged inference results, failed verifier challenges |
| Verifier | Fixed | 100 HONE | Voting against consensus, idle verifier panels |
| Clock node | Fixed | 10 HONE | Missing heartbeats for extended periods |
| Store operator | Bonding curve | 1 HONE / product slot | Fraud, non-fulfillment, failed dispute resolution |
| Storage host (v2.11) | Proportional | 1 HONE / GB committed | Failed blob challenges, dropped blobs |
| Service host (v2.13) | Proportional | 10 HONE / CPU-epoch committed | Uptime failure, failed challenges, bad state commits |
| Sensor bridge (v2.15) | Fixed | 1 HONE / gateway | Relaying fraudulent readings, dropped packets |
| Sensor registration (v2.15) | Fixed | 0.1 HONE / sensor | Malformed data, fraudulent readings |

### Stake can be cross-allocated

A single account can stake once and allocate that stake across multiple capabilities. `shindevlin` with a 1,000 HONE stake pool might allocate:

```
Mining:       100 HONE (one mining node)
Verifier:     100 HONE
Store:         50 HONE (50 product slots)
Storage:      500 HONE (500 GB committed)
Service:      200 HONE (20 CPU-epochs)
Sensor bridge: 10 HONE
                = 960 HONE allocated
Free:          40 HONE (can allocate to new capabilities)
```

Slashing one capability does NOT automatically slash the others — each capability has its own slashable portion. But a sufficiently severe offense can cascade (e.g., confirmed identity fraud affects all roles of that account).

### Unbonding period

All stakes have a 14-epoch unbonding period (roughly 70 minutes) after requesting withdrawal. This gives verifiers time to catch in-flight disputes and ensures the operator cannot exit right before a fraud becomes visible. The 14-epoch window is the same across all capabilities.

---

## 7. The Helium miner reuse story (v2.15 hardware angle)

HONE can absorb the dormant Helium miner fleet (400,000+ devices) as storage hosts + LoRa sensor bridges. This is a distribution opportunity, not a dependency — the project works without it, but it makes adoption dramatically cheaper and faster.

### What Helium operators earn on HONE

A repurposed Helium gateway running HONE-nano can simultaneously be:

1. **Clock node** (trivial, day one) — earns from the 5% clock reward pool
2. **Storage host** (v2.11) — earns from the 10% storage_hosts reward pool + direct blob storage/bandwidth fees
3. **Gateway operator** (v2.10.2) — earns HTTP relay tips
4. **Verifier** (existing) — earns from the 10% verifier reward pool
5. **Sensor bridge** (v2.15) — earns from the 5% sensor_bridges reward pool + direct sensor subscription fees

**Five concurrent income streams on hardware the owner already owns.** No new purchases required. This is the distribution moat that gets Helium operators excited.

### Economic example (conservative numbers)

Assume block reward averages 121 HONE/epoch (year 2 baseline), 288 epochs/day, HONE priced at $1:

```
Storage hosts share (10%):    12.1 HONE/epoch = 3,484/day total pool
Suppose 1,000 active storage hosts sharing equally: 3.48 HONE/day per host = $3.48/day
Typical Helium gateway power draw: ~10W = ~7.2 kWh/month at $0.15/kWh = $1.08/month
Net profit at $1 HONE: ~$104/month per gateway

At 10,000 active storage hosts: ~$10.50/month per gateway (still profitable)
At 100,000 active storage hosts: ~$1.05/month per gateway (still covers electricity)
```

These numbers scale naturally — more hosts reduce per-host earnings but also prove the network is working, which drives HONE price, which increases real-dollar earnings.

### Why this doesn't require sub-tokens (locked decision)

Every sub-token would:
1. Fragment liquidity
2. Confuse users ("which token do I hold?")
3. Repeat Helium's exact failure (HNT/MOBILE/IOT split was widely criticized and contributed to HNT's collapse)
4. Break the unified stake + escrow + reputation model

**The Helium community is specifically traumatized by sub-tokens.** Offering them a native-HONE-only story is a feature, not a limitation. It's a differentiator.

---

## 8. The protocol treasury

A small portion of certain fees flows to a "protocol treasury" account (currently managed by the genesis operator, transitioning to governance in a later phase). This funds:

1. **Development** — paying contributors, auditors, technical writers
2. **Security audits** — periodic third-party reviews of chain code, consensus, cryptography
3. **Bug bounties** — paying out for responsibly disclosed vulnerabilities
4. **Infrastructure** — running reference gateways, documentation sites, test networks
5. **Community outreach** — conference sponsorships, hackathons, educational content

### Treasury funding sources

- 50% of commerce bonding curve stake purchases (USD-denominated, paid in wrapped stables)
- 0% of emission (the treasury does NOT receive mining rewards — it earns only from fees)
- 0% of slashing (slashing flows to honest participants + recycle, not treasury)

### Treasury accountability

The treasury address is public (`hone_treasury`). All inflows and outflows are visible on chain. Spending decisions during genesis phase are made by the genesis operator (shindevlin); a transition to governance-weighted spending is planned for a later phase, with specific governance mechanism TBD.

The treasury is NOT a monetary policy tool. It does not affect token supply, emission, or the recycle mechanism. It is simply a publicly-visible fund used for ecosystem development.

---

## 9. Reputation and its economic weight

Reputation is tracked per-capability axis (see `stateStore.reputation` Map in v2.10.0). It affects earning directly in the following ways:

### Reputation-gated earning

- Nodes with `rep_score < 0` earn zero from their capability's reward pool for that epoch
- Nodes with `rep_score >= 0` earn proportional to their work × (1 + rep_score/100), capped at 2x multiplier
- A node with rep_score of 100 earns 2x what an identical node with rep_score of 0 earns

### Reputation decay

Reputation naturally decays over time if a node stops participating:

- No activity for 1000 epochs (3.5 days) → 10% decay per 1000 idle epochs thereafter
- Keeps the signal fresh — old reputation doesn't protect against current bad behavior

### Reputation is stake-weighted on voting

When voters cast `REPUTATION_VOTE` entries, their vote weight is:

```
weight = min(100, 1 + sqrt(voter_stake_hone))
```

- A voter with 0 stake has weight 1
- A voter with 100 HONE stake has weight 11
- A voter with 10,000 HONE stake has weight 101 (capped at 100)
- Diminishing returns prevents whale dominance

### Reputation cannot be bought

- You cannot transfer reputation between accounts
- You cannot buy reputation from a friend
- You cannot split your account into ten shards to multiply your vote weight (each needs its own stake)
- You CAN delegate reputation voting power through explicit `REPUTATION_DELEGATE` entries (planned for a later phase)

---

## 10. Reward calculation per epoch (pseudocode)

```
total_reward = emission_schedule(epoch) + recycle_drain(epoch)

// where recycle_drain is capped so that total_reward never exceeds 2x emission_schedule
recycle_drain = min(
  recycle_wallet.balance(HONE),
  emission_schedule(epoch)
)

// split by role
miner_pool        = total_reward * 0.75   // v2.11+
verifier_pool     = total_reward * 0.10
clock_pool        = total_reward * 0.05
storage_host_pool = total_reward * 0.10

// distribute within each pool
for role in [miners, verifiers, clocks, storage_hosts]:
  for node in active_in_role(epoch):
    if node.reputation < 0: skip(node)
    node_reward = role_pool * (
      node.work_contribution / total_work_in_role
    ) * (1 + node.reputation / 100)

// actually pay out
for node in all_rewarded_nodes:
  if node_reward > 0:
    recordMiningReward(node, node_reward, epoch)
    // the MINING_REWARD entry internally debits fresh_emission first,
    // then recycle_wallet for the remainder
```

---

## 11. Summary of constants for code reference

```
// src/services/emissionSchedule.js
TOTAL_SUPPLY_HONE = 42_000_000
HUNITS_PER_HONE = 100_000_000
EPOCH_DURATION_MS = 5 * 60 * 1000
GENESIS_REWARD_HONE_PER_EPOCH = 243.06

// src/services/rewardSplits.js (new)
REWARD_SPLIT_v2_10 = {
  miners: 0.85,
  verifiers: 0.10,
  clocks: 0.05
}
REWARD_SPLIT_v2_11 = {
  miners: 0.75,
  verifiers: 0.10,
  clocks: 0.05,
  storage_hosts: 0.10
}
REWARD_SPLIT_v2_13 = {
  miners: 0.65,
  verifiers: 0.10,
  clocks: 0.05,
  storage_hosts: 0.10,
  service_hosts: 0.10
}
REWARD_SPLIT_v2_15 = {
  miners: 0.60,
  verifiers: 0.10,
  clocks: 0.05,
  storage_hosts: 0.10,
  service_hosts: 0.10,
  sensor_bridges: 0.05
}

// src/services/feeSchedule.js (new)
COMMERCE_PLATFORM_FEE_PCT = 0.01
COMMERCE_PLATFORM_FEE_SPLIT = {
  recycle: 0.50,
  stakers: 0.40,
  reputation_pool: 0.10
}
BONDING_CURVE_STAKE_SPLIT = {
  recycle: 0.50,
  treasury: 0.50
}
STORAGE_FEE_SPLIT = {
  host: 0.90,
  recycle: 0.09,
  reputation_pool: 0.01
}
STORAGE_BANDWIDTH_SPLIT = {
  host: 0.95,
  recycle: 0.05
}
SERVICE_FEE_SPLIT = {
  host: 0.90,
  recycle: 0.09,
  reputation_pool: 0.01
}
SENSOR_FEE_SPLIT = {
  gateway: 0.70,
  sensor_owner: 0.25,
  recycle: 0.05
}
SLASHING_SPLIT = {
  refund_or_honest: 0.50,
  recycle: 0.50
}

// src/services/stakeRequirements.js (new)
STAKE_MINING = 100
STAKE_VERIFIER = 100
STAKE_CLOCK = 10
STAKE_STORE_PER_SLOT = 1
STAKE_STORAGE_PER_GB = 1
STAKE_SERVICE_PER_CPU_EPOCH = 10
STAKE_SENSOR_BRIDGE = 1
STAKE_SENSOR_REGISTRATION = 0.1
UNBONDING_EPOCHS = 14

// src/services/reputation.js (new)
REP_MIN_EARNING_FLOOR = 0
REP_MAX_MULTIPLIER = 2.0
REP_DECAY_IDLE_EPOCHS = 1000
REP_DECAY_PCT_PER_1000_EPOCHS = 0.10
REP_VOTE_WEIGHT_CAP = 100
```

---

## 12. What is explicitly NOT in scope

To keep the tokenomics focused, these ideas are explicitly OUT of scope and will NOT be built without a significant change in project direction:

1. **Burn mechanism** — locked out architecturally. No burns, ever.
2. **Founder / team / VC allocation** — there are none and will never be any. 100% of tokens are earned through work.
3. **Pre-sale or ICO** — never happened, never will.
4. **Reserve fund / insurance fund / safety module** — the recycle wallet serves this function. Slashed tokens don't go to a separate insurance pool; they go to honest participants + recycle.
5. **Governance token** — HONE is both the utility token and (eventually) the governance token. Stake-weighted voting when governance launches.
6. **Stablecoin issuance** — HONE does not issue its own stablecoin. Wrapped stables (wUSDC, wUSDT, wDAI) handle stable-denominated transactions.
7. **Lending / staking-as-a-service / DeFi primitives** — can be built ON HONE by third parties, but not part of the core protocol.
8. **Sub-tokens for specific capabilities** — one chain, one token. See Section 7 for the specific reasoning.
9. **Adjustable supply** — the 42M cap is hard. Governance can change fee splits, role allocations, and bonding curve parameters, but cannot increase the supply cap.

---

## 13. Change log

- 2026-04-10 (v2.10.0 baseline): initial tokenomics doc
  - Established No Burn, All Recycle positioning
  - Defined platform fee split for commerce (1% on orders)
  - Added storage_hosts role to v2.11+ block reward split
  - Added service_hosts role to v2.13+ block reward split
  - Added sensor_bridges role to v2.15+ block reward split
  - Locked the 42M supply cap
  - Confirmed native HONE only — no sub-tokens for any capability track
  - Documented stake requirements for all capabilities
  - Established treasury funding model (50% of bonding curve stake purchases)

---

## 14. Location verification and trust levels

Location claims on HONE (for Area Pioneer NFTs, `SERVICE_AREA_REQUEST` bids, RF witness attestations, sensor bridge earnings) are verified through a **six-layer defensive stack** that combines multiple imperfect signals. No single layer is sufficient alone; the combination makes fraud economically irrational.

### The six layers

**Layer 1 — Time-of-flight (ToF) challenges.** The chain issues random challenges: *"Gateway A, transmit the hash of block B at epoch T. Gateway B, record exact receive time."* ToF (receive - transmit time) is measured with ~10 microsecond precision (achievable via NTP+PPS), giving distance upper bounds of ~3 km. This replaces RSSI as the primary distance signal — because ToF is independent of antenna gain, directional-antenna attacks (Helium's #1 fraud vector) stop working.

**Layer 2 — Multi-witness consensus with stake-weighted attestation.** A gateway's claimed location is only "verified" after N independent witnesses (different accounts, different funding sources, different uptime patterns, geographically distributed) co-sign attestations over M epochs. Each witness stakes HONE proportional to the value being claimed. If the location is later proven fraudulent, all co-signing witnesses lose their stake. This breaks single-operator sybil attacks.

**Layer 3 — Progressive trust with delayed rewards.** New gateways start at `verification_level: 0` and earn zero from block reward pools until they accumulate enough evidence. Verification is achieved by ToF + witnesses + 50+ epochs continuous operation + behavioral consistency. Probationary period is ~8 hours before a gateway can claim high-value privileges. This specifically kills the Area Pioneer NFT fraud vector — you can't mint a Pioneer claim on day one.

**Layer 4 — Verification rigor scales with stakes.** Low-value actions (relaying a single sensor reading) get lightweight checks. High-value actions (Area Pioneer claim, oracle feed registration) require full verification. The dispatcher enforces a `min_verification_level` per entry type:

```
verification_level_required = {
  "RELAY_SENSOR_READING":          0,
  "PRODUCE_BLOB_SERVE_PROOF":      1,  // ToF required
  "BID_ON_SERVICE_AREA_REQUEST":   2,  // ToF + witnesses
  "CLAIM_AREA_PIONEER_NFT":        3,  // ToF + witnesses + behavioral + 100 epoch history
  "OPERATE_ORACLE_FEED":           4,  // full verification + stake-scaled
  "RUN_VERIFIER_NODE":             4,
  "REGISTER_HIGH_VALUE_STORAGE":   3
}
```

**Layer 5 — Behavioral fingerprinting.** Does the gateway's uptime pattern match its claimed timezone? Does network latency to reference nodes match the claimed region? Does the RPi's internal temperature correlate with known weather in the claimed location? Each signal is weak individually, but combined into a `locationConfidence` score from 0.0 to 1.0. Low confidence triggers additional verification challenges.

**Layer 6 — Total fraud cascade.** If a single gateway is proven fraudulent, **ALL gateways from the same operator account are slashed**, not just the one caught. Slashing includes:
- Forfeiture of accumulated rewards from the fraudulent period (clawback from current balance)
- Reputation set to -100 for the operator's account and all its gateways
- All associated earning suspended while reputation is negative
- Pioneer NFTs on that account become "tarnished" (grey marker on map, no associated benefits)

Critically: **soulbound NFTs including Area Pioneer NFTs are NOT revoked.** They are historical records of fact. The NFT persists; the benefits associated with it (reputation multiplier, bidding priority, recycle drip) are zeroed while reputation is negative and restored when reputation rebuilds.

### Reputation rebuild after fraud

Reputation is recoverable through honest work. This is intentional — permanent reputation death discourages fraudsters from ever contributing honestly again, which is worse for the network than a slow rebuild.

**Mechanics**:

- **Every verified work event adds `+0.1` to reputation** (successful inference, passed blob challenge, delivered order, validated sensor reading, passed ToF challenge)
- **Rate cap: `+5 per epoch`** — prevents farming recovery via high-volume fake work
- **Natural decay: `-0.01 per epoch` if idle** — reputation still erodes without ongoing work
- **Earning floor at `rep_score >= 0`**: nodes with negative reputation earn zero from block reward pools, zero from Pioneer drip, zero from reputation-weighted multipliers. They can still earn from direct consumer payments (specific consumers who choose to transact with them).
- **Rebuild path**: from -100 to 0 requires 1,000 verified work events. At max rate (+5/epoch) that's 200 epochs (~16 hours) minimum, but realistically weeks to months of continuous honest work given normal work volumes.
- **Rebuild to pre-fraud state** may take 6+ months of continuous honest operation.

### Why rebuild is philosophically correct

- **Slow enough to be genuinely painful** — fraudsters lose months of earnings and have to grind back from zero
- **Fast enough for redemption to be real** — no lifetime ban
- **Tied to real work, not time passing** — you can't just wait out the punishment
- **Fresh accounts still better for clean restart** — starting over is always strictly better than grinding from -100, unless you have attached benefits (valuable Area Pioneer NFTs, customer relationships) worth preserving
- **Clawback is permanent** — rebuilt reputation restores future earning capacity, but stolen earnings from the fraud period are gone forever (distributed to honest participants and recycle wallet)

### What location verification can and cannot promise

**Can**:
- Reject most fraud patterns at low cost
- Make high-value fraud attempts expensive enough to be irrational
- Provide graceful degradation (some fraud may succeed but cost more than the reward)
- Retroactively punish fraud discovered after the fact (via total cascade)
- Scale verification effort with economic stakes

**Cannot**:
- Perfectly verify location in a trustless system (physics and hardware diversity make this intractable)
- Prevent state-level actors with deep resources from occasional fraud
- Eliminate false positives entirely (honest operators may occasionally need to re-verify)
- Provide instant verification (inherent delay between joining and full trust)

This honesty matters. HONE does not claim to have solved decentralized location verification — it claims to have made fraud **economically irrational for realistic attacker profiles**, which is a weaker but achievable guarantee.

---

## 15. Token standards — the 42M chain, for every token

**Every token on HONE has the same supply cap and precision.**

### The standard (locked in v2.10.1)

- **Supply**: exactly 42,000,000 (the chain's namesake number)
- **Decimals**: exactly 10 (same as HONE, future-proof precision)
- **Applies to**: all fungible user-created tokens via `TOKEN_CREATE`
- **Exception**: NFT collections (where "supply" is a max-mint counter and "decimals" is always 0 for indivisible tokens)

### Why

1. **Brand coherence.** "The 42M chain" becomes literal. Every asset on HONE is bounded by the same number. One mental model, applied universally.
2. **Anti-spam / anti-memecoin.** Fixed 42M prevents the "1 quadrillion supply with X% token burn" meme-hype pattern. Every token must earn its value from utility, not from fake scarcity games.
3. **UX simplification.** A balance of "1,000 / 42,000,000" is interpretable across every token on the chain. No one has to wonder "is 1M TOKEN a lot or nothing?" — the answer is always "compared to 42M."
4. **Aligned with No Burn, All Recycle.** Both HONE and every user token are fixed-supply, non-inflatable assets. Consistent monetary philosophy across the entire chain. No "well this memecoin has a 5% burn but the native token doesn't" weirdness.
5. **Prevents token inflation gaming.** Creators cannot mint arbitrary amounts after launch. Fixed 42M at creation = no post-launch supply surprises.

### Enforcement

The token standard is enforced at two levels:

1. **`src/services/ledger.js:recordTokenCreate`** — throws an error if `supply !== 42000000` or `decimals !== 10` (for fungible tokens). Gives clear error messages to callers.
2. **`src/chain/stateStore.js:applyEntry TOKEN_CREATE`** — silently drops entries with invalid supply/decimals (chain-level invariant, defense in depth).

### Fee structure

A single flat fee replaces the old tiered system:

- **Fungible token creation**: 42 HONE (matching the supply for memorability)
- **NFT collection creation**: 10 HONE
- **Both fees**: 100% routed to `hone_recycle` — not burned, not sent to treasury, recycled back to block rewards over time.

### Existing tokens

Tokens created before v2.10.1 under the old tiered system remain valid — the chain doesn't retroactively invalidate history. But all new `TOKEN_CREATE` entries from v2.10.1 forward must meet the standard. This is similar to the "old Area Pioneer NFTs stay valid" rule — history is immutable.

### NFTs

NFT collections are exempt because:
- Their "supply" field is a max-mint counter (e.g., 10,000 for a collection of 10,000 unique NFTs), not a fungible amount
- Their "decimals" field is always 0 (NFTs are indivisible by definition)
- Enforcing 42M/10 decimals on NFT collections would break the primitive

The 42M/10 rule applies only to fungible `TOKEN_CREATE` with `type: "fungible"`. NFT `TOKEN_CREATE` with `type: "nft"` uses the existing NFT collection mechanics.

---

## 16. Area Pioneer NFTs — the full four-layer structure (v2.15)

Area Pioneer NFTs reward operators who deploy LoRa gateways in previously-unserved regions. The rewards are structured in four layers, each of which respects the Conditional Payout Rule (no fixed HONE promises).

### Qualification

A gateway qualifies for an Area Pioneer NFT when:
1. It operates continuously from a claimed location for 30+ epochs
2. No other HONE gateway has operated within 10km of its location in the last 30 days
3. It passes `verification_level >= 3` (Layer 1 ToF + Layer 2 witnesses + Layer 5 behavioral)
4. Its reputation score is `>= 0`

### Layer 1 — Free benefits (always granted)

Zero-cost-to-protocol benefits granted at the moment of qualification:

- **Soulbound Area Pioneer NFT** — minted via `recordSoulboundMint`, permanent, non-transferable, **non-revokable** (survives any future fraud events on the same account)
- **Public map marker** — gold "Pioneer of [region]" marker on HONE coverage map
- **Geographic reputation boost** — `+10` to reputation score within the region's coordinates
- **First-right-of-refusal** — for 100 epochs (~8 hours) after any new `SERVICE_AREA_REQUEST` in the region, the Pioneer has bid priority if they submit a competitive bid

### Layer 2 — Activity-triggered fee share

When the **first** fee-bearing transaction happens in a Pioneered area, the Pioneer receives the protocol's share of that transaction instead of `hone_recycle`. Applies to: SERVICE_AREA_REQUEST settlement, BLOB_STORE_COMMIT payment, ORDER_PLACE escrow release, etc.

Under the standard storage fee split (90% host / 9% recycle / 1% reputation pool), the 9% that normally goes to recycle is routed to the Pioneer **for the first fee-bearing transaction only**. After that, normal splits apply forever.

**Properties**:
- Rule (a) compliance: percentage of specific known stream
- Zero cost to buyer (they pay the same)
- Zero cost to service provider (they earn the same)
- Zero cost to recycle wallet in steady state (only redirects the first transaction)
- Self-scales with first-transaction value
- Vanishes if no activity ever happens — no promise, no debt

### Layer 3 — Ongoing recycle drip

When the recycle wallet has HONE balance, Pioneers share in a tiny ongoing drip:

- At each epoch, **0.1% of current recycle wallet balance** is distributed pro-rata among active Pioneers (heartbeated in the last 100 epochs)
- Rule (b) compliance: fraction of measurable pool at moment of payout
- If recycle wallet is empty: distribution is zero
- If recycle wallet has 10,000 HONE and 100 active Pioneers: each gets 0.1 HONE/epoch ≈ 29/day
- If 10,000 active Pioneers: each gets 0.001/epoch — effectively nothing
- **Self-caps mathematically** — impossible to over-commit

### Layer 4 — Permanent reputation-weighted earning multiplier

The +10 reputation boost (from Layer 1) compounds through the standard earning formula:

```
node_reward = role_pool * (work_contribution / total_work) * (1 + rep_score / 100)
```

A Pioneer with `rep_score = 60` (base + Pioneer boost) earns 60% more per unit of work than an identical gateway with `rep_score = 0`. This is **the actual long-term value** of being a Pioneer — not a one-time bonus, but a permanent earning uplift that compounds over months and years as reputation accumulates.

### Fraud interaction with Pioneer NFT

If the Pioneer's account commits fraud and is caught:
- The NFT **persists** (soulbound, non-revokable, historical record of fact)
- The map marker **turns grey** ("Tarnished Pioneer")
- The +10 reputation boost is **zeroed** (rep goes to -100 immediately)
- The Layer 2 first-activity fee share is **consumed** if already triggered, or **forfeit** if not yet triggered
- The Layer 3 recycle drip is **suspended** while rep < 0
- The Layer 4 earning multiplier is **zero or negative** while rep < 0
- Reputation rebuild restores the marker to gold and the benefits incrementally

The NFT itself is a permanent badge of fact. The benefits are tied to reputation and are restorable through honest work.

---

## 17. Historical data ingestion (v2.15 Helium miner bonus)

Helium operators who upload their gateway's historical packet data (asserted location history, witness receipts, beacon reception logs, coverage records) from their Helium days receive:

### Free benefits (always granted)

- **Soulbound Data Contributor NFT** — permanent badge recognizing the historical contribution
- **Reputation boost** — `+20` to the `dataset_contributor` reputation axis
- **Map recognition** — contribution visible on profile and public map

### Conditional rewards (only when data proves valuable)

- **Dataset access royalty** — when subscribers pay to query historical HONE data that includes this operator's contributions, the operator receives a pro-rata share of the subscription fee based on how much of the query result came from their contributed data
- Rule (a) compliance: percentage of known revenue stream
- Pays only when real demand materializes; no fixed promise

### No fixed HONE payment for upload

Consistent with the Conditional Payout Rule. The upload itself creates no debt to the uploader — the value comes from ongoing royalties when the data is consumed, plus the permanent badge and reputation uplift.

### Validation

Uploaded data is validated via:
- Cross-reference with Helium public records (where available)
- Consistency checks against current LoRa physics (signal propagation, timing)
- Reputation stake: uploader stakes HONE on the validity of their upload, slashable if the data is proven fraudulent

---

## 18. Change log update (v2.10.1 decisions)

- Added Section 2.5: Conditional payout rule
- Updated Section 3: HONE is 10 decimals (unified with user tokens)
- Added Section 14: Location verification and trust levels (six-layer stack)
- Added Section 15: Token standards (42M/10 decimals for all user tokens)
- Added Section 16: Area Pioneer NFT four-layer reward structure
- Added Section 17: Historical data ingestion rules
- Locked fraud cascade mechanics: total slashing across all operator gateways, clawback, reputation to -100
- Locked reputation rebuild mechanics: +0.1 per verified work event, +5/epoch cap, -0.01/epoch idle decay, earning floor at rep >= 0
- Locked Area Pioneer NFT as soulbound and non-revokable even on fraud
- Token creation fees now route to `hone_recycle` (was `hone_treasury`)
- NFT collection fees now route to `hone_recycle` (was `hone_treasury`)
- Removed TOKEN_FEE_TIERS multi-tier system, replaced with flat 42 HONE fee for the single standard
