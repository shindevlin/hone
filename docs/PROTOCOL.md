# BTCPC Protocol Primitives

Reference document for external auditors, SDK authors, and integration developers.
All values are canonical — they match `crates/hone-types/src/` exactly.
If this document conflicts with the code, the code wins.

---

## 1. Account Model

An **account** is an on-chain identity identified by a UTF-8 username (1–32 characters,
`[a-z0-9._-]`). Accounts are created via `AccountCreate` and are permanent; the name
can be transferred to a new key set via `AccountTransfer`.

### Key slots

Each account carries up to six named key slots. All keys are ed25519 public keys
encoded as 64-character lowercase hex strings.

| Slot | Purpose |
|------|---------|
| `owner` | Full account control. Can update all other keys. |
| `active` | Financial operations (Transfer, Stake, InferenceJobPost). |
| `posting` | High-frequency operations (Mine, SensorReading, Gossip). |
| `memo` | Decrypts encrypted memo fields. |
| `hide` | Generates hide-side commitment for hide/seek commerce. |
| `seek` | Generates seek-side commitment for hide/seek commerce. |

Any operation is authorized by the key slot appropriate for that operation (defined
in `tx.rs:canonical_signing_message`). The `signed_by` field names the account, and
the `sig` parameter carries the ed25519 signature over the canonical message.

### Nonce

Every account holds a monotonically-increasing `nonce` (u64, starts at 0). Financial
entries (`Transfer`, `Stake`, `InferenceJobPost`, etc.) must carry the account's
current nonce and increment it atomically. This prevents replay attacks.

### Name stake

When `chain_param:name_stake_enabled = true`, creating an account costs
`chain_param:name_stake_amount` dreams (default: `NAME_REGISTRATION_STAKE = 10 BTCPC`),
debited from the funding account.

---

## 2. Token Units

| Symbol | Unit | Value |
|--------|------|-------|
| `BTCPC` | Base token | 1 BTCPC = 10,000,000,000 dreams |
| `dreams` | Indivisible unit (u64) | Minimum transfer amount |
| `NATIVE_TOKEN` | Token identifier string | `"BTCPC"` |

All balances and fees in the protocol are denominated in **dreams** (u64). No floating
point arithmetic is used in consensus paths.

**Supply cap:** 42,000,000 BTCPC (= `SUPPLY_CAP_HUNITS = 420,000,000,000,000,000,000`
dreams). After the supply cap is reached (era 5+), rewards are paid from the
`__recycle_fund__` account.

---

## 3. Entry Format

Every state transition is a **`LedgerEntry`** — a tagged-union JSON object with a
`type` discriminant (derived automatically by serde from the variant name).

```jsonc
{
  "type": "Transfer",
  "from": "alice",
  "to": "bob",
  "amount": 1000000000,
  "token": "BTCPC",
  "epoch": 42,
  "nonce": 7,
  "signed_by": "alice"
}
```

### Canonical signing message

Every client-submitted entry is authenticated by an ed25519 signature over a
deterministic UTF-8 string defined in `tx.rs:canonical_signing_message`. The format
per entry type is documented in `tx.rs` (line ~1234). General pattern:

```
BTCPC:{chain_id}:{entry_type}:{field1}:{field2}:...
```

The `chain_id` is always included to prevent cross-chain replay.

| Chain | `chain_id` |
|-------|-----------|
| Mainnet | `hone` |
| Testnet | `hone-testnet` |

### Entry weights (fee basis)

| Class | Weight | Examples |
|-------|--------|---------|
| System | 0 | `EpochSeal`, `MineReward`, `ClockReward` |
| Micro | 1 | `Transfer`, `LivenessProof`, `AccountCreate` |
| Standard | 3 | `Mine`, `SensorDataCommit`, `GatewayHeartbeat`, `CoverageReport` |
| Heavy | 5 | `InferenceJobPost`, `StorageHeartbeat` |
| Bulk | 10 | `BlobStore`, `LinkGitRefUpdate` |
| Registration | 20 | `AccountCreate`, `ClockNodeRegister`, `SensorRegister` |

Fee = `entry_weight × base_fee`. `base_fee` adjusts ±10% per epoch targeting 50%
capacity utilization (EIP-1559-style). All fees route to `__recycle_fund__` — no burn.

### Stale-entry window

Entries whose `epoch` field is more than `STALE_WINDOW = 5` epochs behind the current
chain tip are rejected at `validate_and_apply` time. System entries bypass this check.

---

## 4. Epoch Lifecycle

An **epoch** is the chain's fundamental time unit. In era 0 an epoch is 30 seconds.
The epoch duration doubles every `DOUBLING_INTERVAL = 4,200,000` epochs.

```
epoch_ms(e) = INITIAL_EPOCH_MS << min(era(e), RECYCLE_ERA)
            = 30_000 << min(floor(e / 4_200_000), 5)
```

