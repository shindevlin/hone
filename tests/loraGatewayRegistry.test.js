"use strict";

/**
 * LoRa Gateway Registry tests — v2.15-alpha
 *
 * Covers registration, heartbeats, retirement, region filtering,
 * stats tracking, and state transitions.
 */

const gr = require('../src/services/loraGatewayRegistry');

function baseSpec(overrides) {
  return Object.assign({
    region: 'us-east-1',
    latitude: 40.7128,
    longitude: -74.0060,
    antenna_gain_dbi: 3,
    hardware_model: 'helium-rak-v2',
    firmware_version: '2.1.0',
    max_sensors: 100,
  }, overrides || {});
}

describe('loraGatewayRegistry — gateway ID parsing', () => {
  it('accepts <owner>/<gateway-name>', () => {
    const p = gr.parseGatewayId('shindevlin/helium-01');
    expect(p.ok).toBe(true);
    expect(p.owner).toBe('shindevlin');
    expect(p.name).toBe('helium-01');
  });

  it('rejects missing slash', () => {
    expect(gr.parseGatewayId('shindevlinhelium01').ok).toBe(false);
  });

  it('rejects uppercase characters', () => {
    expect(gr.parseGatewayId('Shindevlin/helium-01').ok).toBe(false);
  });

  it('rejects non-string input', () => {
    expect(gr.parseGatewayId(null).ok).toBe(false);
  });
});

describe('loraGatewayRegistry — registration', () => {
  beforeEach(() => gr.resetForTests());

  it('registers a gateway with valid spec', () => {
    const rec = gr.registerGateway('shindevlin', 'shindevlin/helium-01', baseSpec(), { epoch: 50 });
    expect(rec.gateway_id).toBe('shindevlin/helium-01');
    expect(rec.owner).toBe('shindevlin');
    expect(rec.region).toBe('us-east-1');
    expect(rec.latitude).toBe(40.7128);
    expect(rec.longitude).toBe(-74.0060);
    expect(rec.antenna_gain_dbi).toBe(3);
    expect(rec.hardware_model).toBe('helium-rak-v2');
    expect(rec.max_sensors).toBe(100);
    expect(rec.status).toBe('active');
    expect(rec.created_epoch).toBe(50);
  });

  it('rejects registration where gateway_id prefix does not match owner', () => {
    expect(() => gr.registerGateway('alice', 'shindevlin/helium-01', baseSpec())).toThrow(/owner prefix/);
  });

  it('rejects missing region', () => {
    expect(() => gr.registerGateway('shindevlin', 'shindevlin/helium-01', baseSpec({ region: '' }))).toThrow(/region/);
  });

  it('rejects invalid latitude', () => {
    expect(() => gr.registerGateway('shindevlin', 'shindevlin/helium-01', baseSpec({ latitude: 91 }))).toThrow(/latitude/);
    expect(() => gr.registerGateway('shindevlin', 'shindevlin/helium-01', baseSpec({ latitude: -91 }))).toThrow(/latitude/);
    expect(() => gr.registerGateway('shindevlin', 'shindevlin/helium-01', baseSpec({ latitude: NaN }))).toThrow(/latitude/);
  });

  it('rejects invalid longitude', () => {
    expect(() => gr.registerGateway('shindevlin', 'shindevlin/helium-01', baseSpec({ longitude: 181 }))).toThrow(/longitude/);
    expect(() => gr.registerGateway('shindevlin', 'shindevlin/helium-01', baseSpec({ longitude: -181 }))).toThrow(/longitude/);
  });

  it('allows updating an existing gateway', () => {
    gr.registerGateway('shindevlin', 'shindevlin/helium-01', baseSpec(), { epoch: 1 });
    const updated = gr.registerGateway('shindevlin', 'shindevlin/helium-01', baseSpec({ antenna_gain_dbi: 6 }), { epoch: 2 });
    expect(updated.antenna_gain_dbi).toBe(6);
    expect(updated.last_updated_epoch).toBe(2);
  });

  it('cannot update a gateway owned by a different account', () => {
    gr.registerGateway('shindevlin', 'shindevlin/helium-01', baseSpec());
    expect(() => gr.registerGateway('natoshisakamoto', 'shindevlin/helium-01', baseSpec())).toThrow(/owner/);
  });

  it('cannot update a retired gateway', () => {
    gr.registerGateway('shindevlin', 'shindevlin/helium-01', baseSpec());
    gr.retireGateway('shindevlin', 'shindevlin/helium-01');
    expect(() => gr.registerGateway('shindevlin', 'shindevlin/helium-01', baseSpec())).toThrow(/retired/);
  });
});

