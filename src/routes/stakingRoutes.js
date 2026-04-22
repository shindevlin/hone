"use strict";
const express = require('express');
const router = express.Router();
const {
  stake,
  unstake,
  withdrawStake,
  getStakingInfo,
  getNetworkStaking,
  getStakeRequirements,
  sponsorStake,
  setStakePolicy,
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
// Public per-role stake requirements (dynamic based on demand)
router.get('/requirements', getStakeRequirements);
// Sponsor another account's stake (anyone can sponsor, earns share of rewards)
router.post('/sponsor', authenticateToken, sponsorStake);
// Admin: set stake policy (free period threshold)
router.post('/policy', authenticateToken, setStakePolicy);

module.exports = router;
