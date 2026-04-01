"use strict";

const express = require('express');
const crypto = require('crypto');
const axios = require('axios');
const WorkProof = require('../models/WorkProof');
const Project = require('../models/Project');
const { getCurrentEpoch } = require('../services/epochManager');
const { getModelWeight } = require('../mining/workGenerator');
const { calculateCost, getCurrentPricing, getAutoBid } = require('../services/pricing');
const { requestInference, hasMiners, peerCount } = require('./p2pRouter');

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
  }

  next();
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
        '100_tokens': parseFloat((100 * pricing.costPerToken).toFixed(8)),
        '500_tokens': parseFloat((500 * pricing.costPerToken).toFixed(8)),
        '1000_tokens': parseFloat((1000 * pricing.costPerToken).toFixed(8))
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

    // Calculate bid — use requester's max_fee or auto-bid
    let fee = max_fee;
    if (!fee) {
      const autoBid = await getAutoBid(selectedModel, max_tokens || 512);
      fee = autoBid.bid;
      console.log(`[BTCPC Inference] Auto-bid: ${fee} BTCPC (coverage: ${autoBid.block_reward_coverage}, multiplier: ${autoBid.bid_multiplier})`);
    }

    console.log(`[BTCPC Inference] Routing via P2P (${selectedModel}, fee: ${fee})`);
    const result = await requestInference({
      model: selectedModel,
      messages,
      maxTokens: max_tokens,
      temperature,
      maxFee: fee
    });

    const assistantContent = result.content || '';
    const evalCount = result.tokens || estimateTokens(assistantContent);
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
