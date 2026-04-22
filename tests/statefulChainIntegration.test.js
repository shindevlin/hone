"use strict";

/**
 * Stateful Compute Chain Integration tests — v2.14-beta
 *
 * Verifies the full path: ledger recordX → stateStore.applyEntry dispatcher
 * → getSnapshots/getLatestSnapshot reads.
 * Also covers the HTTP routes for snapshot management.
 */

// ─────────────────────────────────────────────────────────────────
// Mock setup
// ─────────────────────────────────────────────────────────────────

// Bypass JWT
jest.mock('../src/middlewares/auth', () => ({
  authenticateToken: (req, res, next) => {
    const username = req.headers['x-test-user'];
    if (!username) return res.status(401).json({ error: 'unauthenticated' });
    req.user = { username, id: 'test-id-' + username };
    next();
  },
  limiter: (req, res, next) => next(),
}));

// Stub mempool
jest.mock('../src/p2p/mempool', () => ({
  submit: jest.fn(),
  addTransaction: jest.fn().mockReturnValue(true),
}));

// Stub P2P / network gossip so ledger._gossipEntry doesn't blow up
jest.mock('../src/p2p/network', () => ({
  NODE_ID: 'test-node',
  broadcast: jest.fn(),
}));
jest.mock('../src/p2p/protocol', () => ({
  createMessage: jest.fn().mockReturnValue({}),
}));

const http = require('http');
const express = require('express');
const stateStore = require('../src/chain/stateStore');
const epochBandwidth = require('../src/services/epochBandwidth');
const ledger = require('../src/services/ledger');
const serviceRegistry = require('../src/services/serviceRegistry');
const serviceRoutes = require('../src/routes/serviceRoutes');

const CID_A = 'a'.repeat(64);
const CID_B = 'b'.repeat(64);
const CID_C = 'c'.repeat(64);

// ─────────────────────────────────────────────────────────────────
// HTTP test helpers
// ─────────────────────────────────────────────────────────────────

function makeTestServer() {
  const app = express();
  app.use(express.json());
  app.use('/api/services', serviceRoutes);
  const server = http.createServer(app);
  return new Promise((resolve) => {
    server.listen(0, '127.0.0.1', () => {
      const port = server.address().port;
      resolve({ server, port });
    });
  });
}

function closeServer(server) {
  return new Promise((resolve) => server.close(resolve));
}

function request(port, method, path, body, user) {
  return new Promise((resolve, reject) => {
    const headers = { 'Content-Type': 'application/json' };
    if (user) headers['x-test-user'] = user;
    const data = body ? JSON.stringify(body) : null;
    if (data) headers['Content-Length'] = Buffer.byteLength(data);

    const req = http.request(
      { host: '127.0.0.1', port, method, path, headers },
      (res) => {
        let raw = '';
        res.on('data', (chunk) => (raw += chunk));
        res.on('end', () => {
          let parsed = null;
          try { parsed = raw ? JSON.parse(raw) : null; } catch (_) { parsed = raw; }
          resolve({ status: res.statusCode, body: parsed });
        });
      }
    );
    req.on('error', reject);
    if (data) req.write(data);
    req.end();
  });
}

function basicRuntime() {
  return {
    type: 'http',
    binary_cid: CID_A,
    cpu: 1,
    ram_mb: 512,
    port: 8080,
    entrypoint: './server',
  };
}

// ─────────────────────────────────────────────────────────────────
// Setup / teardown
// ─────────────────────────────────────────────────────────────────

let testServer;
let port;

beforeAll(async () => {
  testServer = await makeTestServer();
  port = testServer.port;
});

afterAll(async () => {
  if (testServer) await closeServer(testServer.server);
});

beforeEach(() => {
  stateStore.resetAll();
  epochBandwidth.resetAll();
  ['shindevlin', 'alice', 'outsider', 'host-1', 'shindevlin/voxel-wars'].forEach(a => epochBandwidth.seedForTest(a));
  serviceRegistry.resetForTests();
});

// ─────────────────────────────────────────────────────────────────
// 1. recordStatefulServiceDeploy — ledger + stateStore
// ─────────────────────────────────────────────────────────────────

