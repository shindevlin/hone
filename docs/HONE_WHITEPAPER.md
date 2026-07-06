# HONE

### A Proof-of-Compute Blockchain with Native Commerce, IoT, Version Control, and Rendering Protocols

**Shin Devlin**
**Version 1.1 — July 2026**

---

## Abstract

HONE is a blockchain where every token in existence was
earned by a machine doing useful work. Mining on HONE does not mean burning electricity
on a hash that disappears the moment it is found. It means running an AI inference job
that a user submitted, storing a file that someone committed, reporting a sensor reading
that a subscriber is paying for, or keeping the clock alive that the network depends on.
The computational output is the proof of work. The work had a customer.

Total supply is fixed at **42,000,000 HONE** with 10 decimal places:
**1 HONE = 10,000,000,000 hunits**. Epochs run every **30 seconds**, driven by
permissionless clock nodes that any machine on the network can operate. Emission
decays over epochs. There is no pre-mine. There is no founder allocation. Every token
in existence was earned.

Four protocols are native to the HONE chain from genesis block 0: **Freeport**
(a sovereign peer-to-peer marketplace with built-in escrow), **Verasens** (a decentralized
IoT sensor network with on-chain data provenance), **LinkGit** (a decentralized version
control system that mirrors seamlessly to GitHub), and **Wiiv** (a decentralized rendering
marketplace that turns a creative brief into a finished image, video, audio, or 3D
deliverable). Each protocol is a standalone business with its own revenue model, owned
initially by the protocol founder (shindevlin) and independently transferable via on-chain
key rotation.

HONE is **model-agnostic** and runs inference **in-process**: the node ships with no
default model and never auto-downloads one, and the compute that earns rewards is
performed by the trusted node binary itself — not a supervised external engine — so that
verifier nodes can re-run a job and confirm the work was real.

---

## 1. The Chain

### 1.1 Proof of Compute

HONE replaces the arbitrary SHA-256 puzzle with a requirement that miners produce
output that someone requested. The principle is simple: **mining should produce output
that someone wanted.**

HONE uses a fully dynamic reward model. Every epoch, the chain measures actual proven
work across six categories: inference/compute, storage, sensors/IoT, verifiers,
services (decentralized compute), and clock nodes. The share each category receives
from the epoch reward is proportional to its actual utilization against a calibrated
target — not a fixed percentage. If the chain becomes a storage chain, storage earns
the majority. If inference dominates, inference earns the majority. The market decides.

**Active work categories:**

- **Inference/Compute:** Nodes completing AI inference jobs requested by users.
  Rewarded proportional to verified value score — output tokens multiplied by hardware
  tier weight, model weight, and complexity factor — as assessed by verifiers. Inference
  runs **embedded in the node binary** (in-process candle GGUF), never delegated to a
  supervised external engine (see §1.1.1).

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
`hone_recycle` system account. Tokens are never burned. The supply ceiling of
42,000,000 HONE is a hard limit, not a target that gets compressed by burning.

### 1.1.1 Embedded Inference, the Model Registry, and Capability Provisioning

A chain that pays for compute has to be able to check that the compute happened. HONE
solves this at the foundation: **inference runs inside the trusted node binary** — an
in-process candle GGUF engine — rather than being handed off to a separately-supervised
external server. This is a chain-integrity property, not an implementation convenience.
Because the node decodes deterministically (greedy decode), any **verifier node can
re-run the same job against the same model and reproduce the same output** to confirm a
worker did the work it claims. An external, operator-controlled engine is a cheating
surface — it can return fabricated or cached output the chain cannot reproduce — so HONE
does the compute itself.

**Model-agnostic by design.** HONE has no hardcoded default model and never
auto-downloads one. There is no `DEFAULT_MODEL` the network picks for you. The operator
chooses which model(s) their machine serves, from a local model store, based on what the
hardware can actually run. A node with nothing enabled simply reports inference
unavailable until the operator enables a model — the network guesses nothing. This is a
sovereignty property: your machine runs what *you* chose, from weights *you* already hold.

