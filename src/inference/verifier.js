"use strict";

/**
 * BTCPC Inference Verifier
 * Shin Devlin
 *
 * Random nodes verify inference output quality and compute validity.
 * Verifiers see the full response (not the prompt) and check:
 *   1. Did real compute happen? (timing, tokens, model fingerprint)
 *   2. Is the output real? (not garbage, not copied, not template)
 *   3. Is it useful? (coherent, substantive, answers something)
 *
 * Verifiers are randomly selected per job based on block hash.
 * Any node can verify — no GPU required. Clock nodes, light nodes, anyone.
 *
 * Earning: miner gets 85%, verifiers split 15%
 */

var crypto = require("crypto");

// Verification count scales with network size
var VERIFIER_TIERS = [
  { minNodes: 0,   verifiers: 1 },
  { minNodes: 10,  verifiers: 3 },
  { minNodes: 50,  verifiers: 5 },
  { minNodes: 200, verifiers: 7 }
];

var MINER_SHARE = 0.85;
var VERIFIER_SHARE = 0.15;

// Minimum quality thresholds
var MIN_TOKENS = 10;           // at least 10 tokens output
var MIN_OUTPUT_LENGTH = 20;    // at least 20 characters
var MAX_REPEAT_RATIO = 0.5;    // no more than 50% repeated phrases
var MIN_UNIQUE_WORDS = 5;      // at least 5 unique words

/**
 * Determine how many verifiers needed based on network size.
 */
function getVerifierCount(totalNodes) {
  var count = 1;
  for (var i = 0; i < VERIFIER_TIERS.length; i++) {
    if (totalNodes >= VERIFIER_TIERS[i].minNodes) {
      count = VERIFIER_TIERS[i].verifiers;
    }
  }
  return count;
}

/**
 * Get verifier policy: how many verifiers to use, capped by opt-in supply.
 * @param {number} totalNodes — total registered nodes
 * @param {number} optInCount — nodes that have opted into verification
 * @returns {{ count: number, phase: string }}
 */
function getVerifierPolicy(totalNodes, optInCount) {
  if (!optInCount || optInCount === 0) {
    return { count: 0, phase: "no-verifiers" };
  }
  var ideal = getVerifierCount(totalNodes);
  var count = Math.min(ideal, optInCount);
  var phase = totalNodes < 10 ? "bootstrap" : totalNodes < 50 ? "early" : totalNodes < 200 ? "growth" : "mature";
  return { count: count, phase: phase };
}

/**
 * Deterministically decide whether a specific job should be verified.
 * Based on jobId + blockHash + coverage probability.
 * @param {string} jobId
 * @param {string} blockHash
 * @param {number} probability — 0..1
 * @returns {boolean}
 */
function shouldVerifyJob(jobId, blockHash, probability) {
  if (probability <= 0) return false;
  if (probability >= 1) return true;
  var hash = crypto.createHash("sha256")
    .update((jobId || "") + ":" + (blockHash || ""))
    .digest();
  var val = hash.readUInt32BE(0) / 0xffffffff;
  return val < probability;
}

/**
 * Select random verifiers for a job.
 * Deterministic based on block hash + job ID — everyone computes the same selection.
 * Excludes the miner who did the work.
 *
 * @param {string} blockHash — current or previous block hash
 * @param {string} jobId — the inference job ID
 * @param {string} minerAccount — the miner who did the work (excluded)
 * @param {Array<string>} allNodes — list of all registered node usernames
 * @param {number} count — number of verifiers to select
 * @returns {string[]} selected verifier usernames
 */
function selectVerifiers(blockHash, jobId, minerAccount, allNodes, count) {
  // Remove the miner from candidates
  var candidates = allNodes.filter(function (n) { return n !== minerAccount; });
  if (candidates.length === 0) return [];
  if (candidates.length <= count) return candidates;

  // Deterministic shuffle using hash
  var seed = crypto.createHash("sha256")
    .update((blockHash || "0") + ":" + jobId)
    .digest("hex");

  var selected = [];
  var remaining = candidates.slice();

  for (var i = 0; i < count && remaining.length > 0; i++) {
    var stepSeed = crypto.createHash("sha256")
      .update(seed + ":" + i)
      .digest();
    var index = stepSeed.readUInt32BE(0) % remaining.length;
    selected.push(remaining[index]);
    remaining.splice(index, 1);
  }

  return selected;
}

