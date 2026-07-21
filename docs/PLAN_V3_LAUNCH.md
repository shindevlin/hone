# HONE v3.0 Launch Plan

> Build for millions. Restart clean. Go public.

## Overview

This plan takes HONE from the current 3-node dev network to a public
chain designed for millions of users. Five scaling phases ship BEFORE
the chain restarts at epoch 0. Then we go public, deploy Nebra gateways,
update local projects, and anchor to external chains.

---

## Pre-Launch: Engine Upgrades (build before chain restart)

### Phase 1 — Faster epochs + dynamic block cap

**Goal:** 30-second epochs, 1-32 MB adaptive block cap → ~1,000 TPS ceiling

**Changes:**
- `EPOCH_DURATION_MS`: 300,000 → 30,000
- `bin/hone-clock`: update `computeEpochNumber` timing, heartbeat intervals
- `src/chain/blockSizeCap.js`: replace fixed 1 MB with adaptive cap
  - Start at 1 MB
  - If block is >50% full: next block cap = min(current × 1.25, 32 MB)
  - If block is <25% full: next block cap = max(current × 0.75, 1 MB)
  - Recorded in block header so all nodes agree on the cap
- `src/mining/miner.js`: update epoch duration constants
- `src/services/epochManager.js`: update interval
- Emission schedule: block reward per epoch shrinks proportionally
  (same annual emission, 10× more epochs per year → 1/10 per epoch)
  Genesis reward: ~243 HONE/5min → ~24.3 HONE/30s

**Tests:** epoch timing, adaptive cap growth/shrink, emission math

### Phase 2 — Disk-backed stateStore

**Goal:** Handle billions of entries, survive restarts without replay, scale beyond RAM

**Changes:**
- `src/chain/stateStore.js`: replace `var balances = new Map()` etc. with
  LevelDB (npm `level` package) operations
- Same API surface: `getBalance`, `getAccount`, `applyEntry`, etc.
- All 1,151+ tests pass without changes (same interface, different backend)
- `resetAll()` clears the LevelDB instance
- Data dir: `$HONE_DATA_DIR/statedb/` (alongside blocks/)
- New node downloads statedb snapshot from any peer + verifies merkle root

**Tests:** same existing tests work; add LevelDB-specific tests for persistence across restarts

### Phase 3 — P2P mesh + finality anchoring contracts

**Goal:** Decentralized peer discovery, no single relay dependency. Anchor state to external chains.

**P2P changes:**
- Replace single Cloudflare relay with DHT-based peer discovery (libp2p or Kademlia)
- Keep the relay as a bootstrap node, but nodes find each other via DHT after initial connect
- Topic-based gossip: separate channels for blocks, transactions, clock heartbeats, sensor data
- Peer scoring: nodes that relay invalid/unsigned messages get deprioritized

**Finality anchoring Solidity contracts:**
- `contracts/HONEAnchor.sol` — receives state root hashes from anchor submitters
  - `submitAnchor(uint256 epoch, bytes32 stateRoot, bytes signature)` — verifies ECDSA signature from a registered authority
  - `getAnchor(uint256 epoch)` → `(bytes32 stateRoot, address submitter, uint256 timestamp)`
  - Deploy on: Base, Arbitrum, Ethereum, Bitcoin (via Stacks or ordinals)
- `contracts/wHONE.sol` — ERC20 for the bridge (v2.16-alpha spec, 4.2M per chain)
  - Constructor mints 4,200,000 to bridge reserve address
  - No `mint()`, no `burn()`, just standard ERC20 transfers
  - Immutable: no admin, no upgrade, no pause
- `contracts/BridgeLock.sol` — source-side lock contract
  - `lockForWrap(uint256 amount, uint256 destChainId)` — locks HONE native, emits event for the bridge relay
  - `unlockFromUnwrap(address recipient, uint256 amount)` — called by bridge relay after unwrap verification

**Tests:** Hardhat/Foundry test suite for all contracts

### Phase 4 — Snapshot sync

**Goal:** New nodes sync in minutes, not hours

**Changes:**
- `src/chain/snapshotSync.js` — new module
  - `serveSnapshot(req, res)` — HTTP endpoint that streams the LevelDB state as a compressed tarball
  - `downloadSnapshot(peerUrl)` — fetches the snapshot from any peer
  - `verifySnapshot(snapshot, expectedMerkleRoot)` — verifies integrity
  - `applySnapshot(snapshot)` — replaces local state with downloaded snapshot
- New node startup flow:
  1. Connect to peers
  2. Ask for latest finality block header (get the merkle root)
  3. Download state snapshot from the peer with the best uptime
  4. Verify merkle root matches
  5. Start participating (no block replay needed)
