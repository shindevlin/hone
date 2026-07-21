# HONE Agent Orchestration Layer

## Decision
Agent Lightning is useful to HONE as a reference architecture, not as code to import into the Rust node.

HONE needs the same general control-plane concepts, but implemented in Rust with HONE identity, signed attestations, verifier challenges, and reward safety.

## Rust Crate
Path:

```text
rust/hone-orchestrator
```

The crate defines:
- `RuntimeResource`: versioned prompts, models, API catalog snapshots, WASM modules, containers, policies.
- `RuntimeJob`: a queued unit of useful work.
- `RuntimeAttempt`: a worker claim/attempt with retry and timeout state.
- `RuntimeSpan`: structured trace event for tool calls, inference, API requests, operations, rewards, and exceptions.
- `RuntimeAttestation`: worker output commitment and trace-root proof envelope.
- `RuntimeWorker`: worker account, endpoint, stake, status, and capabilities.
- `InMemoryOrchestrator`: test/handoff scheduler that exercises the model without chain integration.

Binary:

```text
hone-orchestratord
```

Current command:

```bash
cargo run --manifest-path rust/hone-orchestrator/Cargo.toml -- \
  run-api-tool \
  --catalog /mnt/btcpc-storage/catalogs/public-apis.snapshot.json \
  --category Weather \
  --query Open-Meteo \
  --out /tmp/hone-api-tool-report.json
```

This command:
- loads a `hone-api-catalog` snapshot
- selects a no-auth HTTPS API candidate
- publishes the catalog snapshot as a `RuntimeResource`
- enqueues an `ApiTool` `RuntimeJob`
- claims it with a local `RuntimeWorker`
- probes the selected URL
- records an `ApiRequest` `RuntimeSpan`
- emits a `RuntimeAttestation` when the HTTP result is successful

## Agent Lightning Mapping
| Agent Lightning Concept | HONE Rust Equivalent | HONE Difference |
|---|---|---|
| LightningStore | Orchestration store / chain-backed runtime store | Must be challengeable and eventually consensus-aware |
| Rollout | RuntimeJob | Covers inference, API tools, services, background work |
| Attempt | RuntimeAttempt | Bound to worker identity and reward/slash policy |
| Span | RuntimeSpan | Hashable, privacy-aware, signer-bound trace event |
| Resource | RuntimeResource | CID/hash-backed prompt/model/catalog/runtime artifact |
| Reward emitter | RuntimeSpan::RewardSignal / RuntimeAttestation | Must settle through HONE economics, not local trainer state |
| Runner | RuntimeWorker | Must advertise capabilities and stake/slash status |

## Integration Path
1. Keep `hone-orchestrator` as a standalone Rust model until the branch is clean enough to wire into `hone-node`.
2. Use it to define API-compatible JSON for jobs, attempts, spans, and attestations.
3. Add node-side ledger entries for these objects only after invariants are finalized.
4. Add a worker sidecar that pulls jobs, runs local tools/models/services, emits spans, and submits attestations.
5. Add verifiers/challengers before enabling automatic rewards.

## Trust Model
Phase 1 is not fully trustless. It is signed, auditable, and challengeable.

Minimum required before rewards:
- deterministic job IDs
- signed worker identity
- immutable resource IDs
- output commitments
- trace-root commitments
- challenge window
- duplicate attempt prevention
- timeout/retry policy

## Python Sidecars
Agent Lightning can still be useful as an optional Python sidecar for experimentation:
- train prompts or agents off-chain
- run local eval loops
- produce traces that HONE converts into `RuntimeSpan`
- submit signed outputs back to the Rust orchestrator

It should not control HONE rewards or consensus state directly.
