"use strict";

/**
 * Recovery Routes
 * Shin Devlin
 *
 * REST endpoints for BTCPC account recovery (Whitepaper 2.3.3).
 */

var express = require("express");
var router = express.Router();
var {
  requestRecovery,
  contestRecovery,
  completeRecovery,
  getRecoveryStatus
} = require("../controllers/recoveryController");

// Recovery does NOT require JWT auth — it uses Owner key signatures.
// The owner key IS the authentication for recovery operations.
router.post("/request", requestRecovery);
router.post("/contest", contestRecovery);
router.post("/complete", completeRecovery);
router.get("/status", getRecoveryStatus);
router.get("/status/:account", getRecoveryStatus);

module.exports = router;
