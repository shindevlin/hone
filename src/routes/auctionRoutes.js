"use strict";

/**
 * Name Auction HTTP Routes — v3.6
 * Shin Devlin
 *
 * Reserved BTCPC names (held by shindevlin in genesis) can be auctioned for
 * USDC / USDT / DAI. Bidders pay on an external chain (Ethereum, Solana, TON),
 * provide their tx hash, and submit their desired public keys. shindevlin
 * verifies payment and settles, which transfers the name account to the winner
 * via KEY_ROTATE.
 *
 * All mutations flow through ledger.recordNameAuctionX → stateStore.
 * No Mongoose. Block files are the canonical source of truth.
 *
 * Routes:
 *   GET  /api/auctions                    — list auctions (public)
 *   GET  /api/auctions/available          — names available to auction (public)
 *   GET  /api/auctions/:name              — single auction detail (public)
 *   POST /api/auctions/open               — open an auction (shindevlin only)
 *   POST /api/auctions/:name/bid          — place a bid (public)
 *   POST /api/auctions/:name/settle       — settle (shindevlin only)
 *   POST /api/auctions/:name/cancel       — cancel (shindevlin only)
 */

const express = require('express');
const router = express.Router();
const ledger = require('../services/ledger');
const stateStore = require('../chain/stateStore');

const SELLER_ACCOUNT = 'shindevlin';

// Valid BTCPC name pattern
const NAME_PATTERN = /^[a-z0-9][a-z0-9._-]{0,19}$/;
// Supported chains
const VALID_CHAINS = ['ethereum', 'solana', 'ton'];

function isValidName(s) {
  return typeof s === 'string' && NAME_PATTERN.test(s);
}

function isValidPubkeys(pk) {
  if (!pk || typeof pk !== 'object') return false;
  // At minimum require a posting key
  return typeof pk.posting === 'string' && pk.posting.length > 0;
}

