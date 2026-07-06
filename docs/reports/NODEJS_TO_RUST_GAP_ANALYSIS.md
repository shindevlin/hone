# Node.js → Rust Gap Analysis
*BTCPC / Freeport Protocol — Migration Completeness Report*
*Generated: 2026-04-29*

---

## EXECUTIVE SUMMARY

The Rust rewrite is approximately **40-50% feature-complete** relative to the Node.js codebase. The core chain state machine, ed25519 transaction validation, libp2p networking, inference job marketplace, WASM contract runtime, and the emission schedule have all been ported with genuine improvements (type safety, RocksDB persistence, proper era-based doubling model). However, a substantial set of subsystems that gave the Node.js implementation its richness — from the block explorer to the cross-chain bridge, from the fee market to the finalization consensus protocol, from the Telegram bot to the MCP server — are entirely absent from the Rust codebase.

The single largest category of missing logic is **reward-distribution sophistication**: the Node.js `blockProposal.js` + `rewardEngine.js` combo has over 600 lines of scarcity multipliers, activity-gated emission, stake-weighted miner scoring, testnet carveouts, sensor/gateway sub-pools, service hosts, and yield-staker splits. The Rust clock emits flat `MineReward` and `ClockReward` entries but does **none** of this proportional, multi-role splitting.

The most critical incompatibility: **Node.js and Rust produce fundamentally different per-epoch reward amounts** (approximately 12x apart). A mixed network cannot reach consensus.

---

## DOMAIN-BY-DOMAIN GAP ANALYSIS

---

### 1. Wallet & Key Management

**Node.js had:**
- `src/controllers/walletController.js` — full account creation, balance lookup, multi-token balances, transfer, staking, key management via HTTP
- `src/controllers/authController.js` — JWT-based authentication, session management, TOTP 2FA
- `src/controllers/recoveryController.js` — account recovery flow
- `src/services/privateAuthorization.js` — secp256k1-based private key authentication for EVM, Solana, TON, Bitcoin, Lightning payment verification, multi-factor threshold signing
- `src/services/activeKeyAuthorization.js` — per-chain key authorization layer
- `src/routes/walletRoutes.js`, `totpRoutes.js`, `recoveryRoutes.js` — full HTTP API surface
- `src/services/accountCreation.js` — dedicated account provisioning service

**Rust has:**
- `rust/hone-node/src/tx.rs` — ed25519 signature validation for Transfer, Stake, Unstake, AccountUpdateKey, EpochSeal, and all inference entries
- `rust/hone-node/src/api.rs` — `/api/transfer`, `/api/stake`, `/api/unstake`, `/api/account/create`, `/api/account/update-key`
- `rust/hone-cli/src/key.rs` — local keypair generation

**Gap / What's missing:**
- Zero JWT/session-based authentication — the Rust API accepts unsigned entry submissions with no auth gate
- No TOTP 2FA subsystem
- No account recovery flow
- No `privateAuthorization` equivalent: secp256k1/EVM/Solana/TON/Lightning challenge-response enrollment is entirely absent
- No multi-factor threshold signing
- No dedicated account creation service (account creation is a bare `AccountCreate` entry in Rust, with no validation of reserved names, no onboarding flow, no email verification)
- Reserved name protection (420 premium names) from `src/services/reservedNames.js` has no Rust equivalent
- The Node.js stateStore tracks per-account metadata fields (public_keys map, chain_addresses, heartbeat_epoch, device keys); Rust stores a flat JSON blob per account

**Severity: HIGH**

---

### 2. Transaction Types & Validation

**Node.js had:**
- 50+ ledger entry types applied via `stateStore.applyEntry()` — including NFTs, token creation, project records, delegate balance, faucet, community models, sponsored stakes, etc.
- `src/chain/stateStore.js` — handles `ACCOUNT_CREATE`, `TRANSFER`, `STAKE`, `UNSTAKE`, `MINING_REWARD`, `CLOCK_REWARD`, `STORAGE_REWARD`, `SERVICE_REWARD`, `IOT_REWARD`, `NFT_MINT`, `NFT_TRANSFER`, `TOKEN_CREATE`, `TOKEN_TRANSFER`, `PROJECT_CREATE`, `DELEGATE`, `UNDELEGATE`, `ESCROW_LOCK`, `ESCROW_RELEASE`, `DATA_BUY`, `MODEL_COMMUNITY_REGISTER`, `DEVICE_YIELD_STAKE`, etc.
- Nonce validation, anti-replay via seenEntries Set
- `src/p2p/mempoolFeeMarket.js` — fee-per-byte priority sorting, block size cap enforcement
- `src/p2p/mempool.js` — per-account pendingDebit tracking, double-spend prevention, MAX_MEMPOOL_SIZE cap

**Rust has:**
- `rust/hone-node/src/tx.rs` — strong validation for Transfer, Stake, Unstake, AccountCreate, AccountUpdateKey, EpochSeal, and all inference entries
- `rust/hone-types/src/entry.rs` — LedgerEntry enum with ~50 variants including commerce, sensors, LinkGit, testnet, inference
- nonce-check + bump on Transfer/Stake/Unstake
- No dedicated mempool module

**Gap / What's missing:**
- **No mempool implementation**: Rust has no `pending` transaction pool. Entries submitted to the API are applied immediately to chain state or rejected — there is no mempool queue, no fee-per-byte sorting, no block-size cap enforcement, no max-mempool-size limit, and no per-account pending-debit tracking to prevent double-spends between entries that haven't been block-included yet.
- **No fee market**: `mempoolFeeMarket.js` fee-per-byte selection and `blockSizeCap.js` have no Rust equivalent.
- **NFT types**: `NFT_MINT`, `NFT_TRANSFER`, `NFT_SOUL_BIND`, `NFT_TIME_LOCK`, `NFT_EVOLVE` are missing from the Rust LedgerEntry enum (type variants exist in entry.rs but `apply_entry()` in chain.rs does not handle them).
- **Token creation**: `TOKEN_CREATE`, `TOKEN_TRANSFER` (custom fungible tokens) — apply logic absent.
- **Delegation**: `DELEGATE`, `UNDELEGATE`, separate `delegatedReceived` balance tracking — absent.
- **Community model registry**: `MODEL_COMMUNITY_REGISTER` and associated state are absent.
- **Sponsored stakes**: No Rust equivalent.
- **Project records**: `PROJECT_CREATE`, `PROJECT_REVENUE_SPLIT` — absent.

