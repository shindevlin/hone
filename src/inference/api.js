"use strict";

const express = require('express');
const crypto = require('crypto');
const axios = require('axios');
const WorkProof = require('../models/WorkProof');
const { getCurrentEpoch } = require('../services/epochManager');
const { getModelWeight, OLLAMA_URL } = require('../mining/workGenerator');

const router = express.Router();

/**
 * Bearer token authentication middleware.
 * Accepts any non-empty token for now.
 */
function authenticateBearer(req, res, next) {
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
  req.apiKey = authHeader.slice(7).trim();
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
 * POST /v1/chat/completions
 * OpenAI-compatible chat completions endpoint.
 * Routes inference to Ollama, logs a WorkProof, returns OpenAI-format response.
 */
router.post('/v1/chat/completions', async (req, res) => {
  const { model, messages, max_tokens, temperature, stream } = req.body;

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
    // Build Ollama request
    const ollamaPayload = {
      model: selectedModel,
      messages: messages,
      stream: false,
      options: {}
    };

    if (typeof temperature === 'number') {
      ollamaPayload.options.temperature = temperature;
    }
    if (typeof max_tokens === 'number') {
      ollamaPayload.options.num_predict = max_tokens;
    }

    const startTime = Date.now();

    const ollamaResponse = await axios.post(
      `${OLLAMA_URL}/api/chat`,
      ollamaPayload,
      { timeout: 180000 }
    );

    const data = ollamaResponse.data;
    const assistantContent = (data.message && data.message.content) || '';
    const evalCount = data.eval_count || estimateTokens(assistantContent);
    const promptEvalCount = data.prompt_eval_count || estimateTokens(
      messages.map(m => m.content || '').join(' ')
    );

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

    // Estimate cost: 0.001 BTCPC per 100 tokens of output (placeholder pricing)
    const cost = (evalCount / 100 * 0.001).toFixed(8);

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
          finish_reason: data.done_reason || 'stop'
        }
      ],
      usage: {
        prompt_tokens: promptEvalCount,
        completion_tokens: evalCount,
        total_tokens: promptEvalCount + evalCount
      },
      btcpc: {
        cost: cost,
        epoch: epochNumber,
        proof_hash: proofHash,
        verified: verified
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
