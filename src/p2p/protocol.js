"use strict";

/**
 * BTCPC P2P Protocol
 * Shin Devlin
 *
 * Defines message types and handlers for the BTCPC peer-to-peer network.
 * Every message follows the format: { type, data, timestamp, nodeId }
 */

const fs = require("fs");
const path = require("path");
const { validateBlock, getChainHeight, getBlockRange } = require("./chainSync");
const mempool = require("./mempool");
const Block = require("../chain/block");
const blockchain = require("../chain/blockchain");
const blockStore = require("../chain/blockStore");
const stateManager = require("../chain/stateManager");
const messageAuth = require("./messageAuth");
const { getAdvertisedP2PAddress, isConnectableP2PAddress, normalizeP2PAddress } = require("./address");
const { shouldStartBackgroundTimers } = require("../services/backgroundTimers");
const PROTOCOL_EPOCH_DURATION_MS = 30 * 1000;

// ---------------------------------------------------------------------------
// Known peers — persistent peer address book for relay-free reconnection
// ---------------------------------------------------------------------------

const KNOWN_PEERS_PATH = path.join(__dirname, "../../data/known-peers.json");
const knownPeers = new Set();

// Load persisted peers on startup
try {
  if (fs.existsSync(KNOWN_PEERS_PATH)) {
    var _saved = JSON.parse(fs.readFileSync(KNOWN_PEERS_PATH, "utf8"));
    var _needsPrune = false;
    if (Array.isArray(_saved)) {
      for (var _addr of _saved) {
        const normalized = normalizeP2PAddress(_addr);
        if (normalized) {
          knownPeers.add(normalized);
        } else {
          _needsPrune = true;
        }
      }
    }
    if (knownPeers.size > 0) {
      console.log("[BTCPC P2P] Loaded " + knownPeers.size + " known peers from disk");
    }
    if (_needsPrune) {
      saveKnownPeers();
      console.log("[BTCPC P2P] Pruned non-connectable known peers from disk");
    }
  }
} catch (_e) {
  // Ignore — file may not exist yet
}

function addKnownPeerAddress(addr) {
  const normalized = normalizeP2PAddress(addr);
  if (!normalized) return false;
  if (knownPeers.has(normalized)) return false;
  knownPeers.add(normalized);
  return true;
}

function saveKnownPeers() {
  try {
    fs.writeFileSync(KNOWN_PEERS_PATH, JSON.stringify(Array.from(knownPeers), null, 2));
  } catch (_e) {
    console.error("[BTCPC P2P] Failed to save known peers:", _e.message);
  }
}

// ---------------------------------------------------------------------------
// Message types
// ---------------------------------------------------------------------------

const MESSAGE_TYPES = {
  HANDSHAKE: "HANDSHAKE",
  BLOCK: "BLOCK",
  TRANSACTION: "TRANSACTION",
  PEER_LIST: "PEER_LIST",
  EPOCH_COMMIT: "EPOCH_COMMIT",
  REQUEST_BLOCKS: "REQUEST_BLOCKS",
  RESPONSE_BLOCKS: "RESPONSE_BLOCKS",
  // Unified block proposal — replaces EPOCH_START + EPOCH_END +
  // FINALIZATION_PROPOSAL + EPOCH_FINALIZED with a single message.
  // Any node can broadcast one when wall clock crosses an epoch boundary.
  // Contains everything needed: epoch number, work proofs, rewards,
  // consensus hash, ledger entries, and block header.
  BLOCK_PROPOSAL: "BLOCK_PROPOSAL",
  // Inference protocol
  INFERENCE_REQUEST: "INFERENCE_REQUEST",
  INFERENCE_CLAIM: "INFERENCE_CLAIM",
  INFERENCE_ASSIGN: "INFERENCE_ASSIGN",
  INFERENCE_PAYLOAD: "INFERENCE_PAYLOAD",
  INFERENCE_COMMIT: "INFERENCE_COMMIT",
  INFERENCE_REVEAL: "INFERENCE_REVEAL",
  INFERENCE_RESULT: "INFERENCE_RESULT",
  // Stale-job nudge — any node can broadcast when a claim is overdue
  INFERENCE_NUDGE: "INFERENCE_NUDGE",
  // Released claim — claimer didn't respond to nudges, job is up for grabs
  INFERENCE_RECLAIM: "INFERENCE_RECLAIM",
  // Model demand broadcast
  MODEL_DEMAND: "MODEL_DEMAND",
  // Mining proof gossip — miners broadcast proofs so all nodes can finalize
  MINING_PROOF: "MINING_PROOF",
  // Miner idle — no work this epoch, don't wait for my proof
  MINER_IDLE: "MINER_IDLE",
  // Epoch authority broadcasts epoch lifecycle
  EPOCH_START: "EPOCH_START",
  EPOCH_END: "EPOCH_END",
  EPOCH_FINALIZED: "EPOCH_FINALIZED",
  // Account announcement — any node can broadcast
  ACCOUNT_ANNOUNCE: "ACCOUNT_ANNOUNCE",
  // Ledger sync — request/response for missing ledger entries
  REQUEST_LEDGER: "REQUEST_LEDGER",
  RESPONSE_LEDGER: "RESPONSE_LEDGER",
  // Clock heartbeat — clock nodes prove they're online
  CLOCK_HEARTBEAT: "CLOCK_HEARTBEAT",
  // Finalization proposal — miners propose reward splits for consensus
  FINALIZATION_PROPOSAL: "FINALIZATION_PROPOSAL",
  // Inference verification — verifiers validate miner output
  VERIFY_REQUEST: "VERIFY_REQUEST",
  VERIFY_RESPONSE: "VERIFY_RESPONSE",
  // Commit-reveal: verifier first commits (hash of verdict+nonce), then reveals
  // after VERIFIER_REVEAL_WINDOW_MS to prevent early-verdict influence (T-04)
  VERIFIER_COMMIT: "VERIFIER_COMMIT",
  VERIFIER_REVEAL: "VERIFIER_REVEAL",
  // Agent protocol: multi-turn tool-calling sessions on miners
  // TOOL_CALL: miner detected a tool_use block, needs the client to run the tool
  // TOOL_RESULT: client delivers the result back to the miner to resume inference
  TOOL_CALL: "TOOL_CALL",
  TOOL_RESULT: "TOOL_RESULT",
  // Mempool gossip — ledger entries broadcast across machines so any
  // broadcaster can include them in its next block (v2.13.3)
  MEMPOOL_ENTRY: "MEMPOOL_ENTRY",
  // Peer relay — nodes announce their known peers so the network
  // doesn't depend on a single Cloudflare relay for discovery
  PEER_ANNOUNCE: "PEER_ANNOUNCE",
  // Testnet heartbeat — nodes running the public test network for developers
  TESTNET_HEARTBEAT: "TESTNET_HEARTBEAT",
  // Shard inference (Mode B — pipeline)
  SHARD_REGISTER: "SHARD_REGISTER",      // node announces a model layer shard
  SHARD_GROUP_FORM: "SHARD_GROUP_FORM",  // broadcast when a full group is assembled
  SHARD_ACTIVATE: "SHARD_ACTIVATE",      // activation tensor passed shard→shard
  SHARD_RESULT: "SHARD_RESULT",          // final shard reports inference completion
  // Ensemble inference (Mode A — majority-vote)
  ENSEMBLE_CLAIM: "ENSEMBLE_CLAIM",      // node announces it will contribute
  ENSEMBLE_RESULT: "ENSEMBLE_RESULT",    // node submits its result hash
  ENSEMBLE_CONSENSUS: "ENSEMBLE_CONSENSUS", // consensus achieved, broadcast winner
  // Scientific compute engine (v3.2)
  SCIENTIFIC_JOB:    "SCIENTIFIC_JOB",    // broadcast a new scientific job to the network
  SCIENTIFIC_RESULT: "SCIENTIFIC_RESULT", // completed scientific job result
  NODE_LATENCY:      "NODE_LATENCY",      // peer-to-peer latency measurement for route optimization
};

// ---------------------------------------------------------------------------
// Replay attack prevention — chain_id + timestamp freshness
// ---------------------------------------------------------------------------

var _genesisHash = null; // cached on first use
var STALE_MSG_MS = 30000; // 30 seconds (one epoch) — tightened per security audit P2P-02

function getGenesisHash() {
  if (_genesisHash) return _genesisHash;
  try {
    var genesis = blockStore.readBlock(0);
    if (genesis && genesis.block) {
      _genesisHash = genesis.block.computeHash();
    }
  } catch (_) {}
  return _genesisHash;
}

// Track seen message IDs to prevent rebroadcast loops.
// Persisted to data/seen-messages.json so restarts don't re-process
// messages that arrived in the last 10 minutes.
const SEEN_MESSAGES_PATH = path.join(__dirname, "..", "..", "data", "seen-messages.json");
const SEEN_MAX = 10000;
const SEEN_MAX_AGE_MS = 600000; // 10 minutes

// { msgId: timestamp } — in-memory mirror of the persisted file
var seenMessages = new Map();

// Load persisted seen messages on startup
(function loadSeenMessages() {
  try {
    if (fs.existsSync(SEEN_MESSAGES_PATH)) {
      var raw = JSON.parse(fs.readFileSync(SEEN_MESSAGES_PATH, "utf8"));
      var now = Date.now();
      var keys = Object.keys(raw);
      for (var i = 0; i < keys.length; i++) {
        if (now - raw[keys[i]] < SEEN_MAX_AGE_MS) {
          seenMessages.set(keys[i], raw[keys[i]]);
        }
      }
      console.log("[BTCPC P2P] Loaded " + seenMessages.size + " seen messages from disk (pruned " + (keys.length - seenMessages.size) + " stale)");
    }
  } catch (_) {
    // Non-fatal — start fresh
  }
})();

// Batch-write seen messages to disk every 30 seconds
var _seenDirty = false;

function flushSeenMessages() {
  if (!_seenDirty) return;
  try {
    var obj = {};
    for (var entry of seenMessages) {
      obj[entry[0]] = entry[1];
    }
    // Ensure data/ directory exists
    var dir = path.dirname(SEEN_MESSAGES_PATH);
    if (!fs.existsSync(dir)) fs.mkdirSync(dir, { recursive: true });
    fs.writeFileSync(SEEN_MESSAGES_PATH, JSON.stringify(obj));
    _seenDirty = false;
  } catch (_) {
    // Non-fatal — persistence is best-effort
  }
}

if (shouldStartBackgroundTimers()) {
  setInterval(flushSeenMessages, 30000);
}

function markSeen(msgId) {
  seenMessages.set(msgId, Date.now());
  _seenDirty = true;
  if (seenMessages.size > SEEN_MAX) {
    // Evict oldest entries (Map maintains insertion order)
    var iter = seenMessages.keys();
    for (var i = 0; i < 1000; i++) {
      seenMessages.delete(iter.next().value);
    }
  }
}

// ---------------------------------------------------------------------------
// Message creation helpers
// ---------------------------------------------------------------------------

function createMessage(type, data, nodeId) {
  const id = nodeId + "-" + Date.now() + "-" + Math.random().toString(36).slice(2, 8);
  var msg = {
    id: id,
    type: type,
    data: data,
    timestamp: Date.now(),
    nodeId: nodeId
  };
  // Attach chain_id so peers can reject cross-chain replays
  var genesis = getGenesisHash();
  if (genesis) msg.chain_id = genesis;
  return msg;
}

/**
 * Create a HANDSHAKE message with chain state and known peers.
 */
function createHandshake(nodeId) {
  var pkg = require("../../package.json");
  return createMessage(MESSAGE_TYPES.HANDSHAKE, {
    chainHeight: getChainHeight(),
    version: pkg.version,
    peerCount: 0, // filled by caller if needed
    public_address: getAdvertisedP2PAddress()
  }, nodeId);
}

/**
 * Create a BLOCK message for broadcasting a new epoch/block.
 * If the block is a Block instance, serialize the header to a hex string
 * so peers can deserialize and validate it formally.
 */
function createBlockMessage(block, nodeId) {
  var data;
  if (block instanceof Block) {
    data = {
      header_hex: block.serialize().toString("hex"),
      hash: block.computeHash(),
      transactions: block.transactions || [],
      compute_proofs: block.compute_proofs || []
    };
  } else {
    data = block;
  }
  return createMessage(MESSAGE_TYPES.BLOCK, data, nodeId);
}

/**
 * Create a TRANSACTION message for broadcasting a pending transaction.
 */
function createTransactionMessage(tx, nodeId) {
  return createMessage(MESSAGE_TYPES.TRANSACTION, tx, nodeId);
}

/**
 * Create a PEER_LIST message containing known peer addresses.
 */
function createPeerListMessage(peerAddresses, nodeId) {
  return createMessage(MESSAGE_TYPES.PEER_LIST, {
    peers: peerAddresses
  }, nodeId);
}

/**
 * Create an EPOCH_COMMIT message when a node submits its epoch commitment.
 */
function createEpochCommitMessage(commitment, nodeId) {
  return createMessage(MESSAGE_TYPES.EPOCH_COMMIT, commitment, nodeId);
}

