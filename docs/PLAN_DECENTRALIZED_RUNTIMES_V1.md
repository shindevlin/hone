# PLAN: Decentralized Runtime Hosting v1

## Summary
HONE will add a decentralized runtime layer so users can deploy and run services like Deadman Switch across multiple hosts with durable scheduling, failover, and cryptographic accountability. This phase is not fully trustless yet; it is crypto-verifiable and slashable, with a staged path to deterministic trustless execution.

Defaults selected for v1:
- Trust model: signed attestations + challenge proofs + slashing
- Runtime scope: HTTP services + background workers + stateful session services
- Runtime placement: OCI containers and WASM modules from day one

## Implementation Changes

### 1) Protocol and State Additions
Add ledger entry families for runtime lifecycle:
- `RUNTIME_REGISTER`
- `RUNTIME_DEPLOY`
- `RUNTIME_JOB_ENQUEUE`
- `RUNTIME_CLAIM`
- `RUNTIME_ATTEST`
- `RUNTIME_CHALLENGE`
- `RUNTIME_SLASH`
- `RUNTIME_UNDEPLOY`

Add state maps and snapshot hydration support:
- `runtimes`
- `runtime_hosts`
- `runtime_leases`
- `runtime_jobs`
- `runtime_attestations`
- `runtime_challenges`
- `runtime_slashes`

Deterministic IDs:
- `runtime_id = sha256(owner + manifest_cid + nonce)`
- `job_id = sha256(runtime_id + epoch + sequence)`

Invariants:
- Unique `job_id`
- One active lease per `(runtime_id, job_class)`
- Idempotent settlement via attestation ID

### 2) Runtime Manifest + Host Capability Model
Define chain-anchored runtime manifest (CID-backed):
- metadata: name, version, owner
- execution: `runtime_type` (`oci` | `wasm`), entrypoint, args
- network: protocol/ports/exposure policy
- resources: CPU, RAM, disk, timeout caps
- persistence: stateless/stateful mode and snapshot policy
- verification policy: quorum/challenge window/slash matrix

Host capability registration:
- runtime support (`oci`, `wasm`)
- capacity limits
- staking/slashability status
- host pubkey and endpoint metadata

### 3) Distributed Scheduler + Lease Failover
Replace process-local timer ownership with lease-based distributed claims:
- workers scan due jobs
- claim via `RUNTIME_CLAIM` entry
- lease has TTL and renewal policy
- if lease expires, another host may claim

Execution flow:
1. enqueue durable job
2. host claims lease
3. host executes runtime action
4. host posts attestation
5. settlement finalizes reward/penalty

This specifically supports Deadman-like flows by persisting check-in and trigger jobs, removing single-process timer dependency.

### 4) Verification, Challenge, and Slashing
Attestation requirements:
- signed by host key
- includes runtime ID, job ID, output commitment, timing, and resource proof envelope

Challenge flow:
- open challenge window
- peers submit evidence for invalid output/non-delivery/double-claim
- adjudication emits slash entries when faults proven

Offense classes:
- `runtime_downtime`
- `invalid_attestation`
- `double_claim`
- `state_commit_mismatch`

### 5) API / Interface Surface
Add runtime API routes (or equivalent RPC endpoints):
- `POST /runtime/register`
- `POST /runtime/deploy`
- `POST /runtime/jobs/:runtimeId/enqueue`
- `POST /runtime/claims`
- `POST /runtime/attest`
- `POST /runtime/challenge`
- `POST /runtime/undeploy`
- `GET /runtime/:runtimeId/status`

Expose registry read paths:
- host capabilities and health
- lease ownership and expiry
- attestation/challenge outcomes

## Phased Execution Plan
1. Protocol + state plumbing (entries, maps, snapshots)
2. Manifest and host-capability registration
3. Lease-based scheduler and durable job execution
4. Attestation, challenge, and slashing integration
5. API routes and Deadman-style reference runtime migration
6. Hardening: multi-node race, failover, adversarial testing

## Test Plan
- Unit:
  - entry validation, ID determinism, lease state transitions
  - attestation signature verification and idempotency
- Integration:
  - multi-host claim race and lease expiry failover
  - duplicate prevention under concurrent workers
  - slashing on proven invalid attestation/double-claim
- E2E:
  - deploy runtime, enqueue jobs, fail host mid-run, confirm takeover
  - Deadman scenario where check-in/trigger remains correct across restart/failover
- Determinism:
  - snapshot hash stability with runtime maps populated

## Assumptions
- Existing chain/state pipeline remains source of truth.
- Economic parameters (bond, challenge window, slash amounts) are configurable and governance-adjustable later.
- Trustless deterministic replay is a staged follow-on for eligible runtime classes, not required for initial rollout.
