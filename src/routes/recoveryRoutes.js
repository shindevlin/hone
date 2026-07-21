"use strict";

/**
 * Recovery Routes
 * Shin Devlin
 *
 * REST endpoints for HONE account recovery (Whitepaper 2.3.3).
 */

var express = require("express");
var router = express.Router();
var {
  requestRecovery,
  contestRecovery,
  completeRecovery,
  getRecoveryStatus
} = require("../controllers/recoveryController");

// DISABLED: Recovery routes are not properly authenticated.
// The controller accepts any owner_signature without cryptographic verification.
// Re-enable after implementing proper secp256k1 signature verification against
// the account's registered owner public key.
//
// router.post("/request", requestRecovery);
// router.post("/contest", contestRecovery);
// router.post("/complete", completeRecovery);
router.get("/status", getRecoveryStatus);
router.get("/status/:account", getRecoveryStatus);

router.post("/request", (_req, res) => res.status(503).json({ error: "Recovery is temporarily disabled pending security upgrade" }));
router.post("/contest", (_req, res) => res.status(503).json({ error: "Recovery is temporarily disabled pending security upgrade" }));
router.post("/complete", (_req, res) => res.status(503).json({ error: "Recovery is temporarily disabled pending security upgrade" }));

module.exports = router;
