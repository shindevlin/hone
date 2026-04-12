"use strict";

/**
 * BTCPC P2P Network Manager
 * Shin Devlin
 *
 * WebSocket-based peer-to-peer network layer for the BTCPC sovereign chain.
 * Handles peer discovery, connection management, message broadcasting,
 * and auto-reconnection with exponential backoff.
 */

const WebSocket = require("ws");
const crypto = require("crypto");
const { handleMessage, createHandshake } = require("./protocol");

// Node identity — generated once on first start, persisted in env or memory
const NODE_ID = process.env.BTCPC_NODE_ID || crypto.randomBytes(16).toString("hex");

const DEFAULT_PORT = 6942;
const MAX_PEERS = 50;
const HEARTBEAT_INTERVAL_MS = 30000;
const MAX_RECONNECT_DELAY_MS = 300000; // 5 minutes

/**
 * Peer connection tracking.
 * Map<address, { ws, nodeId, chainHeight, status, reconnectAttempts, reconnectTimer }>
 */
const peers = new Map();
const messageHandlers = [];
let wss = null;
let heartbeatTimer = null;

// ---------------------------------------------------------------------------
// Server
// ---------------------------------------------------------------------------

/**
 * Start the WebSocket server to accept incoming peer connections.
 */
function startServer(port) {
  const listenPort = port || parseInt(process.env.P2P_PORT) || DEFAULT_PORT;

  wss = new WebSocket.Server({ port: listenPort });

  wss.on("connection", function (ws, req) {
    // Strip IPv4-mapped IPv6 prefix (::ffff:) — not a valid WebSocket URL
    var rawAddr = req.socket.remoteAddress || "unknown";
    if (rawAddr.startsWith("::ffff:")) rawAddr = rawAddr.slice(7);
    var remoteAddr = "inbound:" + rawAddr + ":" + req.socket.remotePort;
    console.log("[BTCPC P2P] Incoming connection from " + rawAddr);

    setupPeerSocket(ws, remoteAddr, "inbound");

    // Send handshake to the newly connected peer
    sendHandshake(ws);
  });

  wss.on("error", function (err) {
    console.error("[BTCPC P2P] Server error:", err.message);
  });

  // Start heartbeat loop
  heartbeatTimer = setInterval(heartbeat, HEARTBEAT_INTERVAL_MS);

  console.log("[BTCPC P2P] Server listening on port " + listenPort);
  console.log("[BTCPC P2P] Node ID: " + NODE_ID);

  return wss;
}

/**
 * Stop the P2P server and disconnect all peers.
 */
function stopServer() {
  if (heartbeatTimer) {
    clearInterval(heartbeatTimer);
    heartbeatTimer = null;
  }

  // Clear all reconnect timers
  for (const [addr, peer] of peers) {
    if (peer.reconnectTimer) {
      clearTimeout(peer.reconnectTimer);
    }
    if (peer.ws && peer.ws.readyState === WebSocket.OPEN) {
      peer.ws.close();
    }
  }
  peers.clear();

  if (wss) {
    wss.close();
    wss = null;
  }

  console.log("[BTCPC P2P] Server stopped");
}

// ---------------------------------------------------------------------------
// Client connections
// ---------------------------------------------------------------------------

/**
 * Connect to a known peer by address (e.g. ws://host:port).
 */
function connectToPeer(address) {
  if (!address) return;

  // Skip inbound peer addresses — they're not connectable
  if (address.startsWith("inbound:")) return;

  // Skip IPv4-mapped IPv6 addresses — not valid WebSocket URLs
  if (address.includes("::ffff:")) return;

  // Normalize address
  if (!address.startsWith("ws://") && !address.startsWith("wss://")) {
    address = "ws://" + address;
  }

  // Don't connect to ourselves or duplicate connections
  if (peers.has(address) && peers.get(address).status === "connected") {
    return;
  }

  if (peers.size >= MAX_PEERS) {
    console.log("[BTCPC P2P] Max peers reached, skipping " + address);
    return;
  }

  console.log("[BTCPC P2P] Connecting to peer: " + address);

  try {
    const ws = new WebSocket(address);

    ws.on("open", function () {
      console.log("[BTCPC P2P] Connected to " + address);
      setupPeerSocket(ws, address, "outbound");
      sendHandshake(ws);
    });

    ws.on("error", function (err) {
      console.error("[BTCPC P2P] Connection error (" + address + "):", err.message);
      scheduleReconnect(address);
    });
  } catch (err) {
    console.error("[BTCPC P2P] Failed to connect to " + address + ":", err.message);
    scheduleReconnect(address);
  }
}

/**
 * Connect to a list of seed peers from environment or explicit list.
 */
function connectToSeeds(seedList) {
  const seeds = seedList || process.env.BTCPC_SEED_PEERS || "";
  if (!seeds) return;

  const addresses = seeds.split(",").map(function (s) { return s.trim(); }).filter(Boolean);
  for (const addr of addresses) {
    connectToPeer(addr);
  }
}

// ---------------------------------------------------------------------------
// Peer socket management
// ---------------------------------------------------------------------------

