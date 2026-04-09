#!/usr/bin/env node
"use strict";

/**
 * Generate ready-to-post Reddit content for r/btcpc.
 * Usage: node scripts/reddit-content.js [type]
 * Types: stats, milestone, weekly, launch
 */

const mongoose = require("mongoose");
const { getBlockReward } = require("../src/services/emissionSchedule");

const MONGO_URI = process.env.MONGODB_URI || "mongodb://root:example@localhost:27017/btcpc?authSource=admin";

const CHAINS = [
  { name: "Base", explorer: "https://basescan.org/address/0x25E434d38F4dEc7AF2F6f6488BAe34fBc5781D47" },
  { name: "Arbitrum", explorer: "https://arbiscan.io/address/0x25E434d38F4dEc7AF2F6f6488BAe34fBc5781D47" },
  { name: "Optimism", explorer: "https://optimistic.etherscan.io/address/0x25E434d38F4dEc7AF2F6f6488BAe34fBc5781D47" },
  { name: "Ethereum", explorer: "https://etherscan.io/address/0x25E434d38F4dEc7AF2F6f6488BAe34fBc5781D47" },
  { name: "Polygon", explorer: "https://polygonscan.com/address/0x25E434d38F4dEc7AF2F6f6488BAe34fBc5781D47" },
  { name: "BSC", explorer: "https://bscscan.com/address/0x25E434d38F4dEc7AF2F6f6488BAe34fBc5781D47" },
  { name: "Solana", explorer: "https://solscan.io/token/2WAQ9PMEMe7AVEvhCu2tcM5wJktwsThEJ9pWbEhibAof" },
  { name: "Hive", explorer: "https://hiveblocks.com/@shindevlin" },
];

async function getStats() {
  await mongoose.connect(MONGO_URI);
  const db = mongoose.connection.db;

  const epochs = await db.collection("epochs").countDocuments();
  const latestEpoch = await db.collection("epochs").findOne({}, { sort: { epoch_number: -1 } });
  const ledgerCount = await db.collection("ledgerentries").countDocuments();
  const miners = await db.collection("nodes").countDocuments({ isActive: true });

  // Sum mined rewards
  const miningEntries = await db.collection("ledgerentries").find({ type: "MINING_REWARD" }).toArray();
  const totalMined = miningEntries.reduce((sum, e) => sum + (e.amount || 0), 0);

  const currentReward = latestEpoch ? getBlockReward(latestEpoch.epoch_number) : getBlockReward(0);

  await mongoose.disconnect();

  return {
    epochs,
    latestEpoch: latestEpoch?.epoch_number || 0,
    ledgerCount,
    miners,
    totalMined: totalMined.toFixed(2),
    currentReward: currentReward.toFixed(4),
    supply: 42000000,
    chains: CHAINS.length,
  };
}

function generateStats(stats) {
  const title = `BTCPC Network Stats — Epoch ${stats.latestEpoch} | ${stats.totalMined} BTCPC Mined`;
  const body = `## Network Snapshot

| Metric | Value |
|--------|-------|
| Current Epoch | ${stats.latestEpoch} |
| Total Epochs | ${stats.epochs} |
| BTCPC Mined | ${stats.totalMined} / 42,000,000 |
| Block Reward | ${stats.currentReward} BTCPC/epoch |
| Active Miners | ${stats.miners} |
| Ledger Entries | ${stats.ledgerCount} |
| Live Chains | ${stats.chains} |

### Cross-Chain Deployments

${CHAINS.map(c => `- **${c.name}**: [View on Explorer](${c.explorer})`).join("\n")}

### Reward Split

- 85% Miners (proportional to compute work)
- 10% Verifiers (random panel checks output quality)
- 5% Clocks (any hardware, keep epoch timing)
- Idle epochs: 98% unminted, 1% verifiers, 1% clocks

---

*Mining is useful work. Every BTCPC is earned by running AI inference.*

[Website](https://btcpc.net) | [Explorer](https://btcpc.net:4242) | [GitHub](https://github.com/shindevlin/btcpc)`;

  return { title, body };
}

