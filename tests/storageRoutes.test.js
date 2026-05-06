"use strict";

const http = require("http");
const express = require("express");

jest.mock("../src/chain/stateStore", () => ({
  getChainHeight: jest.fn(() => 29167),
  getActiveStorageHosts: jest.fn(() => ([
    {
      host: "storage-a",
      last_heartbeat_epoch: 29160,
      total_heartbeats: 9,
      uptime_window_start: 29000,
      capacity_used_gb: 12.5,
      cids: ["cid-a", "cid-b"],
      last_seen: 1713900000000,
    },
  ])),
  getStorageHeartbeat: jest.fn(() => ({
    host: "storage-a",
    last_heartbeat_epoch: 29160,
    total_heartbeats: 9,
    uptime_window_start: 29000,
    capacity_used_gb: 12.5,
    cids: ["cid-a", "cid-b"],
    last_seen: 1713900000000,
  })),
  getAccount: jest.fn(),
}));

jest.mock("../src/services/ledger", () => ({
  recordStorageHeartbeat: jest.fn(),
  recordFileStore: jest.fn(),
  recordFileGrant: jest.fn(),
  recordFileRevoke: jest.fn(),
  recordFileShard: jest.fn(),
}));

jest.mock("../src/services/storageCrypto", () => ({
  generateDEK: jest.fn(),
  encryptManifest: jest.fn(),
  wrapDEK: jest.fn(),
  splitDEK: jest.fn(),
  wrapJSON: jest.fn(),
}));

jest.mock("../src/wallet/keyManager", () => ({
  verifySignature: jest.fn(),
}));

const storageRoutes = require("../src/routes/storageRoutes");

function makeTestServer() {
  const app = express();
  app.use(express.json());
  app.use("/api/storage", storageRoutes);
  const server = http.createServer(app);
  return new Promise((resolve) => {
    server.listen(0, "127.0.0.1", () => {
      resolve({ server, port: server.address().port });
    });
  });
}

function closeServer(server) {
  return new Promise((resolve) => server.close(resolve));
}

function request(port, method, path) {
  return new Promise((resolve, reject) => {
    const req = http.request({ host: "127.0.0.1", port, method, path }, (res) => {
      let raw = "";
      res.on("data", (chunk) => (raw += chunk));
      res.on("end", () => {
        let parsed = null;
        try { parsed = raw ? JSON.parse(raw) : null; } catch (_) { parsed = raw; }
        resolve({ status: res.statusCode, body: parsed });
      });
    });
    req.on("error", reject);
    req.end();
  });
}

describe("storage routes redaction", () => {
  let server;
  let port;

  beforeAll(async () => {
    const s = await makeTestServer();
    server = s.server;
    port = s.port;
  });

  afterAll(async () => {
    if (server) await closeServer(server);
  });

  test("GET /hosts does not expose raw CID lists", async () => {
    const res = await request(port, "GET", "/api/storage/hosts");
    expect(res.status).toBe(200);
    expect(res.body.hosts).toHaveLength(1);
    expect(res.body.hosts[0]).not.toHaveProperty("cids");
    expect(res.body.hosts[0].cids_count).toBe(2);
  });

  test("GET /hosts/:host redacts raw CID lists", async () => {
    const res = await request(port, "GET", "/api/storage/hosts/storage-a");
    expect(res.status).toBe(200);
    expect(res.body.record).not.toHaveProperty("cids");
    expect(res.body.record.cids_count).toBe(2);
  });
});
