"use strict";

/**
 * URL content filter for browser agent jobs.
 * Shin Devlin
 *
 * HONE is censorship-resistant infrastructure. The filter's job is to protect
 * the MINER'S machine from attacks and illegal content — not to restrict what
 * buyers can do with their agent sessions.
 *
 * HARD BLOCKS (no config, always enforced — protect the miner's machine):
 *   • Private/loopback/cloud-metadata IPs — SSRF against miner's LAN
 *   • Non-http(s) protocols — file://, ftp://, data://, etc.
 *   • CSAM keyword patterns in hostname/path
 *
 * SOFT BLOCKS (miner opt-in, all default false — miner's machine, miner's choice):
 *   HONE_BROWSER_BLOCK_ONION=true|false      — Tor hidden services (default: false)
 *   HONE_BROWSER_BLOCK_ADULT=true|false      — adult content (default: false)
 *   HONE_BROWSER_BLOCK_GAMBLING=true|false   — gambling (default: false)
 *   HONE_BROWSER_BLOCK_DRUGS=true|false      — drug marketplaces (default: false)
 *   HONE_BROWSER_BLOCK_WEAPONS=true|false    — illegal weapons (default: false)
 *
 * ALLOWLIST (optional — restrict to specific domains only):
 *   HONE_BROWSER_ALLOWED_DOMAINS=example.com,myapp.io
 *
 * Note on .onion: Tor hidden services include SecureDrop, whistleblower platforms,
 * censored news outlets, and privacy tools. Blocking them by default contradicts
 * HONE's censorship-resistance mission. Miners who don't want onion traffic can
 * set HONE_BROWSER_BLOCK_ONION=true.
 */

const net = require("net");
const { URL } = require("url");

// ── Hard block: SSRF protection ───────────────────────────────────────────────

const PRIVATE_IP_PATTERNS = [
  /^127\./,
  /^10\./,
  /^172\.(1[6-9]|2[0-9]|3[01])\./,
  /^192\.168\./,
  /^169\.254\./,
  /^100\.(6[4-9]|[7-9]\d|1[01]\d|12[0-7])\./,
  /^0\./,
  /^::1$/,
  /^fc[\da-f]{2}:/i,
  /^fe80:/i,
];

const CLOUD_METADATA_HOSTS = new Set([
  "169.254.169.254",        // AWS / GCP / Azure instance metadata
  "metadata.google.internal",
  "metadata.internal",
  "169.254.170.2",          // AWS ECS metadata
  "100.100.100.200",        // Alibaba Cloud metadata
]);

// ── Hard block: CSAM keyword patterns ────────────────────────────────────────
// Applied to hostname + path. Not exhaustive — supplements other protections.

const CSAM_PATTERNS = [
  /\bpedo\b/i,
  /childporn/i,
  /child[\s_-]?porn/i,
  /kiddie[\s_-]?porn/i,
  /underage[\s_-]?sex/i,
  /preteen[\s_-]?sex/i,
];

// ── Soft block: miner-configurable category signals ───────────────────────────

const ADULT_SIGNALS = [
  /pornhub/i, /xvideos/i, /xhamster/i, /redtube/i, /youporn/i,
  /brazzers/i, /onlyfans/i, /chaturbate/i, /livejasmin/i, /stripchat/i,
  /bangbros/i, /naughtyamerica/i,
  /\bporn\b/i, /\bxxx\b/i,
];

const GAMBLING_SIGNALS = [
  /betway/i, /bet365/i, /draftkings/i, /fanduel/i, /bovada/i,
  /pokerstars/i, /partypoker/i, /888casino/i, /betmgm/i,
  /\bcasino\b/i, /\bsportsbet\b/i,
];

const DRUGS_SIGNALS = [
  /silkroad/i, /darkmarket/i, /\bbuycocaine\b/i, /\bbuymeth\b/i,
];

const WEAPONS_SIGNALS = [
  /\billegal[\s_-]?weapons\b/i, /\bbuy[\s_-]?illegal[\s_-]?gun/i,
];

// ── Helpers ───────────────────────────────────────────────────────────────────

function _getAllowedDomains() {
  const raw = (process.env.HONE_BROWSER_ALLOWED_DOMAINS || "").trim();
  if (!raw) return null;
  return new Set(raw.split(",").map((d) => d.trim().toLowerCase()).filter(Boolean));
}

class BrowserContentError extends Error {
  constructor(reason, category, url) {
    super(reason);
    this.name = "BrowserContentError";
    this.category = category;
    this.url = url;
  }
}

// ── Main check ────────────────────────────────────────────────────────────────