// ─────────────────────────────────────────────────────────────────
// GET /api/auctions — list auctions
// Query: ?status=open|settled|cancelled|all (default: open)
// ─────────────────────────────────────────────────────────────────
router.get('/', function(req, res) {
  try {
    var status = (req.query.status || 'open').toLowerCase();
    var auctions;
    if (status === 'settled' || status === 'cancelled') {
      auctions = stateStore.getAuctionHistory().filter(function(a) {
        return a.status === status;
      });
    } else if (status === 'all') {
      var open = stateStore.getOpenAuctions();
      var history = stateStore.getAuctionHistory();
      auctions = open.concat(history);
    } else {
      // Default: open
      auctions = stateStore.getOpenAuctions();
    }
    res.json({ auctions: auctions });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

// ─────────────────────────────────────────────────────────────────
// GET /api/auctions/available — names available to be auctioned
// Must come before /:name route
// ─────────────────────────────────────────────────────────────────
router.get('/available', function(req, res) {
  try {
    var names = stateStore.getAvailableReservedNames();
    res.json({ names: names, count: names.length });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

// ─────────────────────────────────────────────────────────────────
// GET /api/auctions/:name — single auction detail
// ─────────────────────────────────────────────────────────────────
router.get('/:name', function(req, res) {
  try {
    var name = String(req.params.name || '').toLowerCase();
    if (!isValidName(name)) {
      return res.status(400).json({ error: 'invalid name format' });
    }
    var auction = stateStore.getAuction(name);
    if (!auction) {
      return res.status(404).json({ error: 'auction not found for name: ' + name });
    }
    res.json({ auction: auction });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

// ─────────────────────────────────────────────────────────────────
// POST /api/auctions/open — open a new auction (shindevlin only)
// Body: { name, start_price_usd, min_bid_increment_usd, duration_epochs?, seller_account }
// ─────────────────────────────────────────────────────────────────
router.post('/open', async function(req, res) {
  try {
    var body = req.body || {};

    // Only shindevlin may open auctions
    var sellerAccount = String(body.seller_account || '').toLowerCase();
    if (sellerAccount !== SELLER_ACCOUNT) {
      return res.status(403).json({ error: 'only shindevlin may open name auctions' });
    }

    var name = String(body.name || '').toLowerCase();
    if (!isValidName(name)) {
      return res.status(400).json({ error: 'invalid name format' });
    }

    var startPrice = parseFloat(body.start_price_usd);
    if (isNaN(startPrice) || startPrice < 0) {
      return res.status(400).json({ error: 'start_price_usd must be a non-negative number' });
    }

    var minIncrement = parseFloat(body.min_bid_increment_usd);
    if (isNaN(minIncrement) || minIncrement <= 0) {
      return res.status(400).json({ error: 'min_bid_increment_usd must be a positive number' });
    }

    // Verify name account exists and is reserved (no active public key)
    if (!stateStore.hasAccount(name)) {
      return res.status(400).json({ error: 'name account does not exist on chain' });
    }

    // Check no existing open auction
    var existing = stateStore.getAuction(name);
    if (existing && existing.status === 'open') {
      return res.status(409).json({ error: 'auction already open for: ' + name });
    }
    if (existing && existing.status === 'settled') {
      return res.status(409).json({ error: 'name already sold: ' + name });
    }

    var epoch = await ledger.getCurrentEpoch();
    var durationEpochs = body.duration_epochs
      ? parseInt(body.duration_epochs, 10)
      : ledger.DEFAULT_AUCTION_DURATION_EPOCHS;
    if (isNaN(durationEpochs) || durationEpochs < 1) {
      return res.status(400).json({ error: 'duration_epochs must be a positive integer' });
    }
    var auctionEndEpoch = epoch + durationEpochs;

    var entry = await ledger.recordNameAuctionOpen(
      SELLER_ACCOUNT,
      name,
      startPrice,
      minIncrement,
      auctionEndEpoch,
      epoch
    );

    res.json({
      ok: true,
      name: name,
      auction_end_epoch: auctionEndEpoch,
      start_price_usd: startPrice,
      min_bid_increment_usd: minIncrement,
      entry: entry,
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

// ─────────────────────────────────────────────────────────────────
// POST /api/auctions/:name/bid — place a bid
// Body: { bidder, bid_usd, chain, tx_hash, btcpc_account, btcpc_pubkeys: { owner, active, posting, memo } }
// ─────────────────────────────────────────────────────────────────
router.post('/:name/bid', async function(req, res) {
  try {
    var name = String(req.params.name || '').toLowerCase();
    if (!isValidName(name)) {
      return res.status(400).json({ error: 'invalid name format' });
    }

    var auction = stateStore.getAuction(name);
    if (!auction) {
      return res.status(404).json({ error: 'no auction found for: ' + name });
    }
    if (auction.status !== 'open') {
      return res.status(409).json({ error: 'auction is not open (status: ' + auction.status + ')' });
    }

    var body = req.body || {};

    var bidder = String(body.bidder || '').trim();
    if (!bidder) {
      return res.status(400).json({ error: 'bidder is required' });
    }

    var bidUsd = parseFloat(body.bid_usd);
    if (isNaN(bidUsd) || bidUsd <= 0) {
      return res.status(400).json({ error: 'bid_usd must be a positive number' });
    }

    // Validate bid exceeds current top + min increment
    var topBid = auction.bids.length > 0
      ? auction.bids[auction.bids.length - 1].bid_usd
      : auction.start_price_usd - auction.min_bid_increment_usd;
    var minRequired = topBid + auction.min_bid_increment_usd;
    if (bidUsd < minRequired) {
      return res.status(400).json({
        error: 'bid_usd must be at least ' + minRequired + ' (current top: ' + topBid + ', min increment: ' + auction.min_bid_increment_usd + ')',
        min_required: minRequired,
        current_top_bid: topBid,
        min_bid_increment_usd: auction.min_bid_increment_usd,
      });
    }

    var chain = String(body.chain || '').toLowerCase();
    if (!VALID_CHAINS.includes(chain)) {
      return res.status(400).json({ error: 'chain must be one of: ' + VALID_CHAINS.join(', ') });
    }

    var txHash = String(body.tx_hash || '').trim();
    if (!txHash) {
      return res.status(400).json({ error: 'tx_hash is required (your stablecoin payment transaction hash)' });
    }

    var btcpcAccount = String(body.btcpc_account || bidder).trim();

    var btcpcPubkeys = body.btcpc_pubkeys || {};
    if (!isValidPubkeys(btcpcPubkeys)) {
      return res.status(400).json({ error: 'btcpc_pubkeys must include at least a posting key' });
    }

    var epoch = await ledger.getCurrentEpoch();
    var entry = await ledger.recordNameAuctionBid(
      name,
      bidder,
      bidUsd,
      chain,
      txHash,
      btcpcAccount,
      btcpcPubkeys,
      epoch
    );

    // Re-fetch auction to get updated bids
    var updated = stateStore.getAuction(name);
    var topBidNow = updated && updated.bids.length > 0
      ? updated.bids[updated.bids.length - 1].bid_usd
      : bidUsd;

    res.json({
      ok: true,
      name: name,
      bidder: bidder,
      bid_usd: bidUsd,
      top_bid_usd: topBidNow,
      bids_count: updated ? updated.bids.length : 1,
      entry: entry,
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

// ─────────────────────────────────────────────────────────────────
// POST /api/auctions/:name/settle — settle (shindevlin only)
// Body: { seller_account }
// ─────────────────────────────────────────────────────────────────
router.post('/:name/settle', async function(req, res) {
  try {
    var name = String(req.params.name || '').toLowerCase();
    if (!isValidName(name)) {
      return res.status(400).json({ error: 'invalid name format' });
    }

    var body = req.body || {};
    var sellerAccount = String(body.seller_account || '').toLowerCase();
    if (sellerAccount !== SELLER_ACCOUNT) {
      return res.status(403).json({ error: 'only shindevlin may settle name auctions' });
    }

    var auction = stateStore.getAuction(name);
    if (!auction) {
      return res.status(404).json({ error: 'no auction found for: ' + name });
    }
    if (auction.status !== 'open') {
      return res.status(409).json({ error: 'auction is not open (status: ' + auction.status + ')' });
    }
    if (auction.bids.length === 0) {
      return res.status(400).json({ error: 'no bids to settle' });
    }

    var epoch = await ledger.getCurrentEpoch();
    var entry = await ledger.recordNameAuctionSettle(name, null, epoch);

    var settled = stateStore.getAuction(name);
    res.json({
      ok: true,
      name: name,
      winner: settled ? settled.winner : null,
      settled_epoch: epoch,
      entry: entry,
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

// ─────────────────────────────────────────────────────────────────
// POST /api/auctions/:name/cancel — cancel (shindevlin only)
// Body: { seller_account }
// ─────────────────────────────────────────────────────────────────
router.post('/:name/cancel', async function(req, res) {
  try {
    var name = String(req.params.name || '').toLowerCase();
    if (!isValidName(name)) {
      return res.status(400).json({ error: 'invalid name format' });
    }

    var body = req.body || {};
    var sellerAccount = String(body.seller_account || '').toLowerCase();
    if (sellerAccount !== SELLER_ACCOUNT) {
      return res.status(403).json({ error: 'only shindevlin may cancel name auctions' });
    }

    var auction = stateStore.getAuction(name);
    if (!auction) {
      return res.status(404).json({ error: 'no auction found for: ' + name });
    }
    if (auction.status !== 'open') {
      return res.status(409).json({ error: 'auction is not open (status: ' + auction.status + ')' });
    }

    var epoch = await ledger.getCurrentEpoch();
    var entry = await ledger.recordNameAuctionCancel(name, null, epoch);

    res.json({
      ok: true,
      name: name,
      cancelled_epoch: epoch,
      entry: entry,
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

module.exports = router;