**The on-chain model registry — analyze once, share to all.** Every node facing a model
needs the same objective facts to decide whether it can run it: minimum VRAM, quantization,
context length, file size, architecture. Having each machine independently analyze every
model is wasteful, so HONE makes model analysis a chain primitive. The first node to
analyze a given model publishes a `ModelManifestPublish` entry — keyed by the model's
**content hash** (sha256 of the weights), not its name — recording those objective facts
and the source. Every other node reads that manifest for free. The original analyzer earns
a small, usage-weighted, per-epoch-capped `ModelAnalysisReward` — drawn from the
**existing** reward pool, not new issuance, so cataloging useful models reallocates a
sliver of reward toward a public good rather than inflating supply. Reward is proportional
to a model's **attested usage in settled paid jobs**: `ModelUsageAttest` is a
system-emitted entry that fires only when a model is actually used in a settled paid job,
so usage is unfakeable — you cannot inflate a model's popularity without paying for real
jobs. Only the first valid manifest per content hash earns, so there is no incentive to
spam-publish, and a manifest that lies about its hardware requirements is slashable when a
real load contradicts it. Cataloging a model everyone runs earns; cataloging junk nobody
runs earns nothing.

**One uniform opt-in path — the capability provisioner.** A machine does not silently
become an inference, storage, sensor, or render node. A node-wide capability provisioner
gives every role the same "autopilot" onboarding: the machine **discovers** what it is
capable of, **hardware-gates** each capability against detected VRAM/RAM/disk, and then
the **operator explicitly enables** the roles it wants to serve. Auto-detection may
*suggest* what a machine could do; suggesting is never enabling. This is what makes large
render models safe to support without surprise multi-gigabyte downloads (see §2.4): nothing
heavy is fetched until an operator opts into a specific capability that clears its hardware
gate. The registry is the discovery input to this provisioner — "which models fit this
machine" is answered by gating the chain's published manifests against the local hardware.

### 1.2 Supply and Emission

The total supply of HONE is fixed at **42,000,000 HONE**. This is a hard ceiling
with no exception. The smallest unit is one **hunit**: 1 HONE = 10,000,000,000 hunits
(10^10, ten decimal places). All on-chain accounting is denominated in hunits; the
HONE display unit is a human convenience.

**Design intent:** HONE's new-supply emission is explicitly designed to end on the same day and at the same hour as Bitcoin's last mined satoshi. Both chains will exhaust their initial coin issuance simultaneously — two sovereign monetary networks converging at the same moment. This is not approximate; the chain's self-calibrating epoch schedule (described in §1.2.1) tracks Bitcoin's projected last-coin timestamp continuously and narrows to within minutes of precision by the time the final era arrives.

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
the Layer A adjusted ceiling. Any remainder flows to `hone_recycle`. Tokens are
never burned.

No tokens exist at chain launch except those earned through the emission schedule.
There is no pre-mine. There is no team allocation. The 17 founding accounts seeded
at genesis carry preserved keys — not pre-allocated balances.

### 1.2.1 Bitcoin Supply Alignment

HONE's 42,000,000-coin supply is designed to be fully mined at the same moment Bitcoin's last satoshi is mined — the two sovereign monetary networks converging at the same hour, by design.

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

Any HONE node that also operates a Bitcoin full node may submit `BtcHeightReport` entries to the chain. These reports carry the current Bitcoin block height and the average block interval over the most recent 2016-block difficulty window. The chain maintains a 7-entry rolling median of recent reports and derives a continuously-refined `btc_end_ms` estimate from them:

```
btc_end_ms = report_timestamp + (6,930,000 − reported_height) × recent_avg_block_ms
```

This value supersedes the compile-time constant in era calibrations. The oracle is purely additive — nodes that do not run a Bitcoin node continue to function normally and benefit passively from reports submitted by those that do. No coordination, no voting, no governance.

Report validity rules (enforced on-chain):
- Must be signed by an active clock-node posting key.
- Bitcoin block height must be strictly increasing (no rewinding).
- Implied block rate must be within ±10% of 10 minutes (540–660 s).
- Reports older than 720 epochs are ignored in the median calculation.

**Result:** HONE's new supply ends within minutes of Bitcoin's last coin, automatically, converging tighter with each passing era as the oracle accumulates more data.

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

**This is not inflation.** The total supply ceiling of 42,000,000 HONE is never
breached. The recycle fund holds tokens already in existence — earned, paid as fees,
and returned to the protocol. Distributing them as era-5 rewards is a redistribution
of existing supply, not the creation of new supply.

### 1.3 Account Model

**Names are the primary identity.** A HONE account is, first and foremost, a
human-readable name — `bullship`, written `@bullship` — in the ENS/Hive tradition, not a
hex blob. A person is a name. Names are claimed identities backed by stake, so the handle
you read is the handle you can trust. System accounts (`__treasury__`, `__recycle_fund__`)
have names but no key-derived address.

