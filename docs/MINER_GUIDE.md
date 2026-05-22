# BTCPC Node Operator Guide

This guide is for technically literate operators who want to run a BTCPC node
and earn rewards. It covers the Rust implementation only. The Node.js code in
`src/`, `bin/`, and `package.json` is a deprecated prototype — do not use it.

---

## What BTCPC Is

BTCPC is a sovereign proof-of-compute chain with a fixed supply of 42,000,000
BTCPC (each divisible into 100,000,000 dreams). Epochs seal every 30 seconds.
Rewards flow to nodes that do real, verifiable work each epoch: sealing the
clock, submitting hardware and sensor data, running inference jobs, relaying
traffic, and storing data. There is no proof-of-work puzzle. Earning requires
a running node, a registered account, and hardware capable of the role you
register for.

---

## Node Roles

A single binary supports multiple roles simultaneously. Enable each role with
its environment variable.

| Role | Env var | What it does | Earns |
|------|---------|--------------|-------|
| Clock | `BTCPC_CLOCK=true` | Participates in epoch consensus — broadcasts a signed seal each epoch, counts toward quorum | `ClockReward` per epoch sealed |
| Sensor | `BTCPC_SENSOR=true` | Submits `SensorDataCommit` each epoch (CPU temp, uptime, GNSS if present) | `SensorReward` per epoch |
| Worker | `BTCPC_WORKER=true` | Watches the chain for posted inference jobs, bids, calls Ollama when awarded, submits `InferenceJobComplete` | Fee per completed job |
| Storage | `BTCPC_STORAGE=true` | Emits `StorageHeartbeat` each epoch, proving bytes on disk | `StorageReward` per epoch |
| Mempool | `BTCPC_MEMPOOL=true` | Reports gossip entries relayed per epoch | `MempoolReward` per epoch |
| Miner (legacy) | `BTCPC_MINER=true` | Submits `Mine` entries each epoch via a direct Ollama call, bypassing the inference marketplace | `MineReward` per epoch (see note) |

> **Note on Miner vs Worker:** `BTCPC_MINER` is the original inference path.
> `BTCPC_WORKER` is the current path — it participates in the inference
> marketplace and earns per-job fees. New operators should use `BTCPC_WORKER`.

Clock registration requires a minimum stake. See the staking requirements
endpoint: `GET /api/staking/requirements`.

---

## Hardware Requirements

### Clock node (minimum viable node)

- Any Linux machine (x86\_64 or ARM)
- 1 GB RAM
- 10 GB free disk
- Stable network connection (the node must stay peered)
- No GPU required
- No Ollama required

### Sensor node

- Raspberry Pi 4 or equivalent ARM board (aarch64)
- 1 GB RAM minimum
- 8 GB disk
- Sensor input: at minimum, CPU temperature and uptime are read automatically.
  GNSS data is submitted automatically if `btcpc-gnss-capture` is running and
  a GNSS receiver is connected.

### Worker node (inference marketplace)

- Any machine with Ollama installed and at least one model pulled
- GPU with 8 GB+ VRAM recommended; CPU-only works but is significantly slower
  and may time out on larger models
- 20 GB+ disk for model storage
- `OLLAMA_URL` must point to the Ollama instance (default: `http://localhost:11434`)
- Default model: `qwen2.5:0.5b`. Set `BTCPC_MODEL` to use a different model.

---

## Installation

### Step 1: Download the binary

**x86\_64 Linux:**

```bash
curl -L https://github.com/shindevlin/btcpc/releases/latest/download/btcpc-node-x86_64-linux \
  -o btcpc-node
chmod +x btcpc-node
sudo mv btcpc-node /usr/local/bin/
```

**aarch64 Linux (Raspberry Pi 4+, Nebra Pi, other ARM boards):**

```bash
curl -L https://github.com/shindevlin/btcpc/releases/latest/download/btcpc-node-aarch64-linux \
  -o btcpc-node
chmod +x btcpc-node
sudo mv btcpc-node /usr/local/bin/
```

Verify the binary starts:

```bash
btcpc-node --version
```

### Step 2: Create a data directory

```bash
mkdir -p ~/.btcpc
```

The node defaults to `~/.btcpc` for chain state. Override with `BTCPC_DATA_DIR`.

### Step 3: Get a posting key

On first start, the node generates a fresh wallet and writes it to two files:

- `$BTCPC_DATA_DIR/wallet.key` — full key material as JSON
- `~/.btcpc/{account}.txt` — human-readable backup including all role keys

Run the node once to generate the wallet:

```bash
BTCPC_ACCOUNT=mynode btcpc-node
```

Wait for the line:

```
wallet: backed up to /home/user/.btcpc/mynode.txt
```

Then stop the node (Ctrl+C) and read the posting key from the backup:

