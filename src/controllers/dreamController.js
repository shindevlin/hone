"use strict";

/**
 * Dream Controller
 * Shin Devlin
 *
 * Genesis Dreams are NFT-like inscriptions stored on blocks.
 * Each block produces one Genesis Dream — owned by the miner, transferable.
 *
 * Phase E: GenesisDream Mongoose model removed. Dreams read from stateStore
 * (which is populated by block replay). In-memory dream store for mutations.
 */

const User = require("../models/User");
const stateStore = require("../chain/stateStore");
const { filterInscription } = require("../services/contentFilter");
const { rejectObjectInputs, sanitizeString, validAccountName } = require("../middlewares/validate");

const INSCRIPTION_CHAR_LIMIT = 4200;

// In-memory dream store: blockNumber → dreamObject
// Populated from stateStore (GENESIS_DREAM ledger entries) on first access.
const dreamCache = new Map();

function _getDreamFromStore(blockNumber) {
  // Try dreamCache first
  if (dreamCache.has(blockNumber)) return dreamCache.get(blockNumber);

  // Try stateStore dreams (if stateStore has a getDream method)
  if (stateStore.getDream) {
    const d = stateStore.getDream(blockNumber);
    if (d) { dreamCache.set(blockNumber, d); return d; }
  }

  return null;
}

function _getDreamsByOwner(account) {
  const results = [];
  for (const [, dream] of dreamCache) {
    if (dream.current_owner === account) results.push(dream);
  }
  // Also check stateStore
  if (stateStore.getAllDreams) {
    const all = stateStore.getAllDreams();
    for (const dream of Object.values(all)) {
      if (dream.current_owner === account && !dreamCache.has(dream.block_number)) {
        results.push(dream);
      }
    }
  }
  return results.sort((a, b) => a.block_number - b.block_number);
}

// -------------------------------------------------------------------------
// Core functions
// -------------------------------------------------------------------------

async function getDreams(account) {
  return _getDreamsByOwner(account);
}

async function getDream(blockNumber) {
  return _getDreamFromStore(blockNumber);
}

async function inscribeDream(blockNumber, account, inscription) {
  const dream = _getDreamFromStore(blockNumber);
  if (!dream) {
    throw new Error("Genesis Dream #" + blockNumber + " not found");
  }
  if (dream.current_owner !== account) {
    throw new Error(
      "Account '" + account + "' does not own Genesis Dream #" + blockNumber +
      " (owned by '" + dream.current_owner + "')"
    );
  }
  if (dream.inscription && dream.inscription.project && dream.inscription.project !== "") {
    throw new Error("Genesis Dream #" + blockNumber + " is already inscribed");
  }

  if (blockNumber > 0 && inscription.custom_data) {
    const dataStr = JSON.stringify(inscription.custom_data);
    if (dataStr.length > INSCRIPTION_CHAR_LIMIT) {
      throw new Error(
        "Inscription exceeds " + INSCRIPTION_CHAR_LIMIT +
        " char limit (" + dataStr.length + " chars). Only Dream #0 is unlimited."
      );
    }
  }

  const filteredProject = filterInscription(inscription.project);
  const filteredTag = filterInscription(inscription.tag);
  let filteredCustomData = inscription.custom_data || null;
  if (filteredCustomData && typeof filteredCustomData === "string") {
    const customResult = filterInscription(filteredCustomData);
    filteredCustomData = customResult.filtered_text;
  }

  dream.inscription = {
    project: filteredProject.filtered_text,
    tag: filteredTag.filtered_text,
    custom_data: filteredCustomData
  };
  dream.inscribed_at = new Date().toISOString();
  dreamCache.set(blockNumber, dream);

  return dream;
}

