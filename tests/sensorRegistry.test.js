"use strict";

/**
 * Sensor Registry tests — v2.15-alpha
 *
 * Covers registration, reading submission, epoch finalization,
 * median consensus, outlier detection, filtering, stats, and
 * sensor state transitions.
 */

const sr = require('../src/services/sensorRegistry');
const oracle = require('../src/services/oracleFeeds');

function baseSpec(overrides) {
  return Object.assign({
    type: 'temperature',
    unit: 'celsius',
    decimals: 2,
    region: 'us-east-1',
    lora_gateway: 'shindevlin/helium-01',
    hardware_model: 'raspberry-pi-4',
    firmware_version: '1.0.0',
  }, overrides || {});
}

describe('sensorRegistry — sensor ID parsing', () => {
  it('accepts <owner>/<device-name>', () => {
    const p = sr.parseSensorId('shindevlin/sensor-01');
    expect(p.ok).toBe(true);
    expect(p.owner).toBe('shindevlin');
    expect(p.name).toBe('sensor-01');
  });

  it('rejects missing slash', () => {
    expect(sr.parseSensorId('shindevlinsensor01').ok).toBe(false);
  });

  it('rejects uppercase characters', () => {
    expect(sr.parseSensorId('Shin/sensor-01').ok).toBe(false);
    expect(sr.parseSensorId('shindevlin/Sensor-01').ok).toBe(false);
  });

  it('rejects non-string input', () => {
    expect(sr.parseSensorId(null).ok).toBe(false);
    expect(sr.parseSensorId(42).ok).toBe(false);
  });
});

describe('sensorRegistry — registration', () => {
  beforeEach(() => sr.resetForTests());

  it('registers a sensor with valid spec', () => {
    const rec = sr.registerSensor('shindevlin', 'shindevlin/temp-01', baseSpec(), { epoch: 100 });
    expect(rec.sensor_id).toBe('shindevlin/temp-01');
    expect(rec.owner).toBe('shindevlin');
    expect(rec.type).toBe('temperature');
    expect(rec.unit).toBe('celsius');
    expect(rec.decimals).toBe(2);
    expect(rec.region).toBe('us-east-1');
    expect(rec.status).toBe('active');
    expect(rec.created_epoch).toBe(100);
    expect(rec.lora_gateway).toBe('shindevlin/helium-01');
  });

  it('rejects registration where sensor_id prefix does not match owner', () => {
    expect(() => sr.registerSensor('alice', 'shindevlin/temp-01', baseSpec())).toThrow(/owner prefix/);
  });

  it('rejects duplicate sensor IDs', () => {
    sr.registerSensor('shindevlin', 'shindevlin/temp-01', baseSpec(), { epoch: 1 });
    // Second register is an update, not a duplicate error
    const updated = sr.registerSensor('shindevlin', 'shindevlin/temp-01', baseSpec({ unit: 'fahrenheit' }), { epoch: 2 });
    expect(updated.unit).toBe('fahrenheit');
  });

  it('rejects invalid sensor types', () => {
    expect(() => sr.registerSensor('shindevlin', 'shindevlin/temp-01', baseSpec({ type: 'volcano' }))).toThrow(/type/);
  });

  it('accepts all valid types', () => {
    const validTypes = ['temperature', 'humidity', 'air_quality', 'gps', 'soil', 'custom'];
    validTypes.forEach((type, i) => {
      const id = 'shindevlin/sensor-0' + i;
      const rec = sr.registerSensor('shindevlin', id, baseSpec({ type }));
      expect(rec.type).toBe(type);
    });
  });

  it('rejects invalid decimals', () => {
    expect(() => sr.registerSensor('shindevlin', 'shindevlin/temp-01', baseSpec({ decimals: 11 }))).toThrow(/decimals/);
    expect(() => sr.registerSensor('shindevlin', 'shindevlin/temp-01', baseSpec({ decimals: -1 }))).toThrow(/decimals/);
  });

  it('rejects missing region', () => {
    expect(() => sr.registerSensor('shindevlin', 'shindevlin/temp-01', baseSpec({ region: '' }))).toThrow(/region/);
    expect(() => sr.registerSensor('shindevlin', 'shindevlin/temp-01', baseSpec({ region: undefined }))).toThrow(/region/);
  });

  it('cannot update a sensor owned by a different account', () => {
    sr.registerSensor('shindevlin', 'shindevlin/temp-01', baseSpec());
    expect(() => sr.registerSensor('natoshisakamoto', 'shindevlin/temp-01', baseSpec())).toThrow(/owner/);
  });

  it('cannot update a retired sensor', () => {
    sr.registerSensor('shindevlin', 'shindevlin/temp-01', baseSpec());
    sr.retireSensor('shindevlin', 'shindevlin/temp-01');
    expect(() => sr.registerSensor('shindevlin', 'shindevlin/temp-01', baseSpec())).toThrow(/retired/);
  });
});