- Old blocks remain downloadable for historical queries via `GET /api/blocks/:epoch`

### Phase 5 — Parallel execution / sharding (post-launch)

**Goal:** 10,000+ TPS via state sharding

**Changes:**
- Partition stateStore by account prefix into N shards (start with 4: a-f, g-m, n-s, t-z)
- Each shard has its own consensus + block production running in parallel
- Cross-shard transfers go through an atomic two-phase commit:
  1. Source shard locks the amount
  2. Destination shard credits the amount
  3. Source shard finalizes the debit
  If step 2 fails, step 1 is rolled back
- The v2.16 bridge architecture already handles cross-chain atomics; cross-shard uses the same pattern internally
- This is the longest-term work and ships incrementally post-launch

---

## Pre-Launch: Content + Tooling

### Full whitepaper rewrite

The current whitepaper (`docs/HONE_WHITEPAPER.md`, 821 lines) reflects v0.3 from before most of the system was built. The rewrite must reflect the full v3.0 reality:

- Proof of Compute consensus (not PoW, not PoS — machines earn by doing useful work)
- Five-pool emission model (60/10/5/15/10 split)
- No Burn All Recycle economics
- HONE-FS content-addressed storage
- Stateless + stateful compute hosting
- IoT sensor mesh + LoRa gateways
- Lock-and-recycle cross-chain bridge
- Four-tier finality anchoring
- P2P security model (ECDSA signatures on all messages)
- LevelDB state store with snapshot sync
- 30-second epochs, dynamic block cap

This whitepaper gets inscribed in Dream #0 of the new chain.

### One-page whitepaper (DONE — `docs/HONE_ONEPAGER.md`)

### Genesis migration tooling

**`scripts/genesis-migration.js`:**
1. Replays current chain to get full stateStore
2. Exports all 19 accounts with balances, public keys, chain addresses
3. Generates genesis block entries:
   - Dream #0: new whitepaper inscription
   - 19 × ACCOUNT_CREATE (preserving all keys)
   - 19 × GENESIS_MINT (preserving exact balances)
   - System accounts: hone_recycle, hone_treasury
4. Writes to `data/genesis-migration.json`

**`scripts/reset-chain.js`:**
1. Reads `data/genesis-migration.json`
2. Wipes `data/blocks/`, `data/statedb/`, `data/pending-entries.jsonl`
3. Creates epoch 0 block with all migration entries
4. Initializes new LevelDB stateStore from the genesis entries
5. Writes new `data/blocks/block-00000000.bin`

### Nebra setup

**Target hardware:** Nebra Helium Indoor/Outdoor Hotspot (ARM-based, runs Linux)

**`bin/hone-nebra`:**
1. Detects hardware (ARM arch, LoRa concentrator chip)
2. Installs Node.js if missing (via nvm)
3. Clones hone repo (needs git + network)
4. Configures as: `HONE_ROLES=clock,storage,sensor`
5. Sets up LoRa packet forwarder to listen for sensor packets
6. Registers as gateway via `POST /api/gateways`
7. Starts `bin/hone-all` as a systemd service
8. First Nebra belongs to shin → `HONE_MINER=shindevlin`

**Setup instructions** (step-by-step for the user):
1. SSH into the Nebra: `ssh root@<nebra-ip>`
2. Run: `curl -fsSL https://honemesh.net/nebra-install.sh | bash`
3. Enter your HONE username when prompted
4. The script handles everything else (Node, git clone, LoRa config, systemd service)

**`website/nebra-install.sh`:** hosted installer for Nebra, similar pattern to install.sh but ARM-specific.

### Map function on honemesh.net

**`website/map.html`:**
- Leaflet.js map showing all registered gateways + sensors
- Fetches from `GET /api/gateways` and `GET /api/sensors`
- Markers colored by type (gateway=blue, temperature=red, humidity=green, etc.)
- Click a marker → popup with: device name, owner, region, last reading, uptime
- Auto-refreshes every 60 seconds
- Shin's first Nebra appears as the first pin on the map

### Local project updates

**`~/repos/honebot/`:** (Telegram wallet bot)
- Update API calls to use current endpoints
- Add inference job submission: bot can send `/infer <prompt>` → creates an InferenceJob → miners earn from real work
- Add auto-update: `scripts/auto-update.sh` does `git pull origin main && npm install && systemctl restart honebot`
- Cron: every 15 minutes

**`~/repos/honewalletbot/`:** (Telegram buy bot)
- Same auto-update pattern
- Add bridge commands: `/wrap <amount> <chain>`, `/unwrap <amount>`
- Add sensor commands: `/sensors`, `/readings <sensor-id>`