describe('loraGatewayRegistry — heartbeat', () => {
  beforeEach(() => gr.resetForTests());

  it('records a heartbeat and updates stats', () => {
    gr.registerGateway('shindevlin', 'shindevlin/helium-01', baseSpec(), { epoch: 1 });
    const rec = gr.heartbeat('shindevlin/helium-01', {
      sensors_connected: 5,
      packets_relayed_this_epoch: 120,
      uptime_seconds: 300,
      battery_pct: null,
    }, 10);
    expect(rec.last_heartbeat_epoch).toBe(10);
    expect(rec.total_heartbeats).toBe(1);
    expect(rec.total_packets_relayed).toBe(120);
    expect(rec.last_stats.sensors_connected).toBe(5);
    expect(rec.last_stats.packets_relayed_this_epoch).toBe(120);
  });

  it('accumulates packets across multiple heartbeats', () => {
    gr.registerGateway('shindevlin', 'shindevlin/helium-01', baseSpec(), { epoch: 1 });
    gr.heartbeat('shindevlin/helium-01', { packets_relayed_this_epoch: 50 }, 10);
    gr.heartbeat('shindevlin/helium-01', { packets_relayed_this_epoch: 75 }, 11);
    const rec = gr.getGateway('shindevlin/helium-01');
    expect(rec.total_packets_relayed).toBe(125);
    expect(rec.total_heartbeats).toBe(2);
  });

  it('throws for unknown gateway', () => {
    expect(() => gr.heartbeat('shindevlin/ghost', {}, 1)).toThrow(/gateway not found/);
  });

  it('throws for retired gateway', () => {
    gr.registerGateway('shindevlin', 'shindevlin/helium-01', baseSpec());
    gr.retireGateway('shindevlin', 'shindevlin/helium-01');
    expect(() => gr.heartbeat('shindevlin/helium-01', {}, 10)).toThrow(/retired/);
  });
});

describe('loraGatewayRegistry — retire', () => {
  beforeEach(() => gr.resetForTests());

  it('retires a gateway', () => {
    gr.registerGateway('shindevlin', 'shindevlin/helium-01', baseSpec(), { epoch: 1 });
    const rec = gr.retireGateway('shindevlin', 'shindevlin/helium-01', 200);
    expect(rec.status).toBe('retired');
    expect(rec.retired_epoch).toBe(200);
  });

  it('retire is idempotent', () => {
    gr.registerGateway('shindevlin', 'shindevlin/helium-01', baseSpec(), { epoch: 1 });
    gr.retireGateway('shindevlin', 'shindevlin/helium-01', 100);
    const rec = gr.retireGateway('shindevlin', 'shindevlin/helium-01', 200);
    expect(rec.retired_epoch).toBe(100);
  });

  it('only the owner can retire', () => {
    gr.registerGateway('shindevlin', 'shindevlin/helium-01', baseSpec());
    expect(() => gr.retireGateway('alice', 'shindevlin/helium-01')).toThrow(/owner/);
  });
});

