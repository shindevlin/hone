"use strict";

const express = require('express');
const crypto = require('crypto');
const axios = require('axios');
const WorkProof = require('../models/WorkProof');
const Project = require('../models/Project');
const { getCurrentEpoch } = require('../services/epochManager');

// D.5-delta: secretStore-first project lookup, Mongo fallback.
let _secretStoreLoaded = false;
async function getSecretStore() {
  const secretStore = require('../services/secretStore');
  if (!_secretStoreLoaded) {
    try {
      await secretStore.load();
      _secretStoreLoaded = true;
    } catch (err) {
      console.warn('[inference-api] secretStore load failed: ' + err.message);
    }
  }
  return secretStore;
}

// Look up a project by plaintext API key: secretStore first, Mongo fallback.
// Returns a project-like object or null.
async function _findProjectByApiKey(plaintextKey) {
  try {
    const ss = await getSecretStore();
    if (ss && typeof ss.getProjectByApiKey === 'function') {
      const ssProject = ss.getProjectByApiKey(plaintextKey);
      if (ssProject) {
        return {
          _source: 'secretStore',
          name: _findProjectName(ss, ssProject),
          owner: ssProject.owner,
          repo: ssProject.repo,
          repo_url: ssProject.repo_url,
          wallet_address: ssProject.wallet_address,
          balance: ssProject.balance || 0,
          verified: ssProject.verified !== false,
          isActive: ssProject.isActive !== false,
          totalSpent: ssProject.totalSpent || 0,
          totalRequests: ssProject.totalRequests || 0,
          save: async function () {
            try {
              const ss2 = await getSecretStore();
              await ss2.updateProject(this.name, {
                balance: this.balance,
                totalSpent: this.totalSpent,
                totalRequests: this.totalRequests,
              });
            } catch (_) {}
          }
        };
      }
    }
  } catch (_) {}

  // Mongo fallback
  const project = await Project.findOne({ apiKey: plaintextKey, isActive: true });
  return project || null;
}

// Helper: find name key for a secretStore project record.
function _findProjectName(ss, projectRecord) {
  try {
    const all = ss.getAllProjects ? ss.getAllProjects() : [];
    const entry = all.find(p =>
      p.wallet_address === projectRecord.wallet_address ||
      p.owner === projectRecord.owner
    );
    return entry ? entry.name : (projectRecord.owner + '/' + (projectRecord.repo || ''));
  } catch (_) {
    return projectRecord.owner + '/' + (projectRecord.repo || '');
  }
}
const { getModelWeight } = require('../mining/workGenerator');
const { calculateCost, getCurrentPricing, getAutoBid } = require('../services/pricing');
const { submitInference, getJob, hasMiners, peerCount } = require('./p2pRouter');

const router = express.Router();
const OLLAMA_URL = process.env.OLLAMA_URL || 'http://localhost:11434';

function extractOllamaText(data) {
  const msg = data?.message || {};
  return msg.content || msg.thinking || data?.response || data?.thinking || '';
}

function sleep(ms) {
  return new Promise(resolve => setTimeout(resolve, ms));
}

function estimateTokens(text) {
  return Math.max(1, Math.ceil(String(text || '').length / 4));
}

function sseChunk(id, content, finishReason, model) {
  return {
    id,
    object: 'chat.completion.chunk',
    model,
    choices: [
      {
        index: 0,
        delta: content ? { content } : {},
        finish_reason: finishReason || null
      }
    ]
  };
}

function writeSSE(res, payload) {
  res.write(`data: ${JSON.stringify(payload)}\n\n`);
}

function chunkText(text) {
  const words = String(text || '').split(/\s+/).filter(Boolean);
  const chunks = [];
  let i = 0;
  while (i < words.length) {
    const size = 3 + (chunks.length % 3);
    chunks.push(words.slice(i, i + size).join(' '));
    i += size;
  }
  return chunks.length ? chunks : [''];
}

async function runDirectOllamaChat({ model, messages, max_tokens, temperature }) {
  const response = await axios.post(`${OLLAMA_URL}/api/chat`, {
    model,
    messages,
    stream: false,
    think: false,
    options: { temperature: temperature || 0.7, num_predict: max_tokens || 512 }
  }, { timeout: 300000 });

  return {
    text: extractOllamaText(response.data),
    tokens: response.data.eval_count || 0,
    elapsedMs: response.data.total_duration ? Math.round(response.data.total_duration / 1e6) : 0
  };
}

async function streamDirectOllamaChat({ model, messages, max_tokens, temperature, res, requestId }) {
  const response = await axios.post(`${OLLAMA_URL}/api/chat`, {
    model,
    messages,
    stream: true,
    think: false,
    options: { temperature: temperature || 0.7, num_predict: max_tokens || 512 }
  }, {
    timeout: 300000,
    responseType: 'stream'
  });

  let buffer = '';
  let sawContent = false;

  for await (const chunk of response.data) {
    buffer += chunk.toString('utf8');
    const lines = buffer.split('\n');
    buffer = lines.pop() || '';

    for (const line of lines) {
      const trimmed = line.trim();
      if (!trimmed) continue;
      let parsed;
      try {
        parsed = JSON.parse(trimmed);
      } catch (_) {
        continue;
      }
      const content = extractOllamaText(parsed);
      if (content) {
        sawContent = true;
        writeSSE(res, sseChunk(requestId, content, null, model));
      }
      if (parsed.done) {
        writeSSE(res, sseChunk(requestId, '', 'stop', model));
        res.write('data: [DONE]\n\n');
        res.end();
        return;
      }
    }
  }

  if (!sawContent) {
    writeSSE(res, sseChunk(requestId, '', 'stop', model));
    res.write('data: [DONE]\n\n');
    res.end();
  }
}

async function pollInferenceJob(jobId, maxWait, pollInterval) {
  const startTime = Date.now();
  let job = null;

  while (Date.now() - startTime < maxWait) {
    job = await getJob(jobId);
    if (job && (job.status === 'completed' || job.status === 'failed')) break;
    await sleep(pollInterval);
  }

  return job;
}

