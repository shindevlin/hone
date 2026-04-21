"use strict";

/**
 * BTCPC IoT Sensor + LoRa Gateway HTTP Routes — v2.15-beta
 * Shin Devlin
 *
 * REST wrapper around sensorRegistry (v2.15-alpha) and
 * loraGatewayRegistry (v2.15-alpha). Follows the serviceRoutes
 * pattern exactly — auth via JWT, HTTP layer enforces input
 * validation + principal matching, registry enforces chain
 * invariants.
 *
 * Sensor ID and gateway ID follow the "<owner>/<name>" slug pattern.
 * The slash means :id must be URL-encoded by the client.
 *
 * Exports two routers:
 *   sensorsRouter  — mounted at /api/sensors
 *   gatewaysRouter — mounted at /api/gateways
 *
 * Route summary:
 *   POST   /api/sensors                    register sensor (auth: owner)
 *   POST   /api/sensors/:id/readings       submit reading (auth: any authenticated user)
 *   POST   /api/sensors/:id/retire         retire sensor (auth: owner)
 *   GET    /api/sensors                    list (public, filterable)
 *   GET    /api/sensors/:id               single sensor + stats
 *
 *   POST   /api/gateways                  register gateway (auth: owner)
 *   POST   /api/gateways/:id/heartbeat    gateway heartbeat (auth: owner)
 *   POST   /api/gateways/:id/retire       retire gateway (auth: owner)
 *   GET    /api/gateways                  list (public, filterable)
 *   GET    /api/gateways/:id             single gateway + stats
 */

const express = require('express');
const crypto = require('crypto');
const { authenticateToken } = require('../middlewares/auth');
const sensorRegistry = require('../services/sensorRegistry');
const gatewayRegistry = require('../services/loraGatewayRegistry');
const blobStore = require('../services/blobStore');
const ledger = require('../services/ledger');
const stateStore = require('../chain/stateStore');

// ─────────────────────────────────────────────────────────────────
// Helpers
// ─────────────────────────────────────────────────────────────────

function decodeId(raw) {
  try {
    return decodeURIComponent(raw);
  } catch (_) {
    return null;
  }
}

function sanitizeString(val, maxLen) {
  if (typeof val !== 'string') return '';
  var trimmed = val.trim();
  if (trimmed.length > maxLen) trimmed = trimmed.slice(0, maxLen);
  return trimmed;
}

function sanitizePagination(pageRaw, limitRaw, maxLimit) {
  var page = parseInt(pageRaw, 10);
  var limit = parseInt(limitRaw, 10);
  if (!Number.isFinite(page) || page < 1) page = 1;
  if (!Number.isFinite(limit) || limit < 1) limit = 20;
  if (limit > maxLimit) limit = maxLimit;
  return { page: page, limit: limit };
}

async function getCurrentEpoch() {
  try {
    var ledger = require('../services/ledger');
    if (ledger && typeof ledger.getCurrentEpoch === 'function') {
      var e = await ledger.getCurrentEpoch();
      if (Number.isFinite(e)) return e;
    }
  } catch (_) {}
  return 0;
}

// ─────────────────────────────────────────────────────────────────
// Sensors router
// ─────────────────────────────────────────────────────────────────

const sensorsRouter = express.Router();

/**
 * POST /api/sensors
 * Register a sensor. The authenticated user becomes the owner.
 * Body: { name, type, unit, decimals, region, lora_gateway?, hardware_model?, firmware_version? }
 * sensor_id is computed server-side as "<owner>/<name>".
 */
