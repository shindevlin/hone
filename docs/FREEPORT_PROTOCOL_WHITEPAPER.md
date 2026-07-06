# Freeport Protocol (FP)

### A Sovereign Blockchain Where Every Token Is Earned by a Machine Doing Real Work

**Shin Devlin**
**Version 3.1 — April 2026**

---

> **Note (2026):** Freeport Protocol is now a native protocol within BTCPC, deployed at genesis block 0. All entry types described in this whitepaper are natively supported by the BTCPC chain. No separate deployment required. See [NATIVE_PROTOCOLS.md](NATIVE_PROTOCOLS.md) for the full native protocol overview.

---

## Abstract

Freeport Protocol (FP) is a sovereign blockchain where block rewards are earned
by machines performing verifiable, useful work. The native token — **BTCPC** — is
the unit of settlement for all work and commerce on the chain. Five categories of work
produce emissions each epoch: AI inference, data storage, service hosting, IoT sensor
reporting, and epoch timing. Five categories of work produce emissions
each epoch: AI inference, data storage, service hosting, IoT sensor reporting, and epoch
timing. Each category has its own reward pool. Every pool with active participants pays
out pro-rata to those participants. Pools with no claimants are recycled — never burned —
back into future block rewards via the `btcpc_recycle` system account.

Total supply is fixed at **42,000,000 BTCPC** with 10 decimal places. Epochs run every
**30 seconds**, driven by permissionless clock nodes. There is no proof-of-work puzzle,
no staking barrier to entry, and no synthetic work. If nobody submits an AI inference
job today, the miner pool recycles. The chain's value is entirely derived from the
real work it coordinates.

The design principle is simple: **mining should produce output that someone wanted.**

---

## 1. Introduction

### 1.1 The Problem with Proof of Work

Bitcoin's proof-of-work consumes approximately 150 TWh of electricity per year. The
SHA-256 puzzles that miners solve produce nothing except chain security. The energy
expenditure is not wasted in the security sense — it is precisely what makes attacks
expensive — but the *computational output* of every solved puzzle is discarded immediately.
The hash goes nowhere. The GPU cycles produce no artifact that any user ever requested.

This was a reasonable trade-off in 2009, when the goal was to build a credibly neutral
settlement layer with no trusted parties. The work proved the miner's commitment. The
puzzle was intentionally useless so nobody could capture its output.

But the world has changed. AI inference is a multi-hundred-billion-dollar industry. Data
storage is a multi-hundred-billion-dollar industry. IoT telemetry, application hosting,
and sensor networks are all real economic demand that currently flows through centralized
clouds at monopoly prices. Millions of GPUs sit inside gaming rigs, mining rigs, and
cloud servers that are underutilized for large portions of every day.

The question was never "why another blockchain?" The question is: **why are we still
paying Amazon for cloud computing when millions of GPUs sit idle mining hashes nobody
uses?**

### 1.2 The Freeport Insight

Freeport Protocol applies Bitcoin's core insight — costly, verifiable work can secure a
network and back a scarce asset — to a domain where the work itself has market value.

Miners on BTCPC earn by completing AI inference jobs that real users submitted through
the API. The inference result is the proof of work. The job was something the user
actually wanted. The token earned is payment for a real service, not a reward for solving
an artificial puzzle.

The same pattern applies to every other work category: storage hosts store files that
someone committed to the chain. Clock nodes keep timing that the consensus protocol
depends on. Sensor bridges relay readings from physical devices that subscribers are
paying for. Service hosts run applications that users are accessing.

Every token in existence represents a verified unit of economic activity.

### 1.3 Positioning

**Bitcoin** is digital gold. Store of value. Proof of Work secures the ledger.
The computational output is discarded by design.

**Ethereum** is a world computer. Proof of Stake secures a general-purpose VM.
Smart contracts are the product. The chain is the platform.

**Freeport Protocol** is the digital labor market and sovereign commerce layer. Proof of
Compute means the mining IS the product. Every BTCPC token represents real work done:
an AI prompt answered, a file stored, a sensor reading verified, an application served,
a clock heartbeat delivered. Commerce on Freeport requires no middleman — the chain is
the marketplace, the escrow, and the reputation system.

---

## 2. Consensus: Proof of Compute

### 2.1 Epochs

BTCPC operates on fixed 30-second epoch intervals. At each epoch boundary:

1. Clock nodes submit a heartbeat transaction with their local timestamp
2. The median of all heartbeats within a tolerance window establishes the canonical
   epoch close time
3. Pending ledger entries (inference results, transfers, storage proofs, etc.) are
   collected into a block candidate
4. A consensus hash is computed from the epoch's entries
5. Miners and verifiers agree on the canonical block
6. The block is written to disk and broadcast to peers

Epochs are driven by permissionless clock nodes. Any device capable of making an HTTP
request and sending a signed heartbeat transaction can be a clock node. A laptop, a
phone, a Raspberry Pi — all qualify. Clock participation earns 5% of every epoch's
block reward, split equally among active clocks.

There is no fixed "block proposer" or leader. The broadcaster for a given epoch is
selected deterministically from the participating set based on the epoch number and
peer availability. This prevents any single node from controlling block production.

### 2.2 Mining: Real Inference Only

Miners earn BTCPC by completing AI inference jobs. There is no make-work puzzle. A
miner that has no pending jobs to complete earns nothing for that epoch. The entire
miner reward pool for an epoch with no submitted jobs is recycled to `btcpc_recycle`.

The inference pipeline:

1. A user submits an `InferenceJob` via the API (REST or P2P)
2. The job enters the pending-entries queue, visible to all nodes
3. Miners pick up the job, run inference via their local Ollama instance
4. The miner commits to the result using a hash (commit phase)
5. After a short reveal window, the miner reveals the full result (reveal phase)
6. Multiple verifiers independently re-run a sample of the inference or validate the
   result against a known reference
7. Median consensus across verifiers determines whether the result is valid
8. Valid results: miner earns reward proportional to `work_value`
   (tokens generated × model parameter count in billions)
9. Invalid or missing results: miner earns nothing for that job; verifiers flag the
   discrepancy; stake may be slashed on repeated fraud

Ollama is the inference backend. BTCPC is model-agnostic: miners run any model that
Ollama supports (qwen, llama, mistral, gemma, deepseek, etc.). Work value scales with
the verified parameter count from Ollama's `/api/show` endpoint, not the model name.
A miner running a 70B model earns more per job than one running a 7B model, because
the work is harder and the output quality is proportionally higher.

No model is required or preferred. The protocol rewards verified compute, not brand
loyalty.

### 2.3 Verification

Verification is a separate role from mining. Verifiers:

- Validate that the inference result hash matches the revealed content
- Spot-check storage hosts by issuing challenge-response probes against stored blobs
- Validate sensor readings for statistical plausibility
- Cross-check service host heartbeats and session proofs

A verifier panel for a given inference job is selected pseudo-randomly from registered
verifiers. A verifier that consistently votes against the winning consensus has their
stake reduced. A verifier that is persistently idle earns nothing. A good verifier
earns 10% of each epoch's block reward, split pro-rata by verification count.

### 2.4 P2P Security

All P2P messages in BTCPC are signed with ECDSA using the sender's Active key. Unsigned
messages are rejected. Message types that require signatures:

- `BLOCK_PROPOSAL` — signed by the broadcasting node
- `EPOCH_FINALIZED` — signed by the consensus coordinator
- `MEMPOOL_ENTRY` — signed by the originating account
- `ACCOUNT_ANNOUNCE` — signed by the account's Owner key
- `CLOCK_HEARTBEAT` — signed by the clock node's Posting key
- `STORAGE_HEARTBEAT` — signed by the storage host's Posting key
- `SENSOR_READING` — signed by the gateway's Posting key

Peers that relay unsigned or mis-signed messages are deprioritized in peer scoring.
Peers that relay messages with valid signatures but invalid semantic content (e.g., a
balance transfer with insufficient funds) are rejected at the entry validation layer.

### 2.5 Block Finalization

When consensus is reached on an epoch:

1. The winning block is written to `data/blocks/block-NNNNNNNN.bin`
   (the permanent source of truth)
2. Ledger entries are applied to the in-memory stateStore (fast reads for all nodes)
3. The stateManager's Sparse Merkle Tree (SMT) is updated with the new state root
4. Every N epochs, a finality snapshot is written to `data/blocks/finality-NNNNNNNN.bin`
   to enable fast-forward sync for new nodes
5. Rewards are distributed: each reward pool is computed and credited in the same block

The blockchain on disk is the canonical source of truth. The in-memory stateStore is a
performance cache rebuilt from blocks on startup. MongoDB is never required for
consensus-critical operations.

### 2.6 Dynamic Block Cap

BTCPC uses an adaptive block size cap that responds to demand. The cap starts at 1 MB
and adjusts each epoch:

- If the current block is more than 50% full: next cap = min(current × 1.25, 32 MB)
- If the current block is less than 25% full: next cap = max(current × 0.75, 1 MB)

The current block cap is recorded in the block header so all nodes agree on the limit
for the next epoch. This creates a self-regulating throughput ceiling of approximately
1,000 TPS at steady state, scaling to 32 MB blocks (~32,000 TPS ceiling) under
sustained demand.

---

## 3. Account Model

### 3.1 Key Hierarchy

Every BTCPC account is derived from a single BIP-39 mnemonic (12 words). The mnemonic
derives four secp256k1 key pairs with distinct permission levels:

```
12-Word BIP-39 Mnemonic
  │
  ├── m/44'/btcpc'/0'/0/0  → Owner Key
  ├── m/44'/btcpc'/0'/1/0  → Active Key
  ├── m/44'/btcpc'/0'/2/0  → Posting Key
  └── m/44'/btcpc'/0'/3/0  → Memo Key
```

**Owner key** — cold storage, account recovery. Used only for recovery operations and
for changing other keys. Never exposed online. The 72-hour recovery window is enforced
by the protocol: any account recovery must wait 72 hours before the new keys are
accepted, giving the original owner time to cancel a fraudulent recovery.

**Active key** — financial operations. Transfers, staking, escrow, bridge operations,
store management, and any other operation that moves funds. This key should be kept
offline when not in active use.

**Posting key** — operational identity. Mining submissions, clock heartbeats, storage
heartbeats, service heartbeats, inference job submissions, reviews, reputation votes,
and any high-frequency chain interaction. Lower value per action; higher exposure.

**Memo key** — encrypted communication. Used for encrypting inference prompts, private
messages, and any data that should be readable only by the recipient. The memo key is
never used for signing transactions; it is used only for ECDH key agreement to produce
a shared secret for AES-256-GCM encryption.

