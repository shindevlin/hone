---
title: BTCPC Core Architecture Decisions
description: Canonical design decisions for the BTCPC sovereign chain node
captured_at: 2026-04-29
author: Shin Devlin
---

# BTCPC Core Architecture Decisions

## Emission Model
- **Rust node is the canonical implementation.** JS node is deprecated.
- Do NOT port the JS 11-period allotment model. Rust era-based doubling epoch model is correct.

## Genesis / Testnet Fund
- **No genesis BTCPC allocation for anything** — not testnet fund, not anyone.
- `__testnet_fund__` starts at zero. Fills only from the ongoing 0.5% mandatory reserve split.
- Testnet gets nothing until the chain is live and producing rewards.

## Scarcity: Divider, Not Multiplier
- The 2.5x scarcity bonus (old JS behaviour) is wrong. Scarcity penalises small pools.
- `payout_factor = min(1.0, pool_size / CRITICAL_MASS_TARGET)`
- Pools below critical mass pay out at reduced rate. At critical mass = full rate.
- Critical mass is dynamic: EMA of active participants over trailing N epochs.

## Mempool as Staked Node Type
- Mempool operators stake BTCPC → earn portion of transaction fees.
- Reward weighted inversely by propagation latency: lowest-lag nodes earn most.
- Slashable stake for censorship, double-inclusion, or fee front-running.
- `MempoolOperatorRegister` entry type, minimum stake threshold.

## Device Auth: Hardware Serial + Posting Key + Claim Stake
- Posting key required for all entries.
- Device serial = hardware-burned unique identifier (IMEI, CPU serial, TPM endorsement key).
  - Same physical device re-registered later → same serial → claim is maintained.
  - NOT based on model/specs. Each physical unit is unique.
- Device claim stake: stake BTCPC to register a serial. First staker wins.
  - Overbid allowed at 1.5× existing stake; premium distributes to yield stakers.
  - Stake slashable for fraudulent claims.
- This is the auth model for IoT/sensor devices — NOT JWT.

## Delegation: Scoped, Multi-Target, Wallet-Native
- Delegation must specify scope: Stake, Inference, SensorSubmission, StorageOps, GatewayOps, Governance, Commerce.
- Delegatee: specific account, list of accounts, or `"*"` (all).
- Time-limited (N epochs) or permanent. Independently revocable per scope.
- Delegation UI built into user's wallet, not a separate admin tool.

## Sensor Registry: Chain as Server via Service Nodes
- Server-side sensor fraud detection and geo-corroboration runs ON service nodes.
- Service nodes (decentralized Docker/K8s) host the sensor registry logic.
- Results committed back to the chain.

## Device Yield Staking: Opt-In
- Default: 90% owner / 10% recycle, no stakers.
- Opt-in: device owner enables staking slot, up to 10 stakers can participate.
- With stakers: 70% owner / 20% staker pool / 10% recycle.

## Cross-Chain Claims: 2FA Signature Model
- Cross-chain wallet signature as 2FA for spending BTCPC.
- Signing a message with a wallet on another chain (EVM, Solana, etc.) authorises spend on BTCPC.
- Not just a bridge — a cross-chain authentication mechanism.

## Commerce: Three Sub-Protocols
1. **Freeport** — general commerce/marketplace (storefronts, products, orders, disputes)
2. **Verasens** — sensor data marketplace (purchase-triggered rewards, geo data)
3. **LinkGit** — decentralised git hosting (already in entry.rs)

## MCP Scope
- JSON-RPC server, tool registry with multiplier, RAG pipeline (local embeddings via candle), CLI passthrough, multiple inference providers (Ollama/OpenAI/Anthropic/custom).
- Tool multiplier feeds into MineReward for tool-augmented inference.
- Non-deterministic tool output auto-committed to BlobStore with trace_cid.

## Ensemble Inference
- Multiple nodes run same prompt → consensus when ≥ N produce same result_hash.
- Consensus nodes earn bonus; partial contributors earn reduced amount.

## Finalization Consensus
- Quorum threshold: 51% (can be raised to 2/3 BFT by governance).
- `hashRewards()` must be bit-for-bit deterministic: sorted entries, fixed-precision integer arithmetic, no floats.
- Single-node on mainnet cannot self-finalize — requires `MIN_CONSENSUS_SOURCES` distinct peers.

## Permissive Tokens
- ALL tokens (fungible, NFT, soulbound) are permissive — receipt requires explicit acceptance.
- Token sent → enters PENDING state, not credited.
- `TokenAccept` credits balance; `TokenReject` sends to `__recycle__`.
- Soulbound: permissive on receipt, then permanently locked.
- Tokens never accepted after timeout epoch → auto-recycle.

## Finality Fast-Forward via Hive
- JS node posted finality snapshots to Hive blockchain.
- New Rust nodes can use these existing Hive anchors as fast-forward starting points.
- On startup: check Hive for latest anchored snapshot → load state → replay only recent blocks.
