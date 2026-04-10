"use strict";

/**
 * Commerce tests — exercise the full stateStore + ledger flow for stores,
 * products, orders, disputes, and reputation voting.
 *
 * These tests don't touch Mongo — everything runs against the in-memory
 * stateStore via ledger.recordX functions.
 */

const mockMempoolSubmit = jest.fn();
jest.mock('../src/p2p/mempool', () => ({
  submit: (...args) => mockMempoolSubmit(...args),
}));

const stateStore = require('../src/chain/stateStore');
const ledger = require('../src/services/ledger');
const bondingCurve = require('../src/services/stakeBondingCurve');

describe('commerce (v2.10)', () => {
  beforeEach(() => {
    stateStore.resetAll();
    ledger.flushPendingEntries();
    mockMempoolSubmit.mockReset();
    mockMempoolSubmit.mockReturnValue({ accepted: true });
  });

  describe('bonding curve', () => {
    test('first slots are cheap, scales up with capacity', () => {
      const c1 = bondingCurve.costForCapacity(0, 1);
      const c10 = bondingCurve.costForCapacity(0, 10);
      const c100 = bondingCurve.costForCapacity(0, 100);
      expect(c1).toBeGreaterThan(0);
      expect(c10).toBeGreaterThan(c1 * 9); // nonlinear growth
      expect(c100 / c10).toBeGreaterThan(10); // supra-linear at scale
    });

    test('capacityForPayment inverts costForCapacity', () => {
      const payment = 100; // USD
      const capacity = bondingCurve.capacityForPayment(0, payment);
      const cost = bondingCurve.costForCapacity(0, capacity);
      const costNext = bondingCurve.costForCapacity(0, capacity + 1);
      expect(cost).toBeLessThanOrEqual(payment);
      expect(costNext).toBeGreaterThan(payment);
    });

    test('stakeForCapacity is linear', () => {
      expect(bondingCurve.stakeForCapacity(10)).toBe(10);
      expect(bondingCurve.stakeForCapacity(100)).toBe(100);
    });
  });

  describe('store lifecycle', () => {
    test('opening a store creates the record with capacity + stake', async () => {
      await ledger.recordStoreOpen('alice', { name: 'Alice Shop' }, 10, 10, 12.25, 1);
      const store = stateStore.getStore('alice');
      expect(store).toBeTruthy();
      expect(store.name).toBe('Alice Shop');
      expect(store.capacity).toBe(10);
      expect(store.status).toBe('active');
    });

    test('updating store metadata', async () => {
      await ledger.recordStoreOpen('alice', { name: 'Alice' }, 5, 5, 6, 1);
      await ledger.recordStoreUpdate('alice', { banner_cid: 'Qmbanner', categories: ['home'] }, 2);
      const store = stateStore.getStore('alice');
      expect(store.banner_cid).toBe('Qmbanner');
      expect(store.categories).toEqual(['home']);
    });

    test('closing a store delists its products', async () => {
      await ledger.recordStoreOpen('alice', { name: 'Alice' }, 5, 5, 6, 1);
      await ledger.recordProductCreate('alice', { product_id: 'p1', title: 'W', price: 1, stock: 10 }, 1);
      await ledger.recordProductCreate('alice', { product_id: 'p2', title: 'G', price: 2, stock: 5 }, 1);
      await ledger.recordStoreClose('alice', 2);

      expect(stateStore.getStore('alice').status).toBe('closed');
      expect(stateStore.getProduct('p1').status).toBe('delisted');
      expect(stateStore.getProduct('p2').status).toBe('delisted');
    });

    test('capacity expansion via STAKE_PURCHASE', async () => {
      await ledger.recordStoreOpen('alice', { name: 'Alice' }, 5, 5, 6, 1);
      await ledger.recordStakePurchase('alice', 10, 'wUSDC', 15, 10, 2);
      const store = stateStore.getStore('alice');
      expect(store.capacity).toBe(15);
      expect(store.stake_amount).toBe(15);
      expect(store.stake_paid_usd).toBeCloseTo(21, 5);
    });
  });

  describe('product chain invariants', () => {
    test('product creation requires an active store', async () => {
      await ledger.recordProductCreate('nostore', { product_id: 'p1', title: 'W', price: 1, stock: 10 }, 1);
      expect(stateStore.getProduct('p1')).toBeNull();
    });

    test('product creation rejects when capacity exceeded', async () => {
      await ledger.recordStoreOpen('alice', { name: 'Alice' }, 2, 2, 2.05, 1);
      await ledger.recordProductCreate('alice', { product_id: 'p1', title: 'W', price: 1, stock: 10 }, 1);
      await ledger.recordProductCreate('alice', { product_id: 'p2', title: 'G', price: 2, stock: 5 }, 1);
      // third exceeds capacity
      await ledger.recordProductCreate('alice', { product_id: 'p3', title: 'T', price: 3, stock: 1 }, 1);
      expect(stateStore.getProduct('p1')).toBeTruthy();
      expect(stateStore.getProduct('p2')).toBeTruthy();
      expect(stateStore.getProduct('p3')).toBeNull();
    });

    test('PRODUCT_UPDATE only respected from the seller', async () => {
      await ledger.recordStoreOpen('alice', { name: 'Alice' }, 5, 5, 6, 1);
      await ledger.recordProductCreate('alice', { product_id: 'p1', title: 'W', price: 1, stock: 10 }, 1);
      // mallory tries to update alice's product
      await ledger.recordProductUpdate('mallory', 'p1', { title: 'HIJACKED', price: 0.01 }, 2);
      expect(stateStore.getProduct('p1').title).toBe('W');
      expect(stateStore.getProduct('p1').price).toBe(1);
    });
  });

  describe('order lifecycle + reputation', () => {
    test('place → fulfill → delivered flow updates store counters', async () => {
      await ledger.recordStoreOpen('alice', { name: 'Alice' }, 5, 5, 6, 1);
      await ledger.recordProductCreate('alice', { product_id: 'p1', title: 'W', price: 5, stock: 10 }, 1);
      await ledger.recordOrderPlace('bob', 'alice', 'o1', 'p1', 2, 5, 'BTCPC', 'escrow-1', 2);
      await ledger.recordOrderFulfill('alice', 'o1', 'Qmshipped', 2);
      await ledger.recordOrderDelivered('bob', 'o1', 3);

      const order = stateStore.getOrder('o1');
      const store = stateStore.getStore('alice');
      const product = stateStore.getProduct('p1');

      expect(order.status).toBe('delivered');
      expect(order.fulfillment_cid).toBe('Qmshipped');
      expect(store.total_sales).toBe(1);
      expect(store.total_fulfilled).toBe(1);
      expect(product.stock).toBe(8);
      expect(product.total_sold).toBe(2);
    });

    test('cancellation restores stock', async () => {
      await ledger.recordStoreOpen('alice', { name: 'Alice' }, 5, 5, 6, 1);
      await ledger.recordProductCreate('alice', { product_id: 'p1', title: 'W', price: 5, stock: 10 }, 1);
      await ledger.recordOrderPlace('bob', 'alice', 'o1', 'p1', 3, 5, 'BTCPC', 'escrow-1', 2);
      expect(stateStore.getProduct('p1').stock).toBe(7);
      await ledger.recordOrderCancel('bob', 'o1', 3);
      expect(stateStore.getProduct('p1').stock).toBe(10);
      expect(stateStore.getOrder('o1').status).toBe('cancelled');
    });

    test('buyer can dispute, seller cannot', async () => {
      await ledger.recordStoreOpen('alice', { name: 'Alice' }, 5, 5, 6, 1);
      await ledger.recordProductCreate('alice', { product_id: 'p1', title: 'W', price: 5, stock: 10 }, 1);
      await ledger.recordOrderPlace('bob', 'alice', 'o1', 'p1', 1, 5, 'BTCPC', 'escrow-1', 2);
      // seller can't self-dispute
      await ledger.recordOrderDispute('alice', 'o1', 'fake', 3);
      expect(stateStore.getOrder('o1').status).toBe('placed');
      // buyer can
      await ledger.recordOrderDispute('bob', 'o1', 'never shipped', 3);
      expect(stateStore.getOrder('o1').status).toBe('disputed');
      expect(stateStore.getStore('alice').total_disputed).toBe(1);
    });

    test('reputation vote updates score, same voter re-votes replaces', async () => {
      await ledger.recordStoreOpen('alice', { name: 'Alice' }, 5, 5, 6, 1);
      await ledger.recordReputationVote('bob', 'store', 'alice', 1, 10, 'good', 2);
      expect(stateStore.getStore('alice').rep_votes_up).toBe(10);
      expect(stateStore.getStore('alice').rep_score).toBeGreaterThan(0);

      // Bob flips his vote
      await ledger.recordReputationVote('bob', 'store', 'alice', -1, 10, 'changed my mind', 3);
      expect(stateStore.getStore('alice').rep_votes_up).toBe(0);
      expect(stateStore.getStore('alice').rep_votes_down).toBe(10);
      expect(stateStore.getStore('alice').rep_score).toBeLessThan(0);
    });

    test('miner reputation is tracked independently', async () => {
      await ledger.recordReputationVote('verifier1', 'miner', 'shindevlin', 1, 20, null, 1);
      await ledger.recordReputationVote('verifier2', 'miner', 'shindevlin', 1, 10, null, 1);
      const rep = stateStore.getReputation('miner', 'shindevlin');
      expect(rep.votes_up).toBe(30);
      expect(rep.score).toBeGreaterThan(0);
    });
  });
});
