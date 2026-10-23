"use strict";

const crypto = require("crypto");
const nodeRegistry = require("../chain/nodeRegistry");
const stateStore = require("../chain/stateStore");

const DEFAULT_MIN_STAKE = parseFloat(process.env.HONE_REVIEWER_MIN_STAKE || "100");
const DEFAULT_COMMITTEE_SIZE = parseInt(process.env.HONE_REVIEWER_COMMITTEE_SIZE || "3", 10) || 3;

function _seed(jobId, stage, epoch, size) {
  return crypto
    .createHash("sha256")
    .update([jobId || "", stage || "review", epoch || 0, size || DEFAULT_COMMITTEE_SIZE].join(":"))
    .digest("hex");
}

function _scoreNode(node, category) {
  if (!node || !node.username) return 0;
  const stake = Math.max(0, Number(node.stake) || 0);
  if (stake < DEFAULT_MIN_STAKE) return 0;

  const rep = stateStore.getNodeReputation ? stateStore.getNodeReputation(node.username) : null;
  const reviewRep = rep && rep.review ? rep.review : null;
  const repScore = reviewRep && typeof reviewRep.score === "number" ? reviewRep.score : 50;
  const repFactor = Math.max(0.5, 1 + (repScore / 200));
  const stakeFactor = Math.sqrt(stake);
  const ageFactor = node.registeredEpoch ? Math.min(1.5, 1 + Math.min(1000, node.registeredEpoch) / 5000) : 1;
  return stakeFactor * repFactor * ageFactor;
}

function _pickWeightedWithoutReplacement(candidates, committeeSize, seedHex, category) {
  const selected = [];
  const remaining = candidates.slice();

  for (let i = 0; i < committeeSize && remaining.length > 0; i++) {
    const seed = crypto.createHash("sha256").update(seedHex + ":" + i).digest();
    let totalWeight = 0;
    const weighted = remaining.map((node) => {
      const weight = _scoreNode(node, category);
      totalWeight += weight;
      return { node, weight };
    }).filter((entry) => entry.weight > 0);

    if (weighted.length === 0 || totalWeight <= 0) break;

    const pick = seed.readUInt32BE(0) / 0x100000000 * totalWeight;
    let cursor = 0;
    let chosenIndex = -1;
    for (let j = 0; j < weighted.length; j++) {
      cursor += weighted[j].weight;
      if (pick <= cursor) {
        chosenIndex = remaining.findIndex((n) => n.username === weighted[j].node.username);
        break;
      }
    }
    if (chosenIndex < 0) chosenIndex = remaining.findIndex((n) => n.username === weighted[weighted.length - 1].node.username);
    if (chosenIndex < 0) break;
    selected.push(remaining[chosenIndex]);
    remaining.splice(chosenIndex, 1);
  }

  return selected;
}

function getEligibleReviewers(job, opts) {
  opts = opts || {};
  const mode = opts.mode || (job && job.review_mode) || "computer";
  const minStake = Number.isFinite(opts.minStake) ? opts.minStake : DEFAULT_MIN_STAKE;
  const exclude = new Set([job && job.buyer, job && job.miner].filter(Boolean));

  const nodes = nodeRegistry.getRegisteredNodes().filter((node) => {
    if (!node || exclude.has(node.username)) return false;
    if ((Number(node.stake) || 0) < minStake) return false;
    if (mode === "human" && node.permissioned === false && (Number(node.stake) || 0) < minStake) return false;
    return true;
  });

  return nodes;
}

function selectCommittee(job, opts) {
  opts = opts || {};
  const mode = opts.mode || (job && job.review_mode) || "computer";
  const committeeSize = Math.max(1, parseInt(opts.committeeSize || DEFAULT_COMMITTEE_SIZE, 10) || DEFAULT_COMMITTEE_SIZE);
  const stage = opts.stage || (job && job.review_stage) || "initial";
  const epoch = opts.epoch || 0;
  const candidates = getEligibleReviewers(job, { mode, minStake: opts.minStake });
  const seedHex = _seed(job && job.job_id, stage, epoch, committeeSize);
  const committee = _pickWeightedWithoutReplacement(candidates, committeeSize, seedHex, "verifier");
  const committeeAccounts = committee.map((n) => n.username);
  const committeeHash = crypto.createHash("sha256").update(JSON.stringify({
    job_id: job && job.job_id,
    stage,
    mode,
    epoch,
    committee: committeeAccounts,
  })).digest("hex");

  return {
    committee: committeeAccounts,
    committee_hash: committeeHash,
    committee_size: committeeAccounts.length,
    mode,
    stage,
  };
}

module.exports = {
  DEFAULT_MIN_STAKE,
  DEFAULT_COMMITTEE_SIZE,
  getEligibleReviewers,
  selectCommittee,
};
