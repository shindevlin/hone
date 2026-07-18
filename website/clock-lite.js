#!/usr/bin/env node
"use strict";

/**
 * hone-clock-lite — Zero-dependency clock node
 * Works on Node.js 14+. No npm install needed.
 * Uses built-in http/crypto modules for WebSocket.
 *
 * Usage: node hone-clock-lite.js
 */

var http = require("http");
var https = require("https");
var crypto = require("crypto");
var net = require("net");
var tls = require("tls");
var url = require("url");

var ACCOUNT = process.env.HONE_CLOCK_ACCOUNT || "josh";
var PORT = parseInt(process.env.P2P_PORT) || 6946;
var NODE_ID = crypto.randomBytes(16).toString("hex");
var RELAY = "wss://hone-relay.shindevlin.workers.dev/ws";
var SEEDS = (process.env.HONE_SEED_PEERS || "").split(",").filter(Boolean);

var currentEpoch = -1;
var connections = [];
var seen = new Set();

console.log("");
console.log("  hone-clock-lite");
console.log("  Account: " + ACCOUNT);
console.log("  Node: " + NODE_ID.slice(0, 12));
console.log("  Port: " + PORT);
console.log("");

// ── Minimal WebSocket client (RFC 6455) ──────────────────────

function wsConnect(addr, onMessage, onClose) {
  var parsed = new url.URL(addr);
  var useTLS = parsed.protocol === "wss:";
  var port = parsed.port || (useTLS ? 443 : 80);
  var path = parsed.pathname || "/";
  var key = crypto.randomBytes(16).toString("base64");

  var opts = { host: parsed.hostname, port: port, servername: parsed.hostname };
  var socket = useTLS ? tls.connect(opts) : net.connect(opts);

  var connected = false;
  var buffer = Buffer.alloc(0);

  socket.on("connect", function () {
    socket.write(
      "GET " + path + " HTTP/1.1\r\n" +
      "Host: " + parsed.hostname + "\r\n" +
      "Upgrade: websocket\r\n" +
      "Connection: Upgrade\r\n" +
      "Sec-WebSocket-Key: " + key + "\r\n" +
      "Sec-WebSocket-Version: 13\r\n\r\n"
    );
  });

  if (useTLS) {
    socket.on("secureConnect", function () {
      socket.write(
        "GET " + path + " HTTP/1.1\r\n" +
        "Host: " + parsed.hostname + "\r\n" +
        "Upgrade: websocket\r\n" +
        "Connection: Upgrade\r\n" +
        "Sec-WebSocket-Key: " + key + "\r\n" +
        "Sec-WebSocket-Version: 13\r\n\r\n"
      );
    });
  }

  socket.on("data", function (chunk) {
    buffer = Buffer.concat([buffer, chunk]);

    if (!connected) {
      var headerEnd = buffer.indexOf("\r\n\r\n");
      if (headerEnd === -1) return;
      var header = buffer.slice(0, headerEnd).toString();
      if (header.indexOf("101") === -1) {
        socket.destroy();
        return;
      }
      connected = true;
      buffer = buffer.slice(headerEnd + 4);
    }

    // Parse WebSocket frames
    while (buffer.length >= 2) {
      var b0 = buffer[0];
      var b1 = buffer[1];
      var opcode = b0 & 0x0F;
      var masked = (b1 & 0x80) !== 0;
      var len = b1 & 0x7F;
      var offset = 2;

      if (len === 126) {
        if (buffer.length < 4) return;
        len = buffer.readUInt16BE(2);
        offset = 4;
      } else if (len === 127) {
        if (buffer.length < 10) return;
        len = buffer.readUInt32BE(6); // ignore high 4 bytes
        offset = 10;
      }

      if (masked) offset += 4;
      if (buffer.length < offset + len) return;

      var payload = buffer.slice(offset, offset + len);
      if (masked) {
        var mask = buffer.slice(offset - 4, offset);
        for (var i = 0; i < payload.length; i++) payload[i] ^= mask[i % 4];
      }

      buffer = buffer.slice(offset + len);

      if (opcode === 1) { // text frame
        try { onMessage(payload.toString()); } catch (e) {}
      } else if (opcode === 8) { // close
        socket.destroy();
        return;
      } else if (opcode === 9) { // ping
        wsSendRaw(socket, 10, payload); // pong
      }
    }
  });

  socket.on("error", function () {});
  socket.on("close", function () { if (onClose) onClose(); });

  return {
    send: function (data) {
      if (!connected) return;
      wsSendRaw(socket, 1, Buffer.from(data));
    },
    close: function () { socket.destroy(); },
    socket: socket
  };
}

