## HONE Re-Genesis Runbook (current)

Supersedes the July-4 dating in `GENESIS_LAUNCH_RUNBOOK.md`. Same gate, same tooling,
current paths. **[YOU]** = human-only (touches keys). **[ME]** = Claude can do it.

### State (verified 2026-07-27)
- Vault: `C:\Users\oshha\Documents\Hone\HONE Wallets Vault.rar` — 11 keystores, each an
  **encrypted mnemonic** (`Keystore::seal(account, mnemonic, password)`). Recoverable with the password.
- `rust/hone-node/genesis.json`: 11 accounts + 3 system funds, zero-balance fair start.
  All 11 posting pubkeys already match the vault's public wallets (cross-checked).
- Binaries built on Beastly:
  - `hone` CLI: `rust/hone-cli/target/release/hone`
  - node: `rust/target/release/hone-node`
- Only stale item: `genesis_timestamp` (still the past July-4 value `1783191600000`).

### Step 0 — pick the launch time  ← the one decision
Choose a UTC instant a few hours out (so all nodes reach block 0 together).
Give it to Claude as epoch-ms. **[ME]** then sets it in all three gated files
(`rust/hone-node/src/config.rs` default, `rust/hone-node/genesis.json`,
`docs/CHAIN_CONSTANTS.md`) — CI `check-constants.yml` requires they match.

### Step 1 — extract the vault  **[YOU]**
Extract `HONE Wallets Vault.rar` (its password) to a Linux-side dir, e.g.:
```
mkdir -p ~/hone-genesis && cd ~/hone-genesis
# extract the .rar here so you have ~/hone-genesis/wallets/<account>.keystore.json
```
Keep it off `/mnt/*` (Linux ext4 avoids the SQLite/DrvFs issues).

### Step 2 — ★ THE GATE: verify-vault ★  **[YOU]** (needs the keystore password)
```
HONE=/mnt/x/hone/rust/hone-cli/target/release/hone
$HONE wallet verify-vault \
  --vault ~/hone-genesis/wallets \
  --genesis /mnt/x/hone/rust/hone-node/genesis.json \
  --require-accounts /mnt/x/hone/rust/hone-node/genesis-required-accounts.txt
```
- **Exit 0 / "Safe to launch"** → every required account has a recoverable keystore whose
  key matches genesis. Proceed.
- **Exit 2 / FAIL** → stop. Tell Claude which account/reason; **[ME]** fixes genesis.json
  (e.g. regenerate a pubkey from the vault) and you re-run. Do NOT launch on a red gate.

### Step 3 — back up the vault + password  **[YOU]**
- Copy `HONE Wallets Vault.rar` to a second physical location (USB / second machine).
- Save the vault password in a second safe place — it is now the single factor.
- (Optional, strong) decrypt and write each mnemonic on paper: offline layer-2 backup.

### Step 4 — fresh-genesis smoke test + record block-0 hash  **[ME or YOU]**
```
NODE=/mnt/x/hone/rust/target/release/hone-node
HONE_DATA_DIR=/tmp/regen-smoke HONE_CHAIN_ID=hone \
  HONE_GENESIS_FILE=/mnt/x/hone/rust/hone-node/genesis.json \
  HONE_GENESIS_TIMESTAMP=<new-ts> HONE_API_PORT=4299 HONE_P2P_PORT=6999 \
  $NODE &
sleep 8
curl -s http://localhost:4299/api/block/0 | grep -o '"hash":"[^"]*"'   # RECORD THIS
kill %1
```
Every genesis node must reproduce this exact block-0 hash. (Pre-genesis the node reports
`epoch 0` — that is correct, not a failure.)

### Step 5 — launch the primary (Beastly seed) at T-zero  **[YOU starts signing node]**
Fresh `HONE_DATA_DIR` (node refuses to re-init over an existing block 0). Set chain_id
`hone`, the new timestamp, and the account's posting PRIVATE key from its keystore. The
node waits at epoch 0 until the timestamp, then seals. **[ME]** has already prepped the
Beastly networking (port-proxy + firewall + announce addr) so peers can reach it.

### Step 6 — mesh joins (all **[ME]**, per the earlier plan)
Grouchly (Tailscale) → Nebra (Grouchly SSH, stop-old-service-first) → Android (last).
Each starts with the same genesis.json / timestamp / chain_id and bootstraps to Beastly.
Confirm every node's `/api/block/0` hash equals the recorded one, then watch
`peer_count` rise and `truth_bearing` flip true across the mesh.

### The one rule (unchanged)
> No account is signable unless its keystore exists and its key matches genesis.
> `verify-vault` green is the only signal that matters. A wrong/missing key at genesis is
> permanent once the chain runs — delay an hour and fix the vault rather than relaunch a sixth time.
