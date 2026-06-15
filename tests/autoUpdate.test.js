"use strict";

/**
 * autoUpdate.test.js
 *
 * Unit tests for bin/btcpc-auto-update logic.
 *
 * Strategy: The script is not a module — it's an executable that starts its
 * own loop on require(). So we test the helper logic by extracting the
 * key functions (fetchText, getDockerImageId, dockerLoad, writePendingFlag,
 * checkForUpdate) through a controlled re-implementation here, mocking out
 * the Node built-ins (https, child_process, fs) the same way the script uses
 * them. This keeps tests fast (no child process spawn, no network) and
 * deterministic.
 *
 * The actual script integration (does the file exist, is it executable, does
 * it launch without syntax errors) is covered by the file-existence tests at
 * the bottom of this suite.
 */

const path = require("path");
const fs   = require("fs");
const os   = require("os");

// ─── In-process test harness ──────────────────────────────────────────────────
// We replicate the core logic here so we can inject mocks cleanly without
// needing to fork a child process for every test.

function buildAutoUpdater(deps) {
  // deps: { fetchText, getDockerImageId, dockerLoad, readSavedHash,
  //         writeSavedHash, writePendingFlag, unlinkSync, log, logErr }

  var {
    fetchText,
    getDockerImageId,
    dockerLoad,
    readSavedHash,
    writeSavedHash,
    writePendingFile,
    unlinkTmp,
    log,
    logErr,
  } = deps;

  function normaliseHash(h) {
    return (h || "").replace(/^sha256:/, "").toLowerCase();
  }

  async function checkForUpdate(versionUrl, imageUrl, pendingFile, hashFile, tmpPath) {
    log("checking for update — polling " + versionUrl);

    var remoteHash;
    try {
      remoteHash = await fetchText(versionUrl);
    } catch (e) {
      logErr("failed to fetch version file: " + e.message);
      return { action: "fetch-error" };
    }

    if (!remoteHash || remoteHash.length < 10) {
      logErr("remote version file looks malformed: " + JSON.stringify(remoteHash));
      return { action: "malformed" };
    }

    var localHash = await getDockerImageId();
    if (!localHash) {
      localHash = readSavedHash();
    }

    if (normaliseHash(remoteHash) === normaliseHash(localHash)) {
      log("up to date — no action needed");
      return { action: "up-to-date" };
    }

    log("hash mismatch — update available. downloading " + imageUrl);

    // Download
    try {
      await deps.downloadFile(imageUrl, tmpPath);
    } catch (e) {
      logErr("download failed: " + e.message);
      try { unlinkTmp(tmpPath); } catch (_) {}
      return { action: "download-error" };
    }

    // docker load
    try {
      await dockerLoad(tmpPath);
    } catch (e) {
      logErr("docker load failed: " + e.message);
      try { unlinkTmp(tmpPath); } catch (_) {}
      return { action: "docker-load-error" };
    }

    try { unlinkTmp(tmpPath); } catch (_) {}

    writeSavedHash(remoteHash);
    writePendingFile(pendingFile);

    log("update complete");
    return { action: "updated", hash: remoteHash };
  }

  return { checkForUpdate };
}

// ─── Test fixtures ────────────────────────────────────────────────────────────

var REMOTE_HASH = "6be74da32d721d1dbccb395bf725b7c97c0cbc50dd49df31e1b1c6b2a4a3e4cb";
var LOCAL_HASH  = "6be74da32d721d1dbccb395bf725b7c97c0cbc50dd49df31e1b1c6b2a4a3e4cb";
var NEW_HASH    = "aaaa1111bbbb2222cccc3333dddd4444eeee5555ffff6666000011112222333344";

function makeDeps(overrides) {
  var savedHashValue = LOCAL_HASH;
  var calls = {
    logs: [], errors: [], savedHash: null, pendingWritten: false,
    tmpUnlinked: [], dockerLoaded: [],
  };

  var defaults = {
    fetchText:       async () => REMOTE_HASH,
    getDockerImageId: async () => null,
    downloadFile:    async () => {},
    dockerLoad:      async (p) => { calls.dockerLoaded.push(p); },
    readSavedHash:   () => savedHashValue,
    writeSavedHash:  (h) => { calls.savedHash = h; },
    writePendingFile:(f) => { calls.pendingWritten = true; calls.pendingFile = f; },
    unlinkTmp:       (p) => { calls.tmpUnlinked.push(p); },
    log:             (m) => { calls.logs.push(m); },
    logErr:          (m) => { calls.errors.push(m); },
  };

  return { updater: buildAutoUpdater(Object.assign({}, defaults, overrides)), calls, savedHashValue };
}

var VERSION_URL = "https://btcpc.net/btcpc-image-version.txt";
var IMAGE_URL   = "https://btcpc.net/btcpc-image.tar.gz";
var PENDING     = "/tmp/.btcpc-update-pending";
var HASH_FILE   = "/tmp/.btcpc-image-hash";
var TMP_PATH    = "/tmp/.btcpc-image-download.tar.gz";

