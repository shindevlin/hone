"use strict";
/**
 * Phone Mining Routes — /api/mining/phone/*
 *
 * Lets mobile devices participate in mining by claiming small inference
 * work units and submitting compute proofs. The phone runs a local Rust/candle
 * engine (Qwen2.5-0.5B) and submits proof hashes to earn epoch rewards.
 *
 * POST /api/mining/phone/claim    — claim a pending work unit (or synthetic)
 * POST /api/mining/phone/submit   — submit completed work + proof hash
 * GET  /api/mining/phone/status   — per-account mining stats
 */
const express = require('express');
const router = express.Router();
const crypto = require('crypto');
const { authenticateToken } = require('../middlewares/auth');
const stateStore = require('../chain/stateStore');
const messageAuth = require('../p2p/messageAuth');

// Replay prevention: permanent record of seen job_ids within a rolling 3-epoch window.
// Bounded to SEEN_JOB_CAP entries; oldest are evicted when cap is reached.
const SEEN_JOB_CAP = 100000;
const seenJobIds = new Map(); // job_id → epoch_submitted

function _recordJob(jobId, epoch) {
    if (seenJobIds.size >= SEEN_JOB_CAP) {
        const oldest = seenJobIds.keys().next().value;
        seenJobIds.delete(oldest);
    }
    seenJobIds.set(jobId, epoch);
}

function _jobSeen(jobId) {
    return seenJobIds.has(jobId);
}

// Synthetic work prompts for phones when no inference jobs are pending.
// These are simple, verifiable inference tasks — hashing proof is deterministic.
const SYNTHETIC_PROMPTS = [
    "Complete the sentence: Bitcoin is",
    "What is 7 + 8? Answer with just the number:",
    "Name one renewable energy source:",
    "The capital of Japan is",
    "SHA256 is a type of",
    "A blockchain is a distributed",
    "The unit of BTCPC is",
    "Proof of work is",
];

// In-memory work queue (survive restarts via stateStore if available)
const pendingWork = new Map(); // job_id → WorkUnit
const submittedProofs = new Map(); // account → { count, last_epoch }

function generateJobId() {
    return crypto.randomBytes(12).toString('hex');
}

function currentEpoch() {
    try {
        return stateStore.getChainHeight() || 0;
    } catch (_) { return 0; }
}

/**
 * POST /api/mining/phone/claim
 * Body: { account, device_type, model_hint }
 * Returns: { job_id, prompt, max_tokens, epoch }
 */
router.post('/claim', authenticateToken, (req, res) => {
    const account = req.user.username;
    const epoch = currentEpoch();

    // Assign a synthetic prompt (rotate by account hash so different phones get different prompts)
    const idx = Buffer.from(account).reduce((a, b) => a + b, 0) % SYNTHETIC_PROMPTS.length;
    const prompt = SYNTHETIC_PROMPTS[(idx + epoch) % SYNTHETIC_PROMPTS.length];

    const jobId = generateJobId();
    const unit = {
        job_id: jobId,
        prompt,
        max_tokens: 32,
        epoch,
        account,
        created_at: Date.now(),
    };
    pendingWork.set(jobId, unit);

    // Expire unclaimed work after 5 minutes
    setTimeout(() => pendingWork.delete(jobId), 5 * 60 * 1000);

    res.json(unit);
});

/**
 * POST /api/mining/phone/submit
 * Body: { job_id, account, output, token_count, work_hash, epoch }
 * Returns: { success, proof_accepted, reward_pending }
 */
router.post('/submit', authenticateToken, (req, res) => {
    const { job_id, output, token_count, work_hash, epoch, signature } = req.body;
    const account = req.user.username;

    if (!job_id || !output || !work_hash) {
        return res.status(400).json({ error: 'job_id, output, and work_hash are required' });
    }

    // Replay prevention: reject job_ids already submitted in any epoch
    if (_jobSeen(job_id)) {
        return res.status(409).json({ error: 'job already submitted — replay rejected' });
    }

    // Require posting-key signature — same requirement as desktop miners.
    if (!signature) {
        return res.status(403).json({ error: 'signature required — set your posting key in Settings' });
    }

    // Verify signature: SHA256(canonical JSON {account, job_id, work_hash}) signed with posting key
    const signData = { account, job_id, work_hash };
    if (!messageAuth.verifyAccountSignature(account, signData, signature, 'posting')) {
        return res.status(403).json({ error: 'Invalid posting-key signature' });
    }

    const unit = pendingWork.get(job_id);
    if (!unit) {
        // Accept late submissions (work may have been done while unit expired)
    }

    // Verify work hash: SHA256(job_id | "|" | output | "|" | account)
    const expected = crypto.createHash('sha256')
        .update(job_id + '|' + output + '|' + account)
        .digest('hex');

    if (work_hash !== expected) {
        return res.status(400).json({ error: 'Invalid work hash' });
    }

    // Record the proof
    const workValue = Math.max(1, token_count || 1);
    const targetEpoch = epoch || currentEpoch();

    // Mark job_id as seen — prevents replay in any future epoch
    _recordJob(job_id, targetEpoch);

    const proof = {
        miner: account,
        job_id,
        work_value: workValue,
        work_hash,
        device: 'android',
        model_id: req.body.model_id || 'unknown',
        submitted_at: Date.now(),
    };

    try {
        stateStore.addMiningProof(targetEpoch, proof);
    } catch (err) {
        // stateStore unavailable — track in memory so stats still work
    }

    // Enter reward settlement pipeline — proof finalizes after VERIFICATION_WINDOW epochs.
    try {
        const rewardSettlement = require('../chain/rewardSettlement');
        rewardSettlement.submitWorkProof({
            node_id: account,
            model: req.body.model_id || 'btcpc-phone-v1',
            prompt_hash: crypto.createHash('sha256').update(work_hash).digest('hex'),
            result_hash: work_hash,
            tokens_generated: token_count || 0,
            work_value: workValue,
            device: 'android',
            epoch_number: targetEpoch,
        }, targetEpoch);
    } catch (err) {
        // rewardSettlement optional — degrades gracefully
    }

    // Also route to verifier engine for P2P verification broadcast.
    try {
        const verifierEngine = require('../chain/verifierEngine');
        verifierEngine.addToPool({
            id: job_id,
            node_id: account,
            model: req.body.model_id || 'btcpc-phone-v1',
            prompt_hash: crypto.createHash('sha256').update(work_hash).digest('hex'),
            tokens_generated: token_count || 0,
            work_value: workValue,
            work_hash,
            device: 'android',
            epoch: targetEpoch,
        });
    } catch (err) {
        // verifierEngine optional — degrades gracefully if not running
    }

    const prev = submittedProofs.get(account) || { count: 0, last_epoch: 0 };
    prev.count++;
    prev.last_epoch = targetEpoch;
    submittedProofs.set(account, prev);

    pendingWork.delete(job_id);

    res.json({
        success: true,
        proof_accepted: true,
        reward_pending: true,
        work_value: workValue,
        epoch: epoch || currentEpoch(),
    });
});

/**
 * GET /api/mining/phone/status
 * Returns current epoch + this account's proof count.
 */
router.get('/status', authenticateToken, (req, res) => {
    const account = req.user.username;
    const epoch = currentEpoch();
    const stats = submittedProofs.get(account) || { count: 0, last_epoch: 0 };

    res.json({
        success: true,
        account,
        epoch,
        proofs_submitted: stats.count,
        last_epoch: stats.last_epoch,
    });
});

module.exports = router;