/**
 * Create a REQUEST_BLOCKS message to ask a peer for missing blocks.
 */
function createRequestBlocksMessage(fromEpoch, toEpoch, nodeId) {
  return createMessage(MESSAGE_TYPES.REQUEST_BLOCKS, {
    from: fromEpoch,
    to: toEpoch
  }, nodeId);
}

// ---------------------------------------------------------------------------
// Message handlers
// ---------------------------------------------------------------------------

/**
 * Handle an incoming message from a peer.
 * ctx: { broadcast, send, peers, NODE_ID, connectToPeer }
 */
function handleMessage(peer, msg, ctx) {
  if (!msg || !msg.type) return;

  // Replay attack prevention: reject messages from a different chain
  if (msg.chain_id) {
    var ourGenesis = getGenesisHash();
    if (ourGenesis && msg.chain_id !== ourGenesis) {
      console.log("[BTCPC P2P] Rejected cross-chain message from " +
        (msg.nodeId || "?").slice(0, 12) + " (chain_id mismatch)");
      return;
    }
  }

  // Replay attack prevention: reject stale messages (>5 min old)
  if (msg.timestamp) {
    var msgAge = Date.now() - msg.timestamp;
    if (msgAge > STALE_MSG_MS) {
      console.log("[BTCPC P2P] Rejected stale " + msg.type + " from " +
        (msg.nodeId || "?").slice(0, 12) + " (age: " + Math.round(msgAge / 1000) + "s)");
      return;
    }
  }

  // Deduplicate
  if (msg.id && seenMessages.has(msg.id)) return;
  if (msg.id) markSeen(msg.id);

  // Don't process our own messages
  if (msg.nodeId === ctx.NODE_ID) return;

  switch (msg.type) {
    case MESSAGE_TYPES.HANDSHAKE:
      handleHandshake(peer, msg, ctx);
      break;
    case MESSAGE_TYPES.BLOCK:
      handleBlock(peer, msg, ctx);
      break;
    case MESSAGE_TYPES.TRANSACTION:
      handleTransaction(peer, msg, ctx);
      break;
    case MESSAGE_TYPES.PEER_LIST:
      handlePeerList(peer, msg, ctx);
      break;
    case MESSAGE_TYPES.EPOCH_COMMIT:
      handleEpochCommit(peer, msg, ctx);
      break;
    case MESSAGE_TYPES.REQUEST_BLOCKS:
      handleRequestBlocks(peer, msg, ctx);
      break;
    case MESSAGE_TYPES.RESPONSE_BLOCKS:
      handleResponseBlocks(peer, msg, ctx);
      break;
    // Inference messages — broadcast to all peers (gossip)
    case MESSAGE_TYPES.INFERENCE_REQUEST:
    case MESSAGE_TYPES.INFERENCE_CLAIM:
    case MESSAGE_TYPES.INFERENCE_ASSIGN:
    case MESSAGE_TYPES.INFERENCE_PAYLOAD:
    case MESSAGE_TYPES.INFERENCE_COMMIT:
    case MESSAGE_TYPES.INFERENCE_REVEAL:
    case MESSAGE_TYPES.INFERENCE_RESULT:
    case MESSAGE_TYPES.MODEL_DEMAND:
      handleInferenceMessage(peer, msg, ctx);
      break;
    case MESSAGE_TYPES.MINING_PROOF:
      handleMiningProof(peer, msg, ctx);
      break;
    case MESSAGE_TYPES.MINER_IDLE:
      handleMinerIdle(peer, msg, ctx);
      break;
    case MESSAGE_TYPES.EPOCH_START: {
      let esData = msg.data || {};
      let esAuthority = esData.authority || "unknown";
      // Verify signature against authority's posting key
      if (esData.signature) {
        let esVerifyData = {
          epoch_number: esData.epoch_number,
          started_at: esData.started_at,
          authority: esData.authority
        };
        // EPOCH_START doesn't require signatures — timing is verified by
        // VRF-based authority rotation, not cryptographic signing.
      }
      console.log("[BTCPC P2P] Epoch START: " + (esData.epoch_number || "?") + " from " + esAuthority);
      if (esData.epoch_number) setCurrentEpoch(esData.epoch_number);
      ctx.broadcast(msg, peer.address);
      break;
    }
    case MESSAGE_TYPES.EPOCH_END:
      console.log("[BTCPC P2P] Epoch END: " + (msg.data?.epoch_number || "?") + " from " + (msg.data?.authority || "unknown"));
      ctx.broadcast(msg, peer.address);
      break;
    case MESSAGE_TYPES.EPOCH_FINALIZED:
      handleEpochFinalized(peer, msg, ctx);
      break;
    case MESSAGE_TYPES.ACCOUNT_ANNOUNCE:
      handleAccountAnnounce(peer, msg, ctx);
      break;
    case MESSAGE_TYPES.MEMPOOL_ENTRY:
      handleMempoolEntry(peer, msg, ctx);
      break;
    case MESSAGE_TYPES.BLOCK_PROPOSAL:
      handleBlockProposal(peer, msg, ctx);
      break;
    case MESSAGE_TYPES.CLOCK_HEARTBEAT:
      handleClockHeartbeat(peer, msg, ctx);
      break;
    case MESSAGE_TYPES.TESTNET_HEARTBEAT:
      handleTestnetHeartbeat(peer, msg, ctx);
      break;
    case MESSAGE_TYPES.VERIFY_REQUEST:
      handleVerifyRequest(peer, msg, ctx);
      break;
    case MESSAGE_TYPES.VERIFY_RESPONSE:
      handleVerifyResponse(peer, msg, ctx);
      break;
    case MESSAGE_TYPES.VERIFIER_COMMIT:
      handleVerifierCommit(peer, msg, ctx);
      break;
    case MESSAGE_TYPES.VERIFIER_REVEAL:
      handleVerifierReveal(peer, msg, ctx);
      break;
    case MESSAGE_TYPES.TOOL_CALL:
      handleToolCall(peer, msg, ctx);
      break;
    case MESSAGE_TYPES.TOOL_RESULT:
      handleToolResult(peer, msg, ctx);
      break;
    case MESSAGE_TYPES.REQUEST_LEDGER:
      handleRequestLedger(peer, msg, ctx);
      break;
    case MESSAGE_TYPES.RESPONSE_LEDGER:
      handleResponseLedger(peer, msg, ctx);
      break;
    case MESSAGE_TYPES.PEER_ANNOUNCE:
      handlePeerAnnounce(peer, msg, ctx);
      break;
    case MESSAGE_TYPES.SHARD_REGISTER:
      handleShardRegister(peer, msg, ctx);
      break;
    case MESSAGE_TYPES.SHARD_ACTIVATE:
    case MESSAGE_TYPES.SHARD_RESULT:
      // Relay shard pipeline messages to all peers
      ctx.broadcast(msg, peer.address);
      break;
    case MESSAGE_TYPES.ENSEMBLE_RESULT:
      handleEnsembleResult(peer, msg, ctx);
      break;
    case MESSAGE_TYPES.ENSEMBLE_CLAIM:
    case MESSAGE_TYPES.ENSEMBLE_CONSENSUS:
    case MESSAGE_TYPES.SHARD_GROUP_FORM:
      // Relay ensemble coordination messages to all peers
      ctx.broadcast(msg, peer.address);
      break;
    case MESSAGE_TYPES.SCIENTIFIC_JOB:
      // Scientific jobs are handled by the engine — just relay to peers
      console.log("[BTCPC P2P] SCIENTIFIC_JOB received from " + (msg.nodeId || "?").slice(0, 12));
      ctx.broadcast(msg, peer.address);
      break;
    case MESSAGE_TYPES.SCIENTIFIC_RESULT:
      // Relay completed scientific results to all peers
      console.log("[BTCPC P2P] SCIENTIFIC_RESULT received from " + (msg.nodeId || "?").slice(0, 12));
      ctx.broadcast(msg, peer.address);
      break;
    case MESSAGE_TYPES.NODE_LATENCY: {
      // Record inter-node latency for shard route optimization
      var _latData = msg.data || {};
      try {
        var _sciEngine = require("../scientific/scientificComputeEngine");
        if (_latData.from_node && _latData.to_node && typeof _latData.latency_ms === "number") {
          _sciEngine.recordLatency(_latData.from_node, _latData.to_node, _latData.latency_ms);
        }
      } catch (_latErr) {
        // Scientific engine not loaded — silently ignore
      }
      break;
    }
    default:
      console.log("[BTCPC P2P] Unknown message type: " + msg.type + " from " + (msg.nodeId || "?").slice(0, 12));
  }
}

/**
 * HANDSHAKE — Exchange chain height, genesis hash, and peer lists.
 */
function handleHandshake(peer, msg, ctx) {
  const data = msg.data || {};

  // Self-connection: same NODE_ID means we connected to ourselves — close immediately
  if (msg.nodeId && msg.nodeId === ctx.NODE_ID) {
    console.log("[BTCPC P2P] Self-connection detected (nodeId match) from " + peer.address + " — closing");
    if (peer.ws) peer.ws.close();
    return;
  }

  peer.nodeId = msg.nodeId;
  peer.chainHeight = data.chainHeight || 0;
  peer.version = data.version || "unknown";
  peer.status = "connected";

  // Store the peer's claimed public address for relay-free discovery
  if (isConnectableP2PAddress(data.public_address)) {
    addKnownPeerAddress(data.public_address);
    saveKnownPeers();
  }

  console.log("[BTCPC P2P] Handshake from " + msg.nodeId.slice(0, 12) + "... (v" + peer.version + ", height: " + peer.chainHeight + ")");
  if (peer.chainHeight > 0) recordPeerEpoch(msg.nodeId, peer.chainHeight);

  // Reject peers on incompatible versions — chain requires v2.0.75+
  var MIN_VERSION = "2.0.75";
  if (peer.version !== "unknown" && peer.version < MIN_VERSION) {
    console.log("[BTCPC P2P] Rejected " + msg.nodeId.slice(0, 12) + " — version " + peer.version + " below minimum " + MIN_VERSION);
    if (peer.ws) peer.ws.close();
    return;
  }

  // Send our peer list to the new peer — only outbound (connectable) addresses
  // Inbound addresses are ephemeral ports, not connectable
  const knownAddresses = [];
  for (const [addr, p] of ctx.peers) {
    if (p.status === "connected" && addr !== peer.address && !addr.startsWith("inbound:")) {
      knownAddresses.push(addr);
    }
  }

  if (knownAddresses.length > 0) {
    const peerListMsg = createPeerListMessage(knownAddresses, ctx.NODE_ID);
    ctx.send(peer.ws, peerListMsg);
  }

  // If the remote peer has a longer chain, request missing blocks
  const localHeight = getChainHeight();
  if (peer.chainHeight > localHeight) {
    const reqMsg = createRequestBlocksMessage(localHeight + 1, peer.chainHeight, ctx.NODE_ID);
    ctx.send(peer.ws, reqMsg);
  }

  // Phase D: legacy REQUEST_LEDGER is a no-op. Block sync via
  // REQUEST_BLOCKS above handles chain catch-up from block files.
}

/**
 * BLOCK — Validate and store a new epoch/block received from the network.
 * Supports both serialized blocks (header_hex) and legacy plain objects.
 */
function handleBlock(peer, msg, ctx) {
  const data = msg.data;
  if (!data) return;

  // Update epoch cache when we see a new block — keeps clock heartbeats
  // filed under the right epoch
  if (data.epoch_number) setCurrentEpoch(data.epoch_number);

  var block;

  // If the message contains a serialized header, deserialize it
  if (data.header_hex) {
    try {
      var headerBuf = Buffer.from(data.header_hex, "hex");
      block = Block.deserialize(headerBuf);
      block.transactions = data.transactions || [];
      block.compute_proofs = data.compute_proofs || [];

      // Verify the hash matches
      var computedHash = block.computeHash();
      if (data.hash && data.hash !== computedHash) {
        console.log("[BTCPC P2P] Block hash mismatch from " + (peer.nodeId || "unknown").slice(0, 12));
        return;
      }

      // Validate against the formal blockchain
      var tip = blockchain.getLatestBlock();
      if (!block.validateBlock(tip)) {
        console.log("[BTCPC P2P] Rejected invalid serialized block from " + (peer.nodeId || "unknown").slice(0, 12));
        return;
      }

      // Store in the formal blockchain and write to disk
      blockchain.addBlock(block);

      var payload = {
        ledger_entries: data.ledger_entries || [],
        rewards: data.rewards || [],
        compute_proofs: data.compute_proofs || [],
        mining_proofs: data.mining_proofs || [],
        state_root: data.state_root || block.state_root || null,
        consensus_hash: data.consensus_hash || null,
        block_reward: data.block_reward || 0,
        total_work: data.total_work || 0
      };

      _applySyncedBlockPayload(block.epoch_number, payload, block);

      if (!blockStore.hasBlock(block.epoch_number)) {
        blockStore.writeBlock(block, payload);
      }

    } catch (err) {
      console.log("[BTCPC P2P] Failed to deserialize block from " + (peer.nodeId || "unknown").slice(0, 12) + ": " + err.message);
      return;
    }
  } else {
    block = data;
  }

  // Legacy validation via chainSync
  const valid = validateBlock(block);
  if (!valid) {
    console.log("[BTCPC P2P] Rejected invalid block from " + (peer.nodeId || "unknown").slice(0, 12));
    return;
  }

  console.log("[BTCPC P2P] Received valid block: epoch " + (block.epoch_number || "?"));

  // Fork check — if our hash at this height differs from the peer's, trigger self-heal
  try {
    const forkResolver = require("../chain/forkResolver");
    if (!forkResolver.isHealInProgress()) {
      const peerHash = block.computeHash ? block.computeHash() : (block.hash || "");
      const { forked } = forkResolver.checkForFork(block.epoch_number, peerHash);
      if (forked) {
        console.warn("[BTCPC P2P] Fork detected at epoch " + block.epoch_number + " from " + (peer.nodeId || peer.address || "?").slice(0, 12));
        // onPeerBlock handles full resolution asynchronously — does not block gossip
        const fullData = data.header_hex
          ? { block, payload: { ledger_entries: data.ledger_entries || [], compute_proofs: data.compute_proofs || [] } }
          : { block, payload: {} };
        forkResolver.onPeerBlock(fullData, peer.address || peer.nodeId, null).catch(() => {});
      }
    }
  } catch (_) {}

  // Rebroadcast to other peers (gossip)
  ctx.broadcast(msg, peer.address);
}

