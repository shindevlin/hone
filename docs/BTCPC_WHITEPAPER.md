# Bitcoin Proof of Compute (BTCPC)

### A Decentralized Network for Verifiable AI Inference, Secured by Useful Work

**Version 0.1 — March 2026**

---

## Abstract

Bitcoin proved that a decentralized network, secured by proof of work, can create and maintain digital scarcity. But Bitcoin's work is deliberately useless — SHA-256 puzzles that produce nothing except security. Meanwhile, the world's demand for AI compute grows exponentially, concentrated in the hands of a few corporations.

Bitcoin Proof of Compute (BTCPC) applies Bitcoin's core insight — that costly, verifiable work can secure a network and back a scarce asset — to a problem that matters: AI inference. Miners earn BTCPC by providing real AI compute to the network. The work that secures the network is the work that users actually want to buy.

Total supply: **42,000,000 BTCPC** — the answer to life, the universe, and everything.

---

## 1. The Problem

### 1.1 Centralized AI Compute

Today, AI inference is controlled by a handful of companies: OpenAI, Google, Anthropic, Meta. This creates:

- **Single points of failure** — one API outage affects millions
- **Censorship risk** — providers decide what can and cannot be computed
- **Rent extraction** — monopoly pricing on compute that costs fractions of a cent
- **Privacy violation** — every prompt passes through corporate servers

### 1.2 Wasted Proof of Work

Bitcoin's network consumes approximately 150 TWh of electricity annually — more than many countries. This energy produces nothing except SHA-256 hashes. The security model works, but the work is pure waste.

### 1.3 The Opportunity

What if the energy spent securing a network also produced something people want? What if mining meant running AI models, and the resulting compute was available to anyone willing to pay in the network's native token?

---

## 2. Proof of Compute

### 2.1 How Mining Works

A BTCPC miner runs a node with one or more AI models (via Ollama, vLLM, or compatible inference engines). Mining consists of:

1. **Registering** as a node: declaring available models, hardware specs, and API endpoint
2. **Serving inference requests** from the network
3. **Logging work proofs**: cryptographic commitments to each inference performed
4. **Submitting epoch commitments** summarizing all work done in the epoch

The network verifies this work and distributes block rewards proportional to verified compute.

### 2.2 Epochs

Time in BTCPC is measured in **epochs**. One epoch = 5 minutes (approximately 100 Hive blocks when operating as a Hive L2, or ~60 standalone blocks at 5-second intervals).

At each epoch boundary:
1. Nodes submit their epoch commitment: `{state_hash, tx_count, inference_count, proof_samples[]}`
2. The network reaches consensus on the canonical state
3. Block rewards are distributed to honest miners
4. Difficulty adjusts if necessary

### 2.3 The Work Function

Unlike Bitcoin's single work function (SHA-256), BTCPC has a composite work function:

```
Work = Σ(inference_tokens_generated × model_weight_factor)
```

Where `model_weight_factor` scales with model size:
- 1B-7B parameters: 1.0x
- 7B-13B parameters: 2.0x
- 13B-30B parameters: 4.0x
- 30B-70B parameters: 8.0x
- 70B+ parameters: 16.0x

This means running a 70B model earns 16x more than a 7B model per token generated — reflecting the real compute cost difference.

---

## 3. Verification: Proving Compute Is Real

The core challenge: how do you verify that a node actually performed the inference it claims, without re-running every computation?

### 3.1 The Verification Trilemma

Any verification system must balance:
- **Security** — cheaters must be caught
- **Efficiency** — verification must cost less than the work itself
- **Speed** — verification must not bottleneck the network

BTCPC solves this with a three-layer verification stack.

### 3.2 Layer 1: Deterministic Commitment

All inference is performed with deterministic parameters:
- Temperature = 0
- Fixed random seed (derived from epoch number + request hash)
- Quantization-aware tolerance bounds

Before serving a result, the node commits:
```
commitment = hash(prompt_hash || result_hash || model_weights_hash || timestamp)
```

This commitment is posted to the network. It cannot be forged after the fact because the prompt_hash is determined by the requester, not the miner.

### 3.3 Layer 2: Stochastic Spot Checking

The network randomly selects **2-3% of inferences** for verification each epoch. Verification works as follows:

1. A verification request is broadcast to 3 randomly selected nodes (not the original miner)
2. Each verifier re-runs the inference with identical parameters
3. If 2/3 verifiers produce a matching result_hash → original node confirmed honest
4. If result doesn't match → challenge escalation (Section 3.5)

