#!/usr/bin/env bash
# HONE Node — Linux / WSL launcher
# Usage: ./start.sh
# Requires: Docker (with Compose v2) + Ollama installed on the host.
# For hosts without Ollama: docker compose --profile bundled-ollama up -d

set -euo pipefail

export OLLAMA_HOST=0.0.0.0

# Restart Ollama with the new binding if it is already running.
if pgrep -x ollama >/dev/null 2>&1; then
    echo "Restarting Ollama with OLLAMA_HOST=0.0.0.0 ..."
    pkill -x ollama
    sleep 2
fi

echo "Starting Ollama..."
ollama serve &>/dev/null &

echo "Waiting for Ollama to start..."
for i in $(seq 1 10); do
    if curl -sf http://localhost:11434 >/dev/null 2>&1; then
        break
    fi
    sleep 1
done

echo "Starting HONE node..."
docker compose up -d

echo ""
echo "Done. Check logs with: docker compose logs -f hone-node"
