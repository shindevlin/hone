"use strict";

/**
 * Name Auction — stateStore + ledger integration tests.
 * Tests applyEntry logic for all four auction entry types, plus
 * the stateStore getter functions (getAuction, getOpenAuctions,
 * getAuctionHistory, getAvailableReservedNames).
 */

const stateStore = require('../src/chain/stateStore');
const ledger = require('../src/services/ledger');

const SHIN = 'shindevlin';
const TEST_NAME = 'btctest';
const BIDDER_A = 'alice';
const BIDDER_B = 'bob';

const KEYS_A = { owner: 'STMowner_a', active: 'STMactive_a', posting: 'STMposting_a', memo: 'STMmemo_a' };
const KEYS_B = { owner: 'STMowner_b', active: 'STMactive_b', posting: 'STMposting_b', memo: 'STMmemo_b' };

function makeOpenEntry(name, opts) {
  opts = opts || {};
  return {
    type: 'NAME_AUCTION_OPEN',
    from: SHIN,
    epoch: opts.epoch || 100,
    auction_data: {
      name: name,
      seller: SHIN,
      start_price_usd: opts.start_price_usd !== undefined ? opts.start_price_usd : 10,
      min_bid_increment_usd: opts.min_bid_increment_usd !== undefined ? opts.min_bid_increment_usd : 5,
      auction_end_epoch: opts.auction_end_epoch || 20260,
    },
  };
}

function makeBidEntry(name, bidder, bidUsd, opts) {
  opts = opts || {};
  return {
    type: 'NAME_AUCTION_BID',
    from: bidder,
    epoch: opts.epoch || 200,
    auction_data: {
      name: name,
      bidder: bidder,
      bid_usd: bidUsd,
      chain: opts.chain || 'ethereum',
      tx_hash: opts.tx_hash || '0xabc123',
      btcpc_account: opts.btcpc_account || bidder,
      btcpc_pubkeys: opts.btcpc_pubkeys || { posting: 'STMposting_' + bidder },
    },
  };
}

function makeSettleEntry(name, opts) {
  opts = opts || {};
  return {
    type: 'NAME_AUCTION_SETTLE',
    from: SHIN,
    epoch: opts.epoch || 300,
    auction_data: { name: name, seller_sig: opts.seller_sig || null },
  };
}

function makeCancelEntry(name, opts) {
  opts = opts || {};
  return {
    type: 'NAME_AUCTION_CANCEL',
    from: SHIN,
    epoch: opts.epoch || 300,
    auction_data: { name: name, seller_sig: opts.seller_sig || null },
  };
}

// Create a reserved name account (no public keys = reserved placeholder)
function ensureReservedAccount(name) {
  stateStore.applyEntry({
    type: 'ACCOUNT_CREATE',
    to: name,
    epoch: 1,
    account_data: { public_keys: {}, chain_addresses: {} },
  });
}

// ─────────────────────────────────────────────────────────────────

beforeEach(function() {
  stateStore.resetAll();
  ensureReservedAccount(TEST_NAME);
});

// ─────────────────────────────────────────────────────────────────
// 1. Basic auction open
// ─────────────────────────────────────────────────────────────────

test('opens an auction and records it in state', function() {
  stateStore.applyEntry(makeOpenEntry(TEST_NAME));
  var auction = stateStore.getAuction(TEST_NAME);
  expect(auction).not.toBeNull();
  expect(auction.status).toBe('open');
  expect(auction.name).toBe(TEST_NAME);
  expect(auction.seller).toBe(SHIN);
  expect(auction.start_price_usd).toBe(10);
  expect(auction.min_bid_increment_usd).toBe(5);
  expect(auction.bids).toHaveLength(0);
  expect(auction.winner).toBeNull();
});

// ─────────────────────────────────────────────────────────────────
// 2. Bid placed, top bid updates
// ─────────────────────────────────────────────────────────────────