### Epoch flow

```
1. Clock nodes gossip EpochSeal messages (every 30s in era 0)
2. Clock consensus resolves quorum → winning seal_hash elected
3. drain_pending_sorted() — sort mempool by sha256(entry_bytes), deterministic
4. validate_and_apply() — each entry validated in sort order
5. Epoch seal applied — current_epoch advances
6. Reward distribution — inference / storage / sensor / clock / tracker pools
7. EpochFinalize — Merkle state root committed, rewards_hash broadcast
```

### Genesis

- **Genesis timestamp:** `1783191600000` ms (2026-05-01 12:00:00 IST / 11:00:00 UTC)
- **Chain ID:** `hone` (mainnet), `hone-testnet` (testnet)
- **Genesis block:** `rust/hone-node/genesis.json`

---

## 5. Signing Specification

All entries use **ed25519** (dalek implementation). The signature is passed out-of-band
as a hex string (128 hex chars = 64 bytes) alongside the JSON entry in the gossip
envelope or HTTP request body.

Signature target: `ed25519_sign(private_key, canonical_signing_message.as_bytes())`

For entries that embed the signature (e.g. `EpochSeal.signature`), the signature
covers the seal message format: `BTCPC:{chain_id}:EpochSeal:{epoch}:{seal_hash}:{node_id}:{timestamp}`.

#### External-chain signatures

`VerifyChainLink` accepts signatures from external chains. Supported types:

| `sig_type` | Verification method |
|-----------|---------------------|
| `eth_personal_sign` | EIP-191 prefix, secp256k1 recovery, lowercase hex address |
| `sol_sign` | Ed25519, base58 pubkey |
| `btc_legacy` | BIP-322 P2PKH, secp256k1 ECDSA, base58check address |

---

## 6. Reward Model

Each epoch, the epoch reward pool is split across six work pools. The split
targets calibrated utilization — if a pool is underutilized relative to its
target, it earns proportionally less of the global pool.

| Pool | What earns it |
|------|--------------|
| Inference | `Mine` entries with approved `InferenceJobVerify` |
| Storage | `StorageHeartbeat` with Merkle range proof |
| Sensor | `SensorDataCommit`, `SensorReading`, `CoverageReport` |
| Verifier | `InferenceJobVerify` verdicts |
| Clock | `EpochSeal` quorum participants |
| Tracker | `TrackerSightingCommit` from Verified observers |
| Service | `ServiceHeartbeat` (desktop nodes) |
| Mempool | `MempoolHeartbeat` (relay operators) |

A mandatory **2% reserve split** fires before pool distribution:
- 0.5% → `__testnet_fund__`
- 1.5% → `__recycle_fund__`

### Emission schedule (new supply)

| Era | Epoch duration | Per-epoch reward | Daily emission |
|-----|---------------|-----------------|----------------|
| 0 | 30s | 2 BTCPC | 5,760 BTCPC |
| 1 | 60s | 2 BTCPC | 2,880 BTCPC |
| 2 | 2 min | 2 BTCPC | 1,440 BTCPC |
| 3 | 4 min | 2 BTCPC | 720 BTCPC |
| 4 | 8 min | 2 BTCPC | 360 BTCPC |
| 5+ | 16 min | 0 (recycle only) | — |

New supply exhausted after ~124 years (≈ 2150). Era 5+ rewards come from
`__recycle_fund__` at rate `RECYCLE_REWARD_RATE / RECYCLE_REWARD_DENOM` per epoch.

---

## 7. Staking and Unbonding

Staked tokens back an account's trust score and are required for:
- Clock node registration (`ClockNodeRegister`)
- Inference verifier participation
- Device tracker claims (`DeviceClaimStake`)

`Unstake` begins a **10-epoch unbonding window** (`UNBONDING_EPOCHS = 10`).
Tokens are held in `unbonding:{release_epoch}:{account}` and released at epoch seal
via `drain_unbonding`. Instant unstake is not possible; this window allows slashing
to be submitted before funds are returned.

---

## 8. Consensus and Finality

### Clock quorum

The winning `EpochSeal` is elected by quorum among registered clock nodes. Quorum
requires `≥ 2/3` of registered nodes to agree on a `seal_hash` within `SEAL_COLLECT_MS`.
A node that submits two different `seal_hash` values for the same epoch is slashable
via `ClockDoubleSignEvidence` — stake is zeroed and distributed 10% submitter /
10% `__legal__` / 80% recycle.

**Equivocation guard:** the chain rejects a second `EpochSeal` from the same `node_id`
for the same epoch if the `seal_hash` differs. Identical re-delivery is idempotent.

### EpochFinalize and fork choice

