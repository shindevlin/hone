"use strict";

const dns = require("dns").promises;

jest.mock("../src/chain/stateStore", () => ({
  getBalance: jest.fn().mockReturnValue(0),
  getStakedAmount: jest.fn().mockReturnValue(0),
  getCurrentEpoch: jest.fn().mockReturnValue(1),
  getChainHeight: jest.fn().mockReturnValue(1),
  getActivePeerCount: jest.fn().mockReturnValue(0),
  getAccount: jest.fn().mockReturnValue(null),
  getDelegatedAmount: jest.fn().mockReturnValue(0),
  getAllSensors: jest.fn().mockReturnValue([]),
}));

const {
  PROTOCOL_NATIVE_TOOLS,
  canMinerExecute,
  isProtocolNativeTool,
  executeProtocolTool,
} = require("../src/services/protocolTools");

afterEach(() => {
  jest.restoreAllMocks();
  delete process.env.BTCPC_CODE_EXEC_ENABLED;
});

// ── Issue 1: execute_code removed from buyer-accessible tool set ───────────────

describe("execute_code — removed from protocol-native set", () => {
  test("execute_code is not in PROTOCOL_NATIVE_TOOLS", () => {
    expect(PROTOCOL_NATIVE_TOOLS.has("execute_code")).toBe(false);
  });

  test("isProtocolNativeTool returns false for execute_code", () => {
    expect(isProtocolNativeTool("execute_code")).toBe(false);
  });

  test("canMinerExecute returns false for execute_code", () => {
    expect(canMinerExecute("execute_code")).toBe(false);
  });
});

describe("execute_code — opt-in disabled by default", () => {
  test("returns disabled error when env var is not set", async () => {
    delete process.env.BTCPC_CODE_EXEC_ENABLED;
    const result = await executeProtocolTool("execute_code", { language: "bash", code: "id" });
    expect(result.error).toBe("disabled");
    expect(result.trusted).toBe(true);
  });

  test("returns disabled error when env var is explicitly 'false'", async () => {
    process.env.BTCPC_CODE_EXEC_ENABLED = "false";
    const result = await executeProtocolTool("execute_code", { language: "bash", code: "id" });
    expect(result.error).toBe("disabled");
  });

  test("returns disabled error when env var is a non-true value ('1')", async () => {
    process.env.BTCPC_CODE_EXEC_ENABLED = "1";
    const result = await executeProtocolTool("execute_code", { language: "bash", code: "id" });
    expect(result.error).toBe("disabled");
  });
});

// ── Issue 2: web_fetch SSRF protection ────────────────────────────────────────

describe("web_fetch — SSRF guard", () => {
  test("rejects http://127.0.0.1 (loopback literal)", async () => {
    const result = await executeProtocolTool("web_fetch", { url: "http://127.0.0.1/secret" });
    expect(result.error).toBe("ssrf_blocked");
    expect(result.trusted).toBe(false);
  });

  test("rejects http://localhost", async () => {
    const result = await executeProtocolTool("web_fetch", { url: "http://localhost/admin" });
    expect(result.error).toBe("ssrf_blocked");
  });

  test("rejects http://10.0.0.1 (RFC 1918 class A)", async () => {
    const result = await executeProtocolTool("web_fetch", { url: "http://10.0.0.1/internal" });
    expect(result.error).toBe("ssrf_blocked");
  });

  test("rejects http://192.168.1.1 (RFC 1918 class C)", async () => {
    const result = await executeProtocolTool("web_fetch", { url: "http://192.168.1.1/" });
    expect(result.error).toBe("ssrf_blocked");
  });

  test("rejects http://[::1] (IPv6 loopback)", async () => {
    const result = await executeProtocolTool("web_fetch", { url: "http://[::1]/secret" });
    expect(result.error).toBe("ssrf_blocked");
  });

  test("rejects http://[::ffff:127.0.0.1] (IPv4-mapped loopback)", async () => {
    const result = await executeProtocolTool("web_fetch", { url: "http://[::ffff:127.0.0.1]/secret" });
    expect(result.error).toBe("ssrf_blocked");
  });

  test("rejects decimal-encoded loopback IP (2130706433 = 127.0.0.1)", async () => {
    const result = await executeProtocolTool("web_fetch", { url: "http://2130706433/secret" });
    expect(result.error).toBe("ssrf_blocked");
  });

  test("rejects hex-encoded loopback IP (0x7f000001 = 127.0.0.1)", async () => {
    const result = await executeProtocolTool("web_fetch", { url: "http://0x7f000001/secret" });
    expect(result.error).toBe("ssrf_blocked");
  });

  test("rejects hostname that DNS-resolves to a private address", async () => {
    jest.spyOn(dns, "lookup").mockResolvedValue([{ address: "10.0.0.5", family: 4 }]);
    const result = await executeProtocolTool("web_fetch", { url: "http://internal.corp/data" });
    expect(result.error).toBe("ssrf_blocked");
  });

  test("rejects non-http schemes", async () => {
    const result = await executeProtocolTool("web_fetch", { url: "file:///etc/passwd" });
    expect(result.error).toBe("invalid_url");
  });
});
