"use strict";

/**
 * Amber Pill geo-pioneer NFT — tests (v2.18)
 *
 * Covers:
 *  1.  First node in a region gets the pill
 *  2.  Second node in same region does NOT get a pill
 *  3.  Different roles in same region each get their own pill
 *  4.  getAmberPillWeight returns 1.5 within first year
 *  5.  getAmberPillWeight returns 1.0 after expiry
 *  6.  Non-pioneer gets weight 1.0
 *  7.  getAllAmberPills returns all awarded
 *  8.  getAmberPillsByAccount filters correctly
 *  9.  AMBER_PILL_MINT ledger entry dispatches correctly (stateStore NFT map)
 *  10. Amber Pill NFT is soulbound in stateStore
 *  11. recordNFTTransfer rejects amber-pill-* token IDs
 *  12. loadFromEntry hydrates in-memory state without duplication
 *  13. checkHeartbeat convenience wrapper
 *  14. resetForTests clears all state
 *  15. getAmberPill(role, region) returns correct record
 */

// Mock dependencies that make I/O calls
const mockMempoolSubmit = jest.fn();
jest.mock('../src/p2p/mempool', () => ({
  submit: (...args) => mockMempoolSubmit(...args),
}));
jest.mock('../src/p2p/network', () => ({
  broadcast: jest.fn(),
  NODE_ID: 'test-node',
}));
jest.mock('../src/p2p/protocol', () => ({
  createMessage: jest.fn(() => ({})),
}));

const amberPill  = require('../src/services/amberPill');
const stateStore = require('../src/chain/stateStore');
const ledger     = require('../src/services/ledger');

function setup() {
  amberPill.resetForTests();
  stateStore.resetAll();
  ledger.flushPendingEntries();
  mockMempoolSubmit.mockReset();
  mockMempoolSubmit.mockReturnValue({ accepted: true });
}

// ─────────────────────────────────────────────────────────────────
// Core pill logic
// ─────────────────────────────────────────────────────────────────

