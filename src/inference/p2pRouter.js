"use strict";

/**
 * P2P Inference Router — Async Model
 *
 * Requests are submitted to the blockchain, stored in DB, and
 * broadcast via P2P. Miners claim and process them. Results come
 * back via INFERENCE_RESULT and update the job in DB.
 *
 * No timeouts. No long-polling. Submit and check back.
 */

const crypto = require("crypto");
const p2p = require("../p2p/network");
const { createMessage } = require("../p2p/protocol");
const InferenceJob = require("../models/InferenceJob");

// Track request IDs we originated to avoid re-processing our own echoes
const originatedRequests = new Set();

/**
 * Initialize the P2P result listener.
 */
function initP2PRouter() {
  p2p.onMessage(async (msg) => {
    if (!msg) return;

    const data = msg.data || msg;
    const reqId = data.request_id;

    if (msg.type === 'INFERENCE_CLAIM' && reqId) {
      // Miner claimed our job — update status
      try {
        await InferenceJob.findOneAndUpdate(
          { job_id: reqId, status: 'pending' },
          { status: 'claimed', claimed_by: data.node_name || data.node_id, claimed_at: new Date() }
        );
      } catch (_) {}
    }

    if (msg.type === 'INFERENCE_RESULT' && reqId) {
      if (data.error) {
        // Only accept failure from the miner who claimed the job.
        // Other miners failing (e.g. wrong model) should not kill the job.
        const job = await InferenceJob.findOne({ job_id: reqId });
        if (!job) return;

        // If the job is claimed by a specific miner, only that miner can fail it
        if (job.claimed_by && data.node_name && job.claimed_by !== data.node_name) {
          // Different miner failed — ignore, the real miner is still working
          return;
        }

        // If job is still pending (no one claimed), revert to pending so another miner can try
        if (job.status === 'pending') {
          return; // don't fail unclaimed jobs
        }

        const failedJob = await InferenceJob.findOneAndUpdate(
          { job_id: reqId, claimed_by: data.node_name, status: { $in: ['claimed', 'processing'] } },
          { status: 'failed', result_text: data.error, completed_at: new Date() },
          { new: true }
        );
        if (!failedJob) return; // not our claim to fail

        // Refund pre-deducted cost on failure
        if (failedJob.project_id && failedJob.cost > 0) {
          const Project = require('../models/Project');
          await Project.findByIdAndUpdate(failedJob.project_id, { $inc: { balance: failedJob.cost } });
          console.log(`[BTCPC P2P Router] Refunded ${failedJob.cost} BTCPC to project`);
        }
        console.log(`[BTCPC P2P Router] Job ${reqId.slice(0, 12)} failed by ${data.node_name}: ${data.error}`);
        return;
      }

      // Update existing job OR create it (authority may not have the job locally)
      let updated = await InferenceJob.findOneAndUpdate(
        { job_id: reqId, status: { $in: ['pending', 'claimed', 'processing'] } },
        {
          status: 'completed',
          result_text: data.result_text || '',
          result_hash: data.result_hash || null,
          tokens_generated: data.tokens_generated || 0,
          elapsed_ms: data.elapsed_ms || 0,
          node_name: data.node_name || 'unknown',
          completed_at: new Date()
        },
        { new: true }
      );

      // If job doesn't exist locally (e.g. authority received result from miner),
      // create it so the settlement sweep can find it
      if (!updated) {
        const existing = await InferenceJob.findOne({ job_id: reqId });
        if (!existing) {
          updated = await InferenceJob.create({
            job_id: reqId,
            status: 'completed',
            model: data.model || 'unknown',
            messages: [],
            result_text: data.result_text || '',
            result_hash: data.result_hash || null,
            tokens_generated: data.tokens_generated || 0,
            elapsed_ms: data.elapsed_ms || 0,
            node_name: data.node_name || 'unknown',
            completed_at: new Date()
          });
          console.log(`[BTCPC P2P Router] Created job ${reqId.slice(0, 12)} from P2P result (authority sync)`);
        }
      }

      if (updated) {
        // Reconcile billing — refund difference between estimated and actual cost
        if (updated.project_id && updated.cost > 0) {
          const Project = require('../models/Project');
          const { calculateCost } = require('../services/pricing');
          const actual = await calculateCost(updated.tokens_generated, updated.model);
          const actualCost = actual.cost;
          const diff = updated.cost - actualCost;

          if (diff > 0) {
            await Project.findByIdAndUpdate(updated.project_id, { $inc: { balance: diff } });
          }
          await Project.findByIdAndUpdate(updated.project_id, { $inc: { totalSpent: actualCost } });
          await InferenceJob.findByIdAndUpdate(updated._id, { cost: actualCost });
        }
        console.log(`[BTCPC P2P Router] Job ${reqId.slice(0, 12)} completed: ${updated.tokens_generated} tokens, ${updated.elapsed_ms}ms`);
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
async function submitInference({ model, messages, maxTokens, temperature, maxFee, projectId }) {
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
    const Project = require('../models/Project');
    const { calculateCost } = require('../services/pricing');
    const escrow = require('../services/escrow');
    const estimated = await calculateCost(maxTokens || 1024, model);
    estimatedCost = estimated.cost;

    const project = await Project.findById(projectId);
    if (project) {
      // Lock funds via escrow — deducts from the project's funded account
      // project.repo is the account name that holds the BTCPC (e.g. "bullship")
      const payerAccount = project.repo || project.owner;
      try {
        await escrow.lockFunds(jobId, payerAccount, estimatedCost);
      } catch (err) {
        throw new Error(`Escrow lock failed for ${payerAccount}: ${err.message}`);
      }

      project.totalRequests += 1;
      await project.save();
    }
  }

  // Store job in DB
  const job = new InferenceJob({
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
    expires_at: new Date(Date.now() + 600000) // 10 min expiry
  });
  await job.save();

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
      temperature: temperature || 0.7
    }, p2p.NODE_ID);
    p2p.broadcast(payload);
  }, 500);

  console.log(`[BTCPC P2P Router] Submitted job ${jobId.slice(0, 12)} (model: ${model})`);

  return { job_id: jobId, status: 'pending' };
}

/**
 * Get job status and result.
 */
async function getJob(jobId) {
  return InferenceJob.findOne({ job_id: jobId }).lean();
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
