# BTCPC — Bitcoin Proof of Compute

## The problem

Every day, millions of GPUs burn electricity solving puzzles that produce nothing. Bitcoin mining consumes 150 TWh/year to generate hashes that nobody uses. Ethereum moved to proof-of-stake, but replaced one waste with another: capital lockup instead of useful work. Meanwhile, the world pays $500B/year for cloud computing, AI inference, and data storage — all routed through a handful of corporations.

## The idea

What if mining meant doing real work?

BTCPC is a blockchain where every token is earned by a machine doing something useful. AI inference. Data storage. Sensor reporting. Application hosting. The same GPUs, hard drives, and devices that currently mine worthless hashes instead serve real demand — and get paid for it.

## How it works

**You bring a device. Any device.**

- A PC with a GPU → runs AI models, earns mining rewards
- A laptop or phone → keeps epoch timing, earns clock rewards
- A NAS or spare disk → stores content-addressed data, earns storage rewards
- A Helium miner or Raspberry Pi → reports sensor data via LoRa, earns IoT rewards
- A cloud VM → hosts services and applications, earns service rewards

**Five reward pools split every block's emission:**

| Pool | Share | What earns it |
|------|-------|---------------|
| Miners | 60% | Running real AI inference via Ollama |
| Verifiers | 10% | Validating inference results |
| Clocks | 5% | Keeping epoch timing (any device) |
| Storage | 15% | Hosting files on BTCPC-FS |
| Services + IoT | 10% | Running apps or reporting sensor data |

Pools with no active participants recycle their share back into future rewards. Nothing is ever burned.

## Key design decisions

**No burn, ever.** All fees, slashed stakes, and unclaimed rewards flow to `btcpc_recycle` and are re-earned via future block rewards. The supply is 42,000,000 BTCPC, forever, in perpetual circulation. BTCPC is the "No Burn" chain.

**No slashing for going offline.** Storage hosts, sensor nodes, and service hosts are paid for delivery — never punished for absence. Your home internet goes down? You just stop earning until you're back. Your balance is untouched.

**No synthetic work.** Miners only earn from real inference jobs submitted by real users through the API. No make-work puzzles. If nobody needs AI inference today, the miner pool recycles. This creates natural demand pressure: the chain is only as valuable as the work it does.

**No MongoDB, no cloud dependency.** The blockchain IS the database. Block files on disk are the source of truth. An in-memory state cache rebuilds from blocks on startup. A new node downloads the latest state snapshot, verifies the Merkle root, and starts participating in minutes.

## The token

| Property | Value |
|----------|-------|
| Symbol | BTCPC |
| Total supply | 42,000,000 (fixed, like Bitcoin) |
| Decimals | 10 |
| Genesis reward | ~243 BTCPC per epoch |
| Epoch duration | 30 seconds |
| Halving | Every ~4 years (same schedule as Bitcoin) |

## Cross-chain bridge

BTCPC connects to Ethereum, Base, Arbitrum, and Bitcoin via a **lock-and-recycle bridge** — not burn-and-mint. When you wrap BTCPC to wBTCPC on Base, your native tokens are locked 1:1. When you unwrap, they're released. No minting, no burning, just transfers between reserve pools.

Each destination chain has a hard cap of 4,200,000 wBTCPC (10% of native supply), pre-minted at deployment. Bridge liquidity is provided by permissionless LPs who lock BTCPC with a variable commitment period (30 days to 4 years) and earn fees weighted by their lock duration — the same veCRV model that powers Curve Finance.

## Four-tier finality

BTCPC state is anchored to external chains for independent verification:

| Tier | Chain | Frequency | Purpose |
|------|-------|-----------|---------|
| Native | BTCPC | Every epoch (30s) | Working consensus |
| L2 | Base, Arbitrum | Every 100 epochs (~50 min) | Fast cross-chain verification |
| Ethereum | Ethereum | Every 1,000 epochs (~8 hrs) | Deep finality |
| Bitcoin | Bitcoin | Every 10,000 epochs (~3.5 days) | Ultimate anchoring |

## What makes this different

**Bitcoin** is digital gold. Store of value. Proof of Work produces nothing useful.

**Ethereum** is a world computer. Proof of Stake secures a VM. Smart contracts do the work.

**BTCPC** is the digital labor market. Proof of Compute means the mining itself IS the product. Every token represents real work done — an AI prompt answered, a file stored, a sensor reading verified, an application served.

The question isn't "why another blockchain?" The question is: "why are we still paying Amazon for cloud computing when millions of GPUs sit idle mining hashes that nobody uses?"

## Get started

```bash
# Clock node (any device, earns 5% of rewards)
HONE_MINER=yourname HONE_ROLES=clock node bin/btcpc-all

# Full node (GPU required for mining)
HONE_MINER=yourname node bin/btcpc-all

# Docker (Windows/Mac)
irm https://honemesh.net/btcpc-start.ps1 | iex
```

**Website:** https://honemesh.net
**Telegram:** @btcpcbot
**GitHub:** https://github.com/shindevlin/btcpc
**License:** AGPL-3.0