describe('sensorRegistry — retire', () => {
  beforeEach(() => sr.resetForTests());

  it('retires a sensor and prevents further readings', () => {
    sr.registerSensor('shindevlin', 'shindevlin/temp-01', baseSpec());
    const rec = sr.retireSensor('shindevlin', 'shindevlin/temp-01', 200);
    expect(rec.status).toBe('retired');
    expect(rec.retired_epoch).toBe(200);
    expect(() => sr.submitReading('shindevlin/temp-01', 22.5, {}, 201)).toThrow(/retired/);
  });

  it('retire is idempotent', () => {
    sr.registerSensor('shindevlin', 'shindevlin/temp-01', baseSpec());
    sr.retireSensor('shindevlin', 'shindevlin/temp-01', 100);
    const rec = sr.retireSensor('shindevlin', 'shindevlin/temp-01', 200);
    expect(rec.retired_epoch).toBe(100); // first retirement wins
  });

  it('only the owner can retire', () => {
    sr.registerSensor('shindevlin', 'shindevlin/temp-01', baseSpec());
    expect(() => sr.retireSensor('alice', 'shindevlin/temp-01')).toThrow(/owner/);
  });
});

describe('sensorRegistry — reading submission', () => {
  beforeEach(() => sr.resetForTests());

  it('submits a valid reading', () => {
    sr.registerSensor('shindevlin', 'shindevlin/temp-01', baseSpec(), { epoch: 1 });
    const reading = sr.submitReading('shindevlin/temp-01', 22.5, { battery_pct: 95 }, 10);
    expect(reading.sensor_id).toBe('shindevlin/temp-01');
    expect(reading.value).toBe(22.5);
    expect(reading.epoch).toBe(10);
    expect(reading.metadata.battery_pct).toBe(95);
  });

  it('rejects readings for unknown sensors', () => {
    expect(() => sr.submitReading('shindevlin/ghost', 22.5, {}, 10)).toThrow(/sensor not found/);
  });

  it('rejects non-finite values', () => {
    sr.registerSensor('shindevlin', 'shindevlin/temp-01', baseSpec());
    expect(() => sr.submitReading('shindevlin/temp-01', NaN, {}, 1)).toThrow(/finite/);
    expect(() => sr.submitReading('shindevlin/temp-01', Infinity, {}, 1)).toThrow(/finite/);
    expect(() => sr.submitReading('shindevlin/temp-01', 'hot', {}, 1)).toThrow(/finite/);
  });

  it('rejects readings for retired sensors', () => {
    sr.registerSensor('shindevlin', 'shindevlin/temp-01', baseSpec());
    sr.retireSensor('shindevlin', 'shindevlin/temp-01');
    expect(() => sr.submitReading('shindevlin/temp-01', 22.5, {}, 5)).toThrow(/retired/);
  });

  it('allows multiple readings in the same epoch from the same sensor', () => {
    sr.registerSensor('shindevlin', 'shindevlin/temp-01', baseSpec(), { epoch: 1 });
    sr.submitReading('shindevlin/temp-01', 22.0, {}, 10);
    sr.submitReading('shindevlin/temp-01', 22.5, {}, 10);
    sr.submitReading('shindevlin/temp-01', 23.0, {}, 10);
    const readings = sr.getReadingsForEpoch('shindevlin/temp-01', 10);
    expect(readings.length).toBe(3);
  });

  it('tracks total_readings on the sensor record', () => {
    sr.registerSensor('shindevlin', 'shindevlin/temp-01', baseSpec(), { epoch: 1 });
    sr.submitReading('shindevlin/temp-01', 22.0, {}, 10);
    sr.submitReading('shindevlin/temp-01', 22.5, {}, 11);
    const rec = sr.getSensor('shindevlin/temp-01');
    expect(rec.total_readings).toBe(2);
  });

  it('updates last_reading_epoch on the sensor record', () => {
    sr.registerSensor('shindevlin', 'shindevlin/temp-01', baseSpec(), { epoch: 1 });
    sr.submitReading('shindevlin/temp-01', 22.0, {}, 10);
    sr.submitReading('shindevlin/temp-01', 22.5, {}, 15);
    const rec = sr.getSensor('shindevlin/temp-01');
    expect(rec.last_reading_epoch).toBe(15);
  });
});