describe('recordStatefulServiceDeploy', () => {
  it('persists a SERVICE_DEPLOY_STATEFUL entry to stateStore', async () => {
    const entry = await ledger.recordStatefulServiceDeploy(
      'shindevlin',
      'shindevlin/voxel-wars',
      basicRuntime(),
      { snapshot_interval_epochs: 10, replication_factor: 2 },
      5
    );

    expect(entry.type).toBe('SERVICE_DEPLOY_STATEFUL');
    expect(entry.from).toBe('shindevlin');
    expect(entry.epoch).toBe(5);
    expect(entry.service_data.slug).toBe('shindevlin/voxel-wars');
    expect(entry.service_data.stateful).toBe(true);
    expect(entry.service_data.snapshot_interval_epochs).toBe(10);
    expect(entry.service_data.replication_factor).toBe(2);
  });

  it('stores service record in stateStore with stateful metadata', async () => {
    await ledger.recordStatefulServiceDeploy(
      'shindevlin',
      'shindevlin/voxel-wars',
      basicRuntime(),
      { snapshot_interval_epochs: 20, replication_factor: 3 },
      7
    );

    const rec = stateStore.getStatefulServiceRecord('shindevlin/voxel-wars');
    expect(rec).not.toBeNull();
    expect(rec.slug).toBe('shindevlin/voxel-wars');
    expect(rec.deployer).toBe('shindevlin');
    expect(rec.stateful).toBe(true);
    expect(rec.snapshot_interval_epochs).toBe(20);
    expect(rec.replication_factor).toBe(3);
    expect(rec.status).toBe('active');
  });

  it('defaults replication_factor to 3 when not specified', async () => {
    await ledger.recordStatefulServiceDeploy(
      'shindevlin',
      'shindevlin/voxel-wars',
      basicRuntime(),
      {},
      1
    );

    const rec = stateStore.getStatefulServiceRecord('shindevlin/voxel-wars');
    expect(rec.replication_factor).toBe(3);
  });

  it('throws if deployer is missing', async () => {
    await expect(
      ledger.recordStatefulServiceDeploy('', 'alice/svc', basicRuntime(), {}, 1)
    ).rejects.toThrow('deployer required');
  });

  it('throws if slug is missing', async () => {
    await expect(
      ledger.recordStatefulServiceDeploy('alice', '', basicRuntime(), {}, 1)
    ).rejects.toThrow('slug required');
  });

  it('throws if runtimeSpec is missing', async () => {
    await expect(
      ledger.recordStatefulServiceDeploy('alice', 'alice/svc', null, {}, 1)
    ).rejects.toThrow('runtimeSpec required');
  });
});

// ─────────────────────────────────────────────────────────────────
// 2. recordSnapshotCommit — ledger + stateStore
// ─────────────────────────────────────────────────────────────────

