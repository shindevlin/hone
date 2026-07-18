"use strict";

/**
 * HONE Model Registry
 *
 * GET  /api/models/registry        — all models: built-in + community, with source flag
 * GET  /api/models/:id             — metadata for one model
 * POST /api/models/upload          — register a community model and earn royalties on inference
 * POST /api/models/fetch           — chain pulls model from HuggingFace itself (admin auth)
 * GET  /api/models/:id/*           — fetch a model file
 *
 * Download routing (most-efficient-first):
 *   1. Active storage hosts that hold the CID → 302 redirect (client downloads
 *      directly from the storage node, zero load on the API server)
 *   2. Local blob store fallback (API server streams the file)
 *
 * transformers.js points at https://hone.net/api/models/ and fetches
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
// hone-phone-v1: encoder-only sentence embedding model (MiniLM-L6-v2).
// No KV-cache / no past_key_values — compatible with tract-onnx on Android.
// ~24MB ONNX, runs on CPU in <500ms on mid-range phones.
const MODEL_CATALOG = {
  'hone-phone-v1': {
    hf_repo: 'Xenova/all-MiniLM-L6-v2',
    files: [
      'config.json', 'tokenizer.json', 'tokenizer_config.json',
      'special_tokens_map.json', 'vocab.txt', 'onnx/model.onnx',
    ],
    phone_model: true,
  },
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
  'llama-3.2-1b': {
    hf_repo: 'onnx-community/Llama-3.2-1B-Instruct-ONNX',
    files: [
      'config.json', 'generation_config.json', 'tokenizer.json',
      'tokenizer_config.json', 'special_tokens_map.json', 'onnx/model_q4.onnx',
    ],
  },
  'llama-3.2-3b': {
    hf_repo: 'onnx-community/Llama-3.2-3B-Instruct-ONNX',
    files: [
      'config.json', 'generation_config.json', 'tokenizer.json',
      'tokenizer_config.json', 'special_tokens_map.json', 'onnx/model_q4.onnx',
    ],
  },
  'phi-3.5-mini': {
    hf_repo: 'onnx-community/Phi-3.5-mini-instruct-onnx-web',
    files: [
      'config.json', 'generation_config.json', 'tokenizer.json',
      'tokenizer_config.json', 'special_tokens_map.json', 'added_tokens.json',
      'tokenizer.model', 'genai_config.json', 'onnx/model_q4f16.onnx',
    ],
  },
  'deepseek-r1-1.5b': {
    hf_repo: 'onnx-community/DeepSeek-R1-Distill-Qwen-1.5B-ONNX',
    files: [
      'config.json', 'generation_config.json', 'tokenizer.json',
      'tokenizer_config.json', 'special_tokens_map.json', 'onnx/model_q4.onnx',
    ],
  },
  'gemma-3-1b': {
    hf_repo: 'onnx-community/gemma-3-1b-it-ONNX',
    files: [
      'config.json', 'generation_config.json', 'tokenizer.json',
      'tokenizer_config.json', 'special_tokens_map.json', 'onnx/model_q4.onnx',
    ],
  },
  'qwen3-0.6b': {
    hf_repo: 'onnx-community/Qwen3-0.6B-ONNX',
    files: [
      'config.json', 'generation_config.json', 'tokenizer.json',
      'tokenizer_config.json', 'special_tokens_map.json', 'added_tokens.json',
      'vocab.json', 'merges.txt', 'onnx/model_q4.onnx',
    ],
  },
  'qwen3-1.7b': {
    hf_repo: 'onnx-community/Qwen3-1.7B-ONNX',
    files: [
      'config.json', 'generation_config.json', 'tokenizer.json',
      'tokenizer_config.json', 'special_tokens_map.json', 'added_tokens.json',
      'vocab.json', 'merges.txt', 'onnx/model_q4.onnx',
    ],
  },
  'qwen3-4b': {
    hf_repo: 'onnx-community/Qwen3-4B-ONNX',
    files: [
      'config.json', 'generation_config.json', 'tokenizer.json',
      'tokenizer_config.json', 'special_tokens_map.json', 'added_tokens.json',
      'vocab.json', 'merges.txt', 'onnx/model_q4f16.onnx',
    ],
  },
  // ── GGUF models (for Ollama GPU miners) ───────────────────────────────────
  // Large single-file or split downloads streamed directly into the blob store.
  // Miners pull the GGUF parts from chain, import into Ollama via modelfile.
  // Split files: llama.cpp loads them natively when all parts are present.
  'dirty-muse-writer': {
    hf_repo: 'mradermacher/Dirty-Muse-Writer-v01-Uncensored-Erotica-NSFW-GGUF',
    format: 'gguf',
    files: ['Dirty-Muse-Writer-v01-Uncensored-Erotica-NSFW.Q4_K_M.gguf'],
    base_repo: 'Mantis2024/Dirty-Muse-Writer-v01-Uncensored-Erotica-NSFW',
    base_files: ['config.json', 'tokenizer.json', 'tokenizer_config.json', 'special_tokens_map.json', 'tokenizer.model'],
  },
  'mythomax-l2-13b': {
    hf_repo: 'mradermacher/MythoMax-L2-13b-GGUF',
    format: 'gguf',
    files: ['MythoMax-L2-13b.Q4_K_M.gguf'],
    base_repo: 'Gryphe/MythoMax-L2-13b',
    base_files: ['config.json', 'tokenizer.json', 'tokenizer_config.json', 'special_tokens_map.json', 'tokenizer.model'],
  },
  'dolphin-mixtral-8x7b': {
    hf_repo: 'TheBloke/dolphin-2.7-mixtral-8x7b-GGUF',
    format: 'gguf',
    files: ['dolphin-2.7-mixtral-8x7b.Q4_K_M.gguf'],
    base_repo: 'cognitivecomputations/dolphin-2.7-mixtral-8x7b',
    base_files: ['config.json', 'tokenizer.json', 'tokenizer_config.json', 'special_tokens_map.json', 'tokenizer.model'],
  },
  'wizardlm-2-8x22b': {
    hf_repo: 'mradermacher/WizardLM-2-8x22B-GGUF',
    format: 'gguf',
    // IQ4_XS split — best quality that fits (~76GB total, gpu80 tier)
    files: ['WizardLM-2-8x22B.IQ4_XS.gguf.part1of2', 'WizardLM-2-8x22B.IQ4_XS.gguf.part2of2'],
    base_repo: 'microsoft/WizardLM-2-8x22B',
    base_files: ['config.json', 'tokenizer.json', 'tokenizer_config.json', 'special_tokens_map.json', 'tokenizer.model'],
  },
  'goliath-120b': {
    hf_repo: 'TheBloke/goliath-120b-GGUF',
    format: 'gguf',
    // Q2_K — only single-file variant available, ~49.6GB, gpu80 tier
    files: ['goliath-120b.Q2_K.gguf'],
    base_repo: 'alpindale/goliath-120b',
    base_files: ['config.json', 'tokenizer.json', 'tokenizer_config.json', 'special_tokens_map.json', 'tokenizer.model'],
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
    mod.get(url, { headers: { 'User-Agent': 'hone-model-fetcher/1.0' } }, (res) => {
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

// Stream a URL directly into the blob store — no RAM buffering, handles files of any size.
function fetchUrlToBlobStream(url) {
  return new Promise((resolve, reject) => {
    const mod = url.startsWith('https') ? https : http;
    mod.get(url, { headers: { 'User-Agent': 'hone-model-fetcher/1.0' } }, (res) => {
      if ([301, 302, 307, 308].includes(res.statusCode)) {
        let loc = res.headers.location;
        if (loc && loc.startsWith('/')) {
          const u = new URL(url);
          loc = u.origin + loc;
        }
        return fetchUrlToBlobStream(loc).then(resolve).catch(reject);
      }
      if (res.statusCode !== 200) return reject(new Error('HTTP ' + res.statusCode));
      blobStore.putBlobStream(res).then(resolve).catch(reject);
    }).on('error', reject);
  });
}

// Stream a URL into 16MB chunks stored as separate blobs.
function fetchUrlToChunks(url, chunkSize) {
  return new Promise(function(resolve, reject) {
    var mod = url.startsWith('https') ? https : http;
    mod.get(url, { headers: { 'User-Agent': 'hone-model-fetcher/1.0' } }, function(res) {
      if ([301, 302, 307, 308].includes(res.statusCode)) {
        var loc = res.headers.location;
        if (loc && loc.startsWith('/')) { var u = new URL(url); loc = u.origin + loc; }
        return fetchUrlToChunks(loc, chunkSize).then(resolve).catch(reject);
      }
      if (res.statusCode !== 200) return reject(new Error('HTTP ' + res.statusCode));
      blobStore.putBlobStreamChunked(res, chunkSize).then(resolve).catch(reject);
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
 * POST /api/models/upload
 * Anyone can register a community model on the HONE chain and earn royalties.
 * Body:
 *   model_id       — unique slug (lowercase alphanumeric + hyphens), e.g. "my-finetune-v1"
 *   name           — human-readable name
 *   description    — short description
 *   format         — "onnx" (default)
 *   size_mb        — approximate model size
 *   files          — { "config.json": "<cid>", "onnx/model_q4.onnx": "<cid>", ... }
 *   royalty_percent — 0–10 (default 5). % of miner reward paid to uploader each inference.
 *   stake_amount    — HONE to stake (minimum 1 per 100MB). Held in chain escrow.
 * Auth required. Stake is deducted from uploader's on-chain balance.
 */
