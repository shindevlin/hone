# Self-Heal PRD

> **Hard rule:** every BTCPC fail path must auto-repair. Non-technical home users cannot run commands. Replace every `[ERROR] do X` with an automatic action. See `~/.claude/projects/-home-ubuntclaw-repos-btcpc/memory/feedback_self_heal_no_asks.md` for the full spec.

This file is the canonical backlog of every place in the codebase that violates the self-heal rule. Agents pick the next `[ ]` item, fix it, run tests, commit atomically, and tick the box.

When the list is empty, BTCPC is fully self-healing for non-technical home users.

---

## P0 — User-facing install paths (non-technical users hit these first)

- [x] **`website/btcpc-start.bat`** — every `[ERROR] ... pause ... exit /b 1` deathtrap. Rewrite under self-heal rule:
  - `where docker` fails → poll for Docker Desktop on PATH for 60s, then attempt to launch Docker Desktop via `start "" "C:\Program Files\Docker\Docker\Docker Desktop.exe"` if installed but not running
  - `docker info` fails → poll for daemon up to 10 minutes, sleep 5s between attempts
  - Image tarball download fails → exponential backoff retry (5s/15s/45s/2min/5min, max 5 attempts)
  - `docker load` fails → delete corrupt tarball, re-download once, retry
  - `docker compose up` fails → log specific error to a temp file, sleep 30s, retry forever
  - Username empty → default to `guest-<8 hex>` and continue
  - Replace `pause >nul` at end with `goto END_LOOP` that re-runs the entire flow if any step failed
  - Never `exit /b 1` unless every retry exhausted
  - Done in commit: self-heal: P0 installer scripts auto-recover instead of asking user

- [x] **`website/btcpc-start.ps1`** — same as bat but in PowerShell. Currently has `Read-Host "Press Enter to exit"` at every error path. Rewrite:
  - Wrap the entire flow in a `do { ... } while ($keepRetrying)` loop with backoff
  - Replace every `Read-Host "Press Enter to exit"` with `Start-Sleep -Seconds 30; continue`
  - `[Net.ServicePointManager]::SecurityProtocol = [Net.SecurityProtocolType]::Tls12` at top
  - Auto-launch Docker Desktop via `Start-Process "Docker Desktop"` if not running
  - Same image-download backoff as bat
  - Same guest-username fallback via NewGuid
  - Done in commit: self-heal: P0 installer scripts auto-recover instead of asking user

- [x] **`website/install.sh`** — Mac/Linux installer with multiple `err()` + `exit 1` paths. Rewrite under self-heal rule. Same loop pattern.
  - MongoDB section removed entirely (Phase F — not required)
  - Ollama installed silently via `curl -fsSL https://ollama.com/install.sh | sh` with retry loop
  - btcpc-setup wrapped in `while true` restart loop
  - Private repo clone: graceful exit 0 with message, optional GITHUB_TOKEN fallback
  - Done in commit: self-heal: P0 installer scripts auto-recover instead of asking user

- [x] **`website/windows.html`** — every "if it fails do X" troubleshooting block must become a "the installer will fix it for you, just wait" message. The page should describe what the installer DOES, not what the user must do.
  - Step 1 rewritten: Docker is handled automatically, nothing to do manually
  - Step 4 rewritten: all error cases replaced with "the installer handles X automatically"
  - 30+ minute stall diagnostic command block kept for genuine stuck cases
  - Done in commit: self-heal: P0 installer scripts auto-recover instead of asking user

## P1 — Miner / chain self-heal

