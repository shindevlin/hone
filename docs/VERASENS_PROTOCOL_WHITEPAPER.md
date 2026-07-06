# Verasens Protocol

### Sovereign IoT Telemetry Anchored to a Proof-of-Compute Chain

**Shin Devlin**
**Version 1.0 — April 2026**

---

> **Note (2026):** Verasens Protocol is a native protocol within BTCPC, deployed at genesis block 0. All entry types described in this whitepaper are natively supported by the BTCPC chain. No separate deployment or smart contract required. See [NATIVE_PROTOCOLS.md](NATIVE_PROTOCOLS.md) for the full native protocol overview.

---

## Abstract

Verasens is a sovereign IoT telemetry protocol built into the BTCPC chain at genesis. It provides a tamper-evident, permissionless infrastructure for registering physical sensor devices, committing batched sensor readings to a distributed file layer, and paying storage nodes for long-term data availability — all without any trusted intermediary.

Every sensor reading committed through Verasens is content-addressed, cryptographically linked to a registered device identity, and permanently anchored to the BTCPC ledger. Gateway operators, device manufacturers, and data subscribers interact through ten first-class chain primitives. Protocol ownership is held by the `verasens` account (controlled by `shindevlin` at genesis) and is transferable via standard key rotation.

---

## 1. Introduction

### 1.1 The Problem with Centralized IoT

The global IoT market processes trillions of sensor readings daily. Nearly all of it flows through centralized cloud brokers: AWS IoT, Google Cloud IoT, Azure IoT Hub. These platforms offer convenience, but they impose critical structural weaknesses on the data they aggregate.

**Vendor lock-in.** A device manufacturer that commits to AWS IoT cannot migrate sensor data to a competing platform without losing historical continuity. The data lives in AWS's schema, AWS's storage, under AWS's terms of service. The manufacturer has no cryptographic claim to their own sensor history.

**No provenance.** A downstream subscriber purchasing sensor data from a cloud marketplace has no way to verify that the reading they received matches what the physical device actually transmitted. The broker is trusted. The sensor is not. Fraud — inserting synthetic readings, suppressing inconvenient spikes, back-filling data that was never collected — is undetectable.

**Single point of failure.** A prolonged outage at the broker (or a policy decision to terminate service for a particular customer) means the entire sensor network goes dark. There is no fallback. There is no sovereign copy.

**Extractive pricing.** Cloud IoT platforms charge both the device operator and the data subscriber. The broker earns a margin on every byte. Neither the device operator nor the subscriber receives the surplus their work creates — it flows upward to the platform.

### 1.2 The Verasens Insight

Verasens applies the same principle that underlies the rest of BTCPC: replace trusted intermediaries with cryptographic primitives.

A physical sensor device generates a keypair at manufacture or first boot. That keypair is registered on-chain. Every reading the device commits is signed by that keypair. Any observer — years later, on any chain replica — can verify that a reading came from a specific registered device, at a specific time, with no modification since it was written. The device's signature is the proof of provenance. No broker is required to vouch for it.

Storage nodes earn BTCPC by proving they are hosting the committed blobs. Gateway operators earn BTCPC for relaying readings from constrained devices that cannot connect directly to the chain. Staking a device signals active participation and earns a pro-rata share of the IoT reward pool — emissions that currently flow to AWS instead flow to the device operators and storage providers who are doing the actual work.

The data subscriber no longer pays a broker. They pay a storage node a fee denominated in BTCPC, retrieve the encrypted blob, and verify the device signature locally. The entire pipeline is auditable and permissionless.

### 1.3 Positioning

**Traditional IoT cloud platforms** are brokers. They hold data under their own terms. Trust is delegated entirely to the platform. The device operator has no cryptographic claim to their own readings once they are ingested.

**Helium** introduced token incentives for IoT coverage but remains focused on network infrastructure (LoRaWAN gateways) rather than the data layer. Helium nodes earn tokens for providing radio coverage; the data payloads are not content-addressed, not permanently stored, and not cryptographically bound to a registered device identity.

**Filecoin and Arweave** provide content-addressed permanent storage but have no IoT-native primitives. There is no concept of a registered device identity, no gateway heartbeat, no sensor staking, no real-time batch commit with chain-anchored timestamps.