**Typed, checksummed addresses for machines.** Alongside names, HONE defines a typed
address format for tooling, QR codes, interop, and hardware confirmation — anywhere a raw
entity reference is needed instead of a claimed name. An address is **bech32**-encoded
(BIP-173) with a two-letter human-readable prefix that encodes *what kind of entity* it
refers to. Every HONE address begins with `h`:

| Entity | Prefix | Example |
|--------|--------|---------|
| Account / user | `hh` | `hh1…` |
| Contract | `hk` | `hk1…` |
| Token | `ht` | `ht1…` |
| Device / sensor | `hd` | `hd1…` |
| Vault | `hv` | `hv1…` |
| Escrow | `he` | `he1…` |

Because the prefix is folded into a checksummed encoding, a mistyped address — or one of
the wrong type — is **rejected at parse time, before any value moves**. You cannot send a
token to a contract by fat-fingering the recipient. This is a safety guarantee a bare hex
string cannot make, and it is deliberately *not* an `0x…` scheme: hex fights HONE's
name-based identity and echoes Ethereum. A claimed name (`@bullship`) and its typed
address (`hh1…`) resolve to the same account; resolvers accept either.

**Hardware-wallet compatible by construction.** HONE keys are **ed25519 on the SLIP-10
hardened path `m/44'/6942'/role'/0'`** (6942 is HONE's coin index; `role` selects the key
below). This is the same curve and derivation Ledger, Trezor, and Keystone already support
natively for Cosmos, Solana, and Stellar, and the `hh1…` bech32 address is the same
Cosmos-style bech32-over-ed25519-pubkey pattern those devices already display and sign for.
A future HONE Ledger app is therefore a build, not a redesign: nothing in the key or
address scheme obstructs on-device signing where the private key never leaves the device.

From one BIP-39 mnemonic seed phrase, a node derives six HONE role keys and four external
chain wallets:

**HONE role keys:**

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
HONE does not take custody of external chain assets — the derivation is
deterministic and happens client-side.

**Recoverable key storage (sovereignty guarantee).** Self-custody is only real
if the user can actually recover their keys. Every wallet created on HONE writes
a **recoverable encrypted keystore** — `<account>.keystore.json` — that seals the
BIP-39 mnemonic with **Argon2id** (a memory-hard password KDF) and
**AES-256-GCM** (authenticated encryption). The password is set by the user and
never leaves the device; only ciphertext is ever written or, optionally, backed
up. Recovery has three layers, so no single loss is fatal:

1. **Keystore file** — unlock with your password to recover the account.
2. **Recovery phrase** — the mnemonic is displayed once at creation, with a
   write-it-down confirmation gate; it recovers the account even with no file.
3. **Optional encrypted relay backup** — the user may store the *ciphertext*
   blob on a HONE relay so the account survives losing the local file. The
   password never leaves the device; the relay holds only data it cannot decrypt.

Wallet creation cannot complete without producing a recoverable keystore — you
can no longer create an account that leaves no way back to it. This is enforced
in the wallet tooling (`hone wallet new`), not left to user discipline.

### 1.4 Chain ID and Genesis

| Network | Chain ID |
|---------|---------|
| Mainnet | `hone` |
| Testnet | `hone-testnet` |

**Genesis: July 4th, 2026 — noon Los Angeles (`1783191600000` ms, 19:00 UTC).**
HONE launches on America's 250th anniversary. It is freedom tech for a
freedom-based country: a sovereign chain with self-custody, no gatekeepers, and
— from this genesis forward — no silent loss of keys (§1.3).

The genesis block is anchored to a canonical `HONE_GENESIS_TIMESTAMP`. Every
node on the network must use the identical genesis timestamp when computing epoch
boundaries. A node with a mismatched timestamp will fork immediately and find
zero peers. The genesis block seeds founding accounts with preserved keypairs,
establishing initial identity for protocol accounts and the chain founder. Every
founding key is created under the recoverable-keystore flow (§1.3) and backed up
locally — the genesis itself embodies the sovereignty guarantee. These accounts
carry no pre-allocated token balance — they exist only so that protocol-owned
chain operations (fee collection, escrow settlement, recycle) have a verified
identity from block 0.

> **Why a fresh genesis.** The prior chain (May 1 2026, "Mayday") created wallets
> without durable key storage: seed phrases were shown once and, in practice,
> lost — leaving accounts that were owned but unsignable. That is a chain-fatal
> flaw that cannot be patched in place. Genesis v2 makes recoverable key storage
> a precondition of account creation, so it can never recur.

