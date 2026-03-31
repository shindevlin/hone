"use strict";

/**
 * P2P Inference Router
 *
 * Routes inference requests through the P2P network instead of
 * calling Ollama directly. The API broadcasts INFERENCE_REQUEST,
 * miners claim and process it, and the result comes back via
 * INFERENCE_RESULT message.
 */

const crypto = require("crypto");
const p2p = require("../p2p/network");
const { createMessage } = require("../p2p/protocol");

// Pending requests waiting for results
// Map<requestId, { resolve, reject, timer, startTime }>
const pendingRequests = new Map();

const REQUEST_TIMEOUT_MS = parseInt(process.env.BTCPC_INFERENCE_TIMEOUT) || 30000; // 30s P2P, then fallback

/**
 * Initialize the P2P result listener.
 * Call once after P2P network is started.
 */
function initP2PRouter() {
  p2p.onMessage(async (msg) => {
    if (!msg) return;

    // Log all inference messages for debugging
    if (msg.type && msg.type.startsWith('INFERENCE_')) {
      const data = msg.data || msg;
      console.log(`[BTCPC P2P Router] Received ${msg.type} | request_id: ${data.request_id || 'none'} | pending: ${pendingRequests.size}`);
    }

    if (msg.type !== 'INFERENCE_RESULT') return;

    const data = msg.data || msg;
    const requestId = data.request_id;
    if (!requestId) {
      console.log('[BTCPC P2P Router] INFERENCE_RESULT with no request_id, keys:', Object.keys(data));
      return;
    }

    const pending = pendingRequests.get(requestId);
    if (!pending) {
      console.log(`[BTCPC P2P Router] No pending request for ${requestId.slice(0, 12)}`);
      return;
    }

    clearTimeout(pending.timer);
    pendingRequests.delete(requestId);

    if (data.error) {
      pending.reject(new Error(data.error));
    } else {
      pending.resolve({
        content: data.result_text || '',
        tokens: data.tokens_generated || 0,
        model: data.model || 'unknown',
        elapsed_ms: data.elapsed_ms || (Date.now() - pending.startTime),
        node: data.node_name || 'unknown',
        result_hash: data.result_hash || null
      });
    }
  });

  console.log('[BTCPC P2P Router] Listening for inference results');
}

/**
 * Send an inference request via P2P and wait for the result.
 *
 * @param {Object} options
 * @param {string} options.model - Model name
 * @param {Array} options.messages - OpenAI-format messages
 * @param {number} [options.maxTokens] - Max tokens
 * @param {number} [options.temperature] - Temperature
 * @param {number} [options.maxFee] - Max BTCPC willing to pay
 * @returns {Promise<Object>} Inference result
 */
function requestInference({ model, messages, maxTokens, temperature, maxFee }) {
  return new Promise((resolve, reject) => {
    const requestId = 'req_' + crypto.randomBytes(16).toString('hex');

    // Build the prompt from messages
    const prompt = messages.map(m => {
      if (m.role === 'system') return `System: ${m.content}`;
      if (m.role === 'assistant') return `Assistant: ${m.content}`;
      return m.content;
    }).join('\n\n');

    const promptHash = crypto.createHash('sha256').update(prompt).digest('hex');

    // Set timeout
    const timer = setTimeout(() => {
      pendingRequests.delete(requestId);
      reject(new Error(`Inference request timed out after ${REQUEST_TIMEOUT_MS / 1000}s. No miner responded.`));
    }, REQUEST_TIMEOUT_MS);

    // Store pending request
    pendingRequests.set(requestId, { resolve, reject, timer, startTime: Date.now() });

    // Broadcast INFERENCE_REQUEST to the network
    const msg = createMessage('INFERENCE_REQUEST', {
      request_id: requestId,
      model: model || 'qwen3.5:27b',
      prompt_hash: promptHash,
      max_fee: maxFee || 10,
      max_tokens: maxTokens || 1024,
      temperature: temperature || 0.7,
      redundancy: 1
    }, p2p.NODE_ID);

    p2p.broadcast(msg);

    // Also broadcast the payload immediately (single-step for now,
    // full claim/assign flow for multi-miner later)
    const payload = createMessage('INFERENCE_PAYLOAD', {
      request_id: requestId,
      prompt: prompt,
      model: model || 'qwen3.5:27b',
      max_tokens: maxTokens || 1024,
      temperature: temperature || 0.7
    }, p2p.NODE_ID);

    // Small delay to let claim happen, then send payload
    setTimeout(() => {
      p2p.broadcast(payload);
    }, 500);

    console.log(`[BTCPC P2P Router] Sent inference request ${requestId.slice(0, 12)} (model: ${model})`);
  });
}

/**
 * Check if we have P2P peers connected (can route requests).
 */
function hasMiners() {
  return p2p.peers && p2p.peers.size > 0;
}

/**
 * Get count of connected peers.
 */
function peerCount() {
  return p2p.peers ? p2p.peers.size : 0;
}

module.exports = { initP2PRouter, requestInference, hasMiners, peerCount };
