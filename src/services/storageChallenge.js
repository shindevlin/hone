"use strict";
// Storage challenge-response — proves hosts actually store the data they claim

var crypto = require("crypto");
var path = require("path");
var fs = require("fs");
var stateStore = require("../chain/stateStore");

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
 * @param {string} host
 * @param {string} cid
 * @param {number} epochNumber
 * @param {object} [opts] — optional { offset, length } for deterministic testing
 */
function _blobFilePath(cid) {
  var blobDir = process.env.BTCPC_BLOB_DIR || path.resolve(__dirname, "../../data/blobs");
  return path.join(blobDir, cid.slice(0, 2), cid.slice(2, 4), cid);
}

function generateChallenge(host, cid, epochNumber, opts) {
  var blobPath = _blobFilePath(cid);
  if (!fs.existsSync(blobPath)) {
    throw new Error("cannot challenge missing blob: " + cid);
  }
  var id = crypto.randomBytes(16).toString("hex");
  var offset = (opts && opts.offset !== undefined) ? opts.offset : Math.floor(Math.random() * 1024);
  var length = (opts && opts.length !== undefined) ? opts.length : 32;

  // Compute expected hash of the range so we can verify the response
  var stat = fs.statSync(blobPath);
  var safeLen = Math.min(length, Math.max(0, stat.size - offset));
  var expected_hash = null;
  if (safeLen > 0) {
    var fd = fs.openSync(blobPath, "r");
    var buf = Buffer.alloc(safeLen);
    fs.readSync(fd, buf, 0, safeLen, offset);
    fs.closeSync(fd);
    expected_hash = crypto.createHash("sha256").update(buf).digest("hex");
  }

  var challenge = {
    id: id,
    host: host,
    cid: cid,
    offset: offset,
    length: length,
    epoch: epochNumber,
    deadline: epochNumber + RESPONSE_WINDOW_EPOCHS,
    expected_hash: expected_hash
  };
  pendingChallenges.set(id, challenge);
  return challenge;
}

function verifyResponse(challengeId, responseBytes) {
  var challenge = pendingChallenges.get(challengeId);
  if (!challenge) return { valid: false, reason: "unknown challenge" };
  pendingChallenges.delete(challengeId);

  if (!responseBytes || responseBytes.length === 0) {
    var failures0 = (hostFailures.get(challenge.host) || 0) + 1;
    hostFailures.set(challenge.host, failures0);
    return { valid: false, host: challenge.host, reason: "empty response", failures: failures0, suspended: failures0 >= MAX_FAILURES };
  }

  // Verify hash if we have an expected_hash
  if (challenge.expected_hash) {
    var responseHash = crypto.createHash("sha256").update(responseBytes).digest("hex");
    if (responseHash !== challenge.expected_hash) {
      var failures = (hostFailures.get(challenge.host) || 0) + 1;
      hostFailures.set(challenge.host, failures);
      return { valid: false, host: challenge.host, reason: "range hash mismatch", failures: failures, suspended: failures >= MAX_FAILURES };
    }
  }

  hostFailures.delete(challenge.host);
  return { valid: true, host: challenge.host };
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

/**
 * Reset state for testing — clears all pending challenges and failures.
 */
function _resetForTests() {
  pendingChallenges.clear();
  hostFailures.clear();
}

module.exports = { shouldChallenge, generateChallenge, verifyResponse, isHostSuspended, expireOldChallenges, _resetForTests, CHALLENGE_INTERVAL_EPOCHS };