async function finalizeChatResult({ req, selectedModel, messages, max_tokens, temperature, max_fee }) {
  const startTime = Date.now();

  if (!hasMiners()) {
    const error = new Error('No miners connected to the P2P network. Inference requires at least one active miner.');
    error.status = 503;
    error.payload = {
      error: {
        message: error.message,
        type: 'network_error',
        code: 'no_miners',
        peers: peerCount()
      }
    };
    throw error;
  }

  let fee = max_fee;
  if (!fee) {
    const autoBid = await getAutoBid(selectedModel, max_tokens || 512);
    fee = autoBid.bid;
  }

  const { job_id } = await submitInference({
    model: selectedModel,
    messages,
    maxTokens: max_tokens,
    temperature,
    maxFee: fee,
    projectId: req.project?._id
  });

  const job = await pollInferenceJob(job_id, 300000, 2000);
  if (!job || job.status !== 'completed') {
    const status = job?.status || 'unknown';
    const error = new Error(`Inference not completed. Job status: ${status}. Check GET /v1/inference/${job_id}`);
    error.status = 504;
    error.payload = {
      error: {
        message: error.message,
        type: 'timeout',
        code: 'inference_pending',
        job_id
      }
    };
    throw error;
  }

  let assistantContent = job.result_text || '';
  let evalCount = job.tokens_generated || estimateTokens(assistantContent);

  if (!assistantContent.trim()) {
    const direct = await runDirectOllamaChat({
      model: selectedModel,
      messages,
      max_tokens,
      temperature
    });
    assistantContent = direct.text;
    evalCount = direct.tokens || evalCount;
  }

  if (!assistantContent.trim()) {
    const error = new Error('Inference completed without usable output from miners or local backend.');
    error.status = 502;
    error.payload = {
      error: {
        message: error.message,
        type: 'server_error',
        code: 'empty_inference_result',
        job_id
      }
    };
    throw error;
  }

  const promptEvalCount = estimateTokens(messages.map(m => m.content || '').join(' '));
  const promptText = messages.map(m => `${m.role}:${m.content || ''}`).join('|');
  const promptHash = crypto.createHash('sha256').update(promptText).digest('hex');
  const resultHash = crypto.createHash('sha256').update(assistantContent).digest('hex');
  const weightFactor = getModelWeight(selectedModel);
  const workValue = evalCount * weightFactor;

  let epochNumber = 0;
  let proofHash = '';
  let verified = false;

  try {
    epochNumber = await getCurrentEpoch();
    const proof = new WorkProof({
      epoch_number: epochNumber,
      node_id: 'inference-api',
      prompt_hash: promptHash,
      result_hash: resultHash,
      model: selectedModel,
      tokens_generated: evalCount,
      model_weight_factor: weightFactor,
      work_value: workValue
    });
    const saved = await proof.save();
    proofHash = saved._id.toString();
    verified = true;
  } catch (proofErr) {
    console.error('[BTCPC Inference] Failed to save work proof:', proofErr.message);
    proofHash = crypto.createHash('sha256')
      .update(promptHash + resultHash + Date.now().toString())
      .digest('hex');
  }

  const { cost, pricing } = await calculateCost(evalCount, selectedModel);
  if (req.project) {
    req.project.balance = Math.max(0, req.project.balance - cost);
    req.project.totalSpent += cost;
    req.project.totalRequests += 1;
    await req.project.save();
  }

  return {
    jobId: job_id,
    assistantContent,
    evalCount,
    promptEvalCount,
    epochNumber,
    proofHash,
    verified,
    cost,
    pricing,
    elapsedMs: Date.now() - startTime
  };
}

/**
 * Bearer token authentication middleware.
 * Accepts btcpc_ project keys (verified + balance check) or any non-empty token.
 */
async function authenticateBearer(req, res, next) {
  const authHeader = req.headers.authorization;
  if (!authHeader || !authHeader.startsWith('Bearer ') || !authHeader.slice(7).trim()) {
    return res.status(401).json({
      error: {
        message: 'Missing or empty API key. Provide a Bearer token in the Authorization header.',
        type: 'authentication_error',
        code: 'invalid_api_key'
      }
    });
  }

  const token = authHeader.slice(7).trim();
  req.apiKey = token;

  // Internal relay key — used by bot endpoints
  const RELAY_KEY = process.env.BTCPC_RELAY_API_KEY;
  if (RELAY_KEY && token === RELAY_KEY) {
    req.isRelay = true;
    return next();
  }

  // ── Session tokens (btcpc_session_*) — third-party app authorization ──
  if (token.startsWith('btcpc_session_')) {
    const sessionStore = require('../services/sessionStore');
    const session = sessionStore.getSession(token);
    if (!session) {
      return res.status(401).json({
        error: { message: 'Session token invalid or expired.', type: 'authentication_error', code: 'session_expired' }
      });
    }
    const remaining = sessionStore.getRemaining(token);
    if (remaining <= 0) {
      return res.status(402).json({
        error: {
          message: `Session spending limit exhausted (${session.spending_limit} BTCPC). Request a new session token.`,
          type: 'billing_error',
          code: 'session_limit_exceeded',
          spent: session.spent,
          spending_limit: session.spending_limit,
        }
      });
    }
    // Attach session info so downstream handlers can record spend
    req.session_token = session;
    // Create a project-like facade so cost deduction works via the existing path
    req.project = {
      _source: 'session_token',
      name: session.app_name,
      owner: session.account,
      wallet_address: session.account,
      balance: remaining,
      verified: true,
      isActive: true,
      totalSpent: session.spent,
      totalRequests: 0,
      save: async function () {
        // Record the spend delta back into the session store
        const newSpent = this.totalSpent;
        if (newSpent > session.spent) {
          const delta = newSpent - session.spent;
          sessionStore.recordSpend(token, delta);
        }
      }
    };
    return next();
  }

  // Resolve btcpc_ project keys — secretStore first, Mongo fallback
  if (token.startsWith('btcpc_')) {
    const project = await _findProjectByApiKey(token);
    if (!project) {
      return res.status(401).json({
        error: { message: 'Invalid or deactivated API key.', type: 'authentication_error', code: 'invalid_api_key' }
      });
    }
    if (!project.verified) {
      return res.status(403).json({
        error: { message: 'Project not verified. See /api/projects/verify.', type: 'authorization_error', code: 'unverified' }
      });
    }
    if (project.balance <= 0) {
      return res.status(402).json({
        error: { message: `Insufficient project balance (${project.balance} BTCPC). Fund your project wallet.`, type: 'billing_error', code: 'insufficient_balance' }
      });
    }
    req.project = project;
    return next();
  }

  // Non-btcpc_ tokens: reject on inference endpoints, allow on read-only
  // Read-only paths (models, pricing, network) don't require project auth
  const readOnlyPaths = ['/v1/models', '/v1/pricing', '/v1/network/models'];
  if (readOnlyPaths.some(p => req.path.startsWith(p))) {
    return next();
  }

  // Node-operator paths: accept standard JWT (logged-in desktop user pulling models)
  const jwtAllowedPaths = ['/v1/node/pull-model', '/v1/node/model-info', '/v1/agent'];
  if (jwtAllowedPaths.some(p => req.path.startsWith(p))) {
    try {
      const jwtLib = require('jsonwebtoken');
      const decoded = jwtLib.verify(token, process.env.JWT_SECRET || process.env.BTCPC_JWT_SECRET);
      req.jwtUser = decoded.username || decoded.id;
      return next();
    } catch (_) {
      return res.status(401).json({
        error: { message: 'Valid login JWT required to pull models.', type: 'authentication_error', code: 'invalid_token' }
      });
    }
  }

  return res.status(401).json({
    error: {
      message: 'Valid btcpc_ API key required for inference. Register at /api/projects/register.',
      type: 'authentication_error',
      code: 'api_key_required'
    }
  });
}