test('placing a valid bid updates the auction bids array', function() {
  stateStore.applyEntry(makeOpenEntry(TEST_NAME));
  stateStore.applyEntry(makeBidEntry(TEST_NAME, BIDDER_A, 15, { btcpc_pubkeys: KEYS_A }));
  var auction = stateStore.getAuction(TEST_NAME);
  expect(auction.bids).toHaveLength(1);
  expect(auction.bids[0].bidder).toBe(BIDDER_A);
  expect(auction.bids[0].bid_usd).toBe(15);
  expect(auction.bids[0].btcpc_pubkeys).toMatchObject({ posting: KEYS_A.posting });
});

// ─────────────────────────────────────────────────────────────────
// 3. Bid below minimum rejected
// ─────────────────────────────────────────────────────────────────

test('bid below start_price_usd is rejected', function() {
  stateStore.applyEntry(makeOpenEntry(TEST_NAME, { start_price_usd: 50, min_bid_increment_usd: 10 }));
  // First bid must be >= start_price_usd (i.e. 0 + 10 = 10... start_price=50 so min = 50 - 10 + 10 = 50)
  stateStore.applyEntry(makeBidEntry(TEST_NAME, BIDDER_A, 5));
  var auction = stateStore.getAuction(TEST_NAME);
  expect(auction.bids).toHaveLength(0);
});

test('second bid below current_top + increment is rejected', function() {
  stateStore.applyEntry(makeOpenEntry(TEST_NAME, { start_price_usd: 10, min_bid_increment_usd: 5 }));
  stateStore.applyEntry(makeBidEntry(TEST_NAME, BIDDER_A, 15));
  // BIDDER_B bids only 16 — less than 15 + 5 = 20
  stateStore.applyEntry(makeBidEntry(TEST_NAME, BIDDER_B, 16));
  var auction = stateStore.getAuction(TEST_NAME);
  expect(auction.bids).toHaveLength(1);
  expect(auction.bids[0].bidder).toBe(BIDDER_A);
});

// ─────────────────────────────────────────────────────────────────
// 4. Second higher bid wins (bids sorted ascending, last = highest)
// ─────────────────────────────────────────────────────────────────

test('second higher bid becomes top bid', function() {
  stateStore.applyEntry(makeOpenEntry(TEST_NAME));
  stateStore.applyEntry(makeBidEntry(TEST_NAME, BIDDER_A, 15));
  stateStore.applyEntry(makeBidEntry(TEST_NAME, BIDDER_B, 25, { btcpc_pubkeys: KEYS_B }));
  var auction = stateStore.getAuction(TEST_NAME);
  expect(auction.bids).toHaveLength(2);
  var topBid = auction.bids[auction.bids.length - 1];
  expect(topBid.bidder).toBe(BIDDER_B);
  expect(topBid.bid_usd).toBe(25);
});

// ─────────────────────────────────────────────────────────────────
// 5. Settle sets winner to highest bidder
// ─────────────────────────────────────────────────────────────────

test('settle sets status to settled and winner to highest bidder', function() {
  stateStore.applyEntry(makeOpenEntry(TEST_NAME));
  stateStore.applyEntry(makeBidEntry(TEST_NAME, BIDDER_A, 15));
  stateStore.applyEntry(makeBidEntry(TEST_NAME, BIDDER_B, 25, { btcpc_pubkeys: KEYS_B }));
  stateStore.applyEntry(makeSettleEntry(TEST_NAME));
  var auction = stateStore.getAuction(TEST_NAME);
  expect(auction.status).toBe('settled');
  expect(auction.winner).toBe(BIDDER_B);
});

// ─────────────────────────────────────────────────────────────────
// 6. Settle transfers account keys (KEY_ROTATE equivalent)
// ─────────────────────────────────────────────────────────────────

test('settle applies winning bid public keys to the name account', function() {
  stateStore.applyEntry(makeOpenEntry(TEST_NAME));
  stateStore.applyEntry(makeBidEntry(TEST_NAME, BIDDER_B, 25, { btcpc_pubkeys: KEYS_B }));
  stateStore.applyEntry(makeSettleEntry(TEST_NAME));
  var acct = stateStore.getAccount(TEST_NAME);
  expect(acct).not.toBeNull();
  expect(acct.public_keys.posting).toBe(KEYS_B.posting);
  expect(acct.public_keys.active).toBe(KEYS_B.active);
  expect(acct.public_keys.owner).toBe(KEYS_B.owner);
});