### 1.5 P2P Network

HONE uses **libp2p** for peer-to-peer communication. The primary transport is
**QUIC** for its low-latency connection establishment and multiplexing. The fallback
is **TCP with Noise_XX** handshake for environments where UDP is restricted. Maximum
peer connections per node: **MAX_PEERS = 50**.

Peer discovery uses a five-layer fallback stack, queried in order:

1. **RocksDB cache** — peers seen in previous sessions, fastest lookup
2. **Cloudflare DNS** — `_hone._udp.honemesh.net` TXT records with peer multiaddrs
3. **Hive blockchain** — peer list stored as Hive custom JSON ops, censorship-resistant
4. **TON smart contract** — peer registry on the TON blockchain
5. **Bitcoin Ordinals** — peer list inscribed as an Ordinal, immutable last resort

Bootstrap nodes operated by the protocol founder provide guaranteed entry points
for new nodes: `bootstrap1.honemesh.net` and `bootstrap2.honemesh.net`. A node that
reaches any of these five layers will find peers. A node that cannot reach any of
the five layers is not network-connected.

---

## 2. Native Protocol Businesses

Four protocols are native to HONE from genesis block 0. They are not plugins, not
sidechains, not smart contracts compiled to bytecode and deployed after the fact.
They are native ledger entry types processed by every node — as native to HONE as
a token transfer is native to Bitcoin. Each is a standalone business with its own
revenue model, owned initially by the protocol founder (shindevlin on both GitHub and
the HONE chain) and independently transferable via on-chain key rotation plus GitHub
repository ownership transfer.

### 2.1 Freeport — Sovereign Marketplace

Freeport is a peer-to-peer commerce protocol built directly into the HONE ledger.
Any account can open a storefront, list physical goods, digital products, or services,
and transact with buyers using HONE-native escrow — no intermediary, no platform
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
to the `freeport` protocol account in HONE.

**Ownership:** Protocol account `freeport` on the HONE chain. Owner: shindevlin.
GitHub: github.com/shindevlin. Transfer mechanism: on-chain `AccountUpdateKey` to
rotate owner and active keys to the buyer, combined with GitHub repository transfer.

### 2.2 Verasens — IoT Sensor Network

Verasens is a decentralized sensor data network that uses the HONE chain as its
immutable ledger and the HONE emission schedule as its incentive layer for data
quality. Sensor operators register their devices on-chain, stake HONE to signal
commitment to uptime, and earn IoT pool emissions pro-rata to verified readings
submitted. Data consumers query the Verasens API and pay per-query access fees in
HONE. The chain provides the trust layer that makes the sensor data credible: any
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
| `DeviceYieldStake` | active | Stake HONE to earn device rewards |
| `DeviceYieldUnstake` | active | Unstake and withdraw |
| `GatewayHeartbeat` | device key | LoRa gateway uptime proof, once per epoch |
| `StorageHeartbeat` | posting | Storage node uptime proof for sensor data |

**Revenue model:** Data query fees charged per API call to the Verasens historical
data API, payable in HONE. Sensor registration requires a HONE stake, providing
Sybil resistance and protocol revenue. Gateway uptime rewards are funded by the
IoT pool within each epoch's emission.

**Ownership:** Protocol account `verasens` on the HONE chain. Owner: shindevlin.
Transfer mechanism: same as Freeport — on-chain key rotation plus GitHub transfer.

### 2.3 LinkGit — Decentralized Version Control

LinkGit brings Git-compatible version control to the HONE chain. Repositories are
registered as on-chain objects, with branch and tag references stored as ledger entries
and the underlying Git objects (blobs, trees, commits) stored by the HONE storage
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
`linkgit mirror apply` causes every subsequent push to go to both the HONE chain
and the GitHub remote simultaneously. The developer's workflow is unchanged. The
repository gains a censorship-resistant backup with cryptographic provenance at
no additional step cost.

**Revenue model:** Per-object storage fees paid to storage nodes, with the protocol
account taking a percentage cut. Private repository fees at creation and renewal.
`LinkGitStorageExtend` fees charged when repository owners elect to retain orphaned
blobs past the default garbage collection window.

**Ownership:** Protocol account `linkgit` on the HONE chain. Owner: shindevlin.
Transfer mechanism: same as Freeport — on-chain key rotation plus GitHub transfer.

### 2.4 Wiiv — Decentralized Rendering