router.use(authenticateBearer);

/**
 * GET /v1/models
 * Lists available models by querying Ollama's /api/tags endpoint.
 * Returns an OpenAI-compatible model list.
 */
router.get('/v1/models', async (req, res) => {
  try {
    const response = await axios.get(`${OLLAMA_URL}/api/tags`, { timeout: 10000 });
    const ollamaModels = response.data.models || [];

    const models = ollamaModels.map(m => ({
      id: m.name,
      object: 'model',
      created: Math.floor(new Date(m.modified_at || Date.now()).getTime() / 1000),
      owned_by: 'btcpc'
    }));

    res.json({
      object: 'list',
      data: models
    });
  } catch (err) {
    console.error('[BTCPC Inference] Failed to list models:', err.message);
    res.status(502).json({
      error: {
        message: 'Failed to retrieve model list from inference backend.',
        type: 'server_error',
        code: 'backend_unreachable'
      }
    });
  }
});

/**
 * POST /v1/node/pull-model
 * Pull an Ollama model to the local inference backend.
 * Streams newline-delimited JSON progress so callers can show a progress bar.
 * Requires a valid btcpc_ API key or JWT session (same auth as inference).
 */
router.post('/v1/node/pull-model', async (req, res) => {
  const model = (req.body && req.body.model || '').trim();
  if (!model || model.length > 200 || !/^[a-zA-Z0-9_.:/-]+$/.test(model)) {
    return res.status(400).json({ error: { message: 'Invalid model name', code: 'invalid_model' } });
  }

  res.setHeader('Content-Type', 'application/x-ndjson');
  res.setHeader('Transfer-Encoding', 'chunked');
  res.setHeader('Cache-Control', 'no-cache');

  try {
    const pullResp = await axios.post(
      `${OLLAMA_URL}/api/pull`,
      { name: model, stream: true },
      { responseType: 'stream', timeout: 30 * 60 * 1000 }
    );

    pullResp.data.on('data', chunk => {
      try { res.write(chunk); } catch (_) {}
    });
    pullResp.data.on('end', () => res.end());
    pullResp.data.on('error', err => {
      try { res.write(JSON.stringify({ error: err.message }) + '\n'); res.end(); } catch (_) {}
    });
  } catch (err) {
    res.write(JSON.stringify({ error: err.message || 'Pull failed' }) + '\n');
    res.end();
  }
});

/**
 * GET /v1/node/model-info?model=<name>
 * Returns Ollama model digest + storage path for security verification.
 * No billing — read-only metadata. Accessible with login JWT.
 */
router.get('/v1/node/model-info', async (req, res) => {
  const model = (req.query.model || '').trim();
  if (!model || !/^[a-zA-Z0-9_.:/-]+$/.test(model)) {
    return res.status(400).json({ error: 'Invalid model name' });
  }
  try {
    const info = await axios.post(`${OLLAMA_URL}/api/show`, { name: model }, { timeout: 10000 });
    const digest = info.data.details && info.data.details.parent_model
      ? info.data.details.parent_model
      : (info.data.digest || null);
    const blobHash = digest ? digest.replace('sha256:', '') : null;
    const os = process.platform;
    const home = process.env.HOME || process.env.USERPROFILE || '~';
    const ollamaDir = os === 'win32'
      ? '%USERPROFILE%\\.ollama\\models'
      : (os === 'darwin' ? home + '/.ollama/models' : home + '/.ollama/models');
    res.json({
      model,
      digest: digest || null,
      blob_hash: blobHash,
      storage_dir: ollamaDir,
      blob_path: blobHash ? ollamaDir + '/blobs/sha256-' + blobHash : null,
      verify_cmd: blobHash
        ? (os === 'win32'
          ? 'certutil -hashfile "' + ollamaDir + '\\blobs\\sha256-' + blobHash + '" SHA256'
          : 'sha256sum "' + ollamaDir + '/blobs/sha256-' + blobHash + '"')
        : null
    });
  } catch (err) {
    res.status(502).json({ error: 'Could not fetch model info from Ollama: ' + err.message });
  }
});

/**
 * GET /v1/pricing
 * Current dynamic pricing. Accepts ?model= to get model-specific pricing.
 */
