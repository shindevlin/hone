"use strict";

/**
 * inferenceMarketMultimodal.test.js
 *
 * Tests for multi-modal inference features added in v3.1.124:
 *   - inferenceMarketRoutes.js  POST /api/jobs  image_cids / images / audio_cid / audio
 *   - inferenceMarket.js        openJob() with imageCids and audioCid opts
 *   - stateStore.js             INFERENCE_JOB_OPEN handler for image_cids / audio_cid
 */

const path = require("path");
const os = require("os");
const fs = require("fs");
const http = require("http");
const crypto = require("crypto");

// ── Test blob directory ────────────────────────────────────────────────────────
const TEST_BLOB_DIR = path.join(os.tmpdir(), "hone-test-mm-blobs-" + process.pid);
process.env.HONE_BLOB_DIR = TEST_BLOB_DIR;

// ── Mock P2P / network deps that route/service modules pull in ─────────────────
jest.mock("../src/p2p/mempool", () => ({
  submit: jest.fn().mockReturnValue({ accepted: true }),
  addTransaction: jest.fn().mockReturnValue(true),
}));
jest.mock("../src/p2p/network", () => ({
  broadcast: jest.fn(),
  NODE_ID: "test-node",
}));
jest.mock("../src/p2p/protocol", () => ({
  createMessage: jest.fn(() => ({})),
}));

// ── Bypass epoch bandwidth checks so tests don't fail on EB throttling ─────────
jest.mock("../src/services/epochBandwidth", () => ({
  checkAndDeduct: jest.fn().mockReturnValue({ allowed: true, eb_remaining: 9999, cost: 0 }),
  getBalance: jest.fn().mockReturnValue({ eb: 9999, max_eb: 9999, last_epoch: 0 }),
  OPERATION_COSTS: {},
  BANDWIDTH_PER_STAKE_PER_EPOCH: 1.0,
}));

// ── Mock auth middleware ───────────────────────────────────────────────────────
jest.mock("../src/middlewares/auth", () => ({
  authenticateToken: (req, res, next) => {
    const username = req.headers["x-test-user"];
    if (!username) return res.status(401).json({ error: "unauthenticated" });
    req.user = { username, id: "test-id-" + username };
    next();
  },
  limiter: (req, res, next) => next(),
}));

const express = require("express");
const stateStore = require("../src/chain/stateStore");
const ledger = require("../src/services/ledger");
const inferenceMarket = require("../src/services/inferenceMarket");
const inferenceMarketRoutes = require("../src/routes/inferenceMarketRoutes");

// ── Helpers ───────────────────────────────────────────────────────────────────

function makeServer() {
  const app = express();
  app.use(express.json({ limit: "10mb" }));
  app.use("/api/jobs", inferenceMarketRoutes);
  const server = http.createServer(app);
  return new Promise((resolve) => {
    server.listen(0, "127.0.0.1", () => {
      resolve({ server, port: server.address().port });
    });
  });
}

function closeServer(server) {
  if (!server) return Promise.resolve();
  return new Promise((resolve) => server.close(resolve));
}

function request(port, method, urlPath, body, opts) {
  opts = opts || {};
  return new Promise((resolve, reject) => {
    const headers = Object.assign({}, opts.headers || {});
    if (opts.user) headers["x-test-user"] = opts.user;
    let buf = null;
    if (body !== undefined && body !== null) {
      buf = Buffer.from(JSON.stringify(body));
      if (!headers["Content-Type"]) headers["Content-Type"] = "application/json";
      headers["Content-Length"] = buf.length;
    }
    const req = http.request(
      { host: "127.0.0.1", port, method, path: urlPath, headers },
      (res) => {
        const chunks = [];
        res.on("data", (c) => chunks.push(c));
        res.on("end", () => {
          const raw = Buffer.concat(chunks);
          let parsed;
          try { parsed = JSON.parse(raw.toString("utf8")); } catch (_) { parsed = raw.toString("utf8"); }
          resolve({ status: res.statusCode, body: parsed });
        });
      }
    );
    req.on("error", reject);
    if (buf) req.write(buf);
    req.end();
  });
}

/** Create an account with enough HONE balance to open jobs */
function setupBuyer(username, balance) {
  balance = balance !== undefined ? balance : 100;
  stateStore.applyEntry({
    type: "ACCOUNT_CREATE",
    to: username,
    epoch: 1,
    account_data: { username, public_keys: {}, chain_addresses: {} },
    timestamp: 1,
  });
  // Credit directly via MINING_REWARD so balance reads non-zero
  stateStore.applyEntry({
    type: "MINING_REWARD",
    to: username,
    amount: balance,
    token: "HONE",
    epoch: 1,
    timestamp: 1,
  });
}

