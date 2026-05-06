# Bitcoin Proof of Compute (BTCPC)

### A Proof-of-Compute Blockchain with Native Commerce, IoT, and Version Control Protocols

**Shin Devlin**
**Version 1.0 — April 2026**

---

## Abstract

Bitcoin Proof of Compute (BTCPC) is a blockchain where every token in existence was
earned by a machine doing useful work. Mining on BTCPC does not mean burning electricity
on a hash that disappears the moment it is found. It means running an AI inference job
that a user submitted, storing a file that someone committed, reporting a sensor reading
that a subscriber is paying for, or keeping the clock alive that the network depends on.
The computational output is the proof of work. The work had a customer.

Total supply is fixed at **42,000,000 BTCPC** with 10 decimal places:
**1 BTCPC = 10,000,000,000 dreams**. Epochs run every **30 seconds**, driven by
permissionless clock nodes that any machine on the network can operate. Emission
decays over epochs. There is no pre-mine. There is no founder allocation. Every token
in existence was earned.

Three protocols are native to the BTCPC chain from genesis block 0: **Freeport**
(a sovereign peer-to-peer marketplace with built-in escrow), **Verasens** (a decentralized
IoT sensor network with on-chain data provenance), and **LinkGit** (a decentralized version
control system that mirrors seamlessly to GitHub). Each protocol is a standalone business
with its own revenue model, owned initially by the protocol founder (shindevlin) and
independently transferable via on-chain key rotation.

---

## 1. The Chain

### 1.1 Proof of Compute

BTCPC replaces the arbitrary SHA-256 puzzle with a requirement that miners produce
output that someone requested. The principle is simple: **mining should produce output
that someone wanted.**

BTCPC uses a fully dynamic reward model. Every epoch, the chain measures actual proven
work across six categories: inference/compute, storage, sensors/IoT, verifiers,
services (decentralized compute), and clock nodes. The share each category receives
from the epoch reward is proportional to its actual utilization against a calibrated
target — not a fixed percentage. If the chain becomes a storage chain, storage earns
the majority. If inference dominates, inference earns the majority. The market decides.

**Active work categories:**

- **Inference/Compute:** Nodes completing AI inference jobs requested by users.
  Rewarded proportional to verified value score — output tokens multiplied by hardware
  tier weight, model weight, and complexity factor — as assessed by verifiers.

- **Storage:** Nodes storing committed chain data, proven via `StorageHeartbeat`.
  Rewarded proportional to bytes proven multiplied by query activity. Also earns
  contract fees when stored data is accessed.

- **Sensors/IoT:** Sensor nodes submitting verified readings. Baseline epoch reward
  for uptime; additional purchase premium when data is actually bought via
  `SensorDataPurchase`.

- **Verifiers:** Nodes that receive encrypted inference job payloads (prompt and
  result, encrypted to their memo key) and assess whether real work was done
  commensurate with the request. Earn only when verifications occur — zero
  verifications means zero verifier reward. Also earn from job escrow
  (`InferenceJobPay`).

- **Services:** Nodes hosting decentralized containerized services (equivalent to
  decentralized Docker/Kubernetes). Rewarded for active container uptime.

- **Clock:** Permissionless clock nodes that advance epoch consensus.

**Infrastructure base (fires every epoch, regardless of activity):**

- Clock nodes receive a tiny era-scaled reward per active clock node to keep
  consensus alive.
- Testnet operators receive a tiny era-scaled reward to keep the development
  network alive.
- A mandatory 2% reserve is withheld each epoch: 1.5% to the recycle fund,
  0.5% to the testnet fund, providing perpetual top-up for both.

**Human reviewers (dispute path only):** paid purely from job escrow, with no
allocation from the epoch pool.

A pool with no claimants does not accumulate — surplus recycles via the
`btcpc_recycle` system account. Tokens are never burned. The supply ceiling of
42,000,000 BTCPC is a hard limit, not a target that gets compressed by burning.

### 1.2 Supply and Emission

The total supply of BTCPC is fixed at **42,000,000 BTCPC**. This is a hard ceiling
with no exception. The smallest unit is one **dream**: 1 BTCPC = 10,000,000,000 dreams
(10^10, ten decimal places). All on-chain accounting is denominated in dreams; the
BTCPC display unit is a human convenience.