```bash
cat ~/.btcpc/mynode.txt
```

Find the `posting` section:

```
  posting
    public   <hex public key>
    private  <hex private key — this is BTCPC_POSTING_KEY>
```

Copy the `private` hex value. This is your `BTCPC_POSTING_KEY`. It is the
ed25519 seed for signing clock seals. Keep it private.

To extract it without reading the full file:

```bash
cat ~/.btcpc/mynode.wallet.key | python3 -c \
  "import sys,json; k=json.load(sys.stdin); print(k['btcpc_private_key'])"
```

### Step 4: Write an environment file

Create `~/.btcpc/mynode.env`:

```bash
# Required
BTCPC_ACCOUNT=mynode
BTCPC_POSTING_KEY=<hex from step 3>
BTCPC_CHAIN_ID=btcpc-1

# Roles — enable what your hardware supports
BTCPC_CLOCK=true
BTCPC_SENSOR=true
# BTCPC_WORKER=true   # uncomment if Ollama is installed
# BTCPC_STORAGE=true  # uncomment to earn storage rewards

# Ports (defaults shown — change only if there is a conflict)
BTCPC_API_PORT=4242
BTCPC_P2P_PORT=6942

# Leave BTCPC_BOOTSTRAP_PEERS unset to use the default DNS seeds:
#   /dns4/bootstrap1.btcpc.net/tcp/6942
#   /dns4/bootstrap2.btcpc.net/tcp/6942
# Only set this if you need to override the bootstrap nodes.
# BTCPC_BOOTSTRAP_PEERS=/dns4/yourpeer.example.com/tcp/6942

# Worker only — required if BTCPC_WORKER=true
# OLLAMA_URL=http://localhost:11434
# BTCPC_MODEL=qwen2.5:0.5b
```

### Step 5: Run the node

**Test run (foreground):**

```bash
env $(cat ~/.btcpc/mynode.env | grep -v '^#' | xargs) btcpc-node
```

Expected startup output:

```
btcpc-node starting — account=mynode chain=btcpc-1 data="/home/user/.btcpc"
chain state ready — latest epoch=1234
roles — miner=false clock=true storage=true service=false sensor=true worker=false mempool=false
```

**Permanent service (systemd user service):**

Create `~/.config/systemd/user/btcpc-node.service`:

```ini
[Unit]
Description=BTCPC Node
After=network-online.target
Wants=network-online.target

[Service]
Type=simple
EnvironmentFile=%h/.btcpc/mynode.env
ExecStart=/usr/local/bin/btcpc-node
Restart=on-failure
RestartSec=10

[Install]
WantedBy=default.target
```

Enable and start:

```bash
systemctl --user daemon-reload
systemctl --user enable --now btcpc-node
systemctl --user status btcpc-node
```

View logs:

```bash
journalctl --user -u btcpc-node -f
```

---

## Environment Variable Reference

| Variable | Default | Description |
|----------|---------|-------------|
| `BTCPC_ACCOUNT` | `genesis` | Account name for this node. All rewards route here. |
| `BTCPC_POSTING_KEY` | (none) | Hex 32-byte ed25519 seed. Required for clock seals. Auto-generated on first start if absent. |
| `BTCPC_CHAIN_ID` | `btcpc-1` | `btcpc-1` = mainnet, `btcpc-satoshi` = testnet |
| `BTCPC_DATA_DIR` | `~/.btcpc` | RocksDB chain state directory |
| `BTCPC_API_PORT` | `4242` | HTTP API port |
| `BTCPC_P2P_PORT` | `6942` | libp2p listen port |
| `BTCPC_BOOTSTRAP_PEERS` | DNS seeds | Comma-separated multiaddrs. Leave unset to use `bootstrap1.btcpc.net` and `bootstrap2.btcpc.net`. If set, values must be valid multiaddrs starting with `/`. |
| `BTCPC_CLOCK` | `false` | `true` to participate in epoch consensus |
| `BTCPC_SENSOR` | auto | `true` to submit sensor data each epoch. Auto-enabled if GNSS is detected. |
| `BTCPC_WORKER` | auto | `true` to bid on inference jobs. Auto-enabled if Ollama is detected. |
| `BTCPC_STORAGE` | auto | `true` to emit storage heartbeats. Auto-enabled if disk >= 10 GB. |
| `BTCPC_MEMPOOL` | `false` | `true` to report gossip relay counts |
| `BTCPC_MINER` | `false` | `true` for legacy direct-inference mining (not recommended for new nodes) |
| `OLLAMA_URL` | `http://localhost:11434` | Ollama endpoint. Required when `BTCPC_WORKER=true`. |
| `BTCPC_MODEL` | `qwen2.5:0.5b` | Model hint for worker and verifier |
| `BTCPC_GENESIS_TIMESTAMP` | `1777633200000` | Unix ms genesis timestamp. Do not change. Must match on all nodes. |
| `BTCPC_LOG_LEVEL` | `btcpc_node=info` | Tracing filter. `btcpc_node=debug` for verbose output. |
| `BTCPC_AUTO_UPDATE` | `0` | Set to `1` to enable automatic binary updates |
| `BTCPC_UPDATE_URL` | (none) | URL to poll for binary updates. Requires `BTCPC_AUTO_UPDATE=1`. |
| `BTCPC_TOR` | (disabled) | `true` to activate Tor hidden service. Also: `BTCPC_TOR_CONTROL_PORT` (default 9051), `BTCPC_TOR_CONTROL_PASSWORD`. |
| `BTCPC_NOSTR` | (disabled) | `true` to propagate entries via Nostr relays. Also: `BTCPC_NOSTR_RELAYS` (comma-separated WSS URLs). |
| `BTCPC_SECRETS_PASSPHRASE` | hw fingerprint | AES-256-GCM passphrase for the local secrets store |
| `BTCPC_ALERT_WEBHOOK` | (none) | HTTP POST URL for health alert notifications |

