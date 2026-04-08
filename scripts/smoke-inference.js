#!/usr/bin/env node
"use strict";

const API_BASE_URL = process.env.API_BASE_URL || "http://localhost:3000";
const API_KEY = process.env.API_KEY || process.env.BTCPC_RELAY_API_KEY || "";
const MODEL = process.env.MODEL || "qwen3:4b";
const RUNS = Math.max(1, parseInt(process.env.RUNS || "5", 10));
const TIMEOUT_MS = Math.max(1000, parseInt(process.env.TIMEOUT_MS || "60000", 10));

if (!API_KEY) {
  console.error("ERROR: API_KEY or BTCPC_RELAY_API_KEY is required.");
  process.exit(1);
}

const prompts = [
  {
    name: "literal",
    prompt: "Reply with exactly: BTCPC inference ok",
    check: (text) => text.trim().length > 0,
    strict: (text) => text.trim() === "BTCPC inference ok"
  },
  {
    name: "math",
    prompt: "What is 2 + 2? Reply in one short sentence.",
    check: (text) => /\b4\b/.test(text),
    strict: null
  },
  {
    name: "json",
    prompt: "Return valid JSON with a single key named status and value ok.",
    check: (text) => {
      try {
        const parsed = JSON.parse(text);
        return parsed && parsed.status === "ok";
      } catch (_) {
        return false;
      }
    },
    strict: null
  },
  {
    name: "paragraph",
    prompt: "Explain in two short sentences what BTCPC does.",
    check: (text) => text.trim().length > 20,
    strict: null
  }
];

function percentile(values, p) {
  if (!values.length) return 0;
  const sorted = [...values].sort((a, b) => a - b);
  const index = Math.min(sorted.length - 1, Math.ceil((p / 100) * sorted.length) - 1);
  return sorted[index];
}

async function requestCompletion(prompt) {
  const controller = new AbortController();
  const timer = setTimeout(() => controller.abort(), TIMEOUT_MS);
  const started = Date.now();

  try {
    const res = await fetch(`${API_BASE_URL}/v1/chat/completions`, {
      method: "POST",
      headers: {
        "Authorization": `Bearer ${API_KEY}`,
        "Content-Type": "application/json"
      },
      body: JSON.stringify({
        model: MODEL,
        messages: [{ role: "user", content: prompt }],
        max_tokens: 64,
        temperature: 0
      }),
      signal: controller.signal
    });

    const text = await res.text();
    let json = null;
    try {
      json = JSON.parse(text);
    } catch (_) {}

    return {
      status: res.status,
      elapsedMs: Date.now() - started,
      raw: text,
      json
    };
  } finally {
    clearTimeout(timer);
  }
}

async function main() {
  const results = [];

  for (let run = 0; run < RUNS; run++) {
    for (const promptDef of prompts) {
      const response = await requestCompletion(promptDef.prompt);
      const content = response.json?.choices?.[0]?.message?.content || "";
      const btcpc = response.json?.btcpc || null;
      const success = response.status === 200 && promptDef.check(content);
      const strictSuccess = promptDef.strict ? promptDef.strict(content) : null;

      results.push({
        run: run + 1,
        name: promptDef.name,
        status: response.status,
        elapsedMs: response.elapsedMs,
        success,
        strictSuccess,
        content,
        cost: btcpc?.cost ?? null,
        proofHash: btcpc?.proof_hash ?? null,
        verified: btcpc?.verified ?? null,
        raw: response.raw
      });
    }
  }

  const latencies = results.map(r => r.elapsedMs);
  const failures = results.filter(r => !r.success);
  const strictFailures = results.filter(r => r.strictSuccess === false);
  const emptyResponses = results.filter(r => !r.content.trim());

  const summary = {
    apiBaseUrl: API_BASE_URL,
    model: MODEL,
    runs: RUNS,
    promptsPerRun: prompts.length,
    totalRequests: results.length,
    passed: results.length - failures.length,
    failed: failures.length,
    strictLiteralFailures: strictFailures.length,
    emptyResponses: emptyResponses.length,
    avgLatencyMs: Math.round(latencies.reduce((sum, n) => sum + n, 0) / latencies.length),
    p95LatencyMs: percentile(latencies, 95),
    failures: failures.map(f => ({
      run: f.run,
      name: f.name,
      status: f.status,
      elapsedMs: f.elapsedMs,
      content: f.content,
      raw: f.raw.slice(0, 300)
    })),
    strictFailures: strictFailures.map(f => ({
      run: f.run,
      name: f.name,
      content: f.content
    })),
    sample: results.slice(0, 4).map(r => ({
      run: r.run,
      name: r.name,
      elapsedMs: r.elapsedMs,
      success: r.success,
      strictSuccess: r.strictSuccess,
      cost: r.cost,
      verified: r.verified,
      proofHash: r.proofHash,
      content: r.content
    }))
  };

  console.log(JSON.stringify(summary, null, 2));

  if (failures.length > 0 || emptyResponses.length > 0) {
    process.exit(1);
  }
}

main().catch((err) => {
  console.error(JSON.stringify({
    error: err.message,
    apiBaseUrl: API_BASE_URL,
    model: MODEL
  }, null, 2));
  process.exit(1);
});