### 3.2 Protocol-Enforced 2FA

Account creation includes a TOTP secret that becomes part of the account's on-chain
record. Financial operations (Active key actions) require a valid TOTP code to be
included in the transaction. This prevents private key theft from being immediately
exploitable: the attacker needs both the key and access to the TOTP device.

### 3.3 Account Names

Account names are 3–20 characters, lowercase letters and numbers only, no leading or
trailing hyphens. 420 premium names were reserved at genesis for controlled release.
Name registration costs a flat fee that routes to `btcpc_recycle`. Names are permanent
once registered — there is no expiry, no renewal fee, and no squatter protection beyond
first-come-first-served.

### 3.4 Cross-Chain Addresses

Each account can register chain addresses for Ethereum, Solana, Bitcoin, and TON (and
future chains). These addresses are stored on-chain under the account record and are
used by the bridge relay to verify ownership across chains. Cross-chain address
registration does not require a fee; it is included in the account creation transaction.

---

## 4. Emission and Reward Distribution

### 4.1 Supply

Total supply: **42,000,000 BTCPC** — fixed forever. No inflation beyond the emission
schedule. No governance-adjustable issuance. No pre-mine. No founder allocation. No VC
allocation. Every token in existence has been (or will be) earned through work on the
live chain.

Denomination: **1 BTCPC = 10,000,000,000 dreams** (10^10 base units). Dreams are the
base unit for internal calculations. All on-chain amounts are rounded to 10 decimal
places. The dream denomination future-proofs the chain: even at $100,000 per BTCPC, one
dream is worth $0.00001 — fine-grained enough for microtransactions in commerce,
sensor markets, and inference pricing.

### 4.2 Emission Schedule

| Year | Reward/epoch | Yearly emission | Cumulative |
|------|-------------|-----------------|------------|
| 1    | 24.306 BTCPC | ~25,560,000     | 25,560,000 |
| 2    | 12.153 BTCPC | ~12,780,000     | 38,340,000 |
| 3    | 6.077 BTCPC  | ~6,390,000      | ~42,000,000 (capped) |

Emission halts when cumulative reaches 42,000,000 BTCPC. Post-emission, block rewards
are funded entirely from `btcpc_recycle` (fee flows + recycled unclaimed rewards).
Since all fees recycle, block rewards remain meaningful indefinitely — the pool is
continuously replenished.

The 30-second epoch produces 2,880 epochs per day and ~1,051,200 epochs per year.
The genesis reward of 24.306 BTCPC/epoch produces approximately 25.5M BTCPC in year one.

### 4.3 Six Reward Pools

Each epoch's block reward is split among six pools. Unclaimed shares of any pool
(because no eligible participants were active) flow to `btcpc_recycle`.

| Pool | Share | What earns it |
|------|-------|---------------|
| Miners | 55% | Completing real AI inference jobs via Ollama |
| Verifiers | 10% | Validating inference results and issuing spot-checks |
| Clocks | 5% | Delivering epoch heartbeats (any device, always paid if active) |
| Storage Hosts | 12% | Hosting and serving BTCPC-FS blobs |
| Service Hosts | 8% | Running compute workloads (stateless or stateful) |
| IoT / Sensors | 10% | Relaying sensor readings via LoRa gateways |

The IoT pool splits 60% to sensors (per valid reading) and 40% to gateways (per epoch
uptime). Clocks are always paid first if any clocks are active — clock uptime is the
foundation the rest of the chain depends on.

### 4.4 Within-Pool Distribution

Each pool distributes among its active participants according to work done:

- **Miners**: pro-rata by `work_value` (tokens generated × model parameter count in
  billions). Only honest commitments (passing verifier consensus) count.
- **Verifiers**: pro-rata by verification count in the epoch, matching the winning
  consensus panel.
- **Clocks**: equal split among clocks with valid heartbeats in the last 3 epochs.
- **Storage hosts**: pro-rata by `committed_gb × uptime_factor × rep_score +
  bandwidth_served_gb`.
- **Service hosts**: pro-rata by `active_session_count × uptime_factor × rep_score`.
- **Sensors**: pro-rata by valid readings relayed, weighted by registered sensor uptime.
- **Gateways**: equal split among gateways with valid heartbeats in the epoch.

A participant below the minimum reputation threshold earns zero for that epoch,
regardless of work done. This gates out sybil attacks without slashing stake.

### 4.5 No Burn, All Recycle

**BTCPC will never implement a token burn mechanism, under any circumstances.**

Every fee, every slashed stake, every unclaimed pool share, every escrow that expires
without a valid claim flows to `btcpc_recycle` — a system account that is steadily
drained back into block rewards over time. This is a hard architectural commitment,
equivalent in permanence to the 42,000,000 supply cap.

Why recycle instead of burn?

1. **Velocity.** Tokens that flow into recycle flow back out via block rewards. No idle
   reserves, no dead capital. The full 42M is always circulating somewhere.
2. **Honesty.** Fees pay for work. Recycled fees pay for the next round of work.
3. **Fairness.** Burning transfers wealth invisibly to existing holders at the expense
   of late participants. Recycling distributes that wealth to future workers.
4. **Sustainability.** Post-emission rewards come entirely from recycle. Without a robust
   recycle flow, block rewards approach zero. With it, they remain meaningful forever.

The canonical positioning: *"Bitcoin is the digital gold chain. Ethereum is the
burn-fees-for-scarcity chain. BTCPC is the No Burn, All Recycle chain — 42 million
tokens, forever, in perpetual circulation, earned by doing real work."*

---

## 5. Storage: BTCPC-FS

### 5.1 Content-Addressed Blob Store

BTCPC-FS is a decentralized, content-addressed file system built on the BTCPC chain.
Files are identified by SHA-256 content identifiers (CIDs). Every CID commitment is
recorded on-chain as a `BLOB_STORE_COMMIT` entry, creating an immutable provenance
trail for every piece of data stored on the network.

The actual bytes live in storage hosts' local stores. The chain holds only the
metadata: CID, size, uploader, committed epoch, expiration epoch, and the list of hosts
that have committed to serving it.

### 5.2 Two-Tier Host Model

Storage hosts self-select into two tiers based on their uptime commitment:

**Active hosts** commit to high uptime (typically >95%). They respond to spot-check
challenges within the required window, maintain rolling storage heartbeats, and serve
blob requests to paying users. Active hosts earn the majority of the storage pool
reward, weighted by committed gigabytes, challenge pass rate, and bandwidth served.

**Cold hosts** commit to archival storage with lower availability guarantees. They
respond to challenges but may have response windows measured in hours rather than
minutes. Cold hosts earn proportionally less, but provide geographic and hardware
diversity that improves overall durability.

### 5.3 Replication and Durability

The file uploader specifies a replication factor at commit time. The protocol tracks
which hosts have committed to each CID. When a host drops below its committed uptime,
the automatic replication engine on active nodes seeks a replacement host and submits
a new `BLOB_HOST_ADD` entry. This is not slashing — the original host simply stops
earning for that CID until they return.

### 5.4 Pay for Delivery, Never Slash for Absence

Storage hosts are paid for delivery: they earn from storage commitments (per-GB over
time) and from bandwidth (per-GB served). They do NOT get slashed simply for going
offline. A host whose home internet goes down stops earning until they return. Their
stake is untouched.

Slashing is reserved for active fraud: a host that passes a challenge (claiming to have
data) but cannot actually serve the data when a user requests it. This is provable
deception, not absence.

### 5.5 Challenge-Response Auditing

Verifiers issue periodic spot-checks against storage hosts. A challenge specifies:

- Host to challenge
- CID to prove
- Byte range to return
- Expected hash of that range

The host must respond within a fixed window with the correct hash. A passed challenge
improves the host's reputation score and increases their reward weight. A failed
challenge reduces their reward share for that epoch and dips their reputation. Multiple
failed challenges in a row trigger a reputation gate that blocks new commitments until
the host recovers.

### 5.6 Storage Fees

Blob storage fees split:

- 90% to storage hosts (pro-rata by commitment share)
- 9% to `btcpc_recycle`
- 1% to reputation bonus pool (highest challenge pass rate hosts)

Bandwidth fees split:

- 95% to the serving host
- 5% to `btcpc_recycle`

---

## 6. Compute Hosting

### 6.1 Stateless Services

Service hosts deploy and run applications on the BTCPC network. A stateless service
is defined by a deployment spec:

- Runtime type: HTTP, TCP, WASM, or static
- Container image or WASM binary CID (from BTCPC-FS)
- Minimum replicas
- Price per session-hour
- Resource requirements (CPU, memory)

Service deployers submit a `SERVICE_DEPLOY` entry. Willing hosts pick up the
deployment and submit a `SERVICE_HOST_REGISTER` entry. Once a minimum quorum of
hosts is running the service, the service is marked active and users can begin sessions.

Hosts submit heartbeats every few epochs to signal liveness. A host that misses
heartbeats stops earning for active sessions on that service. The session escrow
continues running, but the missing host's share of the session payment is withheld
and recycled.

### 6.2 Stateful Services

Stateful services add periodic snapshot persistence. The host serializes the running
service state, uploads it to BTCPC-FS, and records the snapshot CID on-chain via a
`SERVICE_SNAPSHOT` entry. If the host goes offline, a replacement host can download the
latest snapshot and resume the service with minimal data loss.

Snapshot frequency is configurable by the deployer. Higher snapshot frequency means
smaller recovery windows but higher BTCPC-FS storage costs. The deployer sets the
trade-off at deployment time.

### 6.3 Service Fees

Session fees split:

- 90% to the service host
- 9% to `btcpc_recycle`
- 1% to reputation bonus pool

Hosts are not slashed for downtime. They stop earning for the portion of the session
they missed. Their stake is not reduced unless they are found to have submitted
fraudulent session proofs (claiming sessions that did not exist).

---

## 7. IoT Sensor Mesh

### 7.1 LoRa Gateways and Sensor Packets

The BTCPC IoT layer aggregates sensor data from physical devices via LoRa radio
gateways. This is designed around the Helium miner hardware fleet (400,000+ devices
globally) but works with any hardware running a LoRa packet forwarder.

A gateway running `btcpc-nebra` (the BTCPC gateway daemon) listens for inbound LoRa
packets from registered sensors, formats them as Cayenne LPP payloads, and submits
them to the BTCPC chain via `SENSOR_READING` entries signed by the gateway's Posting
key.

### 7.2 On-Chain Sensor Registration

Sensors register on-chain via `SENSOR_REGISTER` entries specifying:

- Sensor type (temperature, humidity, air quality, GPS location, etc.)
- Region and geographic coordinates
- Hardware model
- Reporting interval
- Attached gateway

