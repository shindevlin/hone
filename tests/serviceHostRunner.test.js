"use strict";

/**
 * HONE Service Host Runner — v2.13-beta tests
 *
 * Exercises serviceHostRunner via a mock spawner and mock blob fetcher.
 * No real child processes are forked — the mock returns a handle with
 * a controllable .on("exit") emitter so we can simulate crashes.
 */

var runner = require("../src/services/serviceHostRunner");

function makeMockProcess() {
  var listeners = {};
  return {
    killed: false,
    pid: Math.floor(Math.random() * 100000),
    on: function (event, cb) {
      listeners[event] = cb;
    },
    kill: function (_signal) {
      this.killed = true;
      if (listeners.exit) listeners.exit(0, "SIGTERM");
    },
    _triggerExit: function (code, signal) {
      if (listeners.exit) listeners.exit(code, signal);
    },
  };
}

function makeMockSpawner(processes) {
  return function (command, args, options) {
    var p = makeMockProcess();
    p._command = command;
    p._args = args;
    p._options = options;
    processes.push(p);
    return p;
  };
}

function makeMockBlobFetcher(blobs) {
  return function (cid) {
    if (!blobs[cid]) throw new Error("blob not found: " + cid);
    return blobs[cid];
  };
}

function makeDeployment(overrides) {
  var dep = {
    slug: "alice/api",
    deployer: "alice",
    runtime: {
      type: "http",
      binary_cid: "a".repeat(64),
      command: "node",
      args: ["server.js"],
      port: 8080,
    },
  };
  if (overrides) Object.assign(dep, overrides);
  return dep;
}

describe("serviceHostRunner — registration", function () {
  it("registers a deployment and returns a record in pending state", function () {
    var r = runner.createRunner({
      spawner: makeMockSpawner([]),
      blobFetcher: makeMockBlobFetcher({ ["a".repeat(64)]: Buffer.from("x") }),
    });
    var rec = r.register(makeDeployment());
    expect(rec.slug).toBe("alice/api");
    expect(rec.state).toBe("pending");
    expect(rec.crash_count).toBe(0);
    expect(r.getServiceCount()).toBe(1);
  });

  it("rejects a deployment without a slug", function () {
    var r = runner.createRunner({});
    expect(function () { r.register({}); }).toThrow(/slug required/);
  });

  it("rejects a deployment without a binary_cid", function () {
    var r = runner.createRunner({});
    expect(function () {
      r.register({ slug: "alice/api", runtime: { type: "http" } });
    }).toThrow(/binary_cid required/);
  });

  it("rejects duplicate registrations", function () {
    var r = runner.createRunner({
      blobFetcher: makeMockBlobFetcher({ ["a".repeat(64)]: Buffer.from("x") }),
    });
    r.register(makeDeployment());
    expect(function () { r.register(makeDeployment()); }).toThrow(/already registered/);
  });
});

describe("serviceHostRunner — prepareBinary", function () {
  it("moves pending → downloading → ready", function () {
    var r = runner.createRunner({
      blobFetcher: makeMockBlobFetcher({ ["a".repeat(64)]: Buffer.from("payload") }),
    });
    r.register(makeDeployment());
    var rec = r.prepareBinary("alice/api");
    expect(rec.state).toBe("ready");
    expect(rec.binary_bytes.toString()).toBe("payload");
  });

  it("marks service failed if blob is missing", function () {
    var r = runner.createRunner({
      blobFetcher: makeMockBlobFetcher({}),
    });
    r.register(makeDeployment());
    expect(function () { r.prepareBinary("alice/api"); }).toThrow();
    expect(r.getStatus("alice/api").state).toBe("failed");
    expect(r.getStatus("alice/api").failure_reason).toMatch(/blob/);
  });

  it("is a no-op if already prepared", function () {
    var r = runner.createRunner({
      blobFetcher: makeMockBlobFetcher({ ["a".repeat(64)]: Buffer.from("x") }),
    });
    r.register(makeDeployment());
    r.prepareBinary("alice/api");
    var again = r.prepareBinary("alice/api");
    expect(again.state).toBe("ready");
  });
});

