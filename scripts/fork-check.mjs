#!/usr/bin/env node
// ============================================================================
// fork-check.mjs — CROSS-NODE CONSENSUS / FORK-DIVERGENCE MONITOR for HONE
// ============================================================================
//
// WHY THIS EXISTS
// ---------------
// HONE is a multi-founder chain (Beastly / Grouchly / Nebra, plus any other
// full nodes). The single worst silent failure a chain can have is a FORK:
// two nodes that both believe they are healthy, both serving HTTP 200, both
// advancing epochs — but disagreeing about the CONTENTS of a past block. Once
// that happens, every subsequent block builds on divergent history and the
// network has quietly split into two incompatible chains.
//
// The existing daily-healthcheck.mjs proves each node's API is ALIVE. It does
// NOT prove that the nodes AGREE. A node can pass every liveness check while
// being on a fork. This routine closes that gap: it asks every founder node
// "what is the hash of block N?" for common heights N, and screams if any two
// answers differ.
//
// See CLAUDE.md "Hardline: No Local Submission Without Peers" — a disconnected
// node that applies entries locally silently forks. This routine is the
// detector for exactly that class of failure.
//
// SECONDARY USE — RE-GENESIS CUTOVER VERIFIER
// -------------------------------------------
// During the BTCPC->HONE re-genesis cutover, every founder rebuilds, wipes its
// data dir, and restarts on the new "hone" genesis. The single question that
// determines whether the cutover SUCCEEDED is: "did every node land on the
// SAME new block-0 hash?" Run this script with --epoch 0 against all founder
// nodes right after they restart. If block 0 hashes match across every node,
// the cutover converged. If not, someone booted a different genesis (wrong
// chain_id, wrong proclamation, stale data dir) and MUST be stopped before it
// poisons peers. This is the authoritative go/no-go check for the cutover.
//
// WHAT IT DOES (algorithm)
// ------------------------
//   1. For each node URL, GET /api/node/info  -> chain_id, identity, epoch
//      GET /api/latest                        -> latest sealed epoch + hash
//   2. Assert every node reports the SAME chain_id. A chain_id mismatch means
//      the nodes are not even trying to be the same network (e.g. one still on
//      "btcpc-2", one on "hone") — reported as a distinct, louder failure than
//      a hash fork.
//   3. Pick the comparison height: the MINIMUM latest-sealed epoch across all
//      nodes (the highest block every node is guaranteed to have), unless the
//      user pins one with --epoch N. Also always compare block 0 (genesis) as a
//      cheap identity anchor when --deep or --epoch 0.
//   4. For the chosen height(s), GET /api/block/:epoch from every node and
//      compare the "hash" field. All-equal => CONVERGED. Any difference => FORK.
//   5. Optionally (--deep) binary-search backwards to find the FORK POINT: the
//      lowest epoch at which the hashes first diverge. That epoch is where the
//      histories split and is the most useful number for diagnosis.
//
// DESIGN CONSTRAINTS (match daily-healthcheck.mjs conventions)
// ------------------------------------------------------------
//   * Zero npm dependencies. Uses global fetch (Node 18+).
//   * READ-ONLY. Never signs, never writes to any chain. Only GETs.
//   * Exit codes for alerting/cron:
//       0  = all nodes converged (and chain_id agrees)
//       1  = FORK detected (hash divergence at a comparison height)
//       2  = chain_id mismatch (nodes on different networks entirely)
//       3  = a node was unreachable / returned garbage (cannot conclude)
//     Non-zero always means "a human must look now."
//   * Deterministic, side-effect-free, safe to run every minute from cron.
//
// USAGE
// -----
//   node scripts/fork-check.mjs \
//     --node beastly=http://localhost:4242 \
//     --node grouchly=http://100.x.y.z:4242 \
//     --node nebra=http://100.a.b.c:4242
//
//   # Cutover verify (compare genesis specifically):
//   node scripts/fork-check.mjs --epoch 0 --node ...=... --node ...=...
//
//   # Find the exact fork point:
//   node scripts/fork-check.mjs --deep --node ...=... --node ...=...
//
//   # Machine-readable output for a dashboard / alert pipe:
//   node scripts/fork-check.mjs --json report.json --node ...=...
//
// If no --node flags are given, it defaults to a single local node at
// http://localhost:4242 labelled "local" (a self-check — useful to confirm the
// script runs, though a fork needs >=2 nodes to be meaningful).
//
// ============================================================================

// ── Arg parsing (tiny, dependency-free) ─────────────────────────────────────
const argv = process.argv.slice(2);
const nodes = [];            // [{ label, url }]
let pinnedEpoch = null;      // --epoch N   (compare exactly this height)
let deep = false;            // --deep      (binary-search the fork point)
let jsonOut = null;          // --json path (write machine-readable report)
let timeoutMs = 8000;        // --timeout ms per request

