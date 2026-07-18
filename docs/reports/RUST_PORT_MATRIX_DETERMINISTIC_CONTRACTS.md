# Rust Port Matrix For Deterministic Contracts

## Scope and intent

This report maps HONE modules into:

- `Port now` (consensus-critical, deterministic state transition logic)
- `Port next` (execution and settlement logic that must become deterministic for on-chain contracts)
- `Keep as edge` (API, UI, integration, and operational adapters that should not be consensus authority)

The goal is one deterministic chain platform in Rust so third parties can deploy contract code safely and predictably.

## Current architecture signal

- `src/chain` is consensus/state heavy and currently JS-first.
- `src/services` is large and mixes deterministic ledger concerns with non-deterministic external integration.
- `src/p2p` has a Rust sidecar option but JS still owns major protocol logic.
- Existing Rust in this repo is mostly:
- `rust/chain-core` (new scaffold)
- `android/rust/hone-miner` (mobile miner path)

## Port now: consensus and state authority

These modules should be reimplemented in Rust first and become canonical:

- `src/chain/stateStore.js`
- `src/services/ledger.js`
- `src/chain/stateManager.js`
- `src/chain/replay.js`
- `src/chain/block.js`
- `src/chain/blockStore.js`
- `src/chain/blockProposal.js`
- `src/chain/finalizationConsensus.js`
- `src/chain/authorityRotation.js`
- `src/chain/nodeRegistry.js`
- `src/chain/rewardEngine.js`
- `src/chain/rewardSettlement.js`
- `src/chain/sparseMerkleTree.js`
- `src/p2p/mempool.js`
- `src/p2p/mempoolFeeMarket.js`
- `src/p2p/messageAuth.js`

Why these first:

- They define ledger entry acceptance and ordering.
- They mutate balances and stake state.
- They produce canonical hashes, roots, and finality outputs.
- They determine whether replay is valid.

## Port next: deterministic execution and settlement

These should move to Rust after state authority is moved:

- `src/chain/sensorRewards.js`
- `src/chain/dynamicSensorRewards.js`
- `src/chain/verifierEngine.js` (deterministic scoring and settlement parts only)
- `src/services/slashing.js`
- `src/services/bridgeRegistry.js`
- `src/services/chainMonitor.js` (deterministic claim consumption path only)
- `src/services/inferenceMarket.js` (state transition core only)
- `src/services/serviceRegistry.js`
- `src/services/statefulServiceRegistry.js`

Important split:

- Deterministic state transition logic goes Rust-core.
- Non-deterministic external fetch, webhooks, and local system calls stay out of consensus runtime.

## Keep as edge adapters (not consensus authority)

These can remain JS or be ported later for operational reasons, but should never define canonical chain truth:

- `src/routes/*`
- `src/controllers/*`
- `src/explorer/*`
- `src/services/modelManager.js`
- `src/services/modelRegistry.js`
- `src/services/audioTranscriber.js`
- `src/services/browserRunner.js`
- `src/services/autoUpdater.js`
- `src/services/secretStore.js`

Rule:

- Edge services may submit signed transactions.
- Edge services must not decide canonical state transition outcomes.

## Deterministic contract platform design (Rust)

### Required crates

- `hone-chain-core`:
- ledger state model
- entry dispatcher
- canonical serialization and hashing
- replay validator
- reward accounting

- `hone-vm`:
- WASM contract execution
- fuel-metered deterministic execution
- restricted deterministic host API

- `hone-state-db`:
- RocksDB or sled backend
- deterministic key codec
- snapshot and proof export

### Contract transaction types

Add explicit entry families in Rust core:

- `CONTRACT_DEPLOY`
- `CONTRACT_CALL`
- `CONTRACT_UPGRADE`
- `CONTRACT_EVENT`
- `CONTRACT_STORAGE_WRITE` (internal projection event, not user-submitted)

### Deterministic runtime rules

- No wall-clock reads inside contracts.
- No random syscalls in contracts.
- No filesystem/network host functions.
- Epoch and block metadata passed as explicit deterministic inputs.
- Canonical serialization for all call inputs and outputs.
- Fuel-based execution limits for deterministic interruption.
- Contract state stored under namespaced keys in chain state.

### Host functions allowed

- `chain.get_balance(account, token)`
- `chain.get_epoch()`
- `chain.transfer(to, token, amount)` (returns deterministic effect record)
- `chain.read_state(key)`
- `chain.write_state(key, value)`

Any host call not in this list is rejected at validation time.

## Immediate deprecations after Rust cutover

After Rust equivalents are production-ready, deprecate JS authority paths:

- `src/chain/stateStore.js` as canonical mutator
- `src/services/ledger.js` as canonical dispatcher
- `src/chain/replay.js` as canonical replay validator
- `src/chain/rewardEngine.js` as canonical reward splitter
- JS mempool ordering authority in `src/p2p/mempool.js`

Keep JS shims only for compatibility during migration windows.

## Acceptance criteria for migration

- Same block stream yields identical final state root across at least 3 independent nodes.
- Replay rejects any negative balance or invalid stake transition.
- Contract call replay is byte-for-byte deterministic across nodes.
- All consensus-critical tests have Rust-first parity coverage.
- JS route/controller layers can be removed without changing chain state outcomes.

## Recommended migration order

1. Finish `hone-chain-core` transaction dispatcher and state model.
2. Port replay and block validation into Rust and make JS use Rust outputs.
3. Port reward engine and settlement into Rust.
4. Port mempool ordering and message auth checks into Rust.
5. Introduce `hone-vm` with deterministic WASM policy.
6. Add contract tx types and storage namespace rules.
7. Switch Node to adapter-only role for chain paths.
8. Remove deprecated JS authority modules.

