# Miner Update Guide

When the HONE network code is updated, miners need to pull the latest and restart to stay compatible with the P2P inference protocol.

## Quick Update

```bash
cd ~/repos/hone          # or wherever your hone repo is
git pull origin main
npm install                # if dependencies changed

# Restart the miner
pkill -f 'node bin/hone-mine'
sleep 2
node bin/hone-mine &
```

## What Changed (Latest)

The inference pipeline now uses P2P routing:

1. **Requester nodes** (no GPU) send `INFERENCE_REQUEST` via P2P
2. **Miners** auto-claim requests for models they have
3. **Miners** receive `INFERENCE_PAYLOAD` with the prompt
4. **Miners** run inference locally via Ollama and broadcast `INFERENCE_RESULT` back

Your miner must be running the latest `src/inference/handler.js` to properly receive and respond to P2P inference requests. If you're running old code, requests will be claimed but never fulfilled.

## Verify Your Miner

After updating, check that:

```bash
# Miner is running
ps aux | grep hone-mine

# P2P is connected (check logs)
tail -20 /tmp/hone-mine.log | grep 'Connected\|Handshake'

# Ollama is serving models
curl http://localhost:11434/api/tags
```

## Auto-Update (Optional)

```bash
# Add to crontab for automatic pulls (checks every hour)
crontab -e
# Add: 0 * * * * cd ~/repos/hone && git pull origin main && npm install 2>/dev/null
```

## Environment

Make sure your `.env` has:

```
OLLAMA_URL=http://localhost:11434      # Ollama running locally on this machine
P2P_PORT=6942                          # P2P server port
HONE_MODEL=qwen3.5:27b               # Default mining model
HONE_WORK_PER_EPOCH=3                 # Work items per epoch
```

The miner automatically:
- Syncs available models to the network registry
- Detects inference engine type (Ollama, vLLM, etc.)
- Claims inference requests for models it has
- Broadcasts results back via P2P
