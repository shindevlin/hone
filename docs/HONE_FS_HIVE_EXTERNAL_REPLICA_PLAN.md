# HONE-FS Hive External Replica Plan

## Goal

Allow a HONE storage operator to run normal local HONE-FS storage and also mirror selected HONE-FS blobs into Hive. The Hive copy counts as a second decentralized storage-domain slot for rewards after independent verification.

This does not treat Hive bytes as local disk. The accounting model is:

- local HONE-FS replica: bytes physically held and served by the node operator
- Hive external replica: bytes or recoverable HONE-FS payloads held by Hive infrastructure
- local plus Hive: one operator, two storage domains, up to two reward slots

## Current Implementation

The first Rust-only implementation adds two ledger entries:

1. `HiveReplicaCommit`
   - Submitted by the storage node after writing a HONE-FS payload, chunk, parity shard, or manifest to Hive `custom_json`.
   - Records the Hive account, custom JSON id, block number, tx id, operation index, HONE CID, Merkle root, payload hash, bytes, replica kind, and confirmations.
   - Does not earn rewards by itself.

2. `HiveReplicaVerify`
   - Submitted by an independent HONE verifier after fetching the Hive transaction through Hive infrastructure.
   - Must match an existing commit.
   - Must be signed by the verifier account.
   - Must use a per-epoch challenge hash:

```text
sha256("{prev_seal_hash}:{node_id}:hive:{cid}:{hive_tx_id}:{epoch}")
```

Only successful verification creates a `storage_beat:{epoch}:{node_id}:hive` reward slot.

## Reward Semantics

Hive is counted as a separate storage domain, not as extra local disk.

The storage node can earn:

- one local storage reward slot from `StorageHeartbeat`
- one Hive external replica reward slot from verified Hive replicas

The Hive slot aggregates verified Hive refs for the node in that epoch but remains one slot:

```text
storage_beat:{epoch}:{node_id}       # local
storage_beat:{epoch}:{node_id}:hive  # Hive external domain
```

Replica score weights:

- `full`: 75% of verified bytes
- `chunk`: 50% of verified bytes
- `parity`: 30% of verified bytes
- `manifest`: 5% of bytes, capped at 1 MiB accountable bytes

The Hive slot is capped at 100 GiB of score per node per epoch.

## Anti-Gaming Rules

- A node cannot self-verify its Hive replica.
- A Hive ref must have at least 20 confirmations in the commit entry.
- The same Hive ref can only create one reward event per epoch.
- Manifest-only records cannot claim full blob storage credit.
- Hive rewards use tier `1` and do not receive local blob capacity multipliers.
- The chain stores Hive as `backend = "hive"` and `storage_domain = "hive"` for auditability.

## API Surface

Storage node commit:

```http
POST /api/storage/hive-replica/commit
```

Verifier attestation:

```http
POST /api/storage/hive-replica/verify
```

Both endpoints queue signed ledger entries and gossip them through the normal HONE pending-entry path.

## Required Hive Adapter

The next component should be a Rust sidecar or node module:

```text
hone-hivefs-adapter
```

Responsibilities:

- write HONE-FS manifests/chunks/parity shards to Hive `custom_json`
- track Hive tx id, block number, op index, and confirmations
- submit `HiveReplicaCommit`
- fetch Hive refs from independent Hive RPC/history nodes
- verify payload hash, CID, Merkle root, and chunk metadata
- submit `HiveReplicaVerify` from an independent verifier account

## Suggested Hive Payloads

Manifest:

```json
{
  "type": "hone_fs_manifest_v1",
  "cid": "...",
  "size_bytes": 123456,
  "chunk_size": 262144,
  "chunk_count": 1,
  "merkle_root": "...",
  "encrypted": true
}
```

Chunk:

```json
{
  "type": "hone_fs_chunk_v1",
  "cid": "...",
  "chunk_index": 0,
  "payload_sha256": "...",
  "merkle_root": "...",
  "payload_b64": "..."
}
```

## Build Phases

### Phase 1: Chain Support

Status: implemented.

- add ledger entries
- add API submission endpoints
- add verifier-gated storage-domain accounting
- add Rust tests for scoring and anti-self-verification

### Phase 2: Rust Hive Writer

Build `hone-hivefs-adapter` in Rust.

- support Hive account/posting-key config
- sign and broadcast Hive `custom_json`
- wait for confirmation depth
- submit HONE commit entry

### Phase 3: Rust Hive Verifier

Add verifier mode.

- fetch Hive tx by block/tx/op
- parse HONE-FS payload
- verify CID/Merkle/payload hash
- compute per-epoch challenge hash
- submit HONE verify entry

### Phase 4: UI/Operator Flow

Expose in miner/storage GUI:

```text
Mirror selected HONE-FS blobs to Hive
Use Hive as external decentralized replica
Verifier account for Hive replica checks
```

### Phase 5: Economic Hardening

- add governance-tunable Hive score weights
- add verifier rewards or slashing for false attestations
- add multi-verifier threshold for high-value blobs
- add external-domain diversity requirements for premium storage classes

## Non-Goals

- Do not store private plaintext on Hive.
- Do not count Hive bytes as local disk capacity.
- Do not allow self-verification to earn storage rewards.
- Do not make HONE consensus depend on live Hive RPC calls.