function generateMilestone(stats) {
  const title = `[Milestone] BTCPC reaches ${stats.epochs} epochs — ${stats.totalMined} BTCPC mined across ${stats.chains} chains`;
  const body = `We just hit **${stats.epochs} epochs** on the BTCPC network.

### What is BTCPC?

Bitcoin Proof of Compute is a sovereign blockchain where **mining = AI inference**. Instead of burning electricity on SHA-256 hashes, miners run AI models and earn tokens for real, useful work.

### Key Numbers

- **${stats.totalMined} BTCPC** mined out of 42M total supply
- **${stats.chains} chains**: Base, Arbitrum, Optimism, Ethereum, Polygon, BSC, Solana, Hive
- **${stats.miners} active miners** running inference
- **Block reward**: ${stats.currentReward} BTCPC per 5-minute epoch

### Three Roles, Any Hardware

| Role | Share | Hardware |
|------|-------|----------|
| Miners | 85% | GPU or CPU — diversity is a security feature |
| Verifiers | 10% | Any hardware — random panels check work quality |
| Clocks | 5% | Any hardware — phones, Raspberry Pi, desktops |

### How It Works

1. Users submit AI inference requests
2. Miners run the models and produce results
3. Random verifiers check output quality
4. Everyone gets paid proportionally

No staking required to mine. No special hardware. A mid-range desktop earns alongside a GPU rig.

---

[Website](https://btcpc.net) | [Whitepaper](https://github.com/shindevlin/btcpc/blob/main/docs/BTCPC_WHITEPAPER.md) | [GitHub](https://github.com/shindevlin/btcpc)`;

  return { title, body };
}

function generateLaunch() {
  const title = "Introducing BTCPC — Bitcoin Proof of Compute: a blockchain where mining means AI inference";
  const body = `## What if mining was useful?

Bitcoin proved that decentralized consensus works. BTCPC asks the next question: **what if the work that secures the chain also produces something people are paying for?**

BTCPC is a sovereign blockchain where miners run AI models instead of hashing. Every token is earned by real compute — inference that users actually requested and paid for.

### The Basics

- **Supply**: 42,000,000 BTCPC (same timeline as Bitcoin, all mined by ~2140)
- **Epochs**: 5 minutes (like Bitcoin blocks, but faster)
- **Mining**: Run any Ollama model — GPU or CPU
- **Verification**: Random verifier panels check output quality (no redundant computation)
- **Cross-chain**: wBTCPC live on 8 chains — Base, Arbitrum, Optimism, Ethereum, Polygon, BSC, Solana, Hive

### Three Roles

- **Miners** (85% of rewards) — run AI inference
- **Verifiers** (10%) — lightweight nodes checking work quality
- **Clocks** (5%) — any device keeping epoch timing (phones work)

Hardware diversity is a feature, not a bug. A network of identical GPUs is a monoculture — one exploit takes it all down. BTCPC wants every machine to be different.

### No Burns, No Tricks

- Tokens are never burned — slashed tokens go to btcpc_recycle
- No minting contracts — every token earned through mining
- Dormancy recycling instead of burning (5yr grace, 10%/yr decay, one tap resets the clock)
- All protocol parameters changeable by network consensus

### Try It

\`\`\`bash
npm install -g btcpc
btcpc-setup
\`\`\`

Or just run a clock node from your phone.

---

[Website](https://btcpc.net) | [Whitepaper](https://github.com/shindevlin/btcpc/blob/main/docs/BTCPC_WHITEPAPER.md) | [GitHub](https://github.com/shindevlin/btcpc) | [Telegram](https://t.me/btcpcbot)

*Built by Shin Devlin. Validated by Natoshi. Voiced by Josh.*`;

  return { title, body };
}

async function main() {
  const type = process.argv[2] || "stats";

  if (type === "launch") {
    const post = generateLaunch();
    console.log(`\n=== TITLE ===\n${post.title}\n\n=== BODY ===\n${post.body}\n`);
    return;
  }

  const stats = await getStats();

  let post;
  switch (type) {
    case "milestone":
      post = generateMilestone(stats);
      break;
    case "stats":
    default:
      post = generateStats(stats);
      break;
  }

  console.log(`\n=== TITLE ===\n${post.title}\n\n=== BODY ===\n${post.body}\n`);
}

main().catch(err => {
  console.error("Error:", err.message);
  process.exit(1);
});
