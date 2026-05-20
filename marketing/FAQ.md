# Frequently Asked Questions

## General

### What is BTCPC?
Bitcoin Proof of Compute — a sovereign blockchain where miners earn tokens by providing useful AI inference instead of solving abstract hash puzzles.

### Is it a Bitcoin fork?
No. BTCPC is its own blockchain with its own genesis block, P2P network, consensus mechanism, and token. It's inspired by Bitcoin's principles but built from scratch.

### Is it a token on Ethereum?
No. BTCPC is a sovereign chain. However, mining BTCPC also generates claimable tokens on Ethereum, Solana, Bitcoin, TON, and Hive.

### Why "42"?
42 million total supply. It's the Answer to the Ultimate Question of Life, the Universe, and Everything (Douglas Adams, The Hitchhiker's Guide to the Galaxy). We're serious about our engineering, but we don't take ourselves too seriously.

### What's a "dream"?
The smallest unit of BTCPC — 1 BTCPC = 100,000,000 dreams. Like satoshis in Bitcoin. The name reflects the concept: every token is a dream computed into reality by AI.

## Mining

### How do I mine BTCPC?
Install Node.js, clone the repository, run `node bin/btcpc-mine`. You need a GPU and an internet connection. The miner automatically connects to the network and starts earning.

### What hardware do I need?
Any modern GPU with 8GB+ VRAM. The miner runs open-source AI models (like qwen3) on your hardware. Better GPU = more work completed = more tokens earned.

### Can I mine on my regular computer?
Yes. The miner auto-detects when you're using your PC and reduces intensity. Full speed when you're away, gentle when you're at the keyboard. You can also set CPU/GPU limits.

### Does mining use a lot of electricity?
It uses what any AI inference workload would use — comparable to running a local ChatGPT-like model. The key difference from Bitcoin: every watt produces useful AI output, not wasted heat.

### What's a clock node?
A lightweight node that keeps the network's timing synchronized. It doesn't mine (no GPU needed) — it just ensures epochs happen on schedule. Clock nodes earn 2% of each block reward. You can run one on a phone or Raspberry Pi.

## Tokens & Economics

### How many BTCPC will ever exist?
42,000,000. No more will ever be created.

### How are new tokens created?
New tokens are minted every 5-minute epoch and distributed to miners (98%) and clock nodes (2%) who were active during that epoch. The reward decreases over time.

### What gives BTCPC value?
Users pay BTCPC to access AI inference on the network. This creates real demand — as more people use the AI, more tokens are needed, creating natural buy pressure.

### What can I buy with BTCPC?
AI inference (ask the network questions, generate content, analyze data), and anything someone is willing to sell for BTCPC.

## Security

### Can someone hack the network?
Attacking the network requires controlling the majority of mining compute AND economic stake, which becomes increasingly expensive as the network grows. Multiple verification layers make fraud detectable.

### What if I lose my password?
Your 12-word mnemonic phrase is your master recovery key. It restores all four key pairs and all cross-chain addresses. If you lose the mnemonic, the account is unrecoverable — no company can reset it for you.

### Is my data private?
Inference requests are distributed across miners, so no single miner sees all traffic. On-chain data (balances, transfers) is public, like Bitcoin. Account identities are pseudonymous (usernames, not real names).

## Technical

### What AI models does it use?
Currently qwen3 (4B and 9B parameter models) and qwen3.5 (9B and 27B). Any Ollama-compatible model can be used. Miners can run whatever models they have hardware for.

### How fast is it?
Epochs (blocks) every 5 minutes. Transactions are visible across the network in under a second (mempool broadcast), settled permanently in the next epoch.

### Is it open source?
Yes. The entire codebase is available for audit. No hidden servers, no closed-source components.

### How does cross-chain work?
When a miner earns BTCPC, the protocol generates cryptographic proofs that can be claimed on other blockchains. The miner's wallet addresses on each chain are derived from the same 12-word mnemonic — one identity, multiple chains.

## Getting Started

### How do I get BTCPC tokens?
1. **Mine them** — run the miner on your GPU
2. **Run a clock node** — earn 2% rewards on any device
3. **Receive from someone** — anyone can send you tokens
4. **Faucet** — new accounts can claim a small amount for free

### How do I create an account?
Send `/create <username>` to the BTCPC Telegram bot (@btcpcbot). It generates your 12-word mnemonic and all keys. Save them immediately — we don't store them.

### Where can I see my balance?
- Telegram: send `/balance` to @btcpcbot
- Explorer: visit localhost:4242/account/yourusername (when running a node)
- CLI: `node bin/btcpc-cli wallet balance`
