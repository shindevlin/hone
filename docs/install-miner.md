# Install: Inference Miner Node

Mine HONE by providing GPU compute to the network.

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
  --name hone-mongo \
  --restart on-failure:5 \
  -p 27017:27017 \
  -e MONGO_INITDB_ROOT_USERNAME=root \
  -e MONGO_INITDB_ROOT_PASSWORD=example \
  -v hone_mongo_data:/data/db \
  mongo:7
```

## Step 3: Clone and Install

```bash
cd ~/repos  # or your preferred directory
git clone https://github.com/shindevlin/hone.git
cd hone
npm install
```

## Step 4: Configure

```bash
cp .env.example .env
```

Edit `.env`:
```
MONGODB_URI=mongodb://root:example@localhost:27017/hone?authSource=admin
OLLAMA_URL=http://localhost:11434
HONE_MODEL=qwen3.5:27b
HONE_WORK_PER_EPOCH=3
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
node bin/hone-mine
```

Expected output:
```
[HONE] HONE Mining Daemon Starting
[HONE] MongoDB connected
[HONE] Silicon ID: a3f8e2c1d7b3...
[HONE] GPU: NVIDIA GeForce RTX 3090 (24576 MB)
[HONE] P2P network started on port 6942
[HONE] Epoch 1 mining started
```

## Step 7: Make Permanent (systemd)

Create `/etc/systemd/system/hone-mine.service`:
```ini
[Unit]
Description=HONE Mining Daemon
After=network.target docker.service

[Service]
Type=simple
User=YOUR_USERNAME
WorkingDirectory=/home/YOUR_USERNAME/repos/hone
ExecStart=/usr/bin/node bin/hone-mine
Restart=on-failure
RestartSec=10
EnvironmentFile=/home/YOUR_USERNAME/repos/hone/.env

[Install]
WantedBy=multi-user.target
```

```bash
sudo systemctl daemon-reload
sudo systemctl enable hone-mine
sudo systemctl start hone-mine
```

## Step 8: Connect to Peers

Add seed peers to `.env`:
```
HONE_SEED_PEERS=ws://100.90.146.17:6942
```

Restart the miner to connect.

## Verify

```bash
# Check mining logs
journalctl -u hone-mine -f

# Check via CLI
node bin/hone-cli status

# Check via Telegram
# Message @honebot: /link <username> then /mining
```

## Choosing a Model

Larger models earn more per epoch but require more VRAM:

| Model | VRAM Needed | Weight | Reward Multiplier |
|-------|------------|--------|-------------------|
| qwen3.5:9b | 8GB | 2.0x | Standard |
| qwen3.5:27b | 20GB | 4.0x | 2x more |
| llama3.1:70b | 48GB | 16.0x | 8x more |

Pick the largest model your GPU can run.
