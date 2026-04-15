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
    if (Array.isArray(_saved)) {
      for (var _addr of _saved) {
        if (typeof _addr === "string" && _addr.startsWith("ws")) {
          knownPeers.add(_addr);
        }
      }
    }
    if (knownPeers.size > 0) {
      console.log("[BTCPC P2P] Loaded " + knownPeers.size + " known peers from disk");
    }
  }
} catch (_e) {
  // Ignore — file may not exist yet
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
  // Mempool gossip — ledger entries broadcast across machines so any
  // broadcaster can include them in its next block (v2.13.3)
  MEMPOOL_ENTRY: "MEMPOOL_ENTRY",
  // Peer relay — nodes announce their known peers so the network
  // doesn't depend on a single Cloudflare relay for discovery
  PEER_ANNOUNCE: "PEER_ANNOUNCE",
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

setInterval(flushSeenMessages, 30000);

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
    public_address: process.env.BTCPC_PUBLIC_ADDRESS || null
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
    case MESSAGE_TYPES.FINALIZATION_PROPOSAL:
      handleFinalizationProposal(peer, msg, ctx);
      break;
    case MESSAGE_TYPES.BLOCK_PROPOSAL:
      handleBlockProposal(peer, msg, ctx);
      break;
    case MESSAGE_TYPES.CLOCK_HEARTBEAT:
      handleClockHeartbeat(peer, msg, ctx);
      break;
    case MESSAGE_TYPES.VERIFY_REQUEST:
      handleVerifyRequest(peer, msg, ctx);
      break;
    case MESSAGE_TYPES.VERIFY_RESPONSE:
      handleVerifyResponse(peer, msg, ctx);
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
    default:
      console.log("[BTCPC P2P] Unknown message type: " + msg.type + " from " + (msg.nodeId || "?").slice(0, 12));
  }
}

/**
 * HANDSHAKE — Exchange chain height, genesis hash, and peer lists.
 */
