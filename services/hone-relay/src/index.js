/**
 * HONE P2P Relay — Cloudflare Durable Object
 * Shin Devlin
 *
 * WebSocket relay for HONE nodes behind NAT.
 * All nodes connect outbound to this relay. Messages are broadcast to all peers.
 * No port forwarding needed on miner machines.
 *
 * Endpoints:
 *   GET /ws         — WebSocket upgrade (nodes connect here)
 *   GET /peers      — list connected peers (JSON)
 *   GET /health     — relay status
 */

export default {
  async fetch(request, env) {
    const url = new URL(request.url);

    if (url.pathname === '/health') {
      return new Response(JSON.stringify({ status: 'ok', service: 'hone-relay' }), {
        headers: { 'Content-Type': 'application/json', 'Access-Control-Allow-Origin': '*' },
      });
    }

    // CORS preflight
    if (request.method === 'OPTIONS') {
      return new Response(null, {
        headers: {
          'Access-Control-Allow-Origin': '*',
          'Access-Control-Allow-Methods': 'GET, POST, OPTIONS',
          'Access-Control-Allow-Headers': 'Content-Type, Authorization',
        },
      });
    }

    // Route all requests to the single Durable Object
    const id = env.RELAY.idFromName('hone-main-relay');
    const relay = env.RELAY.get(id);
    return relay.fetch(request);
  },
};

export class HONERelay {
  constructor(state, env) {
    this.state = state;
    this.peers = new Map(); // ws → { nodeId, connectedAt, lastSeen, msgCount }
  }

  async fetch(request) {
    const url = new URL(request.url);
    const cors = { 'Content-Type': 'application/json', 'Access-Control-Allow-Origin': '*' };

    if (url.pathname === '/peers') {
      return this.handlePeerList();
    }

    if (url.pathname === '/ws') {
      if (request.headers.get('Upgrade') !== 'websocket') {
        return new Response('Expected WebSocket', { status: 426 });
      }
      return this.handleWebSocket();
    }

    // POST /v1/inference — OpenAI-compatible HTTP endpoint
    // Requires Bearer token authorization
    if (url.pathname === '/v1/inference' && request.method === 'POST') {
      const auth = request.headers.get('Authorization');
      if (!auth || !auth.startsWith('Bearer hone_')) {
        return new Response(JSON.stringify({ error: 'Authorization required. Use Bearer hone_<key>' }), {
          status: 401, headers: cors,
        });
      }
      return this.handleHttpInference(request);
    }

    return new Response(JSON.stringify({
      service: 'hone-relay',
      endpoints: {
        websocket: '/ws',
        inference: 'POST /v1/inference',
        peers: 'GET /peers',
        health: 'GET /health',
      }
    }), { status: 200, headers: cors });
  }

  /**
   * HTTP inference endpoint — accepts a prompt, broadcasts to miners,
   * waits for result, returns it. OpenAI-compatible format.
   */
  async handleHttpInference(request) {
    const cors = { 'Content-Type': 'application/json', 'Access-Control-Allow-Origin': '*' };

    let body;
    try {
      body = await request.json();
    } catch (_) {
      return new Response(JSON.stringify({ error: 'Invalid JSON' }), { status: 400, headers: cors });
    }

    const prompt = body.prompt || (body.messages && body.messages.map(m => m.content).join('\n'));
    const model = body.model || 'qwen3.5:27b';
    if (!prompt) {
      return new Response(JSON.stringify({ error: 'prompt or messages required' }), { status: 400, headers: cors });
    }

    const requestId = crypto.randomUUID();

    // Broadcast INFERENCE_REQUEST to all connected miners
    const reqMsg = JSON.stringify({
      id: requestId + '-req',
      type: 'INFERENCE_REQUEST',
      data: { request_id: requestId, model, max_fee: 10, prompt_hash: await sha256(prompt) },
      timestamp: Date.now(),
      nodeId: 'relay',
    });

    // Broadcast INFERENCE_PAYLOAD immediately (skip claim/assign for v1 — any miner can pick it up)
    const payloadMsg = JSON.stringify({
      id: requestId + '-payload',
      type: 'INFERENCE_PAYLOAD',
      data: { request_id: requestId, prompt, model, target_node: null },
      timestamp: Date.now(),
      nodeId: 'relay',
    });

    for (const [ws] of this.peers) {
      try { ws.send(reqMsg); } catch (_) {}
    }
    // Small delay then send payload
    for (const [ws] of this.peers) {
      try { ws.send(payloadMsg); } catch (_) {}
    }

    // Wait for INFERENCE_RESULT with matching request_id (max 120s)
    const result = await this.waitForResult(requestId, 120000);

    if (!result) {
      return new Response(JSON.stringify({
        error: 'No miners responded. Network may be busy or empty.',
        peers: this.peers.size,
      }), { status: 504, headers: cors });
    }

    // OpenAI-compatible response format
    return new Response(JSON.stringify({
      id: requestId,
      object: 'chat.completion',
      model: result.model || model,
      choices: [{
        index: 0,
        message: { role: 'assistant', content: result.result_text || '' },
        finish_reason: result.error ? 'error' : 'stop',
      }],
      usage: {
        completion_tokens: result.tokens_generated || 0,
      },
      hone: {
        node: result.node_name,
        elapsed_ms: result.elapsed_ms,
        error: result.error || null,
      },
    }), { status: result.error ? 500 : 200, headers: cors });
  }