describe('recordSnapshotCommit', () => {
  beforeEach(async () => {
    // Register service in stateStore first so SNAPSHOT_COMMIT can update it
    await ledger.recordStatefulServiceDeploy(
      'shindevlin', 'shindevlin/voxel-wars', basicRuntime(), {}, 1
    );
  });

  it('returns a SNAPSHOT_COMMIT entry', async () => {
    const entry = await ledger.recordSnapshotCommit(
      'shindevlin/voxel-wars', CID_A, ['host-1', 'host-2'], 10
    );
    expect(entry.type).toBe('SNAPSHOT_COMMIT');
    expect(entry.snapshot_data.slug).toBe('shindevlin/voxel-wars');
    expect(entry.snapshot_data.cid).toBe(CID_A);
    expect(entry.snapshot_data.replica_hosts).toEqual(['host-1', 'host-2']);
    expect(entry.epoch).toBe(10);
  });

  it('getSnapshots returns the committed snapshot', async () => {
    await ledger.recordSnapshotCommit(
      'shindevlin/voxel-wars', CID_A, ['host-1'], 10
    );

    const snaps = stateStore.getSnapshots('shindevlin/voxel-wars');
    expect(snaps).toHaveLength(1);
    expect(snaps[0].cid).toBe(CID_A);
    expect(snaps[0].epoch).toBe(10);
    expect(snaps[0].replicas).toEqual(['host-1']);
  });

  it('getSnapshots returns history in insertion order', async () => {
    await ledger.recordSnapshotCommit('shindevlin/voxel-wars', CID_A, [], 10);
    await ledger.recordSnapshotCommit('shindevlin/voxel-wars', CID_B, [], 20);
    await ledger.recordSnapshotCommit('shindevlin/voxel-wars', CID_C, [], 30);

    const snaps = stateStore.getSnapshots('shindevlin/voxel-wars');
    expect(snaps).toHaveLength(3);
    expect(snaps[0].cid).toBe(CID_A);
    expect(snaps[1].cid).toBe(CID_B);
    expect(snaps[2].cid).toBe(CID_C);
    expect(snaps[0].epoch).toBeLessThan(snaps[1].epoch);
    expect(snaps[1].epoch).toBeLessThan(snaps[2].epoch);
  });

  it('getLatestSnapshot returns the most recent snapshot', async () => {
    await ledger.recordSnapshotCommit('shindevlin/voxel-wars', CID_A, [], 10);
    await ledger.recordSnapshotCommit('shindevlin/voxel-wars', CID_B, [], 20);

    const latest = stateStore.getLatestSnapshot('shindevlin/voxel-wars');
    expect(latest).not.toBeNull();
    expect(latest.cid).toBe(CID_B);
    expect(latest.epoch).toBe(20);
  });

  it('getLatestSnapshot returns null when no snapshots exist', () => {
    const latest = stateStore.getLatestSnapshot('shindevlin/no-snaps');
    expect(latest).toBeNull();
  });

  it('updates service record last_snapshot_cid on commit', async () => {
    await ledger.recordSnapshotCommit('shindevlin/voxel-wars', CID_B, [], 15);

    const rec = stateStore.getStatefulServiceRecord('shindevlin/voxel-wars');
    expect(rec.last_snapshot_cid).toBe(CID_B);
    expect(rec.last_snapshot_epoch).toBe(15);
  });

  it('throws if slug is missing', async () => {
    await expect(
      ledger.recordSnapshotCommit('', CID_A, [], 10)
    ).rejects.toThrow('slug required');
  });

  it('throws if cid is invalid', async () => {
    await expect(
      ledger.recordSnapshotCommit('shindevlin/voxel-wars', 'not-a-cid', [], 10)
    ).rejects.toThrow('cid must be 64-char hex sha256');
  });
});

// ─────────────────────────────────────────────────────────────────
// 3. recordSnapshotRestore — ledger + stateStore
// ─────────────────────────────────────────────────────────────────

describe('recordSnapshotRestore', () => {
  beforeEach(async () => {
    await ledger.recordStatefulServiceDeploy(
      'shindevlin', 'shindevlin/voxel-wars', basicRuntime(), {}, 1
    );
    await ledger.recordSnapshotCommit('shindevlin/voxel-wars', CID_A, [], 10);
  });

  it('returns a SNAPSHOT_RESTORE entry', async () => {
    const entry = await ledger.recordSnapshotRestore(
      'shindevlin/voxel-wars', CID_A, 'host-1', 12
    );
    expect(entry.type).toBe('SNAPSHOT_RESTORE');
    expect(entry.snapshot_data.slug).toBe('shindevlin/voxel-wars');
    expect(entry.snapshot_data.cid).toBe(CID_A);
    expect(entry.snapshot_data.host).toBe('host-1');
    expect(entry.epoch).toBe(12);
  });

  it('logs restore event on the service record', async () => {
    await ledger.recordSnapshotRestore('shindevlin/voxel-wars', CID_A, 'host-1', 12);

    const rec = stateStore.getStatefulServiceRecord('shindevlin/voxel-wars');
    expect(rec.last_restore_cid).toBe(CID_A);
    expect(rec.last_restore_epoch).toBe(12);
    expect(rec.restore_history).toHaveLength(1);
    expect(rec.restore_history[0].cid).toBe(CID_A);
    expect(rec.restore_history[0].host).toBe('host-1');
  });

  it('throws if slug is missing', async () => {
    await expect(
      ledger.recordSnapshotRestore('', CID_A, 'host-1', 10)
    ).rejects.toThrow('slug required');
  });
});