---

## Launch Sequence

```
Step 1: Build Phase 1+2 (faster epochs + LevelDB stateStore)
Step 2: Wire remaining chain integrations (sensor→FS, bridge→ledger, IoT pool)
Step 3: Rewrite full whitepaper
Step 4: Build genesis migration scripts
Step 5: Build Nebra installer + map page
Step 6: Test full migration on a local fork
Step 7: Stop all nodes
Step 8: Run genesis-migration.js → captures current state
Step 9: Run reset-chain.js → new epoch 0
Step 10: Make repo public
Step 11: Push new genesis to git
Step 12: Start all nodes (shin WSL, josh WSL, nick Docker)
Step 13: Install Nebra (shin's first gateway)
Step 14: Update local bots
Step 15: Announce on Substack + Telegram + Reddit
```

## Post-Launch

```
Step 16: Phase 3 — P2P mesh + Solidity contracts for anchoring + bridge
Step 17: Phase 4 — Snapshot sync protocol
Step 18: Phase 5 — Sharding (when TPS demands it)
Step 19: DEX liquidity provisioning (Uniswap V3 pools for wHONE)
Step 20: SDK + developer documentation site
Step 21: Native Windows .exe installer (Inno Setup)
Step 22: Mobile app (React Native — balance, sensor data, mining status)
```

---

## Recommended immediate next steps

1. **Dispatch agents for Phase 1 + Phase 2** (parallel, different files)
2. **Start the full whitepaper rewrite** (me, in parallel — content, not code)
3. **Build genesis migration scripts** (me, after Phase 2 lands — needs LevelDB)
4. **Build Nebra installer + map** (agent, after sensor→FS wiring)
5. **Everything else follows the launch sequence above**

Estimated time to Step 10 (go public): 2-3 more sessions if we keep this pace.

---

## Post-Launch Roadmap — Hardware + Partnerships

### Flipper Zero integration (post-genesis)
- Custom firmware for Flipper Zero as a HONE hardware wallet + mobile sensor
- NFC tap-to-authenticate: tap Flipper to sign transactions instead of passwords
- GPIO sensor input: connect DHT22/BME280 via GPIO pins → mobile IoT data collection
- Sub-GHz radio: short-range sensor receiver (~100m) for environments without LoRa coverage
- BLE relay: bridge BLE sensor beacons to HONE network
- Flipper becomes a pocket-sized HONE node that earns IoT rewards while walking around

### ADS-B flight tracking (Wingbits integration)
- $25 USB ADS-B dongle plugged into Nebra's USB port
- Receives airplane transponder signals → earns WINGS tokens
- Same device (Nebra) earns HONE IoT rewards + Wingbits WINGS simultaneously
- Hyfix Wingbits serial discovered: REDACTED_SERIAL

### Additional LoRa sensors
- Dragino LHT65 (temp+humidity, ~$20) — outdoor environmental monitoring
- Dragino LSE01 (soil moisture, ~$25) — agriculture data
- RAK7204 (multi-sensor, ~$30) — temp+humidity+barometer+gas
- All transmit Cayenne LPP format → Nebra gateway receives automatically
- Each sensor earns from 60% of IoT reward pool (sensor share)

### GNSS revenue optimization
- Fix onocoy TLS connection → 4th revenue stream from Hyfix
- Email GEODNET support → deregister previous owner → $96 GEOD/day potential
- Optional: USB GNSS receiver ($30-150) for direct NTRIP without ARP spoofing

### Paid data API (api.honemesh.net)
- REST API selling sensor data to external consumers
- Fiat (Stripe) + stablecoin + HONE payment options
- Pricing tiers: free (100 calls/day), developer ($10/mo), enterprise (custom)
- Revenue split: 70% data owner (gateway/sensor) / 20% recycle / 10% storage hosts
- GNSS correction data: highest value ($500-5000/mo for precision positioning)
- Environmental data: moderate value ($50-500/mo per region)
- UWB positioning data: premium value ($500-10000/mo for asset tracking)

### Cross-DePIN aggregator
- Single device (Nebra + sensors + Hyfix) earns from multiple DePIN networks:
  - HONE (IoT + clock + storage rewards)
  - GEODNET (GEOD tokens for GNSS corrections)
  - RTK Direct (RTK tokens for GNSS data)
  - onocoy (ONO tokens for GNSS coverage)
  - Wingbits (WINGS tokens for ADS-B flight tracking)
  - WeatherXM (WXM tokens for weather data — needs weather station)
- HONE chain monitors all earnings across networks via cross-chain address monitoring
