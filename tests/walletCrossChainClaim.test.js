"use strict";

const fs = require("fs");
const os = require("os");
const path = require("path");
const http = require("http");
const express = require("express");

function makeTempDir() {
  const dir = path.join(os.tmpdir(), "hone-wallet-claim-" + process.pid + "-" + Math.random().toString(36).slice(2));
  fs.mkdirSync(dir, { recursive: true });
  return dir;
}

function removeTempDir(dir) {
  try {
    fs.rmSync(dir, { recursive: true, force: true });
  } catch (_) {}
}

jest.mock("../src/middlewares/auth", () => ({
  authenticateToken: (req, res, next) => {
    req.user = { username: "alice" };
    next();
  },
}));

const mockLedger = {
  recordCrossChainClaim: jest.fn(),
};

jest.mock("../src/services/ledger", () => mockLedger);

const mockStateStore = {
  CROSS_CHAIN_SUPPORTED: ["base"],
  getAccount: jest.fn(() => ({ chain_addresses: { base: "0x" + "2".repeat(40) } })),
  getCrossChainCredits: jest.fn(() => ({ base: 3.5 })),
  getChainHeight: jest.fn(() => 101),
};

jest.mock("../src/chain/stateStore", () => mockStateStore);

describe("wallet cross-chain claim cutoff enforcement", () => {
  let tempDir;
  let prevDataDir;
  let prevBaseContract;
  let router;
  let server;
  let port;

  beforeEach(async () => {
    tempDir = makeTempDir();
    prevDataDir = process.env.HONE_DATA_DIR;
    prevBaseContract = process.env.WHONE_BASE_CONTRACT;
    process.env.HONE_DATA_DIR = tempDir;
    process.env.WHONE_BASE_CONTRACT = "0x" + "1".repeat(40);

    const consumer = require("../src/services/crossChainFinalityConsumer");
    const chainDir = path.join(consumer.getCrossChainDir(), "base");
    fs.mkdirSync(chainDir, { recursive: true });
    fs.writeFileSync(path.join(chainDir, "finality-00000100.json"), JSON.stringify({
      target_chain: "base",
      finality_epoch: 100,
      cutoff_epoch: 100,
      finalized_job_count: 1,
      finalized_jobs: [{ job_id: "job-1" }],
      announcement_hash: "a".repeat(64),
    }, null, 2));

    jest.resetModules();
    router = require("../src/routes/walletRoutes");

    const app = express();
    app.use(express.json());
    app.use("/api/wallet", router);
    server = http.createServer(app);
    await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
    port = server.address().port;
    mockLedger.recordCrossChainClaim.mockReset();
  });

  afterEach(async () => {
    if (server) await new Promise((resolve) => server.close(resolve));
    if (prevDataDir === undefined) delete process.env.HONE_DATA_DIR;
    else process.env.HONE_DATA_DIR = prevDataDir;
    if (prevBaseContract === undefined) delete process.env.WHONE_BASE_CONTRACT;
    else process.env.WHONE_BASE_CONTRACT = prevBaseContract;
    removeTempDir(tempDir);
  });

  function request(method, route, body) {
    return new Promise((resolve, reject) => {
      const data = body ? JSON.stringify(body) : null;
      const headers = { "Content-Type": "application/json" };
      if (data) headers["Content-Length"] = Buffer.byteLength(data);
      const req = http.request({ host: "127.0.0.1", port, method, path: route, headers }, (res) => {
        let raw = "";
        res.on("data", (chunk) => (raw += chunk));
        res.on("end", () => {
          let parsed = null;
          try { parsed = raw ? JSON.parse(raw) : null; } catch (_) { parsed = raw; }
          resolve({ status: res.statusCode, body: parsed });
        });
      });
      req.on("error", reject);
      if (data) req.write(data);
      req.end();
    });
  }

  test("rejects claim generation before finality cutoff", async () => {
    const res = await request("POST", "/api/wallet/claim-cross-chain", {
      chain: "base",
      amount: 1,
      signature: "0x" + "3".repeat(130),
    });

    expect(res.status).toBe(409);
    expect(res.body.finality_cutoff).toBe(100);
    expect(mockLedger.recordCrossChainClaim).not.toHaveBeenCalled();
  });
});