Registered sensors have a stake of 0.1 BTCPC locked at registration to discourage
spam. Gateways have a stake of 1 BTCPC per registered gateway. Neither stake is ever
slashed for simple absence.

### 7.3 Reading Finalization

Per-epoch readings from all sensors go through median consensus:

1. Multiple gateways (or the same gateway with multiple samples) submit readings
   for a given sensor in the epoch window
2. The finalization process computes the median value across all reports for that
   sensor and epoch
3. Readings more than a configurable deviation band from the median are flagged as
   outliers
4. The finalized median reading is recorded on-chain via a `SENSOR_FINALIZE` entry
5. Finalized reading batches are persisted to BTCPC-FS as blobs, with the CID
   recorded on-chain for subscriber access

### 7.4 IoT Reward Distribution

The IoT pool (10% of epoch reward) splits:

- 60% to sensors: pro-rata by valid readings relayed, weighted by uptime and
  reputation
- 40% to gateways: equal split among gateways with valid heartbeats in the epoch

### 7.5 Helium Miner Reuse

A repurposed Helium Indoor or Outdoor Hotspot running `btcpc-nebra` can simultaneously
participate as:

1. Clock node (clock pool, 5% of rewards)
2. Storage host (storage pool, 12% of rewards, plus direct blob fees)
3. LoRa gateway (IoT pool, 10% of rewards, plus sensor subscription fees)
4. Verifier (verifier pool, 10% of rewards)

Four concurrent income streams on hardware the owner already owns. The `btcpc-nebra`
installer handles LoRa packet forwarder configuration, gateway registration, and
systemd service setup with a single command.

### 7.6 GNSS Base Stations

GNSS (Global Navigation Satellite System) base stations produce high-precision
correction data used by surveying, agriculture, and autonomous vehicles. A base
station running `btcpc-gnss-bridge` polls the device for status and submits
`SENSOR_READING` entries to the chain every 30 seconds.

The same correction stream can simultaneously earn from multiple networks:

- **BTCPC** — IoT pool rewards for every epoch with valid GNSS data
- **GEODNET** — GEOD token rewards for RTK correction contributions
- **RTK Direct** — RTK token rewards for NTRIP correction streaming
- **onocoy** — ONO token rewards for GNSS observation data

The `btcpc-gnss-relay` daemon intercepts RTCM3 correction data from the base
station and forwards copies to all configured NTRIP casters simultaneously.
One antenna, one base station, four income streams.

### 7.7 ADS-B Flight Tracking

A USB ADS-B receiver connected to a gateway (such as a Nebra hotspot) can track
aircraft transponder signals and earn from flight tracking networks like Wingbits.
The gateway forwards ADS-B data to the tracking network while simultaneously
submitting coverage proofs to the BTCPC chain as sensor readings.

### 7.8 Hardware Wallets

BTCPC accounts are derived from standard BIP-39 mnemonics (12 words), making them
natively compatible with hardware wallets:

- **Ledger** — store BTCPC mnemonic on a Ledger device for cold storage. The
  owner key never touches a networked machine. Key rotation and recovery
  operations are signed on the device and submitted via `honemesh.net/rotate`.
- **Flipper Zero** — a portable hardware wallet and mobile sensor node. Sub-GHz
  radio, NFC/RFID, BLE, and GPIO make it capable of both securing keys and
  relaying short-range sensor data in the field.

The mnemonic generates four keypairs (Owner, Active, Posting, Memo). Only the
owner key requires cold storage. Active and posting keys live on the node for
day-to-day signing. The memo key enables end-to-end encrypted inference.

### 7.9 Self-Build Nodes

A Raspberry Pi (or any ARM/x86 single-board computer) running the standard BTCPC
installer becomes a full node capable of all roles: clock, storage, gateway, and
verifier. The minimum viable setup:

- **Raspberry Pi 4/5** (4GB+ RAM) — clock + storage + verifier
- **Pi + LoRa HAT** — add gateway role for IoT sensor relay
- **Pi + USB GNSS receiver** — native GNSS corrections without ARP spoofing
- **Pi + USB ADS-B dongle** — flight tracking via Wingbits
- **Pi + external SSD** — high-capacity BTCPC-FS storage host

The `install.sh` one-liner handles Node.js, systemd services, and account setup.
No Docker required. Self-build nodes earn the same rewards as commercial hardware.

### 7.10 Supported Sensor Types

The BTCPC sensor registry accepts any hardware that can produce a signed reading.
Currently supported sensor types:

| Type | Example Hardware | Data Produced |
|------|-----------------|---------------|
| temperature | SenseCAP, LoRa T/H sensors | Celsius readings |
| humidity | SenseCAP, DHT22 | Relative humidity % |
| air_quality | PurpleAir, SenseCAP AQI | PM2.5, PM10, AQI index |
| gps | Hyfix, u-blox GNSS receivers | RTK corrections, position |
| soil | LoRa soil moisture probes | Volumetric water content |
| uwb_position | Hyfix UWB anchors | Indoor positioning coordinates |
| uwb_range | Hyfix UWB tags | Distance measurements |
| motion | Accelerometers, dashcams | Movement vectors, imagery CIDs |
| power | Smart plugs, solar monitors | Watts, kWh, grid feed-in |
| noise | Sound level meters | Decibel readings |
| light | Lux sensors | Illuminance |
| pressure | Barometers, weather stations | Atmospheric pressure hPa |
| seismic | Raspberry Shake, MEMS accelerometers | Ground motion, early warning |
| weather | WeatherXM, Davis, Ambient Weather | Multi-parameter weather bundles |
| water | pH/TDS probes | Water quality metrics |
| traffic | Camera counters, radar | Vehicle/pedestrian counts |
| custom | Any device with a serial output | User-defined payload |

New sensor types are added by registering with `type: "custom"` and a payload
schema. No protocol upgrade required. Any hardware that can reach a gateway
(via LoRa, WiFi, Bluetooth, USB, or serial) can earn from the IoT pool.

### 7.11 Bandwidth and Relay Services

Beyond physical sensors, nodes can earn by providing network infrastructure:

- **VPN exit nodes** — route traffic for subscribers, earn per-GB via escrow
- **CDN edge caching** — serve BTCPC-FS blobs from local storage, earn per-request
- **WiFi hotspot sharing** — metered bandwidth access, paid per-session
- **Mesh relay** — forward LoRa or Meshtastic packets between nodes that cannot
  reach each other directly, earn relay fees from the IoT pool

These services use the same stake-escrow-reputation primitive as all other BTCPC
services. The protocol does not prescribe which hardware or service to run. If a
node can prove it did useful work, it earns.

### 7.12 Device Roadmap

The current Raspberry Pi gateway already supports USB, Serial, I2C, SPI, and GPIO
sensors with no redesign needed. The following hardware is prioritized across seven
deployment phases:

| Phase | Device | Interface | Price Range | Purpose |
|-------|--------|-----------|-------------|---------|
| **1 — Immediate** | RTL-SDR v4 dongle | USB | $30–40 | Software-defined radio receiver |
| | ADS-B 1090 MHz antenna | SMA → RTL-SDR | $15–25 | Air traffic surveillance data |
| **2 — Environmental baseline** | BME280 | I2C / SPI | $3–8 | Temperature, humidity, barometric pressure |
| | Pimoroni Enviro | I2C + onboard MCU | $50–70 | All-in-one weather station (temp, humidity, pressure, light, noise) |
| **3 — Public health** | PMS5003 | Serial (UART) | $15–25 | PM1.0 / PM2.5 / PM10 particulate matter |
| | MH-Z19B | Serial (UART) | $18–25 | NDIR CO₂ concentration (0–5000 ppm) |
| | SGP30 | I2C | $10–15 | Total VOC and eCO₂ indoor air quality |
| **4 — Structural** | ADXL345 | I2C / SPI | $3–8 | 3-axis accelerometer for vibration monitoring |
| | Grove D7S | I2C | $35–45 | Earthquake / structural vibration detection |
| | Raspberry Shake 1D | USB (geophone) | $300–400 | Seismograph-grade ground motion |
| **5 — Water / soil** | JSN-SR04T ultrasonic | GPIO (trigger/echo) | $5–10 | Water level in tanks, rivers, flood zones |
| | Capacitive soil moisture | Analog / I2C (ADC) | $3–6 | Soil moisture for agriculture |
| | Flood detection float switch | GPIO | $2–5 | Binary flood / no-flood alert |
| **6 — Energy** | INA219 | I2C | $3–8 | DC current and voltage monitoring |
| | SCT-013 current clamp | Analog (ADC) | $8–12 | Non-invasive AC current measurement |
| | PZEM-004T | Serial (UART) | $10–15 | AC voltage, current, power, energy metering |
| **7 — Expansion** | ESP32 dev board | WiFi / BLE → gateway | $5–12 | Remote sensor nodes without Pi per location |
| | LoRa SX1276 / RFM95W module | SPI | $8–15 | Long-range (2–15 km) low-power sensor links |
| | GPS NEO-6M / NEO-M8N | Serial (UART) | $8–15 | Geolocation tagging for mobile or field nodes |

Each sensor submits readings through the same on-chain registration and median
consensus pipeline described in Sections 7.2–7.4. Phase numbers indicate suggested
deployment order, not hard dependencies — operators can skip ahead to any phase
their use case requires.

### 7.13 Mobile Sensor Arrays (Flipper Zero)

Fixed sensor networks cover one location each. A Flipper Zero in someone's pocket
covers every location that person walks through. Thousands of Flippers moving through
cities generate real-time maps of RF spectrum, crowd density, weather, and NFC coverage
that update every time someone walks by. No fixed sensor network can produce this data.

**The device.** Flipper Zero is a pocket-sized, open-source multi-tool with Sub-GHz RF
transceiver, BLE 5.0, NFC, infrared, 1-Wire, and a GPIO header for expansion. Out of
the box it performs:

- **Sub-GHz RF spectrum mapping** — scans 300–928 MHz, logs signal strength by frequency
  and GPS coordinate. Detects active transmitters, interference sources, dead zones.
- **BLE device density tracking** — passive BLE scan counts unique devices per sweep,
  producing crowd density estimates without identifying individuals.
- **NFC field detection** — detects active NFC readers (payment terminals, transit gates,
  access control). Maps contactless payment infrastructure coverage.
- **Temperature** — onboard sensor, plus GPIO-extensible to BME280 or any I2C/SPI probe
  for humidity and barometric pressure.
- **GPIO expansion** — the same I2C/SPI/UART sensors listed in the Phase 1–7 device
  roadmap (Section 7.12) can be wired directly to the Flipper's GPIO header.

