#!/bin/bash
# run one lab node: natrun.sh <ns> <name> [extra env...]   (own machine-id per node)
NS=$1; NAME=$2; shift 2
D=/tmp/natlab/$NAME; mkdir -p $D
[ -f $D/../$NAME.machine-id ] || head -c16 /dev/urandom | xxd -p > $D/../$NAME.machine-id
LOGL=${LOGL:-hone_node=info,libp2p_dcutr=debug}
sudo -n ip netns exec lab-$NS sh -c "mount --bind $D/../$NAME.machine-id /etc/machine-id && exec sudo -n -u ubuntclaw env -i HOME=$D TMPDIR=$D PATH=/usr/bin:/bin \
  HONE_ACCOUNT=$NAME HONE_NODE_ID=$NAME HONE_DATA_DIR=$D HONE_CHAIN_ID=hone-testnet \
  HONE_GENESIS_TIMESTAMP=$(cat /tmp/natlab/genesis) HONE_API_PORT=4242 HONE_API_BIND=0.0.0.0 HONE_P2P_PORT=6942 HONE_WS_PORT=4943 \
  HONE_CLOCK=true HONE_SIM=false HONE_STORAGE=false HONE_WORKER=false HONE_MINER=false HONE_SERVICE=false HONE_SENSOR=false \
  HONE_PEER_APIS= HONE_NO_DEFAULT_BOOTSTRAP=1 HONE_UPNP=0 HONE_PROFILE=light HONE_LOG_LEVEL=$LOGL $* \
  setsid /tmp/natlab/bin/hone-node" >> $D/run.log 2>&1 < /dev/null &
echo "started $NAME in lab-$NS"