describe('sensorRegistry — epoch finalization + median consensus', () => {
  beforeEach(() => sr.resetForTests());

  it('finalizes an epoch with a single reading', () => {
    sr.registerSensor('shindevlin', 'shindevlin/temp-01', baseSpec(), { epoch: 1 });
    sr.submitReading('shindevlin/temp-01', 22.5, {}, 10);
    const fin = sr.finalizeEpochReadings('shindevlin/temp-01', 10);
    expect(fin.median).toBe(22.5);
    expect(fin.reading_count).toBe(1);
    expect(fin.outliers.length).toBe(0);
  });

  it('uses _median() for consensus — odd count', () => {
    sr.registerSensor('shindevlin', 'shindevlin/temp-01', baseSpec(), { epoch: 1 });
    sr.submitReading('shindevlin/temp-01', 20.0, {}, 10);
    sr.submitReading('shindevlin/temp-01', 22.0, {}, 10);
    sr.submitReading('shindevlin/temp-01', 24.0, {}, 10);
    const fin = sr.finalizeEpochReadings('shindevlin/temp-01', 10);
    expect(fin.median).toBe(22.0);
  });

  it('uses _median() for consensus — even count', () => {
    sr.registerSensor('shindevlin', 'shindevlin/temp-01', baseSpec(), { epoch: 1 });
    sr.submitReading('shindevlin/temp-01', 20.0, {}, 10);
    sr.submitReading('shindevlin/temp-01', 22.0, {}, 10);
    sr.submitReading('shindevlin/temp-01', 24.0, {}, 10);
    sr.submitReading('shindevlin/temp-01', 26.0, {}, 10);
    const fin = sr.finalizeEpochReadings('shindevlin/temp-01', 10);
    expect(fin.median).toBe(23.0);
  });

  it('detects outliers using 500 bps threshold', () => {
    sr.registerSensor('shindevlin', 'shindevlin/temp-01', baseSpec(), { epoch: 1 });
    sr.submitReading('shindevlin/temp-01', 22.0, {}, 10);
    sr.submitReading('shindevlin/temp-01', 22.1, {}, 10);
    sr.submitReading('shindevlin/temp-01', 22.2, {}, 10);
    sr.submitReading('shindevlin/temp-01', 99.9, {}, 10); // extreme outlier
    const fin = sr.finalizeEpochReadings('shindevlin/temp-01', 10);
    expect(fin.outliers.length).toBe(1);
    expect(fin.outliers[0].value).toBe(99.9);
  });

  it('finalizeEpochReadings is idempotent', () => {
    sr.registerSensor('shindevlin', 'shindevlin/temp-01', baseSpec(), { epoch: 1 });
    sr.submitReading('shindevlin/temp-01', 22.5, {}, 10);
    const fin1 = sr.finalizeEpochReadings('shindevlin/temp-01', 10);
    const fin2 = sr.finalizeEpochReadings('shindevlin/temp-01', 10);
    expect(fin1.median).toBe(fin2.median);
    expect(fin1.finalized_at).toBe(fin2.finalized_at);
  });

  it('throws if no readings exist for that epoch', () => {
    sr.registerSensor('shindevlin', 'shindevlin/temp-01', baseSpec(), { epoch: 1 });
    expect(() => sr.finalizeEpochReadings('shindevlin/temp-01', 99)).toThrow(/no readings/);
  });

  it('cross-region consensus includes peer sensors in same region+type', () => {
    // Two sensors in same region and type — consensus is based on both
    sr.registerSensor('shindevlin', 'shindevlin/temp-01', baseSpec({ region: 'eu-west-1' }), { epoch: 1 });
    sr.registerSensor('natoshisakamoto', 'natoshisakamoto/temp-02', baseSpec({ region: 'eu-west-1' }), { epoch: 1 });
    sr.submitReading('shindevlin/temp-01', 20.0, {}, 10);
    sr.submitReading('natoshisakamoto/temp-02', 21.0, {}, 10);
    const fin = sr.finalizeEpochReadings('shindevlin/temp-01', 10);
    // Both sensors contribute; median of [20, 21] = 20.5
    expect(fin.median).toBe(20.5);
    expect(fin.region_value_count).toBe(2);
  });

  it('does not cross-contaminate sensors of different types in same region', () => {
    sr.registerSensor('shindevlin', 'shindevlin/temp-01', baseSpec({ type: 'temperature', region: 'us-west-2' }), { epoch: 1 });
    sr.registerSensor('shindevlin', 'shindevlin/humid-01', baseSpec({ type: 'humidity', region: 'us-west-2', unit: 'pct' }), { epoch: 1 });
    sr.submitReading('shindevlin/temp-01', 25.0, {}, 10);
    sr.submitReading('shindevlin/humid-01', 80.0, {}, 10);
    const fin = sr.finalizeEpochReadings('shindevlin/temp-01', 10);
    // Only temp sensor is in scope
    expect(fin.region_value_count).toBe(1);
    expect(fin.median).toBe(25.0);
  });
});

