"use strict";

/**
 * BTCPC P2P Protocol
 * Shin Devlin
 *
 * Defines message types and handlers for the BTCPC peer-to-peer network.
 * Every message follows the format: { type, data, timestamp, nodeId }
 */

const { validateBlock, getChainHeight, getBlockRange } = require("./chainSync");
const mempool = require("./mempool");
const Block = require("../chain/block");
const blockchain = require("../chain/blockchain");

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
  // Inference protocol
  INFERENCE_REQUEST: "INFERENCE_REQUEST",
  INFERENCE_CLAIM: "INFERENCE_CLAIM",
  INFERENCE_ASSIGN: "INFERENCE_ASSIGN",
  INFERENCE_PAYLOAD: "INFERENCE_PAYLOAD",
  INFERENCE_COMMIT: "INFERENCE_COMMIT",
  INFERENCE_REVEAL: "INFERENCE_REVEAL",
  INFERENCE_RESULT: "INFERENCE_RESULT",
  // Model demand broadcast
  MODEL_DEMAND: "MODEL_DEMAND",
  // Mining proof gossip — miners broadcast proofs so all nodes can finalize
  MINING_PROOF: "MINING_PROOF",
  // Miner idle — no work this epoch, don't wait for my proof
  MINER_IDLE: "MINER_IDLE",
};

// Track seen message IDs to prevent rebroadcast loops
const seenMessages = new Set();
const SEEN_MAX = 10000;

function markSeen(msgId) {
  seenMessages.add(msgId);
  if (seenMessages.size > SEEN_MAX) {
    // Evict oldest entries (Set maintains insertion order)
    const iter = seenMessages.values();
    for (let i = 0; i < 1000; i++) {
      seenMessages.delete(iter.next().value);
    }
  }
}

// ---------------------------------------------------------------------------
// Message creation helpers
// ---------------------------------------------------------------------------

function createMessage(type, data, nodeId) {
  const id = nodeId + "-" + Date.now() + "-" + Math.random().toString(36).slice(2, 8);
  return {
    id: id,
    type: type,
    data: data,
    timestamp: Date.now(),
    nodeId: nodeId
  };
}

/**
 * Create a HANDSHAKE message with chain state and known peers.
 */
function createHandshake(nodeId) {
  return createMessage(MESSAGE_TYPES.HANDSHAKE, {
    chainHeight: getChainHeight(),
    version: "0.1.0",
    peerCount: 0 // filled by caller if needed
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
    default:
      console.log("[BTCPC P2P] Unknown message type: " + msg.type);
  }
}

/**
 * HANDSHAKE — Exchange chain height, genesis hash, and peer lists.
 */
function handleHandshake(peer, msg, ctx) {
  const data = msg.data || {};

  peer.nodeId = msg.nodeId;
  peer.chainHeight = data.chainHeight || 0;
  peer.status = "connected";

  console.log("[BTCPC P2P] Handshake from " + msg.nodeId.slice(0, 12) + "... (height: " + peer.chainHeight + ")");

  // Send our peer list to the new peer
  const knownAddresses = [];
  for (const [addr, p] of ctx.peers) {
    if (p.status === "connected" && addr !== peer.address) {
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
}

/**
 * BLOCK — Validate and store a new epoch/block received from the network.
 * Supports both serialized blocks (header_hex) and legacy plain objects.
 */
function handleBlock(peer, msg, ctx) {
  const data = msg.data;
  if (!data) return;

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

      // Store in the formal blockchain
      blockchain.addBlock(block);

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

  console.log("[BTCPC P2P] Received " + blocks.length + " blocks from " +
    (peer.nodeId || "unknown").slice(0, 12));

  let accepted = 0;
  for (const block of blocks) {
    if (validateBlock(block)) {
      accepted++;
    }
  }

  console.log("[BTCPC P2P] Accepted " + accepted + "/" + blocks.length + " blocks");
}

/**
 * INFERENCE messages — gossip to all peers and notify local handlers.
 */
function handleInferenceMessage(peer, msg, ctx) {
  console.log("[BTCPC P2P] Inference " + msg.type + " from " + (msg.nodeId || "unknown").slice(0, 12));
  // Rebroadcast to all other peers
  ctx.broadcast(msg, peer.address);
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

  // Store in local DB (if we don't already have it)
  const MiningProof = require("../models/MiningProof");
  MiningProof.findOne({ block_number: data.block_number, miner: data.miner })
    .then(existing => {
      if (existing) return; // already have this proof
      return MiningProof.create({
        block_number: data.block_number,
        miner: data.miner,
        reward_earned: 0, // set during finalization
        model: data.model,
        model_hash: data.model_hash || null,
        tokens_computed: data.tokens_computed || 0,
        work_value: data.work_value || 0,
        state_hash: data.state_hash || null
      });
    })
    .then(created => {
      if (created) {
        console.log("[BTCPC P2P] Stored remote proof: " + data.miner + " block " + data.block_number + " (wv=" + data.work_value + ")");
      }
    })
    .catch(err => {
      console.error("[BTCPC P2P] Failed to store mining proof:", err.message);
    });

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
  createMessage,
  getIdleMiners
};