**Design intent:** BTCPC's new-supply emission is explicitly designed to end on the same day and at the same hour as Bitcoin's last mined satoshi. Both chains will exhaust their initial coin issuance simultaneously — two sovereign monetary networks converging at the same moment. This is not approximate; the chain's self-calibrating epoch schedule (described in §1.2.1) tracks Bitcoin's projected last-coin timestamp continuously and narrows to within minutes of precision by the time the final era arrives.

Emission is structured in four layers that combine each epoch:

**Layer D — Infrastructure base:** Minimal emission, always fires regardless of
network activity. Covers clock node rewards, testnet operator rewards, and the
mandatory 2% reserve (1.5% recycle, 0.5% testnet). Well under 1% of the block
reward in idle epochs. This layer ensures the network stays alive even during
periods of zero user activity.

**Layer B — Activity pools:** Each of the six work categories has a calibration
target. A pool's share of the epoch reward equals its actual utilization divided
by its target, normalized across all active pools. Calibration targets auto-adjust
slowly in an EIP-1559 style, drifting toward 50% long-run utilization per pool.
An empty pool's share recycles immediately — reward never accumulates in an
unearned pool.

**Layer A — Long-term scalar:** A dual exponential moving average governs the
block reward ceiling. A 7-day fast EMA tracks recent utilization (responsive to
activity spikes and troughs). A 90-day slow EMA is the permanent gravity center
that the fast EMA always decays back toward. Together they adjust the block reward
ceiling between 70% and 100% of the nominal schedule: a sustained busy network
approaches the 100% ceiling; a sustained idle network floors at 70%. Short-term
spikes cannot permanently inflate emission — only sustained utilization raises the
ceiling.

**Layer C — Fee-driven boost:** Verified fee volume from the previous epoch can
boost activity pool emission toward the ceiling set by Layer A. Three mechanisms
prevent circular self-payment from inflating rewards: only jobs with an
approved verifier verdict count; net flow accounting per address pair cancels
circular same-epoch flows; the previous-epoch lag requires that capital be locked
across two consecutive epochs.

Total emission each epoch equals base plus activity plus fee boost, hard-capped at
the Layer A adjusted ceiling. Any remainder flows to `btcpc_recycle`. Tokens are
never burned.

No tokens exist at chain launch except those earned through the emission schedule.
There is no pre-mine. There is no team allocation. The 17 founding accounts seeded
at genesis carry preserved keys — not pre-allocated balances.

### 1.2.1 Bitcoin Supply Alignment

BTCPC's 42,000,000-coin supply is designed to be fully mined at the same moment Bitcoin's last satoshi is mined — the two sovereign monetary networks converging at the same hour, by design.

**Era schedule and epoch scaling**

The chain runs in five new-supply eras (0–4). Each era contains 4,200,000 epochs. The epoch duration doubles at every era boundary, keeping daily throughput and daily reward income constant regardless of how long epochs become:

| Era | Epoch duration | Era wall-clock duration |
|-----|---------------|------------------------|
| 0   | ~30 seconds   | ~4 years               |
| 1   | ~1 minute     | ~8 years               |
| 2   | ~2 minutes    | ~16 years              |
| 3   | ~4 minutes    | ~32 years              |
| 4   | ~8 minutes    | ~64 years              |

**"~"** because the epoch durations for eras 1–4 are not fixed — they are recomputed automatically at each era boundary to track Bitcoin's projected last-coin timestamp (see below). Era 0's 30-second duration is the only constant.

**Self-calibrating end-date tracking**

At each era boundary, the chain automatically recomputes the next era's epoch duration using the formula:

```
D_N = (target_end_ms − now_ms) / (4,200,000 × (2^remaining_eras − 1))
```

where `target_end_ms` is Bitcoin's projected last-coin timestamp and `now_ms` is the current epoch seal time. This is evaluated in code — no governance vote or human action is required. The calibration fires automatically.

The base estimate for Bitcoin's last coin is:

```
BTC_GENESIS_MS + 6,930,000 blocks × 600,000 ms/block ≈ 2140-10-01 18:15 UTC
```

