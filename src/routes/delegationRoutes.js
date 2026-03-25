"use strict";
const express = require('express');
const router = express.Router();
const {
  delegate,
  undelegate,
  withdrawDelegation,
  getDelegations,
  getMinerDelegations
} = require('../controllers/delegationController');
const { authenticateToken } = require('../middlewares/auth');

// Protected delegation routes
router.post('/delegate', authenticateToken, delegate);
router.post('/undelegate', authenticateToken, undelegate);
router.post('/withdraw', authenticateToken, withdrawDelegation);
router.get('/list', authenticateToken, getDelegations);

// Public: view delegations to a miner
router.get('/miner/:miner', getMinerDelegations);

module.exports = router;