// ─────────────────────────────────────────────────────────────────
// 7. Cancel clears auction
// ─────────────────────────────────────────────────────────────────

test('cancel sets status to cancelled', function() {
  stateStore.applyEntry(makeOpenEntry(TEST_NAME));
  stateStore.applyEntry(makeCancelEntry(TEST_NAME));
  var auction = stateStore.getAuction(TEST_NAME);
  expect(auction.status).toBe('cancelled');
  expect(auction.winner).toBeNull();
});

// ─────────────────────────────────────────────────────────────────
// 8. Cannot settle if no bids
// ─────────────────────────────────────────────────────────────────

test('settle with no bids is a no-op (status stays open)', function() {
  stateStore.applyEntry(makeOpenEntry(TEST_NAME));
  stateStore.applyEntry(makeSettleEntry(TEST_NAME));
  var auction = stateStore.getAuction(TEST_NAME);
  expect(auction.status).toBe('open');
});

// ─────────────────────────────────────────────────────────────────
// 9. Cannot open duplicate open auction for same name
// ─────────────────────────────────────────────────────────────────

test('opening a second auction while one is open is a no-op', function() {
  stateStore.applyEntry(makeOpenEntry(TEST_NAME, { start_price_usd: 10 }));
  stateStore.applyEntry(makeOpenEntry(TEST_NAME, { start_price_usd: 999 }));
  var auction = stateStore.getAuction(TEST_NAME);
  expect(auction.start_price_usd).toBe(10); // original, not overwritten
});

// ─────────────────────────────────────────────────────────────────
// 10. getOpenAuctions returns only open auctions
// ─────────────────────────────────────────────────────────────────

test('getOpenAuctions returns only open auctions', function() {
  var name2 = 'btctest2';
  ensureReservedAccount(name2);
  stateStore.applyEntry(makeOpenEntry(TEST_NAME));
  stateStore.applyEntry(makeOpenEntry(name2));
  var open = stateStore.getOpenAuctions();
  expect(open.length).toBe(2);
  expect(open.every(function(a) { return a.status === 'open'; })).toBe(true);
});

// ─────────────────────────────────────────────────────────────────
// 11. getOpenAuctions sorts by auction_end_epoch ASC
// ─────────────────────────────────────────────────────────────────

test('getOpenAuctions sorted by auction_end_epoch ascending', function() {
  var name2 = 'btctest2';
  ensureReservedAccount(name2);
  stateStore.applyEntry(makeOpenEntry(TEST_NAME, { auction_end_epoch: 99999 }));
  stateStore.applyEntry(makeOpenEntry(name2, { auction_end_epoch: 10000 }));
  var open = stateStore.getOpenAuctions();
  expect(open[0].auction_end_epoch).toBeLessThan(open[1].auction_end_epoch);
});

// ─────────────────────────────────────────────────────────────────
// 12. getAuctionHistory returns settled + cancelled
// ─────────────────────────────────────────────────────────────────

test('getAuctionHistory returns settled and cancelled auctions', function() {
  var name2 = 'btctest2';
  ensureReservedAccount(name2);
  stateStore.applyEntry(makeOpenEntry(TEST_NAME));
  stateStore.applyEntry(makeBidEntry(TEST_NAME, BIDDER_A, 15));
  stateStore.applyEntry(makeSettleEntry(TEST_NAME));
  stateStore.applyEntry(makeOpenEntry(name2));
  stateStore.applyEntry(makeCancelEntry(name2));
  var history = stateStore.getAuctionHistory();
  expect(history.length).toBe(2);
  expect(history.some(function(a) { return a.status === 'settled'; })).toBe(true);
  expect(history.some(function(a) { return a.status === 'cancelled'; })).toBe(true);
});