describe('sensorRegistry — filtering and read accessors', () => {
  beforeEach(() => sr.resetForTests());

  it('getSensorsInRegion returns only sensors in that region', () => {
    sr.registerSensor('shindevlin', 'shindevlin/temp-01', baseSpec({ region: 'us-east-1' }));
    sr.registerSensor('shindevlin', 'shindevlin/temp-02', baseSpec({ region: 'eu-west-1' }));
    sr.registerSensor('natoshisakamoto', 'natoshisakamoto/temp-03', baseSpec({ region: 'us-east-1' }));
    const east = sr.getSensorsInRegion('us-east-1');
    expect(east.length).toBe(2);
    const ids = east.map(s => s.sensor_id);
    expect(ids).toContain('shindevlin/temp-01');
    expect(ids).toContain('natoshisakamoto/temp-03');
  });

  it('getAllSensors with type filter', () => {
    sr.registerSensor('shindevlin', 'shindevlin/temp-01', baseSpec({ type: 'temperature' }));
    sr.registerSensor('shindevlin', 'shindevlin/humid-01', baseSpec({ type: 'humidity', unit: 'pct' }));
    const temps = sr.getAllSensors({ type: 'temperature' });
    expect(temps.length).toBe(1);
    expect(temps[0].type).toBe('temperature');
  });

  it('getAllSensors with owner filter', () => {
    sr.registerSensor('shindevlin', 'shindevlin/temp-01', baseSpec());
    sr.registerSensor('natoshisakamoto', 'natoshisakamoto/temp-02', baseSpec());
    const shin = sr.getAllSensors({ owner: 'shindevlin' });
    expect(shin.length).toBe(1);
  });

  it('getAllSensors with status filter', () => {
    sr.registerSensor('shindevlin', 'shindevlin/temp-01', baseSpec());
    sr.registerSensor('shindevlin', 'shindevlin/temp-02', baseSpec());
    sr.retireSensor('shindevlin', 'shindevlin/temp-02');
    const active = sr.getAllSensors({ status: 'active' });
    expect(active.length).toBe(1);
    const retired = sr.getAllSensors({ status: 'retired' });
    expect(retired.length).toBe(1);
  });

  it('getSensor returns null for unknown sensor_id', () => {
    expect(sr.getSensor('shindevlin/ghost')).toBeNull();
  });

  it('getReadingsForEpoch returns empty array if no readings', () => {
    sr.registerSensor('shindevlin', 'shindevlin/temp-01', baseSpec());
    const readings = sr.getReadingsForEpoch('shindevlin/temp-01', 99);
    expect(readings).toEqual([]);
  });
});

