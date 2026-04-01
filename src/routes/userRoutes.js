"use strict";
const express = require('express');
const router = express.Router();
const { registerUser, loginUser, linkTelegram, enable2FA } = require('../controllers/authController');
const { authenticateToken } = require('../middlewares/auth');

// Public routes
router.post('/register', registerUser);
router.post('/login', loginUser);

// Protected routes
router.post('/link-telegram', authenticateToken, linkTelegram);
router.post('/enable-2fa', authenticateToken, enable2FA);

// On-chain Telegram verification
const { postVerification } = require('../services/telegramVerify');
router.post('/verify-telegram', authenticateToken, async (req, res) => {
  try {
    const { challenge } = req.body;
    if (!challenge) return res.status(400).json({ error: 'challenge is required' });
    const result = await postVerification(req.user.id, challenge);
    res.json(result);
  } catch (err) {
    res.status(400).json({ error: err.message });
  }
});

module.exports = router;