describe("serviceHostRunner — start", function () {
  it("spawns a process and moves state to running", function () {
    var processes = [];
    var r = runner.createRunner({
      spawner: makeMockSpawner(processes),
      blobFetcher: makeMockBlobFetcher({ ["a".repeat(64)]: Buffer.from("x") }),
    });
    r.register(makeDeployment());
    var rec = r.start("alice/api");
    expect(rec.state).toBe("running");
    expect(rec.started_at).toBeGreaterThan(0);
    expect(processes.length).toBe(1);
    expect(processes[0]._command).toBe("node");
    expect(processes[0]._args).toEqual(["server.js"]);
    expect(rec.port_addr).toBe("host:8080");
  });

  it("auto-prepares binary if in pending state", function () {
    var processes = [];
    var r = runner.createRunner({
      spawner: makeMockSpawner(processes),
      blobFetcher: makeMockBlobFetcher({ ["a".repeat(64)]: Buffer.from("x") }),
    });
    r.register(makeDeployment());
    r.start("alice/api");
    expect(r.getStatus("alice/api").state).toBe("running");
  });

  it("does not spawn a process for static services", function () {
    var processes = [];
    var r = runner.createRunner({
      spawner: makeMockSpawner(processes),
      blobFetcher: makeMockBlobFetcher({ ["a".repeat(64)]: Buffer.from("<html></html>") }),
    });
    r.register(makeDeployment({
      runtime: { type: "static", binary_cid: "a".repeat(64), port: 443 },
    }));
    var rec = r.start("alice/api");
    expect(rec.state).toBe("running");
    expect(processes.length).toBe(0);
    expect(rec.port_addr).toBe("host:443");
  });

  it("transitions to crashed when the child process exits unexpectedly", function () {
    var processes = [];
    var r = runner.createRunner({
      spawner: makeMockSpawner(processes),
      blobFetcher: makeMockBlobFetcher({ ["a".repeat(64)]: Buffer.from("x") }),
    });
    r.register(makeDeployment());
    r.start("alice/api");
    processes[0]._triggerExit(1, "SIGSEGV");
    var status = r.getStatus("alice/api");
    expect(status.state).toBe("crashed");
    expect(status.crash_count).toBe(1);
    expect(status.exit_code).toBe(1);
    expect(status.exit_signal).toBe("SIGSEGV");
  });

  it("marks service failed if spawner throws", function () {
    var badSpawner = function () { throw new Error("ENOENT"); };
    var r = runner.createRunner({
      spawner: badSpawner,
      blobFetcher: makeMockBlobFetcher({ ["a".repeat(64)]: Buffer.from("x") }),
    });
    r.register(makeDeployment());
    expect(function () { r.start("alice/api"); }).toThrow(/ENOENT/);
    expect(r.getStatus("alice/api").state).toBe("failed");
  });

  it("rejects start from an invalid state", function () {
    var r = runner.createRunner({
      spawner: makeMockSpawner([]),
      blobFetcher: makeMockBlobFetcher({ ["a".repeat(64)]: Buffer.from("x") }),
    });
    r.register(makeDeployment());
    r.start("alice/api");
    expect(function () { r.start("alice/api"); }).toThrow(/cannot start/);
  });
});

describe("serviceHostRunner — stop", function () {
  it("transitions running → stopped and kills the child", function () {
    var processes = [];
    var r = runner.createRunner({
      spawner: makeMockSpawner(processes),
      blobFetcher: makeMockBlobFetcher({ ["a".repeat(64)]: Buffer.from("x") }),
    });
    r.register(makeDeployment());
    r.start("alice/api");
    var rec = r.stop("alice/api");
    expect(rec.state).toBe("stopped");
    expect(processes[0].killed).toBe(true);
    expect(rec.stopped_at).toBeGreaterThan(0);
  });

  it("is a no-op if service is already stopped", function () {
    var r = runner.createRunner({
      spawner: makeMockSpawner([]),
      blobFetcher: makeMockBlobFetcher({ ["a".repeat(64)]: Buffer.from("x") }),
    });
    r.register(makeDeployment());
    r.start("alice/api");
    r.stop("alice/api");
    var rec = r.stop("alice/api");
    expect(rec.state).toBe("stopped");
  });

  it("does not mark the service crashed when the exit handler fires from SIGTERM", function () {
    var processes = [];
    var r = runner.createRunner({
      spawner: makeMockSpawner(processes),
      blobFetcher: makeMockBlobFetcher({ ["a".repeat(64)]: Buffer.from("x") }),
    });
    r.register(makeDeployment());
    r.start("alice/api");
    r.stop("alice/api");
    expect(r.getStatus("alice/api").state).toBe("stopped");
    expect(r.getStatus("alice/api").crash_count).toBe(0);
  });
});

