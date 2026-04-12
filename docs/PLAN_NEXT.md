# BTCPC Next Phase Plan

> Everything from v2.10 through v2.16-alpha is built. This plan covers WIRING, HARDENING, and NEW capabilities not in any prior plan.

## Wave 1 — Unblock the chain + auto-update (URGENT)

### 1.1 Container auto-update mechanism
Docker containers don't auto-update. Nodes running old images (josh, shin, nick) are stuck on broken code.

- Create `btcpc.net/btcpc-image-version.txt` — a tiny file with the current image sha256
- Add `bin/btcpc-auto-update` — a script that runs inside the container every hour:
  - Fetches the version file from btcpc.net
  - Compares to local image hash
  - If different: downloads new tarball, docker load, signals supervisor to restart children
- Wire into `bin/btcpc-all` as a background loop (not a separate role — runs alongside all roles)
- **Files**: new `bin/btcpc-auto-update`, edit `bin/btcpc-all`

### 1.2 Role-selection installer
Per user feedback: installers must ask what roles to run. Clock is bare minimum. Ollama/mining is opt-in.

- Add role selection prompt to `.bat`, `.ps1`, `install.sh`:
  - [1] Clock only (~50 MB, earns 5% clock rewards)
  - [2] Clock + Miner (needs Ollama ~2 GB, earns mining + clock)
  - [3] Clock + Storage (needs disk, earns storage + clock)
  - [4] Full node (all roles)
- Only install Ollama if miner role selected
- Set `BTCPC_ROLES` env var based on choice
- Build a `btcpc-clock:latest` mini Docker image (~50 MB) for clock-only users
- Move `ollama` service in docker-compose.yml to `profiles: [miner]`
- **Files**: `website/btcpc-start.bat`, `.ps1`, `install.sh`, `website/docker-compose.yml`, `Dockerfile.clock` (new)

### 1.3 Rebuild + push Docker image with v2.14-v2.16
Current hosted image is v2.13.5. It doesn't have bridge, IoT, or stateful compute code.

- Rebuild from current main
- Re-save tarball to `website/btcpc-image.tar.gz`
- Update `btcpc-image-version.txt`
- **Files**: `website/btcpc-image.tar.gz` (gitignored), new `website/btcpc-image-version.txt`

## Wave 2 — Wire the primitives (connect what's built)

### 2.1 IoT HTTP routes
`sensorRegistry.js` and `loraGatewayRegistry.js` have no REST layer.

- New `src/routes/sensorRoutes.js`:
  - POST /api/sensors — register sensor
  - POST /api/sensors/:id/readings — submit reading
  - POST /api/sensors/:id/retire
  - GET /api/sensors — list (filterable by region/type)
  - GET /api/sensors/:id — single sensor + stats
  - POST /api/gateways — register gateway
  - POST /api/gateways/:id/heartbeat
  - GET /api/gateways — list
- **Files**: new `src/routes/sensorRoutes.js`, new `tests/sensorRoutes.test.js`

### 2.2 Bridge HTTP routes
`bridgeRegistry.js` and `bridgeFeeDistributor.js` have no REST layer.

- New `src/routes/bridgeRoutes.js`:
  - POST /api/bridge/wrap — wrap BTCPC to wBTCPC
  - POST /api/bridge/unwrap — unwrap wBTCPC to BTCPC
  - POST /api/bridge/fund — fund the bridge (become an LP)
  - POST /api/bridge/unlock — request LP unlock
  - GET /api/bridge/chains — list supported chains
  - GET /api/bridge/chains/:id — chain config + state
  - GET /api/bridge/fees — current fee schedule
  - GET /api/bridge/lps — LP leaderboard by weight
- **Files**: new `src/routes/bridgeRoutes.js`, new `tests/bridgeRoutes.test.js`

### 2.3 Oracle feed HTTP routes
`oracleFeeds.js` has no REST layer.

