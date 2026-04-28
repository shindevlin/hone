"use strict";

/**
 * BTCPC Rust P2P IPC Client
 * Shin Devlin
 *
 * Connects Node.js chain process to the btcpc-p2p Rust sidecar via Unix socket.
 * Drop-in replacement for network.js when BTCPC_USE_RUST_P2P=true.
 *
 * Exports the same interface as network.js:
 *   startServer()         — launches the sidecar subprocess
 *   stopServer()
 *   broadcast(msg)
 *   send(ws, msg)         — routes through gossipsub broadcast
 *   onMessage(handler)
 *   getPeers()
 *   getConnectedCount()
 *   connectToPeer(addr)   — no-op; sidecar handles dialing
 *   connectToSeeds()      — no-op
 *   connectToRelays()     — no-op
 *   getNodeId()
 *   getKnownPeerKeys()    — returns empty (sidecar owns key registry)
 *
 * Binary resolution order:
 *   1. BTCPC_P2P_BIN env var
 *   2. /usr/local/bin/btcpc-p2p (installed via install.sh)
 *   3. ../../../btcpc-p2p/target/release/btcpc-p2p (dev: sibling repo)
 */

const net = require("net");
const fs = require("fs");
const { EventEmitter } = require("events");
const { spawn } = require("child_process");
const path = require("path");

const SOCKET_PATH = process.env.BTCPC_P2P_IPC_SOCKET || "/tmp/btcpc-p2p.sock";
const RECONNECT_DELAY_MS = 2000;

function _resolveBin() {
  if (process.env.BTCPC_P2P_BIN) return process.env.BTCPC_P2P_BIN;
  if (fs.existsSync("/usr/local/bin/btcpc-p2p")) return "/usr/local/bin/btcpc-p2p";
  // Dev: repos/btcpc/src/p2p/ → ../../.. → repos/ → btcpc-p2p/target/release/btcpc-p2p
  return path.resolve(__dirname, "../../../btcpc-p2p/target/release/btcpc-p2p");
}

const SIDECAR_BIN = _resolveBin();

class RustP2PClient extends EventEmitter {
  constructor() {
    super();
    this._socket = null;
    this._buf = "";
    this._handlers = [];
    this._peers = new Map();
    this._sidecar = null;
    this._reconnectTimer = null;
    this._stopping = false;
  }

  // ── Lifecycle ──────────────────────────────────────────────────────────────

  startServer() {
    if (this._sidecar) return this;
    this._stopping = false;

    if (!fs.existsSync(SIDECAR_BIN)) {
      console.error("[BTCPC P2P] Rust sidecar binary not found: " + SIDECAR_BIN);
      console.error("[BTCPC P2P] Build it: cd repos/btcpc-p2p && cargo build --release");
      return this;
    }

    console.log("[BTCPC P2P] Starting Rust sidecar:", SIDECAR_BIN);
    this._sidecar = spawn(SIDECAR_BIN, [], {
      env: { ...process.env },
      stdio: ["ignore", "pipe", "pipe"],
      detached: false,
    });

    this._sidecar.stdout.on("data", (d) => process.stdout.write("[btcpc-p2p] " + d));
    this._sidecar.stderr.on("data", (d) => process.stderr.write("[btcpc-p2p] " + d));
    this._sidecar.on("exit", (code, signal) => {
      this._sidecar = null;
      if (!this._stopping) {
        console.warn("[BTCPC P2P] Sidecar exited (code=" + code + " signal=" + signal + ") — restarting in 5s");
        setTimeout(() => this.startServer(), 5000);
      }
    });

    // Give sidecar 500ms to bind the socket before connecting
    setTimeout(() => this._connect(), 500);
    return this;
  }

  stopServer() {
    this._stopping = true;
    if (this._reconnectTimer) { clearTimeout(this._reconnectTimer); this._reconnectTimer = null; }
    if (this._socket) { this._socket.destroy(); this._socket = null; }
    if (this._sidecar) { this._sidecar.kill("SIGTERM"); this._sidecar = null; }
    console.log("[BTCPC P2P] Rust sidecar stopped");
  }

