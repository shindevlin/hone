"use strict";

/**
 * Go-Live Smoke Test Suite
 *
 * Covers every item in docs/GO_LIVE_CHECKLIST.md.
 *
 * Environment:
 *   HONE_SMOKE_URL   — base URL to target (default: http://localhost:4242)
 *   HONE_SMOKE_SKIP  — tests are skipped by default (opt-in). Set to "0" to
 *                       actually run them against a live node at HONE_SMOKE_URL.
 *   HONE_SMOKE_API_KEY — API key for authenticated inference test (optional;
 *                         the authenticated POST test is skipped when absent)
 *
 * Each test retries 3× with 1 s backoff before failing.
 * Every test has a 10 s timeout.
 */

const axios = require("axios");

const BASE = (process.env.HONE_SMOKE_URL || "http://localhost:4242").replace(
  /\/$/,
  ""
);
// Smoke tests hit a live node at HONE_SMOKE_URL. They are opt-IN: skipped by
// default (no node in unit-test/CI runs) and only run when HONE_SMOKE_SKIP is
// explicitly set to "0". Previously this skipped only when ==="1", so the suite
// ran by default and the workers crashed trying to reach a node that isn't up.
const SKIP = process.env.HONE_SMOKE_SKIP !== "0";
const API_KEY = process.env.HONE_SMOKE_API_KEY || "";

const RETRY_COUNT = 3;
const RETRY_DELAY_MS = 1000;
// Axios timeout per attempt — chosen so 3 attempts + 2 s backoff fits within 10 s Jest timeout
const REQUEST_TIMEOUT_MS = 2500;

/** Retry helper: runs fn up to RETRY_COUNT times, delaying RETRY_DELAY_MS between attempts. */
async function withRetry(fn) {
  let lastErr;
  for (let attempt = 1; attempt <= RETRY_COUNT; attempt++) {
    try {
      return await fn();
    } catch (err) {
      lastErr = err;
      if (attempt < RETRY_COUNT) {
        await new Promise((r) => setTimeout(r, RETRY_DELAY_MS));
      }
    }
  }
  throw lastErr;
}

function client(extraHeaders = {}) {
  return axios.create({
    baseURL: BASE,
    timeout: REQUEST_TIMEOUT_MS,
    validateStatus: null, // don't throw on non-2xx — we assert status ourselves
    headers: extraHeaders,
  });
}

// ---------------------------------------------------------------------------
// Conditional describe: all tests are skipped in CI unless HONE_SMOKE_SKIP=0
// ---------------------------------------------------------------------------
const describe_ = SKIP ? describe.skip : describe;