**Data buyers and market value.** The data a mobile sensor mesh produces has direct
commercial value across multiple industries:

| Buyer | Data Product | Market Size |
|-------|-------------|-------------|
| **Telecoms** | RF coverage maps, interference detection, dead-zone identification | $B/year — carriers pay for independent coverage verification |
| **Retailers** | Foot traffic analytics, crowd density by time and location | $10B+ market — currently served by camera systems and WiFi probes |
| **Fintech** | Contactless payment terminal coverage mapping | Terminal operators and payment networks need coverage data |
| **Weather / Environmental** | Hyperlocal temperature grids, urban heat island detection | Complements fixed weather stations with street-level resolution |
| **Security** | Rogue transmitter detection, spectrum compliance monitoring | Regulatory bodies and enterprise security teams |
| **Urban planning** | Pedestrian flow patterns, neighborhood activity scoring | Municipal governments and real estate developers |

**How it works.** The Flipper scans continuously during normal carry. Readings are
buffered to microSD with GPS coordinates and Unix timestamps. When the user connects
to a BTCPC gateway (via USB or BLE), buffered readings sync to the chain. Each reading
is signed with the user's memo key for attestation — the same key hierarchy described
in Section 10 (Security). Gateway nodes that relay Flipper readings co-sign the
submission, providing a second attestation layer.

**Earning.** Flipper readings enter the IoT reward pool (10% of epoch emissions).
Rewards per reading are weighted by:

1. **Gateway attestation** — readings relayed through a registered gateway with a
   co-signature earn full weight. Self-submitted readings earn reduced weight until
   cross-validated by nearby sensors.
2. **Statistical consistency** — median consensus (Section 7.3) applies. Readings that
   deviate significantly from the median of nearby sensors are downweighted. A Flipper
   that reports 40°C in a city where every other sensor reads 22°C earns nothing for
   that reading.
3. **Coverage novelty** — readings from areas with sparse existing coverage earn a
   bonus multiplier. Walking through an unmapped neighborhood is worth more than
   re-scanning a location with 50 existing data points.

**The mobile mesh advantage.** A Nebra or Hyfix gateway covers one rooftop. A Raspberry
Pi covers one room. A Flipper Zero covers every street, subway car, shopping mall, and
park bench its carrier walks through. The network effect is pedestrian-powered: the more
people carry Flippers, the higher the spatial and temporal resolution of every data
product the chain can sell.

### 7.14 Earn Where You Are

BTCPC does not require specialized hardware or a data center. The design principle
is: **earn where you are, not where we want you to be.**

- A phone in your pocket earns from GPS, motion, and orientation sensors while you
  walk. Open the browser, tap "Start Earning," and your device joins the network.
- A laptop on your desk earns from AI inference while you work. The browser-based
  miner uses WebGPU — no installation, no CLI, no Docker. Open a tab and mine.
- A Raspberry Pi on your shelf earns from clock, storage, and gateway roles 24/7.
  One-command install, set and forget.
- A Flipper Zero in your bag earns from Sub-GHz, BLE, and NFC scanning everywhere
  you go. Sync when you get home.

The browser PWA at honemesh.net/app is the easiest onramp: zero installation, works on
any device, earns from three pools simultaneously (clock 5%, sensors 10%, mining 55%).
For users who want maximum earnings, the CLI provides full GPU access, larger models,
and always-on operation.

Every device contributes what it can. A phone's GPS reading is as valuable to the
network as a GPU's inference result — both are real work, both earn BTCPC. The
protocol does not privilege one type of hardware over another. It rewards useful work,
wherever and however it happens.

---

---

## 8. Scientific Compute

### 8.1 Scientific Compute as a First-Class Workload

BTCPC supports long-running distributed inference jobs targeting domains where
computation directly generates scientific value: protein structure prediction,
small-molecule drug discovery, genomic variant analysis, and climate simulation.
These workloads are structurally different from real-time chat inference. They
are batch-oriented, tolerant of high latency, and produce outputs that can be
permanently archived and attributed.

The protocol treats scientific compute as a distinct latency class. Jobs in this
class are queued, not streamed. There is no time-out for a slow node — a
shard group working on a week-long folding run is as legitimate as one completing
a sub-second chat inference. Fee and reward accounting reflect this: the unit of
value is `tokens × shard_param_count`, not response time.

Any Ollama-compatible model may serve scientific jobs. Shard groups allow models
whose parameter count exceeds any single machine's VRAM to be assembled across
multiple nodes. A 70B protein-folding model, for example, can be split across
four machines each holding 17.5B parameters; each machine earns proportionally
to the layers it holds.

### 8.2 Distributed Shard Pipeline for Large Models

Layer splitting follows the same mechanics described in §2 (Consensus). For
scientific workloads the runtime difference is that the pipeline may persist for
hours or days rather than seconds. Each node holds a contiguous block of
transformer layers; activation tensors are passed forward between nodes after
each layer block completes.

**Route optimization.** Nodes measure round-trip latency to their peers and
broadcast `NODE_LATENCY` messages. The shard registry uses these measurements
to order the pipeline so that each activation handoff travels the lowest-latency
hop available. For the scientific latency class, latency tolerance is inherently
high — a 10ms vs 100ms hop difference is irrelevant at multi-hour job scales —
which means global node participation is practical. A node in São Paulo can sit
in the middle of a shard group whose other nodes are in Frankfurt and Seoul
without meaningfully slowing the job.

**Work value.** Each shard node earns based on the layers it holds:

```
work_value(node) = tokens_processed × shard_param_count(node)
```

A node holding 35B of a 70B model earns exactly half the base work value of
the full-model case. Nodes with no shard assignment earn nothing from that job.

### 8.3 Open Science Discount

Requesters who designate their results as open-source — permanently stored on
the BTCPC chain, owned by no one, readable by anyone — receive a **40% reduction
in fees**. This is not a subsidy paid by the protocol treasury. It is funded from
`btcpc_recycle`: the discount lowers the requester's payment, while miners who
process open-science jobs earn a **25% bonus** on top of their standard work
value. The spread is covered by the recycled fee pool.

The economic effect: open science is cheaper to submit than proprietary science,
and more profitable to mine. The network is financially incentivized to prioritize
open research.

Results are stored permanently on-chain as `SCIENTIFIC_RESULT` ledger entries.
The entry records the job title, type, model, requester account, input hash, and
result hash. For large results — defined as more than 50 KB of raw output — the
result bytes are stored in BTCPC-FS and the on-chain entry holds the
content-addressed CID. The chain itself holds the proof of existence and
attribution; the blob store holds the bytes.

### 8.4 On-Chain Scientific Record

Every completed open-source job inscribes a permanent record with the following
fields:

- **job_id** — deterministic hex identifier
- **title** — human-readable description supplied by the requester
- **type** — job domain (see §8.5)
- **model** — Ollama model name and parameter count at time of execution
- **requester** — BTCPC account name permanently linked to the discovery
- **input_hash** — SHA-256 of the input data (sequence, SMILES string, grid
  parameters, or other domain-specific encoding)
- **result_hash** — SHA-256 of the result bytes, regardless of storage location
- **result_blob_cid** — BTCPC-FS CID if the result exceeded the inline threshold
- **epoch** — block at which the result was finalized

Records are immutable once written. No operator, validator, or governance vote
can delete or alter a `SCIENTIFIC_RESULT` entry. The BTCPC chain does not
interpret the science — it timestamps and attributes it. Whether the result is
a correct protein fold or a false lead in drug discovery is outside the
protocol's scope; the chain only guarantees that the computation happened,
who paid for it, and what came out.

### 8.5 Job Types

**protein_folding** — Input is an amino acid sequence (FASTA or one-letter code).
Output is a predicted 3-D structure, typically encoded as a PDB file or coordinate
tensor. Protein structure determines biological function; knowing the fold of a
novel variant enables targeted drug design and viral escape prediction.

**drug_discovery** — Input is a SMILES string or molecular graph encoding a
candidate small molecule. Output includes binding affinity predictions, ADMET
property estimates (absorption, distribution, metabolism, excretion, toxicity),
and selectivity scores against a target protein. A single drug candidate screen
may involve millions of molecules; BTCPC makes it practical to distribute those
screens across thousands of independent nodes.

**climate_modeling** — Input is a grid of initial atmospheric or oceanic state
variables. Output is a forward simulation of that state over a specified time
horizon. Climate models are among the most computationally intensive scientific
workloads in existence; distributing them across heterogeneous hardware requires
a coordination layer that tolerates node failures and partial results, which the
BTCPC shard pipeline provides.

**genomics** — Input is a raw sequencing read set or variant call file. Output
includes alignment, variant annotation, population stratification, or expression
quantification depending on the pipeline. Genomic workloads are highly
parallelizable and have clear value: a single exome interpretation can change a
diagnosis for a patient with a rare disease.

**materials_science** — Input is an atomic structure specification (CIF, POSCAR,
or equivalent). Output includes density functional theory energy calculations,
phonon dispersion curves, or defect formation energy landscapes. Materials
discovery underlies batteries, solar cells, semiconductors, and catalysts.

**general** — Any long-running inference job that does not fit the above
categories. The protocol does not restrict scientific compute to these domains;
`general` is a catch-all for novel workloads. Work value and economics are
identical to domain-specific types.

### 8.6 Economics

**Standard job (closed results).** The requester pays the full quoted fee. The
fee is distributed to shard nodes proportional to their work value
(`tokens × shard_param_count`). No discount applies; results belong to the
requester and are not stored on chain unless the requester separately calls
`recordScientificResult`.

**Open-science job.** The requester pays 60% of the quoted fee (40% discount).
Nodes earn 125% of their standard work value (25% bonus). The differential is
funded by `btcpc_recycle` — consistent with the protocol rule that fees never
burn, they recirculate. Results are written permanently to the chain as described
in §8.4.

**Fee denomination.** All fees are quoted and paid in BTCPC. Current price
discovery happens through the DEX bridge (§10). There is no minimum job size
and no minimum node count — a single CPU machine can hold one or two transformer
layers of a small model and earn proportionally.

**No floors, no ceilings.** A requester who sets `max_fee = 0` will not attract
nodes unless the job is open-source and the recycled pool makes the bonus
sufficient. Market-clearing fees emerge naturally: nodes that find the bonus
insufficient simply do not claim the job, and the requester must raise the fee.
This is the same mechanism as the broader inference market.

## 9. Oracle Feeds

### 9.1 Generic Off-Chain Data Ingestion

The oracle layer extends the sensor reading pipeline to any off-chain data source,
not just physical hardware. Price feeds, weather data, sports scores, and any other
API-sourced data can be submitted as oracle readings.

