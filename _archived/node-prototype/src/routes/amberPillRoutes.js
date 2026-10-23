"use strict";

/**
 * Amber Pill HTTP Routes — v2.18
 * Shin Devlin
 *
 * Public read-only endpoints for Amber Pill geo-pioneer NFTs.
 *
 * Routes:
 *   GET /api/amber-pills                 — list all awarded pills
 *   GET /api/amber-pills/:account        — list pills for one account
 */

const express = require('express');
const router = express.Router();
const amberPill = require('../services/amberPill');

/**
 * GET /api/amber-pills
 * Returns all awarded Amber Pill NFTs, optionally filtered by ?role= and/or ?region=
 */
router.get('/', function(req, res) {
  var pills = amberPill.getAllAmberPills();

  var roleFilter   = req.query.role   ? String(req.query.role).toUpperCase()   : null;
  var regionFilter = req.query.region ? String(req.query.region).toLowerCase() : null;

  if (roleFilter) {
    pills = pills.filter(function(p) { return p.role === roleFilter; });
  }
  if (regionFilter) {
    pills = pills.filter(function(p) { return p.region === regionFilter; });
  }

  return res.json({ pills: pills, count: pills.length });
});

/**
 * GET /api/amber-pills/:account
 * Returns all Amber Pills held by the specified account.
 */
router.get('/:account', function(req, res) {
  var account = req.params.account;
  if (!account) {
    return res.status(400).json({ error: 'account required' });
  }

  var pills = amberPill.getAmberPillsByAccount(account);
  return res.json({ account: account, pills: pills, count: pills.length });
});

module.exports = router;