---

## Testnet vs Mainnet

| | Mainnet | Testnet |
|-|---------|---------|
| Chain ID | `btcpc-1` | `btcpc-satoshi` |
| Genesis | 2026-05-01 00:00:00 IST | Separate genesis |
| Faucet | No | `POST /api/faucet/claim` |
| Status | Active | Active |

The node defaults to `btcpc-1` (mainnet). To join testnet, set:

```bash
BTCPC_CHAIN_ID=btcpc-satoshi
```

On testnet, the node runs a simulation daemon and the faucet endpoint is live.
Testnet rewards are separate from mainnet and have no supply significance.

To claim testnet funds:

```bash
curl -X POST http://localhost:4242/api/faucet/claim \
  -H "Content-Type: application/json" \
  -d '{"account": "mynode"}'
```

---

## Verifying Your Node Is Earning

### Check node identity and roles

```bash
curl -s http://localhost:4242/api/node/info | python3 -m json.tool
```

Expected response fields:

```json
{
  "account": "mynode",
  "chain_id": "btcpc-1",
  "epoch": 1234,
  "peer_count": 3,
  "is_clock": true,
  "is_worker": false,
  "is_miner": false,
  "is_sensor": true,
  "has_gpu": false,
  "disk_gb": 50,
  "model": "qwen2.5:0.5b",
  "version": "0.x.y",
  "hw_fingerprint": "abc123...",
  "hw_summary": "machine:a1b2c3d4"
}
```

What to look for:

- `peer_count` must be greater than 0. A node with zero peers rejects all
  submitted entries by design — see the Troubleshooting section.
- `is_clock`, `is_sensor`, `is_worker` reflect what the node actually has
  active. If a role shows `false` when you expected `true`, check the env var
  and logs.
- `is_sensor` is computed from chain state (whether a `SensorDataCommit` has
  appeared for this account in the last 20 epochs), not just the env var.

### Check your balance

```bash
curl -s http://localhost:4242/api/balance/mynode
```

### Check account history

```bash
curl -s http://localhost:4242/api/account/mynode/history
```

### Check clock registration

```bash
curl -s http://localhost:4242/api/clock/registered
```

Your node ID should appear in the list if clock registration succeeded.

### Check node health

```bash
curl -s http://localhost:4242/health
curl -s http://localhost:4242/api/node/health/detailed
```

---

## Troubleshooting

### 1. No peers (peer\_count: 0)

**Symptom:** `peer_count` is 0 in `/api/node/info`. All entry submissions
return `"error": "not connected to network — entry rejected to prevent local fork"`.

**Why this happens:** This is intentional. A node that applies entries while
disconnected from the network silently forks — its chain state diverges from
the canonical chain with no way to reconcile. The zero-peer rejection is a
chain integrity requirement, not a bug. Do not look for a way to bypass it.

**How to diagnose:**

```bash
# Check that P2P port is reachable
ss -tlnp | grep 6942

# Check logs for bootstrap connection attempts
journalctl --user -u btcpc-node --since "5 minutes ago" | grep -i "peer\|bootstrap\|connect"
```

**Fix:**

- Confirm port 6942 is open in your firewall
- Confirm `BTCPC_P2P_PORT=6942` is set (or the port you chose is accessible)
- If you set `BTCPC_BOOTSTRAP_PEERS`, confirm the values are valid multiaddrs
  starting with `/dns4/` or `/ip4/` — e.g. `/dns4/host.example.com/tcp/6942`
- Leave `BTCPC_BOOTSTRAP_PEERS` unset to fall back to the DNS seed nodes

---

### 2. is\_sensor shows false despite BTCPC\_SENSOR=true

