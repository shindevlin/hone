# Bitcoin Proof of Compute (BTCPC)

### A Sovereign Blockchain Where Every Token Is Earned by a Machine Doing Real Work

**Shin Devlin**
**Version 3.0 — April 2026**

---

## Abstract

Bitcoin Proof of Compute (BTCPC) is a sovereign blockchain where block rewards are earned
by machines performing verifiable, useful work. Five categories of work produce emissions
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

### 1.2 The BTCPC Insight

BTCPC applies Bitcoin's core insight — costly, verifiable work can secure a network and
back a scarce asset — to a domain where the work itself has market value.

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

**BTCPC** is the digital labor market. Proof of Compute means the mining IS the product.
Every token represents real work done: an AI prompt answered, a file stored, a sensor
reading verified, an application served, a clock heartbeat delivered.

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
  operations are signed on the device and submitted via `btcpc.net/rotate`.
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

---

## 8. Oracle Feeds

### 8.1 Generic Off-Chain Data Ingestion

The oracle layer extends the sensor reading pipeline to any off-chain data source,
not just physical hardware. Price feeds, weather data, sports scores, and any other
API-sourced data can be submitted as oracle readings.

Oracle reporters are permissionless: any node that registers as an oracle provider
and maintains the minimum stake can submit readings for any registered feed.

### 8.2 Median Consensus

Oracle feeds use the same median consensus mechanism as sensor readings:

1. Multiple reporters submit values for a given feed and epoch
2. The finalization process computes the median across all reporters
3. Reporters whose values deviate more than `max_bps` from the median get a
   reputation dip (no slashing — reputation is the enforcement mechanism)
4. The finalized median is recorded on-chain

### 8.3 Reputation-Based Quality Control

Repeated outlier submissions drive down a reporter's reputation score below the
minimum threshold, after which their submissions are ignored until they recover.
This creates natural Sybil resistance without slashing: a single reporter cannot
move the median by flooding the system with fake readings, because median is
inherently resistant to outliers, and outlier reporters lose their oracle access
via reputation decay.

---

## 9. Cross-Chain Bridge

### 9.1 Lock-and-Recycle, Not Burn-and-Mint

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

### 9.2 Supply Cap per Chain

Each destination chain has a hard cap of **4,200,000 wBTCPC** pre-minted in the
contract constructor. This is the only mint operation that ever occurs. The contract
has no `mint()` function, no `burn()` function, no admin key, no upgrade proxy, and
no pause mechanism. It is immutable from deployment.

The maximum wBTCPC circulating on any destination chain at any given time equals the
cumulative BTCPC native locked into the source bridge for that destination, capped at
4,200,000. This maintains a strict 1:1 backing relationship.

### 9.3 Bridge Liquidity: Permissionless LPs

The bridge reserve is funded by permissionless LPs who lock BTCPC native into the
source-chain contract with a variable time commitment (30 to 1,460 days). Funders
earn a pro-rata share of all bridge fees proportional to their lock weight:

```
LP weight = amount × remaining_lock_days
```

This is the same veCRV-style commitment weighting used by Curve Finance. Weight
decreases naturally as the lock approaches expiration, preventing permanent
concentration. The LP roster rotates organically as locks expire and new LPs enter.

### 9.4 Bridge Fees

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

### 9.5 Withdrawal Queue

After a lock period expires, an LP requests withdrawal and enters a FIFO queue.
Position 1 in the queue is funded by:

- Incoming unwrap volume (the primary path — organic outflows fund incoming withdrawals)
- A smoothing buffer funded by 10–20% of bridge fees, capped at ~10% of total
  locked liquidity

LPs in queue continue earning fees until their position is filled. This hybrid
queue model prevents stalls under normal operation and avoids the ponzi-adjacent
dynamics of pure auto-redemption schemes.

### 9.6 Destination Chains

| Chain | wBTCPC Supply | Contract Type |
|-------|--------------|---------------|
| Base | 4,200,000 | Immutable ERC-20 |
| Arbitrum | 4,200,000 | Immutable ERC-20 |
| Ethereum | 4,200,000 | Immutable ERC-20 |
| Bitcoin | 4,200,000 | OP_RETURN anchor + off-chain relay |

---

## 10. Four-Tier Finality

### 10.1 Architecture

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

### 10.2 Demand-Driven Submission

Anchor submitters are permissionless. Any node can submit a state root to the finality
contracts and collect the anchor reward from the accumulated `anchor_reserve`. Bridge
operations pay a small fee into the reserve; the reserve funds anchor submission.

When the reserve is empty and bridge volume is low, anchors may be skipped. The chain
continues working. When bridge volume spikes, the reserve fills quickly and anchors
resume at full cadence.

### 10.3 Merkle Batching

Each anchor write commits a Merkle root over multiple epoch anchors (typically 100).
Bridges and clients verify specific epochs via off-chain Merkle proofs. This reduces
gas costs by approximately 98% compared to one-epoch-per-on-chain-write.