router.get('/v1/pricing', async (req, res) => {
  try {
    const model = req.query.model || undefined;
    const pricing = await getCurrentPricing(model);
    res.json({
      model: pricing.model,
      tokens_per_btcpc: pricing.tokensPerBtcpc,
      cost_per_token: pricing.costPerToken,
      load_multiplier: pricing.loadMultiplier,
      model_weight: pricing.modelWeight,
      total_multiplier: pricing.totalMultiplier,
      network_load: pricing.load,
      base_rate: pricing.baseRate,
      example: {
        '100_tokens': parseFloat((100 * pricing.costPerToken).toFixed(10)),
        '500_tokens': parseFloat((500 * pricing.costPerToken).toFixed(10)),
        '1000_tokens': parseFloat((1000 * pricing.costPerToken).toFixed(10))
      }
    });
  } catch (err) {
    res.status(500).json({ error: { message: err.message, type: 'server_error' } });
  }
});

/**
 * GET /v1/pricing/bid
 * Calculate what the auto-bid would be for a given model and token count.
 * Query params: model, tokens (default 512)
 */
router.get('/v1/pricing/bid', async (req, res) => {
  try {
    const model = req.query.model || undefined;
    const tokens = parseInt(req.query.tokens) || 512;
    const autoBid = await getAutoBid(model, tokens);
    res.json(autoBid);
  } catch (err) {
    res.status(500).json({ error: { message: err.message, type: 'server_error' } });
  }
});

/**
 * GET /v1/network/models
 * All models available across the mining network + unmet demand.
 */
router.get('/v1/network/models', async (req, res) => {
  try {
    const { getNetworkModels, getUnmetDemand, checkModelAvailability } = require('../services/modelRegistry');
    const models = await getNetworkModels();
    const demand = getUnmetDemand();

    // Add pricing for each available model
    const modelsWithPricing = await Promise.all(models.map(async (m) => {
      const pricing = await getCurrentPricing(m.model);
      return {
        ...m,
        cost_per_token: pricing.costPerToken,
        tokens_per_btcpc: pricing.tokensPerBtcpc,
        model_weight: pricing.modelWeight
      };
    }));

    res.json({
      available: modelsWithPricing,
      wanted: demand,
      total_miners: models.reduce((sum, m) => Math.max(sum, m.miners), 0)
    });
  } catch (err) {
    res.status(500).json({ error: { message: err.message, type: 'server_error' } });
  }
});

/**
 * POST /v1/inference/submit
 * Submit an inference request asynchronously. Returns job_id immediately.
 * Poll GET /v1/inference/:job_id for the result.
 */