describe('amberPill — core award logic', () => {
  beforeEach(setup);

  test('1. first node in a region gets the pill', () => {
    var pill = amberPill.checkAndAwardPill('shindevlin', 'MINER', 'sv-san-salvador', 100);
    expect(pill).not.toBeNull();
    expect(pill.account).toBe('shindevlin');
    expect(pill.role).toBe('MINER');
    expect(pill.region).toBe('sv-san-salvador');
    expect(pill.epoch).toBe(100);
    expect(pill.nft_id).toBe('amber-pill-miner-sv-san-salvador');
    expect(pill.expires_epoch).toBe(100 + 1051200);
  });

  test('2. second node in same region + role does NOT get a pill', () => {
    amberPill.checkAndAwardPill('shindevlin', 'MINER', 'sv-san-salvador', 100);
    var pill2 = amberPill.checkAndAwardPill('natoshisakamoto', 'MINER', 'sv-san-salvador', 101);
    expect(pill2).toBeNull();
  });

  test('3. different roles in same region each get their own pill', () => {
    var p1 = amberPill.checkAndAwardPill('shindevlin',      'MINER',   'sv-san-salvador', 100);
    var p2 = amberPill.checkAndAwardPill('natoshisakamoto', 'STORAGE',  'sv-san-salvador', 101);
    var p3 = amberPill.checkAndAwardPill('shindevlin',      'CLOCK',   'sv-san-salvador', 102);
    var p4 = amberPill.checkAndAwardPill('natoshisakamoto', 'SENSOR',  'sv-san-salvador', 103);
    var p5 = amberPill.checkAndAwardPill('shindevlin',      'GATEWAY', 'sv-san-salvador', 104);
    expect(p1).not.toBeNull();
    expect(p2).not.toBeNull();
    expect(p3).not.toBeNull();
    expect(p4).not.toBeNull();
    expect(p5).not.toBeNull();
    expect(p1.role).toBe('MINER');
    expect(p2.role).toBe('STORAGE');
    expect(p3.role).toBe('CLOCK');
    expect(p4.role).toBe('SENSOR');
    expect(p5.role).toBe('GATEWAY');
  });

  test('4. getAmberPillWeight returns 1.5 within first year', () => {
    amberPill.checkAndAwardPill('shindevlin', 'MINER', 'sv-san-salvador', 1000);
    // Check at epoch 1001 — well within the bonus window
    var w = amberPill.getAmberPillWeight('shindevlin', 'MINER', 'sv-san-salvador', 1001);
    expect(w).toBe(1.5);
  });

  test('5. getAmberPillWeight returns 1.0 after expiry', () => {
    amberPill.checkAndAwardPill('shindevlin', 'MINER', 'sv-san-salvador', 1000);
    // Exactly at the boundary (>= BONUS_DURATION_EPOCHS away)
    var expiryEpoch = 1000 + amberPill.BONUS_DURATION_EPOCHS;
    var w = amberPill.getAmberPillWeight('shindevlin', 'MINER', 'sv-san-salvador', expiryEpoch);
    expect(w).toBe(1.0);
    // One epoch past expiry
    var w2 = amberPill.getAmberPillWeight('shindevlin', 'MINER', 'sv-san-salvador', expiryEpoch + 1);
    expect(w2).toBe(1.0);
  });

  test('6. non-pioneer gets weight 1.0', () => {
    amberPill.checkAndAwardPill('shindevlin', 'MINER', 'sv-san-salvador', 1000);
    var w = amberPill.getAmberPillWeight('natoshisakamoto', 'MINER', 'sv-san-salvador', 1001);
    expect(w).toBe(1.0);
  });

  test('7. getAllAmberPills returns all awarded', () => {
    amberPill.checkAndAwardPill('shindevlin',      'MINER',   'sv-san-salvador', 100);
    amberPill.checkAndAwardPill('natoshisakamoto', 'STORAGE', 'mx-cdmx',         200);
    var all = amberPill.getAllAmberPills();
    expect(all.length).toBe(2);
    var accounts = all.map(function(p) { return p.account; }).sort();
    expect(accounts).toEqual(['natoshisakamoto', 'shindevlin']);
  });

  test('8. getAmberPillsByAccount filters correctly', () => {
    amberPill.checkAndAwardPill('shindevlin',      'MINER',   'sv-san-salvador', 100);
    amberPill.checkAndAwardPill('natoshisakamoto', 'STORAGE', 'sv-san-salvador', 101);
    amberPill.checkAndAwardPill('shindevlin',      'CLOCK',   'sv-san-salvador', 102);

    var shinPills = amberPill.getAmberPillsByAccount('shindevlin');
    expect(shinPills.length).toBe(2);
    expect(shinPills.every(function(p) { return p.account === 'shindevlin'; })).toBe(true);

    var natoPills = amberPill.getAmberPillsByAccount('natoshisakamoto');
    expect(natoPills.length).toBe(1);
    expect(natoPills[0].role).toBe('STORAGE');
  });

  test('13. checkHeartbeat convenience wrapper mints on first call', () => {
    var pill = amberPill.checkHeartbeat('shindevlin', 'GATEWAY', 'ng-lagos', 500);
    expect(pill).not.toBeNull();
    expect(pill.account).toBe('shindevlin');
    expect(pill.role).toBe('GATEWAY');
    expect(pill.region).toBe('ng-lagos');
  });

  test('13b. checkHeartbeat returns null on second call for same role+region', () => {
    amberPill.checkHeartbeat('shindevlin',      'GATEWAY', 'ng-lagos', 500);
    var pill2 = amberPill.checkHeartbeat('natoshisakamoto', 'GATEWAY', 'ng-lagos', 501);
    expect(pill2).toBeNull();
  });

  test('14. resetForTests clears all state', () => {
    amberPill.checkAndAwardPill('shindevlin', 'MINER', 'sv-san-salvador', 100);
    expect(amberPill.getAllAmberPills().length).toBe(1);
    amberPill.resetForTests();
    expect(amberPill.getAllAmberPills().length).toBe(0);
  });

  test('15. getAmberPill returns correct record for (role, region)', () => {
    amberPill.checkAndAwardPill('shindevlin', 'MINER', 'sv-san-salvador', 100);
    var pill = amberPill.getAmberPill('MINER', 'sv-san-salvador');
    expect(pill).not.toBeNull();
    expect(pill.account).toBe('shindevlin');
    // Different role returns null
    var noPill = amberPill.getAmberPill('STORAGE', 'sv-san-salvador');
    expect(noPill).toBeNull();
  });
});

// ─────────────────────────────────────────────────────────────────
// Ledger + stateStore integration
// ─────────────────────────────────────────────────────────────────

