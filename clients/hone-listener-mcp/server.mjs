#!/usr/bin/env node
// Ornigo listener MCP server (package hone-listener-mcp).
//
// Lets an AI assistant install the Ornigo listener and answer "what birds did we hear?".
// Zero dependencies (Node 18+). Two modes:
//   stdio (default)  all four tools; install_listener runs on this computer.
//   --http [port]    remote, read-only: my_birds, is_back, listener_status (no installs).
//
// Safety rules, enforced in code:
//   - Every download comes from https://honemesh.net/app/listener/ (fixed below). No tool takes a URL.
//   - version.json must carry a valid ed25519 signature (version.json.sig) from the pinned HONE
//     release key, and the installer's sha256 must match the signed manifest, BEFORE anything runs.
//   - The installer runs only after the user has confirmed (confirm: true), from the exact bytes
//     that were verified, with stdin closed.
//   - Only read-only web keys (prefix w_) are used for account reads.
import { createHash, createPublicKey, verify as edVerify } from "node:crypto";
import { spawn, spawnSync } from "node:child_process";
import { mkdtempSync, writeFileSync, readFileSync, existsSync, rmSync } from "node:fs";
import { isAbsolute } from "node:path";
import { tmpdir, homedir, platform, arch, release } from "node:os";
import { join } from "node:path";
import { createServer } from "node:http";

const VERSION = "0.1.1";
const ORIGIN = "https://honemesh.net";
const LISTENER_PATH = "/app/listener/";
const API = ORIGIN + "/api/accounts";
// HONE listener release key (ed25519, raw public key). Pinned: change only with a new MCP release.
const RELEASE_PUB_HEX = "27ef54588674c3391e660056d873b89136e75125b5a433de0747191baa94a10d";
const INSTALLER = { win32: "install.ps1", darwin: "install.sh", linux: "install.sh" };
const MAX_MANIFEST = 64 * 1024;
const MAX_INSTALLER = 8 * 1024 * 1024;
const DISCLAIMER = "Human conversation is automatically deleted. The listener only keeps a count of voices it removed. " +
  "No audio is ever saved or sent; only a short bird report every 15 minutes.";

const releaseKey = createPublicKey({
  key: Buffer.concat([Buffer.from("302a300506032b6570032100", "hex"), Buffer.from(RELEASE_PUB_HEX, "hex")]),
  format: "der", type: "spki",
});

// ── pinned downloads ───────────────────────────────────────────────────────────────────
async function getPinned(name, max) {
  if (!/^[A-Za-z0-9._-]+$/.test(name)) throw new Error("bad file name");
  const url = ORIGIN + LISTENER_PATH + name;
  let res;
  try {
    res = await fetch(url, { redirect: "error", cache: "no-store", signal: AbortSignal.timeout(60000),
      headers: { "User-Agent": "hone-listener-mcp/" + VERSION } });
  } catch (e) {
    throw new Error(`Couldn't reach honemesh.net for ${name} (${e.cause?.code || e.name}).`);
  }
  if (new URL(res.url).origin !== ORIGIN) throw new Error("refused: response did not come from honemesh.net");
  if (res.status === 404) { const e = new Error(`${name} is not published yet`); e.notYet = true; throw e; }
  if (!res.ok) throw new Error(`honemesh.net answered HTTP ${res.status} for ${name}`);
  const buf = Buffer.from(await res.arrayBuffer());
  if (buf.length > max) throw new Error(`${name} is larger than expected; refusing`);
  return { url, buf };
}

function parseSig(buf) {
  const s = buf.toString("utf8").trim();
  if (/^[0-9a-fA-F]{128}$/.test(s)) return Buffer.from(s, "hex");
  const b = Buffer.from(s, "base64");
  if (b.length === 64) return b;
  throw new Error("version.json.sig is not a 64-byte ed25519 signature (hex or base64)");
}

const sha256 = (b) => createHash("sha256").update(b).digest("hex");

