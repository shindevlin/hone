"use strict";

const https = require('https');
const http = require('http');

const DEFAULT_BASE_URL = 'https://api.btcpc.network';

class BTCPC {
  /**
   * @param {Object} options
   * @param {string} options.apiKey - Your btcpc_ API key
   * @param {string} [options.baseUrl] - API base URL (default: https://api.btcpc.network)
   */
  constructor({ apiKey, baseUrl } = {}) {
    if (!apiKey) throw new Error('apiKey is required. Get one at /api/projects/register');
    this.apiKey = apiKey;
    this.baseUrl = (baseUrl || DEFAULT_BASE_URL).replace(/\/+$/, '');
  }

  /**
   * Send a chat completion request (OpenAI-compatible).
   * @param {Object} options
   * @param {string} [options.model] - Model name (default: qwen3.5:27b)
   * @param {Array} [options.messages] - OpenAI-format messages array
   * @param {string} [options.prompt] - Simple string prompt (shorthand for single user message)
   * @param {number} [options.maxTokens] - Max tokens to generate
   * @param {number} [options.temperature] - Sampling temperature
   * @returns {Promise<Object>} OpenAI-compatible response with btcpc billing info
   */
  async chat({ model, messages, prompt, maxTokens, temperature } = {}) {
    if (prompt && !messages) {
      messages = [{ role: 'user', content: prompt }];
    }
    if (!messages || messages.length === 0) {
      throw new Error('Either messages or prompt is required');
    }

    const body = { messages, stream: false };
    if (model) body.model = model;
    if (typeof maxTokens === 'number') body.max_tokens = maxTokens;
    if (typeof temperature === 'number') body.temperature = temperature;

    return this._request('POST', '/v1/chat/completions', body);
  }

  /**
   * List available models.
   * @returns {Promise<Object>} OpenAI-compatible model list
   */
  async models() {
    return this._request('GET', '/v1/models');
  }

  /**
   * Get project info (balance, usage, verification status).
   * @returns {Promise<Object>}
   */
  async project() {
    return this._request('GET', '/api/projects/me');
  }

  /**
   * Convenience: extract just the text from a chat response.
   * @param {Object} options - Same as chat()
   * @returns {Promise<string>} The assistant's response text
   */
  async ask(options) {
    const res = await this.chat(options);
    return res.choices?.[0]?.message?.content || '';
  }

  /** @private */
  _request(method, path, body) {
    return new Promise((resolve, reject) => {
      const url = new URL(path, this.baseUrl);
      const isHttps = url.protocol === 'https:';
      const transport = isHttps ? https : http;

      const payload = body ? JSON.stringify(body) : null;

      const req = transport.request({
        hostname: url.hostname,
        port: url.port || (isHttps ? 443 : 80),
        path: url.pathname,
        method,
        headers: {
          'Authorization': `Bearer ${this.apiKey}`,
          'Content-Type': 'application/json',
          'User-Agent': `@btcpc/sdk/0.1.0`,
          ...(payload ? { 'Content-Length': Buffer.byteLength(payload) } : {})
        },
        timeout: 180000
      }, (res) => {
        let data = '';
        res.on('data', chunk => data += chunk);
        res.on('end', () => {
          try {
            const parsed = JSON.parse(data);
            if (res.statusCode >= 400) {
              const err = new Error(parsed.error?.message || `HTTP ${res.statusCode}`);
              err.status = res.statusCode;
              err.code = parsed.error?.code;
              err.response = parsed;
              return reject(err);
            }
            resolve(parsed);
          } catch (e) {
            reject(new Error(`Invalid JSON response: ${data.slice(0, 200)}`));
          }
        });
      });

      req.on('error', reject);
      req.on('timeout', () => {
        req.destroy();
        reject(new Error('Request timed out (180s)'));
      });

      if (payload) req.write(payload);
      req.end();
    });
  }
}

module.exports = BTCPC;
module.exports.BTCPC = BTCPC;
module.exports.default = BTCPC;
