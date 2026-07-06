#!/usr/bin/env node
// BTCPC Daily Health Check — exercises every functional area against the LIVE
// chain and reports where to FIX (broken) vs where to BUILD (empty / missing).
//
// Classification per area:
//   WORKING  — endpoint responds AND has real activity/data
//   EMPTY    — endpoint responds but no data yet → BUILD/ACTIVATE
//   BROKEN   — endpoint errors / wrong shape / auth fails → FIX
//   SKIP     — needs a signed write we won't do unattended (reported, not run)
//
// Usage:  node scripts/daily-healthcheck.mjs [--url http://localhost:4242] [--json out.json]
// No dependencies (uses global fetch, Node 18+). Read-only — never signs/writes.

const BASE = (argVal("--url") || process.env.BTCPC_HEALTHCHECK_URL || "http://localhost:4242").replace(/\/$/, "");
const JSON_OUT = argVal("--json");
const TIMEOUT_MS = 8000;

function argVal(flag) {
  const i = process.argv.indexOf(flag);
  return i >= 0 ? process.argv[i + 1] : null;
}

async function get(path) {
  const ctl = new AbortController();
  const t = setTimeout(() => ctl.abort(), TIMEOUT_MS);
  try {
    const res = await fetch(`${BASE}${path}`, { signal: ctl.signal });
    const text = await res.text();
    let body;
    try { body = JSON.parse(text); } catch { body = text; }
    return { status: res.status, body };
  } catch (e) {
    return { status: 0, body: null, error: String(e.message || e) };
  } finally {
    clearTimeout(t);
  }
}

// hasData: does the response carry real activity (not just an empty shell)?
function hasData(body) {
  if (body == null) return false;
  if (Array.isArray(body)) return body.length > 0;
  if (typeof body === "object") {
    // common empty shapes: {count:0}, {items:[]}, {storefronts:[],count:0}
    if ("count" in body && body.count === 0) return false;
    for (const k of ["accounts","nodes","peers","repos","storefronts","sensors","jobs","tasks","agents","stores","products","auctions","data","items","list"]) {
      if (Array.isArray(body[k])) return body[k].length > 0;
    }
    return Object.keys(body).length > 0;
  }
  return String(body).length > 0;
}

// Each check: { area, name, path, expectData, writeGated }
// expectData=true → EMPTY (needs data) is a BUILD signal; false → presence is enough.
const CHECKS = [
  // ── Core chain (must WORK) ──
  { area: "core",     name: "health",              path: "/health",                        expectData: false, mustWork: true },
  { area: "core",     name: "node info",           path: "/api/node/info",                 expectData: true,  mustWork: true },
  { area: "core",     name: "node list",           path: "/api/node/list",                 expectData: true,  mustWork: true },
  { area: "core",     name: "latest block",        path: "/api/latest",                    expectData: true,  mustWork: true },
  { area: "core",     name: "genesis block",       path: "/api/block/0",                   expectData: true,  mustWork: true },
  { area: "core",     name: "chain validators",    path: "/api/chain/validators/1",        expectData: false, mustWork: false },
  { area: "core",     name: "state root",          path: "/api/chain/state_root",          expectData: false, mustWork: false },
  { area: "core",     name: "accounts",            path: "/api/accounts",                  expectData: true,  mustWork: true },

  // ── Inference (bullship / the marketplace) ──
  { area: "inference", name: "models",             path: "/v1/models",                     expectData: true,  mustWork: true },
  { area: "inference", name: "pricing",            path: "/v1/pricing",                    expectData: true,  mustWork: true },

  // ── Verasens (sensors) — global discovery list (added to fix the gap) ──
  { area: "verasens",  name: "sensors list",       path: "/api/sensors",                   expectData: true,  mustWork: false },

  // ── LinkGit — global repo discovery list (added to fix the gap) ──
  { area: "linkgit",   name: "repos (all)",        path: "/api/linkgit/repos",             expectData: true,  mustWork: false },

  // ── Freeport (marketplace) — expect EMPTY until storefronts ──
  { area: "freeport",  name: "storefronts",        path: "/api/commerce/storefronts",      expectData: true,  mustWork: false },

  // ── Agent layer ──
  { area: "agents",    name: "agent registry",     path: "/api/agent/registry",            expectData: true,  mustWork: false },
  { area: "agents",    name: "agent tasks",        path: "/api/agent/tasks",               expectData: true,  mustWork: false },

  // ── Runtime / service host ──
  { area: "runtime",   name: "node hosting",       path: "/api/service/node-hosting",      expectData: true,  mustWork: false },

  // ── Bridge / oracle / tracker / auctions (presence check) ──
  { area: "bridge",    name: "bridge status",      path: "/api/bridge/status",             expectData: false, mustWork: false },
  { area: "oracle",    name: "oracle price",       path: "/api/oracle/price/BTC",          expectData: false, mustWork: false },
  { area: "explorer",  name: "public machine status", path: "/public/machine-status",      expectData: true,  mustWork: false },
  { area: "explorer",  name: "explorer network",   path: "/public/network",                expectData: true,  mustWork: false },

  // ── Integration manifest (the self-updating consumer contract) ──
  { area: "manifest",  name: "integration manifest", path: "/api/integration/manifest",    expectData: true,  mustWork: false },
];