router.post('/v1/inference/submit', async (req, res) => {
  const { model, messages, max_tokens, temperature, max_fee, context, mcp_servers, tools, tool_context, use_saved_mcp, local } = req.body;

  if (!messages || !Array.isArray(messages) || messages.length === 0) {
    return res.status(400).json({ error: { message: 'messages is required', type: 'invalid_request_error' } });
  }

  if (!hasMiners()) {
    return res.status(503).json({ error: { message: 'No miners connected', type: 'network_error', code: 'no_miners' } });
  }

  // Per-project rate limit: max 5 concurrent jobs
  if (req.project) {
    const activeJobs = await require('../models/InferenceJob').countDocuments({
      project_id: req.project._id,
      status: { $in: ['pending', 'claimed', 'processing'] }
    });
    if (activeJobs >= 5) {
      return res.status(429).json({
        error: { message: `Too many concurrent jobs (${activeJobs}/5). Wait for current jobs to complete.`, type: 'rate_limit', code: 'too_many_jobs' }
      });
    }
  }

  // ── MCP: Call user-specified tool servers to gather context ──
  // Users bring their own servers — no registration, no approval.
  // Pass inline: mcp_servers: [{ url, tools }]
  // Or use saved: use_saved_mcp: true (loads from user profile)
  // Or both — inline servers merge with saved ones.
  // Validate inline MCP server URLs — block internal addresses (SSRF prevention)
  let allServers = [];
  if (mcp_servers) {
    var blocked = ['localhost', '127.0.0.1', '0.0.0.0', '::1', '169.254', '10.', '192.168.', '172.16.', '172.17.'];
    for (var ms of mcp_servers) {
      try {
        var parsed = new URL(ms.url);
        if (blocked.some(b => parsed.hostname.startsWith(b) || parsed.hostname === b)) continue;
        if (!['http:', 'https:'].includes(parsed.protocol)) continue;
        allServers.push(ms);
      } catch (_) {} // skip invalid URLs
    }
  }

  // Load user's saved MCP servers if requested
  if (use_saved_mcp && req.project) {
    try {
      const User = require('../models/User');
      const Project = require('../models/Project');
      const project = await Project.findById(req.project._id);
      if (project) {
        const user = await User.findOne({ username: project.owner });
        if (user && user.mcpServers && user.mcpServers.length > 0) {
          allServers = allServers.concat(user.mcpServers.map(s => ({
            url: s.url, tools: s.tools, name: s.name
          })));
        }
      }
    } catch (_) {}
  }

  let mcpResults = [];
  if (allServers.length > 0 && tools && Array.isArray(tools)) {
    const toolCalls = [];

    for (const toolName of tools) {
      // Find which server provides this tool
      const server = allServers.find(s =>
        s.tools && s.tools.includes(toolName)
      );
      if (!server || !server.url) continue;

      toolCalls.push(
        axios.post(server.url, {
          jsonrpc: '2.0',
          method: 'tools/call',
          params: { name: toolName, arguments: tool_context || {} },
          id: crypto.randomUUID()
        }, { timeout: 15000 })
        .then(r => ({
          tool: toolName,
          result: r.data?.result?.content?.[0]?.text || JSON.stringify(r.data?.result || r.data),
          server: server.url
        }))
        .catch(err => ({
          tool: toolName,
          result: `[Error: ${err.message}]`,
          server: server.url
        }))
      );
    }

    mcpResults = await Promise.all(toolCalls);
  }

  // ── RAG: Build context from explicit context + MCP tool results ──
  let augmentedMessages = [...messages];
  const contextParts = [];

  // Add explicit context documents
  if (context) {
    if (typeof context === 'string') {
      contextParts.push(context);
    } else if (Array.isArray(context)) {
      for (let i = 0; i < context.length; i++) {
        const doc = context[i];
        const source = doc.source ? ` [source: ${doc.source}]` : '';
        const text = doc.text || doc.content || String(doc);
        contextParts.push(`[Document ${i + 1}${source}]\n${text}`);
      }
    }
  }

  // Add MCP tool results as context
  for (const mcpResult of mcpResults) {
    contextParts.push(`[Tool: ${mcpResult.tool}]\n${mcpResult.result}`);
  }

  // Inject combined context as system message
  if (contextParts.length > 0) {
    const ragSystem = {
      role: 'system',
      content: `Use the following context to answer the user's question. If the context doesn't contain relevant information, say so.\n\n${contextParts.join('\n\n')}`
    };

    const existingSystemEnd = augmentedMessages.findLastIndex(m => m.role === 'system');
    if (existingSystemEnd >= 0) {
      augmentedMessages.splice(existingSystemEnd + 1, 0, ragSystem);
    } else {
      augmentedMessages.unshift(ragSystem);
    }
  }

  // ── Auto model picker ──
  // If no model specified, or model is "auto", pick by complexity:
  // small prompts → smallest model on the network
  // medium prompts → mid-sized model
  // big prompts → largest model
  // The auto-picker is model-agnostic — it works with whatever miners are running
  // (qwen, llama, mistral, gemma, deepseek, etc.).
  let selectedModel = model;
  if (!selectedModel || selectedModel === 'auto') {
    const promptLen = augmentedMessages.reduce((sum, m) => sum + (m.content?.length || 0), 0);
    try {
      const { getNetworkModels } = require('../services/modelRegistry');
      const { getModelWeight } = require('../mining/workGenerator');
      const networkModels = await getNetworkModels();
      const ranked = networkModels
        .filter(m => m.miners > 0)
        .map(m => ({ name: m.model, weight: getModelWeight(m.model) }))
        .sort((a, b) => a.weight - b.weight); // smallest → largest

      if (ranked.length > 0) {
        if (promptLen < 200) {
          selectedModel = ranked[0].name; // smallest
        } else if (promptLen < 2000 && ranked.length >= 2) {
          selectedModel = ranked[Math.floor(ranked.length / 2)].name; // middle
        } else {
          selectedModel = ranked[ranked.length - 1].name; // largest
        }
      } else {
        // No miners — fall back to environment default
        selectedModel = process.env.BTCPC_MODEL || 'qwen3.5:27b';
      }
    } catch (err) {
      console.error('[BTCPC] Auto-picker error:', err.message);
      selectedModel = process.env.BTCPC_MODEL || 'qwen3.5:27b';
    }
  }

  // ── Local mode: direct Ollama, no P2P, no rewards ──
  if (local) {
    try {
      const direct = await runDirectOllamaChat({
        model: selectedModel,
        messages: augmentedMessages,
        max_tokens,
        temperature
      });

      return res.json({
        status: 'completed',
        local: true,
        model: selectedModel,
        result_text: direct.text,
        tokens_generated: direct.tokens,
        elapsed_ms: direct.elapsedMs,
        cost: 0,
        message: 'Local inference — no network rewards, no billing.'
      });
    } catch (err) {
      return res.status(500).json({ error: { message: `Local inference failed: ${err.message}`, type: 'inference_error' } });
    }
  }

  // ── Network mode: submit to P2P for miners to process ──
  try {
    let fee = max_fee;
    if (!fee) {
      const autoBid = await getAutoBid(selectedModel, max_tokens || 512);
      fee = autoBid.bid;
    }

    const job = await submitInference({
      model: selectedModel,
      messages: augmentedMessages,
      maxTokens: max_tokens,
      temperature,
      maxFee: fee,
      projectId: req.project?._id
    });

    res.status(202).json({
      job_id: job.job_id,
      status: 'pending',
      model: selectedModel,
      local: false,
      rag: contextParts.length > 0,
      mcp_tools_called: mcpResults.length > 0 ? mcpResults.map(r => r.tool) : undefined,
      message: 'Request submitted to the network. Poll GET /v1/inference/' + job.job_id + ' for the result.'
    });
  } catch (err) {
    res.status(500).json({ error: { message: err.message, type: 'server_error' } });
  }
});

/**
 * GET /v1/inference/:job_id
 * Check the status of an inference job. Returns result when completed.
 */
router.get('/v1/inference/:job_id', async (req, res) => {
  try {
    const job = await getJob(req.params.job_id);
    if (!job) return res.status(404).json({ error: { message: 'Job not found', type: 'not_found' } });

    const response = {
      job_id: job.job_id,
      status: job.status,
      model: job.model,
      created_at: job.created_at
    };

    if (job.status === 'completed') {
      response.result = {
        content: job.result_text,
        tokens: job.tokens_generated,
        elapsed_ms: job.elapsed_ms,
        node: job.node_name
      };
      response.completed_at = job.completed_at;
    }

    if (job.status === 'failed') {
      response.error = job.result_text;
    }

    if (job.status === 'claimed') {
      response.claimed_by = job.claimed_by;
      response.claimed_at = job.claimed_at;
    }

    res.json(response);
  } catch (err) {
    res.status(500).json({ error: { message: err.message, type: 'server_error' } });
  }
});

/**
 * POST /v1/chat/completions
 * OpenAI-compatible chat completions endpoint.
 * Routes inference to Ollama, logs a WorkProof, returns OpenAI-format response.
 */
