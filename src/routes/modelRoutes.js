"use strict";

/**
 * BTCPC Model Registry
 *
 * GET  /api/models/registry        — JSON listing of all supported models + status
 * GET  /api/models/:id             — metadata for one model
 * POST /api/models/fetch           — chain pulls model from HuggingFace itself (admin auth)
 * GET  /api/models/:id/*           — fetch a model file
 *
 * Download routing (most-efficient-first):
 *   1. Active storage hosts that hold the CID → 302 redirect (client downloads
 *      directly from the storage node, zero load on the API server)
 *   2. Local blob store fallback (API server streams the file)
 *
 * transformers.js points at https://btcpc.net/api/models/ and fetches
 * tokenizer.json, onnx/model_q4.onnx, etc. — fully chain-served.
 */

const express = require('express');
const router = express.Router();
const fs = require('fs');
const https = require('https');
const http = require('http');
const path = require('path');
const blobStore = require('../services/blobStore');
const stateStore = require('../chain/stateStore');
const { authenticateToken } = require('../middlewares/auth');
const ledger = require('../services/ledger');

const REGISTRY_PATH = path.resolve(__dirname, '../../data/model-registry.json');

const DEFAULT_DURATION_EPOCHS = 10000;

// Canonical HuggingFace source for each supported model.
const MODEL_CATALOG = {
  'smollm2-360m': {
    hf_repo: 'onnx-community/SmolLM2-360M-Instruct-ONNX',
    files: [
      'config.json', 'generation_config.json', 'tokenizer.json',
      'tokenizer_config.json', 'special_tokens_map.json', 'vocab.json',
      'merges.txt', 'onnx/model_q4.onnx',
    ],
  },
  'qwen2.5-0.5b': {
    hf_repo: 'onnx-community/Qwen2.5-0.5B-Instruct-ONNX',
    files: [
      'config.json', 'generation_config.json', 'tokenizer.json',
      'tokenizer_config.json', 'special_tokens_map.json', 'vocab.json',
      'merges.txt', 'added_tokens.json', 'onnx/model_q4.onnx',
    ],
  },
};

function loadRegistry() {
  try {
    return JSON.parse(fs.readFileSync(REGISTRY_PATH, 'utf8'));
  } catch (_) {
    return {};
  }
}

function saveRegistry(reg) {
  fs.writeFileSync(REGISTRY_PATH, JSON.stringify(reg, null, 2) + '\n');
}

function fetchUrl(url) {
  return new Promise((resolve, reject) => {
    const mod = url.startsWith('https') ? https : http;
    mod.get(url, { headers: { 'User-Agent': 'btcpc-model-fetcher/1.0' } }, (res) => {
      if ([301, 302, 307, 308].includes(res.statusCode)) {
        let loc = res.headers.location;
        if (loc && loc.startsWith('/')) {
          const u = new URL(url);
          loc = u.origin + loc;
        }
        return fetchUrl(loc).then(resolve).catch(reject);
      }
      if (res.statusCode !== 200) return reject(new Error('HTTP ' + res.statusCode));
      const chunks = [];
      res.on('data', (c) => chunks.push(c));
      res.on('end', () => resolve(Buffer.concat(chunks)));
      res.on('error', reject);
    }).on('error', reject);
  });
}

// CORS open — model files must be downloadable by any origin (browser inference)
router.use(function(req, res, next) {
  res.set('Access-Control-Allow-Origin', '*');
  res.set('Access-Control-Allow-Methods', 'GET, HEAD, POST, OPTIONS');
  res.set('Access-Control-Allow-Headers', 'Range, Content-Type, Authorization');
  if (req.method === 'OPTIONS') return res.status(204).end();
  next();
});

/** Pick the best storage host URL for a CID, or null if none available. */
function bestHostUrl(cid) {
  try {
    const hosts = stateStore.getActiveStorageHosts
      ? stateStore.getActiveStorageHosts(stateStore.getChainHeight(), 400)
      : [];
    // Filter to hosts that declared they hold this CID
    const candidates = hosts.filter(function(h) {
      return h.cids && h.cids.includes(cid) && h.endpoint;
    });
    if (candidates.length === 0) return null;
    // Pick a random active host (load-balances across peers over time)
    const host = candidates[Math.floor(Math.random() * candidates.length)];
    return host.endpoint.replace(/\/$/, '') + '/api/blobs/' + cid;
  } catch (_) {
    return null;
  }
}

/**
 * POST /api/models/fetch
 * Chain pulls model files from HuggingFace directly — no client download needed.
 * Body: { model_id: 'smollm2-360m' }  (omit to fetch all pending/unknown models)
 * Auth required. Runs asynchronously; responds immediately with job status.
 */
