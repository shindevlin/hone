# BTCPC Network — Combined Revenue Forecast
**All Services: Sensors + Storage + Inference + Commerce + Cross-Chain**
*Shin Devlin — April 2026*

---

## What We've Built

BTCPC is not a single product. Every device and node on the network earns from multiple services simultaneously. The protocol takes a small cut of each:

| Service | What it does | Protocol cut |
|---------|-------------|-------------|
| **Sensor data marketplace** | Physical devices sell hyperlocal air quality, weather, seismic, noise, and mobility data to B2B buyers | 30% of data sale revenue |
| **AI inference** | Miners run verified LLM inference jobs (Ollama, any model) and earn per token generated | 10% of job value |
| **Decentralized storage** | Hosts store encrypted files and blobs, earn per epoch, challenged for availability | 15% of storage fees |
| **Cross-chain service payments** | Consumers on ETH/Solana pay for inference and storage in native tokens; provider earns on their chain | 5% of escrow value |
| **wBTCPC mint fees** | Each cross-chain credit claim costs 0.001 BTCPC on-chain + 0.0001 ETH on destination chain | 100% of mint fees |
| **Commerce** | Decentralized marketplace for digital and physical goods, escrow-secured | 2.5% of GMV |
| **Name auctions** | Premium BTCPC account names auctioned on-chain | 100% of auction proceeds |
| **Token creation** | User-issued tokens on BTCPC chain (42M supply, 10 decimals standard) | Small flat BTCPC fee |
| **IoT relay fees** | General and Macro nodes earn relay fees for forwarding Micro device packets | Small per-packet fee |

---

## The Flywheel

These services are not independent. They reinforce each other:

```
More devices deployed
        ↓
More sensor data → higher B2B data value → more BTCPC rewards to owners
        ↓
More owners → more staked BTCPC → more Epoch Bandwidth → more operations
        ↓
More inference demand → more miners → more verified compute
        ↓
More storage → more blobs → more cross-chain data access
        ↓
Cross-chain wBTCPC → more chains → more users → more devices deployed
```

A device owner who buys a General (~$220) earns from sensors, relay fees, and cross-chain credits simultaneously. A Macro owner earns from all nine revenue categories at once. Every participant has a financial incentive to grow the network.

---

## Market Sizing Per Service

### 1. Sensor Data Marketplace

**Comparable markets:**
- PurpleAir (70k air quality sensors) — acquired ~$25M
- Tomorrow.io (hyperlocal weather) — raised $200M
- WeatherFlow/Tempest (50k weather stations) — valued ~$30M
- HERE Technologies (mobility data) — valued ~$1B; pays ~$2/device/month for GPS probe data

**What makes BTCPC different:** 10 sensor categories per device vs. 1–2 for competitors. Every device contributes air quality, weather, noise, light, seismic, and mobility data simultaneously.

**Revenue model:** B2B API subscriptions + per-query pricing. Protocol sells the data, pays 70% to device owners as BTCPC rewards, retains 30%.

| Scale | Active devices | Avg value/device/yr | Total data revenue | Protocol 30% |
|-------|---------------|--------------------|--------------------|-------------|
| Y1 | 7,300 | $80 | $584k | **$175k** |
| Y2 | 44,000 | $180 | $7.92M | **$2.38M** |
| Y3 | 178,000 | $250 | $44.5M | **$13.35M** |

---

### 2. AI Inference (Mining)

**Comparable markets:**
- Global AI inference market: $25B (2024), projected $200B+ by 2030
- Decentralized compute: Akash Network (~$5M ARR), io.net, Gensyn, Bittensor
- A single consumer GPU (RTX 4090) earns $50–300/month running inference on centralized platforms

**What makes BTCPC different:** Proof-of-compute is the consensus mechanism. Every inference job is independently verified by verifier nodes running the same prompt. Work is provable, not self-reported.

**Revenue model:** 10% protocol fee on every settled inference job. Escrow system already built — requestor locks BTCPC, miner completes job, verifier confirms, escrow releases minus protocol fee.

| Scale | Active miners | Avg miner earnings/month | Monthly job volume | Protocol 10% annual |
|-------|--------------|--------------------------|-------------------|---------------------|
| Y1 | 500 | $80 | $40k | **$48k** |
| Y2 | 5,000 | $150 | $750k | **$900k** |
| Y3 | 50,000 | $200 | $10M | **$12M** |

*Miner earnings grow as more compute buyers enter the network. Cross-chain payments (ETH/SOL paying for BTCPC inference) add 20–30% additional volume on top.*

---

### 3. Decentralized Storage (BTCPC-FS)