function wsSendRaw(socket, opcode, payload) {
  var mask = crypto.randomBytes(4);
  var len = payload.length;
  var header;

  if (len < 126) {
    header = Buffer.alloc(6);
    header[0] = 0x80 | opcode;
    header[1] = 0x80 | len;
    mask.copy(header, 2);
  } else if (len < 65536) {
    header = Buffer.alloc(8);
    header[0] = 0x80 | opcode;
    header[1] = 0x80 | 126;
    header.writeUInt16BE(len, 2);
    mask.copy(header, 4);
  } else {
    header = Buffer.alloc(14);
    header[0] = 0x80 | opcode;
    header[1] = 0x80 | 127;
    header.writeUInt32BE(0, 2);
    header.writeUInt32BE(len, 6);
    mask.copy(header, 10);
  }

  var masked = Buffer.alloc(payload.length);
  for (var i = 0; i < payload.length; i++) masked[i] = payload[i] ^ mask[i % 4];

  try { socket.write(Buffer.concat([header, masked])); } catch (e) {}
}

// ── P2P layer ────────────────────────────────────────────────

function addConnection(ws, addr) {
  connections.push({ ws: ws, addr: addr });
}

function removeConnection(addr) {
  connections = connections.filter(function (c) { return c.addr !== addr; });
}

function broadcast(msg, exclude) {
  var data = JSON.stringify(msg);
  connections.forEach(function (c) {
    if (c.addr !== exclude) c.ws.send(data);
  });
}

function makeMsg(type, data) {
  return { type: type, data: data, nodeId: NODE_ID, id: crypto.randomBytes(8).toString("hex"), timestamp: Date.now() };
}

function connectPeer(addr) {
  var ws = wsConnect(addr,
    function (data) {
      try { handleMessage(JSON.parse(data), addr); } catch (e) {}
    },
    function () {
      removeConnection(addr);
      // Reconnect after delay
      setTimeout(function () { connectPeer(addr); }, 10000);
    }
  );

  addConnection(ws, addr);

  // Send handshake after brief delay
  setTimeout(function () {
    ws.send(JSON.stringify(makeMsg("HANDSHAKE", { chainHeight: currentEpoch, version: "2.0.79" })));
    console.log("[CLOCK] Connected: " + addr.slice(0, 40));
  }, 1000);
}

// ── Message handling ─────────────────────────────────────────

function handleMessage(msg, from) {
  if (!msg || !msg.type || msg.nodeId === NODE_ID) return;
  if (msg.id && seen.has(msg.id)) return;
  if (msg.id) {
    seen.add(msg.id);
    if (seen.size > 3000) {
      var it = seen.values();
      for (var i = 0; i < 300; i++) seen.delete(it.next().value);
    }
  }

  switch (msg.type) {
    case "HANDSHAKE":
      var v = (msg.data || {}).version || "?";
      var h = (msg.data || {}).chainHeight || 0;
      console.log("[CLOCK] Peer " + (msg.nodeId || "?").slice(0, 12) + " v" + v + " h=" + h);
      break;

    case "EPOCH_START":
      var e = (msg.data || {}).epoch_number;
      if (e > currentEpoch) {
        currentEpoch = e;
        console.log("[CLOCK] Epoch " + e + " START");
      }
      broadcast(msg, from);
      break;

    case "EPOCH_END":
      broadcast(msg, from);
      break;

    case "EPOCH_FINALIZED":
    case "BLOCK_PROPOSAL":
      var ep = (msg.data || {}).epoch_number;
      var rw = ((msg.data || {}).block_reward || 0).toFixed(2);
      console.log("[CLOCK] Block " + ep + " | " + rw + " HONE");
      if (ep > currentEpoch) currentEpoch = ep;
      broadcast(msg, from);
      break;

    case "VERIFY_REQUEST":
      handleVerifyRequest(msg);
      broadcast(msg, from);
      break;

    default:
      broadcast(msg, from);
  }
}