Each subsequent era recalibration absorbs any drift that accumulated in the previous era, progressively narrowing the deviation. By era 4 (~2076), Bitcoin will have completed 8 additional halvings and its remaining schedule will be known to within weeks. Era 4 epoch durations (~8 minutes) allow end-time precision of ±1 epoch.

**Bitcoin block oracle (precision layer)**

Any BTCPC node that also operates a Bitcoin full node may submit `BtcHeightReport` entries to the chain. These reports carry the current Bitcoin block height and the average block interval over the most recent 2016-block difficulty window. The chain maintains a 7-entry rolling median of recent reports and derives a continuously-refined `btc_end_ms` estimate from them:

```
btc_end_ms = report_timestamp + (6,930,000 − reported_height) × recent_avg_block_ms
```

This value supersedes the compile-time constant in era calibrations. The oracle is purely additive — nodes that do not run a Bitcoin node continue to function normally and benefit passively from reports submitted by those that do. No coordination, no voting, no governance.

Report validity rules (enforced on-chain):
- Must be signed by an active clock-node posting key.
- Bitcoin block height must be strictly increasing (no rewinding).
- Implied block rate must be within ±10% of 10 minutes (540–660 s).
- Reports older than 720 epochs are ignored in the median calculation.

**Result:** BTCPC's new supply ends within minutes of Bitcoin's last coin, automatically, converging tighter with each passing era as the oracle accumulates more data.

### 1.2.2 Perpetual Tail Emission — The Recycle Era

New supply is exhausted at era 5 (~2140, aligned with Bitcoin). At that
point the per-epoch block reward from the protocol drops to zero — but the chain does
not stop paying rewards. Instead, the `__recycle_fund__` system account becomes the
**permanent, self-replenishing reward source** for all subsequent epochs.

**What fills the recycle fund:**

Every economic event on the chain contributes to the fund over its lifetime:

- **Entry fees** — every transaction pays a base fee; 100% of fees flow to the recycle fund.
- **Mandatory reserve** — 1.5% of every epoch's block reward is withheld and deposited to the fund (the other 0.5% goes to the testnet fund). This fires from epoch 1 and never stops.
- **Surplus pool distributions** — any epoch reward that no participant earns (empty inference pool, no active storage nodes, etc.) flows directly to recycle rather than accumulating.
- **Slash proceeds** — 80% of any clock node or verifier slash goes to the recycle fund; 10% to the submitter as a bounty; 10% to `__legal__`.
- **Rounding remainders** — integer arithmetic on large reward pools always leaves indivisible dust. Every remainder accumulates in recycle.

By the time new supply ends, the fund will have been accumulating for 124 years of fee
income and surplus. It is structurally impossible for the fund to reach zero before the
chain itself loses all activity.

**How the fund pays rewards in era 5+:**

Each era-5 epoch, the protocol draws `RECYCLE_REWARD_RATE / RECYCLE_REWARD_DENOM`
(currently `10 / 1,000,000 = 0.001%`) of the fund balance and distributes it across
the same work pools as the new-supply era: inference, storage, sensor, clock, tracker,
verifier, service, and mempool. The same Layer A / B / C / D framework applies — unused
pool shares flow back to the fund.

**The equilibrium:**

```
fund_balance[n+1] = fund_balance[n]
                  + fees_this_epoch
                  + surplus_this_epoch
                  - 0.001% × fund_balance[n]
```

At equilibrium (fund balance stable), fees and surplus exactly cover the draw.
Because the draw is proportional to the balance (not a fixed amount), the fund
self-corrects: a larger balance pays more rewards but also decays faster; a smaller
balance decays slower and rebuilds from fees. The chain can sustain positive rewards
at any activity level, including very low activity — the reward just scales down.
There is no scenario in which the chain runs out of rewards while any transactions
are occurring, because every transaction replenishes the same pool that rewards draw from.

**This is not inflation.** The total supply ceiling of 42,000,000 BTCPC is never
breached. The recycle fund holds tokens already in existence — earned, paid as fees,
and returned to the protocol. Distributing them as era-5 rewards is a redistribution
of existing supply, not the creation of new supply.