**Severity: HIGH**

---

### 3. Chain / Block / Epoch Mechanics

**Node.js had:**
- `src/chain/block.js` — binary-serializable block header with SHA-256 of all fields, Merkle root computation, validateBlock() against parent
- `src/chain/blockchain.js` — in-memory chain store keyed by hash and epoch, full-chain validateChain() walk, duplicate rejection
- `src/chain/blockStore.js` — disk persistence: writeBlock/readBlock with binary header + JSON payload, writeFinality/readFinality, hashLedgerEntry, hashComputeProof, pruneBeforeFinality
- `src/chain/blockSizeCap.js` — MAX_BLOCK_PAYLOAD_BYTES enforcement
- `src/chain/stateHash.js` — Merkle of balances + account states for snapshot
- `src/chain/replay.js` — full chain replay from disk on startup, finality snapshot fast-forward
- `src/chain/finalizationConsensus.js` — multi-proposer consensus: 30s proposal window, group-by-consensus-hash, majority wins, deterministic `hashRewards()` function, min-consensus-sources guard
- `src/chain/authorityRotation.js` — epoch eligibility check, permissioned/permissionless tiers, genesis miner always eligible
- `src/chain/stateManager.js` — Sparse Merkle Tree of account states, generates state root for finality snapshots, `loadFromFinality`, `generateFinalitySnapshot`
- `src/chain/sparseMerkleTree.js` — full 32-level SMT with O(1) root, ~1KB inclusion proofs, `usernameToIndex`, `hashState`, `getProof`, `verifyProof`
- `src/chain/epochFinalizer.js` — applies winning proposal to ledger + stateStore + block files, writes finality snapshot every FINALITY_INTERVAL epochs, rolling commitment hash, finalityAnchoring, LucidPruning of old block files, mempool clearance

**Rust has:**
- `rust/hone-node/src/chain.rs` — `apply_entry()` for all entry types, `apply_block_entries()`, `current_epoch()` tracking via RocksDB meta
- `rust/hone-node/src/finalize.rs` — `run_finalizer()`, `finalize_epoch()`, proxy state root (SHA-256 of block hash + epoch bytes), `redirect_unearned_rewards()` to recycle fund
- `rust/hone-types/src/block.rs` — Block struct with binary header encoding

**Gap / What's missing:**
- **No full validateChain() walk** — no end-to-end chain integrity verification from genesis to tip
- **Proxy state root only** — Rust uses `SHA256(block_hash || epoch)` as the state root. Node.js uses a full Sparse Merkle Tree over all account balances, producing a cryptographically sound inclusion-proof-capable state root. The Rust version's state root is not verifiable by light clients.
- **No Sparse Merkle Tree** — zero inclusion proof capability, no `getProof()`, no `verifyProof()`
- **No finalizationConsensus module** — the multi-proposer 30s proposal window, consensus-hash grouping, and majority-wins logic are entirely absent. The Rust clock seals epochs but does not run proposal collection.
- **No `hashRewards()` deterministic function** — the canonical function used by all nodes to confirm they agree on a reward set has no Rust equivalent.
- **No authority rotation** — permissioned/permissionless node tier logic is absent.
- **No LucidPruning** — old block files are never pruned.
- **No finality anchoring** — `src/chain/finalityAnchoring.js` posts finality snapshots to Hive/cross-chain anchors; absent in Rust.
- **No rolling commitment chain** — the chain of SHA-256(prev_commitment || state_root) that makes finality snapshots tamper-evident is absent.
- **Mempool clearance** — Node.js epochFinalizer explicitly clears included mempool transactions from the pool; Rust has no mempool to clear.
- **Block size cap** — absent in Rust.

**Severity: HIGH**

---

### 4. Consensus & Finalization

**Node.js had:**
- `src/chain/finalizationConsensus.js` — full multi-node proposal collection, 30s collection window, proposal grouping by `consensus_hash`, majority-wins resolution, single-miner fallback, `onResolved` callbacks, configurable `MIN_CONSENSUS_SOURCES`
- `src/chain/clockConsensus.js` — clock heartbeat tracking, epoch boundary detection
- `src/chain/stateManager.js` — SMT-based state root for deterministic equality check
- `src/chain/verifierEngine.js` — cross-checks compute proofs, issues VERIFY_RESPONSE gossip
- `src/chain/computeVerifier.js` — validates account balances against known-good state

**Rust has:**
- `rust/hone-node/src/clock.rs` — quorum-based seal collection: receives EpochSeal gossip, applies timestamp median filtering, outlier rejection, 51% quorum, observer mode, clock reputation scoring
- Clock module properly handles single-seal isolated mode

**Gap / What's missing:**
- **No reward-proposal consensus** — the clock quorum in Rust only decides *whether* an epoch sealed, not *what the rewards are*. There is no equivalent to `finalizationConsensus.js` that collects multiple proposers' reward splits, groups by consensus_hash, and picks the majority.
- **No verifierEngine** — no module cross-checks compute proofs against claimed work values and issues verification gossip
- **No computeVerifier** — no tool to validate account balances against expected state from block data
- **No peer-source count guard** — Node.js `MIN_CONSENSUS_SOURCES` prevents a single isolated node from self-finalizing on mainnet; Rust `HONE_CLOCK_QUORUM` is separate from reward-proposal consensus which is entirely absent.

**Severity: HIGH**

---

### 5. Mining & Reward Distribution