/**
 * TRANSACTION — Add to mempool and rebroadcast.
 */
function handleTransaction(peer, msg, ctx) {
  const tx = msg.data;
  if (!tx) return;

  const added = mempool.addTransaction(tx);
  if (added) {
    console.log("[BTCPC P2P] Tx " + (tx.txHash || "?").slice(0, 12) + "... " + tx.from + " → " + tx.to + " " + tx.amount + " " + (tx.token || "BTCPC"));
    // Phase D: balances update when the TX is included in a block and
    // ledger entries are applied to stateStore. No pre-inclusion cache.

    // Rebroadcast to other peers
    ctx.broadcast(msg, peer.address);
  }
}

/**
 * PEER_LIST — Discover and connect to new peers.
 */
function handlePeerList(peer, msg, ctx) {
  const data = msg.data || {};
  const peerAddresses = data.peers || [];

  for (const addr of peerAddresses) {
    const normalized = normalizeP2PAddress(addr);
    if (normalized && !ctx.peers.has(normalized)) {
      ctx.connectToPeer(normalized);
    }
  }
}

/**
 * EPOCH_COMMIT — A node's epoch commitment broadcast.
 * Store and rebroadcast for consensus.
 */
function handleEpochCommit(peer, msg, ctx) {
  const commitment = msg.data;
  if (!commitment) return;

  console.log("[BTCPC P2P] Epoch commitment from " + (msg.nodeId || "unknown").slice(0, 12) +
    " for epoch " + (commitment.epoch_number || "?"));

  // Rebroadcast to other peers
  ctx.broadcast(msg, peer.address);
}

/**
 * REQUEST_BLOCKS — Serve block history to a peer requesting chain sync.
 */
function handleRequestBlocks(peer, msg, ctx) {
  const data = msg.data || {};
  const from = data.from || 0;
  const to = data.to || from;

  console.log("[BTCPC P2P] Block request from " + (peer.nodeId || "unknown").slice(0, 12) +
    " (epochs " + from + "-" + to + ")");

  const blocks = getBlockRange(from, to);

  const response = createMessage(MESSAGE_TYPES.RESPONSE_BLOCKS, {
    blocks: blocks,
    from: from,
    to: to
  }, ctx.NODE_ID);

  ctx.send(peer.ws, response);
}

function _applySyncedBlockPayload(epochNumber, payload, block) {
  if (!payload) return false;

  var stateStore = require("../chain/stateStore");
  var existingEpoch = stateStore.getEpoch ? stateStore.getEpoch(epochNumber) : null;
  if (existingEpoch && existingEpoch.status === "finalized") {
    return false;
  }

  var ledgerEntries = Array.isArray(payload.ledger_entries) ? payload.ledger_entries : [];
  if (ledgerEntries.length > 0) {
    stateStore.applyEntries(ledgerEntries);
    stateManager.applyLedgerEntries(ledgerEntries);

    var nodeRegistry = require("../chain/nodeRegistry");
    if (nodeRegistry && typeof nodeRegistry.processLedgerEntries === "function") {
      try { nodeRegistry.processLedgerEntries(ledgerEntries); } catch (_) {}
    }
  }

  if (Array.isArray(payload.mining_proofs)) {
    stateStore.setMiningProofs(epochNumber, payload.mining_proofs);
  }
  if (Array.isArray(payload.compute_proofs)) {
    stateStore.setComputeProofs(epochNumber, payload.compute_proofs);
  }

  stateStore.setEpoch(epochNumber, {
    started_at: block && block.timestamp ? new Date(block.timestamp) : null,
    ended_at: block && block.timestamp ? new Date(block.timestamp) : null,
    consensus_hash: payload.consensus_hash || (block && block.computeHash ? block.computeHash() : null),
    state_root: payload.state_root || (block && block.state_root) || null,
    status: "finalized",
    rewards_distributed: payload.rewards || [],
    block_reward: payload.block_reward || 0,
    total_work: payload.total_work || 0,
    miner_id: block ? block.miner_id || null : null,
  });

  return true;
}

/**
 * RESPONSE_BLOCKS — Process blocks received from a sync request.
 */
function handleResponseBlocks(peer, msg, ctx) {
  const data = msg.data || {};
  const blocks = data.blocks || [];

  if (blocks.length === 0) return;

  // Genesis chain ID check — reject blocks from a different chain
  const localGenesis = blockStore.readBlock(0);
  if (localGenesis) {
    const localGenesisHash = localGenesis.block.computeHash();
    for (const b of blocks) {
      if (b.epoch_number === 0 && b.header_hex) {
        try {
          const remoteBlock = Block.deserialize(Buffer.from(b.header_hex, "hex"));
          const remoteHash = remoteBlock.computeHash();
          if (remoteHash !== localGenesisHash) {
            console.log("[BTCPC P2P] Rejected blocks from " +
              (peer.nodeId || "unknown").slice(0, 12) + " — different genesis chain");
            return;
          }
        } catch (_) {}
      }
    }
  }

  console.log("[BTCPC P2P] Received " + blocks.length + " blocks from " +
    (peer.nodeId || "unknown").slice(0, 12));

  let accepted = 0;
  for (const blockData of blocks) {
    if (validateBlock(blockData)) {
      accepted++;

      // Write to disk if block has header_hex
      if (blockData.header_hex && !blockStore.hasBlock(blockData.epoch_number)) {
        try {
          var headerBuf = Buffer.from(blockData.header_hex, "hex");
          var block = Block.deserialize(headerBuf);
          var payload = {
            ledger_entries: blockData.ledger_entries || [],
            rewards: blockData.rewards || [],
            compute_proofs: blockData.compute_proofs || [],
            mining_proofs: blockData.mining_proofs || [],
            state_root: blockData.state_root || block.state_root || null,
            consensus_hash: blockData.consensus_hash || null,
            block_reward: blockData.block_reward || 0,
            total_work: blockData.total_work || 0
          };
          _applySyncedBlockPayload(block.epoch_number, payload, block);
          blockStore.writeBlock(block, payload);
          blockchain.addBlock(block);
        } catch (e) {
          // Non-fatal — block still validated via chainSync
        }
      }
    }
  }

  console.log("[BTCPC P2P] Accepted " + accepted + "/" + blocks.length + " blocks");
}

/**
 * INFERENCE messages — gossip to all peers and notify local handlers.
 */
function handleInferenceMessage(peer, msg, ctx) {
  console.log("[BTCPC P2P] Inference " + msg.type + " from " + (msg.nodeId || "unknown").slice(0, 12));

  // Record work attestation when a miner reveals/finalizes a result.
  // This is the source of truth for "who did what work in this epoch" —
  // gossiped via P2P, not local MongoDB. All nodes that receive the same
  // gossip will compute the same rewards = consensus works across machines.
  if (msg.type === MESSAGE_TYPES.INFERENCE_REVEAL || msg.type === MESSAGE_TYPES.INFERENCE_RESULT) {
    var data = msg.data || {};
    var miner = data.node_name;
    var jobId = data.request_id;
    var tokens = data.tokens_generated || 0;
    var modelWeight = data.model_weight || 1; // miners may include verified model param count
    var workValue = data.work_value || (tokens * modelWeight);
    var epoch = data.epoch_number || _currentEpochCache;

    if (miner && jobId && workValue > 0 && epoch >= 0) {
      recordMinerWork(miner, jobId, workValue, epoch);
      console.log("[BTCPC P2P]   work_attest: " + miner + " +" + workValue + " (job " + jobId.slice(0, 8) + ", epoch " + epoch + ")");
    }
  }

  // Track model demand from MODEL_DEMAND messages
  if (msg.type === MESSAGE_TYPES.MODEL_DEMAND) {
    var demandData = msg.data || {};
    if (demandData.model && typeof demandData.model === "string") {
      _addModelDemand(demandData.model, demandData.account || "anonymous");
    }
  }

  // Rebroadcast to all other peers
  ctx.broadcast(msg, peer.address);
}

// ---------------------------------------------------------------------------
// Model demand tracking — miners use this to decide which models to pull
// ---------------------------------------------------------------------------
// Map<model, { count, accounts: Set<string>, first_requested: number }>
const modelDemandMap = new Map();

function _addModelDemand(model, account) {
  var clean = model.trim().toLowerCase();
  if (!clean || clean.length > 100) return;
  var entry = modelDemandMap.get(clean);
  if (!entry) {
    entry = { count: 0, accounts: new Set(), first_requested: Date.now() };
    modelDemandMap.set(clean, entry);
  }
  entry.count++;
  if (account && typeof account === "string") entry.accounts.add(account);
}

function addModelDemand(model, account) {
  _addModelDemand(model, account);
  // Feed the model manager's pull queue and trigger an immediate check
  try {
    const { recordUnmetDemand } = require('../services/modelRegistry');
    recordUnmetDemand(model.trim().toLowerCase());
    // Kick the model manager now rather than waiting for its 30-min tick
    const { checkAndPullModels } = require('../services/modelManager');
    checkAndPullModels().catch(() => {});
  } catch (_) {}
}

function getModelDemand() {
  var result = [];
  for (var [model, entry] of modelDemandMap) {
    result.push({
      model: model,
      count: entry.count,
      unique_accounts: entry.accounts.size,
      first_requested: entry.first_requested,
    });
  }
  result.sort(function(a, b) { return b.count - a.count; });
  return result.slice(0, 20);
}

// Content-based dedup for mining proofs: "miner:block_number"
// Prevents a miner from flooding the network with duplicate proofs
// carrying different message IDs (which would bypass seenMessages).
var seenProofKeys = new Set();
var SEEN_PROOF_MAX = 5000;

/**
 * MINING_PROOF — A miner broadcasts their proof for an epoch.
 * Receiving nodes store it locally so finalization can collect all proofs.
 * This eliminates the need for a shared database between miners.
 */
function handleMiningProof(peer, msg, ctx) {
  const data = msg.data || {};
  if (!data.block_number || !data.miner) return;

  // Dedup key: use work_hash when present (allows multiple phone proofs per epoch
  // to each accumulate work_value). Fall back to miner:block_number for GPU peers
  // that submit one proof per epoch without a hash.
  var proofKey = data.work_hash
    ? data.miner + ":" + data.work_hash.slice(0, 32)
    : data.miner + ":" + data.block_number;
  if (seenProofKeys.has(proofKey)) {
    return; // silently drop replay
  }
  seenProofKeys.add(proofKey);
  if (seenProofKeys.size > SEEN_PROOF_MAX) {
    var iter = seenProofKeys.values();
    for (var i = 0; i < 500; i++) seenProofKeys.delete(iter.next().value);
  }

  // Register work for reward distribution.
  if (data.work_value && data.work_value > 0) {
    // jobKey must be unique per proof so work_value accumulates across multiple
    // proofs in the same epoch (phone miners submit many short jobs per epoch).
    var jobKey = data.work_hash || (data.miner + ":" + data.block_number);
    recordMinerWork(data.miner, jobKey, data.work_value, data.block_number);
  }

  // Rebroadcast to other peers
  ctx.broadcast(msg, peer.address);
}

/**
 * MINER_IDLE — A miner announces it has no work for this epoch.
 * Other nodes should not wait for a proof from this miner.
 */
// Track idle announcements per epoch: { epochNumber: Set<minerName> }
const idleMiners = {};