### 1.3 Account Model

Every BTCPC account is derived from a single BIP-39 mnemonic seed phrase using
BIP-44 derivation with coin type **8888**. From one seed, a node generates six
BTCPC role keys and four external chain wallets:

**BTCPC role keys:**

| Key | Purpose |
|-----|---------|
| `owner` | Account recovery and key rotation — store offline |
| `active` | Token transfers, escrow operations, staking |
| `posting` | General chain entries: listings, orders, sensor data, repo ops |
| `memo` | Encrypt/decrypt memo fields in transactions |
| `hide` | Receive encrypted content (private repos, digital goods, DMs) |
| `seek` | Auto-deliver encrypted content (store fulfillment, repo push) |

The `hide` and `seek` keys form the end-to-end encrypted delivery layer for
digital commerce. When a buyer purchases a private digital product through
Freeport, the seller's `seek` key encrypts the payload to the buyer's `hide`
key at the `OrderFulfill` step. The buyer decrypts locally. The chain never
sees the plaintext. The same mechanism handles private repository access in
LinkGit: a `LinkGitAccessGrant` entry commits the repo key encrypted to the
grantee's `hide` key.

**External chain wallets (same seed, standard derivation):**

| Chain | Derivation |
|-------|-----------|
| EVM (Ethereum, BSC, etc.) | m/44'/60'/0'/0/0 |
| Bitcoin | m/84'/0'/0'/0/0 (native SegWit) |
| Solana | m/44'/501'/0'/0' |
| TON | standard TON path |

This means a single seed phrase controls a user's entire multi-chain identity.
BTCPC does not take custody of external chain assets — the derivation is
deterministic and happens client-side.

### 1.4 Chain ID and Genesis

| Network | Chain ID |
|---------|---------|
| Mainnet | `btcpc-1` |
| Testnet | `btcpc-satoshi` |

The genesis block is anchored to a canonical `BTCPC_GENESIS_TIMESTAMP`. Every
node on the network must use the identical genesis timestamp when computing epoch
boundaries. A node with a mismatched timestamp will fork immediately and find
zero peers. The genesis block contains 17 founding accounts seeded with preserved
keypairs, establishing initial identity for protocol accounts and the chain
founder. These accounts carry no pre-allocated token balance — they exist only
so that protocol-owned chain operations (fee collection, escrow settlement, recycle)
have a verified identity from block 0.

### 1.5 P2P Network

BTCPC uses **libp2p** for peer-to-peer communication. The primary transport is
**QUIC** for its low-latency connection establishment and multiplexing. The fallback
is **TCP with Noise_XX** handshake for environments where UDP is restricted. Maximum
peer connections per node: **MAX_PEERS = 50**.

Peer discovery uses a five-layer fallback stack, queried in order:

1. **RocksDB cache** — peers seen in previous sessions, fastest lookup
2. **Cloudflare DNS** — `_btcpc._udp.btcpc.net` TXT records with peer multiaddrs
3. **Hive blockchain** — peer list stored as Hive custom JSON ops, censorship-resistant
4. **TON smart contract** — peer registry on the TON blockchain
5. **Bitcoin Ordinals** — peer list inscribed as an Ordinal, immutable last resort

Bootstrap nodes operated by the protocol founder provide guaranteed entry points
for new nodes: `bootstrap1.btcpc.net` and `bootstrap2.btcpc.net`. A node that
reaches any of these five layers will find peers. A node that cannot reach any of
the five layers is not network-connected.

---

## 2. Native Protocol Businesses

Three protocols are native to BTCPC from genesis block 0. They are not plugins, not
sidechains, not smart contracts compiled to bytecode and deployed after the fact.
They are native ledger entry types processed by every node — as native to BTCPC as
a token transfer is native to Bitcoin. Each is a standalone business with its own
revenue model, owned initially by the protocol founder (shindevlin on both GitHub and
the BTCPC chain) and independently transferable via on-chain key rotation plus GitHub
repository ownership transfer.

### 2.1 Freeport — Sovereign Marketplace