**Statistical guarantee:** If a node cheats on X% of its inferences, the probability of being caught within N epochs is:

```
P(caught) = 1 - (1 - 0.025)^(X * N * inferences_per_epoch)
```

At 50% cheating rate and 100 inferences per epoch:
- After 1 epoch: 71.8% chance of detection
- After 3 epochs: 97.8% chance of detection
- After 5 epochs: 99.95% chance of detection

Cheating is not a viable strategy.

### 3.4 Layer 3: Physical Plausibility Bounds

Nodes declare their hardware capabilities upon registration:
- GPU model, VRAM, count
- CPU cores, RAM
- Maximum concurrent inferences

The network maintains benchmark data for known hardware configurations. A node claiming to generate 1000 tokens/second on a single RTX 4090 with a 27B parameter model is physically impossible (~15-30 tokens/second is realistic). Claims that violate plausibility bounds trigger automatic verification of all that node's work for the epoch.

### 3.5 Challenge Protocol

When spot-checking reveals a mismatch:

1. **Challenge period** opens (1 epoch / 5 minutes)
2. 5 additional verification nodes re-run the inference
3. **Supermajority (4/5) determines truth**
4. If original miner was wrong:
   - 25% of staked BTCPC is slashed
   - 50% of slashed amount goes to the initial verifier who caught it
   - 50% is burned (reducing total supply — deflationary pressure)
5. If original miner was right (verifier error):
   - Challenging verifier loses reputation score
   - No economic penalty (false challenges are not punished with slashing to encourage reporting)

### 3.6 Why This Works (Game Theory)

For a rational miner with stake S and epoch earnings E:

```
Expected value of honest mining = E
Expected value of cheating = E × savings_from_faking - S × 0.25 × P(caught)
```

Since P(caught) approaches 1.0 rapidly, and S >> E (minimum stake requirement ensures this), cheating is always negative expected value. The system is **incentive-compatible** — honest behavior is the dominant strategy regardless of what other miners do.

---

## 4. Token Economics

### 4.1 Supply

**Total supply: 42,000,000 BTCPC**

42 — the Answer to the Ultimate Question of Life, the Universe, and Everything, as computed by Deep Thought over 7.5 million years in Douglas Adams' *The Hitchhiker's Guide to the Galaxy*. Deep Thought was, arguably, the first proof of compute system: a massive computer that ran for millennia to produce a single, verified answer.

BTCPC is the modern incarnation of that idea: a global network of computers performing useful computation, producing verified answers, and earning tokens for their work. The total supply honors the original answer.

### 4.2 Emission Schedule

| Phase | Epochs | Duration | Block Reward | Tokens Minted |
|-------|--------|----------|-------------|---------------|
| Genesis | 0 - 420,000 | ~4 years | 50 BTCPC | 21,000,000 |
| Halving 1 | 420,001 - 840,000 | ~4 years | 25 BTCPC | 10,500,000 |
| Halving 2 | 840,001 - 1,260,000 | ~4 years | 12.5 BTCPC | 5,250,000 |
| Halving 3 | 1,260,001 - 1,680,000 | ~4 years | 6.25 BTCPC | 2,625,000 |
| ... | ... | ... | ... | ... |

Halvings continue until the block reward reaches the minimum precision unit (0.00000001 BTCPC). Final token minted approximately 128 years after genesis.

### 4.3 Reward Distribution Per Epoch

Each epoch's block reward is distributed proportional to verified work:

```
miner_reward = block_reward × (miner_work / total_network_work)

miner_work = Σ(tokens_generated × model_weight_factor) [verified only]
```

During the genesis period (Beastly solo mining), 100% of the block reward goes to the single miner. As more nodes join, the reward distributes naturally.

### 4.4 Difficulty Adjustment

Every 1,000 epochs (~3.5 days), the network adjusts difficulty:

```
new_difficulty = old_difficulty × (actual_work / target_work)
target_work = expected tokens across all nodes at current difficulty
```

If more GPUs join the network → each unit of compute earns fewer BTCPC → mirrors Bitcoin's difficulty adjustment exactly.

### 4.5 Fee Market

Users pay for inference in BTCPC:
- **Base fee** = network-determined minimum per 1K tokens (adjusts with demand)
- **Priority fee** = optional tip for faster processing
- **Fee burn** = 50% of base fee is burned (deflationary)
- **Miner revenue** = block reward + remaining 50% of fees + priority fees

As block rewards diminish through halvings, fee revenue becomes the primary miner incentive — exactly like Bitcoin's long-term security model.

---

## 5. Multi-Chain Settlement