**Verasens** combines all three: cryptographic device identity, permanent content-addressed storage, and token incentives for every participant in the data pipeline — from the sensor to the gateway to the storage host to the chain itself.

---

## 2. Architecture

### 2.1 Layers

```
Physical Device (sensor, GNSS receiver, weather station, etc.)
     │
     │  Ed25519 signing key (on-device)
     ▼
Gateway Node (optional — aggregates constrained devices)
     │
     │  Batches readings, computes BLAKE3 root, commits blob to btcpc-fs
     ▼
BTCPC Chain  ←  SensorDataCommit entry (content-addressed hash, device sig, epoch)
     │
     ▼
Storage Nodes  ←  StorageHeartbeat entries prove continued availability
     │
     ▼
Data Subscribers  ←  Fetch blob from any storage node, verify device sig locally
```

### 2.2 Device Identity

Every Verasens device has a registered identity on-chain. The identity is anchored to an Ed25519 keypair generated on the device. The device's public key is registered via `SensorKeyRegister` or `DeviceKeyRegister`. All subsequent data commits from that device are signed by the device key — the owner's wallet key is only required for registration, key rotation, and staking operations.

This architecture has an important property: even if the owner's wallet is compromised, an attacker cannot forge historical readings, because the readings are signed by the device key, not the wallet key. And even if the device is compromised going forward, the owner can rotate the device key with a single `SensorKeyRegister` entry signed by their wallet — invalidating any future readings from the compromised key while preserving the historical record.

### 2.3 Content Addressing

Sensor readings are committed in batches as content-addressed blobs stored in btcpc-fs. The blob format is:

```
[device_id: 32 bytes]
[reading_count: 4 bytes, little-endian]
[readings: variable]
  [timestamp_ms: 8 bytes]
  [value: 8 bytes IEEE 754 double]
  [unit: 4 bytes ASCII]
  [metadata_len: 2 bytes]
  [metadata: variable, CBOR]
[device_sig: 64 bytes Ed25519 over SHA-256 of preceding bytes]
```

The blob CID (SHA-256 content hash) is written into the `SensorDataCommit` ledger entry. Any node can fetch the blob from btcpc-fs, verify the device signature, and confirm that the reading matches what is anchored on-chain.

### 2.4 GNSS / RTCM3 Integration

Verasens has native support for high-precision GNSS sensor data, including RTCM3 correction streams. The `btcpc-gnss-capture` sidecar (written in Rust for real-time scheduling) intercepts RTCM3 frames from GNSS receivers via TCP, assembles them into batches, and submits `SensorDataCommit` entries to the chain at configurable intervals.

RTCM3 correction data committed through Verasens can be consumed by any RTK-capable receiver as a decentralized alternative to proprietary correction networks (Onocoy, Trimble RTX, Leica SmartNet). Operators of GNSS base stations earn BTCPC from the IoT reward pool and from direct subscriber payments — both denominated on-chain.

### 2.5 Gateway Operators

Not all sensors can connect directly to the BTCPC P2P network. LoRaWAN devices, BLE sensors, and other constrained hardware communicate through gateway nodes. A gateway node:

1. Collects readings from one or more constrained devices.
2. Batches the readings, computes the blob, and commits a `SensorDataCommit` on behalf of the device.
3. Submits periodic `GatewayHeartbeat` entries to signal liveness and connected sensor count.

Gateways earn a portion of the IoT reward pool proportional to the number of unique sensor readings they relay per epoch. The on-chain record distinguishes direct commits from gateway-relayed commits, so the reward is correctly attributed to the gateway rather than appearing as a direct device submission.

---

## 3. Entry Types

All Verasens operations are expressed as first-class BTCPC ledger entries. No WASM smart contract is required. The chain applies Verasens state transitions natively.