router.post('/fetch', authenticateToken, express.json(), async function(req, res) {
  const uploader = req.user && req.user.username;
  if (!uploader) return res.status(401).json({ error: 'unauthenticated' });

  const registry = loadRegistry();
  const requested = req.body && req.body.model_id
    ? [req.body.model_id]
    : Object.keys(MODEL_CATALOG);

  const unknown = requested.filter((id) => !MODEL_CATALOG[id]);
  if (unknown.length) {
    return res.status(400).json({
      error: 'unknown model(s)',
      unknown,
      available: Object.keys(MODEL_CATALOG),
    });
  }

  // Respond immediately — fetch runs in background
  res.json({
    status: 'fetching',
    models: requested,
    message: 'Chain is pulling model files from HuggingFace. Poll GET /api/models/registry for status.',
  });

  // Background pull
  (async () => {
    for (const modelId of requested) {
      const catalog = MODEL_CATALOG[modelId];
      const entry = registry[modelId] || { files: {} };
      const files = { ...(entry.files || {}) };
      let changed = false;

      for (const file of catalog.files) {
        if (files[file]) continue; // already on-chain
        const hfUrl = `https://huggingface.co/${catalog.hf_repo}/resolve/main/${file}`;
        let buf;
        try {
          buf = await fetchUrl(hfUrl);
        } catch (err) {
          console.warn(`[model-fetch] ${modelId}/${file} download failed: ${err.message}`);
          continue;
        }
        const result = blobStore.putBlob(buf);
        try {
          const epoch = await ledger.getCurrentEpoch();
          await ledger.recordBlobStoreCommit(uploader, result.cid, result.size, [uploader], DEFAULT_DURATION_EPOCHS, 0, epoch);
        } catch (_) {}
        files[file] = result.cid;
        changed = true;
        console.log(`[model-fetch] ${modelId}/${file} → ${result.cid.slice(0, 12)}... (${result.existed ? 'existed' : 'new'})`);
      }

      if (changed || !entry.status || entry.status === 'pending') {
        const reg2 = loadRegistry(); // re-read in case of concurrent writes
        reg2[modelId] = {
          ...reg2[modelId],
          files,
          status: Object.keys(files).length > 0 ? 'active' : 'pending',
          uploaded_at: new Date().toISOString(),
        };
        saveRegistry(reg2);
      }
    }
    console.log('[model-fetch] Done.');
  })().catch((err) => console.error('[model-fetch] error:', err.message));
});

/** GET /api/models/registry */
router.get('/registry', function(req, res) {
  const registry = loadRegistry();
  // Strip file CIDs from public response — clients only need model metadata + status
  const public_registry = {};
  Object.keys(registry).forEach(function(id) {
    const m = registry[id];
    public_registry[id] = {
      name: m.name,
      description: m.description,
      format: m.format,
      size_mb: m.size_mb,
      status: m.status,
      file_count: Object.keys(m.files || {}).length,
    };
  });
  res.json({ models: public_registry });
});

/** GET /api/models/:id — model metadata (no file CIDs) */
router.get('/:id', function(req, res) {
  const registry = loadRegistry();
  const model = registry[req.params.id];
  if (!model) return res.status(404).json({ error: 'model not in registry', id: req.params.id });
  const { files, ...meta } = model;
  res.json({ ...meta, file_count: Object.keys(files || {}).length });
});

/** GET /api/models/:id/* — serve a model file via best available route */
router.get('/:id/*path', function(req, res) {
  const registry = loadRegistry();
  const model = registry[req.params.id];
  if (!model) return res.status(404).json({ error: 'model not in registry' });
  if (model.status === 'deprecated') return res.status(410).json({ error: 'model deprecated' });
  if (model.status === 'pending') {
    return res.status(503).json({ error: 'model upload in progress — check back shortly', id: req.params.id });
  }

  const filePath = req.params.path;
  const cid = (model.files || {})[filePath];
  if (!cid) return res.status(404).json({ error: 'file not in model manifest', file: filePath });

  // Route 1: redirect to a storage host that has the blob
  const hostUrl = bestHostUrl(cid);
  if (hostUrl) {
    return res.redirect(302, hostUrl);
  }

  // Route 2: serve from local blob store
  if (!blobStore.hasBlob(cid)) {
    return res.status(404).json({ error: 'blob not available on any host yet', cid });
  }

  const stat = blobStore.statBlob(cid);
  const total = stat.size;
  const rangeHeader = req.headers['range'];

  res.set('X-BTCPC-CID', cid);
  res.set('Cache-Control', 'public, max-age=31536000, immutable');
  res.set('Accept-Ranges', 'bytes');

  const ext = path.extname(filePath).toLowerCase();
  const ctMap = { '.json': 'application/json', '.onnx': 'application/octet-stream',
    '.bin': 'application/octet-stream', '.txt': 'text/plain' };
  res.set('Content-Type', ctMap[ext] || 'application/octet-stream');

  if (rangeHeader) {
    const match = /bytes=(\d*)-(\d*)/.exec(rangeHeader);
    const start = match && match[1] ? parseInt(match[1]) : 0;
    const end = match && match[2] ? Math.min(parseInt(match[2]), total - 1) : total - 1;
    const length = end - start + 1;
    res.status(206);
    res.set('Content-Range', `bytes ${start}-${end}/${total}`);
    res.set('Content-Length', String(length));
    return res.end(blobStore.readBlobRange(cid, start, length));
  }

  res.set('Content-Length', String(total));
  fs.createReadStream(blobStore.blobPath(cid)).pipe(res);
});

module.exports = router;