function resetAll() {
  stateStore.resetAll();
  ledger.flushPendingEntries();
  if (fs.existsSync(TEST_BLOB_DIR)) {
    fs.rmSync(TEST_BLOB_DIR, { recursive: true, force: true });
  }
}

// ── Suite: POST /api/jobs — multimodal field handling ─────────────────────────

describe("POST /api/jobs — multimodal fields (routes layer)", () => {
  let testServer, port;

  beforeAll(async () => {
    const s = await makeServer();
    testServer = s.server;
    port = s.port;
  });

  afterAll(async () => {
    await closeServer(testServer);
    if (fs.existsSync(TEST_BLOB_DIR)) {
      fs.rmSync(TEST_BLOB_DIR, { recursive: true, force: true });
    }
  });

  beforeEach(() => {
    resetAll();
    setupBuyer("buyer1");
  });

  // ── image_cids pass-through ────────────────────────────────────────────────

  test("image_cids are passed through to openJob and returned in response", async () => {
    const cid1 = "a".repeat(64);
    const cid2 = "b".repeat(64);
    const r = await request(port, "POST", "/api/jobs", {
      prompt: "describe the images",
      max_fee: 0.5,
      image_cids: [cid1, cid2],
    }, { user: "buyer1" });

    expect(r.status).toBe(200);
    expect(r.body.job_id).toBeDefined();
    expect(Array.isArray(r.body.image_cids)).toBe(true);
    expect(r.body.image_cids).toContain(cid1);
    expect(r.body.image_cids).toContain(cid2);
  });

  test("image_cids is capped at 5 items", async () => {
    const cids = Array.from({ length: 8 }, (_, i) => String(i).repeat(64).slice(0, 64).padEnd(64, "0"));
    const r = await request(port, "POST", "/api/jobs", {
      prompt: "test",
      max_fee: 0.5,
      image_cids: cids,
    }, { user: "buyer1" });

    expect(r.status).toBe(200);
    expect(r.body.image_cids.length).toBeLessThanOrEqual(5);
  });

  // ── inline base64 images → written as blobs, converted to CIDs ────────────

  test("inline base64 images are stored as blobs and returned as image_cids", async () => {
    if (!fs.existsSync(TEST_BLOB_DIR)) fs.mkdirSync(TEST_BLOB_DIR, { recursive: true });

    // Tiny fake image bytes, base64-encoded
    const fakeImageBytes = Buffer.from("FAKEIMAGEDATA0001");
    const b64 = fakeImageBytes.toString("base64");
    const expectedCid = crypto.createHash("sha256").update(fakeImageBytes).digest("hex");

    const r = await request(port, "POST", "/api/jobs", {
      prompt: "analyse inline image",
      max_fee: 0.5,
      images: [b64],
    }, { user: "buyer1" });

    expect(r.status).toBe(200);
    expect(Array.isArray(r.body.image_cids)).toBe(true);
    expect(r.body.image_cids).toContain(expectedCid);

    // Blob should be written to disk
    const blobPath = path.join(TEST_BLOB_DIR, expectedCid);
    expect(fs.existsSync(blobPath)).toBe(true);
    expect(fs.readFileSync(blobPath).equals(fakeImageBytes)).toBe(true);
  });

  test("multiple inline base64 images all get individual CIDs", async () => {
    if (!fs.existsSync(TEST_BLOB_DIR)) fs.mkdirSync(TEST_BLOB_DIR, { recursive: true });

    const images = [
      Buffer.from("IMAGE_A"),
      Buffer.from("IMAGE_B"),
      Buffer.from("IMAGE_C"),
    ];
    const b64s = images.map((b) => b.toString("base64"));
    const expectedCids = images.map((b) =>
      crypto.createHash("sha256").update(b).digest("hex")
    );

    const r = await request(port, "POST", "/api/jobs", {
      prompt: "multi-image test",
      max_fee: 0.5,
      images: b64s,
    }, { user: "buyer1" });

    expect(r.status).toBe(200);
    for (const cid of expectedCids) {
      expect(r.body.image_cids).toContain(cid);
    }
  });

  // ── audio_cid pass-through ─────────────────────────────────────────────────

  test("audio_cid is passed through to openJob and returned in response", async () => {
    const audioCid = "c".repeat(64);
    const r = await request(port, "POST", "/api/jobs", {
      prompt: "transcribe and summarise",
      max_fee: 0.5,
      audio_cid: audioCid,
    }, { user: "buyer1" });

    expect(r.status).toBe(200);
    expect(r.body.audio_cid).toBe(audioCid);
  });

  // ── inline base64 audio → written as blob, sets audio_cid ─────────────────

  test("inline base64 audio is stored as blob and audio_cid is set in response", async () => {
    if (!fs.existsSync(TEST_BLOB_DIR)) fs.mkdirSync(TEST_BLOB_DIR, { recursive: true });

    const fakeAudioBytes = Buffer.from("FAKEAUDIODATA0001");
    const b64 = fakeAudioBytes.toString("base64");
    const expectedCid = crypto.createHash("sha256").update(fakeAudioBytes).digest("hex");

    const r = await request(port, "POST", "/api/jobs", {
      prompt: "transcribe this",
      max_fee: 0.5,
      audio: b64,
    }, { user: "buyer1" });

    expect(r.status).toBe(200);
    expect(r.body.audio_cid).toBe(expectedCid);

    const blobPath = path.join(TEST_BLOB_DIR, expectedCid);
    expect(fs.existsSync(blobPath)).toBe(true);
    expect(fs.readFileSync(blobPath).equals(fakeAudioBytes)).toBe(true);
  });

  test("audio_cid takes precedence over inline audio when both supplied", async () => {
    if (!fs.existsSync(TEST_BLOB_DIR)) fs.mkdirSync(TEST_BLOB_DIR, { recursive: true });

    const explicitCid = "d".repeat(64);
    const fakeAudioBytes = Buffer.from("SHOULD_BE_IGNORED");
    const b64 = fakeAudioBytes.toString("base64");

    const r = await request(port, "POST", "/api/jobs", {
      prompt: "audio test",
      max_fee: 0.5,
      audio_cid: explicitCid,
      audio: b64,
    }, { user: "buyer1" });

    expect(r.status).toBe(200);
    // When audio_cid is already set, audio base64 is not processed
    expect(r.body.audio_cid).toBe(explicitCid);
  });
});

