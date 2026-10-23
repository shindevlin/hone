"use strict";

/**
 * Host selector tests — v2.11.2
 *
 * Tests the protocol-driven host selection + rebalance detection.
 * Seeds a stateStore with a population of hosts with varying
 * capabilities, regions, reputations, and uptimes, then verifies
 * the selector picks sensibly.
 */

const mockMempoolSubmit = jest.fn();
jest.mock('../src/p2p/mempool', () => ({
  submit: (...args) => mockMempoolSubmit(...args),
}));

const stateStore = require('../src/chain/stateStore');
const ledger = require('../src/services/ledger');
const selector = require('../src/services/blobHostSelector');
const epochBandwidth = require('../src/services/epochBandwidth');

const CID_A = 'a'.repeat(64);
const CID_B = 'b'.repeat(64);

async function seedHost(name, opts) {
  opts = opts || {};
  // Create account first
  stateStore.applyEntry({
    type: 'ACCOUNT_CREATE',
    to: name,
    epoch: 0,
    account_data: { username: name, public_keys: {}, chain_addresses: {} },
    timestamp: Math.random(),
  });
  // Register with specific capabilities
  await ledger.recordNodeRegister(
    name,
    opts.node_types || ['cold_host'],
    opts.p2p_address || 'ws://' + name + ':9000',
    true,
    opts.epoch || 1,
    {
      storage_capacity_gb: opts.capacity_gb || 100,
      lora_region: opts.region || 'US',
    }
  );
  // Seed heartbeats for uptime factor
  if (opts.heartbeat_count && opts.heartbeat_count > 0) {
    for (let e = 1; e <= opts.heartbeat_count * 5; e += 5) {
      await ledger.recordStorageHeartbeat(name, [], 0, e);
    }
  }
  // Seed reputation if requested
  if (opts.reputation && opts.reputation > 0) {
    await ledger.recordReputationVote(
      'protocol',
      'storage_host',
      name,
      1,
      opts.reputation,
      null,
      1
    );
  }
}

