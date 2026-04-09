"use strict";
const express = require('express');
const router = express.Router();
const {
  stake,
  unstake,
  withdrawStake,
  getStakingInfo,
  getNetworkStaking
} = require('../controllers/stakingController');
const { authenticateToken } = require('../middlewares/auth');
const { requireTOTP } = require('../services/totp');

// Protected staking routes
router.post('/stake', authenticateToken, requireTOTP, stake);
router.post('/unstake', authenticateToken, requireTOTP, unstake);
router.post('/withdraw', authenticateToken, requireTOTP, withdrawStake);
router.get('/info', authenticateToken, getStakingInfo);

// Public network stats
router.get('/network', getNetworkStaking);

module.exports = router;