sensorsRouter.post('/', async (req, res) => {
  try {
    const owner = (req.user && req.user.username) || (req.body && req.body.account);
    if (!owner) return res.status(401).json({ error: 'unauthenticated' });

    const body = req.body || {};
    const name = sanitizeString(body.name, 63);
    if (!name) return res.status(400).json({ error: 'name required' });

    const sensorId = owner + '/' + name;

    const spec = {
      type: sanitizeString(body.type, 32),
      unit: sanitizeString(body.unit, 32),
      decimals: body.decimals,
      region: sanitizeString(body.region, 128),
    };
    if (!spec.type) return res.status(400).json({ error: 'type required' });
    if (!spec.unit) return res.status(400).json({ error: 'unit required' });
    if (!spec.region) return res.status(400).json({ error: 'region required' });
    if (body.lora_gateway !== undefined) spec.lora_gateway = sanitizeString(body.lora_gateway, 128) || null;
    if (body.hardware_model !== undefined) spec.hardware_model = sanitizeString(body.hardware_model, 128) || null;
    if (body.firmware_version !== undefined) spec.firmware_version = sanitizeString(body.firmware_version, 32) || null;
    if (body.allow_precise_location !== undefined) spec.allow_precise_location = body.allow_precise_location === true;

    const options = { epoch: await getCurrentEpoch() };

    try {
      const record = sensorRegistry.registerSensor(owner, sensorId, spec, options);
      return res.status(201).json({ success: true, sensor: record });
    } catch (err) {
      if (/already/i.test(err.message)) return res.status(409).json({ error: err.message });
      return res.status(422).json({ error: err.message });
    }
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

/**
 * POST /api/sensors/:id/readings
 * Submit a reading for a sensor. Auth: any authenticated user
 * (owners relay directly; gateways relay on behalf of sensors).
 * Body: { value, metadata? }
 */
sensorsRouter.post('/:id/readings', async (req, res) => {
  try {
    const caller = (req.user && req.user.username) || (req.body && req.body.account);
    if (!caller) return res.status(400).json({ error: 'account required (body.account or JWT)' });

    const sensorId = decodeId(req.params.id);
    if (!sensorId) return res.status(400).json({ error: 'invalid sensor id encoding' });

    const body = req.body || {};
    if (body.value === undefined || body.value === null) {
      return res.status(400).json({ error: 'value required' });
    }
    const numeric = Number(body.value);
    if (!Number.isFinite(numeric)) {
      return res.status(400).json({ error: 'value must be a finite number' });
    }

    var metadata = {};
    if (body.metadata && typeof body.metadata === 'object' && !Array.isArray(body.metadata)) {
      if (body.metadata.type !== undefined) metadata.type = sanitizeString(body.metadata.type, 32) || null;
      if (body.metadata.unit !== undefined) metadata.unit = sanitizeString(body.metadata.unit, 32) || null;
      if (body.metadata.source !== undefined) metadata.source = sanitizeString(body.metadata.source, 64) || null;
      if (body.metadata.latitude !== undefined) metadata.latitude = Number(body.metadata.latitude);
      if (body.metadata.longitude !== undefined) metadata.longitude = Number(body.metadata.longitude);
      if (body.metadata.altitude !== undefined) metadata.altitude = Number(body.metadata.altitude);
      if (body.metadata.battery_pct !== undefined) metadata.battery_pct = Number(body.metadata.battery_pct);
      if (body.metadata.signal_strength_dbm !== undefined) metadata.signal_strength_dbm = Number(body.metadata.signal_strength_dbm);
      if (body.metadata.gateway_id !== undefined) metadata.gateway_id = sanitizeString(body.metadata.gateway_id, 128) || null;
    }

    const epoch = await getCurrentEpoch();

    // Optional relay_account — the node that physically received the signal
    var relayAccount = null;
    if (body.relay_account && typeof body.relay_account === 'string') {
      relayAccount = sanitizeString(body.relay_account, 64) || null;
    }
    if (relayAccount) metadata.relay_account = relayAccount;

    // Optional: run requester-side tools to enrich the reading before submission
    // e.g. tools: ['calculator'] to convert units, 'hash' to fingerprint raw data
    let toolTraceHash = null;
    let toolsUsed = [];
    if (body.tools && Array.isArray(body.tools) && body.tools.length > 0) {
      try {
        const { executeTools } = require('../mcp/toolExecutor');
        const toolResult = await executeTools({
          tools: body.tools,
          toolContext: { value: numeric, sensor_id: sensorId, metadata, epoch, ...body.tool_context },
          mcpServers: body.mcp_servers,
        });
        toolTraceHash = toolResult.toolTraceHash;
        toolsUsed = toolResult.toolsUsed;
        // If a tool returned a transformed value, allow overriding (explicit opt-in only)
        if (body.use_tool_value && toolResult.results[0] && toolResult.results[0].output && toolResult.results[0].output.result !== undefined) {
          const transformed = Number(toolResult.results[0].output.result);
          if (Number.isFinite(transformed)) metadata.tool_transformed_value = transformed;
        }
      } catch (_) {}
    }

    try {
      const reading = sensorRegistry.submitReading(sensorId, numeric, { ...metadata, tool_trace_hash: toolTraceHash || undefined, tools_used: toolsUsed.length > 0 ? toolsUsed : undefined }, epoch);
      // witness_count > 1 means a second (or later) gateway observed the same reading
      if (reading.witness_count && reading.witness_count > 1) {
        return res.status(200).json({ success: true, witnessed: true, witness_count: reading.witness_count, reading });
      }
      return res.status(201).json({ success: true, reading: reading });
    } catch (err) {
      if (/duplicate reading/i.test(err.message)) {
        return res.status(200).json({
          success: true,
          duplicate: true,
          sensor_id: sensorId,
          epoch: epoch,
          message: err.message,
        });
      }

      // Auto-register sensor on first reading (self-heal — never 404)
      if (/not found/i.test(err.message)) {
        try {
          const owner = sensorId.split('/')[0];
          const type = (metadata && metadata.type) || 'custom';
          const unit = (metadata && metadata.unit) || 'auto';
          sensorRegistry.registerSensor(owner, sensorId, {
            type, unit, decimals: 2, region: 'auto',
            hardware_model: 'auto-registered',
          });
          const reading = sensorRegistry.submitReading(sensorId, numeric, metadata, epoch);
          return res.status(201).json({ success: true, reading: reading, auto_registered: true });
        } catch (regErr) {
          if (/duplicate reading/i.test(regErr.message)) {
            return res.status(200).json({
              success: true,
              duplicate: true,
              sensor_id: sensorId,
              epoch: epoch,
              message: regErr.message,
            });
          }
          return res.status(422).json({ error: regErr.message });
        }
      }
      return res.status(422).json({ error: err.message });
    }
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

/**
 * POST /api/sensors/:id/retire
 * Retire a sensor. Only the owner can retire.
 */
sensorsRouter.post('/:id/retire', authenticateToken, async (req, res) => {
  try {
    const owner = req.user && req.user.username;
    if (!owner) return res.status(401).json({ error: 'unauthenticated' });

    const sensorId = decodeId(req.params.id);
    if (!sensorId) return res.status(400).json({ error: 'invalid sensor id encoding' });

    const epoch = await getCurrentEpoch();

    try {
      const record = sensorRegistry.retireSensor(owner, sensorId, epoch);
      return res.json({ success: true, sensor: record });
    } catch (err) {
      if (/not found/i.test(err.message)) return res.status(404).json({ error: err.message });
      if (/only the owner/i.test(err.message)) return res.status(403).json({ error: err.message });
      return res.status(422).json({ error: err.message });
    }
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

/**
 * POST /api/sensors/:id/finalize
 * Finalize epoch readings for a sensor: compute median, persist readings
 * as a blob in BTCPC-FS, and record SENSOR_DATA_COMMIT on chain.
 * Auth: owner only (or internal).
 */
sensorsRouter.post('/:id/finalize', authenticateToken, async (req, res) => {
  try {
    const owner = req.user && req.user.username;
    if (!owner) return res.status(401).json({ error: 'unauthenticated' });

    const sensorId = decodeId(req.params.id);
    if (!sensorId) return res.status(400).json({ error: 'invalid sensor id encoding' });

    const sensor = sensorRegistry.getSensor(sensorId);
    if (!sensor) return res.status(404).json({ error: 'sensor not found' });
    if (sensor.owner !== owner) return res.status(403).json({ error: 'only the sensor owner can finalize readings' });

    const body = req.body || {};
    const epoch = body.epoch !== undefined ? parseInt(body.epoch, 10) : await getCurrentEpoch();

    try {
      // 1. Compute median via sensorRegistry finalization
      const finalization = sensorRegistry.finalizeEpochReadings(sensorId, epoch);

      // 2. Serialize finalized readings to JSON and persist as blob
      const serialized = Buffer.from(JSON.stringify(finalization), 'utf8');
      const blobResult = blobStore.putBlob(serialized);

      // 3. Record SENSOR_DATA_COMMIT on chain (non-blocking — swallow errors)
      try {
        await ledger.recordSensorDataCommit(
          sensorId,
          blobResult.cid,
          epoch,
          finalization.reading_count,
          finalization.median
        );
      } catch (commitErr) {
        // Blob persisted; chain entry failed — log but don't fail the HTTP response
        console.error('[btcpc] SENSOR_DATA_COMMIT ledger error:', commitErr.message);
      }

      return res.json({
        success: true,
        finalization: finalization,
        cid: blobResult.cid,
        size: blobResult.size,
        epoch,
      });
    } catch (err) {
      if (/not found/i.test(err.message)) return res.status(404).json({ error: err.message });
      if (/no readings/i.test(err.message)) return res.status(422).json({ error: err.message });
      return res.status(422).json({ error: err.message });
    }
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

/**
 * GET /api/sensors
 * List sensors. Public, paginated, filterable by region/type/owner/status.
 *
 * When ?status=claimable is passed the list is filtered by computed lifecycle
 * status (epoch-gap derived) rather than stored status field — this lets
 * prospective claimants discover abandoned sensors. The response includes
 * last_reading_epoch, last_owner, sensor_type, and region for evaluation.
 */
sensorsRouter.get('/', async (req, res) => {
  const { page, limit } = sanitizePagination(req.query.page, req.query.limit, 200);
  const rawStatus = req.query.status ? sanitizeString(req.query.status, 16) : null;

  // Lifecycle statuses require epoch-gap derivation; stored-status filter
  // would miss records whose stored field has not been updated in place.
  const lifecycleStatuses = ['active', 'dormant', 'claimable'];
  const filterByLifecycle = rawStatus && lifecycleStatuses.indexOf(rawStatus) >= 0;

  const filter = {};
  if (req.query.region) filter.region = sanitizeString(req.query.region, 128);
  if (req.query.type) filter.type = sanitizeString(req.query.type, 32);
  if (req.query.owner) filter.owner = sanitizeString(req.query.owner, 64);
  // Only pass stored-status filter for 'retired' — lifecycle ones filtered below
  if (rawStatus && !filterByLifecycle) filter.status = rawStatus;

  let all = sensorRegistry.getAllSensors(filter);

  if (filterByLifecycle) {
    // Need current epoch to compute gaps
    const currentEpoch = await getCurrentEpoch();
    all = all.filter(function (s) {
      return stateStore.getDeviceStatus(s.sensor_id, currentEpoch) === rawStatus;
    });
  }

  // For claimable discovery, enrich each record with fields prospective
  // claimants need to evaluate the opportunity.
  let sensors;
  if (rawStatus === 'claimable') {
    const currentEpoch = await getCurrentEpoch();
    sensors = all.slice((page - 1) * limit, (page - 1) * limit + limit).map(function (s) {
      return {
        sensor_id: s.sensor_id,
        last_reading_epoch: s.last_reading_epoch,
        last_owner: s.owner,
        sensor_type: s.type,
        region: s.region,
        status: 'claimable',
        current_epoch: currentEpoch,
        epochs_silent: currentEpoch - (s.last_reading_epoch || 0),
      };
    });
  } else {
    const start = (page - 1) * limit;
    sensors = all.slice(start, start + limit);
  }

  const total = all.length;
  res.json({ sensors, page, limit, total });
});

/**
 * GET /api/sensors/:id
 * Get a single sensor + its stats. Includes computed lifecycle `status`
 * derived from epoch-gap via stateStore.getDeviceStatus().
 */
sensorsRouter.get('/:id', async (req, res) => {
  const sensorId = decodeId(req.params.id);
  if (!sensorId) return res.status(400).json({ error: 'invalid sensor id encoding' });
  const sensor = sensorRegistry.getSensor(sensorId);
  if (!sensor) return res.status(404).json({ error: 'sensor not found' });
  const currentEpoch = await getCurrentEpoch();
  const stats = sensorRegistry.getSensorStats(sensorId, currentEpoch);
  // Computed lifecycle status overrides the stored status field so callers
  // always see the epoch-accurate state even if the record was not mutated.
  const computedStatus = stateStore.getDeviceStatus(sensorId, currentEpoch);
  res.json({ sensor: Object.assign({}, sensor, { status: computedStatus }), stats, current_epoch: currentEpoch });
});

// ─────────────────────────────────────────────────────────────────
// Gateways router
// ─────────────────────────────────────────────────────────────────

const gatewaysRouter = express.Router();

/**
 * POST /api/gateways
 * Register a LoRa gateway. The authenticated user becomes the owner.
 * Body: { name, region, latitude, longitude, antenna_gain_dbi?, hardware_model?, firmware_version?, max_sensors? }
 * gateway_id is computed server-side as "<owner>/<name>".
 */
gatewaysRouter.post('/', async (req, res) => {
  try {
    const owner = (req.user && req.user.username) || (req.body && req.body.account);
    if (!owner) return res.status(400).json({ error: 'account required' });

    const body = req.body || {};
    const name = sanitizeString(body.name, 63);
    if (!name) return res.status(400).json({ error: 'name required' });

    const gatewayId = owner + '/' + name;

    const spec = {
      region: sanitizeString(body.region, 128),
      latitude: body.latitude,
      longitude: body.longitude,
    };
    if (!spec.region) return res.status(400).json({ error: 'region required' });
    if (body.antenna_gain_dbi !== undefined) spec.antenna_gain_dbi = Number(body.antenna_gain_dbi);
    if (body.hardware_model !== undefined) spec.hardware_model = sanitizeString(body.hardware_model, 128) || null;
    if (body.firmware_version !== undefined) spec.firmware_version = sanitizeString(body.firmware_version, 32) || null;
    if (body.max_sensors !== undefined) {
      const ms = parseInt(body.max_sensors, 10);
      if (!Number.isFinite(ms) || ms < 1 || ms > 10000) {
        return res.status(400).json({ error: 'max_sensors must be 1-10000' });
      }
      spec.max_sensors = ms;
    }

    const options = { epoch: await getCurrentEpoch() };

    try {
      const record = gatewayRegistry.registerGateway(owner, gatewayId, spec, options);
      return res.status(201).json({ success: true, gateway: record });
    } catch (err) {
      if (/already/i.test(err.message)) return res.status(409).json({ error: err.message });
      return res.status(422).json({ error: err.message });
    }
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

/**
 * POST /api/gateways/:id/heartbeat
 * Gateway reports it is online. Auth: owner only.
 * Body: { sensors_connected?, packets_relayed_this_epoch?, uptime_seconds?, battery_pct? }
 */
gatewaysRouter.post('/:id/heartbeat', async (req, res) => {
  try {
    const owner = (req.user && req.user.username) || (req.body && req.body.account);
    if (!owner) return res.status(400).json({ error: 'account required' });

    const gatewayId = decodeId(req.params.id);
    if (!gatewayId) return res.status(400).json({ error: 'invalid gateway id encoding' });

    const existing = gatewayRegistry.getGateway(gatewayId);
    if (!existing) return res.status(404).json({ error: 'gateway not found' });
    if (existing.owner !== owner) {
      return res.status(403).json({ error: 'only the owner can send heartbeats for this gateway' });
    }

    const body = req.body || {};
    const stats = {};
    if (body.sensors_connected !== undefined) stats.sensors_connected = parseInt(body.sensors_connected, 10) || 0;
    if (body.packets_relayed_this_epoch !== undefined) stats.packets_relayed_this_epoch = parseInt(body.packets_relayed_this_epoch, 10) || 0;
    if (body.uptime_seconds !== undefined) stats.uptime_seconds = Number(body.uptime_seconds) || null;
    if (body.battery_pct !== undefined) stats.battery_pct = Number(body.battery_pct);

    const epoch = await getCurrentEpoch();

    try {
      const record = gatewayRegistry.heartbeat(gatewayId, stats, epoch);
      return res.json({ success: true, gateway: record });
    } catch (err) {
      if (/not found/i.test(err.message)) return res.status(404).json({ error: err.message });
      if (/retired/i.test(err.message)) return res.status(409).json({ error: err.message });
      return res.status(422).json({ error: err.message });
    }
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

/**
 * POST /api/gateways/:id/retire
 * Retire a gateway. Only the owner can retire.
 */
gatewaysRouter.post('/:id/retire', authenticateToken, async (req, res) => {
  try {
    const owner = req.user && req.user.username;
    if (!owner) return res.status(401).json({ error: 'unauthenticated' });

    const gatewayId = decodeId(req.params.id);
    if (!gatewayId) return res.status(400).json({ error: 'invalid gateway id encoding' });

    const epoch = await getCurrentEpoch();

    try {
      const record = gatewayRegistry.retireGateway(owner, gatewayId, epoch);
      return res.json({ success: true, gateway: record });
    } catch (err) {
      if (/not found/i.test(err.message)) return res.status(404).json({ error: err.message });
      if (/only the owner/i.test(err.message)) return res.status(403).json({ error: err.message });
      return res.status(422).json({ error: err.message });
    }
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

/**
 * GET /api/gateways
 * List gateways. Public, paginated, filterable by region/owner/status.
 */
gatewaysRouter.get('/', (req, res) => {
  const { page, limit } = sanitizePagination(req.query.page, req.query.limit, 200);
  const filter = {};
  if (req.query.region) filter.region = sanitizeString(req.query.region, 128);
  if (req.query.owner) filter.owner = sanitizeString(req.query.owner, 64);
  if (req.query.status) filter.status = sanitizeString(req.query.status, 16);
  const all = gatewayRegistry.getAllGateways(filter);
  const total = all.length;
  const start = (page - 1) * limit;
  const gateways = all.slice(start, start + limit);
  res.json({ gateways, page, limit, total });
});

/**
 * GET /api/gateways/:id
 * Get a single gateway + its stats.
 */
gatewaysRouter.get('/:id', async (req, res) => {
  const gatewayId = decodeId(req.params.id);
  if (!gatewayId) return res.status(400).json({ error: 'invalid gateway id encoding' });
  const gateway = gatewayRegistry.getGateway(gatewayId);
  if (!gateway) return res.status(404).json({ error: 'gateway not found' });
  const currentEpoch = await getCurrentEpoch();
  const stats = gatewayRegistry.getGatewayStats(gatewayId, currentEpoch);
  res.json({ gateway, stats, current_epoch: currentEpoch });
});

// ─────────────────────────────────────────────────────────────────
// Devices router — IoT device key registration (v3.2)
// ─────────────────────────────────────────────────────────────────

const devicesRouter = express.Router();

/**
 * POST /api/devices/register
 * Register an IoT device's public key on chain.
 * The owner authorizes with their posting key (cannot move funds).
 *
 * Body: { owner, device_id, device_pubkey, posting_key_sig }
 *   owner          — BTCPC account name
 *   device_id      — "<owner>/<device-name>" (e.g. "josh/flipper-abc123")
 *   device_pubkey  — hex-encoded compressed secp256k1 public key of the device
 *   posting_key_sig — owner's posting-key signature over device_id + device_pubkey
 */
devicesRouter.post('/register', async (req, res) => {
  try {
    const body = req.body || {};

    const owner = sanitizeString(body.owner, 64);
    if (!owner) return res.status(400).json({ error: 'owner required' });

    const deviceId = sanitizeString(body.device_id, 128);
    if (!deviceId) return res.status(400).json({ error: 'device_id required' });

    // device_id must be "<owner>/<device-name>"
    if (!deviceId.startsWith(owner + '/')) {
      return res.status(400).json({ error: 'device_id must start with owner/ (e.g. ' + owner + '/flipper-abc123)' });
    }

    const devicePubkey = sanitizeString(body.device_pubkey, 256);
    if (!devicePubkey) return res.status(400).json({ error: 'device_pubkey required' });

    const postingKeySig = sanitizeString(body.posting_key_sig, 512);
    if (!postingKeySig) return res.status(400).json({ error: 'posting_key_sig required' });

    // Verify the posting key signature — the owner must have a posting key registered
    const ownerAccount = stateStore.getAccount(owner);
    if (!ownerAccount) {
      return res.status(404).json({ error: 'owner account not found on chain' });
    }
    const postingPubkey = ownerAccount.public_keys && ownerAccount.public_keys.posting;
    if (!postingPubkey) {
      return res.status(422).json({ error: 'owner has no registered posting key' });
    }

    // Verify: posting key signed over (device_id + device_pubkey)
    var sigOk = false;
    try {
      // secp256k1 verify via Node.js crypto (DER or raw hex sig)
      const verify = crypto.createVerify('SHA256');
      verify.update(Buffer.from(deviceId + devicePubkey, 'utf8'));
      sigOk = verify.verify(
        { key: Buffer.from(postingPubkey, 'hex'), format: 'der', type: 'spki' },
        Buffer.from(postingKeySig, 'hex')
      );
    } catch (_) {
      // Sig verification failed or malformed key — treat as invalid
      sigOk = false;
    }

    if (!sigOk) {
      return res.status(403).json({ error: 'posting_key_sig verification failed' });
    }

    const epoch = await getCurrentEpoch();

    const entry = ledger.recordDeviceKeyRegister(owner, deviceId, devicePubkey, postingKeySig, epoch);

    return res.status(201).json({
      success: true,
      device: {
        device_id: deviceId,
        owner: owner,
        device_pubkey: devicePubkey,
        registered_epoch: epoch,
      },
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

/**
 * POST /api/devices/:id/rotate-key
 * Rotate the device public key for an IoT device. The current owner
 * (post-claim) authorizes the rotation with their posting key.
 *
 * Path param: :id — URL-encoded "<owner>/<device-name>"
 * Body: { owner, new_device_pubkey, posting_key_sig }
 *   owner            — BTCPC account that currently owns the device
 *   new_device_pubkey — hex-encoded new compressed secp256k1 public key
 *   posting_key_sig  — owner's posting-key signature over (device_id + new_device_pubkey)
 *
 * On success, records a DEVICE_KEY_REGISTER entry on chain with the new key,
 * superseding any previous registration for this device_id.
 */
devicesRouter.post('/:id/rotate-key', async (req, res) => {
  try {
    const deviceId = decodeId(req.params.id);
    if (!deviceId) return res.status(400).json({ error: 'invalid device id encoding' });

    const body = req.body || {};

    const owner = sanitizeString(body.owner, 64);
    if (!owner) return res.status(400).json({ error: 'owner required' });

    const newDevicePubkey = sanitizeString(body.new_device_pubkey, 256);
    if (!newDevicePubkey) return res.status(400).json({ error: 'new_device_pubkey required' });

    const postingKeySig = sanitizeString(body.posting_key_sig, 512);
    if (!postingKeySig) return res.status(400).json({ error: 'posting_key_sig required' });

    // The device must already exist and be owned by `owner`
    const existingDevice = stateStore.getDeviceKey(deviceId);
    if (!existingDevice) {
      return res.status(404).json({ error: 'device not found — register it first via /api/devices/register' });
    }
    if (existingDevice.owner !== owner) {
      return res.status(403).json({ error: 'only the current owner can rotate the device key' });
    }

    // Verify posting key signature
    const ownerAccount = stateStore.getAccount(owner);
    if (!ownerAccount) {
      return res.status(404).json({ error: 'owner account not found on chain' });
    }
    const postingPubkey = ownerAccount.public_keys && ownerAccount.public_keys.posting;
    if (!postingPubkey) {
      return res.status(422).json({ error: 'owner has no registered posting key' });
    }

    var sigOk = false;
    try {
      const verify = crypto.createVerify('SHA256');
      verify.update(Buffer.from(deviceId + newDevicePubkey, 'utf8'));
      sigOk = verify.verify(
        { key: Buffer.from(postingPubkey, 'hex'), format: 'der', type: 'spki' },
        Buffer.from(postingKeySig, 'hex')
      );
    } catch (_) {
      sigOk = false;
    }

    if (!sigOk) {
      return res.status(403).json({ error: 'posting_key_sig verification failed' });
    }

    const epoch = await getCurrentEpoch();

    // Record the new key on chain — supersedes the previous registration
    ledger.recordDeviceKeyRegister(owner, deviceId, newDevicePubkey, postingKeySig, epoch);

    return res.json({
      success: true,
      device: {
        device_id: deviceId,
        owner: owner,
        device_pubkey: newDevicePubkey,
        registered_epoch: epoch,
        rotated: true,
      },
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

// ─────────────────────────────────────────────────────────────────
// Device Stake & Rent routes (v3.5)
// ─────────────────────────────────────────────────────────────────

var SLOT_MULTIPLIERS = [1.85, 1.60, 1.38, 1.19, 1.03, 0.89, 0.77, 0.67, 0.58, 0.50];

function computeRentFloor(device30dayEarnings) {
  var monthlyFloor = Math.max(device30dayEarnings * 0.07, 50);
  return monthlyFloor / 86400; // per-epoch floor (86400 epochs/month @ 30s)
}

function resolveOwner(deviceId) {
  var deviceRecord = stateStore.getDeviceKey(deviceId);
  var sensorRecord = stateStore.getSensor ? stateStore.getSensor(deviceId) : null;
  return (deviceRecord && deviceRecord.owner) ||
         (sensorRecord && sensorRecord.owner) ||
         (deviceId.includes('/') ? deviceId.split('/')[0] : null);
}

/**
 * POST /api/devices/:id/stake
 * Stake on a device to earn yield.
 * Body: { staker, stake_amount, rent_bid, rent_mode }
 */
devicesRouter.post('/:id/stake', async (req, res) => {
  try {
    const deviceId = decodeId(req.params.id);
    if (!deviceId) return res.status(400).json({ error: 'invalid device id encoding' });

    const body = req.body || {};
    const staker = sanitizeString(body.staker, 64);
    if (!staker) return res.status(400).json({ error: 'staker required' });

    const stakeAmount = parseFloat(body.stake_amount);
    if (!Number.isFinite(stakeAmount) || stakeAmount <= 0) {
      return res.status(400).json({ error: 'stake_amount must be a positive number' });
    }

    const rentBid = parseFloat(body.rent_bid);
    if (!Number.isFinite(rentBid) || rentBid < 0) {
      return res.status(400).json({ error: 'rent_bid must be a non-negative number' });
    }

    const rentMode = body.rent_mode === 'stake' ? 'stake' : 'earnings';

    const owner = resolveOwner(deviceId);
    if (!owner) {
      return res.status(404).json({ error: 'device not found — register it first via /api/devices/register or /api/sensors' });
    }

    // Compute rent floor
    const pool = stateStore.getDeviceStakerPool(deviceId);
    const rentFloor = computeRentFloor(0); // no earnings data available in HTTP layer; floor = 50 BTCPC/month
    if (rentBid < rentFloor) {
      return res.status(400).json({ error: 'rent_bid below floor', floor_per_epoch: rentFloor });
    }

    // Check balance
    const stakerBalance = stateStore.getBalance(staker, 'BTCPC');
    if (stakerBalance < stakeAmount) {
      return res.status(422).json({ error: 'insufficient balance', balance: stakerBalance, required: stakeAmount });
    }

    // If pool is full (10), require rent_bid > lowest current rent_bid to bump
    if (pool.length >= 10) {
      const lowest = pool[pool.length - 1];
      if (rentBid <= lowest.rent_bid || (rentBid === lowest.rent_bid && stakeAmount <= lowest.stake_amount)) {
        return res.status(409).json({
          error: 'pool is full; rent_bid must exceed current slot 10 rent_bid to claim a slot',
          current_slot10_rent_bid: lowest.rent_bid,
        });
      }
    }

    const epoch = await getCurrentEpoch();
    const tributeAmount = (staker !== owner) ? parseFloat((stakeAmount * 0.01).toFixed(10)) : 0;

    const entry = {
      type: 'DEVICE_YIELD_STAKE',
      from: staker,
      to: owner,
      amount: stakeAmount,
      token: 'BTCPC',
      epoch: epoch,
      timestamp: Date.now(),
      yield_stake_data: {
        device_id: deviceId,
        staker: staker,
        stake_amount: stakeAmount,
        rent_bid: rentBid,
        rent_mode: rentMode,
        owner: owner,
        entry_epoch: epoch,
      },
      memo: 'device_yield_stake:' + deviceId,
    };

    try {
      stateStore.applyEntry(entry);
      try {
        const ledgerMod = require('../services/ledger');
        if (typeof ledgerMod.recordEntry === 'function') await ledgerMod.recordEntry(entry);
      } catch (_) {}
    } catch (err) {
      return res.status(422).json({ error: err.message });
    }

    const newPool = stateStore.getDeviceStakerPool(deviceId);
    const myRecord = newPool.find(function (r) { return r.staker === staker; });

    return res.status(201).json({
      success: true,
      device_id: deviceId,
      staker: staker,
      owner: owner,
      stake_amount: stakeAmount,
      tribute_paid: tributeAmount,
      rent_bid: rentBid,
      rent_mode: rentMode,
      slot: myRecord ? myRecord.slot : null,
      epoch: epoch,
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

/**
 * POST /api/devices/:id/unstake
 * Withdraw stake. Returns remaining stake_amount.
 * Body: { staker }
 */
devicesRouter.post('/:id/unstake', async (req, res) => {
  try {
    const deviceId = decodeId(req.params.id);
    if (!deviceId) return res.status(400).json({ error: 'invalid device id encoding' });

    const body = req.body || {};
    const staker = sanitizeString(body.staker, 64);
    if (!staker) return res.status(400).json({ error: 'staker required' });

    const pool = stateStore.getDeviceStakerPool(deviceId);
    const record = pool.find(function (r) { return r.staker === staker; });
    if (!record) {
      return res.status(404).json({ error: 'no active stake for this staker on this device' });
    }

    const epoch = await getCurrentEpoch();
    const returnedAmount = record.stake_amount;

    const entry = {
      type: 'DEVICE_YIELD_UNSTAKE',
      from: staker,
      to: staker,
      amount: returnedAmount,
      token: 'BTCPC',
      epoch: epoch,
      timestamp: Date.now(),
      yield_stake_data: { device_id: deviceId, staker: staker },
      memo: 'device_yield_unstake:' + deviceId,
    };

    try {
      stateStore.applyEntry(entry);
      try {
        const ledgerMod = require('../services/ledger');
        if (typeof ledgerMod.recordEntry === 'function') await ledgerMod.recordEntry(entry);
      } catch (_) {}
    } catch (err) {
      return res.status(422).json({ error: err.message });
    }

    return res.json({ success: true, device_id: deviceId, staker: staker, returned_amount: returnedAmount, epoch: epoch });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

/**
 * POST /api/devices/:id/rent-mode
 * Change rent payment mode.
 * Body: { staker, rent_mode }
 */
devicesRouter.post('/:id/rent-mode', async (req, res) => {
  try {
    const deviceId = decodeId(req.params.id);
    if (!deviceId) return res.status(400).json({ error: 'invalid device id encoding' });

    const body = req.body || {};
    const staker = sanitizeString(body.staker, 64);
    if (!staker) return res.status(400).json({ error: 'staker required' });
    const rentMode = body.rent_mode === 'stake' ? 'stake' : 'earnings';

    const pool = stateStore.getDeviceStakerPool(deviceId);
    if (!pool.find(function (r) { return r.staker === staker; })) {
      return res.status(404).json({ error: 'no active stake for this staker on this device' });
    }

    const epoch = await getCurrentEpoch();
    const entry = {
      type: 'DEVICE_YIELD_RENT_MODE',
      from: staker,
      to: staker,
      amount: 0,
      token: 'BTCPC',
      epoch: epoch,
      timestamp: Date.now(),
      yield_stake_data: { device_id: deviceId, staker: staker, rent_mode: rentMode },
      memo: 'device_rent_mode:' + deviceId,
    };

    stateStore.applyEntry(entry);
    try {
      const ledgerMod = require('../services/ledger');
      if (typeof ledgerMod.recordEntry === 'function') await ledgerMod.recordEntry(entry);
    } catch (_) {}

    return res.json({ success: true, device_id: deviceId, staker: staker, rent_mode: rentMode });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

/**
 * POST /api/devices/:id/auto-stake
 * Set auto-stake percentage (owner only).
 * Body: { owner, auto_stake_pct }
 */
devicesRouter.post('/:id/auto-stake', async (req, res) => {
  try {
    const deviceId = decodeId(req.params.id);
    if (!deviceId) return res.status(400).json({ error: 'invalid device id encoding' });

    const body = req.body || {};
    const owner = sanitizeString(body.owner, 64);
    if (!owner) return res.status(400).json({ error: 'owner required' });

    const resolvedOwner = resolveOwner(deviceId);
    if (resolvedOwner && resolvedOwner !== owner) {
      return res.status(403).json({ error: 'only the device owner can set auto-stake' });
    }

    const pct = Math.max(0, Math.min(50, parseFloat(body.auto_stake_pct) || 0));

    const epoch = await getCurrentEpoch();
    const entry = {
      type: 'DEVICE_YIELD_AUTO_STAKE',
      from: owner,
      to: owner,
      amount: 0,
      token: 'BTCPC',
      epoch: epoch,
      timestamp: Date.now(),
      yield_stake_data: { device_id: deviceId, owner: owner, pct: pct },
      memo: 'device_auto_stake:' + deviceId,
    };

    stateStore.applyEntry(entry);
    try {
      const ledgerMod = require('../services/ledger');
      if (typeof ledgerMod.recordEntry === 'function') await ledgerMod.recordEntry(entry);
    } catch (_) {}

    return res.json({ success: true, device_id: deviceId, owner: owner, auto_stake_pct: pct });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

/**
 * GET /api/devices/:id/stakers
 * Get staker pool for a device. Public.
 */
devicesRouter.get('/:id/stakers', async (req, res) => {
  try {
    const deviceId = decodeId(req.params.id);
    if (!deviceId) return res.status(400).json({ error: 'invalid device id encoding' });

    const pool = stateStore.getDeviceStakerPool(deviceId);
    const SMULT = SLOT_MULTIPLIERS;
    const rentFloor = computeRentFloor(0);

    // Compute total weighted for yield estimates
    const totalWeighted = pool.reduce(function (s, r) {
      return s + (SMULT[(r.slot - 1)] || 0) * r.stake_amount;
    }, 0);

    // Build 10-slot array
    var slots = [];
    for (var i = 1; i <= 10; i++) {
      var record = pool.find(function (r) { return r.slot === i; });
      if (record) {
        var mult = SMULT[i - 1];
        var stakerYieldPct = totalWeighted > 0 ? (mult * record.stake_amount / totalWeighted) : 0;
        // Staker pool = 20% of device epoch earn (approximate using rent_bid × 30 as monthly proxy)
        var estMonthlyYield = stakerYieldPct * (record.rent_bid * 86400 * 0.20 / 0.07); // rough
        var estMonthlyRent = record.rent_bid * 86400;
        slots.push({
          slot: i,
          staker: record.staker,
          stake_amount: record.stake_amount,
          rent_bid: record.rent_bid,
          rent_mode: record.rent_mode,
          entry_epoch: record.entry_epoch,
          rent_paid_total: record.rent_paid_total,
          est_monthly_yield: parseFloat(estMonthlyYield.toFixed(4)),
          est_monthly_rent: parseFloat(estMonthlyRent.toFixed(4)),
          est_monthly_net: parseFloat((estMonthlyYield - estMonthlyRent).toFixed(4)),
        });
      } else {
        slots.push({ slot: i, staker: null, available: true });
      }
    }

    return res.json({
      device_id: deviceId,
      slots: slots,
      staker_count: pool.length,
      floor_rent_per_epoch: rentFloor,
      device_30day_earnings: 0, // requires live chain data
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

/**
 * GET /api/devices/staked?account=:name
 * Returns all devices where account is a staker.
 */
devicesRouter.get('/staked', async (req, res) => {
  try {
    const account = req.query.account ? sanitizeString(req.query.account, 64) : null;
    if (!account) return res.status(400).json({ error: 'account query param required' });

    const devices = stateStore.getStakedDevicesForAccount(account);
    return res.json({ account: account, devices: devices, count: devices.length });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

// Legacy yield-stake endpoints (kept for backward compat)
devicesRouter.post('/:id/yield-stake', async (req, res) => {
  req.body = req.body || {};
  if (!req.body.rent_bid) req.body.rent_bid = computeRentFloor(0);
  return devicesRouter.stack.find(function (l) { return l.route && l.route.path === '/:id/stake'; })
    ? res.status(308).json({ redirect: req.path.replace('yield-stake', 'stake'), message: 'use /stake instead' })
    : res.status(410).json({ error: 'use POST /api/devices/:id/stake' });
});

devicesRouter.post('/:id/yield-unstake', async (req, res) => {
  res.status(308).json({ redirect: req.path.replace('yield-unstake', 'unstake'), message: 'use /unstake instead' });
});

devicesRouter.get('/:id/yield-stake', async (req, res) => {
  const deviceId = decodeId(req.params.id);
  if (!deviceId) return res.status(400).json({ error: 'invalid device id encoding' });
  const pool = stateStore.getDeviceStakerPool(deviceId);
  res.json({ device_id: deviceId, pool_size: pool.length, stakers: pool });
});

devicesRouter.get('/yield-stakes', async (req, res) => {
  const stakerAccount = req.query.staker ? sanitizeString(req.query.staker, 64) : null;
  let stakes = stakerAccount
    ? stateStore.getStakedDevicesForAccount(stakerAccount)
    : stateStore.getAllDeviceYieldStakes();
  return res.json({ stakes: stakes, count: stakes.length });
});

module.exports = { sensorsRouter, gatewaysRouter, devicesRouter };