Oracle reporters are permissionless: any node that registers as an oracle provider
and maintains the minimum stake can submit readings for any registered feed.

### 9.2 Median Consensus

Oracle feeds use the same median consensus mechanism as sensor readings:

1. Multiple reporters submit values for a given feed and epoch
2. The finalization process computes the median across all reporters
3. Reporters whose values deviate more than `max_bps` from the median get a
   reputation dip (no slashing — reputation is the enforcement mechanism)
4. The finalized median is recorded on-chain

### 9.3 Reputation-Based Quality Control

Repeated outlier submissions drive down a reporter's reputation score below the
minimum threshold, after which their submissions are ignored until they recover.
This creates natural Sybil resistance without slashing: a single reporter cannot
move the median by flooding the system with fake readings, because median is
inherently resistant to outliers, and outlier reporters lose their oracle access
via reputation decay.

---

## 10. Cross-Chain Bridge

### 10.1 Lock-and-Recycle, Not Burn-and-Mint

The BTCPC bridge connects native BTCPC to wrapped wBTCPC on destination chains
(Base, Arbitrum, Ethereum, Bitcoin) via a **lock-and-recycle** mechanism. There
is no mint function and no burn function. The bridge contract on each destination
chain holds a fixed pre-minted reserve of 4,200,000 wBTCPC (10% of native supply).
All wrap and unwrap operations are plain ERC-20 transfers between user wallets and
the bridge reserve address.

**Wrap flow (BTCPC → wBTCPC):**
1. User locks BTCPC in the source-chain bridge contract via `lockForWrap(amount, destChainId)`
2. Bridge relay detects the lock event
3. Destination-chain bridge contract transfers wBTCPC from the reserve to the user
4. No minting occurs

**Unwrap flow (wBTCPC → BTCPC):**
1. User transfers wBTCPC from their wallet back to the bridge reserve address
2. Bridge relay detects the transfer
3. Source-chain bridge contract releases the locked BTCPC back to the user
4. No burning occurs

### 10.2 Supply Cap per Chain

Each destination chain has a hard cap of **4,200,000 wBTCPC** pre-minted in the
contract constructor. This is the only mint operation that ever occurs. The contract
has no `mint()` function, no `burn()` function, no admin key, no upgrade proxy, and
no pause mechanism. It is immutable from deployment.

The maximum wBTCPC circulating on any destination chain at any given time equals the
cumulative BTCPC native locked into the source bridge for that destination, capped at
4,200,000. This maintains a strict 1:1 backing relationship.

### 10.3 Bridge Liquidity: Permissionless LPs

The bridge reserve is funded by permissionless LPs who lock BTCPC native into the
source-chain contract with a variable time commitment (30 to 1,460 days). Funders
earn a pro-rata share of all bridge fees proportional to their lock weight:

```
LP weight = amount × remaining_lock_days
```

This is the same veCRV-style commitment weighting used by Curve Finance. Weight
decreases naturally as the lock approaches expiration, preventing permanent
concentration. The LP roster rotates organically as locks expire and new LPs enter.

### 10.4 Bridge Fees

| Direction | Volume | Fee |
|-----------|--------|-----|
| Wrap (any size) | All | 0.05% |
| Unwrap | < 1,000 BTCPC | 0.20% |
| Unwrap | 1,000–100,000 BTCPC | 0.15% |
| Unwrap | > 100,000 BTCPC | 0.10% |

Wrap fees are charged in BTCPC. Unwrap fees are charged in wBTCPC. Fees are
distributed to active LPs pro-rata by current weight. The asymmetric
wrap/unwrap fee structure creates a bias toward keeping wBTCPC circulating on
destination chains rather than constant round-trips.

### 10.5 Withdrawal Queue

After a lock period expires, an LP requests withdrawal and enters a FIFO queue.
Position 1 in the queue is funded by:

- Incoming unwrap volume (the primary path — organic outflows fund incoming withdrawals)
- A smoothing buffer funded by 10–20% of bridge fees, capped at ~10% of total
  locked liquidity

LPs in queue continue earning fees until their position is filled. This hybrid
queue model prevents stalls under normal operation and avoids the ponzi-adjacent
dynamics of pure auto-redemption schemes.

### 10.6 Destination Chains

| Chain | wBTCPC Supply | Contract Type |
|-------|--------------|---------------|
| Base | 4,200,000 | Immutable ERC-20 |
| Arbitrum | 4,200,000 | Immutable ERC-20 |
| Ethereum | 4,200,000 | Immutable ERC-20 |
| Bitcoin | 4,200,000 | OP_RETURN anchor + off-chain relay |

---

## 11. Four-Tier Finality

### 11.1 Architecture

BTCPC state is anchored to external chains for independent verification. Anchoring
is additive — BTCPC's consensus continues working even if every external anchor
chain is offline. Anchors provide verifiability, not dependency.

| Tier | Chain | Cadence | Purpose |
|------|-------|---------|---------|
| Native | BTCPC | Every epoch (30s) | Working consensus |
| L2 | Base, Arbitrum | Every 100 epochs (~50 min) | Fast cross-chain verification |
| Mainnet | Ethereum | Every 1,000 epochs (~8.3 hrs) | Deep DeFi finality |
| Deep Seal | Bitcoin | Every 10,000 epochs (~3.5 days) | Ultimate permanence |

Each tier is roughly 10x rarer than the one below it. Tier 2 is cheap enough to run
continuously. Tier 4 (the Bitcoin Deep Seal) is meaningful because it anchors the
chain's state root into the most censorship-resistant and final ledger in existence.

### 11.2 Demand-Driven Submission

Anchor submitters are permissionless. Any node can submit a state root to the finality
contracts and collect the anchor reward from the accumulated `anchor_reserve`. Bridge
operations pay a small fee into the reserve; the reserve funds anchor submission.

When the reserve is empty and bridge volume is low, anchors may be skipped. The chain
continues working. When bridge volume spikes, the reserve fills quickly and anchors
resume at full cadence.

### 11.3 Merkle Batching

Each anchor write commits a Merkle root over multiple epoch anchors (typically 100).
Bridges and clients verify specific epochs via off-chain Merkle proofs. This reduces
gas costs by approximately 98% compared to one-epoch-per-on-chain-write.

Tier 3 (Ethereum) anchors use EIP-4844 blob storage: 1,000 epoch anchors fit into a
single 128 KB blob, costing approximately $1–3 per anchor batch. Blob data is
available for 18 days on Ethereum; permanent storage is provided by BTCPC-FS mirror
nodes that store the full anchor history as on-chain blobs.

### 11.4 Bitcoin Deep Seal

Bitcoin anchoring uses OP_RETURN to inscribe the BTCPC state root hash (~32 bytes)
into a Bitcoin transaction. The rich anchor blob (account counts, epoch statistics,
miner records) lives in BTCPC-FS at a CID referenced by the OP_RETURN data —
dogfooding the chain's own storage layer.

Each Deep Seal mints a Soulbound NFT to the `btcpc_genesis_seals` system account as
a publicly browsable historical artifact. These are the chain's milestones.

### 11.5 Cost Baseline

Annual anchoring cost at current gas prices with all optimizations:

| Tier | Chain | Annual Cost |
|------|-------|-------------|
| L2 | Base + Arbitrum | ~$1 (demand-driven, mostly bridge-funded) |
| Mainnet | Ethereum (blob) | ~$12 |
| Deep Seal | Bitcoin (OP_RETURN) | ~$10 |
| **Total** | | **~$25/year** |

Bridge users pay proportional fees on top of this via the demand-driven path. The
treasury backstop is minimal by design — the principle is "the cheapest dollar is the
one you don't spend."

---

## 12. State Management

### 12.1 Blockchain as Source of Truth

Block files on disk are the canonical source of truth. The full chain state can always
be reconstructed by replaying all blocks from genesis. No external database is
required for chain integrity. MongoDB (if running) is a convenience cache for
application-layer queries only.

Block file format: `data/blocks/block-NNNNNNNN.bin`
Finality snapshots: `data/blocks/finality-NNNNNNNN.bin`

File layout:
```
[180 bytes header] [4 bytes payload length] [JSON payload]
```

Header fields (180 bytes binary):
```
version                    (uint32)
previous_block_hash        (32 bytes)
merkle_root_transactions   (32 bytes)
merkle_root_compute_proofs (32 bytes)
state_root                 (32 bytes)
timestamp                  (uint64)
epoch_number               (uint32)
difficulty                 (uint32)
miner_id                   (32 bytes)
```

### 12.2 In-Memory stateStore

The stateStore module is an in-memory cache of the current chain state, rebuilt from
block files on startup via `replayFromDisk()`. All reads by controllers, routes, the
miner, and the explorer go through stateStore. Balance queries are O(1) Map lookups,
not O(N) database aggregations.

Mutation is always via `applyEntry(entry)`. Entries are the universal state-change
event type — the same shape whether they come from replay, from local ledger recording,
or from P2P gossip sync. Determinism is guaranteed: same sequence of entries produces
the same state.

### 12.3 Sparse Merkle Tree

The stateManager maintains a Sparse Merkle Tree (SMT) that tracks account state. The
SMT root is the `state_root` recorded in every block header. Two nodes that have
processed the same entries will have identical SMT roots. This root is used for:

- Verifying block integrity (expected root vs. computed root)
- Finality snapshot integrity checks
- Cross-chain anchor submissions

### 12.4 Finality Snapshots and Fast Sync

Every N epochs, the state is compacted into a finality snapshot: a full serialization
of all accounts, balances, and essential metadata at that epoch. New nodes can:

1. Connect to any peer
2. Request the latest finality snapshot
3. Verify the Merkle root against the block header
4. Start participating immediately without replaying the full history

Old blocks remain available for historical queries but are not required for consensus
participation. This design allows the chain to grow indefinitely while keeping join
time bounded.

### 12.5 Cross-Process Ledger Queue

In multi-process deployments (e.g., API server + miner + P2P node as separate processes),
ledger entries are shared via `data/pending-entries.jsonl`. Each process appends its
entries to this file. The miner flushes both in-memory and on-disk pending entries when
building a block. This queue is wiped after each successful block write.

---

## 13. Tokenomics Summary

### 13.1 Native Token

| Property | Value |
|----------|-------|
| Symbol | BTCPC |
| Total supply | 42,000,000 (fixed forever) |
| Base unit | Dream (1 BTCPC = 10^10 dreams) |
| Decimals | 10 |
| Genesis reward | 24.306 BTCPC/epoch (30-second epochs) |
| Halving cadence | Every ~4 years |
| Emission timeline | ~3 years to exhaust 42M cap |
| Post-emission rewards | From `btcpc_recycle` + fee market |

