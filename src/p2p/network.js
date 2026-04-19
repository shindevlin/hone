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
const { handleMessage, createHandshake, knownPeers, startPeerAnnounce, stopPeerAnnounce } = require("./protocol");
const transport = require("./encryptedTransport");

// Node identity — generated once on first start, persisted in env or memory
const NODE_ID = process.env.BTCPC_NODE_ID || crypto.randomBytes(16).toString("hex");

const DEFAULT_PORT = 6942;
const MAX_PEERS = 50;
const HEARTBEAT_INTERVAL_MS = 30000;
const MAX_RECONNECT_DELAY_MS = 300000; // 5 minutes

// Relay URLs — supports multiple for redundancy via BTCPC_RELAY_URLS (comma-separated).
// Falls back to BTCPC_RELAY_URL (single) or the built-in Cloudflare relay.
// The Cloudflare relay speaks plain JSON — Noise_XX is skipped for all relay connections.
const RELAY_URLS = (function () {
  const multi = process.env.BTCPC_RELAY_URLS;
  if (multi) return multi.split(",").map(function (u) { return u.trim(); }).filter(Boolean);
  const single = process.env.BTCPC_RELAY_URL || "wss://btcpc-relay.shindevlin.workers.dev/ws";
  return single ? [single] : [];
})();

function isRelayAddress(address) {
  if (!address) return false;
  for (var i = 0; i < RELAY_URLS.length; i++) {
    if (RELAY_URLS[i] === address) return true;
  }
  // Cloudflare Workers fallback match — catches any workers.dev URL even if not in list
  if (address.includes("workers.dev")) return true;
  return false;
}

/**
 * Connect to all configured relay URLs. Call this instead of connectToPeer(relayUrl)
 * so that all relays in BTCPC_RELAY_URLS are tried; surviving relays maintain
 * connectivity if others go down.
 */
function connectToRelays() {
  if (RELAY_URLS.length === 0) return;
  console.log("[BTCPC P2P] Connecting to " + RELAY_URLS.length + " relay(s)");
  for (var i = 0; i < RELAY_URLS.length; i++) {
    connectToPeer(RELAY_URLS[i]);
  }
}

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
  const listenPort = port || parseInt(process.env.BTCPC_API_P2P_PORT || process.env.P2P_PORT) || DEFAULT_PORT;

  function _bindServer(tryPort, retriesLeft) {
    wss = new WebSocket.Server({ port: tryPort });
    wss.once("error", function (err) {
      if (err.code === "EADDRINUSE" && retriesLeft > 0) {
        console.warn("[BTCPC P2P] Port " + tryPort + " in use, retrying on " + (tryPort + 1));
        wss.close();
        _bindServer(tryPort + 1, retriesLeft - 1);
      } else {
        console.error("[BTCPC P2P] Server error:", err.message);
      }
    });
    wss.once("listening", function () {
      console.log("[BTCPC P2P] Server listening on port " + tryPort);
      console.log("[BTCPC P2P] Node ID: " + NODE_ID);
      // Re-attach ongoing error handler after bind succeeds
      wss.on("error", function (err) {
        console.error("[BTCPC P2P] Server error:", err.message);
      });
    });
  }
  _bindServer(listenPort, 5);

  wss.on("connection", function (ws, req) {
    // Strip IPv4-mapped IPv6 prefix (::ffff:) — not a valid WebSocket URL
    var rawAddr = req.socket.remoteAddress || "unknown";
    if (rawAddr.startsWith("::ffff:")) rawAddr = rawAddr.slice(7);
    var remoteAddr = "inbound:" + rawAddr + ":" + req.socket.remotePort;
    console.log("[BTCPC P2P] Incoming connection from " + rawAddr);

    setupPeerSocket(ws, remoteAddr, "inbound");

    // Noise_XX handshake — responder waits for initiator's first message
    transport.acceptHandshake(ws, transport.getStaticKeypair(), remoteAddr).then(function () {
      // Handshake complete — send BTCPC protocol handshake over encrypted channel
      sendHandshake(ws);
    }).catch(function (err) {
      console.error("[BTCPC Noise] Inbound handshake failed from " + rawAddr + ":", err.message);
      ws.close();
    });
  });

  wss.on("error", function (err) {
    console.error("[BTCPC P2P] Server error:", err.message);
  });

  // Start heartbeat loop
  heartbeatTimer = setInterval(heartbeat, HEARTBEAT_INTERVAL_MS);

  // Start peer announce relay — every 5 minutes, broadcast known peers
  startPeerAnnounce({ broadcast, NODE_ID });

  // Connect to any previously known peers from disk (relay-free reconnection)
  for (var savedAddr of knownPeers) {
    connectToPeer(savedAddr);
  }

  return wss;
}