// ── Suite: inferenceMarket.openJob() — multimodal opts ────────────────────────

describe("inferenceMarket.openJob() — imageCids and audioCid", () => {
  beforeEach(() => {
    resetAll();
    setupBuyer("testbuyer", 100);
  });

  test("openJob stores imageCids in the job and getInferenceJob returns them", async () => {
    const imageCids = ["e".repeat(64), "f".repeat(64)];
    const result = await inferenceMarket.openJob("testbuyer", "test prompt", 0.5, {
      imageCids,
    });

    expect(result.job_id).toBeDefined();
    expect(result.image_cids).toEqual(imageCids);

    const stored = stateStore.getInferenceJob(result.job_id);
    expect(stored).toBeTruthy();
    expect(stored.image_cids).toEqual(imageCids);
  });

  test("openJob stores audioCid in the job and getInferenceJob returns it", async () => {
    const audioCid = "g".repeat(64);
    const result = await inferenceMarket.openJob("testbuyer", "test prompt", 0.5, {
      audioCid,
    });

    expect(result.audio_cid).toBe(audioCid);

    const stored = stateStore.getInferenceJob(result.job_id);
    expect(stored).toBeTruthy();
    expect(stored.audio_cid).toBe(audioCid);
  });

  test("openJob with both imageCids and audioCid stores both correctly", async () => {
    const imageCids = ["h".repeat(64)];
    const audioCid = "i".repeat(64);

    const result = await inferenceMarket.openJob("testbuyer", "multimodal test", 0.5, {
      imageCids,
      audioCid,
    });

    const stored = stateStore.getInferenceJob(result.job_id);
    expect(stored.image_cids).toEqual(imageCids);
    expect(stored.audio_cid).toBe(audioCid);
  });

  test("openJob without imageCids defaults to empty array", async () => {
    const result = await inferenceMarket.openJob("testbuyer", "no images", 0.5, {});

    const stored = stateStore.getInferenceJob(result.job_id);
    expect(stored.image_cids).toEqual([]);
  });

  test("openJob without audioCid defaults to null", async () => {
    const result = await inferenceMarket.openJob("testbuyer", "no audio", 0.5, {});

    const stored = stateStore.getInferenceJob(result.job_id);
    expect(stored.audio_cid).toBeNull();
  });

  test("imageCids is capped at 5 items in openJob", async () => {
    const imageCids = Array.from({ length: 10 }, (_, i) => String(i).padStart(1, "0").repeat(64).slice(0, 64));
    const result = await inferenceMarket.openJob("testbuyer", "many images", 0.5, {
      imageCids,
    });

    const stored = stateStore.getInferenceJob(result.job_id);
    expect(stored.image_cids.length).toBeLessThanOrEqual(5);
  });
});

