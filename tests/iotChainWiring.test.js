"use strict";

/**
 * BTCPC IoT Chain Wiring Tests — v3.0-pre
 * Shin Devlin
 *
 * Tests for:
 *   - IoT sensor + gateway ledger entries (SENSOR_REGISTER, SENSOR_READING,
 *     SENSOR_DATA_COMMIT, GATEWAY_REGISTER, GATEWAY_HEARTBEAT)
 *   - stateStore dispatcher cases for all IoT + bridge entry types
 *   - IoT reward pool in computeFinalization (6th pool)
 *   - Bridge ledger entries (BRIDGE_WRAP, BRIDGE_UNWRAP, BRIDGE_FUND, BRIDGE_UNLOCK)
 *   - Sensor data → BTCPC-FS blob persistence via sensorRoutes /finalize
 *
 * Target: adds ≥ 20 tests. All existing 1,151 tests must still pass.
 */

// Mock mempool before any ledger require
const mockMempoolSubmit = jest.fn().mockReturnValue({ accepted: true });
jest.mock('../src/p2p/mempool', () => ({
  submit: (...args) => mockMempoolSubmit(...args),
  addTransaction: jest.fn().mockReturnValue(true),
}));

const stateStore = require('../src/chain/stateStore');
const ledger = require('../src/services/ledger');
const sensorRegistry = require('../src/services/sensorRegistry');
const gatewayRegistry = require('../src/services/loraGatewayRegistry');
const nanoRewards = require('../src/services/nanoRewards');

beforeEach(() => {
  stateStore.resetAll();
  sensorRegistry.resetForTests();
  gatewayRegistry.resetForTests();
  jest.clearAllMocks();
});

// ─── Sensor ledger entries ───────────────────────────────────────────────────

describe('ledger.recordSensorRegister', () => {
  it('creates a SENSOR_REGISTER entry with correct fields', () => {
    const entry = ledger.recordSensorRegister(
      'shindevlin',
      'shindevlin/temp-1',
      { type: 'temperature', unit: 'celsius', decimals: 2, region: 'us-west' },
      10
    );
    expect(entry.type).toBe('SENSOR_REGISTER');
    expect(entry.from).toBe('shindevlin');
    expect(entry.epoch).toBe(10);
    expect(entry.sensor_data.sensor_id).toBe('shindevlin/temp-1');
    expect(entry.sensor_data.type).toBe('temperature');
    expect(entry.sensor_data.unit).toBe('celsius');
    expect(entry.sensor_data.region).toBe('us-west');
  });

  it('throws if owner is missing', () => {
    expect(() => ledger.recordSensorRegister('', 'x/y', { type: 'temperature', unit: 'c', decimals: 0, region: 'r' }, 0))
      .toThrow('owner required');
  });

  it('throws if spec is missing', () => {
    expect(() => ledger.recordSensorRegister('alice', 'alice/s1', null, 0))
      .toThrow('spec required');
  });

  it('applies to stateStore — getSensor returns the record', () => {
    ledger.recordSensorRegister(
      'shindevlin',
      'shindevlin/hum-1',
      { type: 'humidity', unit: 'pct', decimals: 1, region: 'eu-central' },
      5
    );
    const s = stateStore.getSensor('shindevlin/hum-1');
    expect(s).not.toBeNull();
    expect(s.owner).toBe('shindevlin');
    expect(s.type).toBe('humidity');
    expect(s.status).toBe('active');
  });
});

describe('ledger.recordSensorReading', () => {
  it('creates a SENSOR_READING entry', () => {
    const entry = ledger.recordSensorReading(
      'shindevlin/temp-1',
      23.5,
      { latitude: 37.77, longitude: -122.4 },
      7
    );
    expect(entry.type).toBe('SENSOR_READING');
    expect(entry.sensor_data.sensor_id).toBe('shindevlin/temp-1');
    expect(entry.sensor_data.value).toBe(23.5);
    expect(entry.epoch).toBe(7);
  });

  it('throws if value is not a finite number', () => {
    expect(() => ledger.recordSensorReading('x/y', NaN, {}, 0))
      .toThrow('value must be a finite number');
  });

  it('buffers reading in stateStore for the epoch', () => {
    // Register sensor first
    ledger.recordSensorRegister('shindevlin', 'shindevlin/t1',
      { type: 'temperature', unit: 'celsius', decimals: 2, region: 'us' }, 0);
    ledger.recordSensorReading('shindevlin/t1', 21.0, {}, 3);
    ledger.recordSensorReading('shindevlin/t1', 22.0, {}, 3);

    const active = stateStore.getSensorsForEpoch(3);
    expect(active.length).toBeGreaterThan(0);
    const s = active.find(x => x.sensor_id === 'shindevlin/t1');
    expect(s).toBeDefined();
    expect(s.readings).toBe(2);
  });
});