async function transferDream(blockNumber, fromAccount, toAccount) {
  const dream = _getDreamFromStore(blockNumber);
  if (!dream) {
    throw new Error("Genesis Dream #" + blockNumber + " not found");
  }
  if (dream.current_owner !== fromAccount) {
    throw new Error(
      "Account '" + fromAccount + "' does not own Genesis Dream #" + blockNumber +
      " (owned by '" + dream.current_owner + "')"
    );
  }

  const recipient = await User.findOne({ username: toAccount });
  if (!recipient) {
    throw new Error("Recipient account '" + toAccount + "' not found");
  }

  if (fromAccount === toAccount) {
    throw new Error("Cannot transfer a dream to yourself");
  }

  dream.current_owner = toAccount;
  dream.transferred = true;
  if (!dream.transfer_history) dream.transfer_history = [];
  dream.transfer_history.push({ from: fromAccount, to: toAccount, timestamp: new Date().toISOString() });
  dreamCache.set(blockNumber, dream);

  return dream;
}

// -------------------------------------------------------------------------
// Express route handlers
// -------------------------------------------------------------------------

async function getDreamsRoute(req, res) {
  try {
    const account = sanitizeString(req.params.account, 20);
    if (!account || !validAccountName(account)) return res.status(400).json({ error: "invalid account name" });
    const dreams = await getDreams(account);
    res.json({ account, count: dreams.length, dreams });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
}

async function getDreamRoute(req, res) {
  try {
    const blockNumber = parseInt(req.params.blockNumber);
    if (isNaN(blockNumber)) {
      return res.status(400).json({ error: "Invalid block number" });
    }
    const dream = await getDream(blockNumber);
    if (!dream) {
      return res.status(404).json({ error: "Genesis Dream #" + blockNumber + " not found" });
    }
    res.json(dream);
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
}

async function inscribeDreamRoute(req, res) {
  try {
    const blockNumber = parseInt(req.params.blockNumber);
    if (isNaN(blockNumber) || blockNumber < 0) {
      return res.status(400).json({ error: "Invalid block number" });
    }
    const objErr = rejectObjectInputs(req.body, ['project', 'tag']);
    if (objErr) return res.status(400).json({ error: objErr });
    const project = sanitizeString(req.body.project, 200);
    const tag = sanitizeString(req.body.tag, 200);
    const custom_data = req.body.custom_data;
    if (!project || !tag) {
      return res.status(400).json({ error: "project and tag are required" });
    }
    const account = req.user.username;
    const dream = await inscribeDream(blockNumber, account, { project, tag, custom_data: custom_data || null });
    res.json({ message: "Genesis Dream #" + blockNumber + " inscribed", dream });
  } catch (err) {
    const status = err.message.includes("not found") ? 404
      : err.message.includes("does not own") ? 403
      : err.message.includes("already inscribed") ? 409
      : 400;
    res.status(status).json({ error: err.message });
  }
}

async function transferDreamRoute(req, res) {
  try {
    const blockNumber = parseInt(req.params.blockNumber);
    if (isNaN(blockNumber) || blockNumber < 0) {
      return res.status(400).json({ error: "Invalid block number" });
    }
    if (typeof req.body.to === 'object') return res.status(400).json({ error: "to must be a string" });
    const to = sanitizeString(req.body.to, 20);
    if (!to) return res.status(400).json({ error: "to (recipient account) is required" });
    if (!validAccountName(to)) return res.status(400).json({ error: "invalid recipient account name" });
    const fromAccount = req.user.username;
    const dream = await transferDream(blockNumber, fromAccount, to);
    res.json({
      message: "Genesis Dream #" + blockNumber + " transferred to " + to,
      original_miner: dream.original_miner,
      dream
    });
  } catch (err) {
    const status = err.message.includes("not found") ? 404
      : err.message.includes("does not own") ? 403
      : 400;
    res.status(status).json({ error: err.message });
  }
}

module.exports = {
  getDreams,
  getDream,
  inscribeDream,
  transferDream,
  getDreamsRoute,
  getDreamRoute,
  inscribeDreamRoute,
  transferDreamRoute
};
