# Bitcoin Proof of Compute (BTCPC)

### A Sovereign Blockchain for Verifiable AI Inference, Secured by Useful Work

**Shin Devlin**
**Version 0.3 — March 2026**

---

## Abstract

Bitcoin proved that a decentralized network, secured by proof of work, can create and maintain digital scarcity. But Bitcoin's work is deliberately useless — SHA-256 puzzles that produce nothing except security. Meanwhile, the world's demand for AI compute grows exponentially, concentrated in the hands of a few corporations.

Bitcoin Proof of Compute (BTCPC) is a sovereign blockchain that applies Bitcoin's core insight — that costly, verifiable work can secure a network and back a scarce asset — to a problem that matters: AI inference. Miners earn BTCPC by providing real AI compute to the network. The work that secures the network is the work that users actually want to buy.

BTCPC introduces two innovations beyond Bitcoin: **Proof of Compute** replaces wasteful hash puzzles with useful AI inference, and **Cross-Chain Mining Rewards** automatically generate claimable tokens on every blockchain a miner links to their rig — creating multi-chain liquidity from the first block.

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

### 1.3 Siloed Liquidity

Every new blockchain launches with zero liquidity. Projects spend millions bootstrapping trading pairs, market makers, and exchange listings. Cross-chain bridges are complex, expensive, and frequent targets of exploits. There is no mechanism for a new chain to generate liquidity on existing chains organically.

### 1.4 The Opportunity

What if the energy spent securing a network also produced something people want? What if mining on one chain automatically created liquidity on every other chain the miner was connected to? What if the act of doing useful work generated value across the entire crypto ecosystem simultaneously?

---

## 2. The BTCPC Blockchain

### 2.1 Sovereign Chain

BTCPC is its own blockchain — not a Layer 2, not a sidechain, not a token on another platform. Like Bitcoin, it has:

- **Its own genesis block**
- **Its own P2P network** for transaction and block gossip
- **Its own consensus mechanism** (Proof of Compute)
- **Its own mempool** for pending transactions and inference requests
- **Its own account-based ledger** tracking balances, stakes, and mining rewards
- **Its own block format** containing transactions, inference proofs, and epoch commitments

### 2.2 Block Structure

```
Block {
  header: {
    version,
    previous_block_hash,
    merkle_root_transactions,
    merkle_root_compute_proofs,
    timestamp,
    epoch_number,
    difficulty,
    miner_id
  },
  transactions: [
    { type: "transfer", from, to, amount, signature },
    { type: "stake", account, amount, signature },
    { type: "inference_request", requester, model, prompt_hash, fee },
    { type: "inference_result", node, request_id, result_hash, tokens_generated },
    { type: "claim_generate", miner, epoch, chain, amount, proof },
    ...
  ],
  compute_proofs: [
    { node_id, inference_count, tokens_generated, model, commitment_hash }
  ]
}
```

### 2.3 Account Model — Hive-Style Key Hierarchy

Every BTCPC account has four key pairs, derived from a single master password — identical to the Hive blockchain's proven account model:

```
12-Word BIP-39 Mnemonic
  │
  ├── m/44'/btcpc'/0'/0/0  → Owner Key
  ├── m/44'/btcpc'/0'/1/0  → Active Key
  ├── m/44'/btcpc'/0'/2/0  → Posting Key
  └── m/44'/btcpc'/0'/3/0  → Memo Key

2FA Password (separate from mnemonic)
  │
  └── pbkdf2(password + account, 100000 rounds) → 2FA Key
```

**Account creation generates a standard BIP-39 12-word mnemonic.** The mnemonic derives all four role keys via BIP-44 derivation paths. This means any BIP-39 compatible hardware wallet (Ledger, Trezor, etc.) can store and sign with the role keys — no custom firmware required.

The 2FA password is set separately and derives its own key. The mnemonic and password together form complete account control. Losing either one alone is not enough to steal funds.