Tier 3 (Ethereum) anchors use EIP-4844 blob storage: 1,000 epoch anchors fit into a
single 128 KB blob, costing approximately $1–3 per anchor batch. Blob data is
available for 18 days on Ethereum; permanent storage is provided by BTCPC-FS mirror
nodes that store the full anchor history as on-chain blobs.

### 10.4 Bitcoin Deep Seal

Bitcoin anchoring uses OP_RETURN to inscribe the BTCPC state root hash (~32 bytes)
into a Bitcoin transaction. The rich anchor blob (account counts, epoch statistics,
miner records) lives in BTCPC-FS at a CID referenced by the OP_RETURN data —
dogfooding the chain's own storage layer.

Each Deep Seal mints a Soulbound NFT to the `btcpc_genesis_seals` system account as
a publicly browsable historical artifact. These are the chain's milestones.

### 10.5 Cost Baseline

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

## 11. State Management

### 11.1 Blockchain as Source of Truth

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

### 11.2 In-Memory stateStore

The stateStore module is an in-memory cache of the current chain state, rebuilt from
block files on startup via `replayFromDisk()`. All reads by controllers, routes, the
miner, and the explorer go through stateStore. Balance queries are O(1) Map lookups,
not O(N) database aggregations.

Mutation is always via `applyEntry(entry)`. Entries are the universal state-change
event type — the same shape whether they come from replay, from local ledger recording,
or from P2P gossip sync. Determinism is guaranteed: same sequence of entries produces
the same state.

### 11.3 Sparse Merkle Tree

The stateManager maintains a Sparse Merkle Tree (SMT) that tracks account state. The
SMT root is the `state_root` recorded in every block header. Two nodes that have
processed the same entries will have identical SMT roots. This root is used for:

- Verifying block integrity (expected root vs. computed root)
- Finality snapshot integrity checks
- Cross-chain anchor submissions

### 11.4 Finality Snapshots and Fast Sync

Every N epochs, the state is compacted into a finality snapshot: a full serialization
of all accounts, balances, and essential metadata at that epoch. New nodes can:

1. Connect to any peer
2. Request the latest finality snapshot
3. Verify the Merkle root against the block header
4. Start participating immediately without replaying the full history

Old blocks remain available for historical queries but are not required for consensus
participation. This design allows the chain to grow indefinitely while keeping join
time bounded.

### 11.5 Cross-Process Ledger Queue

In multi-process deployments (e.g., API server + miner + P2P node as separate processes),
ledger entries are shared via `data/pending-entries.jsonl`. Each process appends its
entries to this file. The miner flushes both in-memory and on-disk pending entries when
building a block. This queue is wiped after each successful block write.

---

## 12. Tokenomics Summary

### 12.1 Native Token

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

### 12.2 User-Created Tokens

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

### 12.3 Commerce Platform Fees

Order platform fee (1% of order total):

- 0.5% → `btcpc_recycle`
- 0.4% → store stakers (pro-rata by stake)
- 0.1% → reputation bonus pool

Store opening (bonding curve, paid in wrapped stables):

- 50% → `btcpc_recycle`
- 50% → `btcpc_treasury` (protocol development fund)

### 12.4 Stake Requirements

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

## 13. Privacy Roadmap

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

## 14. Governance

### 14.1 Genesis Phase

BTCPC is currently in its genesis phase. Founder-operated by Shin Devlin. Protocol
parameters (epoch duration, block cap bounds, reward pool splits, fee rates) are set
by the genesis operator. The genesis phase exists to allow fast iteration on the
technical foundations before the user base grows large enough to make governance
meaningful.

The genesis operator can change parameters. The genesis operator cannot change the
42,000,000 supply cap, cannot implement a burn mechanism, and cannot alter the
fundamental "no pre-mine, no founder allocation" structure — these are architectural
constants, not governance parameters.

### 14.2 Progressive Decentralization

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

### 14.3 The Design Goal

BTCPC was not built to be run. It was built to let go of. Every architectural decision
— permissionless clocks, demand-driven anchoring, no admin keys on bridge contracts,
permissionless LP, oracle feeds, sensor registration — follows this principle. The
chain should be able to operate without its creators.

*"We didn't build BTCPC to run it. We built it to let go of it."*

---

## 15. Conclusion

The blockchain industry spent its first decade asking "what can we put on a chain?"
The answer was mostly: tokens, speculation, and games.

BTCPC asks a different question: **what work needs to happen in the world, and how
do we pay the machines doing it?**

AI inference needs to happen. Files need to be stored. Sensor data needs to be
collected and verified. Applications need to be served. Epoch timing needs to be
maintained. All of these are real economic activities with real market demand. None
of them require a corporation to intermediate between the person who needs the work
done and the machine that does it.

BTCPC removes the corporation. The chain is the intermediary. The miners, storage
hosts, clock nodes, service hosts, and gateway operators are the workers. The users
are the customers. The token is the payment.

Every token earned. Every machine welcome.

---

*BTCPC v3.0 — April 2026*
*Shin Devlin — shin@btcpc.network*
*License: AGPL-3.0*
*GitHub: https://github.com/shindevlin/btcpc*
*Website: https://btcpc.net*