**Comparable markets:**
- Filecoin: ~$250M raised; charges ~$0.002/GB/month
- Storj: pays hosts ~$1.50/TB/month; charges users ~$4/TB/month
- Arweave: permanent storage, ~$5 per GB
- AWS S3: $0.023/GB/month

**What makes BTCPC different:** Reputation-based host selection with settlement lag (no slashing — bad hosts just lose revenue). Encrypted split-shard storage with quantum-resistant unlinkable shards. Files replicated across minimum 3 hosts automatically.

**Revenue model:** 15% protocol cut on storage payments. Storage hosts receive 85%.

| Scale | Active hosts | Avg capacity | Total network storage | Monthly storage fees | Protocol 15% annual |
|-------|-------------|-------------|----------------------|---------------------|---------------------|
| Y1 | 300 | 100 GB | 30 TB | $120 | **$216** |
| Y2 | 5,000 | 500 GB | 2.5 PB | $10k | **$18k** |
| Y3 | 50,000 | 1 TB | 50 PB | $200k | **$360k** |

*Storage revenue is modest early — it requires data buyers, not just hosts. Grows significantly when inference outputs, sensor archives, and commerce product files are stored on BTCPC-FS.*

---

### 4. Cross-Chain Service Payments

**The model:** A user on Ethereum wants inference or storage. They escrow ETH/USDC in a smart contract on Ethereum. The BTCPC verifier quorum confirms work was done and signs a settlement transaction. ETH releases to the provider's Ethereum address. BTCPC chain issues block rewards as normal. The provider earns twice.

**Protocol revenue:** 5% of cross-chain escrow volume.

This is an additive layer on top of inference and storage — it doesn't replace BTCPC-native payments, it adds ETH/SOL/etc. volume from users who never touch the BTCPC chain directly.

| Scale | Cross-chain escrow volume/month | Protocol 5% annual |
|-------|--------------------------------|--------------------|
| Y1 | $10k | **$6k** |
| Y2 | $500k | **$300k** |
| Y3 | $5M | **$3M** |

---

### 5. wBTCPC Cross-Chain Mint Fees

Every time a device owner claims their accumulated wBTCPC credits:
- **0.001 BTCPC** on the BTCPC chain (routing to btcpc_recycle pool)
- **0.0001 ETH** (or equivalent) on the destination chain (split 50/50 treasury/staker pool)

10 supported chains: Ethereum, Base, Arbitrum, Optimism, Solana, TON, Bitcoin, Hive, BSC, Polygon.

| Scale | Annual claims | BTCPC fees | ETH-equivalent fees | **Total** |
|-------|--------------|------------|---------------------|-----------|
| Y1 | 15,000 | ~15 BTCPC | ~$150 | **~$300** |
| Y2 | 200,000 | ~200 BTCPC | ~$2,000 | **~$5k** |
| Y3 | 2,000,000 | ~2,000 BTCPC | ~$20,000 | **~$50k** |

*Mint fees are not a major revenue line — they exist for spam prevention. Their value is in the btcpc_recycle pool and staker pool, which fund ongoing rewards.*

---

### 6. Commerce (Digital + Physical Goods)

**The play:** Decentralized Amazon — product listings backed by stake, escrow-secured transactions, reputation-gated stores. Cross-chain payments mean buyers pay in ETH/USDC/SOL and sellers receive BTCPC.

**This is a long-term play.** Commerce revenue is negligible in Y1–Y2 but potentially the largest revenue line by Y4–Y5.

| Scale | Annual GMV | Protocol 2.5% |
|-------|-----------|--------------|
| Y1 | $50k | **$1.25k** |
| Y2 | $2M | **$50k** |
| Y3 | $20M | **$500k** |

---

### 7. Name Auctions

420 premium names reserved. Standard names available to anyone. Auction mechanics already built on-chain.

Premium name tiers (comparable to ENS .eth domain sales):
- Common words (weather, data, ai, etc.): $500–5,000
- Short names (3–4 characters): $1,000–20,000
- Tier 1 premiums (btcpc, bitcoin, satoshi, etc.): $10,000–100,000+

| Scenario | Auction revenue |
|----------|----------------|
| Conservative (most names <$1k) | $150k |
| Base (mix of tiers) | $500k |
| Optimistic (premium names bid up) | $2M+ |

This is mostly a Y1 event — one-time revenue from the initial auction, then ongoing renewal fees (~10% of initial price per year).

---

## Combined Revenue Forecast

### Protocol Revenue (What the Business Earns)