- New `src/routes/oracleRoutes.js`:
  - POST /api/oracles/feeds — register feed
  - POST /api/oracles/feeds/:id/reports — submit report
  - POST /api/oracles/feeds/:id/finalize — finalize epoch
  - GET /api/oracles/feeds — list feeds
  - GET /api/oracles/feeds/:id — feed + current value
- **Files**: new `src/routes/oracleRoutes.js`, new `tests/oracleRoutes.test.js`

### 2.4 Sensor data → BTCPC-FS persistence
Finalized sensor readings stored as blobs, CIDs recorded on chain.

- New ledger entry: `SENSOR_DATA_COMMIT`
- stateStore dispatcher case for SENSOR_DATA_COMMIT
- `sensorRegistry.finalizeEpochReadings` → `blobStore.putBlob` → `ledger.recordSensorDataCommit`
- **Files**: edit `src/services/ledger.js`, `src/chain/stateStore.js`, `src/services/sensorRegistry.js`

### 2.5 Bridge → stateStore + ledger
Wire bridge operations into the chain.

- New ledger entries: `BRIDGE_WRAP`, `BRIDGE_UNWRAP`, `BRIDGE_FUND`, `BRIDGE_UNLOCK`
- stateStore dispatcher cases for each
- Bridge state tracked in stateStore Maps
- **Files**: edit `src/services/ledger.js`, `src/chain/stateStore.js`

### 2.6 IoT as 6th reward pool
Add IoT pool to the 5-pool reward distribution.

- Modify `src/mining/miner.js` reward calculation:
  - Current: 60% miner, 10% verifier, 5% clock, 15% storage, 10% service
  - New: 55% miner, 10% verifier, 5% clock, 12% storage, 8% service, 10% IoT
  - Or: keep 5 pools but carve IoT from the service pool
- **Files**: edit `src/mining/miner.js`, `src/mining/rewardDistribution.js`

### 2.7 Anchor → miner integration
Wire `anchorSubmission.js` into epoch finalization.

- At each epoch, check `shouldAnchor(epoch, tier)` for all tiers
- If yes, include anchor data in the block payload
- **Files**: edit `src/mining/miner.js`

## Wave 3 — User-facing features (new)

### 3.1 Block explorer upgrade
Wire `src/explorer/server.js` to show real stateStore data.

- Accounts page: list all accounts with balances
- Blocks page: browse blocks with ledger entries
- Services page: deployed services + hosts + sessions
- Sensors page: IoT sensor map with live readings
- Bridge page: wrap/unwrap volume, LP leaderboard
- **Files**: major edit to `src/explorer/server.js`, new frontend templates

### 3.2 Solidity contracts for wBTCPC
The actual ERC20 contracts for the bridge.

- `contracts/wBTCPC.sol` — ERC20 with 4.2M supply, no mint, no burn, just transfers
- `contracts/BridgeLock.sol` — source-side lock contract
- `contracts/BridgeReserve.sol` — destination-side reserve
- Hardhat/Foundry test suite
- Deployment scripts for Base, Arbitrum, Ethereum
- **Files**: new `contracts/` directory

### 3.3 SDK / npm package
`btcpc-sdk` for third-party developers.

- `sdk/index.js` — client library wrapping the HTTP API
- Methods: `transfer`, `getBalance`, `deployService`, `submitReading`, `wrap`, `unwrap`
- Published to npm as `btcpc`
- **Files**: new `sdk/` directory

### 3.4 CI/CD pipeline
GitHub Actions for automated testing + Docker image builds.

- `.github/workflows/test.yml` — run jest on every push
- `.github/workflows/docker.yml` — build + push image to btcpc.net on tag
- **Files**: new `.github/workflows/`

## Execution order

1. **Wave 1.1** (auto-update) + **1.2** (role-selection) + **1.3** (rebuild image) — parallel agents, different files
2. **Wave 2.1-2.3** (HTTP routes) — parallel agents, all new files
3. **Wave 2.4-2.7** (chain wiring) — sequential, overlapping files
4. **Wave 3** — parallel, all new files/directories