describe('ledger.recordSensorDataCommit', () => {
  it('creates a SENSOR_DATA_COMMIT entry with blob CID', () => {
    const cid = 'a'.repeat(64);
    const entry = ledger.recordSensorDataCommit('shindevlin/t1', cid, 4, 12, 21.5);
    expect(entry.type).toBe('SENSOR_DATA_COMMIT');
    expect(entry.sensor_data.cid).toBe(cid);
    expect(entry.sensor_data.reading_count).toBe(12);
    expect(entry.sensor_data.median_value).toBe(21.5);
  });

  it('throws if CID is not 64-char hex', () => {
    expect(() => ledger.recordSensorDataCommit('x/y', 'notahex', 0, 0, 0))
      .toThrow('cid must be 64-char hex sha256');
  });

  it('applies to stateStore — sensor gets last_commit_cid', () => {
    ledger.recordSensorRegister('shindevlin', 'shindevlin/s2',
      { type: 'temperature', unit: 'celsius', decimals: 2, region: 'us' }, 0);
    const cid = 'b'.repeat(64);
    ledger.recordSensorDataCommit('shindevlin/s2', cid, 5, 10, 20.0);
    const s = stateStore.getSensor('shindevlin/s2');
    expect(s.last_commit_cid).toBe(cid);
    expect(s.last_commit_epoch).toBe(5);
  });
});

// ─── Gateway ledger entries ──────────────────────────────────────────────────

describe('ledger.recordGatewayRegister', () => {
  it('creates a GATEWAY_REGISTER entry', () => {
    const entry = ledger.recordGatewayRegister(
      'shindevlin',
      'shindevlin/nebra-1',
      { region: 'US915', latitude: 37.77, longitude: -122.4, hardware_model: 'nebra-indoor-helium' },
      1
    );
    expect(entry.type).toBe('GATEWAY_REGISTER');
    expect(entry.gateway_data.gateway_id).toBe('shindevlin/nebra-1');
    expect(entry.gateway_data.region).toBe('US915');
    expect(entry.gateway_data.latitude).toBe(37.77);
  });

  it('throws if owner is missing', () => {
    expect(() => ledger.recordGatewayRegister('', 'x/y', { region: 'US915', latitude: 0, longitude: 0 }, 0))
      .toThrow('owner required');
  });

  it('applies to stateStore — getGateway returns the record', () => {
    ledger.recordGatewayRegister('shindevlin', 'shindevlin/gw1',
      { region: 'EU868', latitude: 52.5, longitude: 13.4 }, 2);
    const gw = stateStore.getGateway('shindevlin/gw1');
    expect(gw).not.toBeNull();
    expect(gw.owner).toBe('shindevlin');
    expect(gw.region).toBe('EU868');
    expect(gw.status).toBe('active');
  });
});