router.post('/v1/chat/completions', async (req, res) => {
  const { model, messages, max_tokens, temperature, stream, max_fee, local } = req.body;

  // Validate required fields
  if (!messages || !Array.isArray(messages) || messages.length === 0) {
    return res.status(400).json({
      error: {
        message: 'messages is required and must be a non-empty array.',
        type: 'invalid_request_error',
        code: 'invalid_messages'
      }
    });
  }

  const selectedModel = model || 'qwen3.5:27b';

  if (local) {
    try {
      const requestId = `btcpc-${crypto.randomBytes(12).toString('hex')}`;
      const created = Math.floor(Date.now() / 1000);
      const promptTokens = estimateTokens(messages.map(m => m.content || '').join(' '));

      if (stream === true) {
        res.setHeader('Content-Type', 'text/event-stream');
        res.setHeader('Cache-Control', 'no-cache');
        res.setHeader('Connection', 'keep-alive');
        res.flushHeaders();
        return await streamDirectOllamaChat({
          model: selectedModel,
          messages,
          max_tokens,
          temperature,
          res,
          requestId
        });
      }

      const direct = await runDirectOllamaChat({
        model: selectedModel,
        messages,
        max_tokens,
        temperature
      });

      return res.json({
        id: requestId,
        object: 'chat.completion',
        created,
        model: selectedModel,
        choices: [
          {
            index: 0,
            message: {
              role: 'assistant',
              content: direct.text
            },
            finish_reason: 'stop'
          }
        ],
        usage: {
          prompt_tokens: promptTokens,
          completion_tokens: direct.tokens,
          total_tokens: promptTokens + direct.tokens
        },
        btcpc: {
          cost: 0,
          model_weight: getModelWeight(selectedModel),
          local: true,
          elapsed_ms: direct.elapsedMs
        }
      });
    } catch (err) {
      return res.status(500).json({ error: { message: `Local inference failed: ${err.message}`, type: 'inference_error' } });
    }
  }

  // Check if model is available on the network
  const { checkModelAvailability, recordUnmetDemand } = require('../services/modelRegistry');
  const availability = await checkModelAvailability(selectedModel);

  if (!availability.available) {
    // Record demand so miners see what's wanted
    recordUnmetDemand(selectedModel);

    // Broadcast demand to P2P network
    try {
      const p2p = require('../p2p/network');
      const { createMessage } = require('../p2p/protocol');
      p2p.broadcast(createMessage('MODEL_DEMAND', {
        model: selectedModel,
        demand: (availability.demand || 0) + 1,
        timestamp: new Date().toISOString()
      }, p2p.NODE_ID));
    } catch (_) {}

    return res.status(503).json({
      error: {
        message: `No miner on the network currently has ${selectedModel}. Your request has been broadcast — miners with capable hardware may pull this model to earn from future requests.`,
        type: 'model_unavailable',
        code: 'no_capable_miner',
        model: selectedModel,
        demand: (availability.demand || 0) + 1,
        suggestion: 'Try /v1/network/models to see what is currently available.'
      }
    });
  }

  try {
    const requestId = `btcpc-${crypto.randomBytes(12).toString('hex')}`;
    const created = Math.floor(Date.now() / 1000);

    if (stream === true) {
      res.setHeader('Content-Type', 'text/event-stream');
      res.setHeader('Cache-Control', 'no-cache');
      res.setHeader('Connection', 'keep-alive');
      res.flushHeaders();

      if (local) {
        return await streamDirectOllamaChat({
          model: selectedModel,
          messages,
          max_tokens,
          temperature,
          res,
          requestId
        });
      }

      const final = await finalizeChatResult({
        req,
        selectedModel,
        messages,
        max_tokens,
        temperature,
        max_fee
      });

      for (const piece of chunkText(final.assistantContent)) {
        writeSSE(res, sseChunk(requestId, piece, null, selectedModel));
        await sleep(50);
      }
      writeSSE(res, sseChunk(requestId, '', 'stop', selectedModel));
      res.write('data: [DONE]\n\n');
      return res.end();
    }

    if (local) {
      const direct = await runDirectOllamaChat({
        model: selectedModel,
        messages,
        max_tokens,
        temperature
      });

      return res.json({
        id: requestId,
        object: 'chat.completion',
        created,
        model: selectedModel,
        choices: [
          {
            index: 0,
            message: {
              role: 'assistant',
              content: direct.text
            },
            finish_reason: 'stop'
          }
        ],
        usage: {
          prompt_tokens: estimateTokens(messages.map(m => m.content || '').join(' ')),
          completion_tokens: direct.tokens,
          total_tokens: estimateTokens(messages.map(m => m.content || '').join(' ')) + direct.tokens
        },
        btcpc: {
          cost: 0,
          model_weight: getModelWeight(selectedModel),
          local: true,
          elapsed_ms: direct.elapsedMs
        }
      });
    }

    const final = await finalizeChatResult({
      req,
      selectedModel,
      messages,
      max_tokens,
      temperature,
      max_fee
    });

    res.json({
      id: requestId,
      object: 'chat.completion',
      created: created,
      model: selectedModel,
      choices: [
        {
          index: 0,
          message: {
            role: 'assistant',
            content: final.assistantContent
          },
          finish_reason: 'stop'
        }
      ],
      usage: {
        prompt_tokens: final.promptEvalCount,
        completion_tokens: final.evalCount,
        total_tokens: final.promptEvalCount + final.evalCount
      },
      btcpc: {
        cost: final.cost,
        tokens_per_btcpc: final.pricing.tokensPerBtcpc,
        model_weight: final.pricing.modelWeight,
        load_multiplier: final.pricing.loadMultiplier,
        total_multiplier: final.pricing.totalMultiplier,
        network_load: final.pricing.load,
        epoch: final.epochNumber,
        proof_hash: final.proofHash,
        verified: final.verified,
        remaining_balance: req.project ? req.project.balance : undefined
      }
    });

  } catch (err) {
    console.error('[BTCPC Inference] Inference failed:', err.message);

    if (err.code === 'ECONNREFUSED' || err.code === 'ETIMEDOUT') {
      return res.status(502).json({
        error: {
          message: 'Inference backend is unreachable.',
          type: 'server_error',
          code: 'backend_unreachable'
        }
      });
    }

    // Ollama returned an error
    if (err.response && err.response.data) {
      return res.status(err.response.status || 500).json({
        error: {
          message: err.response.data.error || 'Inference backend error.',
          type: 'server_error',
          code: 'backend_error'
        }
      });
    }

    res.status(500).json({
      error: {
        message: 'Internal server error during inference.',
        type: 'server_error',
        code: 'internal_error'
      }
    });
  }
});

