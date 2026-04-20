# BTCPC Network Update — April 20, 2026

**Bitcoin Proof of Compute** — sovereign blockchain where mining = useful AI inference. 42M BTCPC supply. No burn, ever.

---

## What Shipped This Week

### Blockchain Explorer (btcpcscan)
- Full SPA at `website/explorer.html` — Dashboard, Blocks, Accounts, Sensors, Search
- Hash routing, live block feed, account lookup, sensor registry all in one UI
- No backend dependency — reads directly from the chain

### Four-Tier Finality Anchoring
- Native finality always live
- L2 anchoring kicks in at 10 nodes
- Ethereum `OP_RETURN` anchoring at 1,000 epochs
- Bitcoin anchoring at 10,000 epochs
- Each tier inherits the full security of the outer chain — BTCPC checkpoints propagate outward automatically

### Device Stake & Rent Market
- 10 staking slots per IoT sensor, ranked by `rent_bid` descending
- Slot multipliers: 1.85× (top) down to 0.50× (bottom) — position matters
- Yield split: 70% device owner / 20% staker pool / 10% recycle
- Two rent modes: earnings-first or stake-depletion
- 1% entry tribute; stakers get free data access from devices they're staked on

### Sensor Globe
- `website/globe.html` — interactive 3D globe powered by globe.gl
- 7 device types with distinct markers; staked devices pulse
- GPS privacy halos protect exact coordinates
- Click popups with live device stats — real-time network visualization

### Name Auction (420 Premium Names)
- 5-tier node gate: 4-char+ at 100 nodes, 3-char at 250, premium at 1,000, 2-char at 2,500, 1-char at 25,000
- Pay with USDC / USDT / DAI on Ethereum, Solana, or TON
- shindevlin delegates names into the auction pool; dashboard at `website/name-auction.html`
- Unavailable names shown as coming-soon — no ambiguity about what's live

### Buy BTCPC with Stablecoins
- `website/buy.html` + `bin/btcpc-purchase-watcher` daemon
- Accepts USDC / USDT / DAI on Ethereum, Solana, TON
- Payment watcher polls all three chains every 60 seconds
- Quote calculator, in-browser key generation, step-by-step instructions, live order tracking

### Nested Wallets
- `WALLET_CREATE_CHILD` protocol — accounts can spawn child wallets (e.g. `btcpc/treasury`)
- Parent authorizes child; enables organizational multi-purpose account structures
- Clean on-chain — no off-chain coordination needed

### btcpc_recycle Perpetual Endowment
- When all 42M BTCPC are distributed, mining rewards automatically switch to recycling transaction fees through `btcpc_recycle`
- Phase 2 activates with no manual intervention — no governance vote, no cutover drama
- Hard rule: no burn, ever. All fees return to the network

### Self-Heal Auto-Repair (P1–P3)
- Ollama poll timeouts, secretStore backup failures, blobStore null returns, clock peer-zero crashes, and storage port retries all auto-repair without operator intervention
- Circuit breaker drops chronically crashing roles before they destabilize the node
- Home users never see `[ERROR] do X` — the node fixes itself

### Android APK
- First Android build complete: 376MB debug APK via Capacitor
- App wraps the full BTCPC website — wallet, explorer, sensor globe, buy flow all included
- Early build; production release follows once chain stabilizes at scale

### Sybil & Faucet Hardening
- Inference protocol: requester cannot be their own prover — self-challenge is banned
- A model must run on 3+ distinct miners before it earns rewards — kills private-model farming
- Faucet: account must be at least 1 hour old before first claim

---

## By the Numbers

- **Supply:** 42,000,000 BTCPC (1 BTCPC = 100M dreams)
- **Epoch:** 30 seconds
- **Explorer:** port 4242 | **P2P:** port 6942
- **Finality tiers:** 4 (native → L2 → Ethereum → Bitcoin)
- **Sensor stake slots:** 10 per device, competitive bid market
- **Payment rails:** USDC / USDT / DAI on ETH / SOL / TON

---

## Get In

- **Mine** — run any Ollama model (qwen, llama, mistral, gemma, deepseek). Rewards scale with verified parameter count, not the model name. `node bin/btcpc-mine --miner <yourname>`
- **Run a sensor** — attach an IoT device, earn BTCPC for data delivery, open 10 staking slots to the network
- **Stake a sensor** — bid on open slots, earn yield, get free data access
- **Buy BTCPC** — `website/buy.html` — stablecoins accepted, keys stay in your browser
- **Grab a name** — 420 premium names going to auction as the node count climbs

Chain is live. Nodes are running. The network is open.
