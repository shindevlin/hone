# How BTCPC Works (Non-Technical)

## The Simple Version

1. **Users** send AI questions to the network (like asking ChatGPT something)
2. **Miners** answer the questions using their GPUs (like Bitcoin miners, but doing useful work)
3. **The network** verifies the answers are correct (multiple miners check each other)
4. **Miners get paid** in BTCPC tokens for providing correct answers
5. **Everything is recorded** on the blockchain permanently

## The Five-Minute Version

### Step 1: A User Asks a Question

A developer, app, or Telegram user submits an AI inference request. This could be anything: "Analyze this data," "Write this email," "Check this text for sensitive information," "Help me debug this code."

The user pays a small fee in BTCPC tokens (fractions of a cent per request).

### Step 2: Miners Compete to Answer

Miners are people who run AI models on their hardware (GPUs). When a request arrives on the network, miners claim it and run the AI model to generate an answer. This is the "Proof of Compute" — the miner proves they did useful work by producing a real AI output.

### Step 3: The Network Verifies

Multiple miners independently verify each answer. If they agree, the answer is accepted. If a miner submits a bad answer, they lose their staked tokens (called "slashing"). This ensures quality without any central authority.

### Step 4: Miners Get Paid

Each 5-minute "epoch" (like a Bitcoin block), the network distributes newly minted BTCPC tokens to miners based on how much useful work they did. More AI work = more tokens. This is the mining reward.

### Step 5: The Chain Records Everything

Every inference request, every answer, every payment, and every mining reward is recorded on the BTCPC blockchain permanently. Anyone can verify any transaction by reading the chain.

## Key Concepts (Plain English)

### Epochs (= Bitcoin Blocks)
Every 5 minutes, a new "epoch" starts. This is BTCPC's version of a Bitcoin block. All the AI work done during that 5 minutes gets bundled, verified, and recorded.

### Mining Rewards
New BTCPC tokens are created every epoch and distributed to miners. The initial reward is ~243 BTCPC per epoch. This decreases over time (like Bitcoin's halving) until all 42 million tokens are mined.

### Staking
Miners must put up a deposit (stake) to participate. If they cheat (submit wrong answers), they lose their stake. This creates economic incentive to be honest.

### Clock Nodes
Lightweight nodes that keep the network's timing synchronized. They don't need GPUs — they can run on a phone, laptop, or cheap computer. They earn a small reward (2% of each epoch) for keeping the network alive.

### Inference
The AI process of generating an output from an input. When you ask ChatGPT a question, that's inference. BTCPC miners do the same thing, but decentralized.

### Wallets
Every account has a 12-word recovery phrase (like a Bitcoin wallet) that gives access to all their tokens across all supported blockchains. One phrase = one identity everywhere.

## What Makes It Secure

- **Multiple miners verify every answer** — cheating requires fooling the majority
- **Economic stakes** — miners have tokens at risk (skin in the game)
- **Cryptographic proofs** — every piece of work is mathematically verifiable
- **Decentralized timing** — no single computer controls when things happen
- **Permanent record** — once something is on the chain, it can't be altered

## What You Can Do With BTCPC Tokens

- **Pay for AI inference** — ask questions, generate content, analyze data
- **Stake to mine** — put up tokens and earn rewards by providing compute
- **Transfer to others** — send tokens to any BTCPC address
- **Use across chains** — BTCPC exists on Ethereum, Solana, Bitcoin, TON, and Hive simultaneously
- **Hold as an asset** — limited supply (42M), increasing demand for AI compute
