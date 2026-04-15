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

module.exports = router;