// ─────────────────────────────────────────────────────────────────
// 4. HTTP routes
// ─────────────────────────────────────────────────────────────────

describe('GET /api/services/:slug/snapshots', () => {
  beforeEach(async () => {
    stateStore.resetAll();
    epochBandwidth.resetAll();
    ['shindevlin', 'alice', 'outsider', 'host-1', 'shindevlin/voxel-wars'].forEach(a => epochBandwidth.seedForTest(a));
    serviceRegistry.resetForTests();
    // Deploy the service in both registries (route checks serviceRegistry)
    serviceRegistry.deploy('shindevlin', 'shindevlin/voxel-wars', basicRuntime(), { epoch: 1 });
    await ledger.recordStatefulServiceDeploy(
      'shindevlin', 'shindevlin/voxel-wars', basicRuntime(), {}, 1
    );
  });

  it('returns 404 for unknown service', async () => {
    const res = await request(port, 'GET', '/api/services/shindevlin%2Funknown/snapshots');
    expect(res.status).toBe(404);
  });

  it('returns empty list when no snapshots', async () => {
    const res = await request(port, 'GET', '/api/services/shindevlin%2Fvoxel-wars/snapshots');
    expect(res.status).toBe(200);
    expect(res.body.snapshots).toEqual([]);
    expect(res.body.count).toBe(0);
  });

  it('returns snapshot list after commits', async () => {
    await ledger.recordSnapshotCommit('shindevlin/voxel-wars', CID_A, ['host-1'], 10);
    await ledger.recordSnapshotCommit('shindevlin/voxel-wars', CID_B, ['host-2'], 20);

    const res = await request(port, 'GET', '/api/services/shindevlin%2Fvoxel-wars/snapshots');
    expect(res.status).toBe(200);
    expect(res.body.count).toBe(2);
    expect(res.body.snapshots[0].cid).toBe(CID_A);
    expect(res.body.snapshots[1].cid).toBe(CID_B);
  });
});

describe('GET /api/services/:slug/snapshots/latest', () => {
  beforeEach(async () => {
    stateStore.resetAll();
    epochBandwidth.resetAll();
    ['shindevlin', 'alice', 'outsider', 'host-1', 'shindevlin/voxel-wars'].forEach(a => epochBandwidth.seedForTest(a));
    serviceRegistry.resetForTests();
    serviceRegistry.deploy('shindevlin', 'shindevlin/voxel-wars', basicRuntime(), { epoch: 1 });
    await ledger.recordStatefulServiceDeploy(
      'shindevlin', 'shindevlin/voxel-wars', basicRuntime(), {}, 1
    );
  });

  it('returns 404 when no snapshots exist', async () => {
    const res = await request(port, 'GET', '/api/services/shindevlin%2Fvoxel-wars/snapshots/latest');
    expect(res.status).toBe(404);
  });

  it('returns the latest snapshot', async () => {
    await ledger.recordSnapshotCommit('shindevlin/voxel-wars', CID_A, [], 10);
    await ledger.recordSnapshotCommit('shindevlin/voxel-wars', CID_B, [], 20);

    const res = await request(port, 'GET', '/api/services/shindevlin%2Fvoxel-wars/snapshots/latest');
    expect(res.status).toBe(200);
    expect(res.body.snapshot.cid).toBe(CID_B);
    expect(res.body.snapshot.epoch).toBe(20);
  });
});