describe('amberPill — ledger + stateStore integration', () => {
  beforeEach(setup);

  test('9. AMBER_PILL_MINT entry dispatches to stateStore nfts Map', async () => {
    var entry = await ledger.recordAmberPillMint(
      'shindevlin', 'MINER', 'sv-san-salvador',
      'amber-pill-miner-sv-san-salvador', 100
    );
    expect(entry.type).toBe('AMBER_PILL_MINT');
    expect(entry.to).toBe('shindevlin');
    expect(entry.amber_pill_data.role).toBe('MINER');
    expect(entry.amber_pill_data.region).toBe('sv-san-salvador');
    expect(entry.amber_pill_data.nft_id).toBe('amber-pill-miner-sv-san-salvador');
    expect(entry.amber_pill_data.expires_epoch).toBe(100 + 1051200);

    // stateStore NFT map should contain the new NFT
    var nft = stateStore.getNFT('amber-pills', 'amber-pill-miner-sv-san-salvador');
    expect(nft).toBeTruthy();
    expect(nft.owner).toBe('shindevlin');
    expect(nft.soulbound).toBe(true);
    expect(nft.metadata.role).toBe('MINER');
    expect(nft.metadata.region).toBe('sv-san-salvador');
  });

  test('10. Amber Pill NFT is soulbound in stateStore', async () => {
    await ledger.recordAmberPillMint(
      'shindevlin', 'MINER', 'sv-san-salvador',
      'amber-pill-miner-sv-san-salvador', 100
    );
    var nft = stateStore.getNFT('amber-pills', 'amber-pill-miner-sv-san-salvador');
    expect(nft.soulbound).toBe(true);
  });

  test('11a. recordNFTTransfer rejects amber-pill-* token IDs (prefix check)', async () => {
    await expect(
      ledger.recordNFTTransfer(
        'shindevlin', 'natoshisakamoto',
        'amber-pills', 'amber-pill-miner-sv-san-salvador',
        100, null
      )
    ).rejects.toThrow('non-transferable');
  });

  test('11b. recordNFTTransfer rejects soulbound amber pill from stateStore', async () => {
    // Mint the pill so it exists in stateStore
    await ledger.recordAmberPillMint(
      'shindevlin', 'MINER', 'sv-san-salvador',
      'amber-pill-miner-sv-san-salvador', 100
    );
    // The prefix check fires first, but the soulbound check would also fire
    await expect(
      ledger.recordNFTTransfer(
        'shindevlin', 'natoshisakamoto',
        'amber-pills', 'amber-pill-miner-sv-san-salvador',
        101, null
      )
    ).rejects.toThrow();
  });
});

// ─────────────────────────────────────────────────────────────────
// loadFromEntry — replay hydration
// ─────────────────────────────────────────────────────────────────

describe('amberPill — loadFromEntry replay hydration', () => {
  beforeEach(setup);

  test('12. loadFromEntry hydrates in-memory state', () => {
    amberPill.loadFromEntry({
      account: 'shindevlin',
      role: 'MINER',
      region: 'sv-san-salvador',
      epoch: 500,
      timestamp: 1000000,
      expires_epoch: 500 + 1051200,
      nft_id: 'amber-pill-miner-sv-san-salvador',
    });
    expect(amberPill.hasAmberPill('shindevlin', 'MINER', 'sv-san-salvador')).toBe(true);
  });

  test('12b. loadFromEntry does not overwrite existing pioneer', () => {
    amberPill.checkAndAwardPill('shindevlin', 'MINER', 'sv-san-salvador', 100);
    // Replay tries to load a different account for same slot
    amberPill.loadFromEntry({
      account: 'natoshisakamoto',
      role: 'MINER',
      region: 'sv-san-salvador',
      epoch: 200,
    });
    // Original pioneer is preserved
    expect(amberPill.hasAmberPill('shindevlin', 'MINER', 'sv-san-salvador')).toBe(true);
    expect(amberPill.hasAmberPill('natoshisakamoto', 'MINER', 'sv-san-salvador')).toBe(false);
  });
});

// ─────────────────────────────────────────────────────────────────
// recordAmberPillMint validation
// ─────────────────────────────────────────────────────────────────

describe('amberPill — recordAmberPillMint validation', () => {
  beforeEach(setup);

  test('rejects missing account', async () => {
    await expect(
      ledger.recordAmberPillMint(null, 'MINER', 'sv', 'amber-pill-miner-sv', 1)
    ).rejects.toThrow('account required');
  });

  test('rejects missing role', async () => {
    await expect(
      ledger.recordAmberPillMint('shindevlin', null, 'sv', 'amber-pill-miner-sv', 1)
    ).rejects.toThrow('role required');
  });

  test('rejects missing region', async () => {
    await expect(
      ledger.recordAmberPillMint('shindevlin', 'MINER', null, 'amber-pill-miner-sv', 1)
    ).rejects.toThrow('region required');
  });

  test('rejects missing nftId', async () => {
    await expect(
      ledger.recordAmberPillMint('shindevlin', 'MINER', 'sv', null, 1)
    ).rejects.toThrow('nftId required');
  });
});
