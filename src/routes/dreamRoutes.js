"use strict";
const express = require("express");
const router = express.Router();
const {
  getDreamsRoute,
  getDreamRoute,
  inscribeDreamRoute,
  transferDreamRoute
} = require("../controllers/dreamController");
const { authenticateToken } = require("../middlewares/auth");

// Public routes
router.get("/dreams/:account", getDreamsRoute);
router.get("/dream/:blockNumber", getDreamRoute);

// Protected routes (auth required)
router.post("/dream/:blockNumber/inscribe", authenticateToken, inscribeDreamRoute);
router.post("/dream/:blockNumber/transfer", authenticateToken, transferDreamRoute);

module.exports = router;