/** Download + verify the signed manifest and this OS's installer. Throws on any mismatch. */
async function fetchVerified() {
  const os = platform();
  const file = INSTALLER[os];
  if (!file) throw new Error(`The listener doesn't support ${os} yet (Windows, macOS and Linux only).`);
  const steps = [];
  let man;
  try {
    man = await getPinned("version.json", MAX_MANIFEST);
  } catch (e) {
    if (e.notYet) throw new Error("Ornigo hasn't been published on honemesh.net yet, so nothing was installed. It's coming in the next few days; try again then.");
    throw e;
  }
  steps.push(`Downloaded ${man.url} (${man.buf.length} bytes).`);
  let sigFile;
  try {
    sigFile = await getPinned("version.json.sig", 4096);
  } catch (e) {
    if (e.notYet) throw new Error("The listener release isn't signed yet (no version.json.sig), so nothing will be installed. Try again later.");
    throw e;
  }
  if (!edVerify(null, man.buf, releaseKey, parseSig(sigFile.buf))) {
    throw new Error("REFUSED: version.json's signature does not match the pinned HONE release key. Nothing was run.");
  }
  steps.push(`Signature on version.json checked against the pinned HONE release key ${RELEASE_PUB_HEX.slice(0, 16)}…: valid.`);
  let m;
  try { m = JSON.parse(man.buf.toString("utf8")); } catch { throw new Error("version.json is not valid JSON"); }
  if (m.name !== "hone-listener") throw new Error("REFUSED: signed manifest is not for hone-listener");
  const want = String(m.files?.[file]?.sha256 || "").toLowerCase();
  if (!/^[0-9a-f]{64}$/.test(want)) throw new Error(`REFUSED: the signed manifest has no sha256 for ${file}`);
  const inst = await getPinned(file, MAX_INSTALLER);
  const got = sha256(inst.buf);
  steps.push(`Downloaded ${inst.url} (${inst.buf.length} bytes).`);
  if (got !== want) throw new Error(`REFUSED: ${file} sha256 ${got} does not match the signed manifest (${want}). Nothing was run.`);
  steps.push(`sha256 of ${file} is ${got}: matches the signed manifest.`);
  return { os, file, manifest: m, manifestBytes: man.buf, installer: inst.buf, sha: got, steps };
}

// ── account reads (read-only web key) ──────────────────────────────────────────────────
function token(headerTok) {
  let t = headerTok || process.env.HONE_TOKEN || "";
  if (!t) {
    try { t = readFileSync(join(homedir(), ".hone", "listener-token"), "utf8"); } catch { /* none */ }
  }
  t = t.trim();
  if (!t) return null;
  if (!/^w_[A-Za-z0-9_-]{20,}$/.test(t)) throw new Error("HONE_TOKEN must be a read-only Ornigo key from honemesh.net/me (it starts with w_).");
  return t;
}

async function api(path, tok) {
  if (!tok) {
    throw new Error("Not connected to an Ornigo account yet. Ask the user to open https://honemesh.net/me, sign in, " +
      "tap \"Make an AI key\" and add it to this assistant as HONE_TOKEN. The key can only read birds and listeners.");
  }
  const res = await fetch(API + path, { redirect: "error", signal: AbortSignal.timeout(30000),
    headers: { Authorization: "Bearer " + tok, "User-Agent": "hone-listener-mcp/" + VERSION } });
  const j = await res.json().catch(() => ({}));
  if (res.status === 401) throw new Error("The Ornigo key was refused (expired or signed out). Make a new one at https://honemesh.net/me.");
  if (!res.ok) throw new Error(j.error || `honemesh.net answered HTTP ${res.status}`);
  return j;
}

function when(t) {
  if (!t) return "never";
  const s = Date.now() / 1000 - t;
  const iso = new Date(t * 1000).toISOString().replace(".000Z", "Z");
  if (s < 3600) return `${Math.max(1, Math.round(s / 60))} min ago (${iso})`;
  if (s < 86400 * 2) return `${Math.round(s / 3600)} h ago (${iso})`;
  return `${Math.round(s / 86400)} days ago (${iso})`;
}

