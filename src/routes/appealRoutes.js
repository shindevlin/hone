"use strict";
const express = require('express');
const router = express.Router();
const { authenticateToken } = require('../middlewares/auth');
const slashing = require('../services/slashing');

/**
 * POST /api/appeal
 * Submit an appeal for a slash record.
 * Requires authentication — the caller must own the slashed account.
 */
router.post('/', authenticateToken, async (req, res) => {
  try {
    const { slash_record_id } = req.body;
    if (!slash_record_id) {
      return res.status(400).json({ error: 'slash_record_id is required' });
    }

    const account = req.user.username;
    const record = await slashing.submitAppeal(account, slash_record_id);

    res.json({
      ok: true,
      appeal: {
        slash_record_id: record._id,
        offenseType: record.offenseType,
        submitted: record.appeal.submitted,
        submittedAtEpoch: record.appeal.submittedAtEpoch,
        deadline: record.appeal.deadline,
        panelSize: record.appeal.panelSize
      }
    });
  } catch (err) {
    const status = err.message.includes('not found') ? 404 : 400;
    res.status(status).json({ error: err.message });
  }
});

/**
 * POST /api/appeal/resolve
 * Resolve an appeal with panel verdicts (authority only).
 * Accepts { slash_record_id, panel_verdicts: [{ verifier, overturn: bool }] }
 */
router.post('/resolve', authenticateToken, async (req, res) => {
  try {
    // Authority gate — only the genesis authority can resolve appeals for now
    const AUTHORITY_ACCOUNT = process.env.BTCPC_AUTHORITY || 'shindevlin';
    if (req.user.username !== AUTHORITY_ACCOUNT) {
      return res.status(403).json({ error: 'Only the authority can resolve appeals' });
    }

    const { slash_record_id, panel_verdicts } = req.body;
    if (!slash_record_id) {
      return res.status(400).json({ error: 'slash_record_id is required' });
    }
    if (!Array.isArray(panel_verdicts) || panel_verdicts.length === 0) {
      return res.status(400).json({ error: 'panel_verdicts must be a non-empty array' });
    }

    // Convert { verifier, overturn: bool } to { verifier, vote: 'overturn'|'uphold' }
    const verdicts = panel_verdicts.map(function (v) {
      return { verifier: v.verifier, vote: v.overturn ? 'overturn' : 'uphold' };
    });

    const record = await slashing.resolveAppeal(slash_record_id, verdicts);

    res.json({
      ok: true,
      outcome: record.appeal.outcome,
      slash_record_id: record._id,
      offenseType: record.offenseType,
      amount: record.amount,
      deregistered: record.deregistered
    });
  } catch (err) {
    const status = err.message.includes('not found') ? 404 : 400;
    res.status(status).json({ error: err.message });
  }
});

module.exports = router;
