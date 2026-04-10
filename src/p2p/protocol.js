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
const blockStore = require("../chain/blockStore");
const stateManager = require("../chain/stateManager");

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
  var pkg = require("../../package.json");
  return createMessage(MESSAGE_TYPES.HANDSHAKE, {
    chainHeight: getChainHeight(),
    version: pkg.version,
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
    case MESSAGE_TYPES.EPOCH_START:
      console.log("[BTCPC P2P] Epoch START: " + (msg.data?.epoch_number || "?") + " from " + (msg.data?.authority || "unknown"));
      if (msg.data?.epoch_number) setCurrentEpoch(msg.data.epoch_number);
      ctx.broadcast(msg, peer.address);
      break;
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

  console.log("[BTCPC P2P] Handshake from " + msg.nodeId.slice(0, 12) + "... (v" + peer.version + ", height: " + peer.chainHeight + ")");

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
async function handleEpochFinalized(peer, msg, ctx) {
  const data = msg.data || {};
  if (!data.epoch_number) return;

  const epochNum = data.epoch_number;
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

  console.log("[BTCPC P2P] Account announced: " + data.username + " | evm=" + (data.chain_addresses?.evm || "none"));

  try {
    const stateStore = require("../chain/stateStore");
    if (!stateStore.getAccount(data.username)) {
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

  console.log("[BTCPC P2P] BLOCK_PROPOSAL from " + data.proposer +
    " for epoch " + data.epoch_number +
    " (work=" + (data.total_work || 0) +
    ", hash=" + data.consensus_hash.slice(0, 12) + ")");

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
  var epochMap = minerWorkByEpoch.get(epochNumber);
  if (!epochMap) return {};
  var result = {};
  for (var entry of epochMap) {
    result[entry[0]] = {
      work_value: entry[1].work_value,
      job_count: entry[1].jobs.size,
    };
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

  // Check if this node should verify — deterministic selection
  var verifier = require("../inference/verifier");

  // Eligible verifier pool: any node currently active as a clock or
  // any node we've seen broadcast work in recent epochs. This means
  // clock-only nodes (like josh on Termux) participate in verification
  // alongside miners. Verification is text analysis — no Ollama needed.
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

  if (!myAccount || myAccount === data.miner) {
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
  var vCount = verifier.getVerifierCount(totalNodes);
  var blockHash = data.block_hash || "0".repeat(64);
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

  // Store verification on the InferenceJob if we have it locally
  (async function () {
    try {
      var InferenceJob = require("../models/InferenceJob");
      var job = await InferenceJob.findOne({ job_id: data.job_id });
      if (job) {
        if (!job.verifications) job.verifications = [];
        // Don't duplicate
        var existing = job.verifications.find(function (v) { return v.miner === data.verifier; });
        if (!existing) {
          job.verifications.push({
            miner: data.verifier,
            result_hash: job.result_hash || "",
            work_value: data.verdict === "valid" ? (data.score || 0) : 0,
            completed_at: new Date()
          });
          if (job.status === "completed") job.status = "verifying";
          await job.save();

          // Check for majority rejection → slash the miner
          var required = job.required_verifications || 1;
          if (job.verifications.length >= required) {
            var invalidCount = job.verifications.filter(function (v) {
              return v.work_value === 0;
            }).length;
            if (invalidCount > job.verifications.length / 2) {
              var slashing = require("../services/slashing");
              var minerAccount = job.node_name || (job.verifications[0] && job.verifications[0].miner);
              if (minerAccount) {
                var verdictSummary = job.verifications.map(function (v) {
                  return { verifier: v.miner, valid: v.work_value > 0 };
                });
                slashing.recordOffense(minerAccount, "EMPTY_GARBAGE_INFERENCE", {
                  job_id: job.job_id,
                  verdicts: verdictSummary
                }).catch(function (err) {
                  console.error("[BTCPC P2P] Failed to slash miner:", err.message);
                });
              }
            }
          }
        }
      }
    } catch (err) {
      console.error("[BTCPC P2P] Failed to store verification:", err.message);
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

  // File under THIS node's current epoch (what we think the chain height is),
  // not the sender's claim. Heartbeats can arrive several epochs after they
  // were sent (relay delay, sender's view stale). Crediting them as "active
  // now" is more accurate than crediting an old epoch number.
  var fileEpoch = _currentEpochCache > 0 ? _currentEpochCache : claimedEpoch;

  console.log("[BTCPC P2P] CLOCK_HEARTBEAT from " + account + " (claimed epoch " + claimedEpoch + ", filing under " + fileEpoch + ", source: " + source + ")");

  recordNodeActivity(msg.nodeId, account, fileEpoch);
  // Rebroadcast so all nodes see the heartbeat
  ctx.broadcast(msg, peer.address);
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
  getActiveVerifiers,
  recordMinerWork,
  getMinerWorkForEpoch,
  setCurrentEpoch,
  getCurrentEpochCache
};
