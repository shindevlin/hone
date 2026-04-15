#!/usr/bin/env node
/**
 * BTCPC inference cron — submits periodic jobs to keep the network active.
 * Runs under natoshisakamoto's project key.
 *
 * Usage: node scripts/inference-cron.js
 * Env: BTCPC_API_URL, BTCPC_PROJECT_KEY, CRON_INTERVAL_MS, CRON_MODEL
 */

require('dotenv').config();
const axios = require('axios');

const API_URL = process.env.BTCPC_API_URL || 'http://localhost:3000';
const PROJECT_KEY = process.env.BTCPC_PROJECT_KEY;
const INTERVAL = parseInt(process.env.CRON_INTERVAL_MS) || 120000; // 2 min default
const MODEL = process.env.CRON_MODEL || 'qwen3:4b';

if (!PROJECT_KEY) {
  console.error('BTCPC_PROJECT_KEY required');
  process.exit(1);
}

const prompts = [
  "Explain how proof of useful work differs from proof of work in three sentences.",
  "Write a haiku about decentralized computing.",
  "What are the economic implications of tying token emission to real compute?",
  "Compare Bitcoin's energy usage to a proof-of-compute chain that does AI inference.",
  "Describe how cross-chain wallet derivation from a single mnemonic works.",
  "What is retrieval-augmented generation and why does it matter for blockchain inference?",
  "Explain the Model Context Protocol in simple terms.",
  "Write a short explanation of why parameter count matters for inference reward splitting.",
  "How does BIP-39 mnemonic derivation ensure deterministic wallet generation?",
  "What are the tradeoffs between privacy and performance in multi-party computation?",
  "Describe the commit-reveal verification scheme for distributed compute.",
  "Why does a sovereign chain make sense for AI inference vs using an L2?",
  "Explain how dynamic pricing based on network load and model size works.",
  "What is a genesis dream NFT and why is it soulbound?",
  "How can a blockchain project use MCP servers to enhance inference quality?",
];

let jobCount = 0;

async function submitJob() {
  const prompt = prompts[Math.floor(Math.random() * prompts.length)];
  const ts = new Date().toISOString();

  try {
    const { data } = await axios.post(`${API_URL}/v1/inference/submit`, {
      model: MODEL,
      messages: [{ role: 'user', content: prompt }],
      max_tokens: 256
    }, {
      timeout: 10000,
      headers: { 'Authorization': `Bearer ${PROJECT_KEY}` }
    });

    jobCount++;
    console.log(`[${ts}] Job #${jobCount} submitted: ${data.job_id} (${MODEL})`);

    // Poll for completion
    const startTime = Date.now();
    while (Date.now() - startTime < 300000) {
      await new Promise(r => setTimeout(r, 5000));
      const { data: job } = await axios.get(`${API_URL}/v1/inference/${data.job_id}`, {
        headers: { 'Authorization': `Bearer ${PROJECT_KEY}` }
      });
      if (job.status === 'completed') {
        const tokens = job.result?.tokens_generated || job.tokens_generated || '?';
        console.log(`[${new Date().toISOString()}] Job #${jobCount} complete: ${tokens} tokens`);
        return;
      }
      if (job.status === 'failed') {
        console.log(`[${new Date().toISOString()}] Job #${jobCount} failed`);
        return;
      }
    }
    console.log(`[${new Date().toISOString()}] Job #${jobCount} timed out`);
  } catch (err) {
    console.error(`[${ts}] Job failed: ${err.response?.data?.error?.message || err.message}`);
  }
}

console.log(`[inference-cron] Starting — model: ${MODEL}, interval: ${INTERVAL / 1000}s`);
console.log(`[inference-cron] API: ${API_URL}`);

// Run immediately, then on interval
submitJob();
setInterval(submitJob, INTERVAL);
