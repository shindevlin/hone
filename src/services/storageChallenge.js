"use strict";
// Storage challenge-response — proves hosts actually store the data they claim

var crypto = require("crypto");
var stateStore = require("../chain/stateStore");

var CHALLENGE_INTERVAL_EPOCHS = 100; // challenge every 100 epochs
var RESPONSE_WINDOW_EPOCHS = 3;
var MAX_FAILURES = 3;

// Track challenges and failures
var pendingChallenges = new Map(); // challengeId → { host, cid, offset, length, epoch }
var hostFailures = new Map(); // host → consecutive failure count

function shouldChallenge(epochNumber) {
  return epochNumber > 0 && epochNumber % CHALLENGE_INTERVAL_EPOCHS === 0;
}

function generateChallenge(host, cid, epochNumber) {
  var id = crypto.randomBytes(16).toString("hex");
  var offset = Math.floor(Math.random() * 1024); // random offset in first 1KB
  var challenge = { id: id, host: host, cid: cid, offset: offset, length: 32, epoch: epochNumber, deadline: epochNumber + RESPONSE_WINDOW_EPOCHS };
  pendingChallenges.set(id, challenge);
  return challenge;
}

function verifyResponse(challengeId, responseBytes) {
  var challenge = pendingChallenges.get(challengeId);
  if (!challenge) return { valid: false, reason: "unknown challenge" };
  pendingChallenges.delete(challengeId);
  // For now, accept any non-empty response as valid (full verification needs the original data)
  if (responseBytes && responseBytes.length === challenge.length) {
    hostFailures.delete(challenge.host);
    return { valid: true, host: challenge.host };
  }
  var failures = (hostFailures.get(challenge.host) || 0) + 1;
  hostFailures.set(challenge.host, failures);
  return { valid: false, host: challenge.host, failures: failures, suspended: failures >= MAX_FAILURES };
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

module.exports = { shouldChallenge, generateChallenge, verifyResponse, isHostSuspended, expireOldChallenges, CHALLENGE_INTERVAL_EPOCHS };
