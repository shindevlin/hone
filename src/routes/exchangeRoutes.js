"use strict";

const express = require('express');
const router = express.Router();
const ledger = require('../services/ledger');
const stateStore = require('../chain/stateStore');
const HoneNativeExchange = require('../chain/HoneNativeExchange');
const oracleFeeds = require('../services/oracleFeeds');

/**
 * GET /api/exchange/quote
 * Returns the current price and FIFO queue status.
 */
router.get('/quote', (req, res) => {
    const exchangeStats = stateStore.getExchangeStats ? stateStore.getExchangeStats() : { total_volume: 0 };
    const price = HoneNativeExchange.calculatePrice(exchangeStats.total_volume, oracleFeeds);
    const available = stateStore.availableWhoneInQueue ? stateStore.availableWhoneInQueue() : 0;

    res.json({
        ok: true,
        price_usd: price,
        available_hone: available,
        queue_length: stateStore.getExchangeQueueLength ? stateStore.getExchangeQueueLength() : 0
    });
});

/**
 * POST /api/exchange/deposit
 * User deposits HONE into the FIFO queue.
 */
router.post('/deposit', async (req, res) => {
    const { seller, amount } = req.body;
    
    if (!seller || !amount) {
        return res.status(400).json({ error: 'seller and amount are required' });
    }

    try {
        const epoch = await ledger.getCurrentEpoch();
        await ledger.recordExchangeDeposit(seller, parseFloat(amount), epoch);
        res.json({ ok: true, message: 'Exchange deposit recorded' });
    } catch (err) {
        res.status(400).json({ error: err.message });
    }
});

/**
 * POST /api/exchange/buy
 * User buys HONE using stablecoins.
 */
router.post('/buy', async (req, res) => {
    const { buyer, token, amount } = req.body;
    
    if (!buyer || !token || !amount) {
        return res.status(400).json({ error: 'buyer, token, and amount are required' });
    }

    try {
        const epoch = await ledger.getCurrentEpoch();
        await ledger.recordExchangeBuy(buyer, token, parseFloat(amount), epoch);
        res.json({ ok: true, message: 'Exchange purchase recorded' });
    } catch (err) {
        res.status(400).json({ error: err.message });
    }
});

module.exports = router;