| Key | Permission Level | What It Can Do | Where It Lives |
|-----|-----------------|----------------|----------------|
| **Owner** | Highest | Change password, rotate all keys, recover account, update account authority | Cold storage (paper, hardware wallet, safe) |
| **Active** | Financial | Send BTCPC, stake/unstake, claim cross-chain rewards, set up recurring payments, manage multi-sig | Daily-use device, encrypted with TOTP |
| **Posting** | Operational | Submit epoch commitments, register/update mining node, serve inference requests, sign compute proofs | Mining rig (can mine but CANNOT move funds) |
| **Memo** | Communication | Encrypt/decrypt private messages, encrypt inference request/result payloads between nodes | Any device needing private communication |

**Key properties:**

- **Password changes rotate all derived keys instantly.** If your active key is compromised, change your password with the owner key. All four derived keys rotate. The attacker's stolen key is now worthless.
- **The posting key lives on your mining rig.** It can do everything mining requires but cannot transfer a single token. This means a compromised mining server cannot steal your funds.
- **The memo key enables private inference.** Users can encrypt prompts so only the assigned compute node can read them. Results are encrypted back. No one — not even other validators — sees the actual content.
- **Owner key is used rarely and stored cold.** Like Hive, responsible operators should never need their owner key in normal operations.

### 2.3.1 Protocol-Level Multi-Factor Authentication

BTCPC is the first blockchain with **protocol-enforced multi-factor authentication.** Every account has a configurable authentication profile that validators enforce on every transaction. This is not wallet-level security — it is consensus-level.

**Two additional authentication factors are available:**

**Factor A — Password:**
- User sets a 2FA password (can differ from the master key-derivation password)
- `2fa_private_key = pbkdf2(2fa_password, account_name, 100000 rounds)`
- `2fa_public_key` stored on-chain
- Transaction includes a signature from the 2FA key
- Password changeable at any time via Owner key (rotates the 2FA keypair)
- Validators verify the 2FA signature against the stored public key
- The password never touches the chain — only signatures

**Factor B — TOTP (Google Authenticator):**
- User scans a QR code to set up standard TOTP in any authenticator app
- `totp_commitment = hash(totp_seed)` stored on-chain
- Every 30 seconds, the TOTP seed + time window derives a keypair
- Transaction includes: TOTP signature + time_window
- Validators derive the expected public key from `totp_commitment + time_window` and verify
- Accepts ±1 window (90-second tolerance, standard TOTP)
- The TOTP secret never touches the chain — only the commitment and signatures

**User-configurable authentication profiles:**

| Profile | Factors Required | Use Case |
|---------|-----------------|----------|
| **Key only** | Role key signature | Minimum security (not recommended) |
| **Key + Password** | Role key + password-derived signature | Standard security |
| **Key + TOTP** | Role key + Google Auth signature | Convenient security |
| **Key + Password + TOTP** | All three signatures | Maximum security (3FA) |

Users configure their profile via `AccountUpdate` (requires Owner key). Each profile applies to all Active and Owner key operations. Posting key operations (mining, epoch commits) can be configured separately — miners may want posting-key transactions to require fewer factors for automated operation.

**Transaction format:**

```
Transaction {
  type: "transfer",
  from: "thisthatjosh",
  to: "alice",
  amount: "100.00000000 BTCPC",

  primary_signature: <signed with active key>,

  auth_factors: {
    password_signature: <signed with password-derived key>,  // if enabled
    totp_signature: <signed with TOTP-derived key>,          // if enabled
    totp_time_window: 1743120000                             // if TOTP enabled
  }
}

Validator checks:
  1. Is primary_signature valid for the "from" account's active key? ✓
  2. What auth profile does this account require?
  3. If password enabled: is password_signature valid? ✓
  4. If TOTP enabled: is totp_signature valid for this time_window? ✓
  5. All required factors present → accept transaction
```

**Why no other chain has this:**

- Bitcoin/Ethereum: single key, no recovery, no 2FA
- Hive: hierarchical keys but single-signature per transaction
- BTCPC: hierarchical keys + protocol-enforced 2FA/3FA with user choice