  /**
   * Wait for an INFERENCE_RESULT message matching the request ID.
   */
  waitForResult(requestId, timeoutMs) {
    return new Promise((resolve) => {
      const timer = setTimeout(() => {
        this.resultWaiters.delete(requestId);
        resolve(null);
      }, timeoutMs);

      this.resultWaiters = this.resultWaiters || new Map();
      this.resultWaiters.set(requestId, (data) => {
        clearTimeout(timer);
        this.resultWaiters.delete(requestId);
        resolve(data);
      });
    });
  }

  handleWebSocket() {
    const pair = new WebSocketPair();
    const [client, server] = Object.values(pair);

    server.accept();

    const peerInfo = {
      nodeId: null,
      connectedAt: Date.now(),
      lastSeen: Date.now(),
      msgCount: 0,
    };

    this.peers.set(server, peerInfo);

    server.addEventListener('message', (event) => {
      peerInfo.lastSeen = Date.now();
      peerInfo.msgCount++;

      let msg;
      try {
        msg = JSON.parse(event.data);
      } catch (_) {
        return; // ignore non-JSON
      }

      // Extract node ID from handshake
      if (msg.type === 'HANDSHAKE' && msg.nodeId) {
        peerInfo.nodeId = msg.nodeId;
      }

      // Check if this is an inference result someone is waiting for
      if (msg.type === 'INFERENCE_RESULT' && msg.data?.request_id) {
        const waiter = this.resultWaiters?.get(msg.data.request_id);
        if (waiter) waiter(msg.data);
      }

      // Broadcast to all OTHER connected peers
      for (const [ws, info] of this.peers) {
        if (ws === server) continue;
        try {
          ws.send(event.data);
        } catch (_) {
          // Dead connection, clean up
          this.peers.delete(ws);
        }
      }
    });

    server.addEventListener('close', () => {
      this.peers.delete(server);
    });

    server.addEventListener('error', () => {
      this.peers.delete(server);
    });

    return new Response(null, { status: 101, webSocket: client });
  }

  handlePeerList() {
    const peers = [];
    for (const [ws, info] of this.peers) {
      try {
        // Check if still alive
        if (ws.readyState === WebSocket.READY_STATE_OPEN || ws.readyState === 1) {
          peers.push({
            nodeId: info.nodeId,
            connectedAt: info.connectedAt,
            lastSeen: info.lastSeen,
            msgCount: info.msgCount,
          });
        } else {
          this.peers.delete(ws);
        }
      } catch (_) {
        this.peers.delete(ws);
      }
    }

    return new Response(JSON.stringify({ peers, count: peers.length }), {
      headers: { 'Content-Type': 'application/json', 'Access-Control-Allow-Origin': '*' },
    });
  }
}

async function sha256(text) {
  const data = new TextEncoder().encode(text);
  const hash = await crypto.subtle.digest('SHA-256', data);
  return Array.from(new Uint8Array(hash)).map(b => b.toString(16).padStart(2, '0')).join('');
}