describe('blobHostSelector (v2.11.2)', () => {
  beforeEach(() => {
    stateStore.resetAll();
    epochBandwidth.resetAll();
    ledger.flushPendingEntries();
    mockMempoolSubmit.mockReset();
    mockMempoolSubmit.mockReturnValue({ accepted: true });
    // Seed common test accounts with enough EB
    ['alice', 'bob', 'carol', 'dave', 'eve', 'frank', 'grace', 'henry', 'ida', 'jack', 'protocol'].forEach(
      a => epochBandwidth.seedForTest(a)
    );
  });

  describe('basic selection', () => {
    test('empty pool returns under-replicated', () => {
      const result = selector.selectHosts({
        target_active: 3,
        target_cold: 5,
        current_epoch: 100,
      });
      expect(result.active_hosts).toEqual([]);
      expect(result.cold_hosts).toEqual([]);
      expect(result.under_replicated).toBe(true);
      expect(result.actual_active).toBe(0);
      expect(result.actual_cold).toBe(0);
    });

    test('picks active hosts from active pool', async () => {
      await seedHost('alice', { node_types: ['active_host'], heartbeat_count: 20 });
      await seedHost('bob', { node_types: ['active_host'], heartbeat_count: 20 });
      await seedHost('carol', { node_types: ['active_host'], heartbeat_count: 20 });

      const result = selector.selectHosts({
        target_active: 3,
        target_cold: 0,
        current_epoch: 100,
      });
      expect(result.actual_active).toBe(3);
      expect(result.under_replicated).toBe(false);
      expect(result.active_hosts.sort()).toEqual(['alice', 'bob', 'carol']);
    });

    test('picks cold hosts from cold pool', async () => {
      await seedHost('alice', { node_types: ['cold_host'], heartbeat_count: 20 });
      await seedHost('bob', { node_types: ['cold_host'], heartbeat_count: 20 });

      const result = selector.selectHosts({
        target_active: 0,
        target_cold: 5,
        current_epoch: 100,
      });
      expect(result.actual_cold).toBe(2);
      expect(result.under_replicated).toBe(true); // 2 < 5 target
    });

    test('host with both roles can appear in either list (but only once)', async () => {
      await seedHost('alice', {
        node_types: ['active_host', 'cold_host'],
        heartbeat_count: 20,
      });
      await seedHost('bob', { node_types: ['active_host'], heartbeat_count: 20 });
      await seedHost('carol', { node_types: ['cold_host'], heartbeat_count: 20 });

      const result = selector.selectHosts({
        target_active: 2,
        target_cold: 2,
        current_epoch: 100,
      });
      // alice + bob = active, carol = cold. alice should NOT also be in cold.
      expect(result.active_hosts.length).toBe(2);
      expect(result.cold_hosts.length).toBe(1); // only carol; alice already picked
      expect(result.cold_hosts).toEqual(['carol']);
      expect(result.active_hosts).toContain('alice');
      expect(result.active_hosts).toContain('bob');
    });
  });

  describe('under-replication handling', () => {
    test('small pool returns best effort', async () => {
      await seedHost('alice', { node_types: ['active_host', 'cold_host'], heartbeat_count: 20 });

      const result = selector.selectHosts({
        target_active: 3,
        target_cold: 5,
        current_epoch: 100,
      });
      // Only alice exists; she fills one slot
      expect(result.actual_active + result.actual_cold).toBe(1);
      expect(result.under_replicated).toBe(true);
    });
  });

  describe('region constraints', () => {
    test('region_constraints filters candidates', async () => {
      await seedHost('alice', { node_types: ['cold_host'], region: 'EU', heartbeat_count: 20 });
      await seedHost('bob', { node_types: ['cold_host'], region: 'US', heartbeat_count: 20 });
      await seedHost('carol', { node_types: ['cold_host'], region: 'APAC', heartbeat_count: 20 });

      const result = selector.selectHosts({
        target_active: 0,
        target_cold: 5,
        region_constraints: ['EU'],
        current_epoch: 100,
      });
      expect(result.cold_hosts).toEqual(['alice']);
      expect(result.rejected_by_region).toBe(2);
    });

    test('multi-region constraint accepts any matching', async () => {
      await seedHost('alice', { node_types: ['cold_host'], region: 'EU', heartbeat_count: 20 });
      await seedHost('bob', { node_types: ['cold_host'], region: 'US', heartbeat_count: 20 });
      await seedHost('carol', { node_types: ['cold_host'], region: 'APAC', heartbeat_count: 20 });

      const result = selector.selectHosts({
        target_active: 0,
        target_cold: 5,
        region_constraints: ['EU', 'US'],
        current_epoch: 100,
      });
      expect(result.cold_hosts.sort()).toEqual(['alice', 'bob']);
      expect(result.rejected_by_region).toBe(1);
    });

    test('case-insensitive region matching', async () => {
      await seedHost('alice', { node_types: ['cold_host'], region: 'us', heartbeat_count: 20 });
      const result = selector.selectHosts({
        target_active: 0,
        target_cold: 5,
        region_constraints: ['US'],
        current_epoch: 100,
      });
      expect(result.cold_hosts).toEqual(['alice']);
    });
  });

  describe('scoring + ordering', () => {
    test('higher uptime wins ties', async () => {
      await seedHost('alice', { node_types: ['cold_host'], heartbeat_count: 20 }); // full uptime
      await seedHost('bob', { node_types: ['cold_host'], heartbeat_count: 10 });   // half uptime
      await seedHost('carol', { node_types: ['cold_host'], heartbeat_count: 5 });  // quarter

      const result = selector.selectHosts({
        target_active: 0,
        target_cold: 2,
        current_epoch: 100,
      });
      expect(result.cold_hosts).toEqual(['alice', 'bob']);
    });

    test('higher capacity wins ties with equal uptime', async () => {
      await seedHost('alice', { node_types: ['cold_host'], heartbeat_count: 20, capacity_gb: 100 });
      await seedHost('bob', { node_types: ['cold_host'], heartbeat_count: 20, capacity_gb: 10000 });
      await seedHost('carol', { node_types: ['cold_host'], heartbeat_count: 20, capacity_gb: 1 });

      const result = selector.selectHosts({
        target_active: 0,
        target_cold: 2,
        current_epoch: 100,
      });
      expect(result.cold_hosts).toContain('bob');
    });
  });

  describe('findRebalanceOpportunities', () => {
    test('empty when no commits', () => {
      const opps = selector.findRebalanceOpportunities({ current_epoch: 100 });
      expect(opps).toEqual([]);
    });

    test('finds under-replicated commits and suggests additions', async () => {
      // Seed two hosts
      await seedHost('alice', { node_types: ['cold_host'], heartbeat_count: 20 });
      await seedHost('bob', { node_types: ['cold_host'], heartbeat_count: 20 });

      // Commit a blob with only alice (target is 5 cold, actual 1)
      await ledger.recordBlobStoreCommit(
        'uploader',
        CID_A,
        1000,
        {
          active_hosts: [],
          cold_hosts: ['alice'],
          target_active: 0,
          target_cold: 5,
          duration_epochs: 1000,
          payment_hone: 0,
        },
        1
      );

      const opps = selector.findRebalanceOpportunities({ current_epoch: 100 });
      expect(opps.length).toBe(1);
      expect(opps[0].cid).toBe(CID_A);
      // Bob should be suggested (alice already committed)
      expect(opps[0].add_cold).toContain('bob');
      expect(opps[0].add_cold).not.toContain('alice');
    });

    test('ignores fully replicated commits', async () => {
      await seedHost('alice', { node_types: ['cold_host'], heartbeat_count: 20 });
      await ledger.recordBlobStoreCommit(
        'uploader',
        CID_A,
        1000,
        {
          cold_hosts: ['alice'],
          target_cold: 1,
          duration_epochs: 1000,
        },
        1
      );

      const opps = selector.findRebalanceOpportunities({ current_epoch: 100 });
      expect(opps).toEqual([]);
    });
  });

  describe('v2.11.2 ledger signature', () => {
    test('new recordBlobStoreCommit options signature works', async () => {
      await ledger.recordBlobStoreCommit(
        'uploader',
        CID_A,
        1000,
        {
          active_hosts: ['alice', 'bob'],
          cold_hosts: ['carol', 'dave'],
          duration_epochs: 100,
          payment_hone: 50,
          region_constraints: ['US'],
          target_active: 3,
          target_cold: 5,
        },
        1
      );
      const blob = stateStore.getBlobCommit(CID_A);
      expect(blob.active_hosts.sort()).toEqual(['alice', 'bob']);
      expect(blob.cold_hosts.sort()).toEqual(['carol', 'dave']);
      expect(blob.region_constraints).toEqual(['US']);
      expect(blob.target_active).toBe(3);
      expect(blob.target_cold).toBe(5);
      expect(blob.under_replicated).toBe(true); // 2 < 3 active, 2 < 5 cold
    });

    test('legacy v2.11.0 positional signature still works', async () => {
      await ledger.recordBlobStoreCommit(
        'uploader',
        CID_B,
        1000,
        ['alice', 'bob'],
        100,
        50,
        1
      );
      const blob = stateStore.getBlobCommit(CID_B);
      expect(blob.hosts.sort()).toEqual(['alice', 'bob']);
      expect(blob.uploader).toBe('uploader');
    });
  });
});