/**
 * POST /v1/inference/encrypted
 * End-to-end encrypted inference: prompt encrypted by user, result encrypted by miner.
 * Neither the API node nor any P2P relay ever sees plaintext.
 *
 * Body: {
 *   prompt_encrypted: { ciphertext, iv, tag },  — AES-256-GCM, base64 fields
 *   user_memo_pubkey: string,                    — hex compressed secp256k1 pubkey
 *   miner_account:    string,                    — target miner's BTCPC account name
 *   model:            string (optional),
 *   max_tokens:       number (optional),
 *   temperature:      number (optional)
 * }
 *
 * Response: { result_encrypted: { ciphertext, iv, tag }, job_id, status }
 */
router.post('/v1/inference/encrypted', async (req, res) => {
  const {
    prompt_encrypted,
    user_memo_pubkey,
    miner_account,
    model,
    max_tokens,
    temperature,
  } = req.body;

  // Validate required fields
  if (!prompt_encrypted || !prompt_encrypted.ciphertext || !prompt_encrypted.iv || !prompt_encrypted.tag) {
    return res.status(400).json({
      error: {
        message: 'prompt_encrypted is required and must have ciphertext, iv, tag',
        type: 'invalid_request_error',
        code: 'missing_encrypted_prompt'
      }
    });
  }
  if (!user_memo_pubkey) {
    return res.status(400).json({
      error: {
        message: 'user_memo_pubkey is required so the miner can derive the shared secret',
        type: 'invalid_request_error',
        code: 'missing_user_memo_pubkey'
      }
    });
  }
  if (!miner_account) {
    return res.status(400).json({
      error: {
        message: 'miner_account is required to route the encrypted job',
        type: 'invalid_request_error',
        code: 'missing_miner_account'
      }
    });
  }

  // Verify miner account exists and has a memo public key
  const stateStore = require('../chain/stateStore');
  const minerInfo = stateStore.getAccount(miner_account);
  if (!minerInfo) {
    return res.status(404).json({
      error: {
        message: `Miner account "${miner_account}" not found on chain`,
        type: 'not_found',
        code: 'miner_not_found'
      }
    });
  }
  if (!minerInfo.public_keys || !minerInfo.public_keys.memo) {
    return res.status(422).json({
      error: {
        message: `Miner account "${miner_account}" has no memo public key registered`,
        type: 'invalid_request_error',
        code: 'missing_miner_memo_key'
      }
    });
  }

  if (!hasMiners()) {
    return res.status(503).json({
      error: {
        message: 'No miners connected to the P2P network',
        type: 'network_error',
        code: 'no_miners',
        peers: peerCount()
      }
    });
  }

  try {
    const jobId = 'enc_' + crypto.randomBytes(16).toString('hex');

    // Broadcast encrypted job via P2P — miners see only opaque ciphertext
    const { createMessage } = require('../p2p/protocol');
    const p2p = require('../p2p/network');
    const reqMsg = createMessage('INFERENCE_REQUEST', {
      request_id: jobId,
      model: model || 'auto',
      encrypted: true,
      max_fee: 0,
      max_tokens: max_tokens || 1024,
      temperature: temperature || 0.7,
      redundancy: 1
    }, p2p.NODE_ID);
    p2p.broadcast(reqMsg);

    // Send encrypted payload — miner_account tells the target miner it's theirs
    const payloadMsg = createMessage('INFERENCE_PAYLOAD', {
      request_id: jobId,
      encrypted: true,
      prompt_encrypted,
      user_memo_pubkey,
      miner_account,
      model: model || 'auto',
      max_tokens: max_tokens || 1024,
      temperature: temperature || 0.7,
    }, p2p.NODE_ID);
    setTimeout(() => p2p.broadcast(payloadMsg), 500);

    // Poll for the encrypted result
    const job = await pollInferenceJob(jobId, 300000, 2000);

    if (!job || job.status !== 'completed') {
      const status = job && job.status ? job.status : 'unknown';
      return res.status(504).json({
        error: {
          message: `Encrypted inference not completed. Job status: ${status}`,
          type: 'timeout',
          code: 'inference_pending',
          job_id: jobId
        }
      });
    }

    // result_encrypted is set by the miner in handlePayload for encrypted jobs
    if (!job.result_encrypted) {
      return res.status(502).json({
        error: {
          message: 'Miner completed job but did not return encrypted result',
          type: 'server_error',
          code: 'missing_encrypted_result',
          job_id: jobId
        }
      });
    }

    return res.json({
      job_id: jobId,
      status: 'completed',
      result_encrypted: job.result_encrypted,
    });
  } catch (err) {
    console.error('[BTCPC Inference] Encrypted inference error:', err.message);
    return res.status(500).json({
      error: { message: err.message, type: 'server_error', code: 'internal_error' }
    });
  }
});

// ---------------------------------------------------------------------------
// Agent Protocol — POST /v1/agent/session, /v1/agent/turn, DELETE /v1/agent/session/:id
// ---------------------------------------------------------------------------

const agentSession = require('./agentSession');
const agentEvents = require('./agentEvents');

/**
 * POST /v1/agent/session
 * Open a new agent session.
 *
 * Body: {
 *   system_prompt: string,
 *   tools: [{ name, description, parameters: { type: "object", properties, required } }],
 *   client_public_key: string (hex secp256k1, optional — can also be sent with first turn)
 * }
 *
 * Returns: { session_id, server_public_key, expires_at }
 */
router.post('/v1/agent/session', async (req, res) => {
  try {
    const { system_prompt, tools, client_public_key } = req.body || {};
    if (!system_prompt && (!tools || tools.length === 0)) {
      return res.status(400).json({
        error: { message: 'system_prompt or tools required', type: 'invalid_request', code: 'missing_params' }
      });
    }
    const result = agentSession.createAgentSession({
      system_prompt: system_prompt || '',
      tools: tools || [],
      client_public_key: client_public_key || null,
    });
    return res.json(result);
  } catch (err) {
    console.error('[BTCPC Agent] Session create error:', err.message);
    return res.status(500).json({ error: { message: err.message, type: 'server_error', code: 'internal_error' } });
  }
});

