"use strict";
const express = require('express');
const router = express.Router();
const {
  registerNode,
  updateNode,
  deregisterNode,
  getNodes,
  submitEpochCommitment,
  getEpochInfo,
  getEpochByNumber
} = require('../controllers/nodeController');
const { authenticateToken } = require('../middlewares/auth');
const { getKnownPeerKeys } = require('../p2p/network');

// Protected routes — require authentication
router.post('/register', authenticateToken, registerNode);
router.put('/update', authenticateToken, updateNode);
router.delete('/deregister', authenticateToken, deregisterNode);
router.post('/epoch/commit', authenticateToken, submitEpochCommitment);

// Public routes
router.get('/list', getNodes);
router.get('/epoch/current', getEpochInfo);
router.get('/epoch/:number', getEpochByNumber);

// Admin: known Noise peer public keys (requires auth)
router.get('/admin/peers/keys', authenticateToken, function (req, res) {
  res.json({ peers: getKnownPeerKeys() });
});

// ── Miner capabilities (v3.1.125+) ───────────────────────────────────────────
const stateStore = require('../chain/stateStore');

// GET /api/node/miners — all miners with advertised capabilities
router.get('/miners', function (req, res) {
  try {
    const all = stateStore.getAllMinerCapabilities();
    res.json({ miners: all, count: all.length });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

// GET /api/node/miners/:name — specific miner's capabilities
router.get('/miners/:name', function (req, res) {
  try {
    const caps = stateStore.getMinerCapabilities(req.params.name);
    if (!caps) return res.status(404).json({ error: 'No capability record for ' + req.params.name });
    res.json(caps);
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

// GET /api/node/network/capabilities — aggregate summary of what the live network can serve
router.get('/network/capabilities', function (req, res) {
  try {
    const summary = stateStore.getNetworkCapabilitySummary();
    res.json(summary);
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

// GET /api/node/miners/capable — list miners that can serve a given job's requirements
// Query params: vision=1, audio=1, code_exec=1, tier=reasoning|fast
router.get('/miners/capable', function (req, res) {
  try {
    const requirements = {};
    if (req.query.vision) requirements.vision = true;
    if (req.query.audio) requirements.audio = true;
    if (req.query.code_exec) requirements.code_exec = true;
    if (req.query.finetune) requirements.finetune = true;
    if (req.query.browser) requirements.browser = true;
    if (req.query.tier) requirements.tier = req.query.tier;
    const capable = stateStore.getCapableMiners(Object.keys(requirements).length ? requirements : null);
    res.json({ miners: capable, count: capable.length, requirements });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

module.exports = router;
