# Phase 99: Decentralized Runtimes (HTTP + Worker + Stateful)

## Goal
Enable HONE users to run decentralized services with durable scheduling, multi-host failover, and slashable cryptographic accountability, using OCI and WASM runtime targets.

## Success Criteria
- Runtime lifecycle entries are accepted and deterministically applied.
- Jobs execute from durable queue with lease-based host claiming.
- Host failure does not orphan runtime jobs.
- Attestations are signed, challengeable, and slash-integrated.
- Deadman-style timer workflows can be expressed as durable runtime jobs.

## Scope
In scope:
- Runtime ledger entry families
- Runtime state maps + finality snapshot/hydration support
- Runtime manifest and host capability registration
- Lease scheduler + failover
- Attestation/challenge/slashing integration
- Runtime API surface

Out of scope for this phase:
- Full deterministic trustless replay for every runtime type
- Governance redesign of economics
- Frontend UX redesign

## Decisions Locked
- Trust model: crypto-verifiable + slashable first
- Runtime families: HTTP, worker, and stateful session services
- Placement targets: both OCI and WASM

## Work Plan

### Step 1: Protocol + State Plumbing
- Add runtime entry enums/types and validation rules.
- Extend `applyEntry`/state reducers for runtime maps.
- Extend finality snapshot write/read with runtime maps.
- Add deterministic ID derivation helpers.

Exit checks:
- entries parse/validate/apply
- runtime state survives restart from finality snapshot

### Step 2: Manifest + Host Capabilities
- Add runtime manifest schema and CID reference checks.
- Add host capability registration and read views.
- Gate runtime deployment by host capabilities and stake/slash readiness.

Exit checks:
- runtime deploy rejects incompatible hosts
- runtime metadata is queryable from state

### Step 3: Lease Scheduler + Durable Jobs
- Add durable runtime job records and due-time handling.
- Implement lease claim/renew/expire state machine.
- Enforce one active lease per runtime job class.

Exit checks:
- concurrent claim race yields single winner
- expired lease can be safely reclaimed

### Step 4: Attestation + Challenge + Slashing
- Add signed attestation payload and verification path.
- Add challenge submission and adjudication path.
- Integrate offense mapping into slashing subsystem.

Exit checks:
- invalid attestation triggers challenge/slash path
- duplicate claims are slashable and blocked from settlement

### Step 5: API + Reference Runtime Flow
- Add runtime APIs for register/deploy/enqueue/claim/attest/challenge/status.
- Implement reference Deadman-style runtime flow using durable jobs.
- Ensure legacy single-process timer assumptions are not required for correctness.

Exit checks:
- reference flow works across node restart and host failover

### Step 6: Hardening + Adversarial Testing
- Add chaos tests for worker/node failure and partition timing.
- Add replay/idempotency tests for duplicate network submissions.
- Add observability counters for claims, lease expiry, challenges, slashes.

Exit checks:
- no duplicate settlement under stress
- recovery and failover are deterministic and auditable

## Test Matrix
- Unit: validation, IDs, lease transitions, signature checks
- Integration: claim races, failover, challenge adjudication
- E2E: runtime deployment to failover completion, Deadman durable flow
- Regression: no snapshot compatibility regressions

## Risks and Mitigations
- Risk: duplicate execution under partition/race
  - Mitigation: strict lease TTL + state-level idempotency keys
- Risk: false slashing from weak evidence shape
  - Mitigation: explicit evidence schema and challenge window timing
- Risk: operational complexity for mixed OCI/WASM
  - Mitigation: capability flags and class-specific policy gates

## Deliverables
- Runtime protocol/state implementation
- Runtime APIs
- Scheduler and verification modules
- Test suites for race/failover/challenge
- Updated docs and operator guidance