describe('ledger.recordGatewayHeartbeat', () => {
  it('creates a GATEWAY_HEARTBEAT entry', () => {
    const entry = ledger.recordGatewayHeartbeat(
      'shindevlin/gw1',
      { sensors_connected: 3, packets_relayed_this_epoch: 47, uptime_seconds: 300 },
      9
    );
    expect(entry.type).toBe('GATEWAY_HEARTBEAT');
    expect(entry.gateway_data.gateway_id).toBe('shindevlin/gw1');
    expect(entry.epoch).toBe(9);
  });

  it('updates last_heartbeat_epoch on the gateway record', () => {
    ledger.recordGatewayRegister('shindevlin', 'shindevlin/gw2',
      { region: 'US915', latitude: 40.7, longitude: -74.0 }, 0);
    ledger.recordGatewayHeartbeat('shindevlin/gw2', { packets_relayed_this_epoch: 10 }, 8);
    const gw = stateStore.getGateway('shindevlin/gw2');
    expect(gw.last_heartbeat_epoch).toBe(8);
    expect(gw.total_heartbeats).toBe(1);
  });

  it('getGatewaysForEpoch returns gateway that heartbeated in that epoch', () => {
    ledger.recordGatewayRegister('shindevlin', 'shindevlin/gw3',
      { region: 'US915', latitude: 34.0, longitude: -118.2 }, 0);
    ledger.recordGatewayHeartbeat('shindevlin/gw3', { packets_relayed_this_epoch: 20 }, 15);
    const active = stateStore.getGatewaysForEpoch(15);
    expect(active.length).toBe(1);
    expect(active[0].gateway_id).toBe('shindevlin/gw3');
    expect(active[0].packets_relayed).toBeGreaterThan(0);
  });

  it('getGatewaysForEpoch returns empty for epoch with no heartbeats', () => {
    ledger.recordGatewayRegister('shindevlin', 'shindevlin/gw4',
      { region: 'US915', latitude: 34.0, longitude: -118.2 }, 0);
    expect(stateStore.getGatewaysForEpoch(99)).toHaveLength(0);
  });
});

// ─── IoT reward pool in computeFinalization ─────────────────────────────────

describe('nanoRewards.computeIoTRewards — IoT pool split', () => {
  it('returns empty array when no participants', () => {
    const result = nanoRewards.computeIoTRewards(1, 100, [], []);
    expect(result).toHaveLength(0);
  });

  it('splits 60/40 sensor/gateway when both active', () => {
    const gateways = [{ gateway_id: 'shindevlin/gw1', owner: 'shindevlin', packets_relayed: 100, uptime_pct: 1.0 }];
    const sensors  = [{ sensor_id: 'shindevlin/t1',  owner: 'shindevlin', readings: 5, uptime_pct: 1.0 }];
    const rewards = nanoRewards.computeIoTRewards(1, 100, gateways, sensors);
    const sensorReward  = rewards.filter(r => r.type === 'sensor').reduce((s, r) => s + r.amount, 0);
    const gatewayReward = rewards.filter(r => r.type === 'gateway').reduce((s, r) => s + r.amount, 0);
    // 60% to sensors, 40% to gateways
    expect(sensorReward).toBeCloseTo(60, 5);
    expect(gatewayReward).toBeCloseTo(40, 5);
  });

  it('rolls entire pool to gateways when no sensors active', () => {
    const gateways = [{ gateway_id: 'x/gw', owner: 'x', packets_relayed: 10, uptime_pct: 1.0 }];
    const rewards = nanoRewards.computeIoTRewards(2, 100, gateways, []);
    const gatewayTotal = rewards.reduce((s, r) => s + r.amount, 0);
    expect(gatewayTotal).toBeCloseTo(100, 5);
    expect(rewards.every(r => r.type === 'gateway')).toBe(true);
  });

  it('rolls entire pool to sensors when no gateways active', () => {
    const sensors = [{ sensor_id: 'x/s', owner: 'x', readings: 3, uptime_pct: 0.9 }];
    const rewards = nanoRewards.computeIoTRewards(3, 100, [], sensors);
    const sensorTotal = rewards.reduce((s, r) => s + r.amount, 0);
    expect(sensorTotal).toBeCloseTo(100, 5);
    expect(rewards.every(r => r.type === 'sensor')).toBe(true);
  });

  it('proportionally splits gateway pool among multiple gateways', () => {
    const gateways = [
      { gateway_id: 'a/gw1', owner: 'a', packets_relayed: 300, uptime_pct: 1.0 },
      { gateway_id: 'b/gw2', owner: 'b', packets_relayed: 100, uptime_pct: 1.0 },
    ];
    const rewards = nanoRewards.computeIoTRewards(5, 100, gateways, []);
    const aReward = rewards.find(r => r.account === 'a');
    const bReward = rewards.find(r => r.account === 'b');
    expect(aReward.amount).toBeCloseTo(75, 3); // 300/400 * 100
    expect(bReward.amount).toBeCloseTo(25, 3); // 100/400 * 100
  });
});

// ─── Bridge ledger entries ────────────────────────────────────────────────────