Freeport is a peer-to-peer commerce protocol built directly into the BTCPC ledger.
Any account can open a storefront, list physical goods, digital products, or services,
and transact with buyers using BTCPC-native escrow — no intermediary, no platform
that can ban a seller, no payment processor that can reverse a charge. The chain
enforces escrow lockup at `OrderPlace`, releases funds to the seller after
`OrderFulfill` plus timeout, and routes disputes to the protocol's resolution layer.
Digital products are delivered end-to-end encrypted from seller's `seek` key to
buyer's `hide` key at fulfillment time.

| Entry | Signed by | What it does |
|-------|-----------|-------------|
| `StoreUpdate` | posting | Create or update storefront metadata |
| `ProductCreate` | posting | List a product (physical, digital, or service) |
| `ProductUpdate` | posting | Update price, stock level, or status |
| `OrderPlace` | memo + active | Buyer initiates purchase, locks escrow |
| `OrderFulfill` | seek | Seller delivers; digital products auto-decrypt to buyer's hide key |
| `OrderCancel` | posting | Cancel order and refund escrowed funds |
| `OrderDispute` | posting | Buyer opens a dispute for protocol review |
| `EscrowRelease` | system | Release escrow after fulfillment timeout passes |
| `FlashSale` | posting | Set a time-limited sale price on a product |

**Revenue model:** Freeport charges a settlement fee on escrow release, configurable
by the protocol account owner, defaulting to 0.5% of the transaction value. Additional
revenue comes from storage fees for product blob hosting (images, digital delivery
payloads) and dispute resolution fees charged to the losing party. All fees accrue
to the `freeport` protocol account in BTCPC.

**Ownership:** Protocol account `freeport` on the BTCPC chain. Owner: shindevlin.
GitHub: github.com/shindevlin. Transfer mechanism: on-chain `AccountUpdateKey` to
rotate owner and active keys to the buyer, combined with GitHub repository transfer.

### 2.2 Verasens — IoT Sensor Network

Verasens is a decentralized sensor data network that uses the BTCPC chain as its
immutable ledger and the BTCPC emission schedule as its incentive layer for data
quality. Sensor operators register their devices on-chain, stake BTCPC to signal
commitment to uptime, and earn IoT pool emissions pro-rata to verified readings
submitted. Data consumers query the Verasens API and pay per-query access fees in
BTCPC. The chain provides the trust layer that makes the sensor data credible: any
reading can be traced to a specific device key, timestamp, and epoch, with no ability
for the data provider to retroactively alter the record. LoRa gateways earn gateway
uptime rewards by submitting `GatewayHeartbeat` entries each epoch.

| Entry | Signed by | What it does |
|-------|-----------|-------------|
| `SensorRegister` | posting | Register a sensor device on-chain |
| `SensorKeyRegister` | posting | Register the device's signing key |
| `SensorVouch` | posting | Vouch for a sensor's data quality record |
| `SensorReading` | device key | Submit a single sensor reading |
| `SensorDataCommit` | device key | Commit a batch of readings as a Merkle root |
| `DeviceKeyRegister` | posting | Register an IoT device signing key |
| `DeviceYieldStake` | active | Stake BTCPC to earn device rewards |
| `DeviceYieldUnstake` | active | Unstake and withdraw |
| `GatewayHeartbeat` | device key | LoRa gateway uptime proof, once per epoch |
| `StorageHeartbeat` | posting | Storage node uptime proof for sensor data |

**Revenue model:** Data query fees charged per API call to the Verasens historical
data API, payable in BTCPC. Sensor registration requires a BTCPC stake, providing
Sybil resistance and protocol revenue. Gateway uptime rewards are funded by the
IoT pool within each epoch's emission.

**Ownership:** Protocol account `verasens` on the BTCPC chain. Owner: shindevlin.
Transfer mechanism: same as Freeport — on-chain key rotation plus GitHub transfer.

### 2.3 LinkGit — Decentralized Version Control