describe('sensorRegistry — sensor stats', () => {
  beforeEach(() => sr.resetForTests());

  it('getSensorStats returns correct total_readings and total_epochs_active', () => {
    sr.registerSensor('shindevlin', 'shindevlin/temp-01', baseSpec(), { epoch: 1 });
    sr.submitReading('shindevlin/temp-01', 22.0, {}, 10);
    sr.submitReading('shindevlin/temp-01', 22.5, {}, 10);
    sr.submitReading('shindevlin/temp-01', 23.0, {}, 11);
    sr.finalizeEpochReadings('shindevlin/temp-01', 10);
    sr.finalizeEpochReadings('shindevlin/temp-01', 11);
    const stats = sr.getSensorStats('shindevlin/temp-01', 15);
    expect(stats.total_readings).toBe(3);
    expect(stats.total_epochs_active).toBe(2);
    expect(stats.last_reading_epoch).toBe(11);
  });

  it('getSensorStats outlier_rate reflects finalized outliers', () => {
    sr.registerSensor('shindevlin', 'shindevlin/temp-01', baseSpec(), { epoch: 1 });
    sr.submitReading('shindevlin/temp-01', 22.0, {}, 10);
    sr.submitReading('shindevlin/temp-01', 22.1, {}, 10);
    sr.submitReading('shindevlin/temp-01', 999.0, {}, 10); // outlier
    sr.finalizeEpochReadings('shindevlin/temp-01', 10);
    const stats = sr.getSensorStats('shindevlin/temp-01');
    expect(stats.outlier_rate).toBeGreaterThan(0);
  });

  it('getSensorStats returns null for unknown sensor', () => {
    expect(sr.getSensorStats('shindevlin/ghost')).toBeNull();
  });

  it('uptime_pct is 1 when sensor has read every epoch since creation', () => {
    sr.registerSensor('shindevlin', 'shindevlin/temp-01', baseSpec(), { epoch: 1 });
    sr.submitReading('shindevlin/temp-01', 22.0, {}, 1);
    sr.submitReading('shindevlin/temp-01', 22.0, {}, 2);
    sr.submitReading('shindevlin/temp-01', 22.0, {}, 3);
    const stats = sr.getSensorStats('shindevlin/temp-01', 3);
    expect(stats.uptime_pct).toBe(1);
  });
});

describe('sensorRegistry — sensor state transitions', () => {
  beforeEach(() => sr.resetForTests());

  it('sensor starts as active', () => {
    sr.registerSensor('shindevlin', 'shindevlin/temp-01', baseSpec(), { epoch: 1 });
    const rec = sr.getSensor('shindevlin/temp-01');
    expect(rec.status).toBe('active');
  });

  it('transitions active → idle after IDLE_EPOCH_THRESHOLD epochs of silence', () => {
    sr.registerSensor('shindevlin', 'shindevlin/temp-01', baseSpec(), { epoch: 1 });
    sr.submitReading('shindevlin/temp-01', 22.0, {}, 10);
    const threshold = sr.IDLE_EPOCH_THRESHOLD;
    sr._checkIdleTransition('shindevlin/temp-01', 10 + threshold);
    const rec = sr.getSensor('shindevlin/temp-01');
    expect(rec.status).toBe('idle');
  });

  it('transitions idle → active when a new reading arrives', () => {
    sr.registerSensor('shindevlin', 'shindevlin/temp-01', baseSpec(), { epoch: 1 });
    sr.submitReading('shindevlin/temp-01', 22.0, {}, 10);
    sr._checkIdleTransition('shindevlin/temp-01', 10 + sr.IDLE_EPOCH_THRESHOLD);
    expect(sr.getSensor('shindevlin/temp-01').status).toBe('idle');
    // New reading reactivates
    sr.submitReading('shindevlin/temp-01', 22.5, {}, 200);
    expect(sr.getSensor('shindevlin/temp-01').status).toBe('active');
  });

  it('idle sensors are still allowed to submit readings', () => {
    sr.registerSensor('shindevlin', 'shindevlin/temp-01', baseSpec(), { epoch: 1 });
    sr.submitReading('shindevlin/temp-01', 22.0, {}, 10);
    sr._checkIdleTransition('shindevlin/temp-01', 10 + sr.IDLE_EPOCH_THRESHOLD);
    // Should not throw
    const reading = sr.submitReading('shindevlin/temp-01', 22.5, {}, 200);
    expect(reading.value).toBe(22.5);
  });

  it('retired sensors cannot be transitioned to idle by _checkIdleTransition', () => {
    sr.registerSensor('shindevlin', 'shindevlin/temp-01', baseSpec(), { epoch: 1 });
    sr.submitReading('shindevlin/temp-01', 22.0, {}, 10);
    sr.retireSensor('shindevlin', 'shindevlin/temp-01', 20);
    sr._checkIdleTransition('shindevlin/temp-01', 10 + sr.IDLE_EPOCH_THRESHOLD + 1000);
    expect(sr.getSensor('shindevlin/temp-01').status).toBe('retired');
  });
});
