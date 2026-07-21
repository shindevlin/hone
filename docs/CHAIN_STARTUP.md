# HONE Chain Startup Guide

## For the Epoch Authority (shindevlin)

Shindevlin starts the chain. All other miners follow.

### 1. Drop existing database (FRESH START ONLY)

```bash
# MongoDB shell
mongosh mongodb://localhost:27017/hone --eval "db.dropDatabase()"

# Or via mongoose
node -e "require('mongoose').connect('mongodb://localhost:27017/hone').then(c=>c.connection.db.dropDatabase().then(()=>{console.log('dropped');process.exit()}))"
```

### 2. Pull latest code

```bash
cd ~/repos/hone
git pull
node -e "console.log('v' + require('./package.json').version)"
```

### 3. Drop old indexes (if restarting, not fresh)

```bash
node -e "require('mongoose').connect('mongodb://localhost:27017/hone').then(async()=>{try{await require('mongoose').connection.db.collection('miningproofs').dropIndex('block_number_1');console.log('done')}catch(e){console.log(e.message)}process.exit()})"
```

### 4. Start the miner

```bash
set HONE_MODEL=qwen3.5:27b
set HONE_WORK_PER_EPOCH=3
set P2P_PORT=6942
set HONE_MAX_MODEL_STORAGE_GB=100
node bin/hone-mine
```

### What happens on startup:

1. Genesis block 0 created (if fresh DB)
2. Whitepaper inscribed on Genesis Dream #0
3. shindevlin user, wallet (7 chains), and node created
4. Reserved names registered
5. Mining begins — epoch authority broadcasts EPOCH_START every 5 min
6. Other miners connect, receive EPOCH_START, start mining

### Environment variables:

| Variable | Default | Description |
|----------|---------|-------------|
| HONE_MODEL | qwen3.5:27b | Model to mine with |
| HONE_WORK_PER_EPOCH | 3 | Synthetic work items per epoch |
| P2P_PORT | 6942 | P2P WebSocket port |
| OLLAMA_URL | http://localhost:11434 | Ollama endpoint |
| HONE_MAX_MODEL_STORAGE_GB | 50 | Disk budget for auto-pulled models |
| MONGODB_URI | mongodb://localhost:27017/hone | MongoDB connection |

---

## For Follower Miners (natoshisakamoto, etc.)

### 1. Pull latest code

```bash
cd ~/repos/hone
git pull
```

### 2. Start the miner

```bash
# Linux
HONE_MINER=natoshisakamoto OLLAMA_URL=http://localhost:11434 HONE_MODEL=qwen3:4b HONE_WORK_PER_EPOCH=1 P2P_PORT=6944 node bin/hone-mine

# Windows
set HONE_MINER=natoshisakamoto
set HONE_MODEL=qwen3:4b
set P2P_PORT=6944
node bin/hone-mine
```

### What happens on startup:

1. If miner account doesn't exist, it's auto-created (user + wallet + node)
2. Miner waits for EPOCH_START from authority
3. When EPOCH_START arrives, miner begins work for that epoch
4. Proof is broadcast via P2P when work completes
5. Authority finalizes epoch and broadcasts EPOCH_FINALIZED with rewards

### The miner does NOT:
- Create its own epochs
- Calculate epoch numbers from local clock
- Finalize epochs
- It follows the authority. Period.

---

## Chain Reset Procedure

Only possible while there is a single clock node (genesis phase).

1. Stop ALL miners on ALL machines
2. Drop database on ALL machines
3. Shindevlin starts first (creates genesis)
4. Other miners start after (follow authority)
5. Chain begins from block 0

**Once there are external clock nodes or external miners with real stake, the chain can never be reset.**