describe('POST /api/services/:slug/snapshots', () => {
  beforeEach(async () => {
    stateStore.resetAll();
    epochBandwidth.resetAll();
    ['shindevlin', 'alice', 'outsider', 'host-1', 'shindevlin/voxel-wars'].forEach(a => epochBandwidth.seedForTest(a));
    serviceRegistry.resetForTests();
    serviceRegistry.deploy('shindevlin', 'shindevlin/voxel-wars', basicRuntime(), { epoch: 1 });
    await ledger.recordStatefulServiceDeploy(
      'shindevlin', 'shindevlin/voxel-wars', basicRuntime(), {}, 1
    );
  });

  it('rejects unauthenticated request', async () => {
    const res = await request(port, 'POST', '/api/services/shindevlin%2Fvoxel-wars/snapshots',
      { cid: CID_A });
    expect(res.status).toBe(401);
  });

  it('rejects invalid cid', async () => {
    const res = await request(port, 'POST', '/api/services/shindevlin%2Fvoxel-wars/snapshots',
      { cid: 'bad-cid' }, 'shindevlin');
    expect(res.status).toBe(400);
    expect(res.body.error).toMatch(/cid/i);
  });

  it('rejects non-deployer non-host', async () => {
    const res = await request(port, 'POST', '/api/services/shindevlin%2Fvoxel-wars/snapshots',
      { cid: CID_A }, 'outsider');
    expect(res.status).toBe(403);
  });

  it('deployer can commit a snapshot', async () => {
    const res = await request(port, 'POST', '/api/services/shindevlin%2Fvoxel-wars/snapshots',
      { cid: CID_A, replica_hosts: ['host-1', 'host-2'] }, 'shindevlin');
    expect(res.status).toBe(200);
    expect(res.body.success).toBe(true);
    expect(res.body.entry.type).toBe('SNAPSHOT_COMMIT');

    // Verify stateStore updated
    const snap = stateStore.getLatestSnapshot('shindevlin/voxel-wars');
    expect(snap).not.toBeNull();
    expect(snap.cid).toBe(CID_A);
  });

  it('active host can commit a snapshot', async () => {
    // Add host-1 to active_hosts via heartbeat
    serviceRegistry.heartbeat('host-1', 'shindevlin/voxel-wars', { epoch: 2 });

    const res = await request(port, 'POST', '/api/services/shindevlin%2Fvoxel-wars/snapshots',
      { cid: CID_A }, 'host-1');
    expect(res.status).toBe(200);
    expect(res.body.success).toBe(true);
  });
});

describe('POST /api/services/:slug/restore/:cid', () => {
  beforeEach(async () => {
    stateStore.resetAll();
    epochBandwidth.resetAll();
    ['shindevlin', 'alice', 'outsider', 'host-1', 'shindevlin/voxel-wars'].forEach(a => epochBandwidth.seedForTest(a));
    serviceRegistry.resetForTests();
    serviceRegistry.deploy('shindevlin', 'shindevlin/voxel-wars', basicRuntime(), { epoch: 1 });
    await ledger.recordStatefulServiceDeploy(
      'shindevlin', 'shindevlin/voxel-wars', basicRuntime(), {}, 1
    );
    await ledger.recordSnapshotCommit('shindevlin/voxel-wars', CID_A, [], 10);
  });

  it('rejects unauthenticated request', async () => {
    const res = await request(port, 'POST',
      '/api/services/shindevlin%2Fvoxel-wars/restore/' + CID_A, {});
    expect(res.status).toBe(401);
  });

  it('rejects non-deployer', async () => {
    const res = await request(port, 'POST',
      '/api/services/shindevlin%2Fvoxel-wars/restore/' + CID_A, {}, 'outsider');
    expect(res.status).toBe(403);
  });

  it('deployer can restore from a snapshot', async () => {
    const res = await request(port, 'POST',
      '/api/services/shindevlin%2Fvoxel-wars/restore/' + CID_A,
      { host: 'host-1' }, 'shindevlin');
    expect(res.status).toBe(200);
    expect(res.body.success).toBe(true);
    expect(res.body.entry.type).toBe('SNAPSHOT_RESTORE');

    // Verify stateStore updated
    const rec = stateStore.getStatefulServiceRecord('shindevlin/voxel-wars');
    expect(rec.last_restore_cid).toBe(CID_A);
    expect(rec.restore_history).toHaveLength(1);
    expect(rec.restore_history[0].host).toBe('host-1');
  });

  it('rejects invalid cid in URL', async () => {
    const res = await request(port, 'POST',
      '/api/services/shindevlin%2Fvoxel-wars/restore/bad-cid', {}, 'shindevlin');
    expect(res.status).toBe(400);
    expect(res.body.error).toMatch(/cid/i);
  });
});