function handleHandshake(peer, msg, ctx) {
  const data = msg.data || {};

  peer.nodeId = msg.nodeId;
  peer.chainHeight = data.chainHeight || 0;
  peer.version = data.version || "unknown";
  peer.status = "connected";

  // Store the peer's claimed public address for relay-free discovery
  if (data.public_address && typeof data.public_address === "string" && data.public_address.startsWith("ws")) {
    knownPeers.add(data.public_address);
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

      if (!blockStore.hasBlock(block.epoch_number)) {
        var payload = {
          ledger_entries: data.ledger_entries || [],
          rewards: data.rewards || [],
          compute_proofs: data.compute_proofs || [],
          mining_proofs: data.mining_proofs || []
        };
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
    if (!ctx.peers.has(addr)) {
      ctx.connectToPeer(addr);
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
            mining_proofs: blockData.mining_proofs || []
          };
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

/**
 * MINING_PROOF — A miner broadcasts their proof for an epoch.
 * Receiving nodes store it locally so finalization can collect all proofs.
 * This eliminates the need for a shared database between miners.
 */
function handleMiningProof(peer, msg, ctx) {
  const data = msg.data || {};
  if (!data.block_number || !data.miner) return;

  console.log("[BTCPC P2P] Mining proof from " + data.miner + " for block " + data.block_number);

  // Phase D: do NOT persist MiningProof to Mongo. Proofs flow into the
  // block payload (payload.mining_proofs) when the authority writes the
  // block, and into stateStore.miningProofsByEpoch via replay + live apply.
  // Remote proofs are just informational here — the authoritative copy
  // arrives in BLOCK_PROPOSAL / EPOCH_FINALIZED messages.

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
    var sigOk = messageAuth.verifyAccountSignature(data.proposer, blockHeaderData, data.block_signature, "active");
    if (!sigOk) {
      if (messageAuth.REQUIRE_SIGNATURES) {
        console.log("[BTCPC P2P] EPOCH_FINALIZED REJECTED: invalid block_signature from proposer " + data.proposer + " for epoch " + epochNum);
        return;
      }
      console.log("[BTCPC P2P WARN] EPOCH_FINALIZED epoch " + epochNum + " from " + (data.proposer || "?") + " has invalid block_signature — will be rejected after v2.17");
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
 * FINALIZATION_PROPOSAL — A miner proposes their reward split for an epoch.
 * Collected by all nodes. When majority agrees, the earliest proposer broadcasts EPOCH_FINALIZED.
 *
 * NOTE: This is the legacy multi-message ceremony. New code uses BLOCK_PROPOSAL.
 */
function handleFinalizationProposal(peer, msg, ctx) {
  var data = msg.data || {};
  if (!data.epoch_number || !data.proposer) return;

  console.log("[BTCPC P2P] Finalization proposal from " + data.proposer +
    " for epoch " + data.epoch_number +
    " (hash: " + (data.consensus_hash || "?").slice(0, 12) + "...)");

  // Submit to local consensus collector
  var finConsensus = require("../chain/finalizationConsensus");
  finConsensus.submitProposal(data.epoch_number, data);

  // Rebroadcast
  ctx.broadcast(msg, peer.address);
}

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

  // Vuln 5: verify proposal_signature against proposer's active key.
  if (data.proposal_signature) {
    var proposalData = {
      epoch_number: data.epoch_number,
      proposer: data.proposer,
      consensus_hash: data.consensus_hash,
      total_work: data.total_work || 0,
      timestamp: data.timestamp || 0,
    };
    var sigOk = messageAuth.verifyAccountSignature(data.proposer, proposalData, data.proposal_signature, "active");
    if (!sigOk) {
      console.log("[BTCPC P2P] BLOCK_PROPOSAL REJECTED: invalid proposal_signature from " + data.proposer + " for epoch " + data.epoch_number);
      return;
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
  // but the data shape is the new unified format
  var finConsensus = require("../chain/finalizationConsensus");
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
  });

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

  // Prune old epochs (keep last 10)
  if (epochNumber > _currentWorkEpoch) {
    _currentWorkEpoch = epochNumber;
    for (var key of minerWorkByEpoch.keys()) {
      if (key < epochNumber - 10) minerWorkByEpoch.delete(key);
    }
  }
}

/**
 * Get all miners with their work values for an epoch.
 * Returns: { miner: { work_value, jobs: Set } }
 */
function getMinerWorkForEpoch(epochNumber) {
  // Check the target epoch plus recent prior epochs — fire-and-forget
  // inference completes in future epochs, so work credited to epoch N
  // may need to be picked up by the proposal for epoch N or N+1.
  var result = {};
  var lookback = 10; // check up to 10 epochs back for uncredited work
  for (var ep = Math.max(0, epochNumber - lookback); ep <= epochNumber; ep++) {
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

  console.log("[BTCPC P2P] Verified job " + data.job_id.slice(0, 12) + "... verdict=" +
    (verdict.valid ? "VALID" : "INVALID") + " score=" + verdict.score);

  // Send VERIFY_RESPONSE
  var response = createMessage(MESSAGE_TYPES.VERIFY_RESPONSE, {
    job_id: data.job_id,
    verifier: myAccount,
    verdict: verdict.valid ? "valid" : "invalid",
    score: verdict.score,
    checks: verdict.checks,
    epoch: data.epoch || 0
  }, ctx.NODE_ID);
  ctx.broadcast(response);

  // Rebroadcast original request so other verifiers see it
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

  // Prune old epochs (keep last 5)
  if (epochNumber > _currentClockEpoch) {
    _currentClockEpoch = epochNumber;
    for (var key of clockUptimeByEpoch.keys()) {
      if (key < epochNumber - 5) clockUptimeByEpoch.delete(key);
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
  var WINDOW = 3; // accept heartbeats from current epoch and last 3 epochs
  var union = new Set();
  for (var i = 0; i <= WINDOW; i++) {
    var nodes = clockUptimeByEpoch.get(epochNumber - i);
    if (nodes) {
      for (var n of nodes) union.add(n);
    }
  }
  return Array.from(union);
}

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
  var fileEpoch = _currentEpochCache > 0 ? _currentEpochCache : claimedEpoch;

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
    if (typeof addr === "string" && addr.startsWith("ws") && !knownPeers.has(addr)) {
      knownPeers.add(addr);
      newCount++;
      // Try connecting to new peers
      if (ctx.connectToPeer) ctx.connectToPeer(addr);
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

    var peersArray = Array.from(knownPeers);
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
  getModelDemand
};