`EpochFinalize` commits the epoch's `rewards_hash` and `state_root`. If two
finalizations arrive for the same epoch, the one with higher `quorum` count wins.
Equal-quorum ties break on lexicographically smaller `rewards_hash`.
Fork evidence is stored under `fork_evidence:{epoch}`.

### State root

`state_root` is a binary Merkle root over all account balances, sorted by
`{account}:{token}`. Any party can verify an inclusion proof returned by
`GET /api/proof/balance/:account/:token` against the published root.

---

## 9. Proof Model

### Inference (useful-work proof)

A `Mine` entry is a work commitment. For full reward it must carry a `job_id`
referencing an `InferenceJobAward` where `winner == miner` and the job's status
is `Verified` or `Paid` by epoch seal time.

`output_hash` (field formerly named `compute_proof`) is the SHA-256 hex of the
inference output text — a binding commitment, NOT a reproducibility proof (LLM
outputs are non-deterministic).

Verifiers submit `InferenceJobVerify` with a verdict (`approved` / `rejected` /
`review_required`). A board of 1–5 verifiers (auto-scaled by network size) reaches
quorum. Dissenters lose their fee and an equivalent stake slash. Early majority
resolution fires when a verdict's lead is mathematically unbeatable.

### Storage (Merkle range proof)

`StorageHeartbeat` must include a `MerkleRangeProof` to earn full reward.

| Field | Description |
|-------|-------------|
| `challenge_hash` | `sha256("{prev_seal_hash}:{node_id}:{epoch}")` |
| `range_start` / `range_end` | Byte offsets of the challenged chunk |
| `total_chunks` | Determines `effective_bytes = total_chunks × STORAGE_CHALLENGE_CHUNK_BYTES` |
| `leaf_hash` | `sha256("chunk:" ‖ range_start_be8 ‖ ":" ‖ chunk_bytes)` |
| `merkle_root` | Root of the storage node's content tree |
| `proof_nodes` | Sibling hashes from leaf to root |

Without a valid proof: reward reduced to `STORAGE_PROOF_NO_PROOF_BPS / 10_000` of full.

### Sensor / coverage (plausibility proof)

GNSS readings whose consecutive-epoch Haversine speed exceeds
`SENSOR_GNSS_MAX_SPEED_M_S = 300 m/s` are rejected.

`CoverageReport` entries (cellular dead-spot mapping) earn a 1.5× corroboration bonus
when 3+ independent reporters (`COVERAGE_CORROBORATION_MIN_REPORTERS`) submit for the
same `~100m grid cell × carrier × epoch`. Max 10 reporters per cell count toward the
threshold (`COVERAGE_MAX_CORROBORATING_REPORTERS`); extras earn base score only.

### Tracker (physical-presence proof)

`TrackerSightingCommit` earns reward only from observers that hold at least one
`TrackerClaim` with `status = "Verified"` (achieved via `TrackerAcousticProof`).
Observers with only `"Registered"` status earn zero — desk-based fake sightings
are excluded from the reward pool.

---

## 10. Replay Protection Summary

| Attack | Defence |
|--------|---------|
| Cross-chain replay | `chain_id` in every signing message |
| Stale entry injection | `STALE_WINDOW = 5` epoch check in `validate_and_apply` |
| Same-epoch gossip replay | `seen_entries` HashSet cleared each `drain_pending_sorted` |
| Clock equivocation | `epoch_node_seal:{epoch}:{node_id}` dedup guard + `ClockDoubleSignEvidence` slashing |
| Instant unstake to escape slash | 10-epoch unbonding window |
| Double award of inference job | Job status gate: second award rejected when `status != Posted` |
| Storage fraud | Merkle challenge hash ties proof to specific seal; corrupt proof → 20% rate |

---

## 11. API Endpoints (summary)

Base URL: `http://localhost:4242` (default). Full spec in `docs/api.md`.

| Method | Path | Description |
|--------|------|-------------|
| GET | `/api/balance/:account` | Account balance |
| GET | `/api/chain/state_root` | Current Merkle root |
| GET | `/api/proof/balance/:account/:token` | Balance inclusion proof |
| GET | `/api/chain/validators/:epoch` | Registered validators at epoch |
| GET | `/api/chain/fork/:epoch` | Fork evidence (if any) |
| POST | `/api/transfer` | Submit Transfer entry |
| POST | `/api/stake` | Submit Stake entry |
| POST | `/api/task/post` | Post inference job |
| POST | `/api/task/commit` | Submit inference commit (commit-reveal) |
| POST | `/api/task/verify` | Submit verifier verdict |
| POST | `/api/coverage/report` | Submit CoverageReport |
| POST | `/api/storage/heartbeat` | Submit StorageHeartbeat with Merkle proof |

Rate limit: 60 POST/PATCH requests per IP per 60-second window. Returns `429` on breach.

---

*This document targets v0.8.0 (current testnet). See `docs/ROADMAP.md` for what changes before v1.0.0 mainnet.*