/**
 * Verify an inference result.
 * Called by verifier nodes when they receive a VERIFY_REQUEST.
 *
 * @param {object} work — { response, model, token_count, elapsed_ms, tokens_per_second }
 * @returns {{ valid: boolean, score: number, reasons: string[] }}
 */
function verifyWork(work) {
  var reasons = [];
  var score = 100; // start at 100, deduct for issues

  var response = work.response || "";
  var tokenCount = work.token_count || 0;
  var elapsedMs = work.elapsed_ms || 0;
  var tokensPerSecond = work.tokens_per_second || 0;
  var model = work.model || "";

  // ── 1. Did real compute happen? ──

  // Token count must be positive
  if (tokenCount < MIN_TOKENS) {
    reasons.push("Too few tokens: " + tokenCount);
    score -= 40;
  }

  // Response must exist and have content
  if (response.length < MIN_OUTPUT_LENGTH) {
    reasons.push("Response too short: " + response.length + " chars");
    score -= 30;
  }

  // Timing must be realistic
  if (elapsedMs > 0 && tokenCount > 0) {
    var actualTps = tokenCount / (elapsedMs / 1000);

    // Too fast = probably faked (no GPU can do 10000 tok/s on a 4B+ model)
    if (actualTps > 5000) {
      reasons.push("Unrealistic speed: " + actualTps.toFixed(0) + " tok/s");
      score -= 50;
    }

    // Too slow might be legitimate (CPU inference) but flag if extreme
    if (actualTps < 0.1 && tokenCount > 50) {
      reasons.push("Extremely slow: " + actualTps.toFixed(2) + " tok/s");
      score -= 10;
    }
  }

  // ── 2. Is the output real? ──

  // Check for garbage/random bytes
  var printableRatio = (response.match(/[\x20-\x7E\n\r\t]/g) || []).length / Math.max(response.length, 1);
  if (printableRatio < 0.8) {
    reasons.push("Low printable char ratio: " + (printableRatio * 100).toFixed(0) + "%");
    score -= 30;
  }

  // Check for excessive repetition
  var words = response.toLowerCase().split(/\s+/).filter(Boolean);
  var uniqueWords = new Set(words);
  if (words.length > 20 && uniqueWords.size < MIN_UNIQUE_WORDS) {
    reasons.push("Too few unique words: " + uniqueWords.size);
    score -= 20;
  }

  // Check for repeated phrases (copy-paste detection)
  if (words.length > 20) {
    var trigrams = {};
    for (var i = 0; i < words.length - 2; i++) {
      var tri = words[i] + " " + words[i + 1] + " " + words[i + 2];
      trigrams[tri] = (trigrams[tri] || 0) + 1;
    }
    var repeatedTrigrams = Object.values(trigrams).filter(function (c) { return c > 2; }).length;
    var trigramCount = Math.max(Object.keys(trigrams).length, 1);
    if (repeatedTrigrams / trigramCount > MAX_REPEAT_RATIO) {
      reasons.push("Excessive repetition: " + (repeatedTrigrams / trigramCount * 100).toFixed(0) + "% repeated phrases");
      score -= 20;
    }
  }

  // ── 3. Is it useful? ──

  // Check for common cop-out responses
  var copouts = [
    "i can't help with that",
    "i'm unable to",
    "as an ai language model",
    "i don't have access to",
    "lorem ipsum",
    "test test test"
  ];

  var lowerResponse = response.toLowerCase();
  for (var j = 0; j < copouts.length; j++) {
    if (lowerResponse.startsWith(copouts[j])) {
      reasons.push("Cop-out response detected: " + copouts[j]);
      score -= 15;
      break;
    }
  }

  // Check minimum substance — at least some sentences
  var sentences = response.split(/[.!?]+/).filter(function (s) { return s.trim().length > 10; });
  if (sentences.length < 2 && tokenCount > 50) {
    reasons.push("Low substance: " + sentences.length + " sentence(s) for " + tokenCount + " tokens");
    score -= 10;
  }

  // ── Final verdict ──
  score = Math.max(0, Math.min(100, score));
  var valid = score >= 50; // 50+ = valid work

  return {
    valid: valid,
    score: score,
    reasons: reasons,
    checks: {
      compute_proof: tokenCount >= MIN_TOKENS && elapsedMs > 0,
      real_output: printableRatio >= 0.8 && uniqueWords.size >= MIN_UNIQUE_WORDS,
      useful: sentences.length >= 2 || tokenCount <= 50,
      timing_realistic: tokensPerSecond < 5000
    }
  };
}