A compromised private key alone is not enough to steal funds. An attacker would also need the user's password and/or TOTP device. This is the security model users expect from banking — applied to a decentralized blockchain for the first time.

### 2.3.2 Hardware Wallet Integration

BTCPC's 2FA design works with **any BIP-39 hardware wallet out of the box** — no custom apps, no firmware modifications, no approval processes.

The role keys (owner/active/posting/memo) are derived from the 12-word mnemonic via standard BIP-44 paths. Any hardware wallet that supports BIP-39 can store and sign with these keys. The 2FA factor is handled separately by the CLI or wallet software:

```
$ btcpc-cli transfer --to alice --amount 100

  [Hardware wallet requests confirmation — press button, enter PIN]
  → Signature 1: role key (from hardware wallet)

  Enter password: ********
  → Signature 2: 2FA key (derived from password, computed by CLI)

  Transaction broadcast with both signatures.
```

The hardware wallet handles what hardware wallets do best — securely storing keys and requiring physical confirmation. The CLI handles the 2FA password/TOTP separately. Two independent factors, two independent devices, zero custom integration required.

**Supported configurations:**

| Setup | Factor 1 (role key) | Factor 2 (2FA) |
|-------|-------------------|---------------|
| Hardware wallet + password | Ledger/Trezor signs | Type password in CLI |
| Hardware wallet + TOTP | Ledger/Trezor signs | Enter Google Auth code in CLI |
| Software wallet + password | Local key signs | Type password in CLI |
| Software wallet + TOTP | Local key signs | Enter Google Auth code in CLI |
| Hardware wallet + password + TOTP | Ledger/Trezor signs | Password AND Google Auth (3FA) |

### 2.3.3 Account Recovery

If a user loses access to their 2FA (forgotten password, lost TOTP device) but still has their 12-word mnemonic:

1. Submit a **recovery request** using the Owner key (this is the ONE transaction type that bypasses 2FA)
2. **72-hour time-lock** begins — the recovery request is announced on-chain
3. During the 72 hours, if the real owner (with valid 2FA) submits a **contest transaction**, the recovery is blocked and the attacker's attempt fails
4. After 72 hours with no contest — 2FA resets, user sets a new password/TOTP

This mirrors how banks handle lost 2FA credentials: a delay period with the opportunity to intervene. The 72-hour window gives the real owner time to notice and stop unauthorized recovery attempts.

### 2.4 Purpose-Built Contracts

BTCPC does not support arbitrary smart contracts. Like Bitcoin's Script, BTCPC has a limited set of purpose-built contract types hardcoded into the protocol:

| Contract Type | Required Key | Purpose |
|--------------|-------------|---------|
| **Transfer** | Active | Move BTCPC between accounts |
| **Staking** | Active | Lock/unlock BTCPC for mining eligibility |
| **Escrow** | Active | Hold BTCPC for inference payment until delivery confirmed |
| **Slashing** | Protocol | Automatically slash stakes for consensus violations |
| **Claim** | Active | Generate signed proofs for cross-chain reward claiming |
| **MultiSig** | Active | M-of-N approval requirement on an account |
| **RecurringPay** | Active (once) | Authorized periodic transfers to a target wallet |
| **AccountUpdate** | Owner | Change password, rotate keys, update authorities |
| **NodeRegister** | Posting | Register/update mining node configuration |
| **EpochCommit** | Posting | Submit epoch work commitment |

No Solidity. No EVM. No arbitrary code execution. This is a deliberate design choice: the BTCPC chain does one thing — verifiable compute — and does it securely. Developers who want full smart contract capability use wrapped BTCPC (wBTCPC) on Base, Ethereum, or any EVM chain.

### 2.4 Epochs

Time in BTCPC is measured in **epochs**. One epoch = 5 minutes (~60 blocks at 5-second intervals).

At each epoch boundary:
1. Nodes submit their epoch commitment: `{state_hash, tx_count, inference_count, proof_samples[]}`
2. The network reaches consensus on the canonical state
3. Block rewards are distributed to honest miners
4. Cross-chain claim proofs are generated for linked wallets
5. Difficulty adjusts if threshold reached

