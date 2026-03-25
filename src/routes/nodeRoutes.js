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

// Protected routes — require authentication
router.post('/register', authenticateToken, registerNode);
router.put('/update', authenticateToken, updateNode);
router.delete('/deregister', authenticateToken, deregisterNode);
router.post('/epoch/commit', authenticateToken, submitEpochCommitment);

// Public routes
router.get('/list', getNodes);
router.get('/epoch/current', getEpochInfo);
router.get('/epoch/:number', getEpochByNumber);

module.exports = router;