function setupPeerSocket(ws, address, direction) {
  const peer = {
    ws: ws,
    address: address,
    nodeId: null,
    chainHeight: 0,
    direction: direction,
    status: "connected",
    reconnectAttempts: 0,
    reconnectTimer: null,
    lastSeen: Date.now(),
    // Vuln 5: track the claimed proposer for this connection.
    // If a connection sends BLOCK_PROPOSAL with two different proposer names it's spoofing.
    claimed_proposer: null,
  };

  // Carry over reconnect attempts from previous entry
  if (peers.has(address)) {
    const old = peers.get(address);
    if (old.reconnectTimer) clearTimeout(old.reconnectTimer);
  }

  peers.set(address, peer);

  ws.on("message", function (data) {
    peer.lastSeen = Date.now();
    try {
      const msg = JSON.parse(data.toString());
      handleIncoming(peer, msg);
    } catch (err) {
      console.error("[BTCPC P2P] Bad message from " + address + ":", err.message);
    }
  });

  ws.on("close", function () {
    console.log("[BTCPC P2P] Disconnected from " + address);
    peer.status = "disconnected";
    if (direction === "outbound") {
      scheduleReconnect(address);
    }
  });

  ws.on("error", function () {
    // Error already logged on the ws object; just mark disconnected
    peer.status = "disconnected";
  });
}

function sendHandshake(ws) {
  const handshake = createHandshake(NODE_ID);
  send(ws, handshake);
}

// ---------------------------------------------------------------------------
// Messaging
// ---------------------------------------------------------------------------

/**
 * Send a message object to a single WebSocket.
 */
function send(ws, msg) {
  if (ws.readyState === WebSocket.OPEN) {
    ws.send(JSON.stringify(msg));
  }
}

/**
 * Broadcast a message to all connected peers.
 * Optionally exclude a specific peer address.
 */
function broadcast(message, excludeAddress) {
  for (const [addr, peer] of peers) {
    if (addr === excludeAddress) continue;
    if (peer.ws && peer.ws.readyState === WebSocket.OPEN) {
      peer.ws.send(JSON.stringify(message));
    }
  }
}

/**
 * Register a handler for incoming messages.
 * handler(message, peer) will be called for every parsed message.
 */
function onMessage(handler) {
  messageHandlers.push(handler);
}

/**
 * Internal incoming message dispatcher.
 */
function handleIncoming(peer, msg) {
  // Run through protocol handler first
  handleMessage(peer, msg, { broadcast, send, peers, NODE_ID, connectToPeer });

  // Then notify any registered external handlers
  for (const handler of messageHandlers) {
    try {
      handler(msg, peer);
    } catch (err) {
      console.error("[BTCPC P2P] Message handler error:", err.message);
    }
  }
}

// ---------------------------------------------------------------------------
// Reconnection with exponential backoff
// ---------------------------------------------------------------------------

function scheduleReconnect(address) {
  let peer = peers.get(address);
  if (!peer) {
    peer = {
      ws: null,
      address: address,
      nodeId: null,
      chainHeight: 0,
      direction: "outbound",
      status: "disconnected",
      reconnectAttempts: 0,
      reconnectTimer: null,
      lastSeen: 0
    };
    peers.set(address, peer);
  }

  if (peer.reconnectTimer) return; // already scheduled

  peer.reconnectAttempts++;
  const delay = Math.min(
    1000 * Math.pow(2, peer.reconnectAttempts - 1),
    MAX_RECONNECT_DELAY_MS
  );

  console.log("[BTCPC P2P] Reconnecting to " + address + " in " + (delay / 1000) + "s (attempt " + peer.reconnectAttempts + ")");

  peer.reconnectTimer = setTimeout(function () {
    peer.reconnectTimer = null;
    connectToPeer(address);
  }, delay);
}

// ---------------------------------------------------------------------------
// Heartbeat — prune dead connections
// ---------------------------------------------------------------------------

function heartbeat() {
  const now = Date.now();
  for (const [addr, peer] of peers) {
    if (peer.ws && peer.ws.readyState !== WebSocket.OPEN) {
      peer.status = "disconnected";
    }
    // Prune inbound peers that haven't sent anything in 2 minutes
    if (peer.direction === "inbound" && peer.status === "disconnected") {
      if (now - peer.lastSeen > 120000) {
        peers.delete(addr);
      }
    }
  }
}

// ---------------------------------------------------------------------------
// Accessors
// ---------------------------------------------------------------------------

/**
 * Get the list of connected peers with their status.
 */
function getPeers() {
  const list = [];
  for (const [addr, peer] of peers) {
    list.push({
      address: addr,
      nodeId: peer.nodeId,
      chainHeight: peer.chainHeight,
      direction: peer.direction,
      status: peer.status,
      lastSeen: peer.lastSeen
    });
  }
  return list;
}

/**
 * Get the count of currently connected peers.
 */
function getConnectedCount() {
  let count = 0;
  for (const [, peer] of peers) {
    if (peer.status === "connected" && peer.ws && peer.ws.readyState === WebSocket.OPEN) {
      count++;
    }
  }
  return count;
}

function getNodeId() {
  return NODE_ID;
}

// ---------------------------------------------------------------------------
// Standalone mode — run as a standalone P2P node
// ---------------------------------------------------------------------------

if (require.main === module) {
  const port = parseInt(process.env.P2P_PORT) || DEFAULT_PORT;
  startServer(port);
  connectToSeeds();
  console.log("[BTCPC P2P] Running in standalone mode");
}

// ---------------------------------------------------------------------------
// Exports
// ---------------------------------------------------------------------------

module.exports = {
  startServer,
  stopServer,
  connectToPeer,
  connectToSeeds,
  broadcast,
  send,
  onMessage,
  getPeers,
  getConnectedCount,
  getNodeId,
  NODE_ID,
  peers
};