function parseSince(v) {
  if (v == null || v === "" || v === "all") return 0;
  const s = String(v).trim().toLowerCase();
  const now = Math.floor(Date.now() / 1000);
  const rel = { today: 86400, day: 86400, "24h": 86400, week: 7 * 86400, "7d": 7 * 86400, month: 30 * 86400, "30d": 30 * 86400 };
  if (rel[s]) return now - rel[s];
  const m = /^(\d+)\s*(h|d)$/.exec(s);
  if (m) return now - +m[1] * (m[2] === "h" ? 3600 : 86400);
  if (/^\d{9,11}$/.test(s)) return +s;
  const d = Date.parse(s);
  if (!Number.isNaN(d)) return Math.floor(d / 1000);
  throw new Error('since must be "today", "week", "month", "48h", "3d", an ISO date or unix seconds');
}

// ── local listener status ──────────────────────────────────────────────────────────────
function run(cmd, args) {
  try {
    const r = spawnSync(cmd, args, { encoding: "utf8", timeout: 8000, stdio: ["ignore", "pipe", "pipe"], windowsHide: true });
    return { code: r.status, out: ((r.stdout || "") + (r.stderr || "")).trim().slice(0, 600) };
  } catch (e) { return { code: -1, out: String(e.message).slice(0, 200) }; }
}

const PORT_FILES = {
  windows: "%LOCALAPPDATA%\\HONE\\Listener\\ui-port",
  linux: "/var/lib/hone-listener/ui-port",
  linux_user: "~/.local/share/hone-listener/ui-port",
  macos: "~/Library/Application Support/HONE Listener/ui-port",
};

function expandPath(p) {
  if (typeof p !== "string" || !/ui-port$/.test(p) || p.length > 300) return null;
  let x = p;
  if (x.startsWith("~/")) x = join(homedir(), x.slice(2));
  x = x.replace(/%([A-Za-z_]+)%/g, (_, k) => process.env[k] || "\0");
  return x.includes("\0") || !isAbsolute(x) ? null : x;
}

/** Port from the listener's port file (manifest paths first, then the known defaults). */
function readPortFile(os, fromManifest, scope) {
  const m = fromManifest && typeof fromManifest === "object" ? fromManifest : {};
  const keys = os === "win32" ? ["windows"] : os === "darwin" ? ["macos"] : scope === "user" ? ["linux_user", "linux"] : ["linux", "linux_user"];
  for (const k of keys) {
    for (const raw of [m[k], PORT_FILES[k]]) {
      const f = expandPath(raw);
      if (!f) continue;
      try {
        const n = Number(readFileSync(f, "utf8").slice(0, 16).trim());
        if (Number.isInteger(n) && n > 1023 && n < 65536) return n;
      } catch { /* not there */ }
    }
  }
  return 0;
}

