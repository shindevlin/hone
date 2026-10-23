"use strict";
const express = require('express');
const router = express.Router();
const { authenticateToken } = require('../middlewares/auth');
const totp = require('../services/totp');
const { sanitizeString } = require('../middlewares/validate');

function validateTotpToken(body) {
  if (typeof body.token === 'object') return null;
  var t = sanitizeString(body.token, 20);
  if (!t || !/^[0-9a-zA-Z]{6,20}$/.test(t)) return null;
  return t;
}

// POST /api/totp/setup — generate secret + QR code
router.post('/setup', authenticateToken, async (req, res) => {
  try {
    const result = await totp.generateSecret(req.user.username);
    res.json({
      success: true,
      secret: result.secret,
      otpauthUrl: result.otpauthUrl,
      qrDataUrl: result.qrDataUrl,
      message: 'Scan the QR code with your authenticator app, then POST /api/totp/enable with the 6-digit code.'
    });
  } catch (err) {
    res.status(400).json({ error: err.message });
  }
});

// POST /api/totp/enable — verify first code and activate 2FA
router.post('/enable', authenticateToken, async (req, res) => {
  try {
    const token = validateTotpToken(req.body);
    if (!token) return res.status(400).json({ error: 'token (6-digit code) is required' });

    const result = await totp.enableTOTP(req.user.username, token);
    res.json({
      success: true,
      message: '2FA enabled. Save your backup codes — they will not be shown again.',
      backupCodes: result.backupCodes
    });
  } catch (err) {
    res.status(400).json({ error: err.message });
  }
});

// POST /api/totp/verify — verify a TOTP code (for testing / bot use)
router.post('/verify', authenticateToken, async (req, res) => {
  try {
    const token = validateTotpToken(req.body);
    if (!token) return res.status(400).json({ error: 'token (6-digit code) is required' });

    const valid = await totp.verifyToken(req.user.username, token);
    res.json({ success: true, valid });
  } catch (err) {
    res.status(400).json({ error: err.message });
  }
});

// POST /api/totp/disable — disable 2FA (requires valid TOTP code)
router.post('/disable', authenticateToken, async (req, res) => {
  try {
    const token = validateTotpToken(req.body);
    if (!token) return res.status(400).json({ error: 'token (6-digit code) is required' });

    await totp.disableTOTP(req.user.username, token);
    res.json({ success: true, message: '2FA disabled.' });
  } catch (err) {
    res.status(400).json({ error: err.message });
  }
});

// POST /api/totp/backup-codes — regenerate backup codes (requires valid TOTP code)
router.post('/backup-codes', authenticateToken, async (req, res) => {
  try {
    const token = validateTotpToken(req.body);
    if (!token) return res.status(400).json({ error: 'token (6-digit code) is required' });

    const result = await totp.generateBackupCodes(req.user.username, token);
    res.json({
      success: true,
      message: 'New backup codes generated. Old codes are invalidated.',
      backupCodes: result.backupCodes
    });
  } catch (err) {
    res.status(400).json({ error: err.message });
  }
});

module.exports = router;
