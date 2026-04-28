"use strict";

/**
 * BTCPC Multi-Provider Inference
 * Shin Devlin
 *
 * Each provider exposes: name, baseUrl, formatRequest(), parseResponse()
 * API keys stay local -- never touch the chain.
 */

// Prefer the largest coding model available on the network.
// When beastly (or any miner with a 27b+ model) is connected, P2P routing
// will deliver the job there automatically. Falls back through smaller models.
const CODING_MODEL_PREFERENCE = [
  "gemma3:27b",
  "gemma3:12b",
  "qwen2.5-coder:14b",
  "qwen2.5-coder:7b",
  "qwen3.5:27b",
  "gemma3:4b",
  "gemma3:1b",
];
const DEFAULT_MODEL = CODING_MODEL_PREFERENCE[0]; // gemma3:27b

// ---------------------------------------------------------------------------
// Provider definitions
// ---------------------------------------------------------------------------

const providers = {

  // ---- BTCPC native (local node running OpenAI-compatible API) ------------
  // Uses the relay key so local tools can submit without a user JWT.
  // Port is read from env at startup — matches BTCPC_API_PORT in .env.
  btcpc: {
    name: "btcpc",
    baseUrl: `http://localhost:${process.env.BTCPC_API_PORT || 4242}/v1/chat/completions`,
    getHeaders() {
      return {
        "Content-Type": "application/json",
        Authorization: `Bearer ${process.env.BTCPC_RELAY_API_KEY || "btcpc-local"}`
      };
    },
    formatRequest(prompt, model, options) {
      return {
        model: model || DEFAULT_MODEL,
        messages: [{ role: "user", content: prompt }],
        temperature: options.temperature,
        max_tokens: options.maxTokens,
        stream: false
      };
    },
    parseResponse(data) {
      const choice = data.choices && data.choices[0];
      return {
        content: choice ? choice.message.content : "",
        model: data.model || DEFAULT_MODEL,
        request_id: data.id || null,
        tokens_used: data.usage
          ? data.usage.prompt_tokens + data.usage.completion_tokens
          : 0,
        prompt_tokens: data.usage ? data.usage.prompt_tokens : 0,
        completion_tokens: data.usage ? data.usage.completion_tokens : 0
      };
    }
  },

  // ---- OpenAI -------------------------------------------------------------
  openai: {
    name: "openai",
    baseUrl: "https://api.openai.com/v1/chat/completions",
    getHeaders() {
      const key = process.env.OPENAI_API_KEY;
      if (!key) throw new Error("OPENAI_API_KEY not set in environment.");
      return {
        "Content-Type": "application/json",
        Authorization: "Bearer " + key
      };
    },
    formatRequest(prompt, model, options) {
      const body = {
        model: model || "gpt-4o",
        messages: [{ role: "user", content: prompt }],
        stream: false
      };
      if (options.temperature != null) body.temperature = options.temperature;
      if (options.maxTokens != null) body.max_tokens = options.maxTokens;
      return body;
    },
    parseResponse(data) {
      const choice = data.choices && data.choices[0];
      return {
        content: choice ? choice.message.content : "",
        model: data.model || "gpt-4o",
        request_id: data.id || null,
        tokens_used: data.usage ? data.usage.total_tokens : 0,
        prompt_tokens: data.usage ? data.usage.prompt_tokens : 0,
        completion_tokens: data.usage ? data.usage.completion_tokens : 0
      };
    }
  },

  // ---- Anthropic ----------------------------------------------------------
  anthropic: {
    name: "anthropic",
    baseUrl: "https://api.anthropic.com/v1/messages",
    getHeaders() {
      const key = process.env.ANTHROPIC_API_KEY;
      if (!key) throw new Error("ANTHROPIC_API_KEY not set in environment.");
      return {
        "Content-Type": "application/json",
        "x-api-key": key,
        "anthropic-version": "2023-06-01"
      };
    },
    formatRequest(prompt, model, options) {
      const body = {
        model: model || "claude-sonnet-4-6-20250514",
        messages: [{ role: "user", content: prompt }],
        max_tokens: options.maxTokens || 1024
      };
      if (options.temperature != null) body.temperature = options.temperature;
      return body;
    },
    parseResponse(data) {
      // Anthropic returns content as an array of blocks
      let text = "";
      if (data.content && Array.isArray(data.content)) {
        text = data.content
          .filter(function (b) { return b.type === "text"; })
          .map(function (b) { return b.text; })
          .join("");
      }
      const inputTokens = data.usage ? data.usage.input_tokens : 0;
      const outputTokens = data.usage ? data.usage.output_tokens : 0;
      return {
        content: text,
        model: data.model || "claude-sonnet-4-6-20250514",
        request_id: data.id || null,
        tokens_used: inputTokens + outputTokens,
        prompt_tokens: inputTokens,
        completion_tokens: outputTokens
      };
    }
  },

  // ---- xAI / Grok (OpenAI-compatible) ------------------------------------
  grok: {
    name: "grok",
    baseUrl: "https://api.x.ai/v1/chat/completions",
    getHeaders() {
      const key = process.env.XAI_API_KEY;
      if (!key) throw new Error("XAI_API_KEY not set in environment.");
      return {
        "Content-Type": "application/json",
        Authorization: "Bearer " + key
      };
    },
    formatRequest(prompt, model, options) {
      const body = {
        model: model || "grok-2",
        messages: [{ role: "user", content: prompt }],
        stream: false
      };
      if (options.temperature != null) body.temperature = options.temperature;
      if (options.maxTokens != null) body.max_tokens = options.maxTokens;
      return body;
    },
    parseResponse(data) {
      const choice = data.choices && data.choices[0];
      return {
        content: choice ? choice.message.content : "",
        model: data.model || "grok-2",
        request_id: data.id || null,
        tokens_used: data.usage ? data.usage.total_tokens : 0,
        prompt_tokens: data.usage ? data.usage.prompt_tokens : 0,
        completion_tokens: data.usage ? data.usage.completion_tokens : 0
      };
    }
  },

  // ---- Ollama (local) -----------------------------------------------------
  ollama: {
    name: "ollama",
    baseUrl: (process.env.OLLAMA_URL || "http://localhost:11434") + "/api/chat",
    getHeaders() {
      return { "Content-Type": "application/json" };
    },
    formatRequest(prompt, model, options) {
      const body = {
        model: model || "llama3:8b",
        messages: [{ role: "user", content: prompt }],
        stream: false
      };
      if (options.temperature != null || options.maxTokens != null) {
        body.options = {};
        if (options.temperature != null) body.options.temperature = options.temperature;
        if (options.maxTokens != null) body.options.num_predict = options.maxTokens;
      }
      return body;
    },
    parseResponse(data) {
      const content = (data.message && data.message.content) || "";
      const evalCount = data.eval_count || Math.max(1, Math.ceil(content.length / 4));
      const promptEval = data.prompt_eval_count || 0;
      return {
        content: content,
        model: data.model || "llama3:8b",
        request_id: null,
        tokens_used: promptEval + evalCount,
        prompt_tokens: promptEval,
        completion_tokens: evalCount
      };
    }
  }
};

// ---------------------------------------------------------------------------
// Public API
// ---------------------------------------------------------------------------

function getProvider(name) {
  const p = providers[name];
  if (!p) {
    const known = Object.keys(providers).join(", ");
    throw new Error("Unknown provider '" + name + "'. Supported: " + known);
  }
  return p;
}

function listProviders() {
  return Object.keys(providers);
}

module.exports = {
  getProvider: getProvider,
  listProviders: listProviders,
  DEFAULT_MODEL: DEFAULT_MODEL,
  CODING_MODEL_PREFERENCE: CODING_MODEL_PREFERENCE,
  providers: providers
};