### 2.5 The Work Function

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

Running a 70B model earns 16x more than a 7B model per token generated — reflecting the real compute cost difference.

---

## 3. Verification: Proving Compute Is Real

The core challenge: how do you verify that a node actually performed the inference it claims, without re-running every computation?

### 3.1 The Verification Trilemma

Any verification system must balance:
- **Security** — cheaters must be caught
- **Efficiency** — verification must cost less than the work itself
- **Speed** — verification must not bottleneck the network

BTCPC achieves 100% verification through redundant computation with a cryptographic commit-reveal scheme.

### 3.2 Redundant Computation with Commit-Reveal

Every inference request is assigned to multiple nodes simultaneously. A cryptographic commit-reveal scheme prevents result copying.

**Phase 1 — Commit:**
1. Network assigns the same inference request to N nodes (N=3 for standard, N=5 for high-value)
2. Each node runs the inference independently with deterministic parameters (temperature=0, fixed seed derived from epoch + request hash)
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
- Genesis (solo miner): N=1 (no redundancy needed)
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

### 3.5 Why This Works (Game Theory)

For a rational miner with stake S and epoch earnings E:

```
Expected value of honest mining = E
Expected value of cheating = E × savings_from_faking - S × 0.25 × P(caught)
```

With commit-reveal redundant computation, P(caught) = 1.0 for any request assigned to multiple nodes. Since S >> E (minimum stake requirement ensures this), cheating is always negative expected value. The system is **incentive-compatible** — honest behavior is the dominant strategy regardless of what other miners do.

---

## 4. Inference Protocol: End-to-End Encrypted Compute

### 4.1 Mandatory Encryption

All inference on the BTCPC chain is end-to-end encrypted. This is not optional. No plaintext prompt or result ever touches the chain or is visible to validators, other nodes, or the public.

```
On-chain (public):                     Off-chain (private):
  prompt_hash (sha256)                   actual prompt (plaintext)
  encrypted_prompt (memo key)            actual result (plaintext)
  result_hash (sha256)                   decryption keys
  encrypted_result (memo key)
  model, tokens_used, fee, timestamp
```

### 4.2 Encryption Flow

```
1. User submits inference request:
   - Encrypts prompt with ASSIGNED NODE's memo public key
   - Includes prompt_hash = sha256(plaintext_prompt)
   - Posts: {encrypted_prompt, prompt_hash, model, fee}

2. Network assigns request to N nodes for commit-reveal:
   - Each node decrypts the prompt using their memo private key
   - Runs inference with deterministic parameters
   - Computes result_hash = sha256(plaintext_result)
   - Encrypts result with REQUESTER's memo public key
   - Commits result_hash to the network
   - Discards the decrypted prompt from memory

3. Reveal phase:
   - Nodes reveal result_hashes
   - Consensus determined by matching hashes
   - Winning encrypted_result delivered to requester
   - Requester decrypts with their own memo private key

4. What remains on-chain:
   - prompt_hash, result_hash (for verification audit trail)
   - encrypted_prompt, encrypted_result (unreadable without keys)
   - model, tokens, fee, timestamp (operational metadata)
   - NO plaintext. Ever.
```

### 4.3 Privacy Guarantees

| Party | Can See Prompt? | Can See Result? |
|-------|----------------|-----------------|
| Requester | Yes (they wrote it) | Yes (they decrypt it) |
| Assigned compute nodes | Temporarily (decrypt to process, then discard) | Temporarily (generate, then discard after hashing) |
| Other nodes / validators | No (only see hashes) | No (only see hashes) |
| Public / block explorer | No | No |
| Chain operator / foundation | No | No |

### 4.4 Comparison with Centralized AI

| Property | OpenAI / Google / Anthropic | BTCPC |
|----------|---------------------------|-------|
| Who reads your prompt | The company, their employees, their training pipeline | Only the compute node, temporarily |
| Prompt storage | Stored on corporate servers indefinitely | Encrypted on-chain, only readable by requester |
| Data used for training | Often yes (unless opted out) | Never — nodes discard plaintext after processing |
| Censorship | Provider decides what you can ask | No censorship — encrypted, no one can read to censor |
| Subpoena risk | Company can be compelled to hand over logs | No plaintext exists to hand over |