  // ── IPC connection ─────────────────────────────────────────────────────────

  _connect() {
    if (this._stopping) return;

    const socket = net.createConnection(SOCKET_PATH);
    this._socket = socket;
    this._buf = "";

    socket.on("connect", () => {
      console.log("[BTCPC P2P] Connected to Rust sidecar via", SOCKET_PATH);
      if (this._reconnectTimer) { clearTimeout(this._reconnectTimer); this._reconnectTimer = null; }
    });

    socket.on("data", (chunk) => {
      this._buf += chunk.toString("utf8");
      const lines = this._buf.split("\n");
      this._buf = lines.pop(); // hold incomplete line
      for (const line of lines) {
        if (!line.trim()) continue;
        try {
          this._handlePush(JSON.parse(line));
        } catch (e) {
          console.error("[BTCPC P2P] IPC parse error:", e.message);
        }
      }
    });

    socket.on("close", () => {
      if (!this._stopping) {
        console.warn("[BTCPC P2P] IPC socket closed — reconnecting in " + RECONNECT_DELAY_MS + "ms");
        this._reconnectTimer = setTimeout(() => this._connect(), RECONNECT_DELAY_MS);
      }
    });

    socket.on("error", (err) => {
      // ENOENT = sidecar not ready yet; ECONNREFUSED = socket exists but nobody listening
      if (err.code !== "ENOENT" && err.code !== "ECONNREFUSED") {
        console.error("[BTCPC P2P] IPC socket error:", err.message);
      }
    });
  }

  // ── Push events: Rust → Node.js ───────────────────────────────────────────

  _handlePush(msg) {
    switch (msg.method) {
      case "message": {
        const { from, data } = msg.params;
        // Wrap in a synthetic peer object matching network.js peer structure
        const peer = {
          address: from,
          nodeId: (data && data.nodeId) || from,
          direction: "inbound",
          status: "connected",
          lastSeen: Date.now(),
        };
        for (const handler of this._handlers) {
          try { handler(data, peer); } catch (e) {
            console.error("[BTCPC P2P] Message handler error:", e.message);
          }
        }
        break;
      }
      case "peer_connected":
        this._peers.set(msg.params.peer, { status: "connected", lastSeen: Date.now() });
        break;
      case "peer_disconnected":
        this._peers.delete(msg.params.peer);
        break;
      default:
        break;
    }
  }

  // ── Commands: Node.js → Rust ──────────────────────────────────────────────

  _ipcSend(cmd) {
    if (!this._socket || !this._socket.writable) return;
    try {
      this._socket.write(JSON.stringify(cmd) + "\n");
    } catch (e) {
      console.error("[BTCPC P2P] IPC write error:", e.message);
    }
  }

  // ── network.js-compatible interface ──────────────────────────────────────

  broadcast(message) {
    this._ipcSend({ method: "broadcast", params: { data: message } });
  }

  send(_ws, message) {
    // In sidecar mode all sends go through gossipsub; ws is ignored
    this._ipcSend({ method: "broadcast", params: { data: message } });
  }

  onMessage(handler) {
    this._handlers.push(handler);
  }

  getPeers() {
    return Array.from(this._peers.entries()).map(([id, p]) => ({
      address: id,
      nodeId: id,
      status: p.status,
      lastSeen: p.lastSeen,
      direction: "outbound",
      chainHeight: 0,
    }));
  }

  getConnectedCount() {
    return Array.from(this._peers.values()).filter((p) => p.status === "connected").length;
  }

  getNodeId() {
    return process.env.BTCPC_NODE_ID || "rust-sidecar";
  }

  getKnownPeerKeys() { return {}; }
  connectToPeer()    {}
  connectToSeeds()   {}
  connectToRelays()  {}

  get NODE_ID()    { return this.getNodeId(); }
  get RELAY_URLS() { return []; }
  get peers()      { return this._peers; }
}

const client = new RustP2PClient();
module.exports = client;