describe('loraGatewayRegistry — region filtering', () => {
  beforeEach(() => gr.resetForTests());

  it('getGatewaysInRegion returns only gateways in that region', () => {
    gr.registerGateway('shindevlin', 'shindevlin/helium-01', baseSpec({ region: 'us-east-1' }));
    gr.registerGateway('shindevlin', 'shindevlin/helium-02', baseSpec({ region: 'eu-west-1' }));
    gr.registerGateway('natoshisakamoto', 'natoshisakamoto/gw-01', baseSpec({ region: 'us-east-1' }));
    const east = gr.getGatewaysInRegion('us-east-1');
    expect(east.length).toBe(2);
    const ids = east.map(g => g.gateway_id);
    expect(ids).toContain('shindevlin/helium-01');
    expect(ids).toContain('natoshisakamoto/gw-01');
  });

  it('getAllGateways with owner filter', () => {
    gr.registerGateway('shindevlin', 'shindevlin/helium-01', baseSpec());
    gr.registerGateway('natoshisakamoto', 'natoshisakamoto/gw-01', baseSpec());
    expect(gr.getAllGateways({ owner: 'shindevlin' }).length).toBe(1);
  });

  it('getAllGateways with status filter', () => {
    gr.registerGateway('shindevlin', 'shindevlin/helium-01', baseSpec());
    gr.registerGateway('shindevlin', 'shindevlin/helium-02', baseSpec());
    gr.retireGateway('shindevlin', 'shindevlin/helium-02');
    expect(gr.getAllGateways({ status: 'active' }).length).toBe(1);
    expect(gr.getAllGateways({ status: 'retired' }).length).toBe(1);
  });
});

describe('loraGatewayRegistry — stats', () => {
  beforeEach(() => gr.resetForTests());

  it('getGatewayStats returns uptime_pct based on heartbeat epochs', () => {
    gr.registerGateway('shindevlin', 'shindevlin/helium-01', baseSpec(), { epoch: 1 });
    gr.heartbeat('shindevlin/helium-01', { packets_relayed_this_epoch: 50 }, 1);
    gr.heartbeat('shindevlin/helium-01', { packets_relayed_this_epoch: 50 }, 2);
    gr.heartbeat('shindevlin/helium-01', { packets_relayed_this_epoch: 50 }, 3);
    const stats = gr.getGatewayStats('shindevlin/helium-01', 4);
    // 3 heartbeat epochs out of 4 epoch span (epochs 1-4)
    expect(stats.uptime_pct).toBeCloseTo(0.75, 2);
    expect(stats.total_packets_relayed).toBe(150);
    expect(stats.total_heartbeats).toBe(3);
  });

  it('getGatewayStats returns null for unknown gateway', () => {
    expect(gr.getGatewayStats('shindevlin/ghost')).toBeNull();
  });

  it('idle transition fires after IDLE_HEARTBEAT_THRESHOLD epochs of silence', () => {
    gr.registerGateway('shindevlin', 'shindevlin/helium-01', baseSpec(), { epoch: 1 });
    gr.heartbeat('shindevlin/helium-01', { packets_relayed_this_epoch: 10 }, 10);
    const threshold = gr.IDLE_HEARTBEAT_THRESHOLD;
    gr._checkIdleTransition('shindevlin/helium-01', 10 + threshold);
    expect(gr.getGateway('shindevlin/helium-01').status).toBe('idle');
  });

  it('heartbeat reactivates an idle gateway', () => {
    gr.registerGateway('shindevlin', 'shindevlin/helium-01', baseSpec(), { epoch: 1 });
    gr.heartbeat('shindevlin/helium-01', { packets_relayed_this_epoch: 10 }, 10);
    gr._checkIdleTransition('shindevlin/helium-01', 10 + gr.IDLE_HEARTBEAT_THRESHOLD);
    expect(gr.getGateway('shindevlin/helium-01').status).toBe('idle');
    gr.heartbeat('shindevlin/helium-01', { packets_relayed_this_epoch: 5 }, 200);
    expect(gr.getGateway('shindevlin/helium-01').status).toBe('active');
  });
});