| Entry Type | Who Signs | What It Does |
|---|---|---|
| `SensorRegister` | device owner (posting key) | Registers a new sensor identity. Fields: `hardware_id`, `sensor_type`, `location_meta`, `owner`. |
| `SensorKeyRegister` | device owner (posting key) | Associates an Ed25519 signing keypair with a registered sensor. Used for initial registration and key rotation. |
| `SensorVouch` | existing registered sensor or gateway (posting key) | Writes an on-chain vouch for a sensor's legitimacy, contributing to its reputation score. |
| `SensorReading` | sensor key (Ed25519) | Commits a single real-time reading: value, unit, timestamp, device sig. |
| `SensorDataCommit` | sensor key or gateway (posting key) | Commits a batch of readings as a content-addressed btcpc-fs blob. Includes CID, reading count, epoch, and optional merkle root. |
| `DeviceKeyRegister` | device owner (posting key) | Registers an Ed25519 key for a general IoT device that is not a sensor (actuator, relay, gateway). |
| `DeviceYieldStake` | device owner (active key) | Stakes BTCPC to signal active device participation. Earns pro-rata from the IoT reward pool each epoch the device submits readings. |
| `DeviceYieldUnstake` | device owner (active key) | Initiates unstake. Subject to a 72-epoch (~36 hour) unbonding period. |
| `GatewayHeartbeat` | gateway node (posting key) | Periodic liveness signal. Fields: gateway ID, connected sensor count, epoch, signature. |
| `StorageHeartbeat` | storage host (posting key) | Proves continued availability of sensor data blobs. Includes a Merkle root of currently-stored CIDs. Storage nodes earn reward each epoch they submit a valid heartbeat. |

### 3.1 Entry Schema Details

#### `SensorRegister`
```json
{
  "type": "SensorRegister",
  "account": "alice",
  "hardware_id": "gnss-base-001",
  "sensor_type": "gnss_rtcm3",
  "location_meta": { "lat": 13.6929, "lon": -89.2182, "alt_m": 842 },
  "epoch": 4200,
  "sig": "..."
}
```

#### `SensorDataCommit`
```json
{
  "type": "SensorDataCommit",
  "account": "alice",
  "sensor_id": "gnss-base-001",
  "cid": "sha256:b94d27b9...",
  "reading_count": 150,
  "epoch": 4200,
  "device_sig": "...",
  "sig": "..."
}
```

#### `DeviceYieldStake`
```json
{
  "type": "DeviceYieldStake",
  "account": "alice",
  "device_id": "gnss-base-001",
  "amount": 5000000000,
  "epoch": 4200,
  "sig": "..."
}
```

---

## 4. Reward Model

### 4.1 IoT Reward Pool

The BTCPC emission schedule allocates **8% of each epoch's total reward** to the IoT pool. With a 30-second epoch and 42 million BTCPC total supply, this pool distributes roughly 840,000 BTCPC over the network's lifetime — entirely to IoT participants.

Participants earning from the IoT pool:
- **Sensor operators** — pro-rata share based on number of verified `SensorDataCommit` entries in the epoch, weighted by staked BTCPC
- **Gateway operators** — pro-rata share based on unique sensors relayed per epoch
- **Storage hosts** — pro-rata share of the storage pool (12%) for proving continued availability via `StorageHeartbeat`

Pools with no claimants in a given epoch recycle into the global `btcpc_recycle` account rather than being burned, preserving total supply integrity while smoothing emissions over time.

### 4.2 Staking Weight

Device yield stake functions as a commitment mechanism. Operators who stake more BTCPC demonstrate confidence in their hardware's continued operation and receive proportionally higher IoT rewards. The reward weight formula is:

```
reward_weight(device) = readings_committed(epoch) × log10(1 + stake_dreams)
```

This logarithmic taper prevents pure capital accumulation from dominating — a device doing real work with modest stake earns more per dream staked than an idle device with a large stake.

### 4.3 Slashing

Verasens does not currently implement automatic slashing for bad sensor data — distinguishing sensor malfunction from fraud is a social oracle problem, not a cryptographic one. Instead, the reputation system (`SensorVouch`) provides economic signal: a sensor that consistently publishes outlier data (detected off-chain by subscribers) will lose vouches and drop in reputation, reducing its stake weighting in future epochs.

A governance mechanism for on-chain dispute resolution of sensor fraud is planned for a future protocol version.

---

## 5. Privacy Model

### 5.1 Public vs. Private Readings

