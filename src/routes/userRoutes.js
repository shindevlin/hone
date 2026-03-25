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

module.exports = router;