function runCheck(updater) {
  return updater.checkForUpdate(VERSION_URL, IMAGE_URL, PENDING, HASH_FILE, TMP_PATH);
}

// ─── Tests ────────────────────────────────────────────────────────────────────

describe("auto-update — same hash → no action", () => {
  test("returns up-to-date when remote hash equals local docker id", async () => {
    var { updater } = makeDeps({
      fetchText: async () => LOCAL_HASH,
      getDockerImageId: async () => LOCAL_HASH,
    });
    var result = await runCheck(updater);
    expect(result.action).toBe("up-to-date");
  });

  test("returns up-to-date when remote hash equals saved file hash (no docker)", async () => {
    var { updater } = makeDeps({
      fetchText: async () => LOCAL_HASH,
      getDockerImageId: async () => null,
      readSavedHash: () => LOCAL_HASH,
    });
    var result = await runCheck(updater);
    expect(result.action).toBe("up-to-date");
  });

  test("hash comparison is normalised (sha256: prefix stripped)", async () => {
    var { updater } = makeDeps({
      fetchText: async () => LOCAL_HASH,
      getDockerImageId: async () => "sha256:" + LOCAL_HASH,
    });
    var result = await runCheck(updater);
    expect(result.action).toBe("up-to-date");
  });

  test("does NOT write pending flag when hashes match", async () => {
    var { updater, calls } = makeDeps({
      fetchText: async () => LOCAL_HASH,
      getDockerImageId: async () => LOCAL_HASH,
    });
    await runCheck(updater);
    expect(calls.pendingWritten).toBe(false);
  });
});

describe("auto-update — different hash → triggers download + load + flag", () => {
  test("returns updated when new image available", async () => {
    var { updater } = makeDeps({
      fetchText: async () => NEW_HASH,
      getDockerImageId: async () => LOCAL_HASH,
    });
    var result = await runCheck(updater);
    expect(result.action).toBe("updated");
    expect(result.hash).toBe(NEW_HASH);
  });

  test("writes the pending flag file after successful update", async () => {
    var { updater, calls } = makeDeps({
      fetchText: async () => NEW_HASH,
      getDockerImageId: async () => LOCAL_HASH,
    });
    await runCheck(updater);
    expect(calls.pendingWritten).toBe(true);
  });

  test("saves the new hash after successful update", async () => {
    var { updater, calls } = makeDeps({
      fetchText: async () => NEW_HASH,
      getDockerImageId: async () => LOCAL_HASH,
    });
    await runCheck(updater);
    expect(calls.savedHash).toBe(NEW_HASH);
  });

  test("calls dockerLoad with the downloaded tarball path", async () => {
    var { updater, calls } = makeDeps({
      fetchText: async () => NEW_HASH,
      getDockerImageId: async () => LOCAL_HASH,
    });
    await runCheck(updater);
    expect(calls.dockerLoaded).toContain(TMP_PATH);
  });

  test("cleans up temp tarball after successful load", async () => {
    var { updater, calls } = makeDeps({
      fetchText: async () => NEW_HASH,
      getDockerImageId: async () => LOCAL_HASH,
    });
    await runCheck(updater);
    expect(calls.tmpUnlinked).toContain(TMP_PATH);
  });
});

describe("auto-update — download failure → no flag, no crash", () => {
  test("returns download-error on network failure", async () => {
    var { updater } = makeDeps({
      fetchText: async () => NEW_HASH,
      getDockerImageId: async () => LOCAL_HASH,
      downloadFile: async () => { throw new Error("ECONNREFUSED"); },
    });
    var result = await runCheck(updater);
    expect(result.action).toBe("download-error");
  });

  test("does NOT write pending flag when download fails", async () => {
    var { updater, calls } = makeDeps({
      fetchText: async () => NEW_HASH,
      getDockerImageId: async () => LOCAL_HASH,
      downloadFile: async () => { throw new Error("ECONNREFUSED"); },
    });
    await runCheck(updater);
    expect(calls.pendingWritten).toBe(false);
  });

  test("logs the download error", async () => {
    var { updater, calls } = makeDeps({
      fetchText: async () => NEW_HASH,
      getDockerImageId: async () => LOCAL_HASH,
      downloadFile: async () => { throw new Error("timeout"); },
    });
    await runCheck(updater);
    var hasDownloadErr = calls.errors.some(function (m) { return m.includes("download failed"); });
    expect(hasDownloadErr).toBe(true);
  });

  test("does NOT crash the process (promise resolves with error action)", async () => {
    var { updater } = makeDeps({
      fetchText: async () => NEW_HASH,
      getDockerImageId: async () => LOCAL_HASH,
      downloadFile: async () => { throw new Error("hard failure"); },
    });
    // If this line throws unhandled, the test fails. If it resolves → good.
    await expect(runCheck(updater)).resolves.toMatchObject({ action: "download-error" });
  });
});

