#!/usr/bin/env bash
set -uo pipefail
cd /mnt/x/hone/rust || exit 3
rm -f /mnt/x/hone/_seed4.sh /mnt/x/hone/_verify4.ps1 2>/dev/null
NODE=./target/release/hone-node
[ -x "$NODE" ] || { echo "NO_NODE_BINARY at $NODE"; ls -la target/release/hone-node 2>/dev/null; exit 1; }
echo "node: $(ls -la $NODE | awk '{print $5, $6, $7, $8}')"
rm -rf /tmp/gv2-smoke
TS=$(( ($(date +%s) + 600) * 1000 ))
echo "smoke genesis_timestamp=$TS (placeholder, ~10 min out)"
HONE_DATA_DIR=/tmp/gv2-smoke HONE_CHAIN_ID=hone \
  HONE_GENESIS_FILE=$PWD/hone-node/genesis.json \
  HONE_GENESIS_TIMESTAMP=$TS HONE_API_PORT=4299 HONE_P2P_PORT=6999 \
  nohup $NODE > /tmp/gv2-smoke.log 2>&1 &
NPID=$!
echo "node pid=$NPID"
sleep 12
echo "== healthz =="
curl -s -m 5 http://127.0.0.1:4299/health 2>&1 | head -c 200; echo
echo "== block 0 (record this hash) =="
curl -s -m 5 http://127.0.0.1:4299/api/block/0 2>&1 | head -c 500; echo
echo "== accounts genesis knows =="
curl -s -m 5 http://127.0.0.1:4299/api/node/list 2>&1 | head -c 400; echo
echo "== last 12 lines of node log =="
tail -12 /tmp/gv2-smoke.log 2>/dev/null
kill $NPID 2>/dev/null
sleep 1
rm -rf /tmp/gv2-smoke /tmp/gv2-smoke.log
echo SMOKE_DONE
