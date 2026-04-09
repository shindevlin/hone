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
const { requireTOTP } = require('../services/totp');

// Protected delegation routes
router.post('/delegate', authenticateToken, requireTOTP, delegate);
router.post('/undelegate', authenticateToken, requireTOTP, undelegate);
router.post('/withdraw', authenticateToken, requireTOTP, withdrawDelegation);
router.get('/list', authenticateToken, getDelegations);

// Public: view delegations to a miner
router.get('/miner/:miner', getMinerDelegations);

module.exports = router;