function checkUrl(rawUrl) {
  let parsed;
  try {
    parsed = new URL(rawUrl);
  } catch (_) {
    return { allowed: false, reason: "Invalid URL", category: "invalid" };
  }

  const hostname = parsed.hostname.toLowerCase();
  const full = hostname + parsed.pathname.toLowerCase();

  // Only http/https (and onion-over-http is still http — handled below)
  if (parsed.protocol !== "http:" && parsed.protocol !== "https:") {
    return { allowed: false, reason: `Protocol '${parsed.protocol}' not permitted — only http/https`, category: "protocol" };
  }

  // Hard block: cloud metadata (SSRF)
  if (CLOUD_METADATA_HOSTS.has(hostname)) {
    return { allowed: false, reason: "Cloud metadata endpoint blocked (SSRF protection)", category: "ssrf" };
  }

  // Hard block: private / loopback IPs (SSRF)
  if (net.isIP(hostname)) {
    for (const re of PRIVATE_IP_PATTERNS) {
      if (re.test(hostname)) {
        return { allowed: false, reason: "Private/loopback IP blocked (SSRF protection)", category: "ssrf" };
      }
    }
  }

  // Hard block: CSAM patterns
  for (const re of CSAM_PATTERNS) {
    if (re.test(full)) {
      return { allowed: false, reason: "URL matches prohibited content pattern", category: "csam" };
    }
  }

  // Allowlist mode — if set, only listed domains are ever permitted
  const allowlist = _getAllowedDomains();
  if (allowlist !== null) {
    const apex = hostname.replace(/^www\./, "");
    const matched = allowlist.has(hostname) || allowlist.has(apex) ||
      Array.from(allowlist).some((d) => hostname.endsWith("." + d));
    if (!matched) {
      return { allowed: false, reason: `Domain not in miner's allowed-domains list: ${hostname}`, category: "allowlist" };
    }
  }

  // Soft block: .onion (default: allow — censorship resistance)
  if (hostname.endsWith(".onion") || hostname.endsWith(".i2p")) {
    if (process.env.HONE_BROWSER_BLOCK_ONION === "true") {
      return { allowed: false, reason: "Tor/I2P hidden services blocked by miner policy", category: "onion" };
    }
  }

  // Soft blocks — all default false
  const blockAdult    = process.env.HONE_BROWSER_BLOCK_ADULT    === "true";
  const blockGambling = process.env.HONE_BROWSER_BLOCK_GAMBLING === "true";
  const blockDrugs    = process.env.HONE_BROWSER_BLOCK_DRUGS    === "true";
  const blockWeapons  = process.env.HONE_BROWSER_BLOCK_WEAPONS  === "true";

  if (blockAdult)    { for (const re of ADULT_SIGNALS)    { if (re.test(hostname)) return { allowed: false, reason: "Adult content blocked by miner policy", category: "adult" }; } }
  if (blockGambling) { for (const re of GAMBLING_SIGNALS) { if (re.test(hostname)) return { allowed: false, reason: "Gambling site blocked by miner policy", category: "gambling" }; } }
  if (blockDrugs)    { for (const re of DRUGS_SIGNALS)    { if (re.test(hostname)) return { allowed: false, reason: "Drug marketplace blocked by miner policy", category: "drugs" }; } }
  if (blockWeapons)  { for (const re of WEAPONS_SIGNALS)  { if (re.test(hostname)) return { allowed: false, reason: "Weapons site blocked by miner policy", category: "weapons" }; } }

  return { allowed: true, reason: "OK", category: null };
}

function assertUrlAllowed(url) {
  const result = checkUrl(url);
  if (!result.allowed) {
    throw new BrowserContentError(result.reason, result.category, url);
  }
}

function getContentPolicy() {
  return {
    hard_blocks: ["csam", "ssrf", "private_ips", "non_http"],
    block_onion:    process.env.HONE_BROWSER_BLOCK_ONION    === "true",
    block_adult:    process.env.HONE_BROWSER_BLOCK_ADULT    === "true",
    block_gambling: process.env.HONE_BROWSER_BLOCK_GAMBLING === "true",
    block_drugs:    process.env.HONE_BROWSER_BLOCK_DRUGS    === "true",
    block_weapons:  process.env.HONE_BROWSER_BLOCK_WEAPONS  === "true",
    allowlist_mode: !!process.env.HONE_BROWSER_ALLOWED_DOMAINS,
    allowed_domains: process.env.HONE_BROWSER_ALLOWED_DOMAINS
      ? process.env.HONE_BROWSER_ALLOWED_DOMAINS.split(",").map((d) => d.trim())
      : null,
  };
}

module.exports = { checkUrl, assertUrlAllowed, getContentPolicy, BrowserContentError };
