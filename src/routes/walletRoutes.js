"use strict";
const express = require('express');
const router = express.Router();
const { createWallet, getBalance, transfer, getTransactionHistory } = require('../controllers/walletController');
const { authenticateToken } = require('../middlewares/auth');
const { requireTOTP } = require('../services/totp');

// All wallet routes require authentication
router.post('/create', authenticateToken, createWallet);
router.get('/balance', authenticateToken, getBalance);
router.post('/transfer', authenticateToken, requireTOTP, transfer);
router.get('/transactions', authenticateToken, getTransactionHistory);

module.exports = router;