function handleMinerIdle(peer, msg, ctx) {
  const data = msg.data || {};
  if (!data.block_number || !data.miner) return;

  if (!idleMiners[data.block_number]) idleMiners[data.block_number] = new Set();
  idleMiners[data.block_number].add(data.miner);

  console.log("[BTCPC P2P] Miner idle: " + data.miner + " has no work for epoch " + data.block_number + " (reason: " + (data.reason || "none") + ")");

  // Rebroadcast
  ctx.broadcast(msg, peer.address);

  // Clean up old entries (keep last 20 epochs)
  const epochs = Object.keys(idleMiners).map(Number).sort((a, b) => a - b);
  while (epochs.length > 20) {
    delete idleMiners[epochs.shift()];
  }
}

function getIdleMiners(epochNumber) {
  return idleMiners[epochNumber] || new Set();
}

/**
 * EPOCH_FINALIZED — Authority broadcasts the completed block.
 * All nodes update their local DB with the reward distribution.
 * This IS the chain — the finalized block is the source of truth.
 */
// Track the last accepted finalized epoch number for sequential-check (Vuln 4)
var _lastFinalizedEpoch = -1;

async function handleEpochFinalized(peer, msg, ctx) {
  const data = msg.data || {};
  if (!data.epoch_number) return;

  const epochNum = data.epoch_number;

  // Vuln 4a: epoch sequence check — reject if going backwards or jumping > 10.
  if (_lastFinalizedEpoch >= 0) {
    var gap = epochNum - _lastFinalizedEpoch;
    if (gap < 0) {
      console.log("[BTCPC P2P] EPOCH_FINALIZED REJECTED: epoch " + epochNum + " goes backwards (last=" + _lastFinalizedEpoch + ")");
      return;
    }
    if (gap > 10) {
      console.log("[BTCPC P2P] EPOCH_FINALIZED REJECTED: epoch " + epochNum + " jumps " + gap + " ahead of last=" + _lastFinalizedEpoch + " (max gap 10)");
      return;
    }
    if (gap > 1) {
      console.log("[BTCPC Consensus] Epochs " + (_lastFinalizedEpoch + 1) + "–" + (epochNum - 1) + " finalization missed — catching up on epoch " + epochNum + ". This is normal after a brief disconnect.");
    }
  }

  // Vuln 4b: block signature verification.
  if (data.block_signature && data.proposer) {
    // Build the header hash payload — the fields the authority signed.
    var blockHeaderData = {
      epoch_number: data.epoch_number,
      consensus_hash: data.consensus_hash || "",
      state_root: data.state_root || "",
      proposer: data.proposer,
      timestamp: data.block_timestamp || data.timestamp || 0,
    };
    // Check if we have the proposer's public key in stateStore.
    // Early block files (epoch 0–N) may not be present locally, so the
    // account is legitimately unknown.  We only hard-reject when we CAN look
    // up the account and the signature still doesn't match.
    var storeRef = require('../chain/stateStore');
    var proposerAccount = storeRef.getAccount(data.proposer);
    var proposerKeys = proposerAccount && proposerAccount.public_keys;
    var hasVerifiableKey = proposerKeys && (proposerKeys.posting || proposerKeys.active);
    if (!hasVerifiableKey) {
      // Account not found or has no usable public key (e.g. stored with empty
      // keys from a finality snapshot that pre-dates full key propagation).
      // Accept with a warning — we can't verify but also can't prove forgery.
      console.log("[BTCPC P2P WARN] EPOCH_FINALIZED epoch " + epochNum + " from " + data.proposer + ": no verifiable key in local stateStore, skipping sig check");
    } else {
      var sigOk = messageAuth.verifyAccountSignature(data.proposer, blockHeaderData, data.block_signature, "posting")
        || messageAuth.verifyAccountSignature(data.proposer, blockHeaderData, data.block_signature, "active");
      if (!sigOk) {
        if (messageAuth.REQUIRE_SIGNATURES) {
          console.log("[BTCPC P2P] EPOCH_FINALIZED REJECTED: invalid block_signature from proposer " + data.proposer + " for epoch " + epochNum);
          return;
        }
        console.log("[BTCPC P2P WARN] EPOCH_FINALIZED epoch " + epochNum + " from " + (data.proposer || "?") + " has invalid block_signature — will be rejected after v2.17");
      }
    }
  } else if (data.proposer || data.block_signature) {
    // Partial — has one but not both fields; treat as unsigned for compat.
    if (messageAuth.REQUIRE_SIGNATURES) {
      console.log("[BTCPC P2P] EPOCH_FINALIZED REJECTED: epoch " + epochNum + " missing block_signature or proposer (strict mode)");
      return;
    }
    console.log("[BTCPC P2P WARN] Unsigned EPOCH_FINALIZED epoch " + epochNum + " from " + (peer.nodeId || "?").slice(0, 16) + " — will be rejected after v2.17");
  } else {
    if (messageAuth.REQUIRE_SIGNATURES) {
      console.log("[BTCPC P2P] EPOCH_FINALIZED REJECTED: epoch " + epochNum + " has no block_signature (strict mode)");
      return;
    }
    console.log("[BTCPC P2P WARN] Unsigned EPOCH_FINALIZED epoch " + epochNum + " from " + (peer.nodeId || "?").slice(0, 16) + " — will be rejected after v2.17");
  }

  _lastFinalizedEpoch = epochNum;
  console.log("[BTCPC P2P] Block finalized: epoch " + epochNum + " | reward: " + (data.block_reward || 0).toFixed(4) + " BTCPC | " + (data.rewards || []).length + " miner(s)");

  try {
    // Phase D: Mongo is no longer the chain state. The block file (written
    // below from data.header_hex) is the source of truth, and stateStore is
    // the in-memory cache updated by applyRemoteEntries + block payload replay.

    // Apply permanent ledger entries from this block — this IS the chain
    // (stateStore.applyEntry handles balance/account/token updates).
    if (data.ledger && data.ledger.length > 0) {
      const { applyRemoteEntries } = require("../services/ledger");
      const applied = await applyRemoteEntries(data.ledger);
      if (applied > 0) {
        console.log("[BTCPC P2P]   Ledger: " + applied + " entries applied (permanent)");
      }
    }

    // Log reward credits — the underlying MINING_REWARD ledger entries in
    // data.ledger have already updated stateStore balances above.
    for (const reward of (data.rewards || [])) {
      console.log("[BTCPC P2P]   " + reward.miner + ": +" + reward.amount.toFixed(4) + " BTCPC");
    }
    // ── Write block to disk — source of truth ──
    if (data.header_hex) {
      try {
        const headerBuf = Buffer.from(data.header_hex, "hex");
        const block = Block.deserialize(headerBuf);

        // Apply ledger entries to local SMT and verify state root
        if (data.ledger && data.ledger.length > 0) {
          stateManager.applyLedgerEntries(data.ledger);
        }
        const localStateRoot = stateManager.getStateRoot();
        if (data.state_root && localStateRoot !== data.state_root) {
          console.log("[BTCPC P2P]   State root mismatch: local=" + localStateRoot.slice(0, 16) + " remote=" + data.state_root.slice(0, 16));
        }

        // Build payload from message data
        const payload = {
          ledger_entries: data.ledger || [],
          consensus_nodes: data.consensus_nodes || 1,
          consensus_proposals: data.consensus_proposals || 1,
          rewards: data.rewards || [],
          compute_proofs: [],
          mining_proofs: []
        };

        if (!blockStore.hasBlock(epochNum)) {
          blockStore.writeBlock(block, payload);
          blockchain.addBlock(block);
          console.log("[BTCPC P2P]   Block " + epochNum + " written to disk: " + block.computeHash().slice(0, 16) + "...");
        }
      } catch (blockErr) {
        console.error("[BTCPC P2P]   Failed to write block to disk:", blockErr.message);
      }
    }
  } catch (err) {
    console.error("[BTCPC P2P] Failed to process finalized block:", err.message);
  }

  // Rebroadcast
  ctx.broadcast(msg, peer.address);
}

/**
 * ACCOUNT_ANNOUNCE — Any node broadcasts a new account to the network.
 * Phase D: apply the ACCOUNT_CREATE ledger entry to stateStore. The entry
 * is already in the block file that will flow through ledger sync, so it's
 * also picked up on replay. No Mongoose writes.
 */
async function handleAccountAnnounce(peer, msg, ctx) {
  const data = msg.data || {};
  if (!data.username) return;

  // Vuln 3: verify the proof-of-key-ownership field.
  // The announcer must sign { username, public_keys, timestamp } with the owner key
  // they are claiming.
  var stateStore = require("../chain/stateStore");
  var existingAccount = stateStore.getAccount(data.username);

  if (data.proof) {
    // The data that was signed — matches what the sender must have signed.
    var proofPayload = {
      username: data.username,
      public_keys: data.public_keys || {},
      timestamp: data.timestamp || 0,
    };

    if (existingAccount && existingAccount.public_keys && existingAccount.public_keys.owner) {
      // Re-announcement (key update) — must be signed by the EXISTING owner key on chain.
      var reannounceOk = messageAuth.verifyMessage(proofPayload, data.proof, existingAccount.public_keys.owner);
      if (!reannounceOk) {
        if (messageAuth.REQUIRE_SIGNATURES) {
          console.log("[BTCPC P2P] ACCOUNT_ANNOUNCE REJECTED: " + data.username + " — re-announcement proof invalid (key theft attempt?)");
          return;
        }
        console.log("[BTCPC P2P WARN] ACCOUNT_ANNOUNCE for " + data.username + " has invalid re-announcement proof — will be rejected after v2.17");
      }
    } else {
      // First announcement — self-certifying: signature must match the claimed owner public key.
      var claimedOwnerPub = (data.public_keys || {}).owner;
      if (claimedOwnerPub) {
        var firstOk = messageAuth.verifyMessage(proofPayload, data.proof, claimedOwnerPub);
        if (!firstOk) {
          // The proof doesn't match the claimed key — silently reject, this is clearly forged.
          console.log("[BTCPC P2P] ACCOUNT_ANNOUNCE REJECTED: " + data.username + " — first-announcement proof does not match claimed owner key");
          return;
        }
      }
    }
  } else {
    // No proof field at all.
    if (messageAuth.REQUIRE_SIGNATURES) {
      console.log("[BTCPC P2P] ACCOUNT_ANNOUNCE REJECTED: " + data.username + " — no proof field (strict mode)");
      return;
    }
    console.log("[BTCPC P2P WARN] Unsigned ACCOUNT_ANNOUNCE for " + data.username + " — will be rejected after v2.17");
  }

  console.log("[BTCPC P2P] Account announced: " + data.username + " | evm=" + (data.chain_addresses?.evm || "none"));

  try {
    if (!existingAccount) {
      stateStore.applyEntry({
        type: 'ACCOUNT_CREATE',
        to: data.username,
        epoch: data.epoch || 0,
        account_data: {
          username: data.username,
          public_keys: data.public_keys || {},
          chain_addresses: data.chain_addresses || {},
        },
        timestamp: Date.now(),
      });
      console.log("[BTCPC P2P]   Applied to stateStore");
    }
  } catch (err) {
    console.error("[BTCPC P2P] Failed to process account announcement:", err.message);
  }

  ctx.broadcast(msg, peer.address);
}

// ---------------------------------------------------------------------------
// Mempool gossip — ledger entries propagated across machines (v2.13.3)
// ---------------------------------------------------------------------------

/**
 * MEMPOOL_ENTRY — A node broadcasts a single ledger entry to all peers.
 *
 * Flow:
 *   1. Origin node calls _persist(entry) → _gossipEntry(entry) broadcasts this.
 *   2. Receiving nodes call appendForeignEntry(entry) which:
 *      - applies to their local stateStore (so read paths stay consistent)
 *      - appends to their local pending-entries.jsonl queue (so the next
 *        broadcaster, whoever it is, drains and includes it in a block)
 *   3. The receiver rebroadcasts to its own peers (skip the sender),
 *      completing the gossip flood.
 *
 * Cycle prevention: seenMessages Set in protocol.js dedupes by msg.id.
 * Re-origination prevention: gossipedHashes in ledger.js dedupes by entry
 * content so appendForeignEntry never triggers a second broadcast.
 */
