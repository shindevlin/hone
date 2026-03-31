# Install: Inference Miner Node

Mine BTCPC by providing GPU compute to the network.

## Requirements

- **GPU**: NVIDIA with 8GB+ VRAM (any CUDA-capable card)
- **Node.js**: 20+
- **MongoDB**: 7+ (Docker or native)
- **Ollama**: With at least one supported model
- **OS**: Linux or Windows with WSL2

## Step 1: Install Dependencies

```bash
# Node.js (if not installed)
curl -fsSL https://deb.nodesource.com/setup_20.x | sudo -E bash -
sudo apt-get install -y nodejs

# Ollama
curl -fsSL https://ollama.com/install.sh | sh
ollama pull qwen3.5:27b    # or any supported model
```

## Step 2: MongoDB

```bash
docker run -d \
  --name btcpc-mongo \
  --restart on-failure:5 \
  -p 27017:27017 \
  -e MONGO_INITDB_ROOT_USERNAME=root \
  -e MONGO_INITDB_ROOT_PASSWORD=example \
  -v btcpc_mongo_data:/data/db \
  mongo:7
```

## Step 3: Clone and Install

```bash
cd ~/repos  # or your preferred directory
git clone https://github.com/shindevlin/btcpc.git
cd btcpc
npm install
```

## Step 4: Configure

```bash
cp .env.example .env
```

Edit `.env`:
```
MONGODB_URI=mongodb://root:example@localhost:27017/btcpc?authSource=admin
OLLAMA_URL=http://localhost:11434
BTCPC_MODEL=qwen3.5:27b
BTCPC_WORK_PER_EPOCH=3
P2P_PORT=6942
JWT_SECRET=<generate with: openssl rand -hex 32>
```

## Step 5: Build Silicon Fingerprint (Optional but Recommended)

If you have CUDA toolkit installed:
```bash
cd src/silicon && make && cd ../..
```

This creates the GPU fingerprint binary for Proof of Silicon encryption.

## Step 6: Start Mining

```bash
node bin/btcpc-mine
```

Expected output:
```
[BTCPC] BTCPC Mining Daemon Starting
[BTCPC] MongoDB connected
[BTCPC] Silicon ID: a3f8e2c1d7b3...
[BTCPC] GPU: NVIDIA GeForce RTX 3090 (24576 MB)
[BTCPC] P2P network started on port 6942
[BTCPC] Epoch 1 mining started
```

## Step 7: Make Permanent (systemd)

Create `/etc/systemd/system/btcpc-mine.service`:
```ini
[Unit]
Description=BTCPC Mining Daemon
After=network.target docker.service

[Service]
Type=simple
User=YOUR_USERNAME
WorkingDirectory=/home/YOUR_USERNAME/repos/btcpc
ExecStart=/usr/bin/node bin/btcpc-mine
Restart=on-failure
RestartSec=10
EnvironmentFile=/home/YOUR_USERNAME/repos/btcpc/.env

[Install]
WantedBy=multi-user.target
```

```bash
sudo systemctl daemon-reload
sudo systemctl enable btcpc-mine
sudo systemctl start btcpc-mine
```

## Step 8: Connect to Peers

Add seed peers to `.env`:
```
BTCPC_SEED_PEERS=ws://100.90.146.17:6942
```

Restart the miner to connect.

## Verify

```bash
# Check mining logs
journalctl -u btcpc-mine -f

# Check via CLI
node bin/btcpc-cli status

# Check via Telegram
# Message @btcpcbot: /link <username> then /mining
```

## Choosing a Model

Larger models earn more per epoch but require more VRAM:

| Model | VRAM Needed | Weight | Reward Multiplier |
|-------|------------|--------|-------------------|
| qwen3.5:9b | 8GB | 2.0x | Standard |
| qwen3.5:27b | 20GB | 4.0x | 2x more |
| llama3.1:70b | 48GB | 16.0x | 8x more |

Pick the largest model your GPU can run.