describe_("Go-Live Checklist smoke tests", () => {
  // -------------------------------------------------------------------------
  // Public Surface checks
  // -------------------------------------------------------------------------

  test(
    "GET /health returns 200 OK",
    async () => {
      await withRetry(async () => {
        const res = await client().get("/health");
        expect(res.status).toBe(200);
        const body = res.data;
        const text = typeof body === "string" ? body.toLowerCase() : JSON.stringify(body).toLowerCase();
        expect(text).toContain("ok");
      });
    },
    10000
  );

  test(
    "GET /public/machine-status returns JSON with truth-bearing status",
    async () => {
      await withRetry(async () => {
        const res = await client().get("/public/machine-status");
        expect(res.status).toBe(200);
        const body = res.data;
        expect(typeof body).toBe("object");
        expect(body).not.toBeNull();
        // Must carry a recognisable status field — exact key varies by version
        const hasStatus =
          "status" in body ||
          "machine_status" in body ||
          "health" in body ||
          "ok" in body ||
          "truth_bearing" in body ||
          "peer_count" in body;
        expect(hasStatus).toBe(true);
      });
    },
    10000
  );

  test(
    "GET /public/network returns connected peers",
    async () => {
      await withRetry(async () => {
        const res = await client().get("/public/network");
        expect(res.status).toBe(200);
        const body = res.data;
        expect(typeof body).toBe("object");
        expect(body).not.toBeNull();
        // Must carry a peers / nodes field with at least some content
        const hasPeers =
          "peers" in body ||
          "nodes" in body ||
          "connected" in body ||
          "network" in body ||
          "peer_count" in body ||
          "registered_clocks" in body;
        expect(hasPeers).toBe(true);
      });
    },
    10000
  );

  test(
    "GET /api/node/list returns node identities with P2P addresses",
    async () => {
      await withRetry(async () => {
        const res = await client().get("/api/node/list");
        expect(res.status).toBe(200);
        const body = res.data;
        // Accept both array payload and {nodes: [...]} wrapper
        const nodes = Array.isArray(body)
          ? body
          : body.nodes || body.data || [];
        expect(Array.isArray(nodes)).toBe(true);
        // At least one node must be present on the live testnet
        expect(nodes.length).toBeGreaterThan(0);
        // Each node must carry an identity field
        const first = nodes[0];
        const hasIdentity =
          "node_id" in first ||
          "id" in first ||
          "username" in first ||
          "name" in first ||
          "account" in first;
        expect(hasIdentity).toBe(true);
      });
    },
    10000
  );

  test(
    "GET /v1/models returns model list (retries on transient failure)",
    async () => {
      await withRetry(async () => {
        const res = await client().get("/v1/models");
        expect(res.status).toBe(200);
        const body = res.data;
        expect(typeof body).toBe("object");
        expect(body).not.toBeNull();
        // OpenAI-compatible shape: {object: "list", data: [...]}
        const models = body.data || body.models || body;
        expect(Array.isArray(models)).toBe(true);
        expect(models.length).toBeGreaterThan(0);
      });
    },
    10000
  );

  test(
    "GET /v1/pricing returns pricing info (retries on transient failure)",
    async () => {
      await withRetry(async () => {
        const res = await client().get("/v1/pricing");
        expect(res.status).toBe(200);
        const body = res.data;
        expect(typeof body).toBe("object");
        expect(body).not.toBeNull();
        // Must carry at least one recognisable pricing field
        const hasPricing =
          "tokensPerHone" in body ||
          "tokens_per_hone" in body ||
          "pricing" in body ||
          "price" in body ||
          "cost" in body;
        expect(hasPricing).toBe(true);
      });
    },
    10000
  );

  // POST /v1/chat/completions — unauthenticated path (no API key)
  test(
    "POST /v1/chat/completions returns non-empty content (unauthenticated)",
    async () => {
      await withRetry(async () => {
        const res = await client().post("/v1/chat/completions", {
          model: "default",
          messages: [{ role: "user", content: "ping" }],
          max_tokens: 16,
        });
        // 200 or 201 accepted; 401/403 means auth required — we do a soft skip below
        if (res.status === 401 || res.status === 403) {
          // Unauthenticated inference is gated — this path requires a key.
          // Log a notice but don't fail so runs without HONE_SMOKE_API_KEY still pass
          // on the public health pages.
          console.warn(
            "[smoke] /v1/chat/completions requires authentication — skipping content assertion"
          );
          return;
        }
        expect(res.status).toBeGreaterThanOrEqual(200);
        expect(res.status).toBeLessThan(300);
        const body = res.data;
        expect(typeof body).toBe("object");
        expect(body).not.toBeNull();
        // OpenAI-compatible shape
        const content =
          body.choices?.[0]?.message?.content ||
          body.choices?.[0]?.text ||
          body.content ||
          body.response ||
          body.result;
        expect(content).toBeTruthy();
        expect(content.length).toBeGreaterThan(0);
      });
    },
    10000
  );

  // POST /v1/chat/completions — authenticated path (requires API key)
  const describeAuth = API_KEY ? test : test.skip;
  describeAuth(
    "POST /v1/chat/completions with API key returns non-empty content (authenticated)",
    async () => {
      await withRetry(async () => {
        const res = await client({
          Authorization: `Bearer ${API_KEY}`,
        }).post("/v1/chat/completions", {
          model: "default",
          messages: [{ role: "user", content: "ping" }],
          max_tokens: 16,
        });
        expect(res.status).toBeGreaterThanOrEqual(200);
        expect(res.status).toBeLessThan(300);
        const body = res.data;
        expect(typeof body).toBe("object");
        expect(body).not.toBeNull();
        const content =
          body.choices?.[0]?.message?.content ||
          body.choices?.[0]?.text ||
          body.content ||
          body.response ||
          body.result;
        expect(content).toBeTruthy();
        expect(content.length).toBeGreaterThan(0);
      });
    },
    10000
  );

  // -------------------------------------------------------------------------
  // Operational Gates
  // -------------------------------------------------------------------------

  test(
    "At least two real nodes are connected (operational gate)",
    async () => {
      await withRetry(async () => {
        const res = await client().get("/api/node/list");
        expect(res.status).toBe(200);
        const body = res.data;
        const nodes = Array.isArray(body)
          ? body
          : body.nodes || body.data || [];
        expect(nodes.length).toBeGreaterThanOrEqual(2);
      });
    },
    10000
  );

  test(
    "Node announcements carry a real advertised P2P address (no localhost)",
    async () => {
      await withRetry(async () => {
        // Bootstrap peer list is the canonical source of announced multiaddrs
        const res = await client().get("/api/peers/bootstrap?chain_id=hone-1");
        expect(res.status).toBe(200);
        const peers = res.data.peers || [];
        // At least one peer must have a non-localhost multiaddr
        const publicPeer = peers.find((addr) => {
          return (
            typeof addr === "string" &&
            addr.length > 0 &&
            !addr.includes("localhost") &&
            !addr.includes("127.0.0.1") &&
            !addr.includes("0.0.0.0")
          );
        });
        expect(publicPeer).toBeDefined();
      });
    },
    10000
  );

  test(
    "/health quorum: 3 of 3 repeated reads return 200 OK",
    async () => {
      let successCount = 0;
      for (let i = 0; i < 3; i++) {
        try {
          const res = await client().get("/health");
          if (res.status === 200) {
            // Handle both plain-text "OK" and JSON {"status":"ok"} responses
            const body = res.data;
            const text = typeof body === "string"
              ? body.trim().toLowerCase()
              : JSON.stringify(body).toLowerCase();
            if (text.includes("ok")) successCount++;
          }
        } catch (_) {
          // count stays where it is
        }
        if (i < 2) await new Promise((r) => setTimeout(r, 500));
      }
      expect(successCount).toBe(3);
    },
    10000
  );

  test(
    "Storage host summaries remain redacted in /public/machine-status",
    async () => {
      await withRetry(async () => {
        const res = await client().get("/public/machine-status");
        expect(res.status).toBe(200);
        const raw = JSON.stringify(res.data);
        // Sensitive fields must not appear in the response
        expect(raw).not.toMatch(/private_key/i);
        expect(raw).not.toMatch(/secret/i);
        expect(raw).not.toMatch(/password/i);
        expect(raw).not.toMatch(/mnemonic/i);
        expect(raw).not.toMatch(/seed_phrase/i);
      });
    },
    10000
  );
});
