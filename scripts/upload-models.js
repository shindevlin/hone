#!/usr/bin/env node
/**
 * BTCPC Model Uploader
 *
 * Downloads ONNX model files from HuggingFace (one-time operation),
 * uploads each file to the BTCPC blob store, and writes CIDs to
 * data/model-registry.json. After this runs, phones download models
 * from btcpc.net — not from HuggingFace.
 *
 * Usage:
 *   BTCPC_API=https://btcpc.net BTCPC_JWT=<your-jwt> node scripts/upload-models.js
 *   node scripts/upload-models.js --api http://localhost:3000 --jwt <token> --model smollm2-360m
 */

const https = require('https');
const http = require('http');
const fs = require('fs');
const path = require('path');

const REGISTRY_PATH = path.resolve(__dirname, '../data/model-registry.json');

const API_BASE = process.env.BTCPC_API || 'https://btcpc.net';
const JWT = process.env.BTCPC_JWT;
const MODEL_FILTER = process.argv.includes('--model')
  ? process.argv[process.argv.indexOf('--model') + 1]
  : null;

if (!JWT) {
  console.error('BTCPC_JWT env var required. Get yours from the app settings or /public/login.');
  process.exit(1);
}

// HuggingFace file lists per model (ONNX quantized variants)
const MODEL_FILES = {
  'smollm2-360m': {
    hf_repo: 'HuggingFaceTB/SmolLM2-360M-Instruct',
    files: [
      'tokenizer.json',
      'tokenizer_config.json',
      'special_tokens_map.json',
      'config.json',
      'generation_config.json',
      'onnx/model_q4.onnx',
      'onnx/model_q4.onnx_data',
    ],
  },
  'qwen2.5-0.5b': {
    hf_repo: 'Qwen/Qwen2.5-0.5B-Instruct',
    files: [
      'tokenizer.json',
      'tokenizer_config.json',
      'special_tokens_map.json',
      'config.json',
      'generation_config.json',
      'onnx/model_q4.onnx',
      'onnx/model_q4.onnx_data',
    ],
  },
};

function fetchUrl(url) {
  return new Promise((resolve, reject) => {
    const mod = url.startsWith('https') ? https : http;
    const req = mod.get(url, { headers: { 'User-Agent': 'btcpc-model-uploader/1.0' } }, (res) => {
      if (res.statusCode === 301 || res.statusCode === 302 || res.statusCode === 307 || res.statusCode === 308) {
        return fetchUrl(res.headers.location).then(resolve).catch(reject);
      }
      if (res.statusCode !== 200) {
        return reject(new Error(`HTTP ${res.statusCode} for ${url}`));
      }
      const chunks = [];
      res.on('data', (c) => chunks.push(c));
      res.on('end', () => resolve(Buffer.concat(chunks)));
      res.on('error', reject);
    });
    req.on('error', reject);
  });
}

async function uploadBlob(buffer, jwt) {
  return new Promise((resolve, reject) => {
    const urlObj = new URL(API_BASE + '/api/blobs/stream');
    const mod = urlObj.protocol === 'https:' ? https : http;
    const opts = {
      hostname: urlObj.hostname,
      port: urlObj.port || (urlObj.protocol === 'https:' ? 443 : 80),
      path: urlObj.pathname,
      method: 'POST',
      headers: {
        'Content-Type': 'application/octet-stream',
        'Content-Length': String(buffer.length),
        'Authorization': 'Bearer ' + jwt,
      },
    };
    const req = mod.request(opts, (res) => {
      const chunks = [];
      res.on('data', (c) => chunks.push(c));
      res.on('end', () => {
        try {
          resolve(JSON.parse(Buffer.concat(chunks).toString()));
        } catch (e) {
          reject(new Error('Invalid JSON response from blob upload'));
        }
      });
    });
    req.on('error', reject);
    req.end(buffer);
  });
}

async function processModel(modelId, modelDef, registryEntry) {
  const { hf_repo, files } = MODEL_FILES[modelId];
  const updatedFiles = { ...(registryEntry.files || {}) };

  for (const file of files) {
    if (updatedFiles[file]) {
      console.log(`  [skip] ${file} already has CID ${updatedFiles[file].slice(0, 12)}...`);
      continue;
    }
    // transformers.js resolves files from the Xenova mirror (pre-converted ONNX)
    const hfUrl = `https://huggingface.co/Xenova/${hf_repo.split('/')[1]}/resolve/main/${file}`;
    process.stdout.write(`  [download] ${file} ... `);
    let buf;
    try {
      buf = await fetchUrl(hfUrl);
    } catch (err) {
      // Try alternate Xenova model naming
      const altUrl = `https://huggingface.co/Xenova/${modelId}/resolve/main/${file}`;
      try {
        buf = await fetchUrl(altUrl);
      } catch (err2) {
        console.log(`SKIP (not found on HuggingFace: ${err2.message})`);
        continue;
      }
    }
    process.stdout.write(`${(buf.length / 1024 / 1024).toFixed(1)}MB  `);
    const result = await uploadBlob(buf, JWT);
    if (!result.cid) {
      console.log(`ERROR: ${JSON.stringify(result)}`);
      continue;
    }
    updatedFiles[file] = result.cid;
    console.log(`→ ${result.cid.slice(0, 12)}... (${result.existed ? 'existed' : 'new'})`);
  }

  return updatedFiles;
}

async function main() {
  const registry = JSON.parse(fs.readFileSync(REGISTRY_PATH, 'utf8'));
  const modelsToProcess = MODEL_FILTER
    ? [MODEL_FILTER]
    : Object.keys(MODEL_FILES);

  for (const modelId of modelsToProcess) {
    if (!MODEL_FILES[modelId]) {
      console.error(`Unknown model: ${modelId}. Available: ${Object.keys(MODEL_FILES).join(', ')}`);
      continue;
    }
    console.log(`\n=== ${modelId} ===`);
    const entry = registry[modelId] || { files: {} };
    const updatedFiles = await processModel(modelId, MODEL_FILES[modelId], entry);
    registry[modelId] = {
      ...entry,
      files: updatedFiles,
      status: Object.keys(updatedFiles).length > 0 ? 'active' : 'pending',
      uploaded_at: new Date().toISOString(),
    };
    fs.writeFileSync(REGISTRY_PATH, JSON.stringify(registry, null, 2) + '\n');
    console.log(`  Registry updated.`);
  }
  console.log('\nDone. Restart btcpc-api to serve updated registry.');
}

main().catch((err) => { console.error(err); process.exit(1); });
