#!/bin/bash
# NAT lab: wan(198.51.100.0/24) with seed + two NAT routers; hostA behind natA, hostB behind natB.
# usage: natlab.sh up [symmetricB] | down
set -e
S="sudo -n"
down() {
  for n in seed natA natB hostA hostB wan; do $S ip netns del lab-$n 2>/dev/null || true; done
  pkill -f '/tmp/natlab/bin/hone-node' 2>/dev/null || true
}
if [ "$1" = down ]; then down; echo down; exit 0; fi
down
for n in wan seed natA natB hostA hostB; do $S ip netns add lab-$n; $S ip -n lab-$n link set lo up; done
$S ip -n lab-wan link add br0 type bridge; $S ip -n lab-wan link set br0 up
link() { # ns ifname ip/cidr  (attached to wan bridge)
  $S ip link add v-$1 type veth peer name w-$1
  $S ip link set v-$1 netns lab-$1; $S ip link set w-$1 netns lab-wan
  $S ip -n lab-wan link set w-$1 master br0; $S ip -n lab-wan link set w-$1 up
  $S ip -n lab-$1 link set v-$1 name wan0; $S ip -n lab-$1 addr add $2 dev wan0; $S ip -n lab-$1 link set wan0 up
}
link seed 198.51.100.10/24
link natA 198.51.100.21/24
link natB 198.51.100.22/24
lan() { # host router lanprefix
  $S ip link add l-$1 type veth peer name r-$1
  $S ip link set l-$1 netns lab-$1; $S ip link set r-$1 netns lab-$2
  $S ip -n lab-$2 addr add $3.1/24 dev r-$1; $S ip -n lab-$2 link set r-$1 up
  $S ip -n lab-$1 link set l-$1 name eth0; $S ip -n lab-$1 addr add $3.2/24 dev eth0; $S ip -n lab-$1 link set eth0 up
  $S ip -n lab-$1 route add default via $3.1
  $S ip netns exec lab-$2 sysctl -qw net.ipv4.ip_forward=1
}
lan hostA natA 10.1.0
lan hostB natB 10.2.0
$S ip netns exec lab-natA iptables -t nat -A POSTROUTING -o wan0 -j MASQUERADE
if [ "$2" = symmetricB ]; then
  $S ip netns exec lab-natB iptables -t nat -A POSTROUTING -o wan0 -j MASQUERADE --random-fully
else
  $S ip netns exec lab-natB iptables -t nat -A POSTROUTING -o wan0 -j MASQUERADE
fi
for r in natA natB; do
  $S ip netns exec lab-$r iptables -A FORWARD -i wan0 -m conntrack --ctstate ESTABLISHED,RELATED -j ACCEPT
  $S ip netns exec lab-$r iptables -A FORWARD -i wan0 -j DROP
  # Router firewall drops unsolicited WAN packets to itself (like real CPE): the dropped
  # packet never gets a confirmed conntrack entry, so it cannot steal the NAT mapping.
  $S ip netns exec lab-$r iptables -A INPUT -i wan0 -m conntrack --ctstate NEW -p udp -j DROP
  $S ip netns exec lab-$r iptables -A INPUT -i wan0 -m conntrack --ctstate NEW -p tcp -j DROP
done
echo "lab up ($2)"
$S ip netns exec lab-hostA ping -c1 -W1 198.51.100.10 >/dev/null && echo "A->seed ok"
$S ip netns exec lab-hostA ping -c1 -W1 10.2.0.2 >/dev/null && echo "A->B LEAK" || echo "A->B blocked (good)"