describe('ledger.recordBridgeWrap', () => {
  it('creates a BRIDGE_WRAP entry', async () => {
    stateStore.applyEntry({ type: 'ACCOUNT_CREATE', to: 'shindevlin', epoch: 0, account_data: {} });
    stateStore.applyEntry({ type: 'MINING_REWARD', to: 'shindevlin', token: 'BTCPC', amount: 1000, epoch: 0 });

    const entry = await ledger.recordBridgeWrap('shindevlin', 'base', 100, 0.05, 10);
    expect(entry.type).toBe('BRIDGE_WRAP');
    expect(entry.from).toBe('shindevlin');
    expect(entry.bridge_data.chain_id).toBe('base');
    expect(entry.bridge_data.direction).toBe('wrap');
  });

  it('debits user BTCPC on BRIDGE_WRAP', async () => {
    stateStore.applyEntry({ type: 'ACCOUNT_CREATE', to: 'shindevlin', epoch: 0, account_data: {} });
    stateStore.applyEntry({ type: 'MINING_REWARD', to: 'shindevlin', token: 'BTCPC', amount: 500, epoch: 0 });
    const before = stateStore.getBalance('shindevlin', 'BTCPC');

    await ledger.recordBridgeWrap('shindevlin', 'ethereum', 200, 0, 1);
    const after = stateStore.getBalance('shindevlin', 'BTCPC');
    expect(after).toBe(before - 200);
  });

  it('throws if amount is not positive', async () => {
    await expect(ledger.recordBridgeWrap('x', 'base', 0, 0, 0)).rejects.toThrow('amount must be positive');
  });
});

describe('ledger.recordBridgeUnwrap', () => {
  it('creates a BRIDGE_UNWRAP entry', async () => {
    const entry = await ledger.recordBridgeUnwrap('shindevlin', 'base', 50, 0.1, 11);
    expect(entry.type).toBe('BRIDGE_UNWRAP');
    expect(entry.to).toBe('shindevlin');
    expect(entry.bridge_data.direction).toBe('unwrap');
  });

  it('credits user BTCPC on BRIDGE_UNWRAP', async () => {
    stateStore.applyEntry({ type: 'ACCOUNT_CREATE', to: 'natoshisakamoto', epoch: 0, account_data: {} });
    const before = stateStore.getBalance('natoshisakamoto', 'BTCPC');
    await ledger.recordBridgeUnwrap('natoshisakamoto', 'arbitrum', 100, 0.2, 5);
    expect(stateStore.getBalance('natoshisakamoto', 'BTCPC')).toBe(before + 100);
  });
});

describe('ledger.recordBridgeFund', () => {
  it('creates a BRIDGE_FUND entry', async () => {
    stateStore.applyEntry({ type: 'ACCOUNT_CREATE', to: 'shindevlin', epoch: 0, account_data: {} });
    stateStore.applyEntry({ type: 'MINING_REWARD', to: 'shindevlin', token: 'BTCPC', amount: 10000, epoch: 0 });

    const entry = await ledger.recordBridgeFund('shindevlin', 'base', 1000, 90, 2);
    expect(entry.type).toBe('BRIDGE_FUND');
    expect(entry.bridge_data.lock_days).toBe(90);
  });

  it('debits funder on BRIDGE_FUND and records LP position', async () => {
    stateStore.applyEntry({ type: 'ACCOUNT_CREATE', to: 'lp-user', epoch: 0, account_data: {} });
    stateStore.applyEntry({ type: 'MINING_REWARD', to: 'lp-user', token: 'BTCPC', amount: 5000, epoch: 0 });

    await ledger.recordBridgeFund('lp-user', 'ethereum', 2000, 365, 3);
    const balance = stateStore.getBalance('lp-user', 'BTCPC');
    expect(balance).toBe(3000);

    const pos = stateStore.getBridgeFunder('lp-user', 'ethereum');
    expect(pos).not.toBeNull();
    expect(pos.amount).toBe(2000);
    expect(pos.status).toBe('locked');
  });

  it('throws if amount is not positive', async () => {
    await expect(ledger.recordBridgeFund('x', 'base', -1, 30, 0)).rejects.toThrow('amount must be positive');
  });
});

