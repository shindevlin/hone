# Running a public HONE seed / relay

A public seed is an ordinary hone-node with a public IP that other nodes dial first. It
helps NAT'd nodes (home PCs, Raspberry Pis, phones) find each other, and runs a circuit
relay v2 server so two NAT'd nodes can hole-punch (DCUtR) a direct connection. When the
hole punch fails, gossip still flows because the seed is itself a gossipsub peer.

A seed is NOT a clock. It holds no account keys and earns nothing.

## Requirements

- Public IPv4 (any small VPS: 1 vCPU / 1 GB is enough).
- Inbound TCP 6942, UDP 6942 (QUIC) and TCP 4943 (WebSocket) open.

## Install

1. Copy the same `hone-node` binary the network runs to `/opt/hone-node/hone-node` and
   record its sha256.
2. `useradd -r -m -d /var/lib/hone -s /usr/sbin/nologin hone`
3. Copy a recent state snapshot (`state/` from any node, taken while it is stopped) to
   `/var/lib/hone/node/state`, or start empty and let it catch up.
4. `cp deploy/hone-seed.env /var/lib/hone/node/env` and set `HONE_EXTERNAL_ADDRS` to your
   public IP / DNS name.
5. `cp deploy/hone-seed.service /etc/systemd/system/`, put the binary sha256 in its
   `ExecStartPre` line, `systemctl enable --now hone-seed`.
6. The first start creates `p2p-key` (the seed's peer id). Read it from the log:
   `grep 'P2P running' /var/lib/hone/node/run.log`.

## Publish it

Add a TXT record on `_dnsaddr.bootstrap.honemesh.net`, one line per address:

    dnsaddr=/dns4/<your host>/udp/6942/quic-v1/p2p/<peer id>
    dnsaddr=/dns4/<your host>/tcp/6942/p2p/<peer id>

Every node dials `/dnsaddr/bootstrap.honemesh.net` by default, so new seeds are picked up
with no release. Remove the lines to retire a seed.

## Node-side settings (all optional)

| env | default | meaning |
|---|---|---|
| `HONE_RELAY_SERVER` | off | `1` = serve circuit relay v2 (seeds only) |
| `HONE_EXTERNAL_ADDRS` | none | comma list of addresses this node is reachable on |
| `HONE_UPNP` | on | `0` = no UPnP port mapping on the home router |
| `HONE_NO_DEFAULT_BOOTSTRAP` | off | `1` = do not dial the public seeds (private test nets) |
| `HONE_AUTONAT_ALLOW_PRIVATE` | off | `1` = AutoNAT accepts private addresses (lab only) |