describe("serviceHostRunner — restart", function () {
  it("restarts a crashed service", function () {
    var processes = [];
    var r = runner.createRunner({
      spawner: makeMockSpawner(processes),
      blobFetcher: makeMockBlobFetcher({ ["a".repeat(64)]: Buffer.from("x") }),
    });
    r.register(makeDeployment());
    r.start("alice/api");
    processes[0]._triggerExit(1, "SIGSEGV");
    expect(r.getStatus("alice/api").state).toBe("crashed");
    var rec = r.restart("alice/api");
    expect(rec.state).toBe("running");
    expect(processes.length).toBe(2);
  });

  it("rejects restart for non-crashed services", function () {
    var r = runner.createRunner({
      spawner: makeMockSpawner([]),
      blobFetcher: makeMockBlobFetcher({ ["a".repeat(64)]: Buffer.from("x") }),
    });
    r.register(makeDeployment());
    r.start("alice/api");
    expect(function () { r.restart("alice/api"); }).toThrow(/can only restart crashed/);
  });
});

describe("serviceHostRunner — healthCheck", function () {
  it("returns healthy for a running service", function () {
    var r = runner.createRunner({
      spawner: makeMockSpawner([]),
      blobFetcher: makeMockBlobFetcher({ ["a".repeat(64)]: Buffer.from("x") }),
    });
    r.register(makeDeployment());
    r.start("alice/api");
    var hc = r.healthCheck("alice/api");
    expect(hc.healthy).toBe(true);
    expect(r.getStatus("alice/api").last_health_check).toBeGreaterThan(0);
  });

  it("returns unhealthy for unknown services", function () {
    var r = runner.createRunner({});
    var hc = r.healthCheck("nope/nada");
    expect(hc.healthy).toBe(false);
    expect(hc.reason).toMatch(/not registered/);
  });

  it("returns unhealthy when the process has been killed", function () {
    var processes = [];
    var r = runner.createRunner({
      spawner: makeMockSpawner(processes),
      blobFetcher: makeMockBlobFetcher({ ["a".repeat(64)]: Buffer.from("x") }),
    });
    r.register(makeDeployment());
    r.start("alice/api");
    processes[0].killed = true;
    var hc = r.healthCheck("alice/api");
    expect(hc.healthy).toBe(false);
    expect(r.getStatus("alice/api").state).toBe("crashed");
  });
});

describe("serviceHostRunner — tickHeartbeats", function () {
  it("fires onHeartbeat for each running service", function () {
    var beats = [];
    var r = runner.createRunner({
      spawner: makeMockSpawner([]),
      blobFetcher: makeMockBlobFetcher({
        ["a".repeat(64)]: Buffer.from("x"),
        ["b".repeat(64)]: Buffer.from("y"),
      }),
      onHeartbeat: function (hb) { beats.push(hb); },
    });
    r.register(makeDeployment());
    r.register(makeDeployment({
      slug: "bob/worker",
      deployer: "bob",
      runtime: { type: "tcp", binary_cid: "b".repeat(64), port: 9090 },
    }));
    r.start("alice/api");
    r.start("bob/worker");
    var ticks = r.tickHeartbeats(42);
    expect(ticks.length).toBe(2);
    expect(beats.length).toBe(2);
    expect(beats[0].epoch).toBe(42);
    expect(beats[0].state).toBe("running");
    var slugs = ticks.map(function (t) { return t.slug; }).sort();
    expect(slugs).toEqual(["alice/api", "bob/worker"]);
  });

  it("skips non-running services", function () {
    var beats = [];
    var r = runner.createRunner({
      spawner: makeMockSpawner([]),
      blobFetcher: makeMockBlobFetcher({ ["a".repeat(64)]: Buffer.from("x") }),
      onHeartbeat: function (hb) { beats.push(hb); },
    });
    r.register(makeDeployment());
    var ticks = r.tickHeartbeats(1);
    expect(ticks.length).toBe(0);
    expect(beats.length).toBe(0);
  });

  it("does not propagate exceptions from onHeartbeat callbacks", function () {
    var r = runner.createRunner({
      spawner: makeMockSpawner([]),
      blobFetcher: makeMockBlobFetcher({ ["a".repeat(64)]: Buffer.from("x") }),
      onHeartbeat: function () { throw new Error("boom"); },
    });
    r.register(makeDeployment());
    r.start("alice/api");
    expect(function () { r.tickHeartbeats(1); }).not.toThrow();
  });
});

