# URSNode L2 Architecture — Proof of Useful Work

## Overview

URSNode is a Hive Layer 2 network where node operators earn URS tokens by performing useful computation: processing L2 transactions, maintaining consensus state, and providing decentralized AI inference compute.

Unlike traditional PoW (wasteful hash puzzles) or pure PoS (nothing-at-stake), URSNode uses **Proof of Useful Work (PoUW)** — nodes prove they did real, verifiable computation that the network needs.

## Core Concepts

### 1. Hive as Settlement Layer
- All URS operations are posted to Hive as `custom_json` with id `urs-l2`
- Hive provides the immutable transaction log; URSNode provides execution
- Anyone can reconstruct L2 state by replaying Hive custom_json history
- No separate blockchain — Hive IS the blockchain; URSNode is the execution layer

### 2. Custom_json Operation Types
```json
{"type": "transfer", "from": "alice", "to": "bob", "amount": "100 URS"}
{"type": "stake", "account": "alice", "amount": "500 URS"}
{"type": "unstake", "account": "alice", "amount": "200 URS"}
{"type": "register_node", "account": "alice", "endpoint": "https://node.alice.com", "models": ["qwen3.5:27b"]}
{"type": "inference_request", "requester": "bob", "model": "qwen3.5:27b", "prompt_hash": "abc123"}
{"type": "inference_result", "node": "alice", "request_id": "xyz", "result_hash": "def456"}
{"type": "epoch_commitment", "node": "alice", "epoch": 1234, "state_hash": "aaa", "tx_count": 50, "inference_count": 12}
```

### 3. Epochs
- 1 epoch = 100 Hive blocks (~5 minutes)
- At epoch boundary, each node:
  1. Computes state hash from all L2 state (balances, stakes, node registry)
  2. Counts transactions processed + inference requests served
  3. Submits `epoch_commitment` to Hive
- Rewards distributed to nodes whose state_hash matches majority consensus

## Architecture Layers

### Layer 0: Hive Blockchain
- Immutable transaction log
- Account system (Hive accounts = URS accounts)
- custom_json for all L2 operations
- Resource credits for transaction fees (free for users with Hive stake)

### Layer 1: Transaction Processing
```
Hive custom_json stream
       |
       v
  [Node Process]
       |
       +-- Parse & validate URS operations
       +-- Update local state DB (MongoDB)
       +-- Compute running state hash (Merkle root)
       +-- Serve API: balances, history, staking info
```

**State Model (MongoDB):**
- `accounts`: {hive_account, urs_balance, staked_balance, last_active}
- `stakes`: {account, amount, staked_at, unlock_at}
- `nodes`: {account, endpoint, models[], stake, uptime, reputation}
- `epochs`: {number, state_hash, tx_count, inference_count, rewarded_nodes[]}
- `inference_log`: {request_id, requester, node, model, prompt_hash, result_hash, tokens_used, epoch}

### Layer 2: Consensus
```
Epoch N ends
       |
       v
  Each node submits epoch_commitment to Hive
       |
       v
  Epoch N+1: nodes read all commitments for epoch N
       |
       v
  Majority state_hash wins (>50% of staked weight)
       |
       v
  Matching nodes: earn URS rewards
  Non-matching nodes: reputation penalty (repeated = stake slash)
```

**Reward Formula:**
```
node_reward = epoch_base_reward * (node_stake / total_stake) * work_multiplier

work_multiplier = (node_tx_count + node_inference_count * 3) / avg_work
```

- Inference counts 3x because it's more computationally expensive
- This incentivizes nodes to both process transactions AND serve inference
- Minimum stake to run a node: 1000 URS (prevents spam nodes)

### Layer 3: Decentralized Inference

**How it works:**
1. Node registers with `register_node` custom_json, advertising:
   - API endpoint URL
   - Available Ollama models (e.g., qwen3.5:27b, dirty-muse-writer)
   - Max concurrent requests
   - Price per 1K tokens (in URS)

2. Client sends inference request:
   - Posts `inference_request` to Hive with prompt_hash (not the prompt itself — privacy)
   - Directly calls the node's API endpoint with the actual prompt
   - Node processes via local Ollama, returns result
   - Node posts `inference_result` to Hive with result_hash

3. Verification:
   - Any node can re-run the same prompt and verify the result_hash matches
   - Challenge system: if result_hash doesn't match, challenger posts proof
   - Wrong results = reputation penalty + partial stake slash

4. Payment:
   - Requester's URS balance debited by tokens_used * price_per_1K
   - Node's URS balance credited at epoch settlement
   - Staked nodes get priority routing for inference requests

**Node Selection Algorithm:**
```
score = (node_reputation * 0.4) + (node_stake_weight * 0.3) +
        (1/node_latency * 0.2) + (model_availability * 0.1)
```
Highest scoring node gets the request. If it fails/times out, next node.

## Node Software

### What a node operator runs:
```
urs-node start --hive-account alice --posting-key 5J... --ollama-url http://localhost:11434
```

### Node process components:
1. **Hive Stream Listener** — watches for `urs-l2` custom_json ops
2. **Transaction Processor** — validates and applies state transitions
3. **State Manager** — MongoDB state + Merkle tree for state hashing
4. **Epoch Worker** — computes and submits epoch commitments
5. **Inference Server** — Express API that proxies to local Ollama
6. **API Server** — public API for balance queries, staking info, etc.
7. **Peer Discovery** — finds other nodes via Hive node registry

### Minimum hardware:
- 4 CPU cores, 16GB RAM (for Ollama + node process)
- GPU recommended for inference (earns more rewards)
- 50GB SSD for state DB
- Stable internet connection

## Token Economics

### URS Token
- Initial supply: minted via Hive Engine (or native custom_json tracking)
- Emission: 100 URS per epoch to node operators
- Halving: every 1,000,000 epochs (~9.5 years)
- Use cases: staking, inference payments, governance votes

### Fee Structure
- L2 transactions: free (funded by Hive RC)
- Inference: market-rate in URS (set by node operators)
- Staking: no fee to stake; 7-day unlock period for unstaking

### Slashing Conditions
- Wrong state hash 3 epochs in a row: 5% stake slashed
- Node offline >1 hour during registered period: reputation penalty
- Fraudulent inference result (verified by challenge): 10% stake slashed

## Implementation Phases

### Phase 1: Foundation (current sprint)
- [ ] Wallet controller (transfer, balance)
- [ ] Staking controller (stake, unstake, rewards)
- [ ] Hive custom_json listener
- [ ] Basic state management (MongoDB)
- [ ] Auth + API endpoints

### Phase 2: Consensus
- [ ] Epoch system (100-block epochs)
- [ ] State hash computation (Merkle tree)
- [ ] Epoch commitment submission to Hive
- [ ] Reward distribution logic
- [ ] Node registration

### Phase 3: Inference Network
- [ ] Node Ollama proxy API
- [ ] Inference request/result tracking
- [ ] Model registry (nodes advertise available models)
- [ ] Inference routing (select best node)
- [ ] Payment settlement

### Phase 4: Hardening
- [ ] Challenge/dispute system
- [ ] Slashing implementation
- [ ] Peer-to-peer state sync
- [ ] Node dashboard UI
- [ ] Documentation + node operator guide

## Integration Points

- **bullship**: Game actions as L2 transactions, rewards paid in URS
- **nsfwotica**: Story generation routed through inference network
- **betchu_bot**: Bet settlement via L2 smart logic
- **waitlyfi**: Waitlist token distribution via URS transfers
- **agentextend**: Direct overlap — inference network IS agentextend's core
