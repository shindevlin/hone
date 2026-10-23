"use strict";

/**
 * IoT Sensor + Gateway HTTP route tests (v2.15-beta).
 *
 * Follows the serviceRoutes.test.js pattern: ephemeral express server
 * on port 0, JWT bypassed via x-test-user header, no supertest dependency.
 */

const http = require('http');
const express = require('express');

jest.mock('../src/middlewares/auth', () => ({
  authenticateToken: (req, res, next) => {
    const username = req.headers['x-test-user'];
    if (!username) return res.status(401).json({ error: 'unauthenticated' });
    req.user = { username, id: 'test-id-' + username };
    next();
  },
  limiter: (req, res, next) => next(),
}));

jest.mock('../src/p2p/mempool', () => ({
  submit: jest.fn(),
  addTransaction: jest.fn().mockReturnValue(true),
}));

const sensorRegistry = require('../src/services/sensorRegistry');
const gatewayRegistry = require('../src/services/loraGatewayRegistry');
const { sensorsRouter, gatewaysRouter } = require('../src/routes/sensorRoutes');

function makeTestServer() {
  const app = express();
  app.use(express.json());
  app.use('/api/sensors', sensorsRouter);
  app.use('/api/gateways', gatewaysRouter);
  const server = http.createServer(app);
  return new Promise((resolve) => {
    server.listen(0, '127.0.0.1', () => {
      resolve({ server, port: server.address().port });
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

describe('sensor + gateway HTTP routes (v2.15-beta)', () => {
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
    sensorRegistry.resetForTests();
    gatewayRegistry.resetForTests();
  });

  // ─────────────────────────────────────────────────────────────────
  // POST /api/sensors
  // ─────────────────────────────────────────────────────────────────

  describe('POST /api/sensors', () => {
    it('rejects unauthenticated requests', async () => {
      const res = await request(port, 'POST', '/api/sensors', {
        name: 'temp1', type: 'temperature', unit: 'celsius', decimals: 2, region: 'us-east',
      });
      expect(res.status).toBe(401);
    });

    it('registers a sensor owned by the authenticated user', async () => {
      const res = await request(port, 'POST', '/api/sensors', {
        name: 'temp1', type: 'temperature', unit: 'celsius', decimals: 2, region: 'us-east',
      }, 'alice');
      expect(res.status).toBe(201);
      expect(res.body.success).toBe(true);
      expect(res.body.sensor.sensor_id).toBe('alice/temp1');
      expect(res.body.sensor.owner).toBe('alice');
      expect(res.body.sensor.status).toBe('active');
      expect(res.body.sensor.type).toBe('temperature');
    });

    it('rejects missing name', async () => {
      const res = await request(port, 'POST', '/api/sensors', {
        type: 'temperature', unit: 'celsius', decimals: 2, region: 'us-east',
      }, 'alice');
      expect(res.status).toBe(400);
      expect(res.body.error).toMatch(/name required/);
    });

    it('rejects missing type', async () => {
      const res = await request(port, 'POST', '/api/sensors', {
        name: 'temp1', unit: 'celsius', decimals: 2, region: 'us-east',
      }, 'alice');
      expect(res.status).toBe(400);
      expect(res.body.error).toMatch(/type required/);
    });

    it('rejects missing unit', async () => {
      const res = await request(port, 'POST', '/api/sensors', {
        name: 'temp1', type: 'temperature', decimals: 2, region: 'us-east',
      }, 'alice');
      expect(res.status).toBe(400);
      expect(res.body.error).toMatch(/unit required/);
    });

    it('rejects missing region', async () => {
      const res = await request(port, 'POST', '/api/sensors', {
        name: 'temp1', type: 'temperature', unit: 'celsius', decimals: 2,
      }, 'alice');
      expect(res.status).toBe(400);
      expect(res.body.error).toMatch(/region required/);
    });

    it('rejects invalid sensor type from registry', async () => {
      const res = await request(port, 'POST', '/api/sensors', {
        name: 'temp1', type: 'laser-cannon', unit: 'celsius', decimals: 2, region: 'us-east',
      }, 'alice');
      expect(res.status).toBe(422);
      expect(res.body.error).toMatch(/type must be one of/);
    });
  });

  // ─────────────────────────────────────────────────────────────────
  // POST /api/sensors/:id/readings
  // ─────────────────────────────────────────────────────────────────

  describe('POST /api/sensors/:id/readings', () => {
    beforeEach(async () => {
      await request(port, 'POST', '/api/sensors', {
        name: 'temp1', type: 'temperature', unit: 'celsius', decimals: 2, region: 'us-east',
      }, 'alice');
    });

    it('rejects unauthenticated requests', async () => {
      const res = await request(
        port, 'POST',
        '/api/sensors/' + encodeURIComponent('alice/temp1') + '/readings',
        { value: 22.5 }
      );
      expect(res.status).toBe(401);
    });

    it('submits a reading for an existing sensor', async () => {
      const res = await request(
        port, 'POST',
        '/api/sensors/' + encodeURIComponent('alice/temp1') + '/readings',
        { value: 22.5 },
        'alice'
      );
      expect(res.status).toBe(201);
      expect(res.body.success).toBe(true);
      expect(res.body.reading.value).toBe(22.5);
      expect(res.body.reading.sensor_id).toBe('alice/temp1');
    });

    it('allows any authenticated user to submit readings (gateway relay)', async () => {
      const res = await request(
        port, 'POST',
        '/api/sensors/' + encodeURIComponent('alice/temp1') + '/readings',
        { value: 18.0 },
        'gatewaynode'
      );
      expect(res.status).toBe(201);
    });

    it('rejects missing value', async () => {
      const res = await request(
        port, 'POST',
        '/api/sensors/' + encodeURIComponent('alice/temp1') + '/readings',
        {},
        'alice'
      );
      expect(res.status).toBe(400);
      expect(res.body.error).toMatch(/value required/);
    });

    it('rejects non-finite value', async () => {
      const res = await request(
        port, 'POST',
        '/api/sensors/' + encodeURIComponent('alice/temp1') + '/readings',
        { value: 'hot' },
        'alice'
      );
      expect(res.status).toBe(400);
      expect(res.body.error).toMatch(/finite/);
    });

    it('returns 404 for unknown sensor', async () => {
      const res = await request(
        port, 'POST',
        '/api/sensors/' + encodeURIComponent('alice/ghost') + '/readings',
        { value: 22.5 },
        'alice'
      );
      expect(res.status).toBe(404);
    });

    it('accepts optional metadata', async () => {
      const res = await request(
        port, 'POST',
        '/api/sensors/' + encodeURIComponent('alice/temp1') + '/readings',
        { value: 22.5, metadata: { latitude: 40.7, longitude: -74.0, battery_pct: 85 } },
        'alice'
      );
      expect(res.status).toBe(201);
      expect(res.body.reading.metadata.latitude).toBe(40.7);
      expect(res.body.reading.metadata.battery_pct).toBe(85);
    });
  });

  // ─────────────────────────────────────────────────────────────────
  // POST /api/sensors/:id/retire
  // ─────────────────────────────────────────────────────────────────

  describe('POST /api/sensors/:id/retire', () => {
    beforeEach(async () => {
      await request(port, 'POST', '/api/sensors', {
        name: 'temp1', type: 'temperature', unit: 'celsius', decimals: 2, region: 'us-east',
      }, 'alice');
    });

    it('retires a sensor', async () => {
      const res = await request(
        port, 'POST',
        '/api/sensors/' + encodeURIComponent('alice/temp1') + '/retire',
        {}, 'alice'
      );
      expect(res.status).toBe(200);
      expect(res.body.success).toBe(true);
      expect(res.body.sensor.status).toBe('retired');
    });

    it('rejects retire from non-owner', async () => {
      const res = await request(
        port, 'POST',
        '/api/sensors/' + encodeURIComponent('alice/temp1') + '/retire',
        {}, 'bob'
      );
      expect(res.status).toBe(403);
    });

    it('returns 404 for unknown sensor', async () => {
      const res = await request(
        port, 'POST',
        '/api/sensors/' + encodeURIComponent('alice/ghost') + '/retire',
        {}, 'alice'
      );
      expect(res.status).toBe(404);
    });

    it('rejects reading submission after retirement', async () => {
      await request(
        port, 'POST',
        '/api/sensors/' + encodeURIComponent('alice/temp1') + '/retire',
        {}, 'alice'
      );
      const res = await request(
        port, 'POST',
        '/api/sensors/' + encodeURIComponent('alice/temp1') + '/readings',
        { value: 22.5 }, 'alice'
      );
      expect(res.status).toBe(422);
    });
  });

  // ─────────────────────────────────────────────────────────────────
  // GET /api/sensors
  // ─────────────────────────────────────────────────────────────────

  describe('GET /api/sensors', () => {
    beforeEach(async () => {
      await request(port, 'POST', '/api/sensors', {
        name: 'temp1', type: 'temperature', unit: 'celsius', decimals: 2, region: 'us-east',
      }, 'alice');
      await request(port, 'POST', '/api/sensors', {
        name: 'hum1', type: 'humidity', unit: 'pct', decimals: 1, region: 'us-west',
      }, 'bob');
    });

    it('lists all sensors paginated', async () => {
      const res = await request(port, 'GET', '/api/sensors');
      expect(res.status).toBe(200);
      expect(res.body.total).toBe(2);
      expect(res.body.sensors.length).toBe(2);
    });

    it('filters by owner', async () => {
      const res = await request(port, 'GET', '/api/sensors?owner=alice');
      expect(res.body.total).toBe(1);
      expect(res.body.sensors[0].owner).toBe('alice');
    });

    it('filters by type', async () => {
      const res = await request(port, 'GET', '/api/sensors?type=humidity');
      expect(res.body.total).toBe(1);
      expect(res.body.sensors[0].type).toBe('humidity');
    });

    it('filters by region', async () => {
      const res = await request(port, 'GET', '/api/sensors?region=us-east');
      expect(res.body.total).toBe(1);
      expect(res.body.sensors[0].region).toBe('us-east');
    });
  });

  // ─────────────────────────────────────────────────────────────────
  // GET /api/sensors/:id
  // ─────────────────────────────────────────────────────────────────

  describe('GET /api/sensors/:id', () => {
    beforeEach(async () => {
      await request(port, 'POST', '/api/sensors', {
        name: 'temp1', type: 'temperature', unit: 'celsius', decimals: 2, region: 'us-east',
      }, 'alice');
    });

    it('returns sensor + stats', async () => {
      const res = await request(
        port, 'GET',
        '/api/sensors/' + encodeURIComponent('alice/temp1')
      );
      expect(res.status).toBe(200);
      expect(res.body.sensor.sensor_id).toBe('alice/temp1');
      expect(res.body.stats).toBeDefined();
      expect(res.body.stats.sensor_id).toBe('alice/temp1');
    });

    it('returns 404 for unknown sensor', async () => {
      const res = await request(
        port, 'GET',
        '/api/sensors/' + encodeURIComponent('alice/ghost')
      );
      expect(res.status).toBe(404);
    });
  });

  // ─────────────────────────────────────────────────────────────────
  // POST /api/gateways
  // ─────────────────────────────────────────────────────────────────

  describe('POST /api/gateways', () => {
    it('rejects unauthenticated requests', async () => {
      const res = await request(port, 'POST', '/api/gateways', {
        name: 'gw1', region: 'us-east', latitude: 40.7, longitude: -74.0,
      });
      expect(res.status).toBe(401);
    });

    it('registers a gateway owned by the authenticated user', async () => {
      const res = await request(port, 'POST', '/api/gateways', {
        name: 'gw1', region: 'us-east', latitude: 40.7, longitude: -74.0,
      }, 'alice');
      expect(res.status).toBe(201);
      expect(res.body.success).toBe(true);
      expect(res.body.gateway.gateway_id).toBe('alice/gw1');
      expect(res.body.gateway.owner).toBe('alice');
      expect(res.body.gateway.status).toBe('active');
    });

    it('rejects missing name', async () => {
      const res = await request(port, 'POST', '/api/gateways', {
        region: 'us-east', latitude: 40.7, longitude: -74.0,
      }, 'alice');
      expect(res.status).toBe(400);
      expect(res.body.error).toMatch(/name required/);
    });

    it('rejects missing region', async () => {
      const res = await request(port, 'POST', '/api/gateways', {
        name: 'gw1', latitude: 40.7, longitude: -74.0,
      }, 'alice');
      expect(res.status).toBe(400);
      expect(res.body.error).toMatch(/region required/);
    });

    it('rejects invalid coordinates from registry', async () => {
      const res = await request(port, 'POST', '/api/gateways', {
        name: 'gw1', region: 'us-east', latitude: 999, longitude: -74.0,
      }, 'alice');
      expect(res.status).toBe(422);
      expect(res.body.error).toMatch(/latitude/);
    });
  });

  // ─────────────────────────────────────────────────────────────────
  // POST /api/gateways/:id/heartbeat
  // ─────────────────────────────────────────────────────────────────

  describe('POST /api/gateways/:id/heartbeat', () => {
    beforeEach(async () => {
      await request(port, 'POST', '/api/gateways', {
        name: 'gw1', region: 'us-east', latitude: 40.7, longitude: -74.0,
      }, 'alice');
    });

    it('records a heartbeat from the owner', async () => {
      const res = await request(
        port, 'POST',
        '/api/gateways/' + encodeURIComponent('alice/gw1') + '/heartbeat',
        { sensors_connected: 5, packets_relayed_this_epoch: 120 },
        'alice'
      );
      expect(res.status).toBe(200);
      expect(res.body.success).toBe(true);
      expect(res.body.gateway.total_heartbeats).toBe(1);
      expect(res.body.gateway.last_stats.sensors_connected).toBe(5);
    });

    it('rejects heartbeat from non-owner', async () => {
      const res = await request(
        port, 'POST',
        '/api/gateways/' + encodeURIComponent('alice/gw1') + '/heartbeat',
        {},
        'bob'
      );
      expect(res.status).toBe(403);
    });

    it('returns 404 for unknown gateway', async () => {
      const res = await request(
        port, 'POST',
        '/api/gateways/' + encodeURIComponent('alice/ghost') + '/heartbeat',
        {},
        'alice'
      );
      expect(res.status).toBe(404);
    });

    it('rejects heartbeat for retired gateway', async () => {
      await request(
        port, 'POST',
        '/api/gateways/' + encodeURIComponent('alice/gw1') + '/retire',
        {}, 'alice'
      );
      const res = await request(
        port, 'POST',
        '/api/gateways/' + encodeURIComponent('alice/gw1') + '/heartbeat',
        {}, 'alice'
      );
      expect(res.status).toBe(409);
    });
  });

  // ─────────────────────────────────────────────────────────────────
  // POST /api/gateways/:id/retire
  // ─────────────────────────────────────────────────────────────────

  describe('POST /api/gateways/:id/retire', () => {
    beforeEach(async () => {
      await request(port, 'POST', '/api/gateways', {
        name: 'gw1', region: 'us-east', latitude: 40.7, longitude: -74.0,
      }, 'alice');
    });

    it('retires a gateway', async () => {
      const res = await request(
        port, 'POST',
        '/api/gateways/' + encodeURIComponent('alice/gw1') + '/retire',
        {}, 'alice'
      );
      expect(res.status).toBe(200);
      expect(res.body.success).toBe(true);
      expect(res.body.gateway.status).toBe('retired');
    });

    it('rejects retire from non-owner', async () => {
      const res = await request(
        port, 'POST',
        '/api/gateways/' + encodeURIComponent('alice/gw1') + '/retire',
        {}, 'bob'
      );
      expect(res.status).toBe(403);
    });

    it('returns 404 for unknown gateway', async () => {
      const res = await request(
        port, 'POST',
        '/api/gateways/' + encodeURIComponent('alice/ghost') + '/retire',
        {}, 'alice'
      );
      expect(res.status).toBe(404);
    });
  });

  // ─────────────────────────────────────────────────────────────────
  // GET /api/gateways
  // ─────────────────────────────────────────────────────────────────

  describe('GET /api/gateways', () => {
    beforeEach(async () => {
      await request(port, 'POST', '/api/gateways', {
        name: 'gw1', region: 'us-east', latitude: 40.7, longitude: -74.0,
      }, 'alice');
      await request(port, 'POST', '/api/gateways', {
        name: 'gw2', region: 'us-west', latitude: 37.7, longitude: -122.4,
      }, 'bob');
    });

    it('lists all gateways paginated', async () => {
      const res = await request(port, 'GET', '/api/gateways');
      expect(res.status).toBe(200);
      expect(res.body.total).toBe(2);
      expect(res.body.gateways.length).toBe(2);
    });

    it('filters by owner', async () => {
      const res = await request(port, 'GET', '/api/gateways?owner=alice');
      expect(res.body.total).toBe(1);
      expect(res.body.gateways[0].owner).toBe('alice');
    });

    it('filters by region', async () => {
      const res = await request(port, 'GET', '/api/gateways?region=us-west');
      expect(res.body.total).toBe(1);
      expect(res.body.gateways[0].region).toBe('us-west');
    });
  });

  // ─────────────────────────────────────────────────────────────────
  // GET /api/gateways/:id
  // ─────────────────────────────────────────────────────────────────

  describe('GET /api/gateways/:id', () => {
    beforeEach(async () => {
      await request(port, 'POST', '/api/gateways', {
        name: 'gw1', region: 'us-east', latitude: 40.7, longitude: -74.0,
      }, 'alice');
    });

    it('returns gateway + stats', async () => {
      const res = await request(
        port, 'GET',
        '/api/gateways/' + encodeURIComponent('alice/gw1')
      );
      expect(res.status).toBe(200);
      expect(res.body.gateway.gateway_id).toBe('alice/gw1');
      expect(res.body.stats).toBeDefined();
      expect(res.body.stats.gateway_id).toBe('alice/gw1');
    });

    it('returns 404 for unknown gateway', async () => {
      const res = await request(
        port, 'GET',
        '/api/gateways/' + encodeURIComponent('alice/ghost')
      );
      expect(res.status).toBe(404);
    });
  });
});
