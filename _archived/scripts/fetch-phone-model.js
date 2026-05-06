#!/usr/bin/env node
"use strict";
/**
 * Fetches btcpc-phone-v1 (Xenova/all-MiniLM-L6-v2 encoder-only ONNX)
 * from HuggingFace and registers it in the local blob store + model registry.
 * Run once: node scripts/fetch-phone-model.js
 */
const https = require('https');
const fs = require('fs');
const path = require('path');
const crypto = require('crypto');
const os = require('os');

const REGISTRY_PATH = path.resolve(__dirname, '../data/model-registry.json');
const BLOB_ROOT = process.env.BTCPC_BLOB_DIR || path.join(os.homedir(), '.btcpc', 'blobs');

const MODEL_ID = 'btcpc-phone-v1';
const HF_REPO = 'Xenova/all-MiniLM-L6-v2';
const FILES = [
  'config.json',
  'tokenizer.json',
  'tokenizer_config.json',
  'special_tokens_map.json',
  'vocab.txt',
  'onnx/model.onnx',
];

function blobPath(cid) {
  return path.join(BLOB_ROOT, cid.slice(0, 2), cid.slice(2, 4), cid);
}

function putBlob(buf) {
  const cid = crypto.createHash('sha256').update(buf).digest('hex');
  const dest = blobPath(cid);
  if (!fs.existsSync(dest)) {
    fs.mkdirSync(path.dirname(dest), { recursive: true });
    fs.writeFileSync(dest, buf);
    return { cid, existed: false };
  }
  return { cid, existed: true };
}

function fetchUrl(url) {
  return new Promise((resolve, reject) => {
    https.get(url, { headers: { 'User-Agent': 'btcpc-fetch/1.0' } }, (res) => {
      if ([301, 302, 307, 308].includes(res.statusCode)) {
        let loc = res.headers.location;
        if (loc && loc.startsWith('/')) {
          const u = new URL(url);
          loc = u.origin + loc;
        }
        return fetchUrl(loc).then(resolve).catch(reject);
      }
      if (res.statusCode !== 200) return reject(new Error('HTTP ' + res.statusCode + ' ' + url));
      const chunks = [];
      res.on('data', c => chunks.push(c));
      res.on('end', () => resolve(Buffer.concat(chunks)));
      res.on('error', reject);
    }).on('error', reject);
  });
}

async function main() {
  let registry = {};
  try { registry = JSON.parse(fs.readFileSync(REGISTRY_PATH, 'utf8')); } catch (_) {}

  if (registry[MODEL_ID] && registry[MODEL_ID].status === 'active' &&
      Object.keys(registry[MODEL_ID].files || {}).length >= FILES.length) {
    console.log('[fetch-phone-model] btcpc-phone-v1 already active, nothing to do.');
    return;
  }

  const files = {};
  for (const file of FILES) {
    const hfUrl = `https://huggingface.co/${HF_REPO}/resolve/main/${file}`;
    process.stdout.write(`  Fetching ${file}...`);
    try {
      const buf = await fetchUrl(hfUrl);
      const { cid, existed } = putBlob(buf);
      files[file] = cid;
      console.log(` ${(buf.length / 1e6).toFixed(1)}MB  cid=${cid.slice(0, 12)}... ${existed ? '(existed)' : '(new)'}`);
    } catch (err) {
      console.error(` FAILED: ${err.message}`);
      process.exit(1);
    }
  }

  registry[MODEL_ID] = {
    name: 'MiniLM-L6-v2 (Phone)',
    description: 'Encoder-only sentence embedding model. No KV-cache, runs on any phone CPU via tract-onnx. ~23MB.',
    format: 'onnx',
    size_mb: 23,
    phone_model: true,
    status: 'active',
    files,
    uploaded_at: new Date().toISOString(),
  };

  fs.writeFileSync(REGISTRY_PATH, JSON.stringify(registry, null, 2) + '\n');
  console.log(`\n[fetch-phone-model] Registered ${MODEL_ID} with ${Object.keys(files).length} files.`);
}

main().catch(err => { console.error(err); process.exit(1); });