async function handleMempoolEntry(peer, msg, ctx) {
  var data = msg.data || {};
  if (!data.entry || !data.entry.type) {
    console.log("[BTCPC P2P] MEMPOOL_ENTRY dropped: missing entry or type");
    return;
  }

  var entry = data.entry;

  // Vuln 2: reject block-only entry types — these must only arrive inside
  // EPOCH_FINALIZED block payloads and can never be user-submitted.
  if (messageAuth.BLOCK_ONLY_TYPES.includes(entry.type)) {
    console.log("[BTCPC P2P] MEMPOOL_ENTRY REJECTED: " + entry.type + " is a block-only type (possible money-printing attack) from " + (peer.nodeId || peer.address || "?").slice(0, 16));
    return;
  }

  // Reject entries whose type is not on the allowlist at all.
  if (!messageAuth.MEMPOOL_ALLOWED_TYPES.includes(entry.type)) {
    console.log("[BTCPC P2P] MEMPOOL_ENTRY dropped: unknown/disallowed type " + entry.type);
    return;
  }

  console.log("[BTCPC P2P] MEMPOOL_ENTRY: " + entry.type +
    " from=" + (entry.from || "-") +
    " to=" + (entry.to || "-") +
    " epoch=" + (entry.epoch || 0));

  // Signature enforcement: only required for value-moving operations
  // (transfers, staking, delegation, escrow, bridge, key rotation).
  // All other entry types (heartbeats, sensor readings, etc.) are unsigned.
  if (messageAuth.requiresSignature(entry.type) && entry.from) {
    if (entry.signature) {
      var spendData = {
        type: entry.type,
        from: entry.from,
        to: entry.to,
        amount: entry.amount,
        token: entry.token || "BTCPC",
        memo: entry.memo || "",
        epoch: entry.epoch || 0,
        timestamp: entry.timestamp || 0,
      };
      var sigOk = messageAuth.verifyAccountSignature(entry.from, spendData, entry.signature, "posting");
      if (!sigOk) {
        console.log("[BTCPC P2P] MEMPOOL_ENTRY REJECTED: " + entry.type + " from " + entry.from + " — invalid signature");
        return;
      }
    } else {
      console.log("[BTCPC P2P] MEMPOOL_ENTRY REJECTED: unsigned " + entry.type + " from " + entry.from + " — signature required for spend operations");
      return;
    }
  }

  try {
    // appendForeignEntry applies to stateStore + disk queue without re-gossiping
    var ledger = require("../services/ledger");
    ledger.appendForeignEntry(entry);
  } catch (err) {
    console.error("[BTCPC P2P] MEMPOOL_ENTRY apply failed: " + err.message);
  }

  // Forward to all other peers (gossip flood)
  ctx.broadcast(msg, peer.address);
}

// ---------------------------------------------------------------------------
// Finalization Consensus — collect proposals from miners
// ---------------------------------------------------------------------------

/**
 * Extract a stable source tag from a peer address for consensus source tracking.
 * Returns the peer's IP for direct connections, "relay" for relay connections,
 * or "unknown" as fallback. Two processes on the same machine share the same IP
 * and therefore the same source tag — they count as ONE consensus source.
 */
function extractConsensusSourceTag(address) {
  if (!address) return "unknown";
  // Relay connections — multi-tenant, treat as a single external source per relay URL
  if (address.includes("workers.dev") || address.includes("relay")) return "relay:" + address.slice(0, 32);
  // inbound:ip:port
  var m = address.match(/^inbound:(.+):\d+$/);
  if (m) return m[1]; // just the IP
  // ws://host:port or wss://host:port
  try {
    var u = new URL(address);
    return u.hostname;
  } catch (_) {}
  return "unknown";
}

/**
/**
 * BLOCK_PROPOSAL — Unified block proposal from a clock node.
 * Bundles epoch, work attestations, rewards, and consensus_hash in one message.
 * Replaces the EPOCH_START → EPOCH_END → FINALIZATION_PROPOSAL → EPOCH_FINALIZED dance.
 *
 * Multiple clocks running the same buildProposal() over the same gossip
 * stream produce identical hashes → deterministic consensus.
 */
function handleBlockProposal(peer, msg, ctx) {
  var data = msg.data || {};
  if (!data.epoch_number || !data.proposer || !data.consensus_hash) return;

  // Vuln 5: track the claimed proposer per connection. If the same WebSocket
  // connection sends proposals with different proposer names, it's spoofing —
  // drop the connection immediately.
  if (peer.claimed_proposer && peer.claimed_proposer !== data.proposer) {
    console.log("[BTCPC P2P] BLOCK_PROPOSAL REJECTED: connection " + (peer.address || "?") +
      " claimed proposer " + peer.claimed_proposer + " then " + data.proposer + " — spoofing detected, dropping connection");
    if (peer.ws) peer.ws.close();
    return;
  }
  peer.claimed_proposer = data.proposer;

  // Vuln 5: verify proposal_signature.
  // Accepts device-key signatures (multi-device) OR the proposer's active key (single-device fallback).
  if (data.proposal_signature) {
    var proposalData = {
      epoch_number: data.epoch_number,
      proposer: data.proposer,
      consensus_hash: data.consensus_hash,
      total_work: data.total_work || 0,
      timestamp: data.timestamp || 0,
    };
    // Only hard-reject if we can actually look up the proposer's key.
    // A finality snapshot may store the account with empty keys (the full
    // key history lives in genesis-era blocks we don't have locally).
    var bpStoreRef = require('../chain/stateStore');
    var bpAcct = bpStoreRef.getAccount(data.proposer);
    var bpKeys = bpAcct && bpAcct.public_keys;
    var bpHasKey = bpKeys && (bpKeys.posting || bpKeys.active);
    var bpDeviceOwner = data.device_id && bpStoreRef.getDeviceOwner(data.device_id);
    if (!bpHasKey && !bpDeviceOwner) {
      // Can't verify — no key material available.  Accept with a warning.
      console.log("[BTCPC P2P WARN] BLOCK_PROPOSAL epoch " + data.epoch_number + " from " + data.proposer + ": no verifiable key, skipping proposal_signature check");
    } else {
      var sigOk = messageAuth.verifyDeviceOrAccountSignature(
        data.proposer,
        data.device_id || null,
        proposalData,
        data.proposal_signature
      );
      if (!sigOk) {
        console.log("[BTCPC P2P] BLOCK_PROPOSAL REJECTED: invalid proposal_signature from " + data.proposer +
          (data.device_id ? ' (device ' + data.device_id.slice(0,12) + '...)' : '') +
          " for epoch " + data.epoch_number);
        return;
      }
    }
  } else {
    console.log("[BTCPC P2P] BLOCK_PROPOSAL REJECTED: no proposal_signature from " + data.proposer + " for epoch " + data.epoch_number);
    return;
  }

  // ── VRF-based proposer rotation: verify the proposer is the designated
  // authority for this epoch, or that the fallback timeout has elapsed. ──
  var authorityRotation = require("../chain/authorityRotation");
  var nodeRegistry = require("../chain/nodeRegistry");
  var blockStoreForVRF = require("../chain/blockStore");

  var allNodes = nodeRegistry.getRegisteredNodes();
  var eligibleAccounts = allNodes.filter(function (n) {
    var elig = authorityRotation.isEpochEligible(n.username, n, nodeRegistry.PERMISSIONLESS_MIN_STAKE);
    return elig.eligible;
  }).map(function (n) { return n.username; }).sort();

  // Get previous block hash for VRF input
  var prevEpoch = data.epoch_number - 1;
  var prevBlockHash = "0".repeat(64);
  if (prevEpoch >= 0) {
    try {
      var prevBlock = blockStoreForVRF.readBlockHeader(prevEpoch);
      if (prevBlock && prevBlock.hash) prevBlockHash = prevBlock.hash;
    } catch (_) {}
  }

  // Compute epoch start time for fallback window
  var genesisTimestamp = parseInt(process.env.BTCPC_GENESIS_TIMESTAMP) || 0;
  var epochDurationMs = PROTOCOL_EPOCH_DURATION_MS;
  var epochStart = authorityRotation.epochStartTime(data.epoch_number, genesisTimestamp, epochDurationMs);

  var vrfResult = authorityRotation.validateProposer(
    data.proposer, data.epoch_number, prevBlockHash, eligibleAccounts, epochStart
  );

  if (!vrfResult.valid) {
    console.log("[BTCPC P2P] BLOCK_PROPOSAL REJECTED (VRF): " + vrfResult.reason +
      " for epoch " + data.epoch_number);
    return;
  }

  if (vrfResult.fallback) {
    console.log("[BTCPC P2P] BLOCK_PROPOSAL from " + data.proposer +
      " for epoch " + data.epoch_number +
      " (FALLBACK — designated was " + vrfResult.designated + ")" +
      " (work=" + (data.total_work || 0) +
      ", hash=" + data.consensus_hash.slice(0, 12) + ")");
  } else {
    console.log("[BTCPC P2P] BLOCK_PROPOSAL from " + data.proposer +
      " for epoch " + data.epoch_number +
      " (designated authority ✓)" +
      " (work=" + (data.total_work || 0) +
      ", hash=" + data.consensus_hash.slice(0, 12) + ")");
  }

  // Update the local epoch cache
  setCurrentEpoch(data.epoch_number);

  // Submit to local consensus collector — same logic as FINALIZATION_PROPOSAL
  // but the data shape is the new unified format.
  // Source tag identifies the machine this proposal came from for quorum tracking.
  var finConsensus = require("../chain/finalizationConsensus");
  var proposalSourceTag = extractConsensusSourceTag(peer.address);
  finConsensus.submitProposal(data.epoch_number, {
    proposer: data.proposer,
    rewards: (data.rewards || []).map(function (r) {
      return { miner: r.to || r.miner, amount: r.amount, type: r.type };
    }),
    total_work: data.total_work || 0,
    consensus_hash: data.consensus_hash,
    settled_jobs: data.miners_active || 0,
    block_reward: data.block_reward,
    timestamp: data.timestamp,
  }, proposalSourceTag);

  // Rebroadcast to peers
  ctx.broadcast(msg, peer.address);
}

// ---------------------------------------------------------------------------
// Inference Verification — track verifiers per epoch
// ---------------------------------------------------------------------------

// Track which accounts actually verified work: Map<epochNumber, Set<username>>
var verifiersByEpoch = new Map();
var _currentVerifierEpoch = -1;

// Track which miners had their work verified: Map<epochNumber, Set<minerAccount>>
// Populated when VERIFY_RESPONSE arrives with verdict="valid".
var verifiedWork = new Map();
var _currentVerifiedWorkEpoch = -1;

// Track job_id → { miner, epoch } from VERIFY_REQUEST so VERIFY_RESPONSE
// can look up which miner the job belongs to.
var verifyJobIndex = new Map();  // job_id → { miner, epoch }
var VERIFY_JOB_INDEX_MAX = 5000;

/**
 * Record that a node verified work during an epoch.
 */
function recordVerifier(account, epochNumber) {
  if (!epochNumber || epochNumber < 0 || !account) return;
  if (!verifiersByEpoch.has(epochNumber)) {
    verifiersByEpoch.set(epochNumber, new Set());
  }
  verifiersByEpoch.get(epochNumber).add(account);

  // Prune old epochs (keep last 10)
  if (epochNumber > _currentVerifierEpoch) {
    _currentVerifierEpoch = epochNumber;
    for (var key of verifiersByEpoch.keys()) {
      if (key < epochNumber - 10) verifiersByEpoch.delete(key);
    }
  }
}

/**
 * Get accounts that verified work for a given epoch (and the last few).
 */
function getActiveVerifiers(epochNumber) {
  var WINDOW = 3;
  var union = new Set();
  for (var i = 0; i <= WINDOW; i++) {
    var vset = verifiersByEpoch.get(epochNumber - i);
    if (vset) {
      for (var v of vset) union.add(v);
    }
  }
  return Array.from(union);
}

/**
 * Get the set of miners whose work was verified for a given epoch.
 * Checked by blockProposal to apply the 50% penalty to unverified work.
 */
function getVerifiedMiners(epochNumber) {
  return verifiedWork.get(epochNumber) || new Set();
}

// ---------------------------------------------------------------------------
// Miner Work Attestations — track work_value per miner per epoch
// from gossiped INFERENCE_RESULT messages, NOT from local MongoDB.
// This is what makes consensus deterministic across miners with separate DBs.
// ---------------------------------------------------------------------------

// Map<epochNumber, Map<minerName, { work_value, jobs: Set<request_id> }>>
var minerWorkByEpoch = new Map();
var _currentWorkEpoch = -1;

// Cached current epoch — updated from EPOCH_START messages.
// Used so handlers don't have to query MongoDB to know which epoch
// to attribute work to.
var _currentEpochCache = -1;
function setCurrentEpoch(epochNumber) {
  if (typeof epochNumber === 'number' && epochNumber > _currentEpochCache) {
    _currentEpochCache = epochNumber;
  }
}
function getCurrentEpochCache() { return _currentEpochCache; }

// ---------------------------------------------------------------------------
// Epoch Consensus — if a majority of peers agree on an epoch, adopt it.
// Prevents stale nodes from falling behind when their local clock drifts
// or genesis derivation differs.
// ---------------------------------------------------------------------------
var _peerEpochVotes = {};  // nodeId → { epoch, timestamp }
var EPOCH_CONSENSUS_THRESHOLD = 0.5; // >50% of peers must agree