/**
 * Stop the P2P server and disconnect all peers.
 */
function stopServer() {
  stopPeerAnnounce();

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

      if (isRelayAddress(address)) {
        // Relay speaks plain JSON — skip Noise, send BTCPC handshake directly
        sendHandshake(ws);
        knownPeers.add(address);
        return;
      }

      // Noise_XX handshake — initiator sends first message
      transport.startHandshake(ws, transport.getStaticKeypair(), address).then(function () {
        // Handshake complete — send BTCPC protocol handshake and pin peer
        sendHandshake(ws);
        knownPeers.add(address);
      }).catch(function (err) {
        console.error("[BTCPC Noise] Outbound handshake failed to " + address + ":", err.message);
        ws.close();
      });
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
    noiseEnabled: !isRelayAddress(address),
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

    if (!peer.noiseEnabled) {
      // Relay connection — plain JSON messages
      try {
        const raw = typeof data === "string" ? data : data.toString("utf8");
        const msg = JSON.parse(raw);
        handleIncoming(peer, msg);
      } catch (err) {
        console.error("[BTCPC P2P] Bad relay message from " + address + ":", err.message);
      }
      return;
    }

    // Route through Noise handshake while in progress
    if (transport.isHandshaking(ws)) {
      transport.processHandshakeData(ws, data, transport.getStaticKeypair());
      return;
    }

    // Decrypt transport message, then dispatch
    var plaintext;
    try {
      plaintext = transport.receive(ws, data);
    } catch (err) {
      console.error("[BTCPC Noise] Decrypt error from " + address + ":", err.message);
      return;
    }

    try {
      const msg = JSON.parse(plaintext);
      handleIncoming(peer, msg);
    } catch (err) {
      console.error("[BTCPC P2P] Bad message from " + address + ":", err.message);
    }
  });

  ws.on("close", function () {
    console.log("[BTCPC P2P] Disconnected from " + address);
    peer.status = "disconnected";
    transport.closeTransport(ws);
    if (direction === "outbound") {
      scheduleReconnect(address);
    }
  });

  ws.on("error", function () {
    // Error already logged on the ws object; just mark disconnected
    peer.status = "disconnected";
    transport.closeTransport(ws);
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
 * Relay connections use plain JSON; direct peer connections use Noise-encrypted binary.
 */
function send(ws, msg) {
  if (ws.readyState !== WebSocket.OPEN) return;

  // Look up whether this ws belongs to a relay connection
  var noiseEnabled = true;
  for (var [, p] of peers) {
    if (p.ws === ws) { noiseEnabled = p.noiseEnabled; break; }
  }

  if (!noiseEnabled) {
    try {
      ws.send(typeof msg === "string" ? msg : JSON.stringify(msg));
    } catch (err) {
      console.error("[BTCPC P2P] Relay send error:", err.message);
    }
    return;
  }

  if (transport.isReady(ws)) {
    try {
      transport.send(ws, msg);
    } catch (err) {
      console.error("[BTCPC P2P] Send error:", err.message);
    }
  }
}

/**
 * Broadcast a message to all connected peers.
 * Relay connections use plain JSON; direct peers use Noise-encrypted binary.
 * Optionally exclude a specific peer address.
 */
function broadcast(message, excludeAddress) {
  for (const [addr, peer] of peers) {
    if (addr === excludeAddress) continue;
    if (!peer.ws || peer.ws.readyState !== WebSocket.OPEN) continue;

    if (!peer.noiseEnabled) {
      try {
        peer.ws.send(typeof message === "string" ? message : JSON.stringify(message));
      } catch (err) {
        console.error("[BTCPC P2P] Relay broadcast error to " + addr + ":", err.message);
      }
    } else if (transport.isReady(peer.ws)) {
      try {
        transport.send(peer.ws, message);
      } catch (err) {
        console.error("[BTCPC P2P] Broadcast error to " + addr + ":", err.message);
      }
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
  connectToRelays,
  broadcast,
  send,
  onMessage,
  getPeers,
  getConnectedCount,
  getNodeId,
  NODE_ID,
  RELAY_URLS,
  peers,
  getKnownPeerKeys: transport.getKnownPeerKeys,
};