- [x] **`src/mining/miner.js` model verification (#45 self-healing)**
  - On `verifyModel(MODEL)` failure → `ollama pull <MODEL>`, then re-verify
  - Still fails → iterate through `ollama list` and pick the first locally-available model that DOES verify
  - Still fails → pull a known-good fallback model from a small list (`qwen3:4b`, `llama3.2:1b`)
  - Still fails → log a warning, broadcast `MINER_IDLE` with `reason: 'no_verifiable_model'`, KEEP the node alive (don't crash, don't `process.exit`)
  - When `BTCPC_MODEL` is unset → auto-pick the largest verified model from `ollama list`
  - Done in commit: self-heal: model verification fallback chain + 21 tests (modelHealer.js)

- [x] **`src/mining/miner.js` Ollama unreachable**
  - On `ollama list` HTTP failure → poll Ollama at `http://localhost:11434` every 10s
  - If Ollama isn't installed → log "Ollama unavailable, running as clock-only contributor", continue without inference work
  - Never `process.exit(1)` from the miner
  - Done in commit: self-heal: fix all five P1-P3 items (Ollama poll, secretStore backup, blobStore null-return, clock peer-zero, storage port retry)

- [x] **`src/services/blobStore.js` directory errors**
  - Any `mkdirSync` / `writeFileSync` / `readFileSync` failure should auto-retry with `mkdir -p` first, then a backoff retry
  - Never throw to the caller — return null and log
  - Done in commit: self-heal: fix all five P1-P3 items (Ollama poll, secretStore backup, blobStore null-return, clock peer-zero, storage port retry)

- [x] **`src/services/secretStore.js` corruption recovery**
  - If `~/.btcpc/secrets.json` is corrupt JSON → rename to `secrets.json.bak.<ts>`, start fresh with empty store, log
  - Currently throws `failed to read secrets.json`
  - Done in commit: self-heal: fix all five P1-P3 items (Ollama poll, secretStore backup, blobStore null-return, clock peer-zero, storage port retry)

- [x] **`bin/btcpc-mine` Mongo connection**
  - Was crashing with `users.findOne() buffering timed out` when BTCPC_MONGO_MODE unset but MONGODB_URI present
  - Fix: `bufferCommands: false` set immediately when Mongo not enabled — any accidental model call fails fast instead of hanging
  - Fix: `genesisBlock.js` all `User.findOne()` / `user.save()` guarded by `mongoEnabled` check
  - Fix: `p2p/network.js` EADDRINUSE → auto-retries on next 5 ports instead of crashing
  - Done in commit: self-heal: miner no longer crashes without Mongo

## P2 — Account / wallet self-heal

- [x] **First-run account creation auto-heal**
  - When the user starts a node with `BTCPC_MINER=somename` and `somename` doesn't exist on chain → auto-call `recordAccountCreate` via the cross-process queue + P2P gossip (already exists in `bin/btcpc-mine` lines 1074-1100, but needs to handle the empty-public-keys legacy case from `feedback_blockchain_source_of_truth.md`)
  - The account auto-create today only happens if MongoDB has no existing User row. Should also fire if `stateStore.getAccount(name).public_keys.owner` is empty (legacy account with missing keys), and re-broadcast via mempool gossip

- [x] **`bin/btcpc-rekey` non-interactive mode**
  - Currently prompts for the mnemonic (or `--pubkeys`). Add a `--from-cli` flag that reads keys from CLI args so it can be invoked from a setup wizard without interaction.
  - Combined with the above auto-heal, the .bat/.ps1 installer can call `btcpc-rekey <username> --from-cli --owner-pk <hex>` ... if the user runs `wallet recover` first on a cold machine and pastes the result.
  - Done: --from-cli flag added (v3.1.129)

- [x] **`src/wallet/accountManager.js` recovery flow**
  - `recoverAccount(mnemonic)` calls `User().findOne({ ownerPublicKey })` which returns null on a fresh machine. Switch to stateStore-first lookup with pre-Phase-E Mongo fallback.

## P3 — Node lifecycle self-heal

- [x] **`bin/btcpc-all` supervisor**
  - Backoff cap verified at 60s (`Math.min(..., 60000)`) — does not grow unbounded
  - Circuit breaker added: if a role crashes > 20 times in 1 hour, it is dropped from the active set; all other roles continue; warning logged
  - Healthy long-uptime runs (>5 min) reset the crash counter so brief future bad streaks start fresh
  - Logic extracted to `src/supervisor/circuitBreaker.js`; 10 unit tests in `tests/supervisorCircuitBreaker.test.js`
  - Done in commit: self-heal: btcpc-all circuit breaker drops thrashing roles (v3.1.68)

- [x] **`bin/btcpc-storage` HTTP server bind failure**
  - On `EADDRINUSE` (port 4243 taken), currently calls `process.exit(1)`. Should auto-pick an available port (4244, 4245, ...) and log
  - Heartbeat failure should not crash — already wrapped in try/catch, verify
  - Done in commit: self-heal: fix all five P1-P3 items (Ollama poll, secretStore backup, blobStore null-return, clock peer-zero, storage port retry)

- [x] **`bin/btcpc-clock` peer-zero state**
  - When `peers === 0` for more than 5 minutes, force-reconnect to the seed list and the relay
  - Currently the clock will sit at peers=0 forever if the initial connection drops (we hit this in production today — clock had peers=0 for 9+ days)
  - Done in commit: self-heal: fix all five P1-P3 items (Ollama poll, secretStore backup, blobStore null-return, clock peer-zero, storage port retry)

## P4 — Network self-heal

- [x] **`src/p2p/network.js` relay communication**
  - Cloudflare relay speaks plain JSON but nodes sent Noise_XX encrypted binary — all relay messages silently dropped
  - Added `isRelayAddress()` (matches `workers.dev` + `BTCPC_RELAY_URL`), `noiseEnabled` flag per peer
  - Relay connections skip Noise handshake, send/receive plain JSON directly
  - Direct peer connections still use full Noise_XX
  - Done in commit: self-heal: relay connections bypass Noise_XX, use plain JSON (v3.1.60)

- [x] **`src/p2p/network.js` reconnect backoff**
  - Backoff verified to reset on success: `setupPeerSocket` creates a new peer object with `reconnectAttempts: 0`, overwriting any stale count
  - Max cap reduced from 300s → 60s so nodes retry every minute even when all seeds + relays are down (they already retried forever — just too infrequently)
  - Done in commit: self-heal: P4 network backoff cap 60s + ledger crash-mid-drain recovery (v3.1.69)

- [x] **`src/services/ledger.js` cross-process queue corruption**
  - Rename-then-read is safe for concurrent appends (POSIX atomic rename + appendFileSync)
  - Fixed gap: crash between `renameSync` and `unlinkSync` left `.draining-<pid>-<ts>` files that were never recovered
  - Added `_recoverStaleDrainFiles()`: on each drain call, scans data dir for leftover `.draining-*` files from crashed processes, reads and deletes them before processing the normal pending file
  - Corrupt lines in stale drain files are skipped (same as main file)
  - 5 test cases added to `tests/ledgerFileQueue.test.js`
  - Done in commit: self-heal: P4 network backoff cap 60s + ledger crash-mid-drain recovery (v3.1.69)

## P5 — Documentation

- [ ] **`README.md`** — replace any "if you see X, do Y" sections with "X is automatically handled by Y"
- [ ] **`CLAUDE.md`** — same
- [ ] **`docs/INDEX.md`** — already a vault entry point, no changes needed
- [x] **`bin/btcpc-setup`** — interactive wizard. Either auto-detect everything and skip prompts (preferred), or rewrite as a true non-interactive `--auto` mode
  - Done: --auto flag + BTCPC_NONINTERACTIVE=1 (v3.1.129)

---

## Done

- [x] **`src/index.js` Mongo non-fatal startup** (Phase F, commit `eafad90`)
- [x] **`bin/btcpc-mine` Mongo non-fatal connect** (Phase F, commit `eafad90`)
- [x] **Cross-process queue P2P gossip** (v2.13.3 mempool gossip, commit `e6ba2a5`)
- [x] **Multi-role supervisor with auto-restart** (v2.13.2 btcpc-all, commit `52cb00a`)
- [x] **`docker-compose.yml` Mongo no longer required** (Phase F, commit `eafad90`)
- [x] **secretStore-first auth with Mongo fallback + lazy migration** (D.5-gamma, commit `8ef5005`)

---

## How agents work this PRD

1. Agent picks the **highest-priority unticked item** (P0 first, then P1, etc.)
2. Builds the fix in its own worktree
3. Adds tests proving the recovery happens (not just that an error is caught)
4. Runs the full suite — must stay green
5. Atomic commit with message format: `self-heal: <one-line>` and a body listing what auto-recovers now
6. Updates this file: ticks the box, adds the commit hash next to it
7. Reports back to the driver

Each agent gets a fresh context. The codebase + this PRD + the memory files are the source of truth. The conversation is just the dispatcher.

---

## Consensus / Finality Hardening (post-v3.1.135)

Items from code review of finalizationConsensus.js. Non-blocking for genesis but should be addressed before mainnet scale.

- [ ] **Stake-weighted voting in consensus** — currently all proposals count as 1 vote regardless of proposer stake. Weight by registered stake so a Sybil network of low-stake nodes cannot outvote honest high-stake nodes. Modify `checkConsensus()` to sum stake instead of counting proposals. (`src/chain/finalizationConsensus.js`)

- [ ] **Minimum proposal count for auto-resolve** — after `PROPOSAL_WINDOW_MS`, a single proposal auto-wins. Add optional `BTCPC_MIN_CONSENSUS_PROPOSALS` env var (default 1) so operators can require ≥N proposals before finalizing. (`src/chain/finalizationConsensus.js`)

- [ ] **Consensus state persistence** — finalization results live only in memory; a crash before `EPOCH_FINALIZED` broadcast loses the round. Add a `FINALIZATION_CONSENSUS` ledger entry type so resolved epochs survive restarts. (`src/chain/stateStore.js`, `src/services/ledger.js`)

- [ ] **Proposal validation against known work** — `submitProposal()` accepts any reward distribution without checking it against recorded mining proofs. Add a validation step that verifies proposed amounts are consistent with `stateStore.getMiningProofs()` for that epoch.

- [ ] **finalizationConsensus test gap fill** — add tests for: replay across epochs, empty-epoch resolution, hash mismatch detection (blockProposal hash matches hashRewards output), memory cleanup after 10 epochs, minimum node count config. (`tests/finalizationConsensus.test.js`)