// ─────────────────────────────────────────────────────────────────
// 13. getAuction returns null for unknown name
// ─────────────────────────────────────────────────────────────────

test('getAuction returns null for unknown name', function() {
  expect(stateStore.getAuction('doesnotexist')).toBeNull();
});

// ─────────────────────────────────────────────────────────────────
// 14. resetAll clears name auctions
// ─────────────────────────────────────────────────────────────────

test('resetAll clears name auctions', function() {
  stateStore.applyEntry(makeOpenEntry(TEST_NAME));
  expect(stateStore.getAuction(TEST_NAME)).not.toBeNull();
  stateStore.resetAll();
  expect(stateStore.getAuction(TEST_NAME)).toBeNull();
  expect(stateStore.getOpenAuctions()).toHaveLength(0);
});

// ─────────────────────────────────────────────────────────────────
// 15. getAvailableReservedNames excludes auctioned names
// ─────────────────────────────────────────────────────────────────

test('getAvailableReservedNames excludes names with open auctions', function() {
  ensureReservedAccount('reservedone');
  ensureReservedAccount('reservedtwo');
  stateStore.applyEntry(makeOpenEntry('reservedone'));
  var available = stateStore.getAvailableReservedNames();
  expect(available).not.toContain('reservedone');
  expect(available).toContain('reservedtwo');
});

test('getAvailableReservedNames excludes names with settled auctions', function() {
  ensureReservedAccount('soldoname');
  stateStore.applyEntry(makeOpenEntry('soldoname'));
  stateStore.applyEntry(makeBidEntry('soldoname', BIDDER_A, 15));
  stateStore.applyEntry(makeSettleEntry('soldoname'));
  var available = stateStore.getAvailableReservedNames();
  expect(available).not.toContain('soldoname');
});

test('getAvailableReservedNames includes cancelled-auction names again', function() {
  ensureReservedAccount('cancelname');
  stateStore.applyEntry(makeOpenEntry('cancelname'));
  stateStore.applyEntry(makeCancelEntry('cancelname'));
  var available = stateStore.getAvailableReservedNames();
  expect(available).toContain('cancelname');
});

// ─────────────────────────────────────────────────────────────────
// 16. Bid on non-existent auction is a no-op
// ─────────────────────────────────────────────────────────────────

test('bid on non-existent auction is ignored', function() {
  stateStore.applyEntry(makeBidEntry('nonexistent', BIDDER_A, 100));
  expect(stateStore.getAuction('nonexistent')).toBeNull();
});

// ─────────────────────────────────────────────────────────────────
// 17. Bid on settled auction is a no-op
// ─────────────────────────────────────────────────────────────────

test('bid on settled auction is ignored', function() {
  stateStore.applyEntry(makeOpenEntry(TEST_NAME));
  stateStore.applyEntry(makeBidEntry(TEST_NAME, BIDDER_A, 15));
  stateStore.applyEntry(makeSettleEntry(TEST_NAME));
  stateStore.applyEntry(makeBidEntry(TEST_NAME, BIDDER_B, 999));
  var auction = stateStore.getAuction(TEST_NAME);
  expect(auction.bids).toHaveLength(1); // BIDDER_B not added
  expect(auction.winner).toBe(BIDDER_A);
});

// ─────────────────────────────────────────────────────────────────
// 18. Cancel on already-cancelled auction is a no-op
// ─────────────────────────────────────────────────────────────────

test('cancel on already-cancelled auction is ignored', function() {
  stateStore.applyEntry(makeOpenEntry(TEST_NAME));
  stateStore.applyEntry(makeCancelEntry(TEST_NAME, { epoch: 300 }));
  // try to cancel again — no-op because status != open
  stateStore.applyEntry(makeCancelEntry(TEST_NAME, { epoch: 400 }));
  var auction = stateStore.getAuction(TEST_NAME);
  expect(auction.status).toBe('cancelled');
  expect(auction.cancelled_epoch).toBe(300); // first cancellation epoch preserved
});