// ── Lightweight inline verifier ──
// Pure text analysis — no Ollama needed. This lets clock-only nodes
// (like phones running Termux) participate in the verifier panel.
function lightVerify(work) {
  var response = work.response || "";
  var tokenCount = work.token_count || 0;
  var elapsedMs = work.elapsed_ms || 0;
  var reasons = [];
  var score = 100;

  if (tokenCount < 1) { reasons.push("zero tokens"); score -= 50; }
  if (response.length < 10) { reasons.push("response too short"); score -= 30; }

  if (elapsedMs > 0 && tokenCount > 0) {
    var tps = tokenCount / (elapsedMs / 1000);
    if (tps > 5000) { reasons.push("unrealistic speed: " + tps.toFixed(0) + " tok/s"); score -= 50; }
  }

  // Garbage check
  var printable = (response.match(/[\x20-\x7E\n\r\t]/g) || []).length;
  if (printable / Math.max(response.length, 1) < 0.8) { reasons.push("low printable ratio"); score -= 30; }

  // Repetition
  var words = response.toLowerCase().split(/\s+/).filter(Boolean);
  if (words.length > 20 && new Set(words).size < 5) { reasons.push("excessive repetition"); score -= 30; }

  return { valid: score >= 50, score: Math.max(0, score), reasons: reasons };
}

function handleVerifyRequest(msg) {
  var data = msg.data || {};
  if (!data.job_id || !data.miner) return;
  if (data.miner === ACCOUNT) return; // don't verify own work

  // Simple deterministic selection: hash(job_id + ACCOUNT) mod K
  // We accept any verification we can perform — the network's selectVerifiers
  // will pick canonical winners but accepting from anyone strengthens it.
  var verdict = lightVerify({
    response: data.result || "",
    model: data.model || "",
    token_count: data.token_count || 0,
    elapsed_ms: data.timing_ms || 0,
  });

  console.log("[CLOCK] Verified " + data.job_id.slice(0, 12) + " for " + data.miner + " — " + (verdict.valid ? "VALID" : "INVALID") + " (" + verdict.score + ")");

  var resp = makeMsg("VERIFY_RESPONSE", {
    job_id: data.job_id,
    verifier: ACCOUNT,
    verdict: verdict.valid ? "valid" : "invalid",
    score: verdict.score,
    reasons: verdict.reasons,
    epoch: data.epoch || currentEpoch,
  });
  broadcast(resp);
}

// ── Start ────────────────────────────────────────────────────

// Connect to relay and seeds
connectPeer(RELAY);
SEEDS.forEach(connectPeer);

// HTTP heartbeat fallback — POSTs to /public/clock-heartbeat so the API server's
// browser-clock tracker counts this node even if its P2P relay isn't connected.
var CLIENT_ID = crypto.randomBytes(16).toString("hex");
var HEARTBEAT_URL = process.env.HONE_HEARTBEAT_URL || "https://honemesh.net/public/clock-heartbeat";

