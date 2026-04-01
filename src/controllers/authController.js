"use strict";
const User = require('../models/User');
const jwt = require('jsonwebtoken');
const { createAccount } = require('../wallet/accountManager');

/**
 * Register New User
 * Creates BTCPC account with BIP-39 mnemonic and wallets on all 7 chains.
 * Mnemonic is shown ONCE — user must save it. We never store it.
 */
async function registerUser(req, res) {
  const { username, password } = req.body;

  try {
    const result = await createAccount(username, null, password);

    res.status(201).json({
      success: true,
      username: result.username,
      mnemonic: result.mnemonic,
      wallets: result.chainWallets,
      publicKeys: result.publicKeys,
      warning: "SAVE YOUR MNEMONIC. This is the only time it will be shown."
    });
  } catch (err) {
    res.status(err.message.includes('already exists') ? 400 : 500).json({ error: err.message });
  }
}

/**
 * Login User
 */
async function loginUser(req, res) {
  const { email, password } = req.body;

  try {
    const user = await User.findOne({ email });
    if (!user || !user.isActive) {
      return res.status(401).json({ error: 'Invalid credentials' });
    }

    const isMatch = require('bcryptjs').compareSync(password, user.password);
    if (!isMatch) {
      return res.status(401).json({ error: 'Invalid credentials' });
    }

    const token = jwt.sign(
      { id: user._id, username: user.username },
      process.env.JWT_SECRET,
      { expiresIn: process.env.JWT_EXPIRES_IN || '7d' }
    );

    res.json({
      success: true,
      token,
      user: { id: user._id, username: user.username, email: user.email }
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
}

/**
 * Link Telegram Account
 */
async function linkTelegram(req, res) {
  const userId = req.user.id;
  const { telegramId, telegramUsername } = req.body;

  try {
    const user = await User.findById(userId);
    if (!user) {
      return res.status(404).json({ error: 'User not found' });
    }

    if (user.telegramId) {
      return res.status(400).json({ error: 'Telegram account already linked' });
    }

    user.telegramId = telegramId;
    user.telegramUsername = telegramUsername;
    await user.save();

    res.json({ success: true, message: 'Telegram account linked successfully' });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
}

/**
 * Enable 2FA for User
 */
async function enable2FA(req, res) {
  const userId = req.user.id;
  const { token } = req.body;

  try {
    // Verify 2FA token (simplified - in production use proper TOTP)
    if (!token || token !== process.env.TWOFA_TOKEN) {
      return res.status(400).json({ error: 'Invalid 2FA token' });
    }

    const user = await User.findById(userId);
    if (user.twoFactorEnabled) {
      return res.json({ success: true, message: '2FA already enabled' });
    }

    user.twoFactorEnabled = true;
    await user.save();

    res.json({ success: true, message: '2FA enabled successfully' });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
}

/**
 * Check if Telegram account is linked
 */
function checkTelegramLink(req, res, next) {
  const { id } = req.user;
  User.findById(id)
    .then(user => {
      req.isTelegramLinked = !!user.telegramId;
      next();
    })
    .catch(next);
}

module.exports = {
  registerUser,
  loginUser,
  linkTelegram,
  enable2FA,
  checkTelegramLink
};