/**
 * Calculate reward split between miner and verifiers.
 */
function calculateRewardSplit(totalFee, verifierCount) {
  var minerReward = parseFloat((totalFee * MINER_SHARE).toFixed(10));
  var totalVerifierReward = parseFloat((totalFee * VERIFIER_SHARE).toFixed(10));
  var perVerifier = verifierCount > 0 ? parseFloat((totalVerifierReward / verifierCount).toFixed(10)) : 0;

  return {
    miner: minerReward,
    perVerifier: perVerifier,
    totalVerifier: totalVerifierReward,
    verifierCount: verifierCount
  };
}

/**
 * Aggregate verification votes.
 * Majority rules — if majority says valid, the work is accepted.
 */
function aggregateVotes(votes) {
  var validCount = 0;
  var invalidCount = 0;
  var totalScore = 0;

  for (var i = 0; i < votes.length; i++) {
    if (votes[i].valid) validCount++;
    else invalidCount++;
    totalScore += votes[i].score || 0;
  }

  return {
    accepted: validCount > invalidCount,
    validVotes: validCount,
    invalidVotes: invalidCount,
    totalVotes: votes.length,
    averageScore: votes.length > 0 ? Math.round(totalScore / votes.length) : 0
  };
}

/**
 * Return the verifier policy for the current network state.
 * Caps panel size at the number of opt-in verifiers.
 * @param {number} totalNodes — total network nodes
 * @param {number} optInCount — nodes that have opted into verification
 * @returns {{ count: number, phase: string }}
 */
function getVerifierPolicy(totalNodes, optInCount) {
  if (!optInCount || optInCount <= 0) {
    return { count: 0, phase: 'no-verifiers' };
  }
  var ideal = getVerifierCount(totalNodes);
  var count = Math.min(ideal, optInCount);
  var phase = count >= 7 ? 'full' : count >= 3 ? 'growing' : 'bootstrap';
  return { count: count, phase: phase };
}

/**
 * Deterministically decide whether a given job should be verified,
 * based on the job ID, block hash, and a coverage probability (0-1).
 * Returns true if the job falls within the coverage window.
 * @param {string} jobId
 * @param {string} blockHash
 * @param {number} probability — fraction of jobs to verify (0=none, 1=all)
 * @returns {boolean}
 */
function shouldVerifyJob(jobId, blockHash, probability) {
  if (probability <= 0) return false;
  if (probability >= 1) return true;
  var hash = crypto.createHash('sha256').update(jobId + ':' + blockHash).digest('hex');
  var sample = parseInt(hash.slice(0, 8), 16) / 0xffffffff;
  return sample < probability;
}

module.exports = {
  getVerifierCount: getVerifierCount,
  getVerifierPolicy: getVerifierPolicy,
  shouldVerifyJob: shouldVerifyJob,
  selectVerifiers: selectVerifiers,
  verifyWork: verifyWork,
  calculateRewardSplit: calculateRewardSplit,
  aggregateVotes: aggregateVotes,
  MINER_SHARE: MINER_SHARE,
  VERIFIER_SHARE: VERIFIER_SHARE,
  VERIFIER_TIERS: VERIFIER_TIERS
};
