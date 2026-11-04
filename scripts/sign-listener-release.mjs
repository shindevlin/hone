#!/usr/bin/env node
// Sign a HONE camera-listener release so the listener MCP (clients/hone-listener-mcp) will run it.
//
//   node scripts/sign-listener-release.mjs <release-dir> [--version X.Y.Z]
//
// <release-dir> holds what gets published at https://honemesh.net/app/listener/ (install.sh,
// install.ps1, binaries...). Writes <release-dir>/version.json (keeps any fields already there,
// sets name, version and files{<name>: {sha256, bytes}} for every other file) and
// <release-dir>/version.json.sig (base64 ed25519 over the exact version.json bytes).
// Key: $HONE_LISTENER_KEY or ~/.config/hone-release/listener-ed25519.pem (never printed).
import { createHash, createPrivateKey, createPublicKey, sign } from "node:crypto";
import { readFileSync, writeFileSync, readdirSync, statSync, existsSync } from "node:fs";
import { join } from "node:path";
import { homedir } from "node:os";

const PINNED = "27ef54588674c3391e660056d873b89136e75125b5a433de0747191baa94a10d";
const dir = process.argv[2];
if (!dir) { console.error("usage: sign-listener-release.mjs <release-dir> [--version X.Y.Z]"); process.exit(2); }
const vi = process.argv.indexOf("--version");
const keyPath = process.env.HONE_LISTENER_KEY || join(homedir(), ".config/hone-release/listener-ed25519.pem");
const key = createPrivateKey(readFileSync(keyPath));
const pub = createPublicKey(key).export({ type: "spki", format: "der" }).subarray(12).toString("hex");
if (pub !== PINNED) { console.error("refusing: this key is not the pinned HONE listener release key"); process.exit(1); }

const mpath = join(dir, "version.json");
const m = existsSync(mpath) ? JSON.parse(readFileSync(mpath, "utf8")) : {};
m.name = "hone-listener";
if (vi > 0) m.version = process.argv[vi + 1];
if (!m.version) { console.error("no version: pass --version"); process.exit(2); }
m.files = {};
for (const f of readdirSync(dir).sort()) {
  if (f === "version.json" || f.startsWith("version.json.") || f.startsWith(".")) continue;
  const p = join(dir, f);
  if (!statSync(p).isFile()) continue;
  const b = readFileSync(p);
  m.files[f] = { sha256: createHash("sha256").update(b).digest("hex"), bytes: b.length };
}
if (!m.files["install.sh"] && !m.files["install.ps1"]) { console.error("no install.sh or install.ps1 in " + dir); process.exit(1); }
m.signed = new Date().toISOString();
const bytes = Buffer.from(JSON.stringify(m, null, 2) + "\n");
writeFileSync(mpath, bytes);
writeFileSync(mpath + ".sig", sign(null, bytes, key).toString("base64") + "\n");
console.log(`signed hone-listener ${m.version}: ${Object.keys(m.files).length} files -> version.json + version.json.sig`);
