"use strict";

const os = require("os");
const net = require("net");

const DEFAULT_PORT = 6942;

function _defaultPort() {
  const parsed = parseInt(process.env.P2P_PORT || process.env.HONE_API_P2P_PORT || DEFAULT_PORT, 10);
  return Number.isFinite(parsed) && parsed > 0 ? parsed : DEFAULT_PORT;
}

function _normalize(raw) {
  if (typeof raw !== "string") return null;
  const value = raw.trim();
  return value ? value : null;
}

function _decimalToIpv4(hostname) {
  if (!/^\d+$/.test(hostname)) return null;
  const value = Number(hostname);
  if (!Number.isSafeInteger(value) || value < 0 || value > 0xffffffff) return null;
  return [
    (value >>> 24) & 0xff,
    (value >>> 16) & 0xff,
    (value >>> 8) & 0xff,
    value & 0xff,
  ].join(".");
}

function _hexToIpv4(hostname) {
  if (!/^0x[0-9a-f]+$/i.test(hostname)) return null;
  const value = Number.parseInt(hostname, 16);
  if (!Number.isFinite(value) || value < 0 || value > 0xffffffff) return null;
  return [
    (value >>> 24) & 0xff,
    (value >>> 16) & 0xff,
    (value >>> 8) & 0xff,
    value & 0xff,
  ].join(".");
}

function _isIpv4MappedLocalOnly(hostname) {
  const lowered = String(hostname || "").toLowerCase().replace(/^\[/, "").replace(/\]$/, "");
  const prefixes = ["::ffff:", "0:0:0:0:0:ffff:"];

  for (const prefix of prefixes) {
    if (!lowered.startsWith(prefix)) continue;
    const rest = lowered.slice(prefix.length);
    if (rest.includes(".")) {
      return _isLocalOnlyHost(rest);
    }

    const parts = rest.split(":").filter(Boolean);
    if (parts.length === 2 && parts.every(part => /^[0-9a-f]{1,4}$/i.test(part))) {
      const high = Number.parseInt(parts[0], 16);
      const low = Number.parseInt(parts[1], 16);
      const a = (high >>> 8) & 0xff;
      const b = high & 0xff;
      if (a === 0 || a === 127) return true;
      if (a === 169 && b === 254) return true;
      return false;
    }
  }

  return false;
}

function _isLocalOnlyHost(hostname) {
  const host = _normalize(hostname);
  if (!host) return true;

  const lowered = host.toLowerCase().replace(/^\[/, "").replace(/\]$/, "").replace(/\.$/, "");
  if (lowered === "localhost" || lowered.endsWith(".localhost")) return true;
  if (lowered === "0.0.0.0" || lowered === "::" || lowered === "[::]") return true;
  if (lowered === "127.0.0.1" || lowered === "::1") return true;
  if (_isIpv4MappedLocalOnly(lowered)) return true;

  const decimal = _decimalToIpv4(lowered);
  const hex = _hexToIpv4(lowered);
  const candidate = decimal || hex || lowered;
  const kind = net.isIP(candidate);
  if (kind === 4) {
    const parts = candidate.split(".").map(Number);
    const [a, b] = parts;
    if (a === 0) return true;
    if (a === 127) return true;
    if (a === 169 && b === 254) return true;
    return false;
  }
  if (kind === 6) {
    return candidate === "::1" || candidate === "::";
  }
  return false;
}

function normalizeP2PAddress(raw) {
  const value = _normalize(raw);
  if (!value) return null;
  if (value.startsWith("inbound:")) return null;

  let normalized = value;
  if (!/^wss?:\/\//i.test(normalized)) {
    normalized = "ws://" + normalized;
  }

  try {
    const url = new URL(normalized);
    if (!["ws:", "wss:"].includes(url.protocol)) return null;
    if (_isLocalOnlyHost(url.hostname)) return null;
    if (!url.port && url.pathname === "/") {
      url.port = String(_defaultPort());
    }
    return url.toString().replace(/\/$/, "");
  } catch (_) {
    return null;
  }
}

function isConnectableP2PAddress(raw) {
  return !!normalizeP2PAddress(raw);
}

function getAdvertisedP2PAddress() {
  const port = _defaultPort();

  const envAddress = normalizeP2PAddress(process.env.HONE_PUBLIC_ADDRESS);
  if (envAddress) return envAddress;

  const envHost = _normalize(process.env.P2P_ADVERTISE_IP);
  if (envHost) {
    const explicit = normalizeP2PAddress(envHost);
    if (explicit) return explicit;
    try {
      const url = new URL(envHost.includes("://") ? envHost : `ws://${envHost}`);
      if (!_isLocalOnlyHost(url.hostname)) {
        return url.toString().replace(/\/$/, "");
      }
    } catch (_) {}
  }

  const nets = os.networkInterfaces();
  for (const name of Object.keys(nets)) {
    for (const iface of nets[name] || []) {
      if (!iface || iface.internal) continue;
      if (!iface.address || _isLocalOnlyHost(iface.address)) continue;
      if (iface.family === "IPv6" || iface.family === 6) {
        return `ws://[${iface.address}]:${port}`;
      }
      return `ws://${iface.address}:${port}`;
    }
  }

  return null;
}

module.exports = {
  getAdvertisedP2PAddress,
  isConnectableP2PAddress,
  normalizeP2PAddress,
};