**Node.js had:**
- `src/chain/blockProposal.js` (~500 lines):
  - 6 role score categories: miners, verifiers, clocks, storage hosts, sensors+gateways, service hosts
  - Scarcity multiplier (2.5x when < 3 nodes in a role)
  - Anti-self-credit: proposer's heartbeats only count if witnessed by external peer
  - Stake-weighted miner scoring: `weight = min(sqrt(stake / MIN_STAKE), 10)`, fallback for bootstrap
  - Activity-gated emission: `activityRatio = min(1, max(0.01, totalScore / FULL_ACTIVITY_SCORE))`
  - Testnet carveout: 0.1% off the top to testnet nodes
  - Per-unit work maps: verifierWork, storageWork, sensorWork, gatewayWork, serviceWork
  - Service host rewards from serviceRegistry
  - `_splitPool()` helper: proportional-to-work or equal-split fallback
  - IoT sub-pool: sensor vs gateway sub-split by raw score
  - Deterministic sort + consensus hash via `finalizationConsensus.hashRewards()`
- `src/chain/rewardEngine.js` (~390 lines):
  - Phase 2 endowment: recycle rate computed at first Phase 2 epoch from `blockReward / recycleBalance`
  - Tool multiplier via `computeToolMultiplier(p.tools_used)`
  - Per-proof `effective_work = work_value × tool_multiplier`
  - Device yield staking split: 70/20/10 (owner/top-staker/recycle) with 10-slot `SLOT_MULTIPLIERS`; 90/10 fallback when no stakers
  - `SENSOR_YIELD_REWARD`, `DEVICE_YIELD_RENT` entries with rent_mode ("stake" vs "earnings")
  - `SENSOR_EPOCH_REWARD`, `SENSOR_RENT_COLLECTED` reward types
  - Storage query bonus: up to 2x multiplier for `queries_served`
  - Clock reward proportional to heartbeat count (not equal split)
- `src/mining/rewardDistribution.js` — additional per-role work scoring constants
- `src/services/nanoRewards.js` — small sub-BTCPC rewards for micro-contributions
- `src/services/sensorDataBilling.js` — purchase-triggered sensor rewards

**Rust has:**
- `rust/hone-node/src/main.rs` `emit_epoch_rewards()` function: collects Mine entries for the epoch from RocksDB, computes `inference_score()` per miner, distributes `MineReward` proportionally; emits `ClockReward` equally to all signing clocks; emits `StorageReward` proportional to `bytes_proven`, `SensorReward` proportional to `reading_count`, `VerifierReward` proportional to verifications
- `rust/hone-types/src/emission.rs` — `inference_score()`, `hw_tier_weight()`, `model_weight()`, `clock_reward_at()` era scaling

**Gap / What's missing:**
- **No activity-gated emission** — Rust always distributes `block_reward_at(epoch)` in full regardless of network activity. Node.js scales actual emission down to 1% when the network is idle.
- **No scarcity multiplier** — Rust applies no 2.5x bonus when fewer than 3 nodes participate in a role.
- **No stake-weighted miner scoring** — Rust splits miner rewards by raw `inference_score`. Node.js weights by `min(sqrt(stake / MIN_STAKE), 10)` and applies a 50% penalty for unverified miners.
- **No anti-self-credit for clock** — Rust does not filter the proposer's own heartbeats based on external witness count.
- **No testnet carveout** — the 0.1% testnet pool (TestnetReward) is defined in types but not emitted by `emit_epoch_rewards()`.
- **No Phase 2 endowment** — when supply exhausts, Rust emits nothing. Node.js activates a recycle-rate regime and computes ongoing rewards from the recycle balance.
- **No device yield staking split** — `SLOT_MULTIPLIERS`, per-device owner/staker/recycle splitting, `DEVICE_YIELD_RENT` entries are all absent.
- **No tool multiplier** — `computeToolMultiplier()` adjusts miner rewards based on MCP tools used; absent in Rust.
- **No storage query bonus** — Rust splits storage rewards by `bytes_proven` only; no `queries_served` 2x bonus.
- **No service host rewards** — `ServiceReward` type exists in entry.rs but `emit_epoch_rewards()` does not collect or distribute it.
- **No nano-rewards** — absent.
- **No sensor purchase-triggered rewards** — `sensorDataBilling.js` has no equivalent.

**Severity: HIGH**

---

### 6. Inference Job Lifecycle