LinkGit brings Git-compatible version control to the BTCPC chain. Repositories are
registered as on-chain objects, with branch and tag references stored as ledger entries
and the underlying Git objects (blobs, trees, commits) stored by the BTCPC storage
node network. Private repositories use the `hide`/`seek` key pair for access control:
the repo encryption key is distributed to granted accounts as an `OrderFulfill`-style
encrypted payload committed at `LinkGitAccessGrant`. Storage nodes that perform
garbage collection work — pruning orphaned objects after a `LinkGitRefUpdate` — prove
their work with a `LinkGitPruneProof` entry and earn storage pool rewards. LinkGit
is designed to be used alongside GitHub via a mirror protocol, not as a replacement
for developers who want GitHub's collaboration UI.

| Entry | Signed by | What it does |
|-------|-----------|-------------|
| `LinkGitRepoCreate` | posting | Register a repo; include hide_key for private access |
| `LinkGitRefUpdate` | posting | Update a branch or tag ref; triggers GC queue |
| `LinkGitAccessGrant` | posting | Grant read access to a private repository |
| `LinkGitAccessRevoke` | posting | Revoke a previously granted access |
| `LinkGitPruneProof` | posting | Storage node proves GC work completed, claims reward |
| `LinkGitStorageExtend` | active | Pay to retain orphaned objects past default TTL |

**Mirror protocol:** A `.linkgit/mirrors` config file tracked inside the repository
defines mirror targets. After configuring a mirror with
`linkgit mirror add github https://github.com/owner/repo`, running
`linkgit mirror apply` causes every subsequent push to go to both the BTCPC chain
and the GitHub remote simultaneously. The developer's workflow is unchanged. The
repository gains a censorship-resistant backup with cryptographic provenance at
no additional step cost.

**Revenue model:** Per-object storage fees paid to storage nodes, with the protocol
account taking a percentage cut. Private repository fees at creation and renewal.
`LinkGitStorageExtend` fees charged when repository owners elect to retain orphaned
blobs past the default garbage collection window.

**Ownership:** Protocol account `linkgit` on the BTCPC chain. Owner: shindevlin.
Transfer mechanism: same as Freeport — on-chain key rotation plus GitHub transfer.

---

## 3. Protocol Ownership and Transfer

### 3.1 Initial Ownership

All three protocol accounts — `freeport`, `verasens`, and `linkgit` — were seeded at
genesis with posting keys controlled by shindevlin. The seed phrases for each protocol
account are held by shindevlin. shindevlin is also the GitHub user at
github.com/shindevlin and holds the canonical BTCPC repository. This concentration
of initial control is intentional: protocol parameters (fee rates, timeout windows,
storage pricing) must be tunable during the network's early period, and a single
responsible party provides accountability. The chain's design explicitly contemplates
transfer of each protocol to independent operators.

### 3.2 Transfer Mechanism

Each protocol can be transferred independently of the others and independently of the
BTCPC chain itself. A full transfer has two required components:

**On-chain:** The current owner submits `AccountUpdateKey` entries rotating the
protocol account's `owner` and `active` keys to the buyer's keys. Once this is
confirmed on-chain, the buyer controls all fee parameters, escrow settlement logic,
and protocol revenue. The chain enforces this — no central authority, no admin
override, no multisig council can reverse a valid key rotation. The chain *is* the
ownership record.

**Off-chain:** The GitHub repository holding the protocol's reference implementation
is transferred to the buyer's GitHub account via the standard GitHub repository
transfer mechanism. This gives the buyer control over the canonical codebase, the
issue tracker, and the documentation — the software side of the business.

A partial transfer (on-chain key rotation without GitHub transfer) is valid. The
buyer controls protocol operations on the chain. The seller retains the GitHub
repository, which becomes a fork dependency rather than the canonical source. Full
independence requires the buyer to fork the codebase under their own namespace.
The chain does not care which GitHub account hosts the implementation code — it only
validates signatures against the registered protocol account keys.

### 3.3 Monetization Path

Each protocol generates independent, compounding revenue streams denominated in BTCPC:

- **Freeport** earns on every completed transaction settled through the chain.
  At 0.5% of GMV, a marketplace processing modest volume generates substantial
  protocol revenue with zero marginal cost per transaction.

- **Verasens** earns per data query and per sensor registration stake. As the
  network of registered sensors grows, so does the value of the query API. The
  protocol benefits from network effects in sensor density.