/**
 * POST /v1/agent/turn
 * Submit a user turn and get the assistant response.
 *
 * Plaintext mode (for local / same-network clients):
 *   Body: { session_id, client_public_key?, message: string }
 *   Response: { session_id, role: "assistant", content: string, tool_calls?: [...] }
 *
 * Encrypted mode (for privacy):
 *   Body: { session_id, client_public_key?, encrypted_turn: hex }
 *   encrypted_turn decrypts to JSON: { message: string }
 *   Response: { session_id, encrypted_response: hex }
 *   encrypted_response decrypts to JSON: { role: "assistant", content: string, tool_calls?: [...] }
 *
 * If the model returns tool_use blocks, the response includes tool_calls and
 * the client should re-submit the turn with tool results via the same endpoint:
 *   Body: { session_id, tool_results: [{ call_id, result }] }
 */
router.post('/v1/agent/turn', async (req, res) => {
  try {
    const { session_id, client_public_key, message, encrypted_turn, tool_results } = req.body || {};
    if (!session_id) {
      return res.status(400).json({ error: { message: 'session_id required', type: 'invalid_request', code: 'missing_session_id' } });
    }

    const session = agentSession.getAgentSession(session_id);
    if (!session) {
      return res.status(404).json({ error: { message: 'Session not found or expired', type: 'invalid_request', code: 'session_expired' } });
    }

    // Derive key if client sends public key
    if (client_public_key && !session.key) {
      agentSession.deriveAgentSessionKey(session_id, client_public_key);
    }

    let userMessage = message || null;

    // Decrypt encrypted_turn if provided
    if (encrypted_turn && session.key) {
      const blob = Buffer.from(encrypted_turn, 'hex');
      const plain = agentSession.decrypt(session.key, blob);
      const parsed = JSON.parse(plain.toString('utf8'));
      userMessage = parsed.message;
    }

    // Handle tool_results submission (continuing a paused turn)
    if (tool_results && Array.isArray(tool_results)) {
      for (const tr of tool_results) {
        agentSession.deliverToolResult(session_id, tr.call_id, tr.result, tr.error || null);
      }
      // Add tool results to history
      agentSession.addTurn(session_id, 'tool', JSON.stringify(tool_results));
    }

    if (!userMessage && !tool_results) {
      return res.status(400).json({ error: { message: 'message or tool_results required', type: 'invalid_request', code: 'missing_message' } });
    }

    // Append user turn to history
    if (userMessage) {
      agentSession.addTurn(session_id, 'user', userMessage);
    }

    // Build Ollama request
    const messages = agentSession.buildMessages(session_id, null); // history already has the new user turn
    const ollamaModel = process.env.BTCPC_MODEL || 'qwen2.5:0.5b';
    const ollamaBase = process.env.OLLAMA_HOST || 'http://localhost:11434';

    const reqBody = {
      model: ollamaModel,
      messages: messages,
      stream: false,
    };

    // Attach tool definitions if session has them
    if (session.tools && session.tools.length > 0) {
      reqBody.tools = session.tools;
    }

    const ollamaResp = await axios.post(ollamaBase + '/api/chat', reqBody, { timeout: 120000 });
    const ollamaData = ollamaResp.data;

    const assistantMsg = ollamaData.message || {};
    const content = assistantMsg.content || '';
    const toolCalls = assistantMsg.tool_calls || null;

    // Append assistant response to history
    agentSession.addTurn(session_id, 'assistant', content || JSON.stringify(assistantMsg));

    // If the model returned tool calls, broadcast TOOL_CALL via P2P and include in response
    let p2pToolCalls = null;
    if (toolCalls && toolCalls.length > 0) {
      p2pToolCalls = toolCalls.map(tc => {
        const callId = crypto.randomBytes(8).toString('hex');
        // Broadcast to P2P so any subscriber can receive the tool call
        try {
          const protocol = require('../p2p/protocol');
          const network = require('../p2p/network');
          const ctx = network.getContext ? network.getContext() : null;
          if (ctx) {
            const toolCallMsg = protocol.createMessage(protocol.MESSAGE_TYPES.TOOL_CALL, {
              session_id: session_id,
              call_id: callId,
              tool_name: tc.function ? tc.function.name : (tc.name || 'unknown'),
              arguments: tc.function ? tc.function.arguments : (tc.arguments || {}),
            }, ctx.NODE_ID);
            ctx.broadcast(toolCallMsg);
          }
        } catch (_) { /* P2P not available in all deployments */ }
        return {
          call_id: callId,
          tool_name: tc.function ? tc.function.name : (tc.name || 'unknown'),
          arguments: tc.function ? tc.function.arguments : (tc.arguments || {}),
        };
      });
    }

    // Build response
    const responseData = {
      session_id: session_id,
      role: 'assistant',
      content: content,
      tool_calls: p2pToolCalls || undefined,
      model: ollamaModel,
      turn_count: session.history.length,
    };

    // Encrypt response if session has a derived key
    if (session.key) {
      const encBuf = agentSession.encrypt(session.key, Buffer.from(JSON.stringify(responseData), 'utf8'));
      return res.json({ session_id: session_id, encrypted_response: encBuf.toString('hex') });
    }

    return res.json(responseData);
  } catch (err) {
    console.error('[BTCPC Agent] Turn error:', err.message);
    return res.status(500).json({ error: { message: err.message, type: 'server_error', code: 'internal_error' } });
  }
});

/**
 * DELETE /v1/agent/session/:id
 * Close and destroy an agent session, zeroing its key material.
 */
router.delete('/v1/agent/session/:id', async (req, res) => {
  try {
    agentSession.closeAgentSession(req.params.id);
    return res.json({ ok: true, session_id: req.params.id });
  } catch (err) {
    return res.status(500).json({ error: { message: err.message, type: 'server_error', code: 'internal_error' } });
  }
});

/**
 * GET /v1/agent/sessions
 * Return active session count (admin info, no session content).
 */
router.get('/v1/agent/sessions', async (req, res) => {
  return res.json({ active_sessions: agentSession.getSessionCount() });
});

module.exports = router;