// ─────────────────────────────────────────────────────────────────
// 19. ledger.recordNameAuctionOpen produces a valid entry
// ─────────────────────────────────────────────────────────────────

test('ledger.recordNameAuctionOpen returns an entry and applies to state', async function() {
  ensureReservedAccount('myname');
  var entry = await ledger.recordNameAuctionOpen(SHIN, 'myname', 25, 5, 30000, 500);
  expect(entry).toBeDefined();
  expect(entry.type).toBe('NAME_AUCTION_OPEN');
  var auction = stateStore.getAuction('myname');
  expect(auction).not.toBeNull();
  expect(auction.start_price_usd).toBe(25);
  expect(auction.auction_end_epoch).toBe(30000);
});

// ─────────────────────────────────────────────────────────────────
// 20. ledger.recordNameAuctionBid + recordNameAuctionSettle integration
// ─────────────────────────────────────────────────────────────────

test('ledger recordNameAuctionBid then recordNameAuctionSettle full flow', async function() {
  ensureReservedAccount('flowname');
  await ledger.recordNameAuctionOpen(SHIN, 'flowname', 10, 5, 50000, 100);
  await ledger.recordNameAuctionBid('flowname', BIDDER_A, 15, 'ethereum', '0xdeadbeef', BIDDER_A, KEYS_A, 200);
  await ledger.recordNameAuctionBid('flowname', BIDDER_B, 25, 'solana', 'solxyz', BIDDER_B, KEYS_B, 210);
  await ledger.recordNameAuctionSettle('flowname', null, 300);
  var auction = stateStore.getAuction('flowname');
  expect(auction.status).toBe('settled');
  expect(auction.winner).toBe(BIDDER_B);
  var acct = stateStore.getAccount('flowname');
  expect(acct.public_keys.posting).toBe(KEYS_B.posting);
});

// ─────────────────────────────────────────────────────────────────
// 21. ledger.recordNameAuctionCancel
// ─────────────────────────────────────────────────────────────────

test('ledger.recordNameAuctionCancel cancels the auction', async function() {
  ensureReservedAccount('cancelme');
  await ledger.recordNameAuctionOpen(SHIN, 'cancelme', 10, 5, 50000, 100);
  await ledger.recordNameAuctionCancel('cancelme', null, 200);
  var auction = stateStore.getAuction('cancelme');
  expect(auction.status).toBe('cancelled');
});

// ─────────────────────────────────────────────────────────────────
// 22. Bids array stays sorted by bid_usd ascending
// ─────────────────────────────────────────────────────────────────

test('bids array is sorted ascending so last entry is always highest bid', function() {
  stateStore.applyEntry(makeOpenEntry(TEST_NAME, { start_price_usd: 0, min_bid_increment_usd: 1 }));
  var bids = [15, 30, 10, 50, 20];
  var prev = 0;
  for (var i = 0; i < bids.length; i++) {
    var amt = bids[i];
    // Only apply if amt > prev + 1
    if (amt > prev + 1) {
      stateStore.applyEntry(makeBidEntry(TEST_NAME, 'bidder' + i, amt, { tx_hash: '0x' + i }));
      prev = amt;
    }
  }
  var auction = stateStore.getAuction(TEST_NAME);
  for (var j = 1; j < auction.bids.length; j++) {
    expect(auction.bids[j].bid_usd).toBeGreaterThanOrEqual(auction.bids[j-1].bid_usd);
  }
});

// ─────────────────────────────────────────────────────────────────
// 23. ledger recordNameAuctionOpen throws on missing seller
// ─────────────────────────────────────────────────────────────────

test('ledger.recordNameAuctionOpen throws if seller is missing', async function() {
  await expect(ledger.recordNameAuctionOpen('', 'myname', 10, 5, 50000, 100)).rejects.toThrow('seller required');
});

test('ledger.recordNameAuctionBid throws if txHash is missing', async function() {
  await expect(ledger.recordNameAuctionBid('flowname', BIDDER_A, 15, 'ethereum', '', BIDDER_A, KEYS_A, 200)).rejects.toThrow('txHash required');
});