- **LinkGit** earns on storage operations and private repository fees. As BTCPC
  adoption increases and developers route repositories through the network, the
  storage fee revenue scales with usage.

A buyer acquiring a protocol receives four things: the **protocol account** (controls
fee parameters and receives fee revenue), the **on-chain state** (all registered
stores, sensors, or repositories accumulated since genesis), the **codebase** (the
GitHub repository), and the **brand** (the protocol name and its established user
base). Protocols can also be licensed rather than acquired outright — a buyer deploys
a fork on another chain, pays a license fee to the `freeport`/`verasens`/`linkgit`
account, and operates under the brand in that context.

---

## 4. Key Management

BTCPC accounts use a role-based key architecture derived from a single BIP-39 mnemonic.
The derivation standard is BIP-44 with coin type **8888** (BTCPC). The six role keys
serve distinct operational security tiers: the `owner` key signs infrequently and
should be stored offline or in cold hardware; the `active` key signs financial
operations and should be protected at the same level as a bank password; the `posting`
key signs chain entries and can be stored in a hot wallet appropriate for frequent use.

The `memo`, `hide`, and `seek` keys handle encryption. The `memo` key encrypts and
decrypts the memo fields attached to standard transactions — it is the least sensitive
of the encryption keys. The `hide` key is the inbound delivery key: anything encrypted
to this key can only be decrypted by the account holder. It should be treated as a
private key. The `seek` key is the outbound delivery key used by automated processes
(store fulfillment daemons, repository push hooks) to encrypt payloads for delivery.
It should not be stored on internet-connected servers in plaintext.

The four external chain wallets (EVM, Bitcoin, Solana, TON) use standard BIP-44
derivation paths appropriate to each chain, derived from the same master seed. This
allows a hardware wallet holding the BTCPC seed to also sign transactions on any of
these chains, giving users a single physical device for their entire on-chain identity.

At account creation, the node displays all keys in the following format:

```
BTCPC Account Keys — [account_name]

  owner:   [key]   — store offline
  active:  [key]   — financial operations
  posting: [key]   — chain entries
  memo:    [key]   — message encryption
  hide:    [key]   — receive encrypted content
  seek:    [key]   — deliver encrypted content

External Wallets (same seed)
  EVM:     [address]
  BTC:     [address]
  SOL:     [address]
  TON:     [address]
```

The seed phrase is displayed once and never stored by the node software. The user
is responsible for backup.

---

## 5. Governance

There is no formal on-chain governance at launch. Protocol parameters — fee rates,
timeout windows, storage pricing, reward distribution adjustments — are controlled
by the respective protocol account owner. Chain parameters that are fixed at genesis
(epoch length, total supply, coin type) cannot be changed by any single account; they
require a coordinated network-wide consensus upgrade in which a supermajority of
block-producing nodes adopt the new version simultaneously.

This is not a gap to be filled later. It is a deliberate choice. Formal on-chain
governance systems tend to be captured by large token holders, reducing them to
plutocracies that replicate the centralization problems they were meant to solve.
BTCPC's launch design ensures that no governance token, no council, and no multisig
can modify the chain's fundamental parameters. The chain's rules are its constitution.
Changes require rough consensus among the operators who run the network — the same
mechanism that has governed Bitcoin's protocol changes for sixteen years.

Future versions of the protocol may introduce on-chain governance proposals for
protocol parameter changes, with participation weighted by stake or computational
contribution. Any such system will be introduced as an opt-in upgrade, not imposed
by a founding team.

---

## 6. Links

- **Website:** [btcpc.net](https://btcpc.net)
- **Telegram:** [@btcpcbot](https://t.me/btcpcbot)
- **GitHub:** [github.com/shindevlin/btcpc](https://github.com/shindevlin/btcpc)
- **Explorer:** [scan.btcpc.net](https://scan.btcpc.net)
- **Freeport Whitepaper:** [FREEPORT_PROTOCOL_WHITEPAPER.md](FREEPORT_PROTOCOL_WHITEPAPER.md)
- **Native Protocols Overview:** [NATIVE_PROTOCOLS.md](NATIVE_PROTOCOLS.md)

---

*BTCPC v1.0 — April 2026 — Shin Devlin*
