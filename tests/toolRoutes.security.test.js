"use strict";

const http = require("http");
const express = require("express");

jest.mock("../src/services/ledger", () => ({
  recordToolCapabilityRegister: jest.fn().mockResolvedValue({ ok: true }),
}));

jest.mock("../src/chain/stateStore", () => ({
  getChainHeight: jest.fn().mockReturnValue(123),
}));

const toolRoutes = require("../src/routes/toolRoutes");
const mcpServer = require("../src/mcp/btcpcMcpServer");
const ledger = require("../src/services/ledger");

function makeTestServer() {
  const app = express();
  app.use(express.json());
  app.use(toolRoutes);
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

function request(port, method, path, body) {
  return new Promise((resolve, reject) => {
    const headers = { "Content-Type": "application/json" };
    const data = body ? JSON.stringify(body) : null;
    if (data) headers["Content-Length"] = Buffer.byteLength(data);
    const req = http.request(
      { host: "127.0.0.1", port, method, path, headers },
      (res) => {
        let raw = "";
        res.on("data", (chunk) => (raw += chunk));
        res.on("end", () => {
          let parsed = null;
          try { parsed = raw ? JSON.parse(raw) : null; } catch (_) { parsed = raw; }
          resolve({ status: res.statusCode, body: parsed });
        });
      }
    );
    req.on("error", reject);
    if (data) req.write(data);
    req.end();
  });
}

describe("tool routes security", () => {
  let testServer;
  let port;

  beforeAll(async () => {
    const s = await makeTestServer();
    testServer = s.server;
    port = s.port;
  });

  afterAll(async () => {
    if (testServer) await closeServer(testServer);
  });

  beforeEach(() => {
    jest.clearAllMocks();
  });

  test("POST /api/tools/call executes builtin tools on local requests", async () => {
    const r = await request(port, "POST", "/api/tools/call", {
      tool: "calculator",
      input: { expr: "2+2" },
    });
    expect(r.status).toBe(200);
    expect(r.body.ok).toBe(true);
    expect(r.body.output.result).toBe(4);
  });

  test("POST /api/tools/register records capability for local requests", async () => {
    const r = await request(port, "POST", "/api/tools/register", {
      name: "demo-tool",
      command: "echo demo",
      account: "alice",
    });
    expect(r.status).toBe(200);
    expect(r.body.ok).toBe(true);
    expect(ledger.recordToolCapabilityRegister).toHaveBeenCalledWith(
      "alice",
      [{ name: "demo-tool", kind: "cli", command: "echo demo", description: "", deterministic: false }],
      123
    );
  });

  test("MCP server management still reaches the local sidecar", async () => {
    const startSpy = jest.spyOn(mcpServer, "start").mockImplementation(() => ({ started: true }));
    const stopSpy = jest.spyOn(mcpServer, "stop").mockImplementation(() => undefined);
    const startRes = await request(port, "POST", "/api/tools/mcp/start", {});
    const stopRes = await request(port, "POST", "/api/tools/mcp/stop", {});
    expect(startRes.status).toBe(200);
    expect(stopRes.status).toBe(200);
    expect(startSpy).toHaveBeenCalled();
    expect(stopSpy).toHaveBeenCalled();
  });
});
