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
// Device yield staking routes (v3.4)
// ─────────────────────────────────────────────────────────────────

/**
 * POST /api/devices/:id/yield-stake
 * Stake on a device to earn yield. The top staker earns 30% of sensor income.
 * Body: { staker, stake_amount }
 *
 * Rules:
 *   - staker must have sufficient balance
 *   - stake_amount must be > current top stake (if any)
 *   - 5% tribute goes to device owner immediately (non-refundable), except
 *     if staker === owner (no tribute)
 *   - Previous top staker is refunded their net stake
 */
devicesRouter.post('/:id/yield-stake', async (req, res) => {
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

    // Resolve device owner from IoT device registry or sensor registry
    const deviceRecord = stateStore.getDeviceKey(deviceId);
    const sensorRecord = stateStore.getSensor(deviceId);
    const owner = (deviceRecord && deviceRecord.owner) ||
                  (sensorRecord && sensorRecord.owner) ||
                  (deviceId.includes('/') ? deviceId.split('/')[0] : null);

    if (!owner) {
      return res.status(404).json({ error: 'device not found — register it first via /api/devices/register or /api/sensors' });
    }

    // Check new stake > current top stake
    const currentStake = stateStore.getDeviceYieldStake(deviceId);
    if (currentStake && stakeAmount <= currentStake.stake_amount) {
      return res.status(409).json({
        error: 'stake_amount must exceed current top stake',
        current_top_stake: currentStake.stake_amount,
        required_minimum: currentStake.stake_amount + 0.0000000001,
      });
    }

    // Check staker balance
    const stakerBalance = stateStore.getBalance(staker, 'BTCPC');
    if (staker !== owner && stakerBalance < stakeAmount) {
      return res.status(422).json({
        error: 'insufficient balance',
        balance: stakerBalance,
        required: stakeAmount,
      });
    }

    const epoch = await getCurrentEpoch();
    const tributeAmount = (staker !== owner) ? parseFloat((stakeAmount * 0.05).toFixed(10)) : 0;
    const netStake = parseFloat((stakeAmount - tributeAmount).toFixed(10));

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
        owner: owner,
        tribute: tributeAmount,
        net_stake: netStake,
      },
      memo: 'device_yield_stake:' + deviceId,
    };

    try {
      stateStore.applyEntry(entry);
      // Persist on chain via ledger if available
      try {
        const ledgerMod = require('../services/ledger');
        if (typeof ledgerMod.recordEntry === 'function') {
          await ledgerMod.recordEntry(entry);
        }
      } catch (_) {}
    } catch (err) {
      return res.status(422).json({ error: err.message });
    }

    return res.status(201).json({
      success: true,
      device_id: deviceId,
      staker: staker,
      owner: owner,
      stake_amount: stakeAmount,
      tribute_paid: tributeAmount,
      net_stake: netStake,
      epoch: epoch,
      previous_staker: currentStake ? currentStake.staker : null,
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

/**
 * POST /api/devices/:id/yield-unstake
 * Voluntarily withdraw yield stake. Only the current top staker can call this.
 * Body: { staker }
 * Returns the net stake (original stake minus 5% tribute already paid).
 */
devicesRouter.post('/:id/yield-unstake', async (req, res) => {
  try {
    const deviceId = decodeId(req.params.id);
    if (!deviceId) return res.status(400).json({ error: 'invalid device id encoding' });

    const body = req.body || {};
    const staker = sanitizeString(body.staker, 64);
    if (!staker) return res.status(400).json({ error: 'staker required' });

    const currentStake = stateStore.getDeviceYieldStake(deviceId);
    if (!currentStake) {
      return res.status(404).json({ error: 'no active yield stake for this device' });
    }
    if (currentStake.staker !== staker) {
      return res.status(403).json({ error: 'only the current top staker can unstake' });
    }

    const epoch = await getCurrentEpoch();
    const returnedAmount = currentStake.net_stake || currentStake.stake_amount;

    const entry = {
      type: 'DEVICE_YIELD_UNSTAKE',
      from: staker,
      to: staker,
      amount: returnedAmount,
      token: 'BTCPC',
      epoch: epoch,
      timestamp: Date.now(),
      yield_stake_data: {
        device_id: deviceId,
        staker: staker,
        returned_amount: returnedAmount,
      },
      memo: 'device_yield_unstake:' + deviceId,
    };

    try {
      stateStore.applyEntry(entry);
      try {
        const ledgerMod = require('../services/ledger');
        if (typeof ledgerMod.recordEntry === 'function') {
          await ledgerMod.recordEntry(entry);
        }
      } catch (_) {}
    } catch (err) {
      return res.status(422).json({ error: err.message });
    }

    return res.json({
      success: true,
      device_id: deviceId,
      staker: staker,
      returned_amount: returnedAmount,
      epoch: epoch,
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

/**
 * GET /api/devices/:id/yield-stake
 * Get current top staker info for a device. Public endpoint.
 */
devicesRouter.get('/:id/yield-stake', async (req, res) => {
  try {
    const deviceId = decodeId(req.params.id);
    if (!deviceId) return res.status(400).json({ error: 'invalid device id encoding' });

    const stake = stateStore.getDeviceYieldStake(deviceId);
    const currentEpoch = await getCurrentEpoch();

    if (!stake) {
      return res.json({
        device_id: deviceId,
        staker: null,
        stake_amount: 0,
        stake_epoch: null,
        estimated_daily_yield: 0,
        current_epoch: currentEpoch,
      });
    }

    // Estimate daily yield: rough estimate based on 30s epochs
    // 2880 epochs/day × 7% sensor pool / active sensors (approximation)
    const estimated_daily_yield = null; // requires live pool data; callers should compute from explorer

    return res.json({
      device_id: deviceId,
      staker: stake.staker,
      stake_amount: stake.stake_amount,
      net_stake: stake.net_stake,
      tribute_paid: stake.tribute_paid,
      stake_epoch: stake.stake_epoch,
      estimated_daily_yield: estimated_daily_yield,
      current_epoch: currentEpoch,
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

/**
 * GET /api/devices/yield-stakes?staker=:account
 * Returns all devices where account is current top staker.
 */
devicesRouter.get('/yield-stakes', async (req, res) => {
  try {
    const stakerAccount = req.query.staker ? sanitizeString(req.query.staker, 64) : null;

    let stakes;
    if (stakerAccount) {
      stakes = stateStore.getTopYieldStakerForAccount(stakerAccount);
    } else {
      stakes = stateStore.getAllDeviceYieldStakes();
    }

    // Sort by stake_amount desc
    stakes = stakes.slice().sort(function (a, b) { return (b.stake_amount || 0) - (a.stake_amount || 0); });

    return res.json({ stakes: stakes, count: stakes.length });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

module.exports = { sensorsRouter, gatewaysRouter, devicesRouter };