### 4.5 Submitting Inference Requests

**Layer 1 — On-chain transaction (raw):**
```json
{
  "type": "inference_request",
  "requester": "alice",
  "model": "qwen3.5:27b",
  "prompt_hash": "sha256(prompt)",
  "encrypted_prompt": "<encrypted with node memo key>",
  "max_fee": "10 BTCPC",
  "primary_signature": "<active key>",
  "auth_factors": { "password_signature": "<2fa>" }
}
```
BTCPC held in escrow until delivery confirmed.

**Layer 2 — Node API (practical, OpenAI-compatible):**
```
POST https://node.btcpc.network/v1/inference
Authorization: Bearer btcpc_apikey_...

{
  "model": "qwen3.5:27b",
  "messages": [{"role": "user", "content": "What is 6 times 7?"}]
}

→ Node handles encryption, on-chain escrow, verification
→ Returns decrypted result to the API caller
→ Feels identical to calling OpenAI
```

**Layer 3 — SDK:**
```javascript
const btcpc = require('btcpc-sdk');
const client = new btcpc.Client({ apiKey: 'btcpc_...' });
const res = await client.inference({
  model: 'qwen3.5:27b',
  prompt: 'Explain quantum computing'
});
// Encrypted end-to-end. Paid in BTCPC. Verified on-chain.
```

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
- **Genesis month (solo mining):** 5% of supply — earned through real compute
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

During the genesis period (solo mining), 100% of the block reward goes to the single miner. As more nodes join, the reward distributes naturally.

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

## 5. Cross-Chain Mining Rewards

### 5.1 The Innovation

BTCPC introduces a mechanism never before implemented in cryptocurrency: **cross-chain mining rewards**. When a miner links wallets from other blockchains to their BTCPC mining node, they automatically earn claimable wrapped BTCPC (wBTCPC) tokens on every linked chain — simply by mining on the BTCPC chain.

This is not bridging. This is not wrapping. This is **simultaneous multi-chain value creation from a single act of compute.**

### 5.2 How It Works

```
Miner registers linked wallets:
  BTCPC:   btcpc1abc...        (native, always)
  Hive:    @thisthatjosh        (linked)
  Base:    0x1234...            (linked)
  Solana:  ABC123...            (linked)

Miner performs compute in epoch 1, earns 243 BTCPC:

  BTCPC chain:  +243.0  BTCPC      ← native reward (always)
  Hive:         +243.0  wBTCPC     ← cross-chain bonus (claimable)
  Base:         +243.0  wBTCPC     ← cross-chain bonus (claimable)
  Solana:       +243.0  wBTCPC     ← cross-chain bonus (claimable)
```

**The cross-chain bonus starts at 1:1 parity with the native reward and halves with each emission period:**

| Period | Duration | Native Reward | Cross-Chain Ratio | wBTCPC per Linked Chain |
|--------|----------|--------------|-------------------|------------------------|
| 1 | 1 month | 243.06 / epoch | 1:1 (100%) | 243.06 / epoch |
| 2 | 2 months | 137.85 / epoch | 1:2 (50%) | 68.93 / epoch |
| 3 | 4 months | 78.19 / epoch | 1:4 (25%) | 19.55 / epoch |
| 4 | 8 months | 38.01 / epoch | 1:8 (12.5%) | 4.75 / epoch |
| 5 | 16 months | 25.15 / epoch | 1:16 (6.25%) | 1.57 / epoch |
| 6 | 32 months | 14.24 / epoch | 1:32 (3.125%) | 0.45 / epoch |
| ... | ... | ... | ... | ... |

### 5.3 Key Rules

1. **Wallet must be linked at time of compute.** You cannot retroactively claim rewards for epochs before your wallet was linked. This prevents gaming — you must commit to the ecosystem to benefit.