**Node.js had:**
- `src/inference/verifier.js` — network-size-adaptive verifier selection (1/3/5/7 tiers), `shouldVerifyJob()` deterministic coverage probability, quality assessment (MIN_TOKENS, MIN_OUTPUT_LENGTH, MAX_REPEAT_RATIO, MIN_UNIQUE_WORDS)
- `src/inference/ensembleCoordinator.js` — Mode A/ensemble inference: multiple nodes run the same prompt, consensus declared when ≥ min_ensemble_size produce the same `result_hash`, consensus nodes earn 1.5x bonus, partial contributors earn 0.5x, temperature=0 for determinism
- `src/inference/agentSession.js`, `agentEvents.js` — agent session tracking, tool execution events
- `src/inference/session.js` — session lifecycle with timeout
- `src/inference/shardRegistry.js` — inference sharding: large jobs partitioned across multiple GPU nodes
- `src/inference/providers.js` — multi-provider routing: Ollama, OpenAI, Anthropic, custom endpoints
- `src/inference/protocol.js` — P2P gossip protocol for inference job routing
- `src/inference/encrypted.js`, `encryptedClient.js` — end-to-end encrypted inference (client encrypts prompt to worker's public key)
- `src/inference/crypto.js` — inference-specific crypto helpers
- `src/services/inferenceMarket.js` — standalone inference marketplace state
- `src/services/reviewerSelection.js` — selects human reviewers for disputed jobs

**Rust has:**
- `rust/hone-node/src/inference.rs` — full on-chain state machine: Posted → Awarded → Completed → Verified → Paid, with Disputed → Claimed → Reviewed → Paid path, `NodeReputation` (jobs_accepted/completed/failed, avg latency, score 0-10000), `jobs_ready_to_award()`, `jobs_claim_expired()`, `jobs_past_deadline()`, `select_best_bid()`, `build_pay_entry_happy()`, `build_pay_entry_disputed()`, `build_pay_entry_nofee()`
- `rust/hone-node/src/inference_daemon.rs` — background daemon that processes job state transitions (award, pay, expire)

**Gap / What's missing:**
- **No ensemble/Mode A** — `ensembleCoordinator.js` with consensus bonuses and partial multipliers has no Rust equivalent. The Rust `InferenceJobPost.mode` field accepts "ensemble" but no ensemble coordinator exists.
- **No sharding** — `shardRegistry.js` distributes large jobs across multiple GPU nodes; absent in Rust.
- **No encrypted inference** — end-to-end encryption of prompts/results (client encrypts to worker's memo key, verifier requests decryption key after award) is absent.
- **No multi-provider routing** — Rust always hits a single Ollama URL. Node.js routes to Ollama/OpenAI/Anthropic/custom via `providers.js`.
- **No network-size-adaptive verifier selection** — Rust assigns verifiers via bids; Node.js uses deterministic coverage probability that scales with network size.
- **No quality assessment heuristics** — Rust verifier only checks `verdict` field value. Node.js checks `MIN_TOKENS`, `MIN_OUTPUT_LENGTH`, `MAX_REPEAT_RATIO`, `MIN_UNIQUE_WORDS`.
- **No agent session tracking** — agentSession/agentEvents are absent.

**Severity: MEDIUM-HIGH**

---

### 7. P2P Networking & Peer Discovery

**Node.js had:**
- `src/p2p/network.js` — WebSocket P2P with Noise_XX handshake for direct peers, plain JSON for relay connections, up to 50 peers, exponential backoff reconnect (max 60s), ping/pong keep-alive with zombie detection, Cloudflare relay fallback
- Multiple relay URL support (`HONE_RELAY_URLS`), hardcoded bootstrap peers for eclipse protection
- `src/p2p/protocol.js` — message types: HANDSHAKE, BLOCK_PROPOSAL, MINING_PROOF, INFERENCE_REVEAL, VERIFY_RESPONSE, CLOCK_HEARTBEAT, EPOCH_SEAL, BLOCK_ANNOUNCE, CHAIN_SYNC_REQUEST/RESPONSE, MEMPOOL_ENTRY, PEER_ANNOUNCE, FINALIZATION_PROPOSAL, EPOCH_FINALIZED, STORAGE_HEARTBEAT, SENSOR_READING, etc.
- `src/p2p/encryptedTransport.js` — Noise_XX protocol implementation with static keypair generation and pinning
- `src/p2p/messageAuth.js` — per-message authentication
- `src/p2p/chainSync.js` — block validation, chain height tracking, block cache, peer height comparison, full sync request/response cycle
- `src/p2p/address.js` — address normalization, `isConnectableP2PAddress()`
- Seen-message deduplication via seenMessages Set, flushed to disk on SIGTERM

**Rust has:**
- `rust/hone-node/src/net.rs` — libp2p 0.55: TCP + QUIC transport, gossipsub topics (btcpc/blocks, btcpc/entries, btcpc/seals, btcpc/sync), Kademlia DHT, Identify, Ping; peer store persisted to RocksDB; Kademlia re-bootstrap every 5 minutes when < 3 peers; block sync request/response via btcpc/sync topic
- `rust/hone-node/src/discovery.rs` — Hive + TON registry peer discovery, self-announce

**Gap / What's missing:**
- **No Cloudflare relay** — the Rust node has no fallback relay for NAT-punching. Node.js falls back to `wss://btcpc-relay.shindevlin.workers.dev/ws` when no direct peers are reachable.
- **No per-message authentication layer** — `messageAuth.js` anti-spoofing is absent from the Rust gossipsub handler.
- **No seen-message dedup persistence** — Rust does not flush a seen-message cache to disk on shutdown; restarts may re-process already-applied gossip entries.
- **No explicit block sync response handler** — Rust issues sync requests via gossipsub but the response path does not populate from local block store.
- **No fork resolution** — see section 4.

**Severity: MEDIUM**

---

### 8. HTTP API / REST Routes

**Node.js had (routes directory — 35+ route files):**
`amberPillRoutes`, `appealRoutes`, `auctionRoutes`, `blobRoutes`, `blobServeProofRoutes`, `botRoutes`, `bridgeRoutes`, `commerceRoutes`, `computerUseRoutes`, `delegationRoutes`, `dreamRoutes`, `explorerRoutes`, `faucetRoutes`, `finetuneRoutes`, `inferenceMarketRoutes`, `memoryRoutes`, `modelRoutes`, `nodeRoutes`, `oracleRoutes`, `peerCommerceRoutes`, `phoneMiningRoutes`, `phoneStorageRoutes`, `projectRoutes`, `publicRoutes`, `purchaseRoutes`, `recoveryRoutes`, `sensorDataRoutes`, `sensorRoutes`, `serviceRoutes`, `sessionMarketRoutes`, `sessionRoutes`, `stakingRoutes`, `storageRoutes`, `streamingRoutes`, `toolRegistryRoutes`, `toolRoutes`, `totpRoutes`, `userRoutes`

**Rust has:**
- `rust/hone-node/src/api.rs` (~1100 lines): GET `/api/balance`, `/api/balances`, `/api/account`, `/api/block`, `/api/latest`, `/api/stake`, `/api/epoch`, `/health`; POST `/api/transfer`, `/api/stake`, `/api/unstake`, `/api/account/create`, `/api/account/update-key`, `/api/contract/deploy`, `/api/contract/call`, `/api/contract/view`, `/api/inference/*` (7 endpoints), `/api/faucet/claim`, `/api/linkgit/*` (5 endpoints)

**Gap / What's missing:** ~25 of 35+ Node.js route files have no Rust equivalent. Entirely absent:
- Blob storage API (upload, download, CID proof)
- Bridge API (wrap/unwrap wBTCPC)
- Sensor/IoT data routes
- Oracle feed routes
- Node registration/management routes
- Commerce routes (storefront, products, orders, disputes)
- Delegation routes
- Dream routes (sub-unit transfers)
- Phone mining / phone storage routes
- Streaming inference routes
- Model registry routes
- Fine-tuning routes
- Memory/RAG routes
- Session marketplace routes
- Auction routes
- Appeal routes
- Project routes
- Service hosting routes
- Tool registry routes
- TOTP routes
- MCP tool endpoints
- Computer use routes

**Severity: HIGH**

---

### 9. Authentication & Authorization

**Node.js had:**
- `src/middlewares/auth.js` — JWT Bearer token validation, request-level authentication
- `src/middlewares/apiKeyAuth.js` — API key authentication for bot routes
- `src/middlewares/validate.js` — input sanitization: sanitizeTelegramId, sanitizeString, sanitizeAmount, sanitizePagination, validAddress, validAccountName, validChain, rejectObjectInputs
- `src/controllers/authController.js` — login, session, JWT issuance
- TOTP via `totpRoutes.js`
- Private auth: secp256k1 challenge-response for EVM/Solana/TON/Bitcoin wallets

**Rust has:**
- No auth middleware — the Rust API is completely open (no JWT, no API key checks)
- Basic input validation from Rust's type system and `serde` deserialization
- Signature verification on Transfer/Stake entries provides account-level authorization

**Gap / What's missing:**
- No HTTP-level authentication — any caller can submit entries to the Rust API
- No session tokens
- No TOTP
- No bot-route API key auth
- No input sanitization middleware equivalent to `validate.js`
- No private auth (EVM/Solana/TON wallet sign-in)

**Severity: HIGH**

---

### 10. Staking & Delegation

**Node.js had:**
- `src/controllers/stakingController.js` — staking lifecycle: stake, unstake, get pool info, stake penalty calculation
- `src/controllers/delegationController.js` — delegate tokens to another node, undelegate, track delegated-received balances
- `src/routes/stakingRoutes.js`, `delegationRoutes.js`
- stateStore tracks: stakes Map, delegations Map, delegatedReceived Map, sponsoredStakes Map
- Device yield staking (10 slots per sensor, per-slot multiplier table, rent modes)
- Staking thresholds for clock/miner eligibility from `blockProposal.js`
- Sponsored staking (third-party stake on behalf of another account)

**Rust has:**
- `Stake` / `Unstake` ledger entries validated in `tx.rs`
- `chain.rs` applies stake/unstake to `CF_STAKES` column family
- `DeviceYieldStake` / `DeviceYieldUnstake` entries exist in the type system but have no reward-distribution logic

**Gap / What's missing:**
- No delegation system — `DELEGATE`/`UNDELEGATE` entry types and state tracking absent
- No delegated-received balance tracking
- No sponsored staking
- No staking penalty calculation
- No stake threshold enforcement in reward distribution
- Device yield stake entries exist but their slot multipliers and reward splits are absent from `emit_epoch_rewards()`
- No API routes for staking pool queries or delegation management

**Severity: MEDIUM-HIGH**

---

### 11. Claims System

**Node.js had:**
- `src/claims/claimProofGenerator.js` — generates signed proofs (BTCPC-side signature + chain state proof) for wBTCPC cross-chain claims
- `src/claims/evmClaimSubmitter.js` — submits proofs to EVM wBTCPC contracts via raw JSON-RPC (Ethereum, Base, Arbitrum, Optimism); in-memory claim store keyed by "miner|chain|epoch"
- `src/claims/hiveClaimManager.js` — posts finality anchors and cross-chain proofs to Hive blockchain

**Rust has:**
- Zero claims system — no modules exist for any of: claim proof generation, EVM submission, or Hive claim management

**Gap / What's missing:** The entire claims subsystem is absent from Rust. Earned BTCPC cannot be bridged to EVM chains without the Node.js sidecar.

**Severity: HIGH**

---

### 12. Smart Contracts / WASM Runtime

**Node.js had:**
- No native WASM runtime — contracts were described in whitepaper but not implemented in Node.js

**Rust has:**
- `rust/btcpc-contract-runtime/` — full Wasmtime-based WASM execution engine with gas metering, host functions (storage r/w, transfer, logging, env), contract state
- `rust/btcpc-contract-sdk/` — SDK for writing contracts: collections, events, promises, storage, types, mock test harness
- `rust/btcpc-contract-sdk/examples/ft/` — fungible token example
- `rust/btcpc-contract-sdk/examples/nft/` — NFT example
- `rust/hone-node/src/contracts.rs` — `ContractEngine` integrated into the node

**Gap / What's missing:**
- This is an area where Rust **exceeds** Node.js — the contract runtime didn't exist in Node.js.
- No contract upgrade path (redeploy with state migration) is defined.
- CLI contract commands exist but no helper for composing complex calls or querying contract storage by prefix.

**Severity: LOW** (Rust leads here)

---

### 13. Block Explorer

**Node.js had:**
- `src/explorer/server.js` — full Express.js server on port 4242 with MongoDB integration
- Views: dashboard (network stats, recent blocks, active miners), block detail (with ledger entries), account (balance, transaction history, staking info), transactions (paginated), miners leaderboard, tokenomics (emission schedule, supply progress), blocks list, txDetail, userDashboard, visualizer (network graph), whitepaper renderer
- Real-time data from stateStore, blockStore, ledger, nodeRegistry
- Telegram mini-app integration (`/webapp` route)
- Pagination, filtering, search

**Rust has:**
- Zero block explorer — no web UI, no explorer routes, no paginated block/transaction views

**Gap / What's missing:** The entire block explorer is absent. The Rust API provides raw JSON endpoints but no human-readable UI.

**Severity: HIGH**

---

### 14. Telegram Webapp / Bot Integration

**Node.js had:**
- `src/telegram-webapp/index.html` — Telegram Mini App wallet UI
- `src/routes/botRoutes.js` — `/api/bot/*` endpoints for the Telegram bots
- Full-featured wallet bots in `~/repos/btcpcbot/` and `~/repos/btcpcwalletbot/`
- Explorer serves `/webapp` route for the Mini App

**Rust has:**
- `rust/btcpc-bot/src/` — thin Telegram bot (commands, main, api) that proxies to node API
- `rust/betchu-bot/src/` — betting bot with ESPN oracle and contract integration (new, no Node.js equivalent)

**Gap / What's missing:**
- No Telegram Mini App HTML in the Rust stack
- The Rust bots lack full wallet functionality of the Node.js bots
- No `/api/bot/*` endpoint layer in the Rust API
- `betchu-bot` oracle integration is new Rust-only functionality (a Rust advantage)

**Severity: MEDIUM**

---

### 15. Supervisor / Process Management

**Node.js had:**
- `src/supervisor/` — process supervisor for managing node daemon lifecycle
- `src/services/autoUpdater.js` — automatic self-update from published releases
- `src/services/backgroundTimers.js` — centralized timer management with `shouldStartBackgroundTimers()` guard (prevents timers in test environments)

**Rust has:**
- No supervisor module — systemd assumed as process manager
- No auto-updater
- No background timer management layer

**Gap / What's missing:**
- No in-process supervisor
- No auto-updater
- No background timer guard (all Rust timers start unconditionally)

**Severity: LOW**

---

### 16. Sensor Data & Silicon Subsystem

**Node.js had:**
- `src/services/sensorRegistry.js` (~430 lines) — sensor registration, reading submission with fraud detection (divergence strikes, rate limiting, replay prevention, geo-corroboration), median consensus per epoch, per-type SLUG validation, witness tracking (multi-gateway corroboration), finalized readings, stats accumulator
- `src/chain/dynamicSensorRewards.js` — per-type reward multipliers for 30+ sensor types (GPS 12x, air quality 3x, seismic with novelty_threshold 50x, etc.), stake requirements, novelty multipliers
- `src/services/sensorKeystore.js` — device signing key management
- `src/services/sensorDataBilling.js` — purchase-triggered reward distribution
- `src/routes/sensorRoutes.js`, `sensorDataRoutes.js` — HTTP API for sensor management
- `src/silicon/` — silicon/hardware integration layer

**Rust has:**
- `rust/btcpc-android/src/sensors.rs` — Android sensor reading (accelerometer, GPS, etc.)
- `SensorRegister`, `SensorKeyRegister`, `SensorVouch`, `SensorDataCommit`, `DeviceKeyRegister`, `DeviceYieldStake`, `GatewayHeartbeat` LedgerEntry variants defined
- `chain.rs` applies `SensorDataCommit` to accumulate reading_count per owner per epoch

**Gap / What's missing:**
- No server-side sensor registry (registration, fraud detection, median consensus)
- No per-type reward multipliers (the 30+ type config from `dynamicSensorRewards.js` is absent)
- No geo-corroboration, divergence strike system
- No multi-gateway witness tracking on the server
- No sensor data billing / purchase-triggered rewards
- No HTTP API for sensor management (registration, query, history)

**Severity: MEDIUM**

---

### 17. Scientific Compute

**Node.js had:**
- `src/scientific/scientificComputeEngine.js` — routes scientific compute jobs (matrix operations, statistical analysis, physics simulations) to specialized compute nodes, with job priority and result caching

**Rust has:**
- Zero scientific compute module

**Gap / What's missing:** Entire module absent.

**Severity: LOW** (not yet launched)

---

### 18. MCP (Machine Compute Protocol)

**Node.js had:**
- `src/mcp/btcpcMcpServer.js` — full MCP JSON-RPC server (port 3101) with built-in tools: calculator, hash, btcpc_fs_read, epoch_info, send_btcpc, get_balance, sensor_read, web_fetch (with SSRF guard), generate_text, commit_cid; non-deterministic tools auto-commit output to BTCPC-FS and return trace_cid for verifier checks; CLI passthrough tools via HONE_MCP_CLI_TOOLS
- `src/mcp/toolRegistry.js` — tool registration with `computeToolMultiplier()` (mining reward multiplier for tool use)
- `src/mcp/toolExecutor.js` — tool execution engine
- `src/routes/toolRoutes.js`, `toolRegistryRoutes.js`

**Rust has:**
- Zero MCP server module

**Gap / What's missing:** The entire MCP subsystem — including the `computeToolMultiplier()` that affects mining rewards — is absent from Rust. Rust miners cannot earn the tool-use bonus described in the whitepaper.

**Severity: MEDIUM**

---

### 19. Installer / Node Setup

**Node.js had:**
- `src/installer/index.js` — guided node setup: detects GPU, installs Ollama, registers node on-chain, configures environment
- Engine-specific installers: `hermes.js`, `openclaw.js`, `wizard.js`
- `src/installer/engine-monitor.js` — monitors running inference engines
- `src/installer/nodes.js` — node list management
- `src/installer/register.js` — on-chain node registration
- `src/installer/skills/` — skill definitions (hermes-skill.json, openclaw-skill.md, zeroclaw-skill/)

**Rust has:**
- No installer — node configured via environment variables and `genesis.json`
- `rust/hone-node/src/config.rs` — `Config::from_env()` reads all settings from env

**Gap / What's missing:** No guided installer, no GPU detection, no Ollama installation helper, no skill system.

**Severity: LOW** (UX gap, not chain-critical)

---

### 20. SDK / Developer-Facing APIs

**Node.js had:**
- `src/services/capabilityService.js` — introspects available node capabilities
- Various client libraries inlined in routes

**Rust has:**
- `rust/hone-sdk/src/lib.rs` — thin HTTP client SDK: `BtcpcClient` with transfer(), get_balance(), get_account(), create_account(), submit_entry(); ed25519 signing built in
- `rust/chain-core/src/lib.rs` — re-exports of hone-types
- `rust/hone-cli/src/` — full CLI: balance, transfer, stake, unstake, inference post/bid/complete, contract deploy/call/view, account create/update-key, chain info

**Gap / What's missing:**
- No JavaScript/TypeScript SDK (all client apps currently depend on hand-rolled fetch calls to the Node.js API)
- No read-only explorer SDK for querying chain state by index
- No cross-language SDK for mobile beyond local node JNI

**Severity: MEDIUM**

---

### 21. MongoDB Models & Data Persistence

**Node.js had:**
- `src/models/User.js` — Mongoose user schema: username, email, passwordHash, jwt, publicKey, privateAuth (factors, threshold), telegramId, wallet (chain addresses), createdAt, updatedAt
- `src/models/InferenceJob.js` — inference job schema with all status transitions
- `src/models/WorkProof.js` — compute proof records
- `src/models/Project.js` — project registry
- MongoDB used as cache; block files are canonical (Phase D/E migration note)

**Rust has:**
- RocksDB only — no MongoDB integration
- No User schema equivalent (account state is a flat JSON blob in RocksDB)
- No indexed queries — querying "all inference jobs by requester" requires full prefix scan (`infer_job:*`), O(N)

**Gap / What's missing:**
- No user identity model (email, passwordHash, telegramId linkage)
- No indexed collections for fast queries (e.g., jobs by requester, blocks by miner)
- No migration path from existing MongoDB data to RocksDB

**Severity: MEDIUM**

---

### 22. Error Handling & Resilience

**Node.js had:**
- Graceful degradation throughout (most modules use try/catch with non-fatal fallback)
- `SIGTERM` handlers flush caches to disk in p2p/network.js and ledger.js
- `shouldStartBackgroundTimers()` guard prevents timers in test environments
- Per-peer reconnect with exponential backoff
- Non-fatal blob store write failures logged with rate limiting
- Mempool overflow handled gracefully (drops lowest-priority entries)
- `stateStore.assertBalanceIntegrity()` assertion with descriptive errors

**Rust has:**
- `anyhow::Result` propagation throughout
- `warn!()` on entry application failures (non-fatal)
- Tokio tasks are spawned and logged but not supervised for restart on panic
- Clock consensus observer mode when isolated

**Gap / What's missing:**
- No graceful shutdown hooks — no equivalent of Node.js SIGTERM flush
- No task supervisor — a panicking Tokio task is silently dropped with no restart policy
- No test environment timer guard
- No balance integrity assertion

**Severity: MEDIUM**

---

### 23. Configuration & Environment

**Node.js had:**
- `src/index.js` — 15.5KB main entry that wires all subsystems: MongoDB, P2P, miner, clock, explorer, gateway, inference, sensors, MCP, all routes
- dotenv-based config with 40+ environment variables documented
- Per-role startup flags: `HONE_MINER_CLOCK`, `HONE_USE_RUST_P2P`, `HONE_MONGO_MODE`
- Genesis timestamp: 1776236400000 hardcoded

**Rust has:**
- `rust/hone-node/src/config.rs` — `Config::from_env()`: HONE_DATA_DIR, HONE_ACCOUNT, HONE_NODE_ID, HONE_API_PORT, HONE_P2P_PORT, HONE_MINER, HONE_CLOCK, HONE_GENESIS_FILE, HONE_GENESIS_TIMESTAMP, HONE_LOG_LEVEL, HONE_BOOTSTRAP_PEERS, HONE_CHAIN_ID, HONE_CLOCK_QUORUM, HONE_FINALITY_INTERVAL, HONE_FULL_ACTIVITY_SCORE (referenced but not implemented)

**Gap / What's missing:**
- Genesis timestamp mismatch: CLAUDE.md specifies `1776236400000` (2026-04-15). The Rust node reads `HONE_GENESIS_TIMESTAMP` from env with no hardcoded fallback — it will panic if not set rather than defaulting to the canonical value.
- No `HONE_RELAY_URL` / `HONE_RELAY_URLS` config (no relay support in Rust)
- No `HONE_MIN_CLOCK_STAKE` / `HONE_MIN_MINER_STAKE` stake threshold configuration
- `HONE_FULL_ACTIVITY_SCORE` is read but the activity-gated emission it controls is not implemented

**Severity: MEDIUM**

---

### 24. Testing Coverage

**Node.js had:**
- Jest test suite across multiple modules
- `stateStore.resetAll()` for test isolation
- `blockchain.reset()` for test isolation
- `HONE_DATA_DIR` env var used throughout for test isolation
- Tests in CI

**Rust has:**
- `rust/hone-types/src/emission.rs` — 5 unit tests: `era_boundaries`, `epoch_durations`, `supply_exhausted_after_5_eras`, `total_supply_correct`, `genesis_to_cap_duration`
- `rust/btcpc-contract-sdk/sdk/src/mock.rs` — contract test harness
- Contract example tests (ft, nft)

**Gap / What's missing:**
- No integration tests for chain.rs, tx.rs, inference.rs, clock.rs
- No end-to-end test that submits entries and verifies state
- No network-layer tests
- No reward-distribution correctness tests
- No finalization consensus tests

**Severity: MEDIUM**

---

## ARCHITECTURAL DIFFERENCES

1. **Emission model divergence (CRITICAL)**: Node.js uses an 11-period doubling-allotment model (`emissionSchedule.js`: GENESIS_ALLOTMENT × GROWTH_RATIO, period durations doubling from 1 to 345 months, `reward_per_epoch` varies per period). Rust uses `BLOCK_REWARD_HUNITS = 2 BTCPC` constant for era-0 epochs, with epoch *duration* doubling per era. These produce **different per-epoch reward amounts** (approximately 12x apart). A mixed Node.js/Rust network will immediately diverge.

2. **Persistence layer**: Node.js uses in-memory Maps rebuilt from block files on startup, with optional MongoDB cache. Rust uses RocksDB column families. No defined migration path.

3. **Role separation**: Node.js separates miner/clock/verifier as distinct processes. Rust integrates all roles into a single binary with feature flags (`HONE_MINER=true`, `HONE_CLOCK=true`). Cleaner architecture but means Node.js-style separate clock process cannot clock a Rust node.

4. **State root quality**: Node.js produces a cryptographic Sparse Merkle Tree root over all account states. Rust produces `SHA256(latest_block_hash || epoch)` — a weak proxy that cannot be used for light-client proofs.

5. **Block file format**: Node.js serializes blocks as binary header + length prefix + JSON payload. The Rust `Block::from_bytes` parser expects the same format. Appears compatible but was not cross-tested.

6. **Cross-chain architecture**: Node.js has a full claims/bridge stack (EVM claim submission, Hive anchoring). Rust has none of this — the cross-chain vision is entirely Node.js-only.

---

## PRIORITY PORTING LIST

Ordered by chain-correctness impact first, ecosystem value second:

1. **Emission schedule alignment** — port Node.js 11-period model or reconcile with Rust era model so both produce identical per-epoch rewards; without this, mixed networks cannot agree on blocks.
2. **Activity-gated emission** — implement `activityRatio = min(1, max(0.01, totalScore / FULL_ACTIVITY_SCORE))` scaling in `emit_epoch_rewards()`; without this, idle epochs mint at full rate.
3. **Multi-role reward distribution** — port the 6-pool (mining/verifier/clock/storage/sensor/service) proportional split from `blockProposal.js` into Rust's `emit_epoch_rewards()`; currently only mining + clock + storage + sensor pools are populated.
4. **Finalization consensus** — port `finalizationConsensus.js` proposal-collection → consensus-hash grouping → majority-wins so multiple clock nodes must agree on reward split before applying it.
5. **Sparse Merkle Tree state root** — replace proxy state root with a real SMT over account balances to enable light-client verification.
6. **Fork resolver** — port `forkResolver.js` (checkForFork, findCommonAncestor, selfHeal) so Rust nodes can recover from chain splits.
7. **Mempool with double-spend prevention** — add pending-tx pool with per-account debit tracking and fee-per-byte ordering.
8. **Stake-weighted miner scoring** — add `weight = min(sqrt(stake / MIN_STAKE), 10)` factor and 50% unverified-miner penalty to `emit_epoch_rewards()`.
9. **Device yield staking split** — implement 10-slot SLOT_MULTIPLIERS, 70/20/10 owner/staker/recycle split in sensor reward distribution.
10. **Full-chain replay on startup** — port `replay.js` finality-snapshot fast-forward + block-by-block catchup to Rust startup path.
11. **Claims system** — port `claimProofGenerator.js` + `evmClaimSubmitter.js` to Rust so BTCPC earned on-chain can be bridged to EVM.
12. **Block explorer** — port the dashboard, block detail, account, transaction, miners leaderboard, and tokenomics views to a Rust Axum server (Askama or Tera templates).
13. **Delegation system** — add DELEGATE/UNDELEGATE entry types with delegatedReceived balance tracking.
14. **MCP server + tool multiplier** — port the MCP JSON-RPC server and wire `computeToolMultiplier()` into mining reward calculation.
15. **Ensemble inference coordinator** — port `ensembleCoordinator.js` into Rust inference daemon.
16. **Server-side sensor registry with fraud detection** — port `sensorRegistry.js` divergence strikes, rate limiting, geo-corroboration, multi-gateway witness tracking.
17. **Oracle feeds** — port `oracleFeeds.js` median consensus primitive.
18. **Bridge registry** — port `bridgeRegistry.js` lock-and-recycle cross-chain bridge state.
19. **NFT and custom token entry types** — add NFT_MINT, NFT_TRANSFER, TOKEN_CREATE, TOKEN_TRANSFER apply logic to chain.rs.
20. **Sensor data billing** — purchase-triggered reward distribution for sensor data buyers.
21. **Block size cap + mempool fee market** — port `blockSizeCap.js` and `mempoolFeeMarket.js`.
22. **Auth middleware** — add JWT or API-key HTTP authentication layer to the Rust API.
23. **Graceful shutdown hooks** — add SIGTERM handlers to flush RocksDB writes and seen-message cache.

---

## RAW EVIDENCE (Key Code the Rust Version Does Not Have)

**Emission model divergence:**
- Node.js epoch 0 reward (from `emissionSchedule.js`): `2,100,000 BTCPC / 86,400 epochs ≈ 24.306 BTCPC/epoch`
- Rust epoch 0 reward (from `emission.rs`): `BLOCK_REWARD_HUNITS = 2 × 10^10 dreams = 2.0 BTCPC/epoch`
- These are ~12x apart; any mixed network immediately diverges.

**Missing activity gate** (`src/chain/blockProposal.js`):
```js
var activityRatio = totalScore === 0
  ? 0
  : Math.min(1.0, Math.max(MIN_EMISSION_RATIO, totalScore / FULL_ACTIVITY_SCORE));
var scheduledReward = blockReward;
blockReward = roundAmount(scheduledReward * activityRatio);
```

**Missing stake-weighted miner scoring** (`src/chain/blockProposal.js`):
```js
if (staked >= MIN_MINER_STAKE) {
  weight = Math.min(Math.sqrt(staked / MIN_MINER_STAKE), 10);
} else if (epochNumber < BOOTSTRAP_EPOCHS) {
  weight = 0.25;
} else {
  weight = 0;
}
var ww = (effectiveWork[miner] || 0) * weight;
```

**Missing Phase 2 endowment** (`src/chain/rewardEngine.js`):
```js
if (ss.isPhase2()) {
  isPhase2Epoch = true;
  const r = recycleBalance > 0 ? round(blockReward / recycleBalance) : 0;
  ss.setRecycleRate(r, epochNumber);
  effectiveBlockReward = round(recycleBalance2 * r);
}
```

**Missing device yield staking** (`src/chain/rewardEngine.js`):
```js
if (pool && pool.length > 0) {
  const ownerBase = round(perDevice * 0.70);
  const stakerPool = round(perDevice * 0.20);
  // ... SLOT_MULTIPLIERS weighted distribution per staker ...
}
```

**Missing consensus proposal collection** (`src/chain/finalizationConsensus.js`):
```js
// Group proposals by consensus_hash
var groups = {};
for (var p of epochState.proposals) {
  var key = p.consensus_hash;
  if (!groups[key]) groups[key] = [];
  groups[key].push(p);
}
// Winner = largest group
var winner = Object.values(groups).sort((a, b) => b.length - a.length)[0][0];
```

**Missing fork self-heal** (`src/chain/forkResolver.js`): 70-line `selfHeal()` with rollback, replay, and orphan proof resubmission — has no Rust equivalent.

**Missing Sparse Merkle Tree** (`src/chain/sparseMerkleTree.js`): Full 32-level SMT with `getProof()`/`verifyProof()` — has no Rust equivalent. Rust uses `SHA256(block_hash || epoch)` as a proxy state root.

---

*End of report. Feed this to your third LLM to plan porting priorities.*