router.post('/upload', authenticateToken, express.json({ limit: '64kb' }), async function(req, res) {
  const uploader = req.user && req.user.username;
  if (!uploader) return res.status(401).json({ error: 'unauthenticated' });

  const { model_id, name, description, format, size_mb, files, royalty_percent, stake_amount } = req.body || {};

  if (!model_id || !/^[a-z0-9][a-z0-9._-]{1,63}$/.test(model_id)) {
    return res.status(400).json({ error: 'model_id must be lowercase alphanumeric + hyphens/dots, 2–64 chars' });
  }
  if (!files || typeof files !== 'object' || Object.keys(files).length === 0) {
    return res.status(400).json({ error: 'files required: { "config.json": "<cid>", ... }' });
  }

  // Verify all CIDs exist in the blob store
  const missingCids = [];
  for (const [filename, cid] of Object.entries(files)) {
    if (!blobStore.hasBlob(cid)) missingCids.push({ filename, cid });
  }
  if (missingCids.length > 0) {
    return res.status(400).json({ error: 'some CIDs not found in blob store — upload files first via POST /api/blobs/stream', missing: missingCids });
  }

  const royaltyPct = Math.min(Math.max(parseFloat(royalty_percent) || 5, 0), 10);
  const sizeMb = parseFloat(size_mb) || 0;
  const minStake = Math.max(1, Math.ceil(sizeMb / 100));
  const stakeAmt = Math.max(minStake, parseFloat(stake_amount) || minStake);

  // Check uploader balance
  const balance = stateStore.getBalance ? stateStore.getBalance(uploader, 'HONE') : 0;
  if (balance < stakeAmt) {
    return res.status(402).json({ error: 'insufficient balance for stake', required: stakeAmt, balance, unit: 'HONE' });
  }

  // Check model_id not already owned by someone else
  const existing = stateStore.getCommunityModel ? stateStore.getCommunityModel(model_id) : null;
  if (existing && existing.uploader !== uploader) {
    return res.status(409).json({ error: 'model_id already registered by another uploader', uploader: existing.uploader });
  }

  // Deduct stake (transfer to hone_recycle — drains slowly to storage hosts in future)
  try {
    const epoch = await ledger.getCurrentEpoch();
    await ledger.recordTransfer(uploader, 'hone_recycle', stakeAmt, 'HONE', `Model upload stake: ${model_id}`, epoch);
    await ledger.recordModelUpload(uploader, model_id, {
      name: name || model_id,
      description: description || '',
      format: format || 'onnx',
      size_mb: sizeMb,
      files,
      royalty_percent: royaltyPct,
      stake_amount: stakeAmt,
    }, epoch);
  } catch (err) {
    return res.status(500).json({ error: err.message });
  }

  // Also mirror into model-registry.json so it's immediately serveable
  const registry = loadRegistry();
  if (!registry[model_id] || registry[model_id].uploader === uploader || !registry[model_id].uploader) {
    registry[model_id] = {
      ...registry[model_id],
      name: name || model_id,
      description: description || '',
      format: format || 'onnx',
      size_mb: sizeMb,
      uploader,
      royalty_percent: royaltyPct,
      stake_amount: stakeAmt,
      files,
      status: Object.keys(files).length > 0 ? 'active' : 'pending',
      uploaded_at: new Date().toISOString(),
    };
    saveRegistry(registry);
  }

  res.status(201).json({
    status: 'registered',
    model_id,
    uploader,
    royalty_percent: royaltyPct,
    stake_deducted: stakeAmt,
    file_count: Object.keys(files).length,
    message: `Model ${model_id} is now active on the HONE chain. You earn ${royaltyPct}% of each inference reward when miners use it.`,
  });
});

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

      // Fetch all file lists: main repo files + optional base_repo files
      const allFiles = [
        ...catalog.files.map(f => ({ file: f, repo: catalog.hf_repo })),
        ...(catalog.base_files || []).map(f => ({ file: f, repo: catalog.base_repo })),
      ];

      for (const { file, repo } of allFiles) {
        if (files[file]) continue; // already on-chain
        const hfUrl = `https://huggingface.co/${repo}/resolve/main/${file}`;
        // Use streaming for large files (GGUF) to avoid loading into RAM
        const isLarge = file.endsWith('.gguf') || file.endsWith('.bin');
        let result;
        try {
          if (isLarge) {
            console.log(`[model-fetch] ${modelId}/${file} chunking (large file)...`);
            const chunkResult = await fetchUrlToChunks(hfUrl, 16 * 1024 * 1024);
            // Store chunk manifest alongside files
            const reg3 = loadRegistry();
            if (!reg3[modelId]) reg3[modelId] = {};
            if (!reg3[modelId].chunked_files) reg3[modelId].chunked_files = {};
            reg3[modelId].chunked_files[file] = {
              chunk_size: chunkResult.chunk_size,
              total_size: chunkResult.total_size,
              chunks: chunkResult.chunks.map(function(c) { return { cid: c.cid, size: c.size }; }),
            };
            saveRegistry(reg3);
            // Use first chunk CID as the canonical file CID for backward compat
            result = { cid: chunkResult.chunks[0].cid + '_chunked', size: chunkResult.total_size, existed: false };
            // Skip the normal files[file] = result.cid path for chunked files
            console.log(`[model-fetch] ${modelId}/${file} → ${chunkResult.chunks.length} chunks, ${(chunkResult.total_size/1e6).toFixed(0)}MB`);
            changed = true;
            continue;
          } else {
            const buf = await fetchUrl(hfUrl);
            result = blobStore.putBlob(buf);
          }
        } catch (err) {
          console.warn(`[model-fetch] ${modelId}/${file} download failed: ${err.message}`);
          continue;
        }
        try {
          const epoch = await ledger.getCurrentEpoch();
          await ledger.recordBlobStoreCommit(uploader, result.cid, result.size, [uploader], DEFAULT_DURATION_EPOCHS, 0, epoch);
        } catch (_) {}
        files[file] = result.cid;
        changed = true;
        console.log(`[model-fetch] ${modelId}/${file} → ${result.cid.slice(0, 12)}... ${(result.size/1e6).toFixed(0)}MB (${result.existed ? 'existed' : 'new'})`);
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
  // Merge in any community models registered on-chain but not yet in the JSON file
  const chainModels = stateStore.getAllCommunityModels ? stateStore.getAllCommunityModels() : [];
  for (const cm of chainModels) {
    if (!registry[cm.model_id]) {
      registry[cm.model_id] = {
        name: cm.name, description: cm.description, format: cm.format,
        size_mb: cm.size_mb, uploader: cm.uploader, royalty_percent: cm.royalty_percent,
        status: cm.status, files: cm.files || {},
      };
    }
  }
  const built_in = {};
  const community = {};
  Object.keys(registry).forEach(function(id) {
    const m = registry[id];
    const entry = {
      name: m.name,
      description: m.description,
      format: m.format,
      size_mb: m.size_mb,
      status: m.status,
      file_count: Object.keys(m.files || {}).length,
    };
    if (m.uploader) {
      community[id] = { ...entry, uploader: m.uploader, royalty_percent: m.royalty_percent };
    } else {
      built_in[id] = entry;
    }
  });
  res.json({ built_in, community });
});