By default, all sensor readings committed to btcpc-fs are public. The blob is content-addressed and fetchable by any node. This is appropriate for most IoT use cases where data monetization depends on broad accessibility — weather data, air quality, GNSS corrections, traffic telemetry.

For use cases requiring confidentiality (medical sensors, industrial process monitoring, private location tracking), an operator can encrypt the blob before committing the CID to the chain. The CID of an encrypted blob is indistinguishable from a public blob on-chain; the encryption is at the storage layer. Access control for encrypted sensor data uses the same hide key architecture as LinkGit private repositories:

1. Operator generates a symmetric key (AES-256-GCM or ChaCha20-Poly1305).
2. Blob is encrypted with the symmetric key before upload to btcpc-fs.
3. Subscribers request access; operator encrypts the symmetric key to the subscriber's hide public key and delivers it out-of-band or via a future `SensorAccessGrant` entry.

### 5.2 On-Chain Metadata

Even with encrypted blobs, the `SensorDataCommit` entry itself is public. The device ID, reading count, CID, and epoch are visible to all nodes. Operators who require metadata privacy should use pseudonymous device IDs rather than human-readable hardware identifiers.

---

## 6. GNSS Correction Network

### 6.1 The Problem with Existing Correction Services

High-precision RTK GNSS positioning requires real-time RTCM3 correction streams from a network of base stations. Currently, these streams are delivered by proprietary correction networks:

- **Onocoy** — crypto-incentivized, but corrections are hosted on centralized infrastructure
- **Trimble RTX**, **Leica SmartNet**, **Topcon TopNET** — fully proprietary, expensive subscriptions, geographic gaps
- **NTRIP casters** — open protocol, but relay infrastructure is centralized and single points of failure

None of these networks gives the base station operator a cryptographically verifiable claim to their own data stream. If the network terminates service, the operator loses access to their own corrections.

### 6.2 Verasens GNSS Network

Verasens enables a fully decentralized RTK correction network:

1. Base station operator runs `btcpc-gnss-capture` on a machine connected to their GNSS receiver.
2. RTCM3 frames are intercepted in real time, batched every N frames, and committed as `SensorDataCommit` entries.
3. Rover operators subscribe to the base station's data stream by paying a streaming fee denominated in BTCPC.
4. Fee payment is a `Transfer` entry to the base station operator's account for each epoch of subscription.
5. The rover fetches the RTCM3 batch from btcpc-fs and feeds it to the receiver's correction input.

This creates a market for high-precision GNSS corrections where base station operators set their own prices, corrections are permanently archived, and no central correction network infrastructure is required.

---

## 7. Protocol Ownership and Governance

### 7.1 Initial Ownership

The `verasens` account is controlled by `shindevlin` at genesis. The `verasens-registry` account serves as the on-chain anchor for all sensor identity records.

**What shindevlin controls at genesis:**
- Protocol fee parameters (percentage of each `SensorDataCommit` fee flowing to `verasens`)
- `SensorVouch` weight calibration
- Dispute resolution governance (when implemented)
- Future protocol upgrades (new entry types, schema extensions)

**What shindevlin does not control:**
- Individual sensor identities — once registered, a sensor's key chain belongs to the owner
- Historical data — all committed blobs are content-addressed and immutable
- Staked balances — staking and unstaking is permissionless; no protocol account has authority over user stakes

### 7.2 Transfer Mechanism

Ownership of Verasens can be transferred from shindevlin to any other party via two steps:

1. **On-chain key rotation**: submit `AccountUpdateKey` for the `verasens` account, changing the owner key to the new controller's key. This transfers protocol authority on-chain.
2. **GitHub repository transfer**: transfer the `btcpc/btcpc` GitHub repository (which contains the Verasens implementation) to the new controller's GitHub account. This transfers implementation authority off-chain.

Both steps together constitute a complete transfer of the protocol. Either step alone gives partial authority. The BTCPC governance record at genesis block 0 establishes shindevlin as the original protocol author for historical attribution.

### 7.3 Revenue Model

Verasens earns protocol revenue from two sources:

1. **Data commit fee**: a small percentage of the `SensorDataCommit` fee (denominated in dreams) flows to the `verasens` account. This fee is set at genesis and adjustable by the `verasens` account holder.
2. **Storage extension fee**: a percentage of each `StorageHeartbeat` reward flows to `verasens` as a protocol maintenance fee.

Protocol revenue flows to the `verasens` account and is claimable by the controlling key. Future governance may direct a portion of this revenue to a community fund.

---

## 8. Implementation

### 8.1 btcpc-gnss-capture

The reference implementation of the Verasens sensor capture layer is `btcpc-gnss-capture`, a Rust binary for real-time RTCM3 frame interception. It is a sidecar to the main `btcpc-node` process and communicates with the node via HTTP API.

**Responsibilities:**
- TCP listener intercepts RTCM3 frames on a configurable port
- RTCM3 frame validation: preamble 0xD3, 10-bit length, CRC-24Q
- Frame batching at a configurable interval (default: every 30 frames)
- HTTP POST to the node's sensor intake endpoint with the blob and device signature
- Optional parallel forward to upstream NTRIP casters

**Scheduling:** `btcpc-gnss-capture` runs with elevated scheduler priority (`Nice=-10` or `SCHED_FIFO` in the systemd unit) to minimize GC-related frame drops. The capture loop is timing-sensitive; GC pauses in a Node.js implementation would cause frame loss at high frame rates.

### 8.2 Chain-Side Entry Handling

The BTCPC node handles Verasens entries natively in `apply_entry()`. The chain does not store raw sensor readings — only the commitment (CID + reading count + epoch + device sig). This keeps the chain database lean. The actual data lives in btcpc-fs, with availability proven by periodic `StorageHeartbeat` entries.

### 8.3 Querying Sensor Data

The BTCPC HTTP API exposes sensor query endpoints:

```
GET /api/sensors                    — list registered sensors
GET /api/sensors/{id}               — get sensor metadata + latest commit CID
GET /api/sensors/{id}/commits       — list all SensorDataCommit entries for a sensor
GET /api/sensors/{id}/commits/{cid} — get a specific commit's metadata
```

The raw blob is fetched from btcpc-fs by CID. Clients verify the device signature locally after retrieval.

---

## 9. Comparison

| Feature | AWS IoT / Azure IoT | Helium | Filecoin/Arweave | Verasens |
|---|---|---|---|---|
| Registered device identity | ✓ (centralized) | ✗ | ✗ | ✓ (on-chain) |
| Cryptographic data provenance | ✗ | ✗ | ✗ | ✓ |
| Permanent content-addressed storage | ✗ | ✗ | ✓ | ✓ |
| Token incentives for data producers | ✗ | ✓ (coverage only) | ✗ | ✓ |
| Token incentives for storage hosts | ✗ | ✗ | ✓ | ✓ |
| RTCM3 / GNSS native support | ✗ | ✗ | ✗ | ✓ |
| Permissionless key rotation | ✗ | partial | ✗ | ✓ |
| No central broker | ✗ | ✗ | ✓ | ✓ |
| Works with constrained devices | ✓ | ✓ | ✗ | ✓ (gateway) |

---

## 10. Roadmap

### Phase 1 (Genesis — Q2 2026)
- All 10 entry types live on mainnet
- `btcpc-gnss-capture` sidecar operational
- RTCM3 base station network bootstrapped with initial hardware operators

### Phase 2 (Q3 2026)
- Direct subscriber payment flow (streaming fee per epoch per subscriber)
- `SensorAccessGrant` entry for encrypted sensor data access control
- Mobile SDK for constrained BLE/WiFi device integration

### Phase 3 (Q4 2026)
- On-chain dispute resolution for sensor fraud allegations
- Multi-observer consensus for high-value readings (weather derivatives, carbon credits)
- Verasens marketplace: data subscribers discover and subscribe to sensor streams by type, geography, and price

### Phase 4 (2027)
- Hardware certification program for Verasens-native devices
- Integration with external oracle networks (Chainlink, Pyth) for cross-chain data export
- LoRaWAN gateway standardization — Helium gateway operators can run Verasens alongside Helium

---

## Appendix A: Entry Type Reference

