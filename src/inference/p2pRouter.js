"use strict";

/**
 * P2P Inference Router — Async Model
 *
 * Requests are submitted to the blockchain, stored in stateStore, and
 * broadcast via P2P. Miners claim and process them. Results come
 * back via INFERENCE_RESULT and update the job in stateStore.
 *
 * No timeouts. No long-polling. Submit and check back.
 *
 * Phase E: InferenceJob Mongoose model removed. Jobs tracked in stateStore.
 */

const crypto = require("crypto");
const p2p = require("../p2p/network");
const { createMessage } = require("../p2p/protocol");
const stateStore = require("../chain/stateStore");

// Track request IDs we originated to avoid re-processing our own echoes
const originatedRequests = new Set();

// In-memory job store (stateStore doesn't have a jobs map yet; this bridges the gap)
// key: job_id → { job_id, status, model, messages, result_text, tokens_generated,
//                 elapsed_ms, node_name, claimed_by, cost, project_id, completed_at, ... }
const jobStore = new Map();

/**
 * Initialize the P2P result listener.
 */
function initP2PRouter() {
  p2p.onMessage(async (msg) => {
    if (!msg) return;

    const data = msg.data || msg;
    const reqId = data.request_id;

    if (msg.type === 'INFERENCE_CLAIM' && reqId) {
      const job = jobStore.get(reqId);
      if (job && job.status === 'pending') {
        job.status = 'claimed';
        job.claimed_by = data.node_name || data.node_id;
        job.claimed_at = new Date().toISOString();
      }
    }

    if (msg.type === 'INFERENCE_RESULT' && reqId) {
      if (data.error) {
        const job = jobStore.get(reqId);
        if (!job) return;

        // If the job is claimed by a specific miner, only that miner can fail it
        if (job.claimed_by && data.node_name && job.claimed_by !== data.node_name) {
          return; // Different miner failed — ignore
        }

        if (job.status === 'pending') {
          return; // don't fail unclaimed jobs
        }

        if (job.claimed_by === data.node_name && ['claimed', 'processing'].includes(job.status)) {
          job.status = 'failed';
          job.result_text = data.error;
          job.completed_at = new Date().toISOString();

          // Refund pre-deducted cost on failure
          if (job.project_id && job.cost > 0) {
            try {
              const Project = require('../models/Project');
              await Project.findByIdAndUpdate(job.project_id, { $inc: { balance: job.cost } });
              console.log(`[BTCPC P2P Router] Refunded ${job.cost} BTCPC to project`);
            } catch (_) {}
          }
          console.log(`[BTCPC P2P Router] Job ${reqId.slice(0, 12)} failed by ${data.node_name}: ${data.error}`);
        }
        return;
      }

      // Complete the job
      let job = jobStore.get(reqId);

      if (!job) {
        // Create it (authority may not have the job locally)
        job = {
          job_id: reqId,
          status: 'pending',
          model: data.model || 'unknown',
          messages: [],
          cost: 0,
          project_id: null
        };
        jobStore.set(reqId, job);
        console.log(`[BTCPC P2P Router] Created job ${reqId.slice(0, 12)} from P2P result (authority sync)`);
      }

      if (['pending', 'claimed', 'processing'].includes(job.status)) {
        job.status = 'completed';
        job.result_text = data.result_text || '';
        job.result_hash = data.result_hash || null;
        job.tokens_generated = data.tokens_generated || 0;
        job.elapsed_ms = data.elapsed_ms || 0;
        job.node_name = data.node_name || 'unknown';
        job.completed_at = new Date().toISOString();
        // Preserve encrypted result for encrypted inference jobs
        if (data.encrypted && data.result_encrypted) {
          job.result_encrypted = data.result_encrypted;
        }

        // Reconcile billing — refund difference between estimated and actual cost
        if (job.project_id && job.cost > 0) {
          try {
            const Project = require('../models/Project');
            const { calculateCost } = require('../services/pricing');
            const actual = await calculateCost(job.tokens_generated, job.model);
            const actualCost = actual.cost;
            const diff = job.cost - actualCost;

            if (diff > 0) {
              await Project.findByIdAndUpdate(job.project_id, { $inc: { balance: diff } });
            }
            await Project.findByIdAndUpdate(job.project_id, { $inc: { totalSpent: actualCost } });
            job.cost = actualCost;
          } catch (_) {}
        }

        console.log(`[BTCPC P2P Router] Job ${reqId.slice(0, 12)} completed: ${job.tokens_generated} tokens, ${job.elapsed_ms}ms`);
      }
    }
  });

  console.log('[BTCPC P2P Router] Listening for inference results (async model)');
}