**Symptom:** The node is running with `BTCPC_SENSOR=true` but
`/api/node/info` returns `"is_sensor": false`.

**Why this happens:** `is_sensor` is derived from chain state, not the env
var. It checks whether a `SensorDataCommit` from your account has been sealed
in the last 20 epochs. If the node just started, or if sensor commits are
failing to reach peers before epoch seal, the flag will be `false` even though
the role is enabled.

**How to diagnose:**

```bash
# Look for sensor commit log lines
journalctl --user -u btcpc-node -f | grep -i sensor

# Check that peer_count > 0 (sensor commits are dropped without peers)
curl -s http://localhost:4242/api/node/info | grep peer_count
```

**Fix:**

- Wait at least two full epochs (60 seconds) after the node connects to peers
- Confirm `peer_count` is greater than 0 before expecting sensor data to commit

---

### 3. Worker not picking up jobs

**Symptom:** `BTCPC_WORKER=true` is set and Ollama is running, but the node
is not bidding on or completing jobs.

**How to diagnose:**

```bash
# Confirm Ollama is reachable
curl -s http://localhost:11434/api/tags

# Check available models
curl -s http://localhost:4242/api/node/models

# Check logs for worker activity
journalctl --user -u btcpc-node -f | grep -i "worker\|bid\|job"

# Check posted jobs on the chain
curl -s http://localhost:4242/api/task/jobs
```

**Fix:**

- Confirm `OLLAMA_URL` points to the correct Ollama instance
- Pull at least one model: `ollama pull qwen2.5:0.5b`
- Jobs are only posted when clients call `/api/task/post`. On a new network,
  the job queue may simply be empty. Use `BTCPC_WORK_GENERATOR=true` on a
  test node to generate synthetic demand.
- Confirm `peer_count > 0` — bids are gossip entries and are rejected without peers

---

### 4. Clock not sealing (not appearing in /api/clock/registered)

**Symptom:** `BTCPC_CLOCK=true` is set but the node does not appear in the
registered clock list. No `ClockReward` entries are appearing in account history.

**How to diagnose:**

```bash
# Check registration status
curl -s http://localhost:4242/api/clock/registered

# Check logs for clock registration messages
journalctl --user -u btcpc-node --since "10 minutes ago" | grep -i "clock\|register\|stake"
```

**Likely causes and fixes:**

- **Insufficient stake.** Clock registration requires a minimum stake balance.
  Check the requirement: `curl -s http://localhost:4242/api/staking/requirements`
  and fund your account to meet the minimum before enabling `BTCPC_CLOCK`.
- **Missing or wrong posting key.** The clock auto-registration signs with
  `BTCPC_POSTING_KEY`. If the key is absent or does not match the key
  registered on-chain for your account, auto-registration fails. Check logs
  for `[clock] auto-register failed`. You can also register manually:
  `POST /api/clock/self-register`
- **Zero peers.** Clock seals are gossip messages. The node must have peers
  before seals count toward quorum.

---

### 5. Binary won't start (port already in use)

**Symptom:** The node exits immediately with:
```
btcpc-node: port 4242 is already in use — another instance may be running
```

Or:
```
btcpc-node: another instance is already running (lock: /home/user/.btcpc/node.lock)
```

**Fix:**

```bash
# Find what is using the port
ss -tlnp | grep 4242

# If a stale lock file remains after an unclean shutdown
rm ~/.btcpc/node.lock

# If you need two nodes on the same machine, use different ports and data dirs:
BTCPC_API_PORT=4243 BTCPC_P2P_PORT=6943 BTCPC_DATA_DIR=~/.btcpc-2 btcpc-node
```

---

## Update

The node binary is a single static file. To update:

```bash
# Stop the service
systemctl --user stop btcpc-node

# Download the new binary (same command as installation)
curl -L https://github.com/shindevlin/btcpc/releases/latest/download/btcpc-node-x86_64-linux \
  -o /tmp/btcpc-node-new
chmod +x /tmp/btcpc-node-new
sudo mv /tmp/btcpc-node-new /usr/local/bin/btcpc-node

# Start again
systemctl --user start btcpc-node
systemctl --user status btcpc-node
```

For automatic updates, set `BTCPC_AUTO_UPDATE=1` and `BTCPC_UPDATE_URL` in
your environment file. The node will poll for and apply binary updates without
manual intervention.

---

## Next Steps

- Check account history in the explorer: `http://localhost:4242/api/account/mynode/history`
- View the explorer status page: `http://localhost:4242/api/explorer/status`
- Read the chain constants reference: `docs/CHAIN_CONSTANTS.md`
- Read the consensus spec: `docs/CONSENSUS.md`
- Set up a sensor node with GNSS hardware: `docs/hardware/`
- Join the Telegram bot for chain notifications: see `docs/bots.md`
