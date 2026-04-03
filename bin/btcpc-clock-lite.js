#!/usr/bin/env node
"use strict";

/**
 * btcpc-clock-lite — Zero-dependency clock node
 * Works on Node.js 14+. No npm install needed.
 * Uses built-in http/crypto modules for WebSocket.
 *
 * Usage: node btcpc-clock-lite.js
 */

var http = require("http");
var https = require("https");
var crypto = require("crypto");
var net = require("net");
var tls = require("tls");
var url = require("url");

var ACCOUNT = process.env.BTCPC_CLOCK_ACCOUNT || "josh";
var PORT = parseInt(process.env.P2P_PORT) || 6946;
var NODE_ID = crypto.randomBytes(16).toString("hex");
var RELAY = "wss://btcpc-relay.shindevlin.workers.dev/ws";
var SEEDS = (process.env.BTCPC_SEED_PEERS || "").split(",").filter(Boolean);

var currentEpoch = -1;
var connections = [];
var seen = new Set();

console.log("");
console.log("  btcpc-clock-lite");
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
      var ep = (msg.data || {}).epoch_number;
      var rw = ((msg.data || {}).block_reward || 0).toFixed(2);
      console.log("[CLOCK] Block " + ep + " | " + rw + " BTCPC");
      if (ep > currentEpoch) currentEpoch = ep;
      broadcast(msg, from);
      break;

    default:
      broadcast(msg, from);
  }
}

// ── Start ────────────────────────────────────────────────────

// Connect to relay and seeds
connectPeer(RELAY);
SEEDS.forEach(connectPeer);

// Heartbeat every 60s
setInterval(function () {
  broadcast(makeMsg("CLOCK_HEARTBEAT", { account: ACCOUNT, epoch_number: currentEpoch }));
}, 60000);

// Status every 2 min
setInterval(function () {
  console.log("[CLOCK] epoch " + currentEpoch + " | " + connections.length + " peers");
}, 120000);
