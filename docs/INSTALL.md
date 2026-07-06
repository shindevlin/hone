# BTCPC Mining Node — Installation Guide

Multi-platform setup for running a BTCPC Proof-of-Compute mining node.

## Prerequisites

| Dependency | Minimum Version | Purpose |
|------------|----------------|---------|
| **Node.js** | 18+ | Runtime |
| **npm** | 9+ | Package manager (ships with Node) |
| **MongoDB** | 6+ | Block / epoch / wallet storage |
| **Ollama** | 0.1.0+ | Local LLM inference (the "work" in Proof-of-Compute) |
| **Git** | 2.30+ | Clone the repo |

A GPU is **strongly recommended** — inference on CPU is possible but very slow.
The default model (`qwen3.5:27b`) needs ~18 GB VRAM at Q4_K_M quantization.
Smaller models (e.g. `deepseek-r1:8b`) work on 8 GB cards at lower reward weight.

---

## 1. Install System Dependencies

### Windows

```powershell
# Node.js — download the LTS installer from https://nodejs.org
# or via winget:
winget install OpenJS.NodeJS.LTS

# Git
winget install Git.Git

# MongoDB — run via Docker (recommended):
docker run -d --name btcpc-mongo -p 27017:27017 \
  -e MONGO_INITDB_ROOT_USERNAME=root \
  -e MONGO_INITDB_ROOT_PASSWORD=example \
  mongo:7

# Ollama — download from https://ollama.com/download/windows
# After install, pull the mining model:
ollama pull qwen3.5:27b
```

> **Note:** On Windows, use Git Bash, WSL2, or PowerShell for the commands below.

### Linux (Ubuntu / Debian)

```bash
# Node.js 20 LTS
curl -fsSL https://deb.nodesource.com/setup_20.x | sudo -E bash -
sudo apt-get install -y nodejs git

# MongoDB via Docker
sudo apt-get install -y docker.io
sudo docker run -d --name btcpc-mongo -p 27017:27017 \
  -e MONGO_INITDB_ROOT_USERNAME=root \
  -e MONGO_INITDB_ROOT_PASSWORD=example \
  mongo:7

# Ollama
curl -fsSL https://ollama.com/install.sh | sh
ollama pull qwen3.5:27b
```

### macOS

```bash
# Homebrew (if not installed)
/bin/bash -c "$(curl -fsSL https://raw.githubusercontent.com/Homebrew/install/HEAD/install.sh)"

# Node.js + Git
brew install node git

# MongoDB via Docker
brew install --cask docker   # Docker Desktop
docker run -d --name btcpc-mongo -p 27017:27017 \
  -e MONGO_INITDB_ROOT_USERNAME=root \
  -e MONGO_INITDB_ROOT_PASSWORD=example \
  mongo:7

# Ollama
brew install ollama
ollama pull qwen3.5:27b
```

---

## 2. Clone and Install

```bash
git clone https://github.com/shindevlin/btcpc.git
cd btcpc
npm install
```

---

## 3. Configure Environment

```bash
cp .env.example .env
```

Edit `.env` with the values for your setup. The mining-critical variables are:

```ini
# MongoDB connection (match your Docker credentials)
MONGODB_URI=mongodb://root:example@localhost:27017/btcpc?authSource=admin

# Ollama endpoint (default: localhost)
OLLAMA_URL=http://localhost:11434

# Model to mine with (must be pulled in Ollama)
HONE_MODEL=qwen3.5:27b

# Inference tasks per epoch (default: 3)
HONE_WORK_PER_EPOCH=3

# P2P port for node discovery
P2P_PORT=6942
```

---

## 4. Verify Services

Before starting the miner, confirm MongoDB and Ollama are reachable:

```bash
# MongoDB — should print the version
docker exec btcpc-mongo mongosh --quiet --eval "db.version()"

# Ollama — should list your pulled models
curl -s http://localhost:11434/api/tags | head -c 200
```