Wiiv is HONE's rendering marketplace: a buyer arrives with rough creative intent and
leaves with a finished deliverable. It is **modality-agnostic** — image, video, audio,
3D, and composite renders are all first-class, served by the same job and settlement
machinery — and it is an **A-to-Z production supply chain**, not a single model call.
Wiiv coordinates local models, distributed GPU workers, human specialists, storage nodes,
and reviewers as one pipeline: it drafts the brief, compiles a plan of milestones, takes
worker bids, escrows the budget, delivers artifacts milestone by milestone, and settles
against the buyer's acceptance. Like the other three protocols, Wiiv is native ledger
entry types processed by every node, not a smart contract or sidechain. Its reserved
accounts, `wiiv` and `wiiv-escrow`, are seeded at block 0.

The foundational rule is that **subjective quality is never a consensus input**. The
chain records *facts* — a job was posted, a bid was made, an artifact CID was delivered,
the buyer accepted, escrow was released. Whether a render is *good* is decided by the
buyer, a reviewer market, and reputation — never by chain validators. Nodes do not vote
on aesthetics. This is the boundary that lets a creative marketplace live on a chain
without corrupting consensus: HONE settles the transaction, it does not grade the art.

| Entry | Signed by | What it does |
|-------|-----------|-------------|
| `WiivWorkerRegister` | posting | Advertise a worker's capabilities, modalities, price hints, and hardware attestation — emitted only after a local self-test render passes |
| `WiivWorkerHeartbeat` | posting | Keep a worker's listing live and update availability |
| `WiivRenderJobPost` | posting | Post a job: modality, milestone plan, required capabilities, budget cap, revision and retention policy |
| `WiivJobFund` | active | Commit escrow up to the job's budget cap |
| `WiivRenderBid` | memo | A worker bids on a job or milestone: price, ETA, capabilities |
| `WiivBidAward` | posting | Award a job/milestone to a worker; lock that tranche of escrow |
| `WiivMilestoneDeliver` | posting | Submit a milestone's artifact CIDs plus a provenance record |
| `WiivArtifactDeliver` | seek | Attach the final artifact bundle, encrypted to the buyer's hide key |
| `WiivMilestoneAccept` | posting | Accept a milestone; release its escrow tranche |
| `WiivRevisionRequest` | posting | Return a delivered milestone with notes, bounded by revision policy |
| `WiivJobAccept` | posting | Accept the full deliverable; trigger final settlement |
| `WiivJobSettle` | system | Release remaining escrow and mark the job settled |
| `WiivDisputeOpen` | posting | Open a dispute; freeze settlement and engage the reviewer market |
| `WiivDisputeResolve` | posting | Record the reviewer market's reputation-weighted outcome |
| `WiivStorageExtend` | posting | Pay to retain specific artifact CIDs past the default window |

**Milestones and piecewise settlement.** Long-form work (a multi-shot video, a scored
episode) is broken into milestones with dependency edges, each carrying its deliverable
kinds and required capabilities. Accepting a milestone releases its escrow tranche to the
awarded worker, so a job pays out as it progresses instead of all-or-nothing at the end.
Every delivered artifact is a content-addressed blob in HONE-FS carrying an append-only
**provenance record** — which worker produced it, which capability and model were used,
and the input CIDs it derived from — so a finished deliverable travels with a verifiable
chain of custody from brief to final render. Private-job artifacts are encrypted to the
buyer's `hide` key before storage; no storage node can read them.

**Opt-in render workers, no surprise downloads.** A base node ships with only the small
embedded text model. Render models and runtimes are large — a video diffusion model is
many gigabytes — so they are pulled **only when an operator opts into a specific render
capability** through the capability provisioner (§1.1.1). Enabling is explicit and
per-capability, hardware-gated *before* any download (a capability the machine can't meet
is refused with the shortfall reported), and a node only advertises a capability after the
model is resident and a local self-test render succeeds. Registrations carry a hardware
attestation and a slashable stake, so one machine can't sybil many workers and advertising
work a node can't deliver forfeits stake.

**Revenue model:** Wiiv earns a settlement fee on job escrow release and a dispute fee
funding the reviewer market, plus `WiivStorageExtend` fees for extended artifact
retention. Reviewers are paid from the dispute fee, not the epoch pool; bad-faith
reviewing is slashable. All fees accrue to the `wiiv` protocol account in HONE.