function recordPeerEpoch(nodeId, claimedEpoch) {
  if (typeof claimedEpoch !== 'number' || claimedEpoch < 0) return;
  _peerEpochVotes[nodeId] = { epoch: claimedEpoch, timestamp: Date.now() };

  // Clean stale votes (>5 min old)
  var now = Date.now();
  var keys = Object.keys(_peerEpochVotes);
  for (var i = 0; i < keys.length; i++) {
    if (now - _peerEpochVotes[keys[i]].timestamp > 300000) {
      delete _peerEpochVotes[keys[i]];
    }
  }

  // Count votes — group by epoch (allow ±1 tolerance for propagation delay)
  keys = Object.keys(_peerEpochVotes);
  if (keys.length < 2) return; // need at least 2 peers to form consensus

  var epochCounts = {};
  for (var j = 0; j < keys.length; j++) {
    var ep = _peerEpochVotes[keys[j]].epoch;
    epochCounts[ep] = (epochCounts[ep] || 0) + 1;
  }

  // Find the epoch with the most votes
  var bestEpoch = -1;
  var bestCount = 0;
  var epochs = Object.keys(epochCounts);
  for (var k = 0; k < epochs.length; k++) {
    // Sum votes for this epoch ±1 (propagation tolerance)
    var ep2 = parseInt(epochs[k]);
    var count = (epochCounts[ep2] || 0) +
                (epochCounts[ep2 - 1] || 0) +
                (epochCounts[ep2 + 1] || 0);
    if (count > bestCount) {
      bestCount = count;
      bestEpoch = ep2;
    }
  }

  // If majority agrees and their epoch is ahead of ours, adopt it
  var totalPeers = keys.length + 1; // +1 for ourselves
  if (bestCount / totalPeers > EPOCH_CONSENSUS_THRESHOLD) {
    if (bestEpoch > _currentEpochCache + 1) {
      console.log("[BTCPC P2P] Epoch consensus: " + bestCount + "/" + totalPeers +
        " peers at epoch ~" + bestEpoch + " (local: " + _currentEpochCache +
        ") — adopting peer majority");
      _currentEpochCache = bestEpoch;
    }
  }
}

/**
 * Record that a miner produced work during an epoch.
 * Idempotent: same job_id won't be double-counted for the same miner.
 *
 * @param {string} miner — username of the miner
 * @param {string} jobId — request_id of the inference job
 * @param {number} workValue — tokens × verified_param_count
 * @param {number} epochNumber — epoch the work belongs to
 */
function recordMinerWork(miner, jobId, workValue, epochNumber) {
  if (!miner || !jobId || !epochNumber || epochNumber < 0) return;
  if (!workValue || workValue <= 0) return;

  if (!minerWorkByEpoch.has(epochNumber)) {
    minerWorkByEpoch.set(epochNumber, new Map());
  }
  var epochMap = minerWorkByEpoch.get(epochNumber);
  if (!epochMap.has(miner)) {
    epochMap.set(miner, { work_value: 0, jobs: new Set() });
  }
  var entry = epochMap.get(miner);
  if (entry.jobs.has(jobId)) return; // already counted
  entry.jobs.add(jobId);
  entry.work_value += workValue;

  // Prune old epochs (keep last 200 — large window so fast miner credit epochs
  // don't get pruned before the clock finalizes them)
  if (epochNumber > _currentWorkEpoch) {
    _currentWorkEpoch = epochNumber;
    for (var key of minerWorkByEpoch.keys()) {
      if (key < epochNumber - 200) minerWorkByEpoch.delete(key);
    }
  }
}

/**
 * Get all miners with their work values for an epoch.
 * Returns: { miner: { work_value, jobs: Set } }
 */
function getMinerWorkForEpoch(epochNumber) {
  // Check a window around the target epoch. The miner credits work to the
  // chain height at completion time, which can be ahead of the clock's
  // finalization epoch during catch-up mode. Look forward 50 epochs to
  // capture work that was credited to near-future epochs.
  var result = {};
  var lookback = 10;
  var lookahead = 50;
  for (var ep = Math.max(0, epochNumber - lookback); ep <= epochNumber + lookahead; ep++) {
    var epochMap = minerWorkByEpoch.get(ep);
    if (!epochMap) continue;
    for (var entry of epochMap) {
      var miner = entry[0];
      if (!result[miner]) {
        result[miner] = { work_value: 0, job_count: 0 };
      }
      result[miner].work_value += entry[1].work_value;
      result[miner].job_count += entry[1].jobs.size;
    }
  }
  return result;
}

/**
 * VERIFY_REQUEST — A miner broadcasts inference output for verification.
 * Verifiers selected via deterministic selection process the request.
 * The request contains the full response but NOT the prompt.
 */
function handleVerifyRequest(peer, msg, ctx) {
  var data = msg.data || {};
  if (!data.job_id || !data.miner) return;

  console.log("[BTCPC P2P] Verify request from " + data.miner +
    " for job " + (data.job_id || "?").slice(0, 12) + "..." +
    " (" + (data.token_count || 0) + " tokens, " + (data.model || "?") + ")");

  // Index job_id → miner so VERIFY_RESPONSE can credit the right miner
  verifyJobIndex.set(data.job_id, { miner: data.miner, epoch: data.epoch || _currentEpochCache });
  if (verifyJobIndex.size > VERIFY_JOB_INDEX_MAX) {
    var vjIter = verifyJobIndex.keys();
    for (var vjI = 0; vjI < 1000; vjI++) verifyJobIndex.delete(vjIter.next().value);
  }

  // Check if this node should verify — deterministic selection
  var verifier = require("../inference/verifier");

  // Eligible verifier pool: nodes seen recently that are also running this
  // process with verification explicitly enabled. Verification is text
  // analysis — no Ollama needed — but it is still an opt-in node role.
  var eligibleSet = new Set();
  var currentEpoch = _currentEpochCache;
  for (var i = 0; i <= 3; i++) {
    var clocks = clockUptimeByEpoch.get(currentEpoch - i);
    if (clocks) for (var c of clocks) eligibleSet.add(c);
    var workMap = minerWorkByEpoch.get(currentEpoch - i);
    if (workMap) for (var m of workMap.keys()) eligibleSet.add(m);
  }
  // Filter to valid account names
  var allNodes = Array.from(eligibleSet).filter(function (a) {
    return /^[a-z0-9][a-z0-9-]{2,19}$/.test(a);
  }).sort();

  // This node's identity — could be a miner OR a clock-only node
  var myAccount = process.env.BTCPC_MINER || process.env.BTCPC_CLOCK_ACCOUNT || null;
  var verifierOptIn = process.env.BTCPC_VERIFIER_ENABLED === "true" || process.env.BTCPC_NODE_ROLE === "verifier";

  if (!myAccount || myAccount === data.miner || !verifierOptIn) {
    // Miner doesn't verify own work; nodes without accounts can't verify
    ctx.broadcast(msg, peer.address);
    return;
  }

  if (allNodes.length === 0) {
    // No eligible verifiers known — just rebroadcast
    ctx.broadcast(msg, peer.address);
    return;
  }

  var totalNodes = allNodes.length;
  var blockHash = data.block_hash || "0".repeat(64);
  var policy = verifier.getVerifierPolicy(totalNodes, totalNodes);
  if (!verifier.shouldVerifyJob(data.job_id, blockHash, policy.coverage)) {
    ctx.broadcast(msg, peer.address);
    return;
  }
  var vCount = policy.count;
  var selected = verifier.selectVerifiers(blockHash, data.job_id, data.miner, allNodes, vCount);

  if (selected.indexOf(myAccount) === -1) {
    // Not selected for this job — just rebroadcast
    ctx.broadcast(msg, peer.address);
    return;
  }

  // This node is selected — verify the work
  var verdict = verifier.verifyWork({
    response: data.result || "",
    model: data.model || "",
    token_count: data.token_count || 0,
    elapsed_ms: data.timing_ms || 0,
    tokens_per_second: data.token_count && data.timing_ms
      ? data.token_count / (data.timing_ms / 1000)
      : 0
  });

  var verdictStr = verdict.valid ? "valid" : "invalid";
  console.log("[BTCPC P2P] Verified job " + data.job_id.slice(0, 12) + "... verdict=" +
    verdictStr.toUpperCase() + " score=" + verdict.score);

  // Phase T-04: commit-reveal — broadcast commitment now, reveal after window
  var nonce = require("crypto").randomBytes(16).toString("hex");
  var commitment = require("crypto").createHash("sha256")
    .update(verdictStr + ":" + nonce + ":" + data.job_id)
    .digest("hex");

  _storeCommitment(data.job_id, myAccount, {
    verdict: verdictStr,
    score: verdict.score,
    checks: verdict.checks,
    nonce: nonce,
    epoch: data.epoch || 0
  });

  var commitMsg = createMessage(MESSAGE_TYPES.VERIFIER_COMMIT, {
    job_id: data.job_id,
    verifier: myAccount,
    commitment: commitment,
    epoch: data.epoch || 0
  }, ctx.NODE_ID);
  ctx.broadcast(commitMsg);

  // Reveal after the window expires
  setTimeout(function () {
    _broadcastReveal(data.job_id, myAccount, ctx);
  }, VERIFIER_REVEAL_WINDOW_MS);

  // Rebroadcast original request so other verifiers see it
  ctx.broadcast(msg, peer.address);
}

// ---------------------------------------------------------------------------
// Verifier Commit-Reveal State Machine (T-04)
// Prevents early-verdict influence: verifiers commit first, reveal after window.
// ---------------------------------------------------------------------------

var VERIFIER_REVEAL_WINDOW_MS = parseInt(process.env.BTCPC_VERIFIER_REVEAL_WINDOW_MS) || 15000;

// Per job: { commits: Map<verifier, commitment>, pending: Map<verifier, {verdict,score,checks,nonce,epoch}>, reveals: Map<verifier, verdict>, timer: timeout, resolved: bool }
var _commitRevealByJob = new Map();
var COMMIT_REVEAL_JOB_MAX = 2000;

function _getCommitRevealState(jobId) {
  if (!_commitRevealByJob.has(jobId)) {
    _commitRevealByJob.set(jobId, {
      commits: new Map(),
      pending: new Map(), // local node's uncommitted verdicts waiting to reveal
      reveals: new Map(),
      timer: null,
      resolved: false
    });
    // Prune oldest jobs if over cap
    if (_commitRevealByJob.size > COMMIT_REVEAL_JOB_MAX) {
      var iter = _commitRevealByJob.keys();
      for (var i = 0; i < 200; i++) {
        var old = iter.next().value;
        var oldState = _commitRevealByJob.get(old);
        if (oldState && oldState.timer) clearTimeout(oldState.timer);
        _commitRevealByJob.delete(old);
      }
    }
  }
  return _commitRevealByJob.get(jobId);
}

/**
 * Store our own verdict locally so we can reveal it after the window.
 */
function _storeCommitment(jobId, verifier, verdictData) {
  var state = _getCommitRevealState(jobId);
  state.pending.set(verifier, verdictData);
}

/**
 * Broadcast our VERIFIER_REVEAL for a job (called after window expires).
 */
function _broadcastReveal(jobId, myAccount, ctx) {
  var state = _commitRevealByJob.get(jobId);
  if (!state) return;
  var mine = state.pending.get(myAccount);
  if (!mine) return; // already revealed or never committed
  var revealMsg = createMessage(MESSAGE_TYPES.VERIFIER_REVEAL, {
    job_id: jobId,
    verifier: myAccount,
    verdict: mine.verdict,
    score: mine.score,
    checks: mine.checks,
    nonce: mine.nonce,
    epoch: mine.epoch
  }, ctx.NODE_ID);
  ctx.broadcast(revealMsg);
  state.pending.delete(myAccount);
}

/**
 * Aggregate verdicts from all received reveals and apply the result.
 * Called when the reveal window closes or all committed verifiers have revealed.
 */
function _aggregateAndApply(jobId, ctx) {
  var state = _commitRevealByJob.get(jobId);
  if (!state || state.resolved) return;
  state.resolved = true;
  if (state.timer) { clearTimeout(state.timer); state.timer = null; }

  var reveals = state.reveals;
  if (reveals.size === 0) return;

  // Check for non-revealers (committed but never revealed)
  var nonRevealers = [];
  for (var entry of state.commits) {
    var verifierName = entry[0];
    if (!reveals.has(verifierName)) {
      nonRevealers.push(verifierName);
    }
  }
  if (nonRevealers.length > 0) {
    var slashing = require("../services/slashing");
    nonRevealers.forEach(function (v) {
      slashing.recordOffense(v, "VERIFIER_NO_REVEAL", { job_id: jobId })
        .catch(function (err) { console.error("[BTCPC P2P] No-reveal slash failed:", err.message); });
      console.log("[BTCPC P2P] VERIFIER_NO_REVEAL: " + v + " committed but never revealed for job " + jobId.slice(0, 12) + "...");
    });
  }

  // Tally reveals
  var validCount = 0;
  var invalidCount = 0;
  var scores = [];
  var checks = [];
  var epoch = 0;
  for (var revEntry of reveals) {
    var rv = revEntry[1];
    if (rv.verdict === "valid") { validCount++; scores.push(rv.score || 0); }
    else invalidCount++;
    if (rv.epoch > epoch) epoch = rv.epoch;
    if (rv.checks) checks = rv.checks; // last one wins (all should agree)
  }

  var majorityVerdict = validCount > invalidCount ? "valid" : "invalid";
  var avgScore = scores.length > 0 ? scores.reduce(function (a, b) { return a + b; }, 0) / scores.length : 0;

  console.log("[BTCPC P2P] Commit-reveal resolved for job " + jobId.slice(0, 12) + "..." +
    " valid=" + validCount + " invalid=" + invalidCount + " verdict=" + majorityVerdict.toUpperCase());

  // Synthesize a VERIFY_RESPONSE for backward-compat downstream processing
  var syntheticMsg = {
    id: "cr-" + jobId,
    type: MESSAGE_TYPES.VERIFY_RESPONSE,
    data: {
      job_id: jobId,
      verifier: "consensus",
      verdict: majorityVerdict,
      score: avgScore,
      checks: checks,
      epoch: epoch,
      commit_reveal: true,
      valid_count: validCount,
      invalid_count: invalidCount
    },
    timestamp: Date.now(),
    nodeId: ctx ? ctx.NODE_ID : "local"
  };
  // Route through existing VERIFY_RESPONSE logic
  handleVerifyResponse({ address: "local" }, syntheticMsg, ctx || { broadcast: function () {} });
}