describe("serviceHostRunner — accessors", function () {
  it("getAllStatuses and getRunningServices reflect state", function () {
    var processes = [];
    var r = runner.createRunner({
      spawner: makeMockSpawner(processes),
      blobFetcher: makeMockBlobFetcher({
        ["a".repeat(64)]: Buffer.from("x"),
        ["b".repeat(64)]: Buffer.from("y"),
      }),
    });
    r.register(makeDeployment());
    r.register(makeDeployment({
      slug: "bob/worker",
      deployer: "bob",
      runtime: { type: "tcp", binary_cid: "b".repeat(64), port: 9090 },
    }));
    r.start("alice/api");
    r.start("bob/worker");
    processes[0]._triggerExit(1, "SIGSEGV");
    expect(r.getAllStatuses().length).toBe(2);
    expect(r.getRunningServices().length).toBe(1);
    expect(r.getRunningServices()[0].slug).toBe("bob/worker");
  });

  it("unregister stops a running service and removes it", function () {
    var processes = [];
    var r = runner.createRunner({
      spawner: makeMockSpawner(processes),
      blobFetcher: makeMockBlobFetcher({ ["a".repeat(64)]: Buffer.from("x") }),
    });
    r.register(makeDeployment());
    r.start("alice/api");
    expect(r.unregister("alice/api")).toBe(true);
    expect(r.getServiceCount()).toBe(0);
    expect(processes[0].killed).toBe(true);
  });

  it("reset clears all services", function () {
    var r = runner.createRunner({
      spawner: makeMockSpawner([]),
      blobFetcher: makeMockBlobFetcher({ ["a".repeat(64)]: Buffer.from("x") }),
    });
    r.register(makeDeployment());
    r.start("alice/api");
    r.reset();
    expect(r.getServiceCount()).toBe(0);
  });
});

describe("serviceHostRunner — end-to-end lifecycle", function () {
  it("supports the full pending → running → crashed → restart → stopped flow", function () {
    var processes = [];
    var beats = [];
    var r = runner.createRunner({
      spawner: makeMockSpawner(processes),
      blobFetcher: makeMockBlobFetcher({ ["a".repeat(64)]: Buffer.from("bin") }),
      onHeartbeat: function (hb) { beats.push(hb); },
    });

    // register
    var rec = r.register(makeDeployment());
    expect(rec.state).toBe("pending");

    // prepare + start
    r.start("alice/api");
    expect(r.getStatus("alice/api").state).toBe("running");

    // heartbeat while running
    r.tickHeartbeats(10);
    expect(beats.length).toBe(1);

    // crash
    processes[0]._triggerExit(137, "SIGKILL");
    expect(r.getStatus("alice/api").state).toBe("crashed");

    // heartbeat skipped while crashed
    r.tickHeartbeats(11);
    expect(beats.length).toBe(1);

    // restart
    r.restart("alice/api");
    expect(r.getStatus("alice/api").state).toBe("running");
    expect(processes.length).toBe(2);

    // stop cleanly
    r.stop("alice/api");
    expect(r.getStatus("alice/api").state).toBe("stopped");
  });
});
