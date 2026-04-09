"use strict";
const GenesisDream = require("../models/GenesisDream");
const User = require("../models/User");
const { filterInscription } = require("../services/contentFilter");
const { rejectObjectInputs, sanitizeString, validAccountName } = require("../middlewares/validate");

const INSCRIPTION_CHAR_LIMIT = GenesisDream.INSCRIPTION_CHAR_LIMIT || 4200;

// -------------------------------------------------------------------------
// Core functions (used by both CLI and API)
// -------------------------------------------------------------------------

/**
 * Get all genesis dreams owned by an account.
 */
async function getDreams(account) {
  const dreams = await GenesisDream.find({ current_owner: account })
    .sort({ block_number: 1 });
  return dreams;
}

/**
 * Get a single genesis dream by block number.
 */
async function getDream(blockNumber) {
  const dream = await GenesisDream.findOne({ block_number: blockNumber });
  return dream;
}

/**
 * Inscribe a genesis dream. Once inscribed, it cannot be re-inscribed.
 */
async function inscribeDream(blockNumber, account, inscription) {
  const dream = await GenesisDream.findOne({ block_number: blockNumber });
  if (!dream) {
    throw new Error("Genesis Dream #" + blockNumber + " not found");
  }
  if (dream.current_owner !== account) {
    throw new Error(
      "Account '" + account + "' does not own Genesis Dream #" + blockNumber +
      " (owned by '" + dream.current_owner + "')"
    );
  }
  // Check if already inscribed (has non-default inscription data)
  if (dream.inscription && dream.inscription.project &&
      dream.inscription.project !== "" &&
      dream.inscription.tag !== "") {
    throw new Error(
      "Genesis Dream #" + blockNumber + " is already inscribed"
    );
  }

  // Validate character limit (block 0 is unlimited)
  if (blockNumber > 0 && inscription.custom_data) {
    const dataStr = JSON.stringify(inscription.custom_data);
    if (dataStr.length > INSCRIPTION_CHAR_LIMIT) {
      throw new Error(
        "Inscription exceeds " + INSCRIPTION_CHAR_LIMIT +
        " char limit (" + dataStr.length + " chars). Only Dream #0 is unlimited."
      );
    }
  }

  // Apply content filter to public inscription text
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
  dream.inscribed_at = new Date();

  await dream.save();
  return dream;
}

/**
 * Transfer a genesis dream to another account.
 * The original_miner field NEVER changes.
 */
async function transferDream(blockNumber, fromAccount, toAccount) {
  const dream = await GenesisDream.findOne({ block_number: blockNumber });
  if (!dream) {
    throw new Error("Genesis Dream #" + blockNumber + " not found");
  }
  if (dream.current_owner !== fromAccount) {
    throw new Error(
      "Account '" + fromAccount + "' does not own Genesis Dream #" + blockNumber +
      " (owned by '" + dream.current_owner + "')"
    );
  }

  // Verify the recipient exists
  const recipient = await User.findOne({ username: toAccount });
  if (!recipient) {
    throw new Error("Recipient account '" + toAccount + "' not found");
  }

  if (fromAccount === toAccount) {
    throw new Error("Cannot transfer a dream to yourself");
  }

  dream.current_owner = toAccount;
  dream.transferred = true;
  dream.transfer_history.push({
    from: fromAccount,
    to: toAccount,
    timestamp: new Date()
  });

  await dream.save();
  return dream;
}

// -------------------------------------------------------------------------
// Express route handlers (for API)
// -------------------------------------------------------------------------

/**
 * GET /api/dreams/:account
 */
async function getDreamsRoute(req, res) {
  try {
    const account = sanitizeString(req.params.account, 20);
    if (!account || !validAccountName(account)) return res.status(400).json({ error: "invalid account name" });
    const dreams = await getDreams(account);
    res.json({ account: account, count: dreams.length, dreams: dreams });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
}

/**
 * GET /api/dream/:blockNumber
 */
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

/**
 * POST /api/dream/:blockNumber/inscribe (auth required)
 * Body: { project, tag, custom_data? }
 */
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
    const dream = await inscribeDream(blockNumber, account, {
      project: project,
      tag: tag,
      custom_data: custom_data || null
    });
    res.json({ message: "Genesis Dream #" + blockNumber + " inscribed", dream: dream });
  } catch (err) {
    const status = err.message.includes("not found") ? 404
      : err.message.includes("does not own") ? 403
      : err.message.includes("already inscribed") ? 409
      : 400;
    res.status(status).json({ error: err.message });
  }
}

/**
 * POST /api/dream/:blockNumber/transfer (auth required)
 * Body: { to }
 */
async function transferDreamRoute(req, res) {
  try {
    const blockNumber = parseInt(req.params.blockNumber);
    if (isNaN(blockNumber) || blockNumber < 0) {
      return res.status(400).json({ error: "Invalid block number" });
    }
    if (typeof req.body.to === 'object') return res.status(400).json({ error: "to must be a string" });
    const to = sanitizeString(req.body.to, 20);
    if (!to) {
      return res.status(400).json({ error: "to (recipient account) is required" });
    }
    if (!validAccountName(to)) return res.status(400).json({ error: "invalid recipient account name" });
    const fromAccount = req.user.username;
    const dream = await transferDream(blockNumber, fromAccount, to);
    res.json({
      message: "Genesis Dream #" + blockNumber + " transferred to " + to,
      original_miner: dream.original_miner,
      dream: dream
    });
  } catch (err) {
    const status = err.message.includes("not found") ? 404
      : err.message.includes("does not own") ? 403
      : 400;
    res.status(status).json({ error: err.message });
  }
}

module.exports = {
  // Core functions
  getDreams,
  getDream,
  inscribeDream,
  transferDream,
  // Express route handlers
  getDreamsRoute,
  getDreamRoute,
  inscribeDreamRoute,
  transferDreamRoute
};