async function localStatus() {
  // Service names come from the signed manifest when it's available; else the defaults.
  let local = {};
  try { local = (await fetchVerified()).manifest.local || {}; } catch { /* offline or not published */ }
  const name = (k, d) => (typeof local[k] === "string" && /^[A-Za-z0-9._-]{1,64}$/.test(local[k]) ? local[k] : d);
  const os = platform();
  let svc;
  if (os === "win32") {
    const n = name("windows_service", "HoneListener");
    const r = run("sc.exe", ["query", n]);
    svc = { name: n, installed: r.code === 0, running: /RUNNING/.test(r.out) };
    if (!svc.installed) {
      const t = run("schtasks.exe", ["/Query", "/TN", name("windows_task", "HONE Listener")]);
      if (t.code === 0) svc = { name: name("windows_task", "HONE Listener"), installed: true, running: /Running/i.test(t.out) };
    }
  } else if (os === "darwin") {
    const n = name("launchd_label", "net.honemesh.listener");
    const r = run("launchctl", ["list", n]);
    svc = { name: n, installed: r.code === 0, running: r.code === 0 && /"PID"/.test(r.out) };
  } else {
    const n = name("systemd_unit", "hone-listener");
    svc = { name: n, installed: false, running: false };
    for (const scope of [["--user"], []]) {
      const load = run("systemctl", [...scope, "show", "-p", "LoadState", "--value", n]).out;
      if (load === "loaded") {
        const st = run("systemctl", [...scope, "is-active", n]).out;
        svc = { name: n, scope: scope.length ? "user" : "system", installed: true, running: st === "active", state: st };
        break;
      }
    }
  }
  // 0.1.4+: the listener writes the port it actually bound to a file (status_port_file per OS);
  // status_port is only the preferred port.
  let port = readPortFile(os, local.status_port_file, svc.scope);
  const portFrom = port ? "port file" : "preferred port";
  if (!port) port = Number(local.status_port) || 47811;
  let http = null;
  if (Number.isInteger(port) && port > 1023 && port < 65536) {
    try {
      const r = await fetch(`http://127.0.0.1:${port}/status`, { signal: AbortSignal.timeout(3000), redirect: "error" });
      http = r.ok ? await r.json() : { http: r.status };
    } catch { http = { reachable: false }; }
  }
  if (http) http = { port, from: portFrom, ...http };
  return { os, ...svc, local_status: http };
}

// ── tools ──────────────────────────────────────────────────────────────────────────────
const TOOLS = [
  {
    name: "install_listener",
    description:
      "Install Ornigo, the bird listener, on THIS computer, so the user's home security camera can listen for birds. " +
      "Call it first WITHOUT confirm: it detects the operating system, downloads the signed release manifest and the installer " +
      "from honemesh.net only, checks the ed25519 signature against the pinned HONE release key and the installer's sha256, " +
      "and reports each check without running anything. Explain those steps to the user in plain words, read them the privacy " +
      "note it returns, and ask whether to go ahead. Only after the user says yes, call it again with confirm: true; it verifies " +
      "everything again and then runs the installer. Takes no URL: downloads can only come from honemesh.net. If any check " +
      "fails it refuses and runs nothing; tell the user exactly what failed. The installer may open a setup page in the " +
      "browser to sign in with email and pick the camera; tell the user to finish there.",
    inputSchema: {
      type: "object",
      properties: {
        confirm: { type: "boolean", description: "true only after the user has agreed to run the verified installer." },
        ref: { type: "string", description: "Optional invite code (the ?ref= from the user's invite link), lowercase letters, digits, . _ -" },
      },
      additionalProperties: false,
    },
    local: true,
  },
  {
    name: "listener_status",
    description:
      "Is the user's Ornigo listener running? Checks the listener service on this computer (when running locally) and, if a " +
      "Ornigo key is set, every listener and phone on the user's account: \"Listening now\" or when it last sent a report. " +
      "Reports arrive every 15 minutes; a device counts as listening if one arrived in the last 35 minutes. Explain the result " +
      "simply, e.g. \"Your backyard camera is listening now; its last report was 6 minutes ago.\"",
    inputSchema: { type: "object", properties: {}, additionalProperties: false },
  },
  {
    name: "my_birds",
    description:
      "List the birds the user's own Ornigo listeners and phones have heard, newest first: English and Latin name, how many " +
      "15-minute reports included it, in how many places, first and last time heard, and a rare flag. Use since to narrow it " +
      "(\"today\", \"week\", \"month\", \"48h\", an ISO date or unix seconds). Read-only. When telling the user, lead with the " +
      "most recent or most interesting birds rather than reading the whole list.",
    inputSchema: {
      type: "object",
      properties: { since: { type: "string", description: "today | week | month | 48h | 3d | ISO date | unix seconds. Omit for all time." } },
      additionalProperties: false,
    },
  },
  {
    name: "is_back",
    description:
      "Has a particular bird come back? Give its common or Latin name (e.g. \"cardinal\", \"Strix varia\"). Answers from the " +
      "user's own listeners: when it was last heard, whether it was heard in the last 24 hours, and how long the gap was " +
      "before that. If several species match, it lists them so you can ask which one. Read-only.",
    inputSchema: {
      type: "object",
      properties: { name: { type: "string", description: "Common or Latin name of the bird (partial names are fine)." } },
      required: ["name"],
      additionalProperties: false,
    },
  },
];