describe('ledger.recordBridgeUnlock', () => {
  it('creates a BRIDGE_UNLOCK entry', async () => {
    const entry = await ledger.recordBridgeUnlock('shindevlin', 'base', 100);
    expect(entry.type).toBe('BRIDGE_UNLOCK');
    expect(entry.bridge_data.chain_id).toBe('base');
    expect(entry.bridge_data.direction).toBe('unlock');
  });

  it('transitions LP position status from locked to queued', async () => {
    stateStore.applyEntry({ type: 'ACCOUNT_CREATE', to: 'lp2', epoch: 0, account_data: {} });
    stateStore.applyEntry({ type: 'MINING_REWARD', to: 'lp2', token: 'BTCPC', amount: 3000, epoch: 0 });

    await ledger.recordBridgeFund('lp2', 'base', 1500, 30, 1);
    await ledger.recordBridgeUnlock('lp2', 'base', 50);

    const pos = stateStore.getBridgeFunder('lp2', 'base');
    expect(pos.status).toBe('queued');
    expect(pos.queued_epoch).toBe(50);
  });
});

// ─── stateStore — IoT Maps and getters ───────────────────────────────────────

describe('stateStore IoT Maps', () => {
  it('getAllSensors returns all registered sensors', () => {
    stateStore.applyEntry({ type: 'SENSOR_REGISTER', from: 'alice', epoch: 0,
      sensor_data: { sensor_id: 'alice/s1', owner: 'alice', type: 'temperature', unit: 'celsius', decimals: 2, region: 'us' }});
    stateStore.applyEntry({ type: 'SENSOR_REGISTER', from: 'bob', epoch: 0,
      sensor_data: { sensor_id: 'bob/s1', owner: 'bob', type: 'humidity', unit: 'pct', decimals: 1, region: 'eu' }});
    expect(stateStore.getAllSensors()).toHaveLength(2);
  });

  it('getAllSensors filters by type', () => {
    stateStore.applyEntry({ type: 'SENSOR_REGISTER', from: 'alice', epoch: 0,
      sensor_data: { sensor_id: 'alice/t', owner: 'alice', type: 'temperature', unit: 'c', decimals: 2, region: 'us' }});
    stateStore.applyEntry({ type: 'SENSOR_REGISTER', from: 'alice', epoch: 0,
      sensor_data: { sensor_id: 'alice/h', owner: 'alice', type: 'humidity', unit: 'pct', decimals: 1, region: 'us' }});
    const temps = stateStore.getAllSensors({ type: 'temperature' });
    expect(temps).toHaveLength(1);
    expect(temps[0].type).toBe('temperature');
  });

  it('getAllGateways returns all registered gateways', () => {
    stateStore.applyEntry({ type: 'GATEWAY_REGISTER', from: 'alice', epoch: 0,
      gateway_data: { gateway_id: 'alice/gw1', owner: 'alice', region: 'US915', latitude: 37.7, longitude: -122.4 }});
    stateStore.applyEntry({ type: 'GATEWAY_REGISTER', from: 'bob', epoch: 0,
      gateway_data: { gateway_id: 'bob/gw1', owner: 'bob', region: 'EU868', latitude: 51.5, longitude: -0.1 }});
    expect(stateStore.getAllGateways()).toHaveLength(2);
  });

  it('resetAll clears IoT Maps', () => {
    stateStore.applyEntry({ type: 'SENSOR_REGISTER', from: 'x', epoch: 0,
      sensor_data: { sensor_id: 'x/s', owner: 'x', type: 'temperature', unit: 'c', decimals: 2, region: 'r' }});
    stateStore.applyEntry({ type: 'GATEWAY_REGISTER', from: 'x', epoch: 0,
      gateway_data: { gateway_id: 'x/g', owner: 'x', region: 'US915', latitude: 0, longitude: 0 }});
    stateStore.resetAll();
    expect(stateStore.getAllSensors()).toHaveLength(0);
    expect(stateStore.getAllGateways()).toHaveLength(0);
  });
});

// ─── IoT reward pool percentage check ───────────────────────────────────────

describe('nanoRewards constants', () => {
  it('SENSOR_POOL_FRACTION is 0.60', () => {
    expect(nanoRewards.SENSOR_POOL_FRACTION).toBe(0.60);
  });

  it('GATEWAY_POOL_FRACTION is 0.40', () => {
    expect(nanoRewards.GATEWAY_POOL_FRACTION).toBe(0.40);
  });

  it('sensor + gateway fractions sum to 1.0', () => {
    expect(nanoRewards.SENSOR_POOL_FRACTION + nanoRewards.GATEWAY_POOL_FRACTION).toBe(1.0);
  });
});
