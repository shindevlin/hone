"use strict";

/**
 * Service registry tests — v2.13-alpha
 *
 * Tests the standalone in-memory primitive for stateless compute hosting.
 * Same pay-for-delivery semantics as storage: deploy, host, session,
 * end — no slashing for absence anywhere in this module.
 */

const registry = require('../src/services/serviceRegistry');

const VALID_CID = 'a'.repeat(64);
const VALID_CID_2 = 'b'.repeat(64);

function basicRuntime(opts) {
  opts = opts || {};
  return {
    type: opts.type || 'http',
    binary_cid: opts.binary_cid || VALID_CID,
    cpu: opts.cpu || 1,
    ram_mb: opts.ram_mb || 512,
    port: opts.port || 8080,
    entrypoint: opts.entrypoint || './server',
  };
}

describe('serviceRegistry (v2.13-alpha)', () => {
  beforeEach(() => {
    registry.resetForTests();
  });

  describe('parseSlug', () => {
    test('valid slug parses correctly', () => {
      const r = registry.parseSlug('shindevlin/voxel-wars');
      expect(r.ok).toBe(true);
      expect(r.deployer).toBe('shindevlin');
      expect(r.name).toBe('voxel-wars');
    });

    test('missing slash → invalid', () => {
      expect(registry.parseSlug('voxel-wars').ok).toBe(false);
    });

    test('multiple slashes → invalid', () => {
      expect(registry.parseSlug('a/b/c').ok).toBe(false);
    });

    test('uppercase deployer → invalid', () => {
      expect(registry.parseSlug('SHINDEVLIN/widget').ok).toBe(false);
    });

    test('uppercase name → invalid', () => {
      expect(registry.parseSlug('alice/HACK').ok).toBe(false);
    });

    test('non-string → invalid', () => {
      expect(registry.parseSlug(null).ok).toBe(false);
      expect(registry.parseSlug(123).ok).toBe(false);
    });

    test('leading dash in name → invalid', () => {
      expect(registry.parseSlug('alice/-bad').ok).toBe(false);
    });

    test('long but valid name → ok', () => {
      const longName = 'a'.repeat(63);
      expect(registry.parseSlug('alice/' + longName).ok).toBe(true);
    });

    test('too-long name → invalid', () => {
      const tooLong = 'a'.repeat(64);
      expect(registry.parseSlug('alice/' + tooLong).ok).toBe(false);
    });
  });

  describe('deploy', () => {
    test('deploys a new service', () => {
      const svc = registry.deploy('alice', 'alice/widget', basicRuntime(), {
        epoch: 100,
        price_per_hour: 0.01,
      });
      expect(svc.slug).toBe('alice/widget');
      expect(svc.deployer).toBe('alice');
      expect(svc.binary_cid).toBe(VALID_CID);
      expect(svc.runtime.type).toBe('http');
      expect(svc.status).toBe('active');
      expect(svc.deployed_epoch).toBe(100);
      expect(svc.price_per_hour).toBe(0.01);
    });

    test('rejects deployer mismatch with slug prefix', () => {
      expect(() =>
        registry.deploy('mallory', 'alice/widget', basicRuntime())
      ).toThrow('slug deployer prefix');
    });

    test('rejects invalid slug format', () => {
      expect(() => registry.deploy('alice', 'no-slash', basicRuntime())).toThrow('invalid slug');
    });

    test('rejects missing runtime', () => {
      expect(() => registry.deploy('alice', 'alice/widget', null)).toThrow('runtime spec required');
    });

    test('rejects invalid CID', () => {
      const badRuntime = basicRuntime({ binary_cid: 'not-hex' });
      expect(() => registry.deploy('alice', 'alice/widget', badRuntime)).toThrow('64-char hex');
    });

    test('rejects unknown runtime type', () => {
      const badRuntime = basicRuntime({ type: 'mainframe' });
      expect(() => registry.deploy('alice', 'alice/widget', badRuntime)).toThrow('runtime.type');
    });

    test('valid runtime types', () => {
      ['http', 'tcp', 'wasm', 'static'].forEach((type) => {
        registry.resetForTests();
        const svc = registry.deploy('alice', 'alice/svc-' + type, basicRuntime({ type }));
        expect(svc.runtime.type).toBe(type);
      });
    });

    test('updating existing service preserves deployer', () => {
      registry.deploy('alice', 'alice/widget', basicRuntime(), { epoch: 1 });
      const updated = registry.deploy('alice', 'alice/widget',
        basicRuntime({ binary_cid: VALID_CID_2 }), { epoch: 5 });
      expect(updated.deployer).toBe('alice');
      expect(updated.binary_cid).toBe(VALID_CID_2);
      expect(updated.last_updated_epoch).toBe(5);
      expect(updated.deployed_epoch).toBe(1); // original deploy epoch preserved
    });

    test('non-original deployer cannot update', () => {
      registry.deploy('alice', 'alice/widget', basicRuntime());
      // mallory can't even pass slug validation
      expect(() => registry.deploy('mallory', 'alice/widget', basicRuntime())).toThrow();
    });
  });

  describe('retire', () => {
    beforeEach(() => {
      registry.deploy('alice', 'alice/widget', basicRuntime(), { epoch: 1 });
    });

    test('deployer can retire their service', () => {
      const r = registry.retire('alice', 'alice/widget', 50);
      expect(r.status).toBe('retired');
      expect(r.retired_epoch).toBe(50);
    });

    test('non-deployer cannot retire', () => {
      expect(() => registry.retire('mallory', 'alice/widget', 50)).toThrow('only the deployer');
    });

    test('retiring twice is idempotent', () => {
      registry.retire('alice', 'alice/widget', 50);
      const r = registry.retire('alice', 'alice/widget', 60);
      expect(r.status).toBe('retired');
    });

    test('retire unknown service throws', () => {
      expect(() => registry.retire('alice', 'alice/nothing', 50)).toThrow('not found');
    });
  });

  describe('heartbeat', () => {
    beforeEach(() => {
      registry.deploy('alice', 'alice/widget', basicRuntime(), { epoch: 1 });
    });

    test('host heartbeat adds host to active list', () => {
      const hb = registry.heartbeat('miner-a', 'alice/widget', { epoch: 5, port_addr: 'miner-a:8080' });
      expect(hb.host).toBe('miner-a');
      expect(hb.last_heartbeat_epoch).toBe(5);
      expect(hb.port_addr).toBe('miner-a:8080');

      const svc = registry.getService('alice/widget');
      expect(svc.active_hosts).toContain('miner-a');
    });

    test('multiple heartbeats from same host accumulate', () => {
      registry.heartbeat('miner-a', 'alice/widget', { epoch: 5 });
      registry.heartbeat('miner-a', 'alice/widget', { epoch: 10 });
      registry.heartbeat('miner-a', 'alice/widget', { epoch: 15 });
      const hb = registry.getHeartbeat('alice/widget', 'miner-a');
      expect(hb.total_heartbeats).toBe(3);
      expect(hb.last_heartbeat_epoch).toBe(15);
    });

    test('multiple hosts are tracked independently', () => {
      registry.heartbeat('miner-a', 'alice/widget', { epoch: 5 });
      registry.heartbeat('miner-b', 'alice/widget', { epoch: 5 });
      registry.heartbeat('miner-c', 'alice/widget', { epoch: 5 });
      const svc = registry.getService('alice/widget');
      expect(svc.active_hosts.sort()).toEqual(['miner-a', 'miner-b', 'miner-c']);
    });

    test('heartbeat for unknown service throws', () => {
      expect(() => registry.heartbeat('miner-a', 'alice/nothing', { epoch: 5 })).toThrow('not found');
    });

    test('heartbeat for retired service throws', () => {
      registry.retire('alice', 'alice/widget', 50);
      expect(() => registry.heartbeat('miner-a', 'alice/widget', { epoch: 60 })).toThrow('not active');
    });

    test('host added only once even with many heartbeats', () => {
      for (let i = 0; i < 10; i++) {
        registry.heartbeat('miner-a', 'alice/widget', { epoch: i });
      }
      const svc = registry.getService('alice/widget');
      expect(svc.active_hosts.filter((h) => h === 'miner-a').length).toBe(1);
    });
  });

  describe('startSession', () => {
    beforeEach(() => {
      registry.deploy('alice', 'alice/widget', basicRuntime(), { epoch: 1 });
      registry.heartbeat('miner-a', 'alice/widget', { epoch: 5 });
    });

    test('user can start a session against an active host', () => {
      const session = registry.startSession('bob', 'miner-a', 'alice/widget', 'sess-1', {
        epoch: 10,
        max_duration_epochs: 50,
        escrow_id: 'esc-1',
      });
      expect(session.session_id).toBe('sess-1');
      expect(session.user).toBe('bob');
      expect(session.host).toBe('miner-a');
      expect(session.status).toBe('active');
      expect(session.escrow_id).toBe('esc-1');
    });

    test('total_sessions counter increments', () => {
      registry.startSession('bob', 'miner-a', 'alice/widget', 'sess-1', { epoch: 10 });
      registry.startSession('carol', 'miner-a', 'alice/widget', 'sess-2', { epoch: 11 });
      const svc = registry.getService('alice/widget');
      expect(svc.total_sessions).toBe(2);
    });

    test('cannot session against non-active host', () => {
      expect(() =>
        registry.startSession('bob', 'unknown-miner', 'alice/widget', 'sess-1', { epoch: 10 })
      ).toThrow('host is not active');
    });

    test('cannot session against retired service', () => {
      registry.retire('alice', 'alice/widget', 50);
      expect(() =>
        registry.startSession('bob', 'miner-a', 'alice/widget', 'sess-1', { epoch: 60 })
      ).toThrow('not active');
    });

    test('duplicate session id rejected', () => {
      registry.startSession('bob', 'miner-a', 'alice/widget', 'sess-1', { epoch: 10 });
      expect(() =>
        registry.startSession('carol', 'miner-a', 'alice/widget', 'sess-1', { epoch: 11 })
      ).toThrow('already in use');
    });
  });

  describe('endSession', () => {
    beforeEach(() => {
      registry.deploy('alice', 'alice/widget', basicRuntime(), { epoch: 1 });
      registry.heartbeat('miner-a', 'alice/widget', { epoch: 5 });
      registry.startSession('bob', 'miner-a', 'alice/widget', 'sess-1', { epoch: 10 });
    });

    test('user can end their own session', () => {
      const ended = registry.endSession('bob', 'sess-1', 25);
      expect(ended.status).toBe('ended');
      expect(ended.ended_epoch).toBe(25);
      expect(ended.actual_duration_epochs).toBe(15);
      expect(ended.ended_by).toBe('bob');
    });

    test('host can end the session', () => {
      const ended = registry.endSession('miner-a', 'sess-1', 25);
      expect(ended.status).toBe('ended');
      expect(ended.ended_by).toBe('miner-a');
    });

    test('third party cannot end the session', () => {
      expect(() => registry.endSession('mallory', 'sess-1', 25)).toThrow('only the user or host');
    });

    test('ending unknown session throws', () => {
      expect(() => registry.endSession('bob', 'no-such-session', 25)).toThrow('not found');
    });

    test('ending an already-ended session is idempotent', () => {
      registry.endSession('bob', 'sess-1', 25);
      const ended = registry.endSession('bob', 'sess-1', 30);
      expect(ended.status).toBe('ended');
      expect(ended.ended_epoch).toBe(25); // unchanged
    });
  });

  describe('readers', () => {
    beforeEach(() => {
      registry.deploy('alice', 'alice/widget', basicRuntime(), { epoch: 1 });
      registry.deploy('alice', 'alice/gizmo', basicRuntime({ type: 'tcp' }), { epoch: 1 });
      registry.deploy('bob', 'bob/api', basicRuntime(), { epoch: 1 });
      registry.heartbeat('miner-a', 'alice/widget', { epoch: 5 });
      registry.heartbeat('miner-b', 'alice/widget', { epoch: 5 });
      registry.heartbeat('miner-a', 'bob/api', { epoch: 5 });
    });

    test('getService returns the right one', () => {
      expect(registry.getService('alice/widget').deployer).toBe('alice');
      expect(registry.getService('alice/nope')).toBeNull();
    });

    test('getAllServices returns all', () => {
      const all = registry.getAllServices();
      expect(all.length).toBe(3);
    });

    test('getAllServices filters by status', () => {
      registry.retire('alice', 'alice/gizmo', 50);
      const active = registry.getAllServices({ status: 'active' });
      expect(active.length).toBe(2);
      const retired = registry.getAllServices({ status: 'retired' });
      expect(retired.length).toBe(1);
    });

    test('getAllServices filters by deployer', () => {
      const aliceServices = registry.getAllServices({ deployer: 'alice' });
      expect(aliceServices.length).toBe(2);
    });

    test('getAllServices filters by runtime type', () => {
      const tcpServices = registry.getAllServices({ type: 'tcp' });
      expect(tcpServices.length).toBe(1);
      expect(tcpServices[0].slug).toBe('alice/gizmo');
    });

    test('getServicesByDeployer returns deployer\'s services', () => {
      const bob = registry.getServicesByDeployer('bob');
      expect(bob.length).toBe(1);
      expect(bob[0].slug).toBe('bob/api');
    });

    test('getActiveHostsForService respects window', () => {
      const recent = registry.getActiveHostsForService('alice/widget', 100, 200);
      expect(recent.length).toBe(2);
      const stale = registry.getActiveHostsForService('alice/widget', 1000, 50);
      expect(stale.length).toBe(0);
    });

    test('getSessionsForUser', () => {
      registry.startSession('bob', 'miner-a', 'alice/widget', 'sess-1', { epoch: 10 });
      registry.startSession('bob', 'miner-a', 'bob/api', 'sess-2', { epoch: 11 });
      const bobSessions = registry.getSessionsForUser('bob');
      expect(bobSessions.length).toBe(2);
    });

    test('getSessionsForHost', () => {
      registry.startSession('bob', 'miner-a', 'alice/widget', 'sess-1', { epoch: 10 });
      registry.startSession('carol', 'miner-b', 'alice/widget', 'sess-2', { epoch: 11 });
      const minerASessions = registry.getSessionsForHost('miner-a');
      expect(minerASessions.length).toBe(1);
      expect(minerASessions[0].user).toBe('bob');
    });

    test('getActiveSessions only returns currently active', () => {
      registry.startSession('bob', 'miner-a', 'alice/widget', 'sess-1', { epoch: 10 });
      registry.startSession('carol', 'miner-a', 'alice/widget', 'sess-2', { epoch: 11 });
      registry.endSession('bob', 'sess-1', 25);
      const active = registry.getActiveSessions('alice/widget');
      expect(active.length).toBe(1);
      expect(active[0].user).toBe('carol');
    });
  });

  describe('end-to-end scenario', () => {
    test('full deploy → host → session → end flow', () => {
      // Deployer publishes service
      registry.deploy('shindevlin', 'shindevlin/voxel-wars',
        basicRuntime({ type: 'tcp', port: 9090, ram_mb: 2048, cpu: 2 }),
        { epoch: 1, price_per_hour: 0.05, min_replicas: 3 });

      // Three hosts pick it up and heartbeat
      registry.heartbeat('miner-a', 'shindevlin/voxel-wars', { epoch: 5, port_addr: 'a.host:9090' });
      registry.heartbeat('miner-b', 'shindevlin/voxel-wars', { epoch: 5, port_addr: 'b.host:9090' });
      registry.heartbeat('miner-c', 'shindevlin/voxel-wars', { epoch: 5, port_addr: 'c.host:9090' });

      // Player starts a session
      const session = registry.startSession('player1', 'miner-a', 'shindevlin/voxel-wars', 'play-1', {
        epoch: 10,
        max_duration_epochs: 100,
        escrow_id: 'esc-play-1',
      });
      expect(session.status).toBe('active');

      // Player plays for 30 epochs, then ends
      const ended = registry.endSession('player1', 'play-1', 40);
      expect(ended.actual_duration_epochs).toBe(30);
      expect(ended.status).toBe('ended');

      // Service still active for next player
      const svc = registry.getService('shindevlin/voxel-wars');
      expect(svc.status).toBe('active');
      expect(svc.total_sessions).toBe(1);
      expect(svc.active_hosts.length).toBe(3);
    });
  });
});
