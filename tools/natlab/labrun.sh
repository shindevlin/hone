#!/bin/bash
# labrun.sh [symmetricB] [quic]: fresh lab, seed + A + B, wait 75 s, summarize
MODE=$1; QUIC=$2
for p in $(pgrep -f /tmp/natlab/bin/hone-node); do kill $p 2>/dev/null; done; sleep 2
bash /tmp/natlab.sh up $MODE
cp ~/hone-v21-target/release/hone-node /tmp/natlab/bin/hone-node
rm -rf /tmp/natlab/seed /tmp/natlab/laba /tmp/natlab/labb
echo $(( ($(date +%s) - 60) * 1000 )) > /tmp/natlab/genesis
export LOGL=hone_node=info,libp2p_dcutr=debug
bash /tmp/natrun.sh seed seed HONE_RELAY_SERVER=1 HONE_EXTERNAL_ADDRS=/ip4/198.51.100.10/tcp/6942,/ip4/198.51.100.10/udp/6942/quic-v1
sleep 4
SID=$(sed 's/\x1b\[[0-9;]*m//g' /tmp/natlab/seed/run.log | grep -ao 'P2P running peer_id=[A-Za-z0-9]*' | cut -d= -f2)
SB=/ip4/198.51.100.10/tcp/6942/p2p/$SID
[ "$QUIC" = quic ] && SB=/ip4/198.51.100.10/udp/6942/quic-v1/p2p/$SID,$SB
bash /tmp/natrun.sh hostA laba HONE_BOOTSTRAP_PEERS=$SB
bash /tmp/natrun.sh hostB labb HONE_BOOTSTRAP_PEERS=$SB
sleep 75
for n in seed laba labb; do
  L=/tmp/natlab/$n/run.log
  echo "== $n: seals-from-peers $(grep -ac 'Received epoch seal' $L), relayed-conns $(grep -ac '(relayed)' $L), dcutr-ok $(grep -ac 'hole punch ok' $L), dcutr-fail $(grep -ac 'hole punch to .* failed' $L), circuits $(grep -ac 'relay server: circuit' $L)"
  sed 's/\x1b\[[0-9;]*m//g' $L | grep -a 'Peer connected\|dcutr:\|peer fill\|reservation accepted' | sed 's/12D3KooW[A-Za-z0-9]*\([A-Za-z0-9]\{4\}\)/…\1/g' | cut -c12-200 | head -8
done
echo "A sees B seals: $(sed 's/\x1b\[[0-9;]*m//g' /tmp/natlab/laba/run.log | grep -a 'Received epoch seal' | grep -c $(sed 's/\x1b\[[0-9;]*m//g' /tmp/natlab/labb/run.log | grep -ao 'P2P running peer_id=[A-Za-z0-9]*' | cut -d= -f2))"