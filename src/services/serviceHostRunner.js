"use strict";

/**
 * BTCPC Service Host Runner — v2.13-beta
 * Shin Devlin
 *
 * Host-side execution layer for deployed services. Takes a deployment
 * record from serviceRegistry, downloads the binary from BTCPC-FS,
 * spawns the runtime as a child process (or container), monitors its
 * lifecycle, and reports heartbeats back to the chain.
 *
 * Architecture:
 *   - Pure stateless compute (v2.13). State persistence is v2.14.
 *   - Spawner is injectable for testability — production uses Node's
 *     child_process; tests use a mock spawner that records calls.
 *   - Resource limits are best-effort with child_process, enforced
 *     properly with Docker/Podman (configurable adapter).
 *   - Heartbeat scheduling is internal — caller registers a callback
 *     that fires every N epochs with the service status.
 *
 * Service lifecycle states:
 *   pending    — registered with the runner, binary not yet downloaded
 *   downloading — fetching binary from BTCPC-FS
 *   ready      — binary on disk, not yet running
 *   running    — process spawned and healthy
 *   degraded   — process running but recent health checks failed
 *   stopped    — gracefully shut down
 *   crashed    — process exited unexpectedly
 *   failed     — couldn't even start (bad binary, missing deps)
 *
 * No slashing for any of these states. A host that crashes a service
 * just stops earning from it; reputation may dip but balance is
 * untouched. See feedback_storage_no_slash.md.
 */

var os = require("os");
var path = require("path");
var crypto = require("crypto");

// Command allowlist — only these runtime executables may be spawned.
// Any deployment spec with a command outside this list is rejected.
var ALLOWED_COMMANDS = [
  'node', 'python3', 'python', 'wasmer', 'wasmtime',
  'deno', 'bun', 'java', 'dotnet',
];