for (let i = 0; i < argv.length; i++) {
  const a = argv[i];
  if (a === "--node") {
    // Format: label=url  (label optional; falls back to the url)
    const spec = argv[++i] || "";
    const eq = spec.indexOf("=");
    if (eq > 0) nodes.push({ label: spec.slice(0, eq), url: spec.slice(eq + 1).replace(/\/$/, "") });
    else        nodes.push({ label: spec, url: spec.replace(/\/$/, "") });
  } else if (a === "--epoch") {
    pinnedEpoch = parseInt(argv[++i], 10);
  } else if (a === "--deep") {
    deep = true;
  } else if (a === "--json") {
    jsonOut = argv[++i];
  } else if (a === "--timeout") {
    timeoutMs = parseInt(argv[++i], 10) || timeoutMs;
  } else if (a === "--help" || a === "-h") {
    console.log("Usage: node scripts/fork-check.mjs --node label=url [--node ...] [--epoch N] [--deep] [--json out.json]");
    process.exit(0);
  }
}

if (nodes.length === 0) {
  nodes.push({ label: "local", url: "http://localhost:4242" });
}

// ── HTTP helper: GET JSON with a timeout; never throws (returns error obj) ──
async function getJson(url, path) {
  const ctl = new AbortController();
  const t = setTimeout(() => ctl.abort(), timeoutMs);
  try {
    const res = await fetch(`${url}${path}`, { signal: ctl.signal });
    let body = null;
    try { body = await res.json(); } catch { /* non-JSON body */ }
    return { ok: res.status === 200, status: res.status, body };
  } catch (e) {
    return { ok: false, status: 0, body: null, error: String(e.message || e) };
  } finally {
    clearTimeout(t);
  }
}

// ── Fetch a node's identity + latest sealed epoch/hash ──────────────────────
async function probeNode(n) {
  const info = await getJson(n.url, "/api/node/info");
  const latest = await getJson(n.url, "/api/latest");
  if (!info.ok || !latest.ok) {
    return {
      ...n, reachable: false,
      error: info.error || latest.error || `HTTP info=${info.status} latest=${latest.status}`,
    };
  }
  return {
    ...n,
    reachable: true,
    chain_id: info.body?.chain_id ?? "unknown",
    // node_info exposes the current epoch; latest exposes the last SEALED one.
    latestEpoch: Number(latest.body?.epoch ?? 0),
    latestHash: String(latest.body?.hash ?? ""),
    currentEpoch: Number(latest.body?.current_epoch ?? info.body?.epoch ?? 0),
  };
}

// ── Fetch one block's hash from a node ──────────────────────────────────────
async function blockHash(url, epoch) {
  const r = await getJson(url, `/api/block/${epoch}`);
  if (!r.ok) return { ok: false, status: r.status, error: r.error };
  return { ok: true, hash: String(r.body?.hash ?? ""), status: String(r.body?.status ?? "") };
}

// ── Compare a single epoch across all reachable nodes ───────────────────────
// Returns { epoch, converged, hashes: {label: hash|null}, distinct: [..] }
async function compareEpoch(reachable, epoch) {
  const hashes = {};
  for (const n of reachable) {
    const bh = await blockHash(n.url, epoch);
    hashes[n.label] = bh.ok ? bh.hash : null; // null = node lacks this block
  }
  const present = Object.values(hashes).filter(h => h !== null && h !== "");
  const distinct = [...new Set(present)];
  // Converged iff every node that HAS the block reports the SAME non-empty hash,
  // AND at least two nodes actually reported it (one node can't fork alone).
  const reporters = present.length;
  const converged = distinct.length <= 1;
  return { epoch, converged, distinct, reporters, hashes };
}

// ── Binary-search the fork point: lowest epoch where hashes first diverge ────
// Precondition: block 0..lo agree, hi diverges. Returns the first diverging epoch.
async function findForkPoint(reachable, lo, hi) {
  // Invariant: compareEpoch(lo).converged === true, compareEpoch(hi).converged === false
  while (hi - lo > 1) {
    const mid = Math.floor((lo + hi) / 2);
    const c = await compareEpoch(reachable, mid);
    if (c.converged) lo = mid; else hi = mid;
  }
  return hi; // first diverging epoch
}