2. **Claiming is the miner's responsibility.** The BTCPC chain generates a signed claim proof each epoch. The miner takes this proof to the target chain and submits it to the wBTCPC claim contract. The miner pays any fees on the target chain (Hive RC, Base gas, etc.). BTCPC pays nothing.

3. **One claim per epoch per chain.** Each epoch's reward can only be claimed once on each chain. The claim contract tracks claimed epochs.

4. **wBTCPC is freely tradeable** on each chain. Once claimed, wBTCPC-Hive trades on Hive DEXs, wBTCPC-Base trades on Base DEXs, etc. Each has its own independent market price.

5. **No cap on linked chains.** A miner can link as many chains as BTCPC supports. More linked chains = more total value earned per epoch of compute. This incentivizes miners to expand the BTCPC ecosystem.

### 5.4 Why This Works

**For miners:** Linking wallets is pure upside. Same compute work, more rewards. Every rational miner links every available chain.

**For the BTCPC ecosystem:** Every linked wallet means wBTCPC tokens appearing on another chain. This creates organic liquidity, market presence, and awareness on chains BTCPC never had to launch on. The miners themselves bootstrap the multi-chain ecosystem.

**For other chains:** They get a new tradeable token (wBTCPC) backed by real compute work, with zero effort. It just appears because miners linked their wallets.

**Economic sustainability:** The 1:1 ratio in period 1 is aggressive but limited — only 1 month of mining at that rate, with only 1-2 miners. By period 4, the ratio is 12.5%. By period 6, it's 3.125%. The cross-chain supply growth naturally decays while the native chain's value proposition strengthens.

### 5.5 Claim Proof Format

```json
{
  "chain": "base",
  "miner": "btcpc1abc...",
  "target_wallet": "0x1234...",
  "epoch": 42,
  "amount": "243.06000000",
  "period": 1,
  "cross_chain_ratio": "1.0",
  "linked_at_epoch": 0,
  "proof": "0x[SIGNED_BY_BTCPC_CONSENSUS_NODES]"
}
```

The claim contract on each chain verifies:
1. The proof signature is valid (signed by BTCPC consensus)
2. This epoch has not been claimed on this chain before
3. The target wallet matches the linked wallet in the proof
4. The amount matches the cross-chain ratio for the period

### 5.6 Supported Chains (Day 1 and Beyond)

**Day 1:**
- Hive (wBTCPC as custom_json token)
- Base (wBTCPC as ERC-20)

**Planned:**
- Solana (wBTCPC as SPL token)
- Arbitrum / Optimism
- TON
- Any EVM-compatible chain via standardized claim contract

Adding a new chain requires only deploying a wBTCPC claim contract on that chain. No changes to the BTCPC core protocol.

---

## 6. Genesis: The Beastly Epoch

### 6.1 Solo Mining Phase

BTCPC launches with a single miner: **Beastly** — a GPU-equipped machine running Ollama with multiple AI models.

This mirrors Bitcoin's genesis:
- Satoshi Nakamoto mined alone for months before anyone else joined
- The early blocks were nearly worthless
- The work was real, the tokens accumulated, and value emerged later

Genesis configuration:
- Models: qwen3.5:27b, deepseek-r1:8b, glm-4.7-flash
- Epoch interval: 5 minutes
- Initial block reward: 243 BTCPC per epoch
- Period 1 output: 2,100,000 BTCPC over 1 month (5% of total supply)
- Daily mining output: ~69,984 BTCPC (288 epochs × 243 BTCPC)
- Linked chains: Hive, Base
- Cross-chain bonus: 1:1 per linked chain

### 6.2 Genesis Block

```
Block 0 — BTCPC Genesis
Timestamp: [TBD]
Miner: Beastly
Models: qwen3.5:27b
State Hash: 0x0000000000000000000000000000000000000000000000000000000000000000
Message: "The Answer to the Ultimate Question of Life, the Universe, and Everything"
Reward: 243 BTCPC
Cross-Chain: 243 wBTCPC-Hive (claimable), 243 wBTCPC-Base (claimable)
```

