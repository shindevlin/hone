# Bitcoin Proof of Compute (BTCPC)

### A Decentralized Network for Verifiable AI Inference, Secured by Useful Work

**Shin Devlin**
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

### 3.2 Redundant Computation with Commit-Reveal

BTCPC achieves **100% verification** through redundant computation. Every inference request is assigned to multiple nodes simultaneously, and a cryptographic commit-reveal scheme prevents result copying.

**Phase 1 — Commit:**
1. Network assigns the same inference request to N nodes (N=3 for standard, N=5 for high-value)
2. Each node runs the inference independently with deterministic parameters (temperature=0, fixed seed)
3. Each node encrypts their result and submits only the **hash** to the network
4. No node can see another's result before committing — copying is impossible

**Phase 2 — Reveal:**
1. Once all N hashes are submitted, nodes reveal their actual results
2. Results are compared against the committed hashes (no post-hoc modification possible)
3. Matching results form consensus — this IS the verified answer
4. Non-matching nodes are immediately identified

**Payment distribution:**
```
First node to submit matching hash:   50% of request fee
Second matching node:                 30% of request fee
Third matching node:                  20% of request fee
Non-matching nodes:                   slashed
```

This creates a **race** — nodes compete to finish first, incentivizing better hardware. The fastest honest miner earns the most, just like Bitcoin miners racing to find a hash.

**Scaling N with network size:**
- Genesis (Beastly solo): N=1 (no redundancy needed — single miner)
- Early network (2-5 nodes): N=all nodes (everyone verifies everything)
- Growth phase (5-50 nodes): N=3 (standard redundancy)
- Mature network (50+ nodes): N=3 standard, N=5 for high-value requests

### 3.3 Physical Plausibility Bounds

Nodes declare their hardware capabilities upon registration:
- GPU model, VRAM, count
- CPU cores, RAM
- Maximum concurrent inferences

The network maintains benchmark data for known hardware configurations. A node claiming to generate 1000 tokens/second on a single RTX 4090 with a 27B parameter model is physically impossible (~15-30 tokens/second is realistic). Claims that violate plausibility bounds trigger automatic verification of all that node's work for the epoch.

### 3.4 Slashing Protocol

When a node's result doesn't match consensus:

1. First offense: warning + reputation penalty
2. Second offense within 1000 epochs: **10% of staked BTCPC slashed**
3. Third offense: **25% slashed + 24-hour mining suspension**
4. Persistent offenders: **full stake slashed + permanent ban**

Slashed BTCPC is redistributed to the honest nodes who produced the correct result.

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

### 4.2 Emission Schedule — Doubling Halving Intervals

BTCPC introduces a novel emission model: **doubling halving intervals**. Each period, the block reward halves while the period duration doubles. Unlike Bitcoin's fixed 4-year halvings, BTCPC's emission curve adapts: early periods distribute tokens quickly to bootstrap the network, while later periods extend over decades, ensuring long-term miner incentives.

With a genesis allocation of 5% of total supply (2,100,000 BTCPC) in the first 1-month period, and a growth ratio of ~1.134x per period, each subsequent period's total allotment grows slightly in absolute terms but the per-epoch reward decreases. This means the inflation RATE drops consistently while the network can support an ever-growing number of miners.

| Period | Duration | Allotment | Reward/Epoch | Cumulative | % Supply | Annual Inflation |
|--------|----------|-----------|-------------|------------|----------|-----------------|
| 1 | 1 month | 2,100,000 | 243.06 | 2,100,000 | 5.0% | ∞ (genesis) |
| 2 | 2 months | 2,381,400 | 137.85 | 4,481,400 | 10.7% | High (early) |
| 3 | 4 months | 2,700,508 | 78.19 | 7,181,908 | 17.1% | ~87% |
| 4 | 8 months | 3,062,376 | 38.01 | 10,244,283 | 24.4% | ~29% |
| 5 | 16 months | 3,472,734 | 25.15 | 13,717,017 | 32.7% | ~11% |
| 6 | 32 months | 3,938,080 | 14.24 | 17,655,097 | 42.0% | ~4.8% |
| 7 | 64 months | 4,465,783 | 8.08 | 22,120,881 | 52.7% | ~2.2% |
| 8 | 128 months | 5,064,198 | 4.58 | 27,185,079 | 64.7% | ~1.0% |
| 9 | 256 months | 5,742,801 | 2.60 | 32,927,879 | 78.4% | ~0.5% |
| 10 | 512 months | 6,512,336 | 1.47 | 39,440,215 | 93.9% | ~0.2% |
| 11 | 345 months* | 2,559,785 | 0.86 | 42,000,000 | 100% | ~0.1% |

*Period 11 is truncated to align final mining with Bitcoin's projected end date of ~2140.

**Key properties:**
- **Genesis month (Beastly solo mining):** 5% of supply — earned through real compute
- **By year 5:** inflation drops to ~11%, comparable to Bitcoin's era 2
- **By year 10:** inflation at ~5%, matching BTC's era 3
- **All 42M mined by ~2140** — same timeline as Bitcoin's last satoshi
- **No period dominates:** unlike fixed halvings where era 1 gets 50%, the largest single period (period 10) gets only 15.5%
- **Growing allotments reward growing networks:** more miners in later periods have more tokens to compete for

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
- **Miner revenue** = block reward + 100% of fees + priority fees
- **No burning** — every BTCPC minted stays in circulation forever, like Bitcoin

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
- Initial block reward: 243 BTCPC per epoch
- Period 1 output: 2,100,000 BTCPC over 1 month (5% of total supply)
- Daily mining output: ~69,984 BTCPC (288 epochs × 243 BTCPC)

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
Reward: 243 BTCPC
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
| Halving Interval | 210,000 blocks (fixed ~4 years) | Doubling intervals (1mo → 2mo → 4mo → ...) |
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
| Fee Burns | No | No | No | No (BTC-faithful) |
| AI-Native | No (general GPU) | No (general compute) | Partial | Yes (inference-first) |
