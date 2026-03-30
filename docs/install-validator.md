# Install: Validator / Network State Node

Validates blocks, relays transactions, and maintains chain state. No GPU or mining required. Earns ~10% of what inference nodes earn through relay fees.

## Requirements

- **Node.js**: 20+
- **MongoDB**: 7+
- **50 Mbps+ internet** (block relay)
- **2 CPU cores, 8GB RAM, 50GB SSD** minimum

## Step 1: Clone and Install

```bash
git clone https://github.com/estejosh/btcpc.git
cd btcpc
npm install
```

## Step 2: Configure

```bash
cp .env.example .env
```

Edit `.env`:
```
MONGODB_URI=mongodb://root:example@localhost:27017/btcpc?authSource=admin
P2P_PORT=6942
BTCPC_SEED_PEERS=ws://100.90.146.17:6942,ws://100.122.145.60:6942
JWT_SECRET=<generate with: openssl rand -hex 32>
PORT=3000
```

## Step 3: Start MongoDB

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

## Step 4: Start P2P + API

```bash
npm start
```

The node will:
- Connect to seed peers on the P2P network
- Sync chain state from peers
- Relay blocks and transactions
- Serve API requests
- Validate incoming blocks

## Step 5: Run Block Explorer (Optional)

```bash
node src/explorer/server.js
```

Visit `http://localhost:4242` for the web-based block explorer.

## Step 6: Make Permanent

Create `/etc/systemd/system/btcpc-validator.service`:
```ini
[Unit]
Description=BTCPC Validator Node
After=network.target docker.service

[Service]
Type=simple
User=YOUR_USERNAME
WorkingDirectory=/home/YOUR_USERNAME/repos/btcpc
ExecStart=/usr/bin/node src/index.js
Restart=on-failure
RestartSec=10
EnvironmentFile=/home/YOUR_USERNAME/repos/btcpc/.env

[Install]
WantedBy=multi-user.target
```

```bash
sudo systemctl daemon-reload
sudo systemctl enable btcpc-validator
sudo systemctl start btcpc-validator
```

## What Validators Do

- **Relay blocks** between miners and the network
- **Validate** epoch commitments and state hashes
- **Serve** API requests for users
- **Maintain** chain state in MongoDB
- **Participate** in consensus by voting on epoch commitments

Validators don't run inference — they keep the network honest and available.
