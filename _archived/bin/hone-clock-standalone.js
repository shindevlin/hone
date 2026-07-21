#!/usr/bin/env node
"use strict";

/**
 * hone-clock-standalone — Single-file clock node
 * No repo clone needed. Just: node hone-clock-standalone.js
 *
 * Requires: npm install ws
 */

var WebSocket = require("ws");
var crypto = require("crypto");
var fs = require("fs");

var ACCOUNT = process.env.HONE_CLOCK_ACCOUNT || "josh";
var PORT = parseInt(process.env.P2P_PORT) || 6946;
var NODE_ID = crypto.randomBytes(16).toString("hex");
var RELAY = process.env.HONE_RELAY_URL || "wss://hone-relay.shindevlin.workers.dev/ws";
var SEEDS = (process.env.HONE_SEED_PEERS || "").split(",").filter(Boolean);
var VERSION = "2.0.79";

var currentEpoch = -1;
var peers = new Map();
var seen = new Set();

console.log("");
console.log("  hone-clock — " + ACCOUNT);
console.log("  Node: " + NODE_ID.slice(0, 12) + "...");
console.log("  Port: " + PORT);
console.log("");

// P2P server
var wss = new WebSocket.Server({ port: PORT });
wss.on("connection", function (ws, req) {
  var addr = "in:" + (req.socket.remoteAddress || "").replace("::ffff:", "") + ":" + req.socket.remotePort;
  setup(ws, addr);
  handshake(ws);
});
console.log("[CLOCK] Listening on port " + PORT);

// Connect to seeds + relay
SEEDS.forEach(connect);
if (RELAY) connect(RELAY);

function connect(addr) {
  if (!addr || peers.has(addr) || addr.startsWith("in:")) return;
  try {
    var ws = new WebSocket(addr);
    ws.on("open", function () { setup(ws, addr); handshake(ws); console.log("[CLOCK] Connected to " + addr); });
    ws.on("error", function () {
      var p = peers.get(addr) || { attempts: 0 };
      var delay = Math.min(2000 * Math.pow(2, p.attempts), 300000);
      peers.delete(addr);
      setTimeout(function () { connect(addr); }, delay);
      p.attempts = (p.attempts || 0) + 1;
      peers.set(addr, p);
    });
  } catch (e) {}
}

function setup(ws, addr) {
  peers.set(addr, { ws: ws, attempts: 0 });
  ws.on("message", function (data) {
    try { handle(JSON.parse(data.toString()), addr); } catch (e) {}
  });
  ws.on("close", function () { peers.delete(addr); });
}

function handshake(ws) {
  send(ws, { type: "HANDSHAKE", data: { chainHeight: currentEpoch, version: VERSION }, nodeId: NODE_ID, id: crypto.randomBytes(8).toString("hex"), timestamp: Date.now() });
}

function send(ws, msg) {
  if (ws && ws.readyState === WebSocket.OPEN) ws.send(JSON.stringify(msg));
}

function broadcast(msg, exclude) {
  var d = JSON.stringify(msg);
  peers.forEach(function (p, a) { if (a !== exclude && p.ws && p.ws.readyState === WebSocket.OPEN) p.ws.send(d); });
}

function handle(msg, from) {
  if (!msg || !msg.type || msg.nodeId === NODE_ID) return;
  if (msg.id && seen.has(msg.id)) return;
  if (msg.id) { seen.add(msg.id); if (seen.size > 5000) { var it = seen.values(); for (var i = 0; i < 500; i++) seen.delete(it.next().value); } }

  switch (msg.type) {
    case "HANDSHAKE":
      var v = (msg.data || {}).version || "?";
      var h = (msg.data || {}).chainHeight || 0;
      console.log("[CLOCK] Peer " + (msg.nodeId || "?").slice(0, 12) + " v" + v + " height " + h);
      break;
    case "EPOCH_START":
      var e = (msg.data || {}).epoch_number;
      if (e > currentEpoch) { currentEpoch = e; console.log("[CLOCK] Epoch " + e + " START"); }
      broadcast(msg, from);
      break;
    case "EPOCH_END":
      console.log("[CLOCK] Epoch " + ((msg.data || {}).epoch_number || "?") + " END");
      broadcast(msg, from);
      break;
    case "EPOCH_FINALIZED":
      var ep = (msg.data || {}).epoch_number;
      var rw = ((msg.data || {}).block_reward || 0).toFixed(2);
      var mn = ((msg.data || {}).rewards || []).length;
      console.log("[CLOCK] Block " + ep + " | " + rw + " HONE | " + mn + " miner(s)");
      if (ep > currentEpoch) currentEpoch = ep;
      broadcast(msg, from);
      break;
    default:
      broadcast(msg, from);
  }
}

// Heartbeat every 60s
setInterval(function () {
  broadcast({ type: "CLOCK_HEARTBEAT", data: { account: ACCOUNT, epoch_number: currentEpoch, timestamp: Date.now() }, nodeId: NODE_ID, id: crypto.randomBytes(8).toString("hex"), timestamp: Date.now() });
}, 60000);

// Status every 2 min
setInterval(function () {
  var c = 0; peers.forEach(function (p) { if (p.ws && p.ws.readyState === WebSocket.OPEN) c++; });
  console.log("[CLOCK] epoch " + currentEpoch + " | " + c + " peers");
}, 120000);