### 5.1 Architecture

BTCPC is **chain-agnostic by design**. The compute network maintains its own state and consensus. Token settlement occurs on whichever chains the network supports:

```
┌─────────────────────────────────┐
│     BTCPC Compute Network       │
│  (Nodes, Epochs, Verification)  │
└──────────┬──────────┬───────────┘
           │          │
    ┌──────┴──┐  ┌────┴────┐
    │  Hive   │  │  Base   │   ← Settlement Chains
    │custom_  │  │ ERC-20  │
    │  json   │  │contract │
    └─────────┘  └─────────┘
```

### 5.2 Day 1: Hive

Hive is the natural first settlement chain:
- **Free transactions** (resource credit model)
- **3-second block times**
- **custom_json** allows arbitrary structured data
- **Existing community** of node operators and developers

BTCPC operations on Hive use `custom_json` with id `btcpc`:
```json
{"op": "transfer", "from": "alice", "to": "bob", "amount": "100.00000000 BTCPC"}
{"op": "epoch_commit", "node": "alice", "epoch": 1234, "state_hash": "abc...", "work": 50000}
{"op": "register_node", "account": "alice", "endpoint": "https://...", "models": ["qwen3.5:27b"]}
```

### 5.3 Day 1: Base (Ethereum L2)

Simultaneously, BTCPC exists as an ERC-20 on Base:
- **Low gas fees** (~$0.001 per transaction)
- **Ethereum ecosystem compatibility** (DEXs, bridges, DeFi)
- **Broader market access** than Hive alone

A bridge contract coordinates minting between chains:
- Compute network signs epoch results with a multisig of top-staked nodes
- Bridge contract verifies signatures and mints tokens on Base
- Cross-chain transfers: burn on one chain, mint on the other (with proof)

### 5.4 Future Chains

The architecture supports adding any settlement chain via adapter modules:
- Solana (SPL token)
- Arbitrum / Optimism
- TON
- Own sovereign chain (when network is large enough)

### 5.5 Multi-Chain Reward Split

Miners configure which chains receive their rewards:

```yaml
reward_addresses:
  hive: "@thisthatjosh"
  base: "0x1234..."
split:
  hive: 60%
  base: 40%
```

Epoch settlement mints the appropriate amount on each chain simultaneously.

---

## 6. Genesis: The Beastly Epoch

### 6.1 Solo Mining Phase

BTCPC launches with a single miner: **Beastly** — a GPU-equipped Windows machine running Ollama with multiple AI models.

This mirrors Bitcoin's genesis:
- Satoshi Nakamoto mined alone for months before anyone else joined
- The early blocks were nearly worthless
- The work was real, the tokens accumulated, and value emerged later

Beastly's genesis configuration:
- Models: qwen3.5:27b, dirty-muse-writer, deepseek-r1:8b, glm-4.7-flash
- Epoch interval: 5 minutes
- Initial block reward: 50 BTCPC
- Daily mining output: ~14,400 BTCPC (288 epochs × 50 BTCPC)

### 6.2 Opening the Network

When the first external node operator wants to join:
1. They install the BTCPC node software (based on urs-nerdcore-node)
2. They register on-chain with their Hive account and/or Base wallet
3. They stake a minimum amount of BTCPC (received via transfer from genesis miner or purchased)
4. They begin serving inference and earning rewards

The staking requirement creates a bootstrapping challenge: early nodes need BTCPC to stake, but BTCPC only comes from mining. This is solved by:
- **Genesis grants**: the solo miner (you) distributes initial stakes to early node operators
- **Faucet**: a small amount available for new nodes to bootstrap
- **Delegation**: existing stakers can delegate stake to new nodes

### 6.3 Genesis Block

```
Block 0 — BTCPC Genesis
Timestamp: [TBD]
Miner: Beastly
Models: qwen3.5:27b
State Hash: 0x0000000000000000000000000000000000000000000000000000000000000000
Message: "The Answer to the Ultimate Question of Life, the Universe, and Everything"
Reward: 50 BTCPC
```

---

## 7. Node Software

### 7.1 Reference Implementation

The reference BTCPC node is built on **urs-nerdcore-node** (Node.js/Express/MongoDB):

```
btcpc-node start \
  --hive-account thisthatjosh \
  --base-wallet 0x1234... \
  --ollama-url http://localhost:11434 \
  --stake 1000
```

### 7.2 Node Components