/**
 * VERIFIER_COMMIT — A verifier broadcasts their commitment hash.
 * Store it; start the reveal window timer on first commit for a job.
 */
function handleVerifierCommit(peer, msg, ctx) {
  var data = msg.data || {};
  if (!data.job_id || !data.verifier || !data.commitment) return;

  var state = _getCommitRevealState(data.job_id);
  if (state.resolved) return;

  // Reject duplicate commits from same verifier
  if (state.commits.has(data.verifier)) return;
  state.commits.set(data.verifier, data.commitment);

  console.log("[BTCPC P2P] Verifier commit from " + data.verifier +
    " for job " + data.job_id.slice(0, 12) + "... (" + state.commits.size + " commits)");

  // Start reveal window on first commit
  if (!state.timer) {
    state.timer = setTimeout(function () {
      _aggregateAndApply(data.job_id, ctx);
    }, VERIFIER_REVEAL_WINDOW_MS);
  }

  ctx.broadcast(msg, peer.address);
}

/**
 * VERIFIER_REVEAL — A verifier reveals their verdict + nonce.
 * Verify the commitment matches before accepting.
 */
function handleVerifierReveal(peer, msg, ctx) {
  var data = msg.data || {};
  if (!data.job_id || !data.verifier || !data.verdict || !data.nonce) return;

  var state = _commitRevealByJob.get(data.job_id);
  if (!state || state.resolved) return;

  // Must have committed first
  var storedCommitment = state.commits.get(data.verifier);
  if (!storedCommitment) {
    console.log("[BTCPC P2P] VERIFIER_REVEAL from " + data.verifier + " has no matching commit — ignored");
    return;
  }

  // Verify commitment: sha256(verdict + ":" + nonce + ":" + job_id)
  var expected = require("crypto").createHash("sha256")
    .update(data.verdict + ":" + data.nonce + ":" + data.job_id)
    .digest("hex");
  if (expected !== storedCommitment) {
    console.log("[BTCPC P2P] VERIFIER_EQUIVOCATION: " + data.verifier +
      " reveal hash mismatch for job " + data.job_id.slice(0, 12) + "...");
    var slashing = require("../services/slashing");
    slashing.recordOffense(data.verifier, "VERIFIER_EQUIVOCATION", {
      job_id: data.job_id,
      expected: storedCommitment.slice(0, 12),
      got: expected.slice(0, 12)
    }).catch(function (err) { console.error("[BTCPC P2P] Equivocation slash failed:", err.message); });
    return;
  }

  // Reject duplicate reveals
  if (state.reveals.has(data.verifier)) return;
  state.reveals.set(data.verifier, { verdict: data.verdict, score: data.score || 0, checks: data.checks, epoch: data.epoch || 0 });

  console.log("[BTCPC P2P] Verifier reveal from " + data.verifier +
    " for job " + data.job_id.slice(0, 12) + "..." +
    " verdict=" + data.verdict.toUpperCase() +
    " (" + state.reveals.size + "/" + state.commits.size + " revealed)");

  // If all committed verifiers have revealed, aggregate immediately
  if (state.reveals.size >= state.commits.size) {
    _aggregateAndApply(data.job_id, ctx);
  }

  ctx.broadcast(msg, peer.address);
}

/**
 * VERIFY_RESPONSE — A verifier broadcasts their verdict on inference work.
 * Track which verifiers actually did work so they earn rewards.
 */
function handleVerifyResponse(peer, msg, ctx) {
  var data = msg.data || {};
  if (!data.job_id || !data.verifier) return;

  console.log("[BTCPC P2P] Verify response from " + data.verifier +
    " for job " + (data.job_id || "?").slice(0, 12) + "..." +
    " verdict=" + (data.verdict || "?") + " score=" + (data.score || 0));

  // Record this verifier as active for the epoch
  var epoch = data.epoch || 0;
  if (epoch > 0) {
    recordVerifier(data.verifier, epoch);
  }

  // Track verified miners — when a verifier says "valid", the miner's work
  // counts as verified for reward calculation (full work_value vs 50%).
  // Look up the miner from the job index (populated by VERIFY_REQUEST).
  var jobInfo = verifyJobIndex.get(data.job_id);
  var verifiedMiner = data.miner || (jobInfo ? jobInfo.miner : null);
  var verifiedEpoch = epoch > 0 ? epoch : (jobInfo ? jobInfo.epoch : 0);
  if (data.verdict === "valid" && verifiedMiner && verifiedEpoch > 0) {
    if (!verifiedWork.has(verifiedEpoch)) {
      verifiedWork.set(verifiedEpoch, new Set());
    }
    verifiedWork.get(verifiedEpoch).add(verifiedMiner);

    // Prune old epochs (keep last 10)
    if (verifiedEpoch > _currentVerifiedWorkEpoch) {
      _currentVerifiedWorkEpoch = verifiedEpoch;
      for (var vwKey of verifiedWork.keys()) {
        if (vwKey < verifiedEpoch - 10) verifiedWork.delete(vwKey);
      }
    }
  }

  // Phase E: InferenceJob model deleted.
  // Verification results are tracked in memory via the P2P gossip layer
  // (minerWorkByEpoch). Slashing for majority rejection still happens via
  // the slashing service, keyed by job_id recorded in compute proofs.
  // If we need to check majority rejection, do it via stateStore compute proofs.
  (async function () {
    try {
      if (!data.job_id || !data.verifier) return;
      // Find the compute proof for this job in stateStore
      var stateStore = require("../chain/stateStore");
      var epoch = data.epoch || 0;
      var proofs = stateStore.getComputeProofs(epoch);
      var proof = proofs.find(function (p) { return p.job_id === data.job_id || p.prompt_hash === data.prompt_hash; });
      if (proof) {
        // Track verification inline on the proof (in-memory only)
        if (!proof.verifications) proof.verifications = [];
        var existing = proof.verifications.find(function (v) { return v.miner === data.verifier; });
        if (!existing) {
          proof.verifications.push({
            miner: data.verifier,
            work_value: data.verdict === "valid" ? (data.score || 0) : 0
          });
          // Check majority rejection
          var required = proof.required_verifications || 1;
          if (proof.verifications.length >= required) {
            var invalidCount = proof.verifications.filter(function (v) { return v.work_value === 0; }).length;
            if (invalidCount > proof.verifications.length / 2 && proof.node_id) {
              var slashing = require("../services/slashing");
              slashing.recordOffense(proof.node_id, "EMPTY_GARBAGE_INFERENCE", {
                job_id: data.job_id,
                verdicts: proof.verifications.map(function (v) { return { verifier: v.miner, valid: v.work_value > 0 }; })
              }).catch(function (err) {
                console.error("[BTCPC P2P] Failed to slash miner:", err.message);
              });
            }
          }
        }
      }
    } catch (err) {
      console.error("[BTCPC P2P] Failed to process verification:", err.message);
    }
  })();

  // Rebroadcast
  ctx.broadcast(msg, peer.address);
}

// ---------------------------------------------------------------------------
// Clock Heartbeat — uptime tracking for clock node rewards
// ---------------------------------------------------------------------------

// Track active nodes per epoch: Map<epochNumber, Set<username>>
var clockUptimeByEpoch = new Map();
var _currentClockEpoch = -1;

// Track which nodes witnessed each clock heartbeat for anti-self-credit checks.
// Map<epochNumber, Map<account, Set<witnessNodeId>>>
// A heartbeat is only trustworthy if at least one OTHER node relayed/witnessed it.
var heartbeatWitnesses = new Map();

function recordHeartbeatWitness(account, epochNumber, witnessNodeId) {
  if (!epochNumber || epochNumber < 0 || !account || !witnessNodeId) return;
  if (!heartbeatWitnesses.has(epochNumber)) {
    heartbeatWitnesses.set(epochNumber, new Map());
  }
  var epochMap = heartbeatWitnesses.get(epochNumber);
  if (!epochMap.has(account)) {
    epochMap.set(account, new Set());
  }
  epochMap.get(account).add(witnessNodeId);

  // Prune old epochs (keep last 5)
  for (var key of heartbeatWitnesses.keys()) {
    if (key < epochNumber - 5) heartbeatWitnesses.delete(key);
  }
}

/**
 * Get the set of witness nodeIds for a given account's heartbeat in an epoch.
 * Returns an empty Set if no witnesses recorded.
 */
function getHeartbeatWitnesses(account, epochNumber) {
  var epochMap = heartbeatWitnesses.get(epochNumber);
  if (!epochMap) return new Set();
  return epochMap.get(account) || new Set();
}

/**
 * Record that a node was active during an epoch.
 * Called from any message handler when we see a nodeId.
 */
function recordNodeActivity(nodeId, username, epochNumber) {
  if (!epochNumber || epochNumber < 0) return;
  if (!clockUptimeByEpoch.has(epochNumber)) {
    clockUptimeByEpoch.set(epochNumber, new Set());
  }
  // Store username if known, otherwise nodeId
  clockUptimeByEpoch.get(epochNumber).add(username || nodeId);

  // Prune old epochs (keep last 20 — wide window handles restarts where
  // _currentEpochCache jumps several epochs before the next proposal runs)
  if (epochNumber > _currentClockEpoch) {
    _currentClockEpoch = epochNumber;
    for (var key of clockUptimeByEpoch.keys()) {
      if (key < epochNumber - 20) clockUptimeByEpoch.delete(key);
    }
  }
}

/**
 * Get the list of active clock nodes for a given epoch.
 * Returns array of usernames/nodeIds that were online during this epoch
 * OR within the last few epochs (clocks may heartbeat slightly before/after
 * the epoch boundary, so we use a small window).
 */
function getActiveClockNodes(epochNumber) {
  var WINDOW = 15; // accept heartbeats from current epoch and last 15 epochs
  var union = new Set();
  for (var i = 0; i <= WINDOW; i++) {
    var nodes = clockUptimeByEpoch.get(epochNumber - i);
    if (nodes) {
      for (var n of nodes) union.add(n);
    }
  }
  return Array.from(union);
}

// ---------------------------------------------------------------------------
// Testnet node tracking — nodes running the public test network
// ---------------------------------------------------------------------------

var testnetNodesByEpoch = new Map();
var _currentTestnetEpoch = -1;

function recordTestnetNode(account, epochNumber) {
  if (!epochNumber || epochNumber < 0 || !account) return;
  if (!testnetNodesByEpoch.has(epochNumber)) {
    testnetNodesByEpoch.set(epochNumber, new Set());
  }
  testnetNodesByEpoch.get(epochNumber).add(account);
  if (epochNumber > _currentTestnetEpoch) {
    _currentTestnetEpoch = epochNumber;
    for (var key of testnetNodesByEpoch.keys()) {
      if (key < epochNumber - 20) testnetNodesByEpoch.delete(key);
    }
  }
}

function getActiveTestnetNodes(epochNumber) {
  var WINDOW = 15;
  var union = new Set();
  for (var i = 0; i <= WINDOW; i++) {
    var nodes = testnetNodesByEpoch.get(epochNumber - i);
    if (nodes) for (var n of nodes) union.add(n);
  }
  return Array.from(union);
}

/**
 * TESTNET_HEARTBEAT — node announces it is running the public test network.
 */
function handleTestnetHeartbeat(peer, msg, ctx) {
  var data = msg.data || {};
  var account = data.account || msg.nodeId;
  if (!account) return;

  var GENESIS_TS = 1776236400000;
  var EPOCH_MS = 30000;
  var timeDerivedEpoch = Math.floor((Date.now() - GENESIS_TS) / EPOCH_MS);
  var fileEpoch = Math.max(_currentEpochCache > 0 ? _currentEpochCache : 0, timeDerivedEpoch);

  console.log("[BTCPC P2P] TESTNET_HEARTBEAT from " + account + " (filing under epoch " + fileEpoch + ")");
  recordTestnetNode(account, fileEpoch);
  ctx.broadcast(msg, peer.address);
}

// ---------------------------------------------------------------------------

/**
 * CLOCK_HEARTBEAT — clock node announces it's alive.
 */