### 13.2 User-Created Tokens

BTCPC enforces a chain-wide token standard for all user-created tokens:

- Fixed supply at creation: 42,000,000 max (matching BTCPC)
- 10 decimal places (same as BTCPC)
- No in-protocol mint after creation
- No in-protocol burn

Token creation fees route entirely to `btcpc_recycle`:

| Tier | Max Supply | Fee |
|------|-----------|-----|
| Micro | ≤ 1,000,000 | 21 BTCPC |
| Standard | ≤ 42,000,000 | 42 BTCPC |
| Mega | ≤ 1,000,000,000 | 84 BTCPC |
| Custom | Unbounded | 168 BTCPC |

NFT collection creation fee: 10 BTCPC → `btcpc_recycle`.

### 13.3 Commerce Platform Fees

Order platform fee (1% of order total):

- 0.5% → `btcpc_recycle`
- 0.4% → store stakers (pro-rata by stake)
- 0.1% → reputation bonus pool

Store opening (bonding curve, paid in wrapped stables):

- 50% → `btcpc_recycle`
- 50% → `btcpc_treasury` (protocol development fund)

### 13.4 Stake Requirements

| Role | Minimum Stake | Slashable Condition |
|------|--------------|---------------------|
| Mining node | 100 BTCPC | Forged inference, failed challenges |
| Verifier | 100 BTCPC | Voting against consensus |
| Clock node | 10 BTCPC | Extended missed heartbeats |
| Store operator | 1 BTCPC/slot | Fraud, non-fulfillment |
| Storage host | 1 BTCPC/GB | Active deception on challenges |
| Service host | 10 BTCPC/CPU-epoch | Fraudulent session proofs |
| LoRa gateway | 1 BTCPC | Fraudulent readings |

Stakes are collateral, not consumed. All slashing routes 50% to honest participants
and 50% to `btcpc_recycle`. No stake is ever burned.

The 14-epoch unbonding period (~7 minutes at 30-second epochs) applies to all
stake withdrawals.

---

## 14. Privacy Roadmap

BTCPC's current architecture is fully transparent: all transactions, balances, and
account activity are visible on-chain. The following privacy capabilities are planned
for future releases:

**Encrypted Inference** *(Coming Soon)*
Inference prompts and responses encrypted at the API layer using memo key ECDH key
agreement (secp256k1 curve) plus AES-256-GCM. The user's memo public key is on-chain;
the prompt is encrypted client-side before submission. The miner decrypts using the
shared secret, runs inference, and returns the encrypted result. No prompt or result
passes through any node in plaintext.

**Stealth Accounts** *(Planned)*
One-time payment addresses derived from the recipient's public key, allowing payments
that cannot be linked to the recipient's main account by external observers.

**Anonymous Routing** *(Planned)*
A relay mixer that allows transactions to be submitted through a chain of relay nodes,
obscuring the originating IP address from the network.

**Tor/I2P Node Privacy** *(Planned)*
An opt-in mode for node operators who wish to participate in consensus without
exposing their IP address.

---

## 15. Governance

### 15.1 Genesis Phase

BTCPC is currently in its genesis phase. Founder-operated by Shin Devlin. Protocol
parameters (epoch duration, block cap bounds, reward pool splits, fee rates) are set
by the genesis operator. The genesis phase exists to allow fast iteration on the
technical foundations before the user base grows large enough to make governance
meaningful.

The genesis operator can change parameters. The genesis operator cannot change the
42,000,000 supply cap, cannot implement a burn mechanism, and cannot alter the
fundamental "no pre-mine, no founder allocation" structure — these are architectural
constants, not governance parameters.

### 15.2 Progressive Decentralization

The path from genesis to full decentralization:

1. **Genesis phase** (now): founder-operated, fast parameter iteration
2. **Multi-sig phase** (next): protocol changes require N-of-M signatures from
   recognized chain participants
3. **Governance phase** (future): on-chain proposals with stake-weighted voting,
   time-locked execution, and veto windows

Parameter changes will eventually require:
- On-chain proposal submitted by any account meeting a minimum stake threshold
- 7-day discussion period
- Stake-weighted vote (simple majority or supermajority depending on parameter type)
- 14-epoch time-lock before execution (to allow last-chance vetoes)

### 15.3 The Design Goal

BTCPC was not built to be run. It was built to let go of. Every architectural decision
— permissionless clocks, demand-driven anchoring, no admin keys on bridge contracts,
permissionless LP, oracle feeds, sensor registration — follows this principle. The
chain should be able to operate without its creators.

*"We didn't build BTCPC to run it. We built it to let go of it."*

---

## 16. Conclusion

The blockchain industry spent its first decade asking "what can we put on a chain?"
The answer was mostly: tokens, speculation, and games.

Freeport Protocol asks a different question: **what work needs to happen in the world,
and how do we pay the machines doing it — without asking permission?**

AI inference needs to happen. Files need to be stored. Sensor data needs to be
collected and verified. Applications need to be served. Epoch timing needs to be
maintained. Goods need to be bought and sold. All of these are real economic activities
with real market demand. None of them require a corporation to intermediate between the
person who needs the work done and the machine that does it.

Freeport Protocol removes the corporation. The chain is the intermediary. The miners,
storage hosts, clock nodes, service hosts, and gateway operators are the workers. The
users are the customers. The token is the payment. The marketplace is on-chain.

A freeport, historically, is a port exempt from customs duties — a place where goods
flow without the friction of gatekeepers. That is the precise design goal here: compute,
storage, and commerce that flows between sovereign participants without tariffs, without
platform rent, and without the ability of any single party to deplatform another.

Every token earned. Every machine welcome. Every trade sovereign.

---

---

## Appendix M: Decentralized Commerce Layer

### M.1 Architecture

All commerce state — stores, products, orders, shipping accounts, reputation votes — flows
through the same append-only ledger as every other chain entry. Every BTCPC node that
replays the ledger from genesis holds a complete, verifiable copy of the market. Commerce
state is not a separate database: it is a projection of ledger entries, rebuilt deterministically
from blocks on startup, identical across all nodes that have processed the same chain.

No central marketplace server is required. The catalog is distributed across all nodes.
Reads are served locally from each node's in-memory state store, derived from ledger replay.
Because the data is on-chain, catalog responses from any node are independently verifiable
against the Merkle root in the corresponding block header.

`btcpc-market` is a Rust service (port 7042) that vendors run as an optional sidecar for
full seller operations: order management, carrier label generation, flash sale scheduling,
and Tor hidden service registration. Read-only catalog access — browsing stores and
products — requires only a standard BTCPC node. Buyers do not need to run `btcpc-market`.

### M.2 Public Access Without a Node

The store frontend (`website/store.html`) is a static HTML file with no server-side
dependencies. It can be served from any web host, IPFS gateway, or BTCPC-FS CID. The
same file serves as the vendor control panel (`website/vendor.html`) when accessed with
a signing key.

`API_BASE` is configurable at runtime:

- Default: same-origin (for operators running a local BTCPC node on port 6942)
- Override via `?node=https://node.example.com` query parameter
- Override persisted in `localStorage` for returning users

A user without a local node points at any public BTCPC gateway. The gateway serves the
catalog from its local ledger. Because catalog data is verifiable on-chain (hashes are
committed to block headers), the user does not need to trust the gateway's responses —
any discrepancy is detectable against the public chain state. Gateway nodes have no
privileged access to buyer data: they relay catalog reads and route order placements but
cannot modify ledger entries.

### M.3 Vendor Privacy

**Catalog reads.** Product and store listings are served by any node from its local ledger
replay. The vendor's server IP is not involved in catalog delivery. A vendor does not need
to be online for buyers to browse their listings.

**Order placement.** Order transactions are P2P ledger entries signed by the buyer and
broadcast to the network. The vendor's `btcpc-market` instance receives the order by
monitoring for `ORDER_PLACE` entries addressed to their store. Optionally, the vendor
registers a Tor hidden service (`.onion` address) on-chain via a `STORE_UPDATE` entry.
Buyers running Tor Browser detect the `.onion` address and route order communication
through it automatically, without the buyer or vendor exposing their IP to each other.

**Shipping accounts.** Carrier credentials (UPS, FedEx, USPS, DHL account numbers and
API keys) are stored on-chain under the store record. The API masks account numbers in
responses — only the last four characters are visible in any JSON output. The full value
lives in the ledger, readable only by the store's Active key. At order fulfillment, the
`btcpc-market` service decrypts the shipping account and auto-populates the carrier
dropdown, eliminating manual credential entry per shipment.

**Blob delivery.** Digital products reference a BTCPC-FS content identifier (`delivery_cid`).
The blob is content-addressed: its SHA-256 hash is both its identifier and its integrity
proof. Multiple storage hosts replicate each CID. The vendor's server IP is not in the
download path once the blob has been replicated to other storage hosts. Buyers download
directly from storage hosts, not from the vendor.

### M.4 Escrow Mechanics

Order escrow is locked at `ORDER_PLACE`. The buyer's funds are held by the protocol —
not by the vendor, not by a third party — until the order resolves.

**Auto-deliver.** Digital products with a `delivery_cid` set in the product record fulfill
instantly upon order placement. The protocol writes an `ORDER_FULFILL` entry in the same
block that contains the `ORDER_PLACE`. No seller action is required. The buyer receives
the BTCPC-FS CID immediately. Escrow releases to the vendor in the same block.

**Manual fulfill.** Physical goods and services require the vendor to ship and record a
tracking number via `ORDER_FULFILL`. The seller has 4,800 epochs (~40 hours at 30-second
epochs) to fulfill after order placement. Unfulfilled orders auto-cancel after this
deadline: the protocol writes an `ORDER_CANCEL` entry and returns escrow to the buyer.

**Buyer confirmation.** On receipt of physical goods, the buyer submits `ORDER_DELIVER`.
Escrow releases to the vendor minus the 1% platform fee (0.5% to `btcpc_recycle`,
0.4% to store stakers pro-rata, 0.1% to reputation bonus pool).

**Disputes.** Either party may open a dispute before `ORDER_DELIVER` is submitted by
writing an `ORDER_DISPUTE` entry. Dispute locks escrow pending governance resolution.
The dispute panel is selected from registered verifiers. The winning party receives
the escrowed amount; the losing party's stake is reduced proportionally. All resolved
escrow flows to the appropriate party — nothing is burned.

### M.5 Commerce Ledger Entry Types

The following entry types constitute the complete on-chain surface area of the commerce
layer. All entries are signed by the appropriate key (Active key for financial operations,
Posting key for catalog operations and reputation votes) and subject to the same block
inclusion, ordering, and Merkle commitment rules as any other ledger entry.