---

## 5. Start the Mining Daemon

```bash
node bin/btcpc-mine
```

You should see output like:

```
[BTCPC] MongoDB connected
[BTCPC] ================================================
[BTCPC]    BTCPC Mining Daemon Starting
[BTCPC] ================================================
[BTCPC] Ollama:     http://localhost:11434
[BTCPC] Model:      qwen3.5:27b
[BTCPC] Work/epoch: 3
[BTCPC] Epoch:      30s
[BTCPC] ================================================
[BTCPC] Genesis block already exists
[BTCPC] Epoch 1 mining started
```

The daemon runs continuously with 30-second epoch cycles. Press `Ctrl+C` to stop gracefully.

### Always-On Mode (Recommended)

The miner should run continuously and auto-restart on crash or reboot.
An auto-updater checks GitHub every 15 minutes and restarts the miner when new code is pulled.

**All platforms (pm2):**

```bash
# Install pm2 globally
npm install -g pm2

# Start miner + auto-updater together
cd /path/to/btcpc
pm2 start ecosystem.config.js

# Save for auto-restart on reboot
pm2 save
pm2 startup    # follow the printed instructions to install the boot hook
```

This starts two processes:

| pm2 name | What it does |
|----------|-------------|
| `btcpc-mine` | Mining daemon — runs forever, auto-restarts on crash |
| `btcpc-update` | Checks GitHub every 15 min — pulls new code and restarts the miner if updates are found |

**Useful pm2 commands:**

```bash
pm2 status            # see running processes
pm2 logs              # tail all logs
pm2 logs btcpc-mine   # tail miner logs only
pm2 monit             # live dashboard
pm2 stop all          # stop everything (manual override)
pm2 restart btcpc-mine  # restart miner only
```

**To stop mining:** `pm2 stop btcpc-mine` — the updater will not restart it unless you `pm2 start btcpc-mine` again.

**Linux alternative (systemd):**

```ini
# /etc/systemd/system/btcpc-miner.service
[Unit]
Description=BTCPC Mining Daemon
After=network.target docker.service

[Service]
Type=simple
User=your-user
WorkingDirectory=/home/your-user/btcpc
ExecStart=/usr/bin/node bin/btcpc-mine
Restart=on-failure
RestartSec=10
Environment=NODE_ENV=production

[Install]
WantedBy=multi-user.target
```

```bash
sudo systemctl enable --now btcpc-miner
sudo journalctl -u btcpc-miner -f   # watch logs
```

---

## 6. Other Commands

| Command | Description |
|---------|-------------|
| `npm start` | Start the API server |
| `npm run mine` | Start the mining daemon |
| `npm run update` | Check GitHub for updates and restart miner |
| `npm run cli` | Interactive CLI |
| `npm run explorer` | Block explorer web UI |
| `npm run p2p` | P2P network node |
| `npm test` | Run test suite |

---

## Troubleshooting

| Symptom | Fix |
|---------|-----|
| `MongoDB connection failed` | Ensure the `btcpc-mongo` container is running: `docker ps` |
| `Ollama unreachable after 5 attempts` | Check Ollama is running: `ollama list`. Ensure `OLLAMA_URL` in `.env` is correct. |
| `Genesis miner account not found` | The genesis block creates the `shindevlin` miner account automatically on first run. If the DB was wiped, drop the database and restart the daemon to re-create genesis. |
| Slow epoch times | A 27B model on CPU can take 5+ minutes per work item. Use a GPU or switch to a smaller model (`HONE_MODEL=deepseek-r1:8b`). |
| `Duplicate schema index` warning | Harmless Mongoose warning — does not affect operation. |

---

## Genesis

The genesis block and miner (`shindevlin`) are created automatically on the first run when the database is empty. **Do not modify `src/mining/genesisBlock.js`** — the genesis parameters are consensus-critical.