function classify(check, res) {
  if (res.status === 0) return { verdict: "BROKEN", why: `unreachable: ${res.error || "no response"}` };
  if (res.status === 404) return { verdict: "BROKEN", why: "404 — route missing or renamed" };
  if (res.status >= 500) return { verdict: "BROKEN", why: `HTTP ${res.status} server error` };
  if (res.status === 401 || res.status === 403) return { verdict: "BROKEN", why: `HTTP ${res.status} auth` };
  if (res.status !== 200) return { verdict: "BROKEN", why: `HTTP ${res.status}` };
  if (check.expectData) {
    return hasData(res.body)
      ? { verdict: "WORKING", why: "responds with data" }
      : { verdict: "EMPTY", why: "responds but no data yet" };
  }
  return { verdict: "WORKING", why: "responds" };
}

async function main() {
  const started = new Date().toISOString();
  const results = [];
  for (const c of CHECKS) {
    const res = await get(c.path);
    const cls = classify(c, res);
    results.push({ ...c, status: res.status, ...cls });
  }

  // Roll up
  const byVerdict = { WORKING: [], EMPTY: [], BROKEN: [] };
  for (const r of results) byVerdict[r.verdict]?.push(r);

  // FIX list = BROKEN, especially mustWork ones. BUILD list = EMPTY.
  const fixNow = results.filter(r => r.verdict === "BROKEN");
  const fixCritical = fixNow.filter(r => r.mustWork);
  const buildNext = results.filter(r => r.verdict === "EMPTY");

  const report = renderReport({ started, results, byVerdict, fixNow, fixCritical, buildNext });
  console.log(report);

  if (JSON_OUT) {
    const fs = await import("node:fs");
    fs.writeFileSync(JSON_OUT, JSON.stringify({ started, base: BASE, results }, null, 2));
  }

  // Exit non-zero if any MUST-WORK check is broken (CI/alerting hook).
  process.exit(fixCritical.length > 0 ? 2 : 0);
}

function renderReport({ started, results, byVerdict, fixNow, fixCritical, buildNext }) {
  const L = [];
  L.push(`# BTCPC Daily Health Check`);
  L.push(``);
  L.push(`- **When:** ${started}`);
  L.push(`- **Target:** ${BASE}`);
  L.push(`- **Summary:** ${byVerdict.WORKING.length} working · ${byVerdict.EMPTY.length} empty (build) · ${byVerdict.BROKEN.length} broken (fix)`);
  if (fixCritical.length) L.push(`- **⛔ CRITICAL: ${fixCritical.length} must-work checks are BROKEN**`);
  L.push(``);

  if (fixNow.length) {
    L.push(`## 🔧 FIX — broken (regressions / dead routes)`);
    for (const r of fixNow) {
      const crit = r.mustWork ? " **[CRITICAL]**" : "";
      L.push(`- \`${r.area}/${r.name}\` (${r.path}) — ${r.why}${crit}`);
    }
    L.push(``);
  }

  if (buildNext.length) {
    L.push(`## 🏗️  BUILD — responds but EMPTY (activate / build the vertical)`);
    for (const r of buildNext) {
      L.push(`- \`${r.area}/${r.name}\` (${r.path}) — ${r.why}`);
    }
    L.push(``);
  }

  L.push(`## ✅ WORKING`);
  for (const r of byVerdict.WORKING) L.push(`- \`${r.area}/${r.name}\` — ${r.why}`);
  L.push(``);

  L.push(`## 📋 Known BUILD gaps (missing functionality, not regressions)`);
  L.push(`- **Verticals at zero live usage:** Verasens (sig-bug fixed, needs live submit), LinkGit (no repos), Freeport (no storefronts + missing drop-ship/warehouse/institutional vision). ACTIVATE + BUILD.`);
  L.push(`- (Fixed) Discovery list endpoints \`/api/sensors\` + \`/api/linkgit/repos\` added — Verasens/LinkGit now enumerable.`);
  L.push(``);
  L.push(`## Where to focus`);
  if (fixCritical.length) {
    L.push(`1. **FIX the ${fixCritical.length} critical broken check(s) first** — core chain functionality is degraded.`);
  } else if (fixNow.length) {
    L.push(`1. **FIX ${fixNow.length} broken non-critical route(s)** — regressions, not launch-blocking.`);
  } else {
    L.push(`1. **No breakage** — all responding routes healthy.`);
  }
  if (buildNext.length) {
    L.push(`2. **BUILD/ACTIVATE ${buildNext.length} empty area(s)** — infrastructure exists, no live usage. Priority: verasens → linkgit → freeport (per the post-launch goal).`);
  }
  L.push(``);
  L.push(`_Generated by scripts/daily-healthcheck.mjs — read-only, no writes/signing._`);
  return L.join("\n");
}

main().catch(e => { console.error("healthcheck fatal:", e); process.exit(3); });