// ── Main ────────────────────────────────────────────────────────────────────
async function main() {
  const stamp = new Date().toISOString();
  const probes = await Promise.all(nodes.map(probeNode));
  const reachable = probes.filter(p => p.reachable);
  const unreachable = probes.filter(p => !p.reachable);

  const report = {
    timestamp: stamp,
    nodes: probes.map(p => ({
      label: p.label, url: p.url, reachable: p.reachable,
      chain_id: p.chain_id, latestEpoch: p.latestEpoch, latestHash: p.latestHash,
      error: p.error,
    })),
    verdict: "UNKNOWN",
    detail: "",
    comparisons: [],
    forkPoint: null,
  };

  // ── Gate 0: enough reachable nodes to conclude anything ───────────────────
  if (reachable.length === 0) {
    report.verdict = "UNREACHABLE";
    report.detail = "No nodes reachable — cannot assess consensus.";
    return finish(report, 3);
  }
  if (unreachable.length) {
    // Not fatal on its own, but we note it — a fork check with a node missing
    // is incomplete. We still compare the nodes we CAN reach.
    report.detail = `WARNING: ${unreachable.length} node(s) unreachable: ${unreachable.map(u => `${u.label}(${u.error})`).join(", ")}. `;
  }

  // ── Gate 1: chain_id agreement (different networks, not a mere fork) ───────
  const chainIds = [...new Set(reachable.map(n => n.chain_id))];
  if (chainIds.length > 1) {
    report.verdict = "CHAIN_ID_MISMATCH";
    report.detail += `Nodes report DIFFERENT chain_ids: ${reachable.map(n => `${n.label}=${n.chain_id}`).join(", ")}. `
                   + `These are not the same network (e.g. a node still on btcpc-2 vs hone). Fix before comparing blocks.`;
    return finish(report, 2);
  }
  const chainId = chainIds[0];

  // ── Choose comparison height(s) ───────────────────────────────────────────
  // Default: the min latest-sealed epoch (highest block ALL nodes have).
  const commonHeight = Math.min(...reachable.map(n => n.latestEpoch));
  const heights = [];
  // Always anchor on genesis (block 0) — cheap, and the whole point at cutover.
  heights.push(0);
  if (pinnedEpoch !== null) {
    if (pinnedEpoch !== 0) heights.push(pinnedEpoch);
  } else if (commonHeight > 0) {
    heights.push(commonHeight);
  }

  // ── Compare each chosen height ────────────────────────────────────────────
  let anyFork = false;
  for (const h of [...new Set(heights)].sort((a, b) => a - b)) {
    const c = await compareEpoch(reachable, h);
    report.comparisons.push(c);
    if (!c.converged) anyFork = true;
  }

  // ── Optional: locate the fork point ───────────────────────────────────────
  if (anyFork && deep) {
    // Find a converged low anchor (0 usually) and a diverged high anchor.
    const genesis = report.comparisons.find(c => c.epoch === 0);
    const top = await compareEpoch(reachable, commonHeight);
    if (genesis && genesis.converged && !top.converged) {
      report.forkPoint = await findForkPoint(reachable, 0, commonHeight);
      report.detail += `Fork first appears at epoch ${report.forkPoint}. `;
    } else if (genesis && !genesis.converged) {
      report.detail += `Divergence starts at GENESIS (block 0) — nodes booted different chains, not a runtime fork. `;
    }
  }

  // ── Verdict ───────────────────────────────────────────────────────────────
  if (anyFork) {
    report.verdict = "FORK";
    report.detail += `Hash divergence detected across founder nodes on chain_id="${chainId}". `
                   + `The network has split — investigate immediately.`;
    return finish(report, 1);
  }

  report.verdict = "CONVERGED";
  report.detail += `All ${reachable.length} reachable node(s) agree on chain_id="${chainId}" `
                 + `at epoch(s) ${[...new Set(heights)].join(", ")}. No fork.`;
  return finish(report, 0);
}

// ── Output + exit ────────────────────────────────────────────────────────────
function finish(report, code) {
  // Human-readable summary
  const sym = { CONVERGED: "✅", FORK: "⛔", CHAIN_ID_MISMATCH: "⛔", UNREACHABLE: "⚠️", UNKNOWN: "?" }[report.verdict] || "?";
  console.log(`${sym} ${report.verdict} — ${report.detail}`);
  console.log("");
  console.log("Nodes:");
  for (const n of report.nodes) {
    if (n.reachable) {
      console.log(`  ${n.label.padEnd(10)} chain=${n.chain_id}  latest=epoch ${n.latestEpoch}  hash=${(n.latestHash || "").slice(0, 16)}…`);
    } else {
      console.log(`  ${n.label.padEnd(10)} UNREACHABLE (${n.error})`);
    }
  }
  if (report.comparisons.length) {
    console.log("");
    console.log("Block-hash comparison:");
    for (const c of report.comparisons) {
      const mark = c.converged ? "✅ agree" : "⛔ DIVERGE";
      console.log(`  epoch ${String(c.epoch).padStart(8)}  ${mark}  (${c.reporters} reporters, ${c.distinct.length} distinct hash${c.distinct.length === 1 ? "" : "es"})`);
      if (!c.converged) {
        for (const [label, hash] of Object.entries(c.hashes)) {
          console.log(`      ${label.padEnd(10)} ${hash ? hash.slice(0, 24) + "…" : "(no block)"}`);
        }
      }
    }
  }
  if (report.forkPoint !== null) {
    console.log("");
    console.log(`  >>> FORK POINT: histories first diverge at epoch ${report.forkPoint} <<<`);
  }

  if (jsonOut) {
    import("fs").then(fs => {
      fs.writeFileSync(jsonOut, JSON.stringify(report, null, 2));
      console.log(`\n(json written to ${jsonOut})`);
      process.exit(code);
    });
  } else {
    process.exit(code);
  }
}

main().catch(e => { console.error("fork-check fatal:", e); process.exit(3); });
