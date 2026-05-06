"use strict";

describe("rustChainIpc", () => {
  var ORIGINAL_ENV = process.env;

  beforeEach(() => {
    jest.resetModules();
    process.env = Object.assign({}, ORIGINAL_ENV);
    delete process.env.BTCPC_CHAIN_SOCK;
    delete process.env.BTCPC_CHAIN_IPC_TIMEOUT_MS;
  });

  afterEach(() => {
    process.env = ORIGINAL_ENV;
    jest.restoreAllMocks();
    jest.dontMock("child_process");
    jest.dontMock("fs");
  });

  function loadWithMocks(spawnResults) {
    var spawnSync = jest.fn();
    for (var i = 0; i < spawnResults.length; i++) {
      spawnSync.mockReturnValueOnce(spawnResults[i]);
    }
    var existsSync = jest.fn(function () { return true; });

    jest.doMock("child_process", function () { return { spawnSync: spawnSync }; });
    jest.doMock("fs", function () { return { existsSync: existsSync }; });

    var ipc = require("../src/chain/rustChainIpc");
    return { ipc: ipc, spawnSync: spawnSync, existsSync: existsSync };
  }

  test("writeBlock sends block_write over nc transport", () => {
    var mocked = loadWithMocks([
      { status: 0, stdout: "{\"id\":1,\"result\":true}\n", stderr: "" }
    ]);

    var ok = mocked.ipc.writeBlock(7, Buffer.from("abc"));
    expect(ok).toBe(true);
    expect(mocked.spawnSync).toHaveBeenCalledTimes(1);

    var call = mocked.spawnSync.mock.calls[0];
    expect(call[0]).toBe("nc");
    expect(call[1]).toEqual(["-U", "/tmp/btcpc-chain.sock"]);

    var request = JSON.parse(call[2].input.trim());
    expect(request.method).toBe("block_write");
    expect(request.params.epoch).toBe(7);
    expect(request.params.data_b64).toBe(Buffer.from("abc").toString("base64"));
  });

  test("falls back to python3 transport when nc is unavailable", () => {
    var missingNc = new Error("spawn nc ENOENT");
    missingNc.code = "ENOENT";

    var mocked = loadWithMocks([
      { error: missingNc },
      { status: 0, stdout: "{\"id\":2,\"result\":-1}\n", stderr: "" }
    ]);

    var latest = mocked.ipc.latestBlock();
    expect(latest).toBe(-1);
    expect(mocked.spawnSync).toHaveBeenCalledTimes(2);
    expect(mocked.spawnSync.mock.calls[0][0]).toBe("nc");
    expect(mocked.spawnSync.mock.calls[1][0]).toBe("python3");
  });

  test("throws when rpc response contains an error object", () => {
    var mocked = loadWithMocks([
      {
        status: 0,
        stdout: "{\"id\":3,\"error\":{\"code\":-32000,\"message\":\"boom\"}}\n",
        stderr: ""
      }
    ]);

    expect(() => mocked.ipc.latestFinality()).toThrow("block_latest_finality");
  });
});