### 6.3 Opening the Network

When the first external node operator wants to join:
1. They install the BTCPC node software
2. They create a BTCPC account on the native chain
3. They stake a minimum of 1000 BTCPC (received via transfer from genesis miner or purchased)
4. They link wallets for any chains they want cross-chain rewards on
5. They begin serving inference and earning rewards

The staking requirement creates a bootstrapping challenge: early nodes need BTCPC to stake, but BTCPC only comes from mining. This is solved by:
- **Genesis grants**: the solo miner distributes initial stakes to early node operators
- **Faucet**: a small amount available for new nodes to bootstrap
- **Delegation**: existing stakers can delegate stake to new nodes

---

## 7. Node Software

### 7.1 Reference Implementation

The reference BTCPC node is built in Node.js:

```
btcpc-node start \
  --ollama-url http://localhost:11434 \
  --stake 1000 \
  --link-hive @thisthatjosh \
  --link-base 0x1234...
```

### 7.2 Node Components

```
┌──────────────────────────────────────────────────┐
│                   BTCPC Node                      │
├───────────────┬───────────────┬──────────────────┤
│ P2P Layer     │ Inference     │ Epoch            │
│               │ Server        │ Worker           │
│ Block gossip, │ (Ollama       │                  │
│ tx relay,     │  proxy)       │ Computes state   │
│ peer          │               │ hash, submits    │
│ discovery     │ Serves        │ commitments,     │
│               │ requests,     │ generates claim  │
│               │ logs proofs   │ proofs           │
├───────────────┴───────────────┴──────────────────┤
│              State Manager (MongoDB)              │
│  Accounts, Balances, Stakes, Blocks, Proofs       │
├──────────────────────────────────────────────────┤
│              Chain Manager                        │
│  Block production, mempool, chain sync            │
├──────────────────────────────────────────────────┤
│              Claim Proof Generator                │
│  Signs cross-chain reward proofs for linked       │
│  wallets per epoch                                │
└──────────────────────────────────────────────────┘
```

### 7.3 Minimum Hardware Requirements

**Inference tier (earns full rewards + cross-chain bonuses):**
- GPU with 8GB+ VRAM (runs 7B+ models)
- 16GB RAM
- 4 CPU cores
- 100GB SSD
- 50 Mbps internet

**Relay tier (earns partial rewards, no inference):**
- No GPU required
- 8GB RAM
- 2 CPU cores
- 50GB SSD
- Transaction processing and state validation only
- Earns ~10% of what inference nodes earn
- Still earns cross-chain bonuses on linked wallets

---

## 8. Roadmap

### Phase 0: Genesis (Current)
- [x] Whitepaper
- [x] Architecture specification
- [x] Wallet controller (transfer, balance)
- [x] Staking controller (stake, unstake, withdraw)
- [x] Epoch system with emission schedule
- [x] Node registration
- [ ] P2P network layer
- [ ] Block production and chain sync
- [ ] Genesis mining loop on Beastly

### Phase 1: Solo Mining
- [ ] Beastly mining and accumulating BTCPC
- [ ] Cross-chain claim proof generation
- [ ] wBTCPC claim contract on Hive
- [ ] wBTCPC claim contract on Base (ERC-20)
- [ ] Mining dashboard (web UI)
- [ ] btcpc-cli tool

### Phase 2: Network Opening
- [ ] Node registration protocol (live)
- [ ] Stake requirement enforcement
- [ ] Commit-reveal verification system (N=3)
- [ ] Difficulty adjustment
- [ ] Block explorer

### Phase 3: Inference Market
- [ ] Public inference API (pay with BTCPC)
- [ ] Model marketplace (nodes advertise capabilities)
- [ ] Fee market and priority system
- [ ] Inference routing (match requests to best nodes)

### Phase 4: Maturity
- [ ] First halving (period 2 begins)
- [ ] Additional chain support (Solana, Arbitrum, TON)
- [ ] Governance (stake-weighted voting)
- [ ] Mobile relay node support
- [ ] Formal security audit