```
┌─────────────────────────────────────────────┐
│                BTCPC Node                    │
├──────────────┬──────────────┬───────────────┤
│ Chain Listener│ Inference    │ Epoch         │
│ (Hive/Base)  │ Server       │ Worker        │
│              │ (Ollama      │               │
│ Reads ops,   │  proxy)      │ Computes      │
│ applies state│              │ state hash,   │
│              │ Serves       │ submits       │
│              │ requests,    │ commitments   │
│              │ logs proofs  │               │
├──────────────┴──────────────┴───────────────┤
│              State Manager (MongoDB)         │
│  Accounts, Balances, Stakes, Work Proofs     │
├─────────────────────────────────────────────┤
│              P2P / API Layer                 │
│  Peer discovery, public API, health checks   │
└─────────────────────────────────────────────┘
```

### 7.3 Minimum Hardware Requirements

**Inference tier (earns full rewards):**
- GPU with 8GB+ VRAM (runs 7B+ models)
- 16GB RAM
- 4 CPU cores
- 100GB SSD
- 50 Mbps internet

**Relay tier (earns partial rewards):**
- No GPU required
- 8GB RAM
- 2 CPU cores
- 50GB SSD
- Transaction processing and state validation only
- Earns ~10% of what inference nodes earn

---

## 8. Roadmap

### Phase 0: Genesis (Current)
- [x] Architecture specification
- [ ] Whitepaper (this document)
- [ ] Genesis mining loop on Beastly
- [ ] BTCPC token on Hive (custom_json)
- [ ] Basic wallet (transfer, balance)

### Phase 1: Solo Mining
- [ ] Epoch system with state hashing
- [ ] Work proof logging
- [ ] Multi-model support
- [ ] Mining dashboard (web UI)

### Phase 2: Network Opening
- [ ] Node registration protocol
- [ ] Stake requirement enforcement
- [ ] Spot-check verification system
- [ ] BTCPC ERC-20 on Base
- [ ] Cross-chain bridge

### Phase 3: Inference Market
- [ ] Public inference API (pay with BTCPC)
- [ ] Model marketplace (nodes advertise capabilities)
- [ ] Fee market and priority system
- [ ] Challenge/dispute resolution

### Phase 4: Maturity
- [ ] Difficulty adjustment
- [ ] First halving
- [ ] Sovereign chain option
- [ ] Governance (stake-weighted voting)
- [ ] Mobile node support

---

## 9. Conclusion

Bitcoin proved that decentralized proof of work can create sound money. BTCPC extends this insight: the work that secures the network should be useful. Every BTCPC token is backed by real AI compute — inference that someone wanted, paid for, and received.

The result is a network where:
- **Miners earn by doing useful work** (not burning energy on puzzles)
- **Users get censorship-resistant AI compute** (not dependent on any corporation)
- **The token has intrinsic utility** (you need it to buy inference, creating organic demand)
- **Supply is fixed and predictable** (42M total, halving schedule, deflationary fee burn)

The answer is 42. The question was always about compute.

---

*"I think the problem, to be quite honest with you, is that you've never actually known what the question is."*
— Deep Thought, *The Hitchhiker's Guide to the Galaxy*

---

## Appendix A: Comparison with Bitcoin

| Property | Bitcoin | BTCPC |
|----------|---------|-------|
| Total Supply | 21,000,000 | 42,000,000 |
| Work Function | SHA-256 (useless) | AI Inference (useful) |
| Block Time | ~10 minutes | ~5 minutes (1 epoch) |
| Halving Interval | 210,000 blocks (~4 years) | 420,000 epochs (~4 years) |
| Verification | Check hash (instant) | Spot check 2-3% + slashing |
| Mining Hardware | ASICs | GPUs + AI models |
| Settlement | Bitcoin chain only | Multi-chain (Hive, Base, future) |
| Fee Model | Transaction fees | Inference fees + transaction fees |
| Scarcity Source | Energy expenditure | Useful compute expenditure |

## Appendix B: Comparison with Compute Networks

| Property | Render | Akash | io.net | BTCPC |
|----------|--------|-------|--------|-------|
| Token Model | Utility (inflationary) | Utility (inflationary) | Utility (inflationary) | Fixed supply (42M, deflationary) |
| Mining | No mining | No mining | No mining | Yes — earn by providing compute |
| Consensus | None (marketplace) | Tendermint (PoS) | None (marketplace) | Proof of Compute |
| Verification | Trust-based | Trust-based | Trust-based | Cryptographic spot-checks |
| Halvings | No | No | No | Yes (BTC-style) |
| Fee Burns | No | No | No | Yes (50% of base fee) |
| AI-Native | No (general GPU) | No (general compute) | Partial | Yes (inference-first) |