| Entry Type | Signer | Description |
|---|---|---|
| `STORE_OPEN` | Active | Register a new store. Bonding curve fee applies. |
| `STORE_UPDATE` | Active | Update store metadata, Tor address, or policy fields. |
| `STORE_CLOSE` | Active | Delist store and initiate unbonding of store stake. |
| `STORE_SHIPPING_LINK` | Active | Add a carrier shipping account to the store record. |
| `STORE_SHIPPING_UNLINK` | Active | Remove a previously linked shipping account. |
| `PRODUCT_CREATE` | Posting | Add a product to the store catalog. |
| `PRODUCT_UPDATE` | Posting | Update price, inventory, description, or `delivery_cid`. |
| `PRODUCT_DELIST` | Posting | Remove a product from the active catalog. |
| `PRODUCT_QA_ASK` | Posting | Buyer submits a question on a product listing. |
| `PRODUCT_QA_ANSWER` | Posting | Vendor answers a buyer question; appended to listing. |
| `ORDER_PLACE` | Active | Place an order and lock buyer escrow. |
| `ORDER_FULFILL` | Posting | Vendor marks order shipped; records tracking number. |
| `ORDER_DELIVER` | Active | Buyer confirms receipt; releases escrow to vendor. |
| `ORDER_CANCEL` | Active or protocol | Cancel order and return escrow to buyer. |
| `ORDER_DISPUTE` | Active | Open a dispute; freezes escrow pending resolution. |
| `REPUTATION_VOTE` | Posting | Post-deliver rating (1–5) attached to the order record. |

Protocol-generated entries (`ORDER_CANCEL` on timeout, `ORDER_FULFILL` on auto-deliver)
are written by the epoch finalization process and are not signed by any user key. They
are identified in the block payload by `source: "protocol"` and are subject to the same
deterministic replay rules as all other entries.

---

## Appendix N — Key Architecture

### N.1 Key Types

Every BTCPC account has six cryptographic key roles arranged in a strict privilege hierarchy. Each key is a 64-character hex-encoded Ed25519 private key. They are separated by purpose so that compromising any lower-privilege key does not expose higher-privilege operations.

| Key | Privilege | One job | Lives on |
|-----|-----------|---------|---------|
| **Owner key** | Root | Rotate or revoke any other key. Signs `KEY_ROTATE` and `KEY_REVOKE` entries only. | Cold storage — hardware wallet or paper. Never online. |
| **Posting key** | Account | Signs all account operations: store mutations, product listings, order actions, Q&A, reputation votes. | Owner's device. |
| **Active key** | Financial | Signs token transfers: `ESCROW_LOCK` on order placement, `ESCROW_RELEASE` on delivery confirmation. | Owner's device. Never shared. |
| **Memo key** | Inbox | Encrypts and decrypts all private content addressed to this account: reputation memos, digital goods deliveries, private order data. The account's universal encryption inbox. | Owner's device. |
| **Fulfill key** | Automation | Signs `ORDER_FULFILL` entries for digital goods auto-delivery. Encrypted at rest using ECDH with the service key — only decryptable by the authorized service node. | Service node (safely losable — rotated by posting key). |
| **Service key** | Liveness | Signs `HEARTBEAT`, `SERVICE_LOG`, and `SERVICE_RESULT` entries only. Proves a service node is running. Cannot sign any commercial or financial entry. | Service node (safely losable — rotated by posting key). |

**Key custody.** The owner, posting, active, and memo keys never leave the owner's device. The fulfill and service keys are the only keys designed to live on a remote service node. Compromise of either node key requires only a posting-key rotation — no owner key intervention.

**Hierarchy.** The owner key can rotate any key. The posting key can rotate the fulfill and service keys (via `SERVICE_KEY_DELEGATE` / `SERVICE_KEY_REVOKE`). No lower key can rotate a higher one.

The posting key is the only key required for Phase G operations.

### N.2 Escrow Flow

**Phase G (current) — social commitment.** ORDER_PLACE is a signed ledger entry that records the buyer's intent. Funds are not moved on-chain at order time. Escrow is a social and reputational commitment enforced by the auto-cancel sweep (4,800 epochs / ~40 hours) and by stake-weighted dispute resolution.

**Phase H target — active key escrow.**

1. Buyer places order — active key signs an `ESCROW_LOCK` entry, debiting the buyer's wallet balance into the protocol escrow pool.
2. Seller ships — `ORDER_FULFILL` entry signed with seller's posting key (tracking number included).
3. Buyer confirms receipt — buyer's active key signs `ORDER_DELIVER`; protocol writes `ESCROW_RELEASE` sending funds to the seller minus the 1% fee split (0.5% to `btcpc_recycle`, 0.4% to store stakers pro-rata, 0.1% to reputation bonus pool).
4. Auto-cancel after 4,800 epochs if seller does not fulfill — protocol writes `ESCROW_REFUND` returning the full amount to the buyer.
5. Dispute before ORDER_DELIVER — `ORDER_DISPUTE` entry freezes escrow; arbiters (staked validators) resolve; `ESCROW_RELEASE` or `ESCROW_REFUND` is written by the protocol with the arbiter decision attached.

### N.3 Reputation Memos

After ORDER_DELIVERED, both parties may write a `REPUTATION_MEMO` entry. Memos are independent: the buyer writes one about the seller, the seller writes one about the buyer. Both are permanent, append-only public records on the chain.

**REPUTATION_MEMO entry fields:**

| Field | Type | Description |
|-------|------|-------------|
| `from` | account name | Writer of the memo |
| `to` | account name | Subject of the memo |
| `order_id` | string | The ORDER_PLACE entry ID this memo relates to |
| `memo_cid` | sha256 CID | BTCPC-FS blob containing the memo text, encrypted with the subject's memo key |
| `vote` | +1 / -1 / 0 | Public sentiment signal; 0 means neutral/no vote |
| `sig` | hex | Posting key signature over the above fields |

**Memo privacy.** The `vote` and all structured fields are public and fully readable by any node. The memo text itself is encrypted: only the holder of the subject's memo key can decrypt and read it. The subject can choose to publish their memo key selectively — to a trusted party, a dispute arbiter, or publicly.

**Reputation weighting.** Memos are weighted by the writer's stake and order history. A writer with zero stake and no prior completed orders contributes near-zero weight. A writer with significant stake and a long completed-order history contributes full weight. The weighted sum is the account's public reputation score, queryable at `GET /api/peer/commerce/stores/:seller`.

### N.4 Buyer Staking

Per-transaction active key signing creates UX friction for high-frequency buyers. The buyer staking alternative removes this friction without sacrificing trustlessness.

**Mechanism:**

1. Buyer submits a `STAKE_LOCK` entry signed with their active key, specifying an amount of BTCPC.
2. The staked balance is held in the protocol stake pool, not the buyer's liquid wallet.
3. Subsequent ORDER_PLACE entries reference the buyer's stake pool. The protocol deducts the order amount from the stake pool without requiring an active key signature per order — the initial STAKE_LOCK acts as standing authorization.
4. To withdraw staked funds, the buyer submits `STAKE_UNLOCK` signed with their active key. A 4,800-epoch (~40-hour) cooldown applies. Funds return to the liquid wallet after the cooldown.

**Safety.** The cooldown prevents a buyer from simultaneously draining their stake pool and reneging on open orders. Any in-flight orders are settled against the stake pool before the unlock is processed. If the stake pool balance falls below an open order's escrow amount, that order is auto-cancelled and the partial stake is returned to the buyer.

### N.5 Service Key and Fulfill Key Architecture

#### N.5.1 Service Key Delegation

The service key is a fresh Ed25519 keypair generated by the vendor and delegated via an on-chain entry signed by the posting key:

```
SERVICE_KEY_DELEGATE
  service_key_pubkey:   <Ed25519 pubkey of the service node>
  service_image_cid:    sha256:<WASM binary in BTCPC-FS>  (optional — binds key to exact binary)
  expires_epoch:        <epoch after which the delegation is void>
  signed_by:            posting_key
```

Chain validators reject any `HEARTBEAT`, `SERVICE_LOG`, or `SERVICE_RESULT` entry signed by a service key that has no valid, non-revoked delegation on-chain.

Revocation is immediate:

```
SERVICE_KEY_REVOKE
  service_key_pubkey:   <pubkey being revoked>
  signed_by:            posting_key
```

The `service_image_cid` binding is optional but significant: if set, the service key is cryptographically tied to a specific WASM binary. A node that swaps the binary cannot use the existing delegation. This provides binary attestation without requiring TEE hardware.

#### N.5.2 Fulfill Key Construction

The fulfill key is an Ed25519 keypair whose private half is stored encrypted in BTCPC-FS. The encryption uses ECDH between the vendor's fulfill keypair and the authorized service key:

```
Setup (vendor's device, once):
  generate fulfill_keypair (fulfill_privkey, fulfill_pubkey)
  shared_secret     = ECDH(fulfill_privkey, service_pubkey)
  encrypted_blob    = AES_256_GCM(key=shared_secret, plaintext=fulfill_privkey)
  blob_cid          = HONE_FS.store(encrypted_blob)

FULFILL_KEY_REGISTER (signed by posting_key):
  fulfill_pubkey:           <pubkey>
  encrypted_blob_cid:       sha256:<blob in BTCPC-FS>
  bound_service_key:        <service_pubkey>
  scopes:                   ["ORDER_FULFILL"]
  only_auto_deliver:        true
  seller:                   <account name>
  expires_epoch:            <epoch>
```

Runtime (service node, no vendor device needed):

```
shared_secret   = ECDH(service_privkey, fulfill_pubkey)
fulfill_privkey = AES_256_GCM_decrypt(key=shared_secret, blob from HONE_FS)
```

The vendor's device is required only for the one-time setup. After that the service runs autonomously. If the service key is revoked, the encrypted blob becomes permanently unreadable — the new service key cannot decrypt a blob that was encrypted to the old one. The vendor re-encrypts to the new service key and posts a new `FULFILL_KEY_REGISTER`.

**Scope.** The fulfill key can only sign `ORDER_FULFILL` entries where `auto_deliver = true`. Any attempt to sign a store mutation, product listing, reputation vote, or financial entry with a fulfill key is rejected by validators.

---

### N.6 Digital Goods Delivery Encryption

Digital goods are stored as encrypted blobs in BTCPC-FS. Three buyer paths are supported, unified by a single ECDH construction at the fulfill service.

The fulfill service has one code path regardless of buyer type:

```
encrypted_delivery = AES_256_GCM(
  key  = ECDH(fulfill_privkey, buyer_delivery_pubkey),
  data = digital_good
)
cid = HONE_FS.store(encrypted_delivery)
ORDER_FULFILL includes: delivery_cid = cid
```

The source of `buyer_delivery_pubkey` differs by buyer type.

#### N.6.1 On-Chain Buyer (Memo Key)

The buyer's memo key is their universal encryption inbox. The `buyer_delivery_pubkey` is the buyer's registered memo pubkey, readable from the chain at any time.

```
Fulfill:
  shared_secret = ECDH(fulfill_privkey, buyer_memo_pubkey)
  → encrypted blob in BTCPC-FS

Buyer decrypts:
  shared_secret = ECDH(buyer_memo_privkey, fulfill_pubkey)
  file = AES_decrypt(shared_secret, blob)
```

The content is end-to-end encrypted. No node in the delivery path — including the seller's service node — can read the file after it has been addressed to the buyer. Decryptable forever with the memo key.

#### N.6.2 Guest Buyer (Password-Derived Key)

The buyer enters a password at checkout. The browser derives an Ed25519 keypair from the password without the password ever leaving the device:

```
// In buyer's browser at checkout:
salt             = order_id  ← on-chain, unique per purchase
derived_seed     = HKDF(ikm=password, salt=order_id, info="btcpc-delivery-v1")
derived_keypair  = Ed25519.from_seed(derived_seed)

ORDER_PLACE includes: buyer_delivery_pubkey = derived_keypair.pubkey
                      (password never transmitted — only the derived pubkey)
```

The fulfill service sees only the derived public key and performs the same ECDH. The buyer re-derives the same private key from their password and order ID to decrypt:

```
Buyer decrypts:
  derived_seed    = HKDF(password, order_id, "btcpc-delivery-v1")
  derived_privkey = Ed25519.from_seed(derived_seed).privkey
  shared_secret   = ECDH(derived_privkey, fulfill_pubkey)
  file            = AES_decrypt(shared_secret, blob)
```

**Key properties:**
- The server never sees the password at any point — only the derived pubkey.
- The same password used across two different orders produces two different keypairs because the salt (order_id) differs.
- The buyer needs only their password and order ID to re-derive their key at any future time — no key file to save or lose.
- Weak password selection is the buyer's risk. The UI displays a strength indicator and states clearly: "This password decrypts your purchase. There is no recovery if it is lost."

#### N.6.3 Guest Buyer (No Key, No Account)

For buyers who provide neither a BTCPC account nor a password, the fulfill service issues a signed time-limited download token:

```
token = {
  order_id:      <order_id>,
  expires_epoch: placed_epoch + 4800,
  nonce:         random_256_bits(),
}
token_sig = sign(fulfill_privkey, token)
download_url = "https://<node>/api/commerce/deliver/<token>.<token_sig>"
```

The content is served in plaintext over the link. There is no end-to-end encryption guarantee for this path. The protocol is honest about this: the download is authenticated (only the fulfill service can issue a valid signed token) but not confidential.

After `expires_epoch` the token is rejected. The seller may issue a new token at their discretion.

#### N.6.4 Delivery Path Comparison

| Buyer type | Encryption | Key required | Expiry | Recovery if lost |
|------------|-----------|--------------|--------|-----------------|
| On-chain (memo key) | End-to-end, permanent | Memo key | None — decryptable forever | Rederive from mnemonic |
| Guest with password | End-to-end, permanent | Password + order ID | None — decryptable forever | Re-enter password |
| Guest, no key | Authenticated link only | None | 4,800 epochs (~40 hours) | Contact seller |

#### N.6.5 Content Encryption at Rest

Products with `auto_deliver = true` store their delivery content encrypted in BTCPC-FS at listing time. The fulfill key decrypts the raw content at order time, then re-encrypts it addressed to the buyer's specific delivery pubkey. The plaintext content never exists on any network node after the initial upload from the vendor's device.

---

---

## Appendix O — Freeport Protocol vs. Universal Commerce Protocol

### O.1 What Is a Universal Commerce Protocol?

A **Universal Commerce Protocol (UCP)** is the class of specifications that attempts to
standardize commerce across heterogeneous platforms — shared schemas for products,
orders, and payments that any vendor or storefront can implement. Examples of protocols
in this category include W3C Payment Request API, OpenAPI-based commerce schemas, and
various EDI standards. The defining characteristic of a UCP is **interoperability as the
primary goal**: make any system speak to any other system, regardless of who controls
the underlying infrastructure.

UCPs are valuable and widely deployed. They do not, however, address the core problems
that arise when the infrastructure layer itself is controlled by a small number of
corporations:

- A payment processor can terminate a merchant's account at will.
- A marketplace can delist a product for any reason, without appeal, without recourse.
- A shipping integrator can revoke API access and disable a vendor's fulfillment pipeline overnight.
- Platform fees are set unilaterally and can increase without consent from the merchants who depend on them.
- Buyer and seller identity, transaction history, and behavioral data are owned by the platform, not the parties to the trade.

A UCP running on top of controlled infrastructure inherits all of these failure modes.
The schema interoperability is real; the sovereignty is not.

### O.2 The Freeport Protocol Difference

Freeport Protocol is not a UCP. It is a **sovereign commerce layer** — a blockchain
where the marketplace, the escrow, the reputation system, and the payment rail are all
the same append-only ledger, replicated across every node, controlled by none of them.

The table below maps the core design choices that separate FP from the UCP model:

| Property | Universal Commerce Protocol | Freeport Protocol |
|---|---|---|
| **Ledger custody** | Hosted by platform operator | Replicated across all nodes; no operator |
| **Payment rails** | Credit cards, PayPal, USDT on third-party chains | BTCPC native token; settlement is on-chain and final |
| **Escrow** | Held by the platform (Stripe, PayPal, Shopify Payments) | Held by the protocol in a deterministic escrow pool; no intermediary |
| **Account identity** | KYC/email-gated, controlled by the platform | Ed25519 key hierarchy; pseudonymous by default |
| **Deplatforming** | Platform can remove vendor, product, or buyer at will | No platform operator exists to remove anyone |
| **Reputation data** | Owned by the platform; non-portable | On-chain REPUTATION_MEMO entries; owned by the account, portable to any node |
| **Fees** | 2–15% to platform, unilaterally adjustable | 1% protocol fee; 50% recycled, 50% to store stakers; no governing body can raise it |
| **Digital goods delivery** | Cleartext blob in platform CDN | End-to-end encrypted to buyer's memo key; platform nodes cannot read the content |
| **Catalog availability** | Dependent on platform uptime | Replicated across every full node; readable locally with no internet connection |
| **Vendor privacy** | Vendor's IP, session data, and inventory logged by platform | Vendor IP not in catalog delivery path; order communication optionally via .onion |
| **Settlement finality** | Chargebacks possible 90–180 days post-transaction | Cryptographic finality after challenge window; no chargebacks, no reversals |
| **Network participation** | Merchants pay rent to join the network | Store opening is a bonding curve fee that funds `btcpc_recycle`; no ongoing rent |
| **Compute layer** | None — commerce only | Commerce and compute are the same chain; miners earn by powering the market |

### O.3 The Integration Point: Compute Funds Commerce

The most structurally significant difference between FP and any UCP is what funds the
network.

A UCP has no native economic engine. Its security and availability depend on the
continued willingness of platform operators to pay their cloud bills. When a platform
is unprofitable, it shuts down. When it is profitable, it raises fees. The commerce
participants have no claim on either outcome.

In Freeport Protocol, the network is self-funding:

1. **Miners** run AI inference jobs and earn BTCPC.
2. **Storage hosts** store product blobs and earn BTCPC.
3. **Verifiers** audit inference results and earn BTCPC.
4. **Clock nodes** keep epoch timing and earn BTCPC.
5. **Service hosts** run buyer-facing applications and earn BTCPC.
6. **Sensor gateways** relay real-world data and earn BTCPC.

Commerce on the chain generates fee flow into `btcpc_recycle`. Recycled fees fund future
block rewards alongside the emission schedule. The more commerce happens, the more the
reward pool is replenished. The network does not need external funding; the work is the
funding mechanism.

This creates a property that no UCP can replicate: **the marketplace and the computational
infrastructure are the same economic system.** A merchant who opens a store on Freeport
Protocol is not renting shelf space from a corporation. They are a participant in a
network whose security and availability are maintained by the same workers who are being
paid to process the commerce transactions.

### O.4 What FP Is Not Trying to Replace

Freeport Protocol does not aim to replace every aspect of commerce infrastructure.
Specifically:

**Shipping carriers.** UPS, FedEx, USPS, DHL are physical networks. FP integrates with
them via the STORE_SHIPPING_LINK entry type, which lets vendors bind carrier API
credentials to their store record. FP handles the payment and escrow; the carrier handles
the atoms. This is a deliberate boundary — physical logistics are not a blockchain problem.

**Tax collection.** FP does not handle tax calculation or remittance. Vendors are
responsible for their own regulatory compliance. The chain records the transaction;
the jurisdiction enforces the obligation.

**Fiat on/off ramps.** BTCPC is not pegged. Buyers who need to convert fiat to BTCPC
use exchange infrastructure outside the protocol. FP provides no native exchange — this
is a deliberate sovereignty choice. The chain is not a custodian.

**Dispute resolution for physical goods disputes of fact.** When a buyer claims an item
never arrived and the tracking number shows delivered, the dispute panel makes a
judgment call. FP provides the infrastructure for staked dispute panels, but does not
pretend that on-chain voting resolves questions of physical fact better than a human
arbitrator would.

### O.5 Summary

A Universal Commerce Protocol standardizes the language of commerce across controlled
infrastructure. It solves the interoperability problem without addressing the custody
problem.

Freeport Protocol starts from a different premise: **the custody problem is the problem.**
When the platform owns the ledger, the escrow, the identity, and the distribution, every
participant is a tenant. The commerce protocol is just the API surface of the landlord's
system.

Freeport Protocol removes the landlord. The ledger is owned by the participants in
aggregate. The escrow is the protocol. The reputation is portable. The fee is fixed and
recycled. No entity has the technical ability to deplatform a merchant, reverse a settled
transaction, or raise the rake.

The "freeport" metaphor is precise: a port where goods move between sovereign parties
without a gatekeeper extracting rent at every crossing. The protocol's job is to make
sure the dock is always open.

---

*Freeport Protocol v3.1 — April 2026*
*Shin Devlin — shindevlin@proton.me*
*Native token: BTCPC*
*License: AGPL-3.0*
*GitHub: https://github.com/shindevlin/btcpc*
*Website: https://honemesh.net*
