# BTCPC Go-Live Test Plan

Date: 2026-04-08

Purpose: prove BTCPC is ready to serve inference reliably from other PCs, not just on the primary development machine.

## Goal

Before public or multi-PC rollout, verify:

1. a fresh machine can install and start cleanly
2. the API is reachable from another PC
3. model listing and inference work end-to-end
4. billing / proofs / job lifecycle behave correctly
5. common failure cases fail clearly and recover cleanly

## Environments

### E1. Primary node

The current machine running:
- API
- MongoDB access
- P2P connectivity
- Ollama backend access

### E2. Clean client PC

A second machine with:
- no dev-only shell state
- no preexisting BTCPC processes
- network access to the primary node

This machine is used to prove real client behavior.

### E3. Optional second miner PC

A third machine, if available, used to verify:
- miner startup on another host
- P2P discovery and claim flow
- multi-node behavior

## Test phases

## Phase 1. Install and startup validation

Run on a clean client/miner PC.

### Checks

- clone repo
- install dependencies
- create `.env`
- start API or miner without manual patching
- verify no unexpected port conflicts

### Pass criteria

- setup works from docs alone
- process starts without code edits
- logs show successful DB and network bootstrap

### Record

- OS and version
- Node version
- npm version
- exact `.env` deltas from `.env.example`
- startup logs

## Phase 2. Basic API reachability

Run from another PC against the live API host.

### Checks

- `GET /health`
- `GET /`
- `GET /v1/models` with auth
- `GET /v1/pricing`
- `GET /v1/network/models`

### Pass criteria

- endpoints respond from a different PC
- health is public and stable
- model list matches reachable backend
- pricing/network endpoints return valid JSON

## Phase 3. Inference smoke tests

Run from another PC using real HTTP requests.

### Test set

1. short literal response
   - prompt: `Reply with exactly: BTCPC inference ok`
2. short factual response
   - prompt: `What is 2 + 2?`
3. short structured response
   - prompt: `Return valid JSON with key "status" = "ok"`
4. medium paragraph
   - prompt: short explanation request
5. unavailable model request
   - request a model not present on miners

### Pass criteria

- request completes successfully
- response text is non-empty
- usage and BTCPC proof metadata are present
- latency stays within expected range
- unavailable model returns a clean actionable error

### Important note

The current known issue is not empty output anymore. It is instruction fidelity on some models. For go-live, track:
- exactness of response
- reasoning leakage
- models that ignore strict output constraints

## Phase 4. Auth and project-key validation

### Checks

- missing bearer token
- invalid bearer token
- relay key behavior
- verified `btcpc_` project key
- unverified project key
- zero-balance project

### Pass criteria

- auth failures return correct HTTP code and JSON shape
- verified keys can run inference
- billing failures are explicit and correct

## Phase 5. Billing and ledger validation

Use a real funded project or controlled test project.

### Checks

- project balance before inference
- project balance after inference
- cost matches pricing response closely
- proof hash recorded
- job state stored and completed

### Pass criteria

- balance moves exactly once per successful request
- failed request does not silently burn funds
- proof metadata is present and linked to the request

## Phase 6. Multi-PC miner validation

Run when at least one additional miner PC is available.

### Checks

- miner on second PC joins network
- miner sees demand
- miner claims jobs
- API receives results from remote miner
- remote miner logs successful work

### Pass criteria

- second PC contributes inference successfully
- job lifecycle works across hosts, not just locally
- no host-specific hardcoded assumptions block remote nodes

## Phase 7. Failure and recovery tests

### Induced failures

1. Ollama unavailable
2. MongoDB unavailable
3. P2P port conflict
4. model missing on miner
5. API restart during traffic

### Pass criteria

- failures are visible in logs
- API returns useful errors
- restart restores service
- no stuck jobs or silent empty successes

## Phase 8. Soak test

Run 25-100 requests from another PC over time.

### Measure

- success rate
- average latency
- p95 latency
- empty-response rate
- malformed-response rate
- auth failure rate
- proof creation rate

### Pass criteria

- no empty completions
- no unexplained 5xx spikes
- stable latency under light concurrent use

## Manual test matrix

Minimum matrix before wider rollout:

1. Windows client PC -> live API
2. Linux client PC -> live API
3. macOS client PC -> live API
4. Optional: second Linux or Windows miner PC -> P2P network

If all three desktop client OSes are not available, test at least:
- one non-primary Linux box
- one Windows box

## Required scripts and artifacts

Before go-live, create or standardize:

1. `scripts/smoke-api.sh`
   - health
   - models
   - pricing
   - one inference request

2. `scripts/smoke-inference.js`
   - repeated prompt set
   - JSON summary of pass/fail, latency, and output quality

3. `docs/GO_LIVE_CHECKLIST.md`
   - operator checklist for release day

4. Request/response fixtures
   - save representative successful and failed responses

## Go-live gates

Do not go live to other PCs until all of these are true:

- [ ] `/health` works publicly
- [ ] `/v1/models` works from another PC
- [ ] live completion requests return non-empty content reliably
- [ ] project-key auth paths are verified
- [ ] billing and proof metadata are verified
- [ ] one clean-machine startup test passes
- [ ] one remote-client smoke test passes
- [ ] one failure/recovery exercise passes
- [ ] soak test shows no empty outputs or unexplained 5xx errors

## Immediate next testing tasks

1. Build `scripts/smoke-api.sh`
2. Build `scripts/smoke-inference.js`
3. Add one integration test path for `/health`, `/v1/models`, and `/v1/chat/completions`
4. Add an output-quality assertion for strict prompts
5. Run the smoke suite from a second PC on the same network

## Known risks entering testing

- P2P port conflicts if an older local node is already bound
- some models may return reasoning-style output instead of strict literal output
- auth and route ordering bugs can break operational endpoints if middleware order regresses
- live behavior can differ between local fallback inference and miner-routed inference
