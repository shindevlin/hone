"use strict";
/**
 * Phone Storage Routes — /api/storage/phone/*
 *
 * Mobile devices host HONE-FS blobs and earn rewards for delivery.
 *
 * GET  /api/storage/phone/assignments  — list blobs assigned to this phone
 * POST /api/storage/phone/heartbeat    — report availability + served count
 */
const express = require('express');
const router = express.Router();
const { authenticateToken } = require('../middlewares/auth');

// Track online phone storage hosts in memory: account → { ip, port, blob_count, served, last_seen }
const phoneHosts = new Map();

/**
 * GET /api/storage/phone/assignments
 * Returns a list of small blobs the phone should host.
 * For now returns an empty list until HONE-FS blob sharding assigns phone shards.
 */
router.get('/assignments', authenticateToken, (req, res) => {
    const account = req.user.username;

    // TODO: query blob sharding registry for phone-sized assignments (<50 MB each)
    // For now return empty so the phone starts up cleanly
    res.json({
        success: true,
        account,
        blobs: [],
        note: 'Phone storage assignments will populate as HONE-FS blob sharding rolls out.',
    });
});

/**
 * POST /api/storage/phone/heartbeat
 * Body: { account, port, blob_count, served_count }
 * Registers/renews the phone's presence in the host registry.
 */
router.post('/heartbeat', authenticateToken, (req, res) => {
    const account = req.user.username;
    const { port, blob_count, served_count } = req.body;
    const ip = req.ip || req.connection.remoteAddress || 'unknown';

    phoneHosts.set(account, {
        account,
        ip,
        port: port || 7878,
        blob_count: blob_count || 0,
        served_count: served_count || 0,
        last_seen: Date.now(),
    });

    // Expire stale entries after 5 minutes
    for (const [k, v] of phoneHosts.entries()) {
        if (Date.now() - v.last_seen > 5 * 60 * 1000) phoneHosts.delete(k);
    }

    res.json({
        success: true,
        registered: true,
        online_hosts: phoneHosts.size,
    });
});

/**
 * GET /api/storage/phone/hosts  (public — chain peers use this to locate phone hosts)
 */
router.get('/hosts', (req, res) => {
    const now = Date.now();
    const online = [];
    for (const v of phoneHosts.values()) {
        if (now - v.last_seen < 3 * 60 * 1000) {
            online.push({ account: v.account, ip: v.ip, port: v.port, blob_count: v.blob_count });
        }
    }
    res.json({ success: true, hosts: online });
});

module.exports = router;