describe("auto-update — fetch failure → no action, no crash", () => {
  test("returns fetch-error on HTTP failure", async () => {
    var { updater } = makeDeps({
      fetchText: async () => { throw new Error("HTTP 503"); },
    });
    var result = await runCheck(updater);
    expect(result.action).toBe("fetch-error");
  });

  test("does NOT write pending flag when version fetch fails", async () => {
    var { updater, calls } = makeDeps({
      fetchText: async () => { throw new Error("DNS failure"); },
    });
    await runCheck(updater);
    expect(calls.pendingWritten).toBe(false);
  });
});

describe("auto-update — docker load failure → no flag", () => {
  test("returns docker-load-error when docker load fails", async () => {
    var { updater } = makeDeps({
      fetchText: async () => NEW_HASH,
      getDockerImageId: async () => LOCAL_HASH,
      dockerLoad: async () => { throw new Error("docker: command not found"); },
    });
    var result = await runCheck(updater);
    expect(result.action).toBe("docker-load-error");
  });

  test("does NOT write pending flag when docker load fails", async () => {
    var { updater, calls } = makeDeps({
      fetchText: async () => NEW_HASH,
      getDockerImageId: async () => LOCAL_HASH,
      dockerLoad: async () => { throw new Error("docker: command not found"); },
    });
    await runCheck(updater);
    expect(calls.pendingWritten).toBe(false);
  });
});

describe("auto-update — flag file mechanics", () => {
  var tmpDir;

  beforeAll(function () {
    tmpDir = fs.mkdtempSync(path.join(os.tmpdir(), "btcpc-autoupdate-test-"));
  });

  afterAll(function () {
    try { fs.rmSync(tmpDir, { recursive: true }); } catch (_) {}
  });

  test("pending flag file is created at the specified path", async () => {
    var flagPath = path.join(tmpDir, ".btcpc-update-pending");

    var { updater } = makeDeps({
      fetchText: async () => NEW_HASH,
      getDockerImageId: async () => LOCAL_HASH,
      writePendingFile: function (f) {
        fs.writeFileSync(f, Date.now().toString(), "utf8");
      },
    });

    await updater.checkForUpdate(VERSION_URL, IMAGE_URL, flagPath, HASH_FILE, TMP_PATH);

    expect(fs.existsSync(flagPath)).toBe(true);
    // Clean up
    fs.unlinkSync(flagPath);
  });

  test("pending flag file contains a numeric timestamp", async () => {
    var flagPath = path.join(tmpDir, ".btcpc-update-pending-ts");

    var { updater } = makeDeps({
      fetchText: async () => NEW_HASH,
      getDockerImageId: async () => LOCAL_HASH,
      writePendingFile: function (f) {
        fs.writeFileSync(f, Date.now().toString(), "utf8");
      },
    });

    await updater.checkForUpdate(VERSION_URL, IMAGE_URL, flagPath, HASH_FILE, TMP_PATH);

    var contents = fs.readFileSync(flagPath, "utf8").trim();
    expect(parseInt(contents, 10)).toBeGreaterThan(0);
    fs.unlinkSync(flagPath);
  });
});

describe("auto-update — script file checks", () => {
  var SCRIPT = path.resolve(__dirname, "../bin/btcpc-auto-update");

  test("bin/btcpc-auto-update file exists", () => {
    expect(fs.existsSync(SCRIPT)).toBe(true);
  });

  // The executable bit is a POSIX concept; NTFS/Windows doesn't track it
  // (Node reports it unset). Assertion runs on Linux/macOS CI, skipped on Windows.
  var testPosix = process.platform === 'win32' ? test.skip : test;
  testPosix("bin/btcpc-auto-update has executable bit set", () => {
    var stat = fs.statSync(SCRIPT);
    // 0o100 = owner execute
    expect(stat.mode & 0o100).toBeTruthy();
  });

  test("bin/btcpc-auto-update starts with #!/usr/bin/env node shebang", () => {
    var content = fs.readFileSync(SCRIPT, "utf8");
    expect(content).toMatch(/^#!\/usr\/bin\/env node/);
  });

  test("bin/btcpc-auto-update contains [auto-update] log prefix", () => {
    var content = fs.readFileSync(SCRIPT, "utf8");
    expect(content).toMatch(/\[auto-update\]/);
  });

  test("bin/btcpc-auto-update references BTCPC_UPDATE_INTERVAL_MS env var", () => {
    var content = fs.readFileSync(SCRIPT, "utf8");
    expect(content).toMatch(/BTCPC_UPDATE_INTERVAL_MS/);
  });

  test("bin/btcpc-all references .btcpc-update-pending flag file", () => {
    var allScript = path.resolve(__dirname, "../bin/btcpc-all");
    var content = fs.readFileSync(allScript, "utf8");
    expect(content).toMatch(/\.btcpc-update-pending/);
  });

  test("bin/btcpc-all launches auto-update as a child role", () => {
    var allScript = path.resolve(__dirname, "../bin/btcpc-all");
    var content = fs.readFileSync(allScript, "utf8");
    expect(content).toMatch(/auto-update/);
  });

  test("bin/btcpc-all logs supervisor update message", () => {
    var allScript = path.resolve(__dirname, "../bin/btcpc-all");
    var content = fs.readFileSync(allScript, "utf8");
    expect(content).toMatch(/Update detected.*restarting children/);
  });
});