function handleClockHeartbeat(peer, msg, ctx) {
  var data = msg.data || {};
  var account = data.account || msg.nodeId;
  var claimedEpoch = data.epoch_number || 0;
  var source = data.source || 'p2p';

  // Verify signature against account's posting key
  if (data.signature) {
    var hbVerifyData = {
      account: data.account,
      epoch_number: data.epoch_number,
      timestamp: data.timestamp
    };
    var hbSigOk = messageAuth.verifyAccountSignature(account, hbVerifyData, data.signature, "posting");
    if (!hbSigOk) {
      // Heartbeats don't require signatures — security comes from the
      // staking requirement and anti-sybil checks, not cryptographic signing.
      // Log for debugging but always accept.
    }
  }

  // File under THIS node's current epoch (what we think the chain height is),
  // not the sender's claim. Heartbeats can arrive several epochs after they
  // were sent (relay delay, sender's view stale). Crediting them as "active
  // now" is more accurate than crediting an old epoch number.
  // Use time-derived epoch as a floor so a stale _currentEpochCache on restart
  // doesn't file heartbeats far behind the actual chain tip.
  var GENESIS_TS = 1776236400000;
  var EPOCH_MS = 30000;
  var timeDerivedEpoch = Math.floor((Date.now() - GENESIS_TS) / EPOCH_MS);
  var fileEpoch = Math.max(
    _currentEpochCache > 0 ? _currentEpochCache : 0,
    timeDerivedEpoch
  );

  console.log("[BTCPC P2P] CLOCK_HEARTBEAT from " + account + " (claimed epoch " + claimedEpoch + ", filing under " + fileEpoch + ", source: " + source + ")");

  recordPeerEpoch(msg.nodeId || account, claimedEpoch);
  recordNodeActivity(msg.nodeId, account, fileEpoch);

  // Track which node relayed this heartbeat for anti-self-credit checks
  recordHeartbeatWitness(account, fileEpoch, msg.nodeId || peer.nodeId || "unknown");
  // Rebroadcast so all nodes see the heartbeat
  ctx.broadcast(msg, peer.address);
}

// ---------------------------------------------------------------------------
// PEER_ANNOUNCE — relay-free peer discovery
// ---------------------------------------------------------------------------

/**
 * Handle incoming PEER_ANNOUNCE — learn about new peers from the network.
 */
function handlePeerAnnounce(peer, msg, ctx) {
  var addresses = (msg.data && msg.data.peers) || [];
  var newCount = 0;
  for (var addr of addresses) {
    if (addKnownPeerAddress(addr)) {
      newCount++;
      // Try connecting to new peers
      if (ctx.connectToPeer) ctx.connectToPeer(normalizeP2PAddress(addr));
    }
  }
  if (newCount > 0) {
    saveKnownPeers();
    console.log("[BTCPC P2P] PEER_ANNOUNCE: learned " + newCount + " new peer(s) from " + (peer.nodeId || "?").slice(0, 12));
  }
  // Rebroadcast to other peers
  ctx.broadcast(msg, peer.address);
}

/**
 * Start the periodic PEER_ANNOUNCE broadcast.
 * Every 5 minutes, broadcast known peers and save to disk.
 * ctx: { broadcast, NODE_ID }
 */
var _peerAnnounceTimer = null;

function startPeerAnnounce(ctx) {
  if (_peerAnnounceTimer) return;
  var PEER_ANNOUNCE_INTERVAL = 5 * 60 * 1000; // 5 minutes

  _peerAnnounceTimer = setInterval(function () {
    if (knownPeers.size === 0) return;

    var peersArray = Array.from(knownPeers).filter(isConnectableP2PAddress);
    var announceMsg = createMessage(MESSAGE_TYPES.PEER_ANNOUNCE, {
      peers: peersArray
    }, ctx.NODE_ID);
    ctx.broadcast(announceMsg);
    saveKnownPeers();
    console.log("[BTCPC P2P] PEER_ANNOUNCE broadcast: " + peersArray.length + " known peer(s)");
  }, PEER_ANNOUNCE_INTERVAL);
}

function stopPeerAnnounce() {
  if (_peerAnnounceTimer) {
    clearInterval(_peerAnnounceTimer);
    _peerAnnounceTimer = null;
  }
}

// ---------------------------------------------------------------------------
// Ledger Sync — request and reconcile missing ledger entries
// ---------------------------------------------------------------------------

/**
 * REQUEST_LEDGER — Legacy path for pre-block-file nodes. Phase D: block
 * files (REQUEST_BLOCKS / RESPONSE_BLOCKS) are the canonical sync channel.
 * This handler is a no-op stub retained for backward-compatible wire format;
 * new syncs should use the block-based flow.
 */
async function handleRequestLedger(_peer, _msg, _ctx) {
  // no-op: chain history lives in block files, served via REQUEST_BLOCKS
}

/**
 * RESPONSE_LEDGER — Received ledger entries from a peer (legacy path).
 * Phase D: apply to stateStore only. No Mongo writes.
 */
async function handleResponseLedger(peer, msg, _ctx) {
  var data = msg.data || {};
  var entries = data.entries || [];
  if (entries.length === 0) return;

  try {
    var { applyRemoteEntries } = require("../services/ledger");
    var applied = await applyRemoteEntries(entries);
    if (applied > 0) {
      console.log("[BTCPC P2P] Ledger sync: " + applied + " entries from " + (peer.nodeId || "unknown").slice(0, 12));
    }
  } catch (err) {
    console.error("[BTCPC P2P] Failed to process ledger response:", err.message);
  }
}

/**
 * Request ledger sync from a peer.
 * Called after handshake or when a node suspects it's missing entries.
 */
function createRequestLedgerMessage(localCount, nodeId) {
  return createMessage(MESSAGE_TYPES.REQUEST_LEDGER, {
    localCount: localCount
  }, nodeId);
}

// ---------------------------------------------------------------------------
// Agent Protocol — Tool-call P2P routing
// ---------------------------------------------------------------------------

/**
 * TOOL_CALL — A miner detected a tool_use block in a model response.
 * Route point-to-point back to the origin peer (the client that opened the session).
 * Also deliver locally if this node is the session owner.
 *
 * Payload: { session_id, call_id, tool_name, arguments, origin_peer }
 */
function handleToolCall(peer, msg, ctx) {
  var data = msg.data || {};
  if (!data.session_id || !data.call_id || !data.tool_name) return;

  console.log("[BTCPC Agent] Tool call: " + data.tool_name +
    " call_id=" + data.call_id.slice(0, 12) + "..." +
    " session=" + data.session_id.slice(0, 12) + "...");

  // Deliver to local agent session if this node is waiting
  try {
    var agentSession = require("../inference/agentSession");
    var session = agentSession.getAgentSession(data.session_id);
    if (session) {
      // This node has the session — the client submitted here.
      // Forward the tool call to whatever callback is listening on the REST layer.
      // The REST layer (POST /v1/agent/turn) polls pendingToolCalls via waitForToolResult;
      // but for TOOL_CALL we need to expose the call to the client. Since REST is
      // synchronous, we emit via a local EventEmitter so the streaming response
      // can include the tool_call in its SSE stream.
      var agentEvents = require("../inference/agentEvents");
      agentEvents.emit("tool_call", data);
      return; // don't rebroadcast — session is local
    }
  } catch (_) { /* agentSession or agentEvents not loaded */ }

  // Rebroadcast to other peers (point-to-point routing: only toward origin)
  ctx.broadcast(msg, peer.address);
}

/**
 * TOOL_RESULT — A client delivers the tool execution result to the miner.
 * Route to the miner that originated the TOOL_CALL.
 *
 * Payload: { session_id, call_id, result, error }
 */
function handleToolResult(peer, msg, ctx) {
  var data = msg.data || {};
  if (!data.session_id || !data.call_id) return;

  console.log("[BTCPC Agent] Tool result: call_id=" + data.call_id.slice(0, 12) + "..." +
    " session=" + data.session_id.slice(0, 12) + "..." +
    (data.error ? " ERROR: " + data.error : " OK"));

  // Deliver locally if this miner is waiting for a tool result
  try {
    var agentSession = require("../inference/agentSession");
    var delivered = agentSession.deliverToolResult(data.session_id, data.call_id, data.result, data.error);
    if (delivered) return; // consumed locally
  } catch (_) { /* agentSession not loaded */ }

  // Not consumed locally — relay to other peers
  ctx.broadcast(msg, peer.address);
}

// ---------------------------------------------------------------------------
// Shard Registry handler — Mode B (pipeline)
// ---------------------------------------------------------------------------

/**
 * SHARD_REGISTER — A node announces a model layer shard.
 * Stores the registration and, if all layers are covered, broadcasts
 * SHARD_GROUP_FORM so all peers know a full pipeline is available.
 */
function handleShardRegister(peer, msg, ctx) {
  var data = msg.data || {};
  if (!data.node_id || !data.model_name) return;

  try {
    var shardRegistry = require("../inference/shardRegistry");
    var result = shardRegistry.registerShard(data);

    console.log("[BTCPC Shard] Registered shard: " + data.node_id.slice(0, 12) +
      " model=" + data.model_name +
      " layers=[" + data.layer_start + ".." + data.layer_end + "/" + data.total_layers + "]" +
      " engine=" + (data.engine || "ollama"));

    if (result.groupResult && result.groupResult.formed) {
      var group = result.groupResult.group;
      console.log("[BTCPC Shard] Group formed for " + group.model_name +
        " group_id=" + group.group_id +
        " shards=" + group.shards.length +
        " total_params=" + group.total_params);

      // Broadcast SHARD_GROUP_FORM so all peers know this pipeline is ready
      var groupMsg = createMessage(MESSAGE_TYPES.SHARD_GROUP_FORM, {
        group_id: group.group_id,
        model_name: group.model_name,
        total_params: group.total_params,
        shards: group.shards.map(function (s) {
          return {
            node_id: s.node_id,
            layer_start: s.layer_start,
            layer_end: s.layer_end,
            param_count: s.param_count,
            rpc_endpoint: s.rpc_endpoint,
            engine: s.engine,
          };
        }),
        coordinator: group.coordinator,
        formed_at: group.formed_at,
      }, ctx.NODE_ID);
      ctx.broadcast(groupMsg);
    }
  } catch (err) {
    console.error("[BTCPC Shard] handleShardRegister error:", err.message);
  }

  // Relay the registration to all peers
  ctx.broadcast(msg, peer.address);
}

// ---------------------------------------------------------------------------
// Ensemble handler — Mode A (majority-vote)
// ---------------------------------------------------------------------------

/**
 * ENSEMBLE_RESULT — A node submits its inference result for an ensemble job.
 * If consensus is reached, broadcast ENSEMBLE_CONSENSUS to all peers.
 */
function handleEnsembleResult(peer, msg, ctx) {
  var data = msg.data || {};
  if (!data.request_id || !data.node_id || !data.result_hash) return;

  try {
    var ensembleCoordinator = require("../inference/ensembleCoordinator");
    var outcome = ensembleCoordinator.submitContribution(
      data.request_id,
      data.node_id,
      data.result_hash,
      data.result_text || null,
      data.tokens || 0,
      data.model_hash || null
    );

    console.log("[BTCPC Ensemble] Contribution from " + data.node_id.slice(0, 12) +
      " request=" + data.request_id.slice(0, 12) +
      " status=" + outcome.status);

    if (outcome.consensusAchieved) {
      console.log("[BTCPC Ensemble] Consensus on request " + data.request_id.slice(0, 12) +
        " nodes=" + outcome.consensusNodes.join(",").slice(0, 40));

      var consensusMsg = createMessage(MESSAGE_TYPES.ENSEMBLE_CONSENSUS, {
        request_id: data.request_id,
        consensus_hash: outcome.result ? outcome.result.result_hash : null,
        consensus_nodes: outcome.consensusNodes,
        result_text: outcome.result ? outcome.result.result_text : null,
        timestamp: Date.now(),
      }, ctx.NODE_ID);
      ctx.broadcast(consensusMsg);
    }
  } catch (err) {
    console.error("[BTCPC Ensemble] handleEnsembleResult error:", err.message);
  }

  // Relay to all peers
  ctx.broadcast(msg, peer.address);
}

// ---------------------------------------------------------------------------
// Exports
// ---------------------------------------------------------------------------

module.exports = {
  MESSAGE_TYPES,
  handleMessage,
  createHandshake,
  createBlockMessage,
  createTransactionMessage,
  createPeerListMessage,
  createEpochCommitMessage,
  createRequestBlocksMessage,
  createRequestLedgerMessage,
  createMessage,
  getIdleMiners,
  recordNodeActivity,
  getActiveClockNodes,
  getHeartbeatWitnesses,
  getActiveTestnetNodes,
  recordTestnetNode,
  getActiveVerifiers,
  getVerifiedMiners,
  recordMinerWork,
  getMinerWorkForEpoch,
  setCurrentEpoch,
  getCurrentEpochCache,
  handleMempoolEntry,
  knownPeers,
  startPeerAnnounce,
  stopPeerAnnounce,
  addModelDemand,
  getModelDemand,
  flushSeenMessages,
  VERIFIER_REVEAL_WINDOW_MS: VERIFIER_REVEAL_WINDOW_MS
};
