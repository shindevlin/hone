"use strict";

const dns = require("dns").promises;
const net = require("net");

function _normalizeHostname(hostname) {
  return String(hostname || "").trim().toLowerCase().replace(/\.$/, "");
}

function _ipv4ToInt(parts) {
  return parts.reduce((acc, part) => ((acc << 8) >>> 0) + part, 0) >>> 0;
}

function _decimalToIpv4(hostname) {
  if (!/^\d+$/.test(hostname)) return null;
  const value = Number(hostname);
  if (!Number.isSafeInteger(value) || value < 0 || value > 0xffffffff) return null;
  const parts = [
    (value >>> 24) & 0xff,
    (value >>> 16) & 0xff,
    (value >>> 8) & 0xff,
    value & 0xff,
  ];
  return parts.join(".");
}

function _hexToIpv4(hostname) {
  if (!/^0x[0-9a-f]+$/i.test(hostname)) return null;
  const value = Number.parseInt(hostname, 16);
  if (!Number.isFinite(value) || value < 0 || value > 0xffffffff) return null;
  const parts = [
    (value >>> 24) & 0xff,
    (value >>> 16) & 0xff,
    (value >>> 8) & 0xff,
    value & 0xff,
  ];
  return parts.join(".");
}

function _isPrivateIpv4(ip) {
  const parts = ip.split(".").map(n => Number(n));
  if (parts.length !== 4 || parts.some(n => !Number.isInteger(n) || n < 0 || n > 255)) {
    return true;
  }
  const [a, b] = parts;
  const value = _ipv4ToInt(parts);
  if (a === 0) return true;
  if (a === 10) return true;
  if (a === 127) return true;
  if (a === 169 && b === 254) return true;
  if (a === 172 && b >= 16 && b <= 31) return true;
  if (a === 192 && b === 168) return true;
  if (a === 100 && b >= 64 && b <= 127) return true;
  if (value >= _ipv4ToInt([198, 18, 0, 0]) && value <= _ipv4ToInt([198, 19, 255, 255])) return true;
  if (a >= 224) return true;
  return false;
}

function _isPrivateIpv6(ip) {
  const normalized = String(ip || "").toLowerCase();
  if (normalized === "::1") return true;
  if (normalized === "::") return true;
  if (normalized.startsWith("fe80:")) return true;
  if (normalized.startsWith("fc") || normalized.startsWith("fd")) return true;
  // IPv4-mapped (::ffff:x.y.z.w) and IPv4-compatible (::x.y.z.w) addresses
  // can bypass IPv4 private-range checks, so block them all.
  if (normalized.startsWith("::ffff:") || normalized.startsWith("::ffff:0:")) return true;
  return false;
}

function isPrivateIpLiteral(hostname) {
  const normalized = _normalizeHostname(hostname);
  if (!normalized) return true;
  if (normalized === "localhost" || normalized.endsWith(".localhost")) return true;

  const decimal = _decimalToIpv4(normalized);
  const hex = _hexToIpv4(normalized);
  const candidate = decimal || hex || normalized;
  const kind = net.isIP(candidate);
  if (kind === 4) return _isPrivateIpv4(candidate);
  if (kind === 6) return _isPrivateIpv6(candidate);
  return false;
}

async function isPublicHttpUrl(rawUrl) {
  try {
    const parsed = new URL(rawUrl);
    if (!["http:", "https:"].includes(parsed.protocol)) return false;

    // Node's URL parser returns IPv6 addresses with brackets (e.g. "[::1]").
    // Strip them so net.isIP and isPrivateIpLiteral receive a bare address.
    const rawHostname = _normalizeHostname(parsed.hostname);
    const hostname = rawHostname.startsWith("[") && rawHostname.endsWith("]")
      ? rawHostname.slice(1, -1)
      : rawHostname;
    if (!hostname) return false;
    if (isPrivateIpLiteral(hostname)) return false;

    if (net.isIP(hostname)) {
      return true;
    }

    const lookups = await dns.lookup(hostname, { all: true, verbatim: true });
    if (!lookups || lookups.length === 0) return false;
    for (const entry of lookups) {
      if (entry.family === 4 && _isPrivateIpv4(entry.address)) return false;
      if (entry.family === 6 && _isPrivateIpv6(entry.address)) return false;
    }
    return true;
  } catch (_) {
    return false;
  }
}

module.exports = {
  isPublicHttpUrl,
  isPrivateIpLiteral,
};