// ── Suite: stateStore INFERENCE_JOB_OPEN handler ──────────────────────────────

describe("stateStore INFERENCE_JOB_OPEN handler — multimodal fields", () => {
  beforeEach(() => {
    stateStore.resetAll();
  });

  function openJobEntry(overrides) {
    const jobId = "job_test_" + Math.random().toString(36).slice(2);
    return {
      type: "INFERENCE_JOB_OPEN",
      from: "buyer1",
      epoch: 5,
      timestamp: Date.now(),
      job_data: Object.assign({
        job_id: jobId,
        buyer: "buyer1",
        prompt: "test prompt",
        max_fee: 1.0,
        model: null,
        system_prompt: null,
        ttl_epochs: 20,
        expires_epoch: 25,
        tools: [],
        max_turns: 1,
        output_schema: null,
        tier: "standard",
        rag_cids: [],
        image_cids: [],
        audio_cid: null,
        batch_id: null,
        session_id: null,
      }, overrides),
    };
  }

  test("job with image_cids=['abc...64hex'] is stored correctly", () => {
    const cid = "j".repeat(64);
    const entry = openJobEntry({ image_cids: [cid] });
    stateStore.applyEntry(entry);

    const stored = stateStore.getInferenceJob(entry.job_data.job_id);
    expect(stored).toBeTruthy();
    expect(stored.image_cids).toEqual([cid]);
  });

  test("job without image_cids defaults to empty array", () => {
    const entry = openJobEntry({ image_cids: undefined });
    stateStore.applyEntry(entry);

    const stored = stateStore.getInferenceJob(entry.job_data.job_id);
    expect(stored.image_cids).toEqual([]);
  });

  test("job with audio_cid is stored correctly", () => {
    const audioCid = "k".repeat(64);
    const entry = openJobEntry({ audio_cid: audioCid });
    stateStore.applyEntry(entry);

    const stored = stateStore.getInferenceJob(entry.job_data.job_id);
    expect(stored.audio_cid).toBe(audioCid);
  });

  test("job without audio_cid defaults to null", () => {
    const entry = openJobEntry({ audio_cid: undefined });
    stateStore.applyEntry(entry);

    const stored = stateStore.getInferenceJob(entry.job_data.job_id);
    expect(stored.audio_cid).toBeNull();
  });

  test("job with multiple image_cids stores all of them", () => {
    const cids = ["l".repeat(64), "m".repeat(64), "n".repeat(64)];
    const entry = openJobEntry({ image_cids: cids });
    stateStore.applyEntry(entry);

    const stored = stateStore.getInferenceJob(entry.job_data.job_id);
    expect(stored.image_cids).toEqual(cids);
  });

  test("job with both image_cids and audio_cid stores both", () => {
    const cid = "o".repeat(64);
    const audioCid = "p".repeat(64);
    const entry = openJobEntry({ image_cids: [cid], audio_cid: audioCid });
    stateStore.applyEntry(entry);

    const stored = stateStore.getInferenceJob(entry.job_data.job_id);
    expect(stored.image_cids).toEqual([cid]);
    expect(stored.audio_cid).toBe(audioCid);
  });

  test("base job fields (status, buyer, prompt) are preserved alongside multimodal fields", () => {
    const cid = "q".repeat(64);
    const entry = openJobEntry({ image_cids: [cid], audio_cid: "r".repeat(64) });
    stateStore.applyEntry(entry);

    const stored = stateStore.getInferenceJob(entry.job_data.job_id);
    expect(stored.status).toBe("open");
    expect(stored.buyer).toBe("buyer1");
    expect(stored.prompt).toBe("test prompt");
    expect(stored.miner).toBeNull();
  });

  test("INFERENCE_JOB_OPEN with null image_cids in job_data defaults to []", () => {
    const entry = openJobEntry({ image_cids: null });
    stateStore.applyEntry(entry);

    const stored = stateStore.getInferenceJob(entry.job_data.job_id);
    expect(stored.image_cids).toEqual([]);
  });
});
