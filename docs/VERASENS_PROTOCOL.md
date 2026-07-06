# Verasens Protocol

**Version 1.0 — April 2026**

---

## Overview

Verasens is an IoT sensor and device protocol baked into BTCPC genesis as a native ledger entry type. Sensor registration, key management, data commits, gateway heartbeats, and device yield staking are all first-class chain primitives — no WASM contract required. Sensor data published through Verasens is anchored to the chain, tamper-evident, and attributable to a registered device identity. The reserved accounts `verasens` and `verasens-registry` are seeded at genesis block 0.

---

## Entry Types

| Entry Type | Who Signs | What It Does |
|------------|-----------|--------------|
| `SensorRegister` | device owner | Registers a new sensor identity on-chain (hardware ID, type, location metadata) |
| `SensorKeyRegister` | device owner | Associates a signing keypair with a registered sensor |
| `SensorVouch` | existing registered sensor or gateway | Vouches for a sensor's legitimacy; builds on-chain reputation graph |
| `SensorReading` | sensor key | Submits a single real-time sensor reading (value, unit, timestamp) |
| `SensorDataCommit` | sensor key or gateway | Commits a batch of readings as a content-addressed blob; links to BTCPC-FS |
| `DeviceKeyRegister` | device owner | Registers an Ed25519 key for a general IoT device (non-sensor) |
| `DeviceYieldStake` | device owner | Stakes BTCPC to signal active participation; earns pro-rata from IoT reward pool |
| `DeviceYieldUnstake` | device owner | Initiates unstake; subject to unbonding period |
| `GatewayHeartbeat` | gateway node | Periodic liveness signal from an aggregating gateway; includes connected sensor count |
| `StorageHeartbeat` | storage host | Proves continued availability of sensor data blobs committed to BTCPC-FS |

---

## Device Key Registration Flow

1. Device owner generates an Ed25519 keypair on the device.
2. Owner submits `SensorRegister` (or `DeviceKeyRegister` for non-sensor devices) signed by their wallet key, embedding the device public key.
3. Owner submits `SensorKeyRegister` to bind the device keypair to the registered sensor identity.
4. Device signs all subsequent `SensorReading` and `SensorDataCommit` entries with its own key — the owner's wallet key is no longer required for normal operation.
5. Key rotation: submit a new `SensorKeyRegister` signed by the current authorized key. Old key is invalidated for new entries; historical entries remain valid under the old key.

---

## Reserved Accounts

| Account | Purpose |
|---------|---------|
| `verasens` | Protocol authority; receives platform fee share per data commit |
| `verasens-registry` | On-chain registry anchor; all sensor identity records reference this account |

Both accounts are seeded at genesis block 0 with zero balance. They cannot be registered, transferred, or destroyed by any user key.

---

*See also: [[HONE_WHITEPAPER]], [[PLAN_v2.10.1_to_v2.14]] §v2.15 (BTCPC-nano + LoRa sensor mesh)*
