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

// Protected staking routes
router.post('/stake', authenticateToken, stake);
router.post('/unstake', authenticateToken, unstake);
router.post('/withdraw', authenticateToken, withdrawStake);
router.get('/info', authenticateToken, getStakingInfo);

// Public network stats
router.get('/network', getNetworkStaking);

module.exports = router;