---

## 9. Conclusion

Bitcoin proved that decentralized proof of work can create sound money. BTCPC extends this insight in two fundamental ways:

**First**, the work that secures the network is useful. Every BTCPC token is backed by real AI compute — inference that someone wanted, paid for, and received. No energy is wasted.

**Second**, mining on one chain creates value on many. A single act of compute generates native BTCPC and claimable wBTCPC tokens on every linked blockchain. Miners don't just secure one network — they bootstrap liquidity across the entire crypto ecosystem.

The result is a network where:
- **Miners earn by doing useful work** (not burning energy on puzzles)
- **Users get censorship-resistant AI compute** (not dependent on any corporation)
- **The token has intrinsic utility** (you need it to buy inference, creating organic demand)
- **Supply is fixed and predictable** (42M total, doubling halving intervals, no burns)
- **Multi-chain liquidity emerges organically** (miners create it by linking wallets)

The answer is 42. The question was always about compute.

---

*"I think the problem, to be quite honest with you, is that you've never actually known what the question is."*
— Deep Thought, *The Hitchhiker's Guide to the Galaxy*

---

## Appendix A: Comparison with Bitcoin

| Property | Bitcoin | BTCPC |
|----------|---------|-------|
| Type | Sovereign chain | Sovereign chain |
| Total Supply | 21,000,000 | 42,000,000 |
| Work Function | SHA-256 (useless) | AI Inference (useful) |
| Block Time | ~10 minutes | ~5 minutes (1 epoch) |
| Halving Interval | Fixed 4 years | Doubling intervals (1mo → 2mo → 4mo → ...) |
| Verification | Check hash (instant) | Commit-reveal redundant computation (100%) |
| Mining Hardware | ASICs | GPUs + AI models |
| Multi-Chain | Bitcoin chain only | Native + claimable wBTCPC on linked chains |
| Smart Contracts | Script (limited) | Purpose-built (limited) |
| Fee Model | Transaction fees | Inference fees + transaction fees |
| Burns | None | None |
| Final Token Mined | ~2140 | ~2140 |

## Appendix B: Comparison with Compute Networks

| Property | Render | Akash | io.net | BTCPC |
|----------|--------|-------|--------|-------|
| Chain Type | Token on Ethereum/Solana | Cosmos chain | Token on Solana | Sovereign chain |
| Token Model | Utility (inflationary) | Utility (inflationary) | Utility (inflationary) | Fixed supply (42M) |
| Mining | No mining | No mining | No mining | Yes — earn by providing compute |
| Consensus | None (marketplace) | Tendermint (PoS) | None (marketplace) | Proof of Compute |
| Verification | Trust-based | Trust-based | Trust-based | Commit-reveal (100%) |
| Halvings | No | No | No | Yes (doubling intervals) |
| Multi-Chain Rewards | No | No | No | Yes (1:1 halving cross-chain) |
| Burns | No | No | No | No |
| AI-Native | No (general GPU) | No (general compute) | Partial | Yes (inference-first) |

## Appendix C: Cross-Chain Reward Supply Projections

Assuming genesis miner links 2 chains (Hive + Base) and the network grows to 100 miners by period 4, each linking an average of 3 chains:

| Period | Native BTCPC Minted | Cross-Chain Ratio | wBTCPC per Chain (est.) | Total wBTCPC All Chains |
|--------|--------------------|--------------------|------------------------|------------------------|
| 1 | 2,100,000 | 100% | 2,100,000 | 4,200,000 (2 chains) |
| 2 | 2,381,400 | 50% | 1,190,700 | 2,381,400 (2 chains) |
| 3 | 2,700,508 | 25% | 675,127 | 2,025,381 (3 chains avg) |
| 4 | 3,062,376 | 12.5% | 382,797 | 1,148,391 (3 chains avg) |
| 5 | 3,472,734 | 6.25% | 217,046 | 868,183 (4 chains avg) |

The cross-chain supply growth naturally decelerates while native BTCPC remains the primary store of value.
