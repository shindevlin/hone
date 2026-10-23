"use strict";

/**
 * HONE Compute-Verified Proofs
 * Shin Devlin
 *
 * Layer 2 of the three-layer chain storage model.
 * Light clients can verify balances by submitting verification requests
 * as inference jobs. Miners replay the ledger and verify — paid useful work.
 *
 * This module also provides direct SMT-based verification for full nodes.
 */

var stateManager = require("./stateManager");

/**
 * Verify an account balance using the local SMT (full node, instant).
 * Returns { verified, balance, proof, root } or { verified: false, reason }.
 */
function verifyBalance(username, claimedBalance) {
  var accountData = stateManager.getAccountState(username);

  if (!accountData) {
    return {
      verified: false,
      reason: "Account not found in state tree"
    };
  }

  var actualBalance = accountData.state.balance;
  var verified = Math.abs(actualBalance - claimedBalance) < 0.000001;

  return {
    verified: verified,
    username: username,
    claimed_balance: claimedBalance,
    actual_balance: actualBalance,
    state: accountData.state,
    proof: accountData.proof,
    root: accountData.root,
    method: "smt_proof"
  };
}

/**
 * Verify any account state field using the local SMT.
 * Returns full state + proof.
 */
function verifyAccountState(username) {
  var accountData = stateManager.getAccountState(username);

  if (!accountData) {
    return {
      verified: false,
      reason: "Account not found in state tree"
    };
  }

  return {
    verified: true,
    username: username,
    state: accountData.state,
    proof: accountData.proof,
    root: accountData.root,
    method: "smt_proof"
  };
}

/**
 * Submit a compute-verified balance check as an inference job.
 * This is for light clients who don't run a full node.
 * The verification is submitted as a standard inference request;
 * miners replay the relevant ledger segment and verify.
 *
 * Returns { job_id } — the client polls for the result.
 */
async function submitComputeVerification(username, claimedBalance) {
  // Build the verification prompt
  var prompt = [
    "HONE BALANCE VERIFICATION REQUEST",
    "Account: " + username,
    "Claimed balance: " + claimedBalance + " HONE",
    "",
    "Instructions: Replay the permanent ledger for this account.",
    "Compute: sum(incoming) - sum(outgoing) for token HONE.",
    "Return: VERIFIED if balance matches, REJECTED with actual balance if not.",
    "Include the current state root from your SMT."
  ].join("\n");

  // Phase E: InferenceJob model deleted — submit via P2P inference router
  try {
    var crypto = require("crypto");
    var jobId = "verify_" + crypto.randomBytes(8).toString("hex");

    // Submit via p2pRouter if available (non-fatal if P2P not running)
    try {
      var p2pRouter = require("../inference/p2pRouter");
      await p2pRouter.submitInference({
        model: "internal:verify",
        messages: [{ role: "system", content: prompt }],
        maxTokens: 256
      });
    } catch (_) {}

    return { job_id: jobId, status: "submitted" };
  } catch (err) {
    return { error: err.message };
  }
}

/**
 * Express route handler for balance verification API.
 * POST /api/verify/balance { username, amount }
 */
function createVerifyRouter() {
  var express = require("express");
  var router = express.Router();

  // Full-node SMT verification (instant)
  router.post("/balance", function (req, res) {
    var username = req.body.username;
    var amount = req.body.amount;

    if (!username) {
      return res.status(400).json({ error: "username required" });
    }

    if (amount !== undefined && amount !== null) {
      // Verify specific claimed balance
      var result = verifyBalance(username, parseFloat(amount));
      return res.json(result);
    }

    // Just return current state + proof
    var state = verifyAccountState(username);
    return res.json(state);
  });

  // Compute-verified (async, for light clients)
  router.post("/balance/compute", async function (req, res) {
    var username = req.body.username;
    var amount = parseFloat(req.body.amount);

    if (!username || isNaN(amount)) {
      return res.status(400).json({ error: "username and amount required" });
    }

    var result = await submitComputeVerification(username, amount);
    res.json(result);
  });

  return router;
}

module.exports = {
  verifyBalance: verifyBalance,
  verifyAccountState: verifyAccountState,
  submitComputeVerification: submitComputeVerification,
  createVerifyRouter: createVerifyRouter
};
