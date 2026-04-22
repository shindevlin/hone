"use strict";
// Storage challenge-response — proves hosts actually store the data they claim

var crypto = require("crypto");
var stateStore = require("../chain/stateStore");
var blobStore = require("./blobStore");

var CHALLENGE_INTERVAL_EPOCHS = 100; // challenge every 100 epochs
var RESPONSE_WINDOW_EPOCHS = 3;
var MAX_FAILURES = 3;

// Track challenges and failures
var pendingChallenges = new Map(); // challengeId → { host, cid, offset, length, epoch, expected_hash }
var hostFailures = new Map(); // host → consecutive failure count

function shouldChallenge(epochNumber) {
  return epochNumber > 0 && epochNumber % CHALLENGE_INTERVAL_EPOCHS === 0;
}

/**
 * Generate a storage challenge for a host.
 * @param {string} host — the host account to challenge
 * @param {string} cid — the CID to challenge
 * @param {number} epochNumber
 * @param {object} [opts] — { offset, length } overrides for testing
 */
function generateChallenge(host, cid, epochNumber, opts) {
  // Verify the blob exists locally (challenger must have it to verify)
  if (!blobStore.hasBlob(cid)) {
    throw new Error("cannot challenge missing blob: " + cid);
  }
  var id = crypto.randomBytes(16).toString("hex");
  var offset = (opts && opts.offset !== undefined) ? opts.offset : Math.floor(Math.random() * 1024);
  var length = (opts && opts.length !== undefined) ? opts.length : 32;
  // Pre-compute expected hash for verification
  var expected_hash = blobStore.hashBlobRange(cid, offset, length);
  var challenge = {
    id: id,
    host: host,
    cid: cid,
    offset: offset,
    length: length,
    epoch: epochNumber,
    deadline: epochNumber + RESPONSE_WINDOW_EPOCHS,
    expected_hash: expected_hash,
  };
  pendingChallenges.set(id, challenge);
  return challenge;
}

function verifyResponse(challengeId, responseBytes) {
  var challenge = pendingChallenges.get(challengeId);
  if (!challenge) return { valid: false, reason: "unknown challenge" };
  pendingChallenges.delete(challengeId);
  // Verify by hashing the response bytes and comparing to expected_hash
  if (responseBytes && Buffer.isBuffer(responseBytes) && responseBytes.length === challenge.length) {
    var responseHash = crypto.createHash("sha256").update(responseBytes).digest("hex");
    if (responseHash === challenge.expected_hash) {
      hostFailures.delete(challenge.host);
      return { valid: true, host: challenge.host };
    }
    // Hash mismatch
    var failures = (hostFailures.get(challenge.host) || 0) + 1;
    hostFailures.set(challenge.host, failures);
    return { valid: false, host: challenge.host, failures: failures, suspended: failures >= MAX_FAILURES, reason: "range hash mismatch" };
  }
  var failures2 = (hostFailures.get(challenge.host) || 0) + 1;
  hostFailures.set(challenge.host, failures2);
  return { valid: false, host: challenge.host, failures: failures2, suspended: failures2 >= MAX_FAILURES, reason: "invalid response" };
}

function isHostSuspended(host) {
  return (hostFailures.get(host) || 0) >= MAX_FAILURES;
}

function expireOldChallenges(currentEpoch) {
  for (var [id, ch] of pendingChallenges) {
    if (currentEpoch > ch.deadline) {
      var failures = (hostFailures.get(ch.host) || 0) + 1;
      hostFailures.set(ch.host, failures);
      pendingChallenges.delete(id);
    }
  }
}

function _resetForTests() {
  pendingChallenges.clear();
  hostFailures.clear();
}

module.exports = { shouldChallenge, generateChallenge, verifyResponse, isHostSuspended, expireOldChallenges, CHALLENGE_INTERVAL_EPOCHS, _resetForTests };