function cleanRef(r) {
  const s = String(r || "").trim().toLowerCase();
  if (!s) return "";
  if (!/^[a-z0-9][a-z0-9._-]{0,31}$/.test(s)) throw new Error("ref must be lowercase letters, digits, . _ - (max 32)");
  return s;
}

async function installListener(args) {
  const ref = cleanRef(args.ref);
  const v = await fetchVerified();
  const head = [`Computer: ${v.os} ${arch()} (${release()}).`, ...v.steps,
    `Release: hone-listener ${v.manifest.version || "(no version)"}.`];
  if (!args.confirm) {
    return [...head, "", "Nothing has been run yet.", `Privacy note to read to the user: ${DISCLAIMER}`,
      "Ask the user if they want to install it now. If yes, call install_listener again with confirm: true" + (ref ? ` and ref: "${ref}".` : ".")].join("\n");
  }
  const dir = mkdtempSync(join(tmpdir(), "hone-listener-"));
  const path = join(dir, v.file);
  const manPath = join(dir, "version.json");
  writeFileSync(path, v.installer, { mode: 0o700 });
  writeFileSync(manPath, v.manifestBytes, { mode: 0o600 });
  const env = { ...process.env, HONE_REF: ref, HONE_VERIFIED_MANIFEST: manPath, HONE_INSTALL_SOURCE: "mcp", HONE_NONINTERACTIVE: "1" };
  const [cmd, cargs] = v.os === "win32"
    ? ["powershell.exe", ["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-File", path]]
    : ["/bin/sh", [path]];
  const out = await new Promise((resolve) => {
    let buf = "";
    const p = spawn(cmd, cargs, { env, stdio: ["ignore", "pipe", "pipe"], windowsHide: true });
    const add = (d) => { buf = (buf + d.toString()).slice(-6000); };
    p.stdout.on("data", add); p.stderr.on("data", add);
    const timer = setTimeout(() => { p.kill(); buf += "\n[stopped after 20 minutes]"; }, 20 * 60 * 1000);
    p.on("error", (e) => { clearTimeout(timer); resolve({ code: -1, buf: buf + "\n" + e.message }); });
    p.on("close", (code) => { clearTimeout(timer); resolve({ code, buf }); });
  });
  try { rmSync(dir, { recursive: true, force: true }); } catch { /* best effort */ }
  return [...head, `Ran the verified ${v.file}. Exit code ${out.code}${out.code === 0 ? " (success)" : " (failed)"}.`,
    "Installer output (last lines, from the signed installer):", out.buf.trim() || "(no output)", "",
    out.code === 0 ? "Next: if a setup page opened, the user signs in with email and picks their camera there. Then call listener_status."
      : "Tell the user it didn't finish and share the output above with shindevlin@proton.me if it keeps failing."].join("\n");
}