**Ownership:** Protocol account `wiiv` on the HONE chain (escrow held by `wiiv-escrow`).
Owner: shindevlin. Transfer mechanism: same as Freeport — on-chain key rotation plus
GitHub transfer.

---

## 3. Protocol Ownership and Transfer

### 3.1 Initial Ownership

All four protocol accounts — `freeport`, `verasens`, `linkgit`, and `wiiv` — were seeded at
genesis with posting keys controlled by shindevlin. The seed phrases for each protocol
account are held by shindevlin. shindevlin is also the GitHub user at
github.com/shindevlin and holds the canonical HONE repository. This concentration
of initial control is intentional: protocol parameters (fee rates, timeout windows,
storage pricing) must be tunable during the network's early period, and a single
responsible party provides accountability. The chain's design explicitly contemplates
transfer of each protocol to independent operators.

### 3.2 Transfer Mechanism

Each protocol can be transferred independently of the others and independently of the
HONE chain itself. A full transfer has two required components:

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

Each protocol generates independent, compounding revenue streams denominated in HONE:

- **Freeport** earns on every completed transaction settled through the chain.
  At 0.5% of GMV, a marketplace processing modest volume generates substantial
  protocol revenue with zero marginal cost per transaction.

- **Verasens** earns per data query and per sensor registration stake. As the
  network of registered sensors grows, so does the value of the query API. The
  protocol benefits from network effects in sensor density.

- **LinkGit** earns on storage operations and private repository fees. As HONE
  adoption increases and developers route repositories through the network, the
  storage fee revenue scales with usage.

- **Wiiv** earns a settlement fee on every render job that clears escrow, plus dispute
  and artifact-retention fees. As the network's render capacity grows across modalities,
  larger and more numerous jobs settle through the protocol, and fee revenue scales with
  the total value of production routed through it.

A buyer acquiring a protocol receives four things: the **protocol account** (controls
fee parameters and receives fee revenue), the **on-chain state** (all registered
stores, sensors, repositories, or render jobs accumulated since genesis), the
**codebase** (the GitHub repository), and the **brand** (the protocol name and its
established user base). Protocols can also be licensed rather than acquired outright — a
buyer deploys a fork on another chain, pays a license fee to the
`freeport`/`verasens`/`linkgit`/`wiiv` account, and operates under the brand in that
context.

---

## 4. Key Management

HONE accounts use a role-based key architecture derived from a single BIP-39 mnemonic.
HONE keys are **ed25519 on the SLIP-10 hardened path `m/44'/6942'/role'/0'`** (6942 is
HONE's coin index) — the curve and derivation hardware wallets already sign natively (§1.3).
The six role keys serve distinct operational security tiers: the `owner` key signs
infrequently and should be stored offline or in cold hardware; the `active` key signs financial
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
allows a hardware wallet holding the HONE seed to also sign transactions on any of
these chains, giving users a single physical device for their entire on-chain identity.

At account creation, the node displays all keys in the following format:

```
HONE Account Keys — [account_name]

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
HONE's launch design ensures that no governance token, no council, and no multisig
can modify the chain's fundamental parameters. The chain's rules are its constitution.
Changes require rough consensus among the operators who run the network — the same
mechanism that has governed Bitcoin's protocol changes for sixteen years.


Future versions of the protocol may introduce on-chain governance proposals for
protocol parameter changes, with participation weighted by stake or computational
contribution. Any such system will be introduced as an opt-in upgrade, not imposed
by a founding team.

---

## 6. Links

- **Website:** [honemesh.net](https://honemesh.net)
- **Telegram:** [@btcpcbot](https://t.me/btcpcbot) (rename pending)
- **GitHub:** [github.com/shindevlin/btcpc](https://github.com/shindevlin/btcpc)
- **Explorer:** [scan.honemesh.net](https://scan.honemesh.net)
- **Freeport Whitepaper:** [FREEPORT_PROTOCOL_WHITEPAPER.md](FREEPORT_PROTOCOL_WHITEPAPER.md)
- **Native Protocols Overview:** [NATIVE_PROTOCOLS.md](NATIVE_PROTOCOLS.md)
- **Wiiv Protocol:** [WIIV_PROTOCOL.md](WIIV_PROTOCOL.md)
- **Model Registry Protocol:** [MODEL_REGISTRY_PROTOCOL.md](MODEL_REGISTRY_PROTOCOL.md)
- **Address Scheme:** [ADDRESS_SCHEME.md](ADDRESS_SCHEME.md)

---

*HONE v1.1 — July 2026 — Shin Devlin*