function httpHeartbeat() {
  try {
    var parsed = url.parse(HEARTBEAT_URL);
    var data = JSON.stringify({ account: ACCOUNT, client_id: CLIENT_ID });
    var lib = parsed.protocol === "https:" ? https : http;
    var req = lib.request({
      hostname: parsed.hostname,
      port: parsed.port || (parsed.protocol === "https:" ? 443 : 80),
      path: parsed.path,
      method: "POST",
      headers: {
        "Content-Type": "application/json",
        "Content-Length": Buffer.byteLength(data),
      },
      timeout: 10000,
    }, function (res) {
      var body = "";
      res.on("data", function (chunk) { body += chunk; });
      res.on("end", function () {
        if (res.statusCode === 200) {
          try {
            var json = JSON.parse(body);
            console.log("[CLOCK] HTTP heartbeat ok | epoch " + json.epoch + " | " + json.active_browser_clocks + " clocks");
          } catch (_) {}
        } else {
          console.log("[CLOCK] HTTP heartbeat returned " + res.statusCode);
        }
      });
    });
    req.on("error", function (err) {
      console.log("[CLOCK] HTTP heartbeat error: " + err.message);
    });
    req.write(data);
    req.end();
  } catch (e) {
    console.log("[CLOCK] HTTP heartbeat threw: " + e.message);
  }
}

// Heartbeat every 60s — both via P2P AND via HTTP fallback
setInterval(function () {
  broadcast(makeMsg("CLOCK_HEARTBEAT", { account: ACCOUNT, epoch_number: currentEpoch }));
  httpHeartbeat();
}, 60000);

// First HTTP heartbeat after 5s
setTimeout(httpHeartbeat, 5000);

// ── Auto-updater ──
// Check honemesh.net every hour for a newer clock-lite.js. If the SHA256 differs,
// download it to disk and exit (runit/systemd will restart the service).
var UPDATE_URL = process.env.HONE_UPDATE_URL || "https://honemesh.net/clock-lite.js";
var UPDATE_INTERVAL_MS = parseInt(process.env.HONE_UPDATE_INTERVAL_MS, 10) || 60 * 60 * 1000;
var SCRIPT_PATH = process.argv[1]; // path to this script

function selfHash() {
  try {
    var fs = require("fs");
    var contents = fs.readFileSync(SCRIPT_PATH);
    return crypto.createHash("sha256").update(contents).digest("hex");
  } catch (e) {
    return null;
  }
}

function checkForUpdate() {
  try {
    var parsed = url.parse(UPDATE_URL);
    var lib = parsed.protocol === "https:" ? https : http;
    var req = lib.request({
      hostname: parsed.hostname,
      port: parsed.port || (parsed.protocol === "https:" ? 443 : 80),
      path: parsed.path,
      method: "GET",
      timeout: 30000,
    }, function (res) {
      if (res.statusCode !== 200) {
        console.log("[CLOCK] Update check returned " + res.statusCode);
        return;
      }
      var chunks = [];
      res.on("data", function (chunk) { chunks.push(chunk); });
      res.on("end", function () {
        try {
          var newContents = Buffer.concat(chunks);
          var newHash = crypto.createHash("sha256").update(newContents).digest("hex");
          var currentHash = selfHash();
          if (currentHash && newHash !== currentHash) {
            console.log("[CLOCK] New version available (" + newHash.slice(0, 12) + " vs current " + currentHash.slice(0, 12) + "), updating...");
            var fs = require("fs");
            fs.writeFileSync(SCRIPT_PATH + ".tmp", newContents);
            fs.renameSync(SCRIPT_PATH + ".tmp", SCRIPT_PATH);
            console.log("[CLOCK] Update complete, exiting for restart");
            // runit/systemd will restart us
            setTimeout(function () { process.exit(0); }, 1000);
          } else {
            console.log("[CLOCK] Up to date (" + (currentHash || "unknown").slice(0, 12) + ")");
          }
        } catch (e) {
          console.log("[CLOCK] Update apply error: " + e.message);
        }
      });
    });
    req.on("error", function (err) {
      console.log("[CLOCK] Update check error: " + err.message);
    });
    req.on("timeout", function () {
      req.destroy();
      console.log("[CLOCK] Update check timeout");
    });
    req.end();
  } catch (e) {
    console.log("[CLOCK] Update check threw: " + e.message);
  }
}

// First update check after 5 minutes (don't hammer right after install)
setTimeout(checkForUpdate, 5 * 60 * 1000);
setInterval(checkForUpdate, UPDATE_INTERVAL_MS);

// Status every 2 min
setInterval(function () {
  console.log("[CLOCK] epoch " + currentEpoch + " | " + connections.length + " peers");
}, 120000);