async function listenerStatus(args, ctx) {
  const parts = [];
  if (!ctx.remote) {
    try {
      const s = await localStatus();
      parts.push(`This computer (${s.os}): Ornigo ${s.installed ? (s.running ? "installed and running" : "installed but NOT running") : "not installed"}` +
        ` [service ${s.name}${s.state ? ", state " + s.state : ""}].` + (s.local_status ? ` Local status: ${JSON.stringify(s.local_status).slice(0, 400)}` : ""));
    } catch (e) { parts.push("This computer: couldn't check (" + e.message + ")."); }
  }
  let tok = null;
  try { tok = token(ctx.token); } catch (e) { parts.push(e.message); }
  if (tok) {
    const j = await api("/mine/listeners", tok);
    if (!j.listeners.length) parts.push("Account: no listeners or phones linked yet.");
    for (const d of j.listeners) {
      const name = d.label || (/camera|listener/.test(d.kind) ? "Camera listener" : d.kind === "phone" ? "Phone" : "Device");
      parts.push(`${name} (${d.kind}): ${d.online ? "Listening now" : "not listening"}; last report ${when(d.last_report)}` +
        (d.detections_today != null ? `; ${d.detections_today} bird calls today` : "") + (d.version ? `; version ${d.version}` : ""));
    }
  } else if (!ctx.remote) {
    parts.push("Account status not checked: no Ornigo key set (make one at https://honemesh.net/me).");
  }
  return parts.join("\n");
}

async function myBirds(args, ctx) {
  const since = parseSince(args.since);
  const j = await api("/mine/birds" + (since ? `?since=${since}` : ""), token(ctx.token));
  if (!j.species.length) return since ? "No birds heard in that time." : "No birds yet. Reports arrive every 15 minutes; dawn is the busiest time.";
  const lines = j.species.map((s) => `- ${s.name || s.latin} (${s.latin}): heard in ${s.heard} report${s.heard === 1 ? "" : "s"}, ` +
    `${s.places} place${s.places === 1 ? "" : "s"}; last ${when(s.last)}; first ${when(s.first)}` + (s.rare ? `; RARE here (${s.rare})` : ""));
  return `${j.count} species${since ? " since " + new Date(since * 1000).toISOString() : ""}:\n` + lines.join("\n") +
    "\nPhotos and calls: https://honemesh.net/me";
}

async function isBack(args, ctx) {
  const q = String(args.name || "").trim().toLowerCase();
  if (q.length < 2) throw new Error("Give a bird name, e.g. \"cardinal\".");
  const j = await api("/mine/birds", token(ctx.token));
  const all = j.species;
  let hits = all.filter((s) => (s.name || "").toLowerCase() === q || s.latin.toLowerCase() === q);
  if (!hits.length) hits = all.filter((s) => (s.name || "").toLowerCase().includes(q) || s.latin.toLowerCase().includes(q));
  if (!hits.length) return `No "${args.name}" heard by your listeners yet. ${all.length ? "Heard so far: " + all.slice(0, 12).map((s) => s.name || s.latin).join(", ") + "." : ""}`;
  if (hits.length > 1) return "Several birds match; ask the user which one:\n" + hits.map((s) => `- ${s.name || s.latin} (${s.latin}), last ${when(s.last)}`).join("\n");
  const s = hits[0];
  const now = Date.now() / 1000;
  const recent = now - s.last < 86400;
  // Gap before the latest visit, from the UTC days it was heard on.
  let gap = "";
  try {
    const d = await api("/mine/birds/" + encodeURIComponent(s.latin), token(ctx.token));
    const days = d.days || [];
    if (days.length > 1) {
      const g = Math.round((days[0] - days[1]) / 86400);
      gap = g > 1 ? ` Before this visit it had been away ${g} days.` : " It has been heard on consecutive days.";
    } else if (days.length === 1) gap = " This is the first day it was heard.";
    gap += ` Heard on ${days.length} day${days.length === 1 ? "" : "s"}, in ${d.count} place${d.count === 1 ? "" : "s"}.`;
  } catch { /* optional */ }
  return `${s.name || s.latin} (${s.latin}): ${recent ? "YES, it's around: heard in the last 24 hours" : "not heard in the last 24 hours"}. ` +
    `Last heard ${when(s.last)}; first heard ${when(s.first)}; in ${s.heard} report${s.heard === 1 ? "" : "s"}.${gap}`;
}

const HANDLERS = { install_listener: installListener, listener_status: listenerStatus, my_birds: myBirds, is_back: isBack };