/**
 * Submit an inference request to the blockchain.
 * Returns the job_id immediately — caller polls for result.
 *
 * @param {Object} options
 * @returns {Promise<Object>} { job_id, status }
 */
async function submitInference({ model, messages, maxTokens, temperature, maxFee, projectId, toolTraceHash, toolsUsed }) {
  const jobId = 'req_' + crypto.randomBytes(16).toString('hex');

  // Build prompt for P2P broadcast
  const prompt = messages.map(m => {
    if (m.role === 'system') return `System: ${m.content}`;
    if (m.role === 'assistant') return `Assistant: ${m.content}`;
    return m.content;
  }).join('\n\n');

  const promptHash = crypto.createHash('sha256').update(prompt).digest('hex');

  // Lock funds in escrow — deducts from the permanent ledger
  let estimatedCost = 0;
  if (projectId) {
    try {
      const Project = require('../models/Project');
      const { calculateCost } = require('../services/pricing');
      const escrow = require('../services/escrow');
      const estimated = await calculateCost(maxTokens || 1024, model);
      estimatedCost = estimated.cost;

      const project = await Project.findById(projectId);
      if (project) {
        // Lock funds via escrow
        const payerAccount = project.repo || project.owner;
        try {
          await escrow.lockFunds(jobId, payerAccount, estimatedCost);
        } catch (err) {
          throw new Error(`Escrow lock failed for ${payerAccount}: ${err.message}`);
        }

        project.totalRequests += 1;
        await project.save();
      }
    } catch (err) {
      if (err.message.includes('Escrow lock failed')) throw err;
      // Non-fatal billing errors — proceed anyway
    }
  }

  // Store job in memory
  const job = {
    job_id: jobId,
    status: 'pending',
    model: model || 'qwen3.5:27b',
    messages,
    max_tokens: maxTokens || 1024,
    temperature: temperature || 0.7,
    max_fee: maxFee || 0,
    prompt_hash: promptHash,
    project_id: projectId || null,
    cost: estimatedCost,
    tool_trace_hash: toolTraceHash || null,
    tools_used: toolsUsed && toolsUsed.length > 0 ? toolsUsed : null,
    expires_at: new Date(Date.now() + 600000).toISOString()
  };
  jobStore.set(jobId, job);

  originatedRequests.add(jobId);

  // Broadcast to P2P
  const reqMsg = createMessage('INFERENCE_REQUEST', {
    request_id: jobId,
    model: model || 'qwen3.5:27b',
    prompt_hash: promptHash,
    max_fee: maxFee || 0,
    max_tokens: maxTokens || 1024,
    temperature: temperature || 0.7,
    redundancy: 1
  }, p2p.NODE_ID);
  p2p.broadcast(reqMsg);

  // Send payload after brief delay for claim
  setTimeout(() => {
    const payload = createMessage('INFERENCE_PAYLOAD', {
      request_id: jobId,
      prompt,
      model: model || 'qwen3.5:27b',
      max_tokens: maxTokens || 1024,
      temperature: temperature || 0.7,
      tool_trace_hash: toolTraceHash || null,
      tools_used: toolsUsed && toolsUsed.length > 0 ? toolsUsed : null,
    }, p2p.NODE_ID);
    p2p.broadcast(payload);
  }, 500);

  console.log(`[BTCPC P2P Router] Submitted job ${jobId.slice(0, 12)} (model: ${model})`);

  return { job_id: jobId, status: 'pending' };
}

/**
 * Get job status and result (from in-memory store).
 */
async function getJob(jobId) {
  return jobStore.get(jobId) || null;
}

/**
 * Check if we have P2P peers connected.
 */
function hasMiners() {
  return p2p.peers && p2p.peers.size > 0;
}

function peerCount() {
  return p2p.peers ? p2p.peers.size : 0;
}

module.exports = { initP2PRouter, submitInference, getJob, hasMiners, peerCount };