### `SensorRegister`
| Field | Type | Description |
|---|---|---|
| `account` | string | Owner's BTCPC account name |
| `hardware_id` | string | Unique device identifier (serial number, IMEI, etc.) |
| `sensor_type` | string | Sensor category (`gnss_rtcm3`, `temperature`, `air_quality`, `gps_location`, etc.) |
| `location_meta` | object | Optional: lat, lon, alt_m, description |
| `epoch` | u64 | Chain epoch at submission |
| `sig` | string | Hex-encoded signature (owner's posting key) |

### `SensorKeyRegister`
| Field | Type | Description |
|---|---|---|
| `account` | string | Owner's BTCPC account name |
| `sensor_id` | string | `hardware_id` of the target sensor |
| `device_pubkey` | string | Hex-encoded Ed25519 public key generated on-device |
| `epoch` | u64 | Chain epoch at submission |
| `sig` | string | Hex-encoded signature (owner's posting key, or current device key for rotation) |

### `SensorDataCommit`
| Field | Type | Description |
|---|---|---|
| `account` | string | Submitter's BTCPC account name (owner or gateway) |
| `sensor_id` | string | `hardware_id` of the source sensor |
| `cid` | string | Content hash of the btcpc-fs blob (SHA-256 hex) |
| `reading_count` | u32 | Number of readings in the blob |
| `epoch` | u64 | Chain epoch at submission |
| `device_sig` | string | Ed25519 signature by the device key over the blob hash |
| `sig` | string | Hex-encoded signature (submitter's posting key) |

### `DeviceYieldStake`
| Field | Type | Description |
|---|---|---|
| `account` | string | Owner's BTCPC account name |
| `device_id` | string | `hardware_id` of the device being staked |
| `amount` | u64 | Amount in dreams to stake |
| `epoch` | u64 | Chain epoch at submission |
| `sig` | string | Hex-encoded signature (owner's active key) |

### `GatewayHeartbeat`
| Field | Type | Description |
|---|---|---|
| `account` | string | Gateway operator's BTCPC account name |
| `gateway_id` | string | Unique gateway identifier |
| `sensor_count` | u32 | Number of sensors connected at time of heartbeat |
| `epoch` | u64 | Chain epoch at submission |
| `sig` | string | Hex-encoded signature (gateway operator's posting key) |

### `StorageHeartbeat`
| Field | Type | Description |
|---|---|---|
| `account` | string | Storage host's BTCPC account name |
| `merkle_root` | string | Merkle root of all currently-stored sensor CIDs |
| `blob_count` | u32 | Number of blobs currently stored |
| `total_bytes` | u64 | Total bytes stored |
| `epoch` | u64 | Chain epoch at submission |
| `sig` | string | Hex-encoded signature (storage host's posting key) |

---

## Appendix B: btcpc-gnss-capture Configuration

```ini
# /etc/systemd/system/btcpc-gnss-capture.service
[Unit]
Description=BTCPC GNSS RTCM3 Capture Service
After=network.target btcpc-node.service

[Service]
Type=simple
User=btcpc
ExecStart=/usr/local/bin/btcpc-gnss-capture
Environment="SENSOR_ID=gnss-base-001"
Environment="HONE_ACCOUNT=alice"
Environment="HONE_NODE_URL=http://localhost:4242"
Environment="RTCM3_LISTEN_PORT=2201"
Environment="CHAIN_INTERVAL=30"
Nice=-10

[Install]
WantedBy=multi-user.target
```

---

## Appendix C: Reserved Accounts

| Account | Purpose | Controlled By |
|---|---|---|
| `verasens` | Protocol authority account; receives platform fee share | `shindevlin` at genesis; transferable via key rotation |
| `verasens-registry` | On-chain anchor for sensor identity records | Protocol; no user keys at genesis |

Both accounts are seeded at genesis block 0 with zero BTCPC balance.

---

*Verasens Protocol — Version 1.0 — Shin Devlin — April 2026*
*Part of the BTCPC native protocol suite. See also: [Freeport Protocol Whitepaper](FREEPORT_PROTOCOL_WHITEPAPER.md), [LinkGit Protocol Whitepaper](LINKGIT_PROTOCOL_WHITEPAPER.md)*