// ── JSON-RPC (MCP) ─────────────────────────────────────────────────────────────────────
async function handle(msg, ctx) {
  const { id, method, params } = msg;
  const ok = (result) => ({ jsonrpc: "2.0", id, result });
  if (method === "initialize") {
    return ok({
      protocolVersion: params?.protocolVersion || "2025-06-18",
      capabilities: { tools: {} },
      serverInfo: { name: "hone-listener", version: VERSION },
      instructions: "Ornigo bird listener (made by Honemesh). Explain every step to the user in plain words. Never run the installer " +
        "before the user agrees; install_listener needs confirm: true for that. Privacy: " + DISCLAIMER,
    });
  }
  if (method === "ping") return ok({});
  if (method === "tools/list") {
    return ok({ tools: TOOLS.filter((t) => !(ctx.remote && t.local)).map(({ local, ...t }) => t) });
  }
  if (method === "tools/call") {
    const name = params?.name;
    const tool = TOOLS.find((t) => t.name === name);
    if (!tool || (ctx.remote && tool.local)) return { jsonrpc: "2.0", id, error: { code: -32602, message: `unknown tool ${name}` } };
    try {
      const text = await HANDLERS[name](params.arguments || {}, ctx);
      return ok({ content: [{ type: "text", text }] });
    } catch (e) {
      return ok({ content: [{ type: "text", text: e.message }], isError: true });
    }
  }
  if (id === undefined) return null; // notification
  return { jsonrpc: "2.0", id, error: { code: -32601, message: `method not found: ${method}` } };
}

function stdio() {
  let buf = "";
  const send = (o) => process.stdout.write(JSON.stringify(o) + "\n");
  process.stdin.setEncoding("utf8");
  process.stdin.on("data", (chunk) => {
    buf += chunk;
    let i;
    while ((i = buf.indexOf("\n")) >= 0) {
      const line = buf.slice(0, i).trim();
      buf = buf.slice(i + 1);
      if (!line) continue;
      let msg;
      try { msg = JSON.parse(line); } catch { send({ jsonrpc: "2.0", id: null, error: { code: -32700, message: "parse error" } }); continue; }
      handle(msg, { remote: false }).then((r) => r && send(r));
    }
  });
}

function http(port) {
  const srv = createServer(async (req, res) => {
    const reply = (code, obj, extra = {}) => {
      res.writeHead(code, { "Content-Type": "application/json", "Cache-Control": "no-store", ...extra });
      res.end(obj === undefined ? "" : JSON.stringify(obj));
    };
    const path = new URL(req.url, "http://x").pathname.replace(/\/+$/, "");
    if (req.method === "GET" && path.endsWith("/health")) return reply(200, { ok: true, version: VERSION });
    if (req.method !== "POST") return reply(405, { error: "POST JSON-RPC to this URL" }, { Allow: "POST" });
    let body = "";
    for await (const c of req) { body += c; if (body.length > 64 * 1024) return reply(413, { error: "too large" }); }
    const auth = String(req.headers.authorization || "");
    const ctx = { remote: true, token: auth.startsWith("Bearer ") ? auth.slice(7).trim() : null };
    let msg;
    try { msg = JSON.parse(body); } catch { return reply(400, { jsonrpc: "2.0", id: null, error: { code: -32700, message: "parse error" } }); }
    if (Array.isArray(msg)) {
      const out = (await Promise.all(msg.map((m) => handle(m, ctx)))).filter(Boolean);
      return out.length ? reply(200, out) : reply(202);
    }
    const r = await handle(msg, ctx);
    return r ? reply(200, r) : reply(202);
  });
  srv.listen(port, process.env.HONE_MCP_BIND || "127.0.0.1", () => process.stderr.write(`hone-listener MCP (read-only HTTP) on :${port}\n`));
}

const argv = process.argv.slice(2);
if (argv[0] === "--http") http(Number(argv[1] || process.env.PORT || 9461));
else if (argv[0] === "--version") console.log(VERSION);
else stdio();