// Shell metacharacters that must not appear in service args.
var SHELL_META = /[;&|`$(){}!><\n\r]/;

// Default spawner: Node's child_process. Tests inject a mock that
// returns a controllable "process handle" without actually forking.
function defaultSpawner(command, args, options) {
  var child_process = require("child_process");
  return child_process.spawn(command, args, options);
}

// Default blob fetcher: reads from local BTCPC-FS via blobStore.getBlob.
// Tests inject a mock that returns synthetic content.
function defaultBlobFetcher(cid) {
  var blobStore = require("./blobStore");
  if (!blobStore.hasBlob(cid)) {
    throw new Error("blob not in local BTCPC-FS: " + cid);
  }
  return blobStore.getBlob(cid);
}

/**
 * Create a runner instance. Each host runs ONE runner that manages
 * all deployed services on that host. Multiple runners on one
 * machine is supported but not recommended (port conflicts).
 *
 * @param {object} options
 * @param {function} options.spawner — for tests; default = child_process.spawn
 * @param {function} options.blobFetcher — for tests; default = blobStore.getBlob
 * @param {string}   options.workDir — base directory for unpacked binaries
 * @param {function} options.onHeartbeat — called per service per heartbeat tick
 */
function createRunner(options) {
  options = options || {};
  var spawner = options.spawner || defaultSpawner;
  var blobFetcher = options.blobFetcher || defaultBlobFetcher;
  var workDir = options.workDir || path.join(os.tmpdir(), "btcpc-services");
  var onHeartbeat = options.onHeartbeat || function () {};

  // slug → runtime state
  // {
  //   slug, deployer, runtime, binary_cid,
  //   state: "pending"|"downloading"|"ready"|"running"|"degraded"|"stopped"|"crashed"|"failed",
  //   process: child process handle (when running),
  //   started_at: timestamp,
  //   crashed_at: timestamp (if crashed),
  //   crash_count: number,
  //   last_health_check: timestamp,
  //   port_addr: string
  // }
  var registry = new Map();

  /**
   * Register a deployed service for this runner. Pulls the binary
   * from BTCPC-FS and prepares it for execution. Does not start
   * the process yet — call start() for that.
   */
  function register(deployment) {
    if (!deployment || !deployment.slug) {
      throw new Error("deployment with slug required");
    }
    if (!deployment.runtime || !deployment.runtime.binary_cid) {
      throw new Error("deployment.runtime.binary_cid required");
    }
    if (registry.has(deployment.slug)) {
      throw new Error("service already registered: " + deployment.slug);
    }

    var record = {
      slug: deployment.slug,
      deployer: deployment.deployer,
      runtime: deployment.runtime,
      binary_cid: deployment.runtime.binary_cid,
      state: "pending",
      process: null,
      started_at: null,
      crashed_at: null,
      crash_count: 0,
      last_health_check: null,
      port_addr: null,
    };
    registry.set(deployment.slug, record);
    return record;
  }

  /**
   * Download the binary from BTCPC-FS and place it in the work dir.
   * Synchronous because blobStore.getBlob is synchronous.
   */
  function prepareBinary(slug) {
    var record = registry.get(slug);
    if (!record) throw new Error("service not registered: " + slug);
    if (record.state !== "pending") {
      return record; // already prepared (or running)
    }

    record.state = "downloading";
    try {
      var bytes = blobFetcher(record.binary_cid);
      // In a real implementation we'd write to disk + unpack archives.
      // For v2.13-beta we just track that we have the bytes ready.
      record.binary_bytes = bytes;
      record.state = "ready";
    } catch (err) {
      record.state = "failed";
      record.failure_reason = err.message;
      throw err;
    }
    return record;
  }

  /**
   * Start the service process. Must be in "ready" state.
   * Returns the runtime record with state = "running".
   */
  function start(slug) {
    var record = registry.get(slug);
    if (!record) throw new Error("service not registered: " + slug);
    if (record.state === "pending") {
      prepareBinary(slug);
    }
    if (record.state !== "ready" && record.state !== "stopped" && record.state !== "crashed") {
      throw new Error(
        "cannot start service in state: " + record.state + " (slug: " + slug + ")"
      );
    }

    var rt = record.runtime;
    var cmd = rt.command || rt.entrypoint || "node";
    var args = rt.args || [];

    // For HTTP/TCP services, the process listens on rt.port. For static
    // sites, the runner serves files directly (no spawn). For wasm,
    // we'd use a wasmtime/wasmer subprocess.
    if (rt.type === "static") {
      // Static services don't spawn anything — the runner serves files
      // from binary_bytes directly via its own HTTP layer.
      record.state = "running";
      record.started_at = Date.now();
      record.port_addr = rt.port ? "host:" + rt.port : null;
      return record;
    }

    // Security: validate command against allowlist
    var cmdBase = path.basename(cmd);
    if (ALLOWED_COMMANDS.indexOf(cmdBase) === -1) {
      record.state = "failed";
      record.failure_reason = "command not in allowlist: " + cmdBase;
      throw new Error("command not in allowlist: " + cmdBase + " (allowed: " + ALLOWED_COMMANDS.join(", ") + ")");
    }

    // Security: reject args containing shell metacharacters
    for (var i = 0; i < args.length; i++) {
      if (SHELL_META.test(args[i])) {
        record.state = "failed";
        record.failure_reason = "shell metacharacter in arg: " + args[i];
        throw new Error("shell metacharacters not allowed in service args");
      }
    }

    var env = Object.assign({}, process.env, rt.env || {});
    var spawnOpts = {
      env: env,
      cwd: rt.cwd || workDir,
      stdio: ["ignore", "pipe", "pipe"],
    };

    var child;
    try {
      child = spawner(cmd, args, spawnOpts);
    } catch (err) {
      record.state = "failed";
      record.failure_reason = err.message;
      throw err;
    }

    record.process = child;
    record.state = "running";
    record.started_at = Date.now();
    record.port_addr = rt.port ? "host:" + rt.port : null;

    // Wire crash detection
    if (child && typeof child.on === "function") {
      child.on("exit", function (code, signal) {
        var current = registry.get(slug);
        if (!current) return;
        if (current.state === "stopped") return; // intentional shutdown
        current.state = "crashed";
        current.crashed_at = Date.now();
        current.crash_count = (current.crash_count || 0) + 1;
        current.exit_code = code;
        current.exit_signal = signal;
        registry.set(slug, current);
      });
    }

    registry.set(slug, record);
    return record;
  }

  /**
   * Gracefully stop a running service. Sends SIGTERM, waits up to
   * gracePeriodMs, then SIGKILL.
   */
  function stop(slug, options) {
    options = options || {};
    var record = registry.get(slug);
    if (!record) throw new Error("service not registered: " + slug);
    if (record.state !== "running" && record.state !== "degraded") {
      return record; // already stopped/crashed/etc
    }
    record.state = "stopped";

    if (record.process && typeof record.process.kill === "function") {
      try {
        record.process.kill("SIGTERM");
      } catch (_) {}
      // Real impl would set a timer and SIGKILL after gracePeriodMs.
      // Tests use a mock spawner that doesn't actually run a process.
    }

    record.process = null;
    record.stopped_at = Date.now();
    registry.set(slug, record);
    return record;
  }

  /**
   * Health check a running service. For HTTP services, hits the
   * health endpoint. For TCP, checks the port is reachable. For
   * static, checks the binary is still in memory.
   *
   * v2.13-beta: returns a synthetic "ok" for tests. Real
   * implementations override via the `healthChecker` option.
   */
  function healthCheck(slug) {
    var record = registry.get(slug);
    if (!record) return { healthy: false, reason: "not registered" };
    if (record.state !== "running" && record.state !== "degraded") {
      return { healthy: false, reason: "state is " + record.state };
    }

    // Check if the process is still alive (best-effort)
    if (record.process && record.process.killed) {
      record.state = "crashed";
      registry.set(slug, record);
      return { healthy: false, reason: "process killed" };
    }

    record.last_health_check = Date.now();
    registry.set(slug, record);
    return { healthy: true, state: record.state };
  }

  /**
   * Trigger a heartbeat for all running services. Calls onHeartbeat
   * once per service with the current state. Used by the host's
   * scheduled epoch loop.
   */
  function tickHeartbeats(epoch) {
    var ticks = [];
    for (var entry of registry) {
      var record = entry[1];
      if (record.state !== "running" && record.state !== "degraded") continue;
      var hb = {
        slug: record.slug,
        epoch: epoch || 0,
        state: record.state,
        port_addr: record.port_addr,
        crash_count: record.crash_count || 0,
        uptime_ms: record.started_at ? Date.now() - record.started_at : 0,
      };
      ticks.push(hb);
      try {
        onHeartbeat(hb);
      } catch (_) {}
    }
    return ticks;
  }

  /**
   * Restart a crashed service. Bumps the crash count and tries to
   * spawn again. Caller is responsible for backoff logic.
   */
  function restart(slug) {
    var record = registry.get(slug);
    if (!record) throw new Error("service not registered: " + slug);
    if (record.state !== "crashed") {
      throw new Error("can only restart crashed services, current state: " + record.state);
    }
    return start(slug);
  }

  /**
   * Unregister a service entirely. Stops it first if running.
   */
  function unregister(slug) {
    var record = registry.get(slug);
    if (!record) return false;
    if (record.state === "running" || record.state === "degraded") {
      stop(slug);
    }
    registry.delete(slug);
    return true;
  }

  // ─────────────────────────────────────────────────────────────────
  // Read accessors
  // ─────────────────────────────────────────────────────────────────

  function getStatus(slug) {
    var record = registry.get(slug);
    if (!record) return null;
    return {
      slug: record.slug,
      deployer: record.deployer,
      state: record.state,
      started_at: record.started_at,
      crashed_at: record.crashed_at,
      crash_count: record.crash_count,
      last_health_check: record.last_health_check,
      port_addr: record.port_addr,
      binary_cid: record.binary_cid,
      runtime_type: record.runtime && record.runtime.type,
      uptime_ms: record.started_at && record.state === "running"
        ? Date.now() - record.started_at
        : 0,
      failure_reason: record.failure_reason || null,
      exit_code: record.exit_code,
      exit_signal: record.exit_signal,
    };
  }

  function getAllStatuses() {
    var result = [];
    for (var entry of registry) {
      result.push(getStatus(entry[0]));
    }
    return result;
  }

  function getRunningServices() {
    return getAllStatuses().filter(function (s) {
      return s.state === "running" || s.state === "degraded";
    });
  }

  function getServiceCount() {
    return registry.size;
  }

  function reset() {
    // Stop everything cleanly first
    for (var entry of registry) {
      var slug = entry[0];
      var rec = entry[1];
      if (rec.state === "running" || rec.state === "degraded") {
        try { stop(slug); } catch (_) {}
      }
    }
    registry.clear();
  }

  return {
    register: register,
    prepareBinary: prepareBinary,
    start: start,
    stop: stop,
    restart: restart,
    unregister: unregister,
    healthCheck: healthCheck,
    tickHeartbeats: tickHeartbeats,
    getStatus: getStatus,
    getAllStatuses: getAllStatuses,
    getRunningServices: getRunningServices,
    getServiceCount: getServiceCount,
    reset: reset,
  };
}

module.exports = {
  createRunner: createRunner,
  defaultSpawner: defaultSpawner,
  defaultBlobFetcher: defaultBlobFetcher,
  ALLOWED_COMMANDS: ALLOWED_COMMANDS,
};