| Revenue stream | Year 1 | Year 2 | Year 3 |
|----------------|--------|--------|--------|
| Sensor data (30% cut) | $175k | $2.38M | $13.35M |
| AI inference (10% cut) | $48k | $900k | $12M |
| Storage (15% cut) | <$1k | $18k | $360k |
| Cross-chain services (5% cut) | $6k | $300k | $3M |
| wBTCPC mint fees | <$1k | $5k | $50k |
| Commerce (2.5% GMV) | $1k | $50k | $500k |
| Name auctions (one-time) | $300k | $50k | $25k |
| **Protocol revenue subtotal** | **$530k** | **$3.70M** | **$29.29M** |

### Hardware Margin (from HARDWARE_PRODUCT_LINE.md)

| | Year 1 | Year 2 | Year 3 |
|-|--------|--------|--------|
| Hardware gross margin (40%) | $394k | $2.34M | $6.56M |

### Total Combined Gross Revenue

| | Year 1 | Year 2 | Year 3 |
|-|--------|--------|--------|
| Protocol services | $530k | $3.70M | $29.29M |
| Hardware margin | $394k | $2.34M | $6.56M |
| **TOTAL** | **$924k** | **$6.04M** | **$35.85M** |

---

## What Drives Each Transition

**Y1 → Y2 (bootstrapping to growth):**
The sensor data and inference markets don't produce meaningful revenue until there's density — enough devices in enough cities that B2B buyers can cover their territories. The name auction and hardware sales fund operations while the network builds density. The Flipper Zero / Meshtastic community provides early organic adoption.

**Y2 → Y3 (growth to scale):**
Cross-chain payments become real as wBTCPC launches on ETH/Base/Solana. Businesses in those ecosystems start paying for BTCPC inference and storage in native tokens without needing to hold BTCPC. The data marketplace has enough coverage to negotiate city-level contracts. Commerce begins generating meaningful GMV as reputation scores mature and buyers trust the escrow system.

---

## Token Value as a Parallel Track

Protocol revenue is what the business earns in fiat-denominated terms. But BTCPC also creates value through the token itself:

- **Epoch Bandwidth** requires staked BTCPC for every operation — more network activity = more staking demand = upward pressure on token price
- **42M fixed supply** — same supply cap as our design goal, same scarcity narrative as Bitcoin
- **btcpc_recycle pool** — all fees route here, redistributed as rewards — no value leaves the system
- **Cross-chain wBTCPC** on 10 chains exposes BTCPC value to Ethereum DeFi, Solana yield markets, and TON ecosystem simultaneously

At Year 3 network scale, if BTCPC reaches $10/token with 42M supply, the market cap is $420M. The treasury's protocol-owned tokens represent a meaningful balance sheet even before revenue is considered.

---

## Comparable Company Benchmarks

| Company | Revenue at comparable scale | Valuation | Multiple |
|---------|---------------------------|-----------|----------|
| PurpleAir (70k sensors) | ~$5M ARR (est.) | ~$25M (acquired) | ~5× |
| WeatherFlow (50k stations) | ~$8M ARR (est.) | ~$30M | ~3.75× |
| Akash Network (decentralized compute) | ~$5M ARR | ~$200M token cap | ~40× (token premium) |
| Helium (900k gateways) | ~$15M ARR | $5B peak token cap | ~333× (token premium) |
| Filecoin (decentralized storage) | ~$20M ARR | $7B peak token cap | ~350× (token premium) |

**BTCPC at Year 3:** ~$35M combined revenue, covering sensor + compute + storage + commerce + cross-chain in a single network.

At a conservative 5× revenue multiple (pure software/data company valuation): **$175M**
At a 15× multiple (SaaS/marketplace premium): **$535M**
With token market premium (comparable to Helium/Filecoin at similar scale): **$1B+**

---

## What Needs to Happen

The forecast above requires execution on a clear sequence:

1. **Hardware shipping** — Micro and General units need to reach 10k deployed devices for sensor data to have commercial value. This is the critical path.

2. **B2B data partnerships** — One signed contract with a weather data buyer or city government validates the data marketplace. This de-risks the entire sensor revenue line.

3. **wBTCPC contract deployment** — Contracts deployed on Base and Arbitrum (lowest gas, highest liquidity) first. This makes cross-chain payments real and brings ETH ecosystem users in.

4. **Inference marketplace launch** — The escrow and verifier system is built. The missing piece is a front-end marketplace where compute buyers post jobs and miners claim them. One page, one API.

5. **FCC/CE certification** — Required before hardware can be sold commercially in the US and EU. Budget $15–30k per device per region, 3–6 month timeline. Should begin now to avoid delaying Y1 hardware revenue.
