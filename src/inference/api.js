"use strict";

const express = require('express');
const crypto = require('crypto');
const axios = require('axios');
const WorkProof = require('../models/WorkProof');
const Project = require('../models/Project');
const { getCurrentEpoch } = require('../services/epochManager');
const { getModelWeight } = require('../mining/workGenerator');
const { calculateCost, getCurrentPricing, getAutoBid } = require('../services/pricing');
const { submitInference, getJob, hasMiners, peerCount } = require('./p2pRouter');

const router = express.Router();

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

  // Resolve btcpc_ project keys
  if (token.startsWith('btcpc_')) {
    const project = await Project.findOne({ apiKey: token, isActive: true });
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
  let allServers = mcp_servers ? [...mcp_servers] : [];

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
  // If no model specified, or model is "auto", pick based on prompt complexity
  let selectedModel = model;
  if (!selectedModel || selectedModel === 'auto') {
    const promptLen = augmentedMessages.reduce((sum, m) => sum + (m.content?.length || 0), 0);
    if (promptLen < 200) {
      selectedModel = 'qwen3:4b';       // simple tasks → small model, cheap
    } else if (promptLen < 2000) {
      selectedModel = 'qwen3.5:9b';     // medium tasks → mid model
    } else {
      selectedModel = 'qwen3.5:27b';    // complex / long context → big model
    }
  }

  // ── Local mode: direct Ollama, no P2P, no rewards ──
  if (local) {
    try {
      const OLLAMA_URL = process.env.OLLAMA_URL || 'http://localhost:11434';
      const prompt = augmentedMessages.map(m => {
        if (m.role === 'system') return `System: ${m.content}`;
        if (m.role === 'assistant') return `Assistant: ${m.content}`;
        return m.content;
      }).join('\n\n');

      const ollamaRes = await axios.post(`${OLLAMA_URL}/api/generate`, {
        model: selectedModel,
        prompt,
        stream: false,
        options: { temperature: temperature || 0.7, num_predict: max_tokens || 512 }
      }, { timeout: 300000 });

      return res.json({
        status: 'completed',
        local: true,
        model: selectedModel,
        result_text: ollamaRes.data.response || '',
        tokens_generated: ollamaRes.data.eval_count || 0,
        elapsed_ms: ollamaRes.data.total_duration ? Math.round(ollamaRes.data.total_duration / 1e6) : 0,
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
  const { model, messages, max_tokens, temperature, stream, max_fee } = req.body;

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

  // Streaming not yet supported
  if (stream === true) {
    return res.status(400).json({
      error: {
        message: 'Streaming is not yet supported. Set stream to false.',
        type: 'invalid_request_error',
        code: 'streaming_not_supported'
      }
    });
  }

  try {
    const startTime = Date.now();

    if (!hasMiners()) {
      return res.status(503).json({
        error: {
          message: 'No miners connected to the P2P network. Inference requires at least one active miner.',
          type: 'network_error',
          code: 'no_miners',
          peers: peerCount()
        }
      });
    }

    // Calculate bid
    let fee = max_fee;
    if (!fee) {
      const autoBid = await getAutoBid(selectedModel, max_tokens || 512);
      fee = autoBid.bid;
    }

    // Submit async job
    const { job_id } = await submitInference({
      model: selectedModel,
      messages,
      maxTokens: max_tokens,
      temperature,
      maxFee: fee,
      projectId: req.project?._id
    });

    // Poll until complete (OpenAI compat — client expects sync response)
    const MAX_WAIT = 300000; // 5 min
    const POLL_INTERVAL = 2000; // 2s
    let job = null;

    while (Date.now() - startTime < MAX_WAIT) {
      job = await getJob(job_id);
      if (job && (job.status === 'completed' || job.status === 'failed')) break;
      await new Promise(r => setTimeout(r, POLL_INTERVAL));
    }

    if (!job || job.status !== 'completed') {
      const status = job?.status || 'unknown';
      return res.status(504).json({
        error: {
          message: `Inference not completed. Job status: ${status}. Check GET /v1/inference/${job_id}`,
          type: 'timeout',
          code: 'inference_pending',
          job_id
        }
      });
    }

    const assistantContent = job.result_text || '';
    const evalCount = job.tokens_generated || estimateTokens(assistantContent);
    const promptEvalCount = estimateTokens(messages.map(m => m.content || '').join(' '));

    // Compute hashes for work proof
    const promptText = messages.map(m => `${m.role}:${m.content || ''}`).join('|');
    const promptHash = crypto.createHash('sha256').update(promptText).digest('hex');
    const resultHash = crypto.createHash('sha256').update(assistantContent).digest('hex');
    const weightFactor = getModelWeight(selectedModel);
    const workValue = evalCount * weightFactor;

    // Log as WorkProof
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
      // Still return the inference result even if proof logging fails
      proofHash = crypto.createHash('sha256')
        .update(promptHash + resultHash + Date.now().toString())
        .digest('hex');
    }

    // Build OpenAI-compatible response
    const requestId = `btcpc-${crypto.randomBytes(12).toString('hex')}`;
    const created = Math.floor(Date.now() / 1000);

    // Dynamic pricing based on network load
    const { cost, pricing } = await calculateCost(evalCount, selectedModel);

    // Deduct from project balance if this is a project API key request
    if (req.project) {
      req.project.balance = Math.max(0, req.project.balance - cost);
      req.project.totalSpent += cost;
      req.project.totalRequests += 1;
      await req.project.save();
    }

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
            content: assistantContent
          },
          finish_reason: 'stop'
        }
      ],
      usage: {
        prompt_tokens: promptEvalCount,
        completion_tokens: evalCount,
        total_tokens: promptEvalCount + evalCount
      },
      btcpc: {
        cost,
        tokens_per_btcpc: pricing.tokensPerBtcpc,
        model_weight: pricing.modelWeight,
        load_multiplier: pricing.loadMultiplier,
        total_multiplier: pricing.totalMultiplier,
        network_load: pricing.load,
        epoch: epochNumber,
        proof_hash: proofHash,
        verified: verified,
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
 * Rough token estimate when eval_count is not available.
 */
function estimateTokens(text) {
  return Math.max(1, Math.ceil(text.length / 4));
}

module.exports = router;
