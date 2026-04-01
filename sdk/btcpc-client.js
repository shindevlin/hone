'use strict';

/**
 * Shared BTCPC inference client for all projects.
 *
 * Copy this file into your project or require it from the btcpc repo.
 * Uses async submit/poll — no timeouts, no long HTTP connections.
 *
 * Usage:
 *   const btcpc = require('./btcpc-client');
 *   const text = await btcpc.ask('What is 2+2?');
 *   const result = await btcpc.generate('Write a story', { system: '...', maxTokens: 2048 });
 */

const axios = require('axios');

const BTCPC_URL = process.env.BTCPC_URL || 'http://localhost:3100';
const BTCPC_API_KEY = process.env.BTCPC_API_KEY || '';
const BTCPC_MODEL = process.env.BTCPC_MODEL || 'qwen3.5:27b';

const headers = () => ({
  'Authorization': `Bearer ${BTCPC_API_KEY}`,
  'Content-Type': 'application/json'
});

/**
 * Submit a job and poll until complete.
 */
async function generate(prompt, opts = {}) {
  const model = opts.model || BTCPC_MODEL;
  const messages = [];
  if (opts.system) messages.push({ role: 'system', content: opts.system });
  messages.push({ role: 'user', content: prompt });

  // Submit
  const { data: sub } = await axios.post(`${BTCPC_URL}/v1/inference/submit`, {
    model,
    messages,
    max_tokens: opts.maxTokens || 1024,
    temperature: opts.temperature || 0.7
  }, { headers: headers(), timeout: 10000 });

  const jobId = sub.job_id;
  console.log(`[btcpc] ${jobId.slice(0, 16)} submitted (${model})`);

  // Poll
  const maxWait = opts.timeout || 600000;
  const start = Date.now();

  while (Date.now() - start < maxWait) {
    await new Promise(r => setTimeout(r, 3000));
    const { data: job } = await axios.get(`${BTCPC_URL}/v1/inference/${jobId}`, {
      headers: headers(), timeout: 5000
    });

    if (job.status === 'completed') {
      console.log(`[btcpc] ${jobId.slice(0, 16)} done (${job.result.tokens} tokens, ${job.result.elapsed_ms}ms)`);
      return { text: job.result.content, tokens: job.result.tokens, elapsed: job.result.elapsed_ms, raw: job };
    }
    if (job.status === 'failed') {
      throw new Error(`Inference failed: ${job.error}`);
    }
  }
  throw new Error(`Job ${jobId} did not complete within ${maxWait / 1000}s`);
}

async function ask(prompt, opts = {}) {
  const result = await generate(prompt, opts);
  return result.text;
}

async function checkBalance() {
  const { data } = await axios.get(`${BTCPC_URL}/api/projects/me`, { headers: headers(), timeout: 5000 });
  return data;
}

module.exports = { generate, ask, checkBalance, BTCPC_URL, BTCPC_MODEL };
