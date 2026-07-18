# Routine: Cross-Node Fork / Convergence Check (`fork-check.mjs`)

_Added 2026-07-05. Script: [`scripts/fork-check.mjs`](../scripts/fork-check.mjs).
Read-only, zero-dependency (Node 18+ global fetch)._

---

## 1. What problem this solves

HONE is a **multi-founder chain** (Beastly, Grouchly, Nebra, plus any other full
nodes). The existing `daily-healthcheck.mjs` proves each node's **API is alive** —
it does **not** prove the nodes **agree** with each other.

The failure mode it cannot see is a **fork**: two nodes that are both healthy, both
serving HTTP 200, both advancing epochs — but disagreeing about the **contents/hash**
of some block. From that block onward every node builds on divergent history and the
network has silently split into two incompatible chains. Users on each side see their
transactions "confirmed"; neither side sees the other.

This is the exact class of failure the CLAUDE.md hardline warns about ("A node with
zero peers MUST NOT accept or apply any user-submitted entry" → prevents silent forks).
`fork-check.mjs` is the **detector** for it.

## 2. How it works (algorithm)

1. For each node: `GET /api/node/info` (→ `chain_id`, identity, epoch) and
   `GET /api/latest` (→ latest **sealed** epoch + its `hash`).
2. **chain_id gate:** assert every node reports the same `chain_id`. If not, the nodes
   aren't even the same network (e.g. one still on `hone-2`, one on `hone`) — reported
   as `CHAIN_ID_MISMATCH`, a louder and distinct failure from a hash fork.
3. **Pick comparison height:** the **minimum** latest-sealed epoch across all nodes
   (the highest block every node is guaranteed to hold), plus always block 0 (genesis).
   Override with `--epoch N`.
4. **Compare:** `GET /api/block/:epoch` from every node, compare the `hash` field.
   All equal → **CONVERGED**. Any difference → **FORK**.
5. `--deep`: binary-search backwards to find the **fork point** — the lowest epoch where
   the hashes first diverge. That epoch is where the histories split (the key diagnostic).

## 3. Exit codes (for cron / alerting)

| Code | Verdict | Meaning |
|---|---|---|
| `0` | CONVERGED | All reachable nodes agree. Healthy. |
| `1` | FORK | Hash divergence at a comparison height. **Network split — act now.** |
| `2` | CHAIN_ID_MISMATCH | Nodes on different networks entirely. |
| `3` | UNREACHABLE | No node reachable / garbage response — cannot conclude. |

Any non-zero code means **a human must look now**. Wire code `1` and `2` to your loudest
alert channel.

## 4. Usage

```bash
# Normal multi-node check (fill in the real founder URLs — Tailscale IPs etc.)
node scripts/fork-check.mjs \
  --node beastly=http://localhost:4242 \
  --node grouchly=http://100.x.y.z:4242 \
  --node nebra=http://100.a.b.c:4242

# Machine-readable output for a dashboard / alert pipe
node scripts/fork-check.mjs --json reports/fork/latest.json --node ...=... --node ...=...

# Find the exact epoch where two nodes diverged
node scripts/fork-check.mjs --deep --node ...=... --node ...=...
```

> A meaningful fork check needs **≥2 reachable nodes**. With one node it reports
> CONVERGED trivially (nothing to disagree with). One unreachable node is a **warning**,
> not fatal — the reachable nodes are still compared.

## 5. ⭐ Primary use at the HONE→HONE re-genesis cutover

During the cutover, every founder rebuilds, **wipes its data dir**, and restarts on the
new `hone` genesis. The one question that decides success: **did every node land on the
SAME new block-0 hash?** (chain_id + genesis proclamation changed → a NEW block-0 hash;
all nodes must produce the *same* new one.)

**Cutover go/no-go procedure:**

```bash
# Right after all founder nodes restart on the new "hone" genesis:
node scripts/fork-check.mjs --epoch 0 \
  --node beastly=http://localhost:4242 \
  --node grouchly=http://100.x.y.z:4242 \
  --node nebra=http://100.a.b.c:4242
```

- **`CONVERGED` (exit 0)** → every node agrees on block 0. Cutover succeeded. Proceed.
- **`FORK` (exit 1)** → nodes booted **different** genesis blocks (wrong `chain_id`, wrong
  proclamation text, or a **stale data dir that wasn't wiped**). **STOP.** Do not let the
  divergent node gossip — it will poison peers. Fix and re-verify before continuing.
- **`CHAIN_ID_MISMATCH` (exit 2)** → a node is still on `hone-2`. It never cut over.

This is the **authoritative convergence verifier** for the cutover — do not eyeball
block-0 hashes by hand during the most fork-prone moment the chain will ever have.

## 6. Recommended scheduling (steady state)

Run every 1–5 minutes from cron / a systemd timer, wired to alerts on non-zero exit.
A `fork-check.timer` mirroring `hone-update.timer` is the natural home. Example unit:

```ini
# hone-forkcheck.timer
[Timer]
OnBootSec=3min
OnUnitActiveSec=2min
[Install]
WantedBy=timers.target
```

```ini
# hone-forkcheck.service  (ExecStart runs the check; a non-zero exit triggers OnFailure=)
[Service]
Type=oneshot
ExecStart=/usr/bin/node /opt/hone/scripts/fork-check.mjs --node beastly=http://localhost:4242 --node grouchly=http://GROUCHLY_IP:4242 --json /var/lib/hone/fork-latest.json
OnFailure=hone-alert@%n.service
```

> **Cutover caveat (applies to ALL timers):** pause `hone-update.timer` on every node
> during the cutover window so the auto-updater does not restart a node mid-re-genesis
> onto a half-published binary. `fork-check` is safe to keep running (read-only).

## 7. Notes / limitations

- The check compares the **min common sealed height**. If nodes are at very different
  heights (one far behind syncing), it compares the height the laggard has reached — a
  lagging-but-agreeing node reads as CONVERGED, which is correct (it isn't forked, just
  behind). Watch `latestEpoch` spread in the output to spot a stuck/slow node separately.
- It trusts each node's self-reported `hash`. A node lying about its own hash is a
  different (Byzantine) threat model than an honest fork; this routine targets honest
  divergence, which is the realistic failure for a small founder set.
- `chain_id` is read from `HONE_CHAIN_ID` env (default `hone`) on each node — a
  misconfigured env on one node surfaces as CHAIN_ID_MISMATCH, which is the desired alert.

## 8. Where reports go

Write JSON to `reports/fork/` (mirrors `reports/health/`). Keep dated snapshots so a fork
has a timestamped first-detection record. **Do NOT** put node URLs containing private
Tailscale/LAN IPs into any file that gets committed publicly — see the backup doc for how
to keep the node inventory out of git.