/**
 * GET /api/models/:id/manifest
 * Returns the full file manifest: single-file CIDs and chunked file manifests.
 * Each chunk entry includes the best available download URL.
 * Used by phone miners to discover chunk CIDs and which storage nodes hold them.
 */
router.get('/:id/manifest', function(req, res) {
  const registry = loadRegistry();
  let model = registry[req.params.id];

  // Fall back to MODEL_CATALOG for built-in models not yet stored locally.
  // Return HuggingFace URLs directly so phones can download from source.
  if (!model) {
    const catalog = MODEL_CATALOG[req.params.id];
    if (!catalog) return res.status(404).json({ error: 'model not in registry', id: req.params.id });
    const hfBase = `https://huggingface.co/${catalog.hf_repo}/resolve/main`;
    const files = {};
    for (const filename of catalog.files) {
      files[filename] = { chunked: false, cid: null, size: 0, url: `${hfBase}/${filename}` };
    }
    return res.json({ model_id: req.params.id, files, source: 'huggingface' });
  }

  if (model.status === 'pending') return res.status(503).json({ error: 'model not yet available' });

  const files = {};
  const apiBase = (req.protocol + '://' + req.get('host'));

  // Single-blob files
  for (const [filename, cid] of Object.entries(model.files || {})) {
    if ((model.chunked_files || {})[filename]) continue; // covered below
    const hostUrl = bestHostUrl(cid);
    files[filename] = {
      chunked: false,
      cid: cid,
      size: blobStore.statBlob(cid) ? blobStore.statBlob(cid).size : 0,
      url: hostUrl || (apiBase + '/api/models/' + req.params.id + '/' + filename),
    };
  }

  // Chunked files
  for (const [filename, manifest] of Object.entries(model.chunked_files || {})) {
    files[filename] = {
      chunked: true,
      chunk_size: manifest.chunk_size,
      total_size: manifest.total_size,
      chunks: manifest.chunks.map(function(chunk, idx) {
        const hostUrl = bestHostUrl(chunk.cid);
        return {
          index: idx,
          cid: chunk.cid,
          size: chunk.size,
          url: hostUrl || (apiBase + '/api/blobs/' + chunk.cid),
        };
      }),
    };
  }

  res.json({ model_id: req.params.id, files });
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

  const filePath = Array.isArray(req.params.path) ? req.params.path.join('/') : req.params.path;

  // Check chunked manifest first
  const chunkManifest = (model.chunked_files || {})[filePath];
  if (chunkManifest) {
    const total = chunkManifest.total_size;
    res.set('X-HONE-Chunked', String(chunkManifest.chunks.length));
    res.set('Cache-Control', 'public, max-age=31536000, immutable');
    res.set('Accept-Ranges', 'bytes');
    const ext = require('path').extname(filePath).toLowerCase();
    const ctMap = { '.json': 'application/json', '.onnx': 'application/octet-stream', '.bin': 'application/octet-stream' };
    res.set('Content-Type', ctMap[ext] || 'application/octet-stream');
    const rangeHeader = req.headers['range'];
    if (rangeHeader) {
      const match = /bytes=(\d*)-(\d*)/.exec(rangeHeader);
      const start = match && match[1] ? parseInt(match[1]) : 0;
      const end = match && match[2] ? Math.min(parseInt(match[2]), total - 1) : total - 1;
      const length = end - start + 1;
      res.status(206);
      res.set('Content-Range', `bytes ${start}-${end}/${total}`);
      res.set('Content-Length', String(length));
      const buf = blobStore.readChunkedRange(chunkManifest.chunks, start, length);
      return res.end(buf);
    }
    res.set('Content-Length', String(total));
    return blobStore.createChunkedReadStream(chunkManifest.chunks.map(function(c) { return c.cid; })).pipe(res);
  }

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

  res.set('X-HONE-CID', cid);
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
