"use strict";

/**
 * audioTranscriber.test.js
 *
 * Unit tests for src/services/audioTranscriber.js
 * Tests transcribe() and isWhisperAvailable() using mocked child_process and fs.
 *
 * Strategy:
 *  - jest.mock("child_process") at module scope → execSync is always a jest.fn
 *  - fs methods are spied on per-test via jest.spyOn
 *  - The module under test is required once at the top; no resetModules() needed
 *    because the jest.mock factory persists and the module-level env var is fixed.
 */

const path = require("path");
const os = require("os");

// Must be set before the module is first required.
const TEST_BLOB_DIR = path.join(os.tmpdir(), "hone-test-audio-blobs-" + process.pid);
process.env.HONE_BLOB_DIR = TEST_BLOB_DIR;
process.env.HONE_WHISPER_TIMEOUT_MS = "120000";

// Mock child_process BEFORE requiring the module under test.
jest.mock("child_process", () => ({
  execSync: jest.fn(),
}));

// Now require — module picks up the mocked child_process.
const childProcess = require("child_process");
const fs = require("fs");
const { transcribe, isWhisperAvailable } = require("../src/services/audioTranscriber");

// A valid 64-char lowercase hex CID
const VALID_CID = "a".repeat(64);

// ─────────────────────────────────────────────────────────────────────────────
// isWhisperAvailable()
// ─────────────────────────────────────────────────────────────────────────────

describe("audioTranscriber — isWhisperAvailable()", () => {
  beforeEach(() => {
    childProcess.execSync.mockReset();
  });

  test("returns true when whisper CLI is available", () => {
    childProcess.execSync.mockReturnValue(Buffer.from("usage: whisper"));
    const result = isWhisperAvailable();
    expect(typeof result).toBe("boolean");
    expect(result).toBe(true);
  });

  test("returns true when python3 -m whisper is available (whisper CLI fails)", () => {
    childProcess.execSync.mockImplementation((cmd) => {
      if (!cmd.includes("python3")) throw new Error("whisper: not found");
      return Buffer.from("usage: python3 whisper");
    });
    const result = isWhisperAvailable();
    expect(typeof result).toBe("boolean");
    expect(result).toBe(true);
  });

  test("returns false when neither whisper nor python3 -m whisper is available", () => {
    childProcess.execSync.mockImplementation(() => { throw new Error("command not found"); });
    const result = isWhisperAvailable();
    expect(typeof result).toBe("boolean");
    expect(result).toBe(false);
  });

  test("does not throw even when both CLIs fail", () => {
    childProcess.execSync.mockImplementation(() => { throw new Error("not found"); });
    expect(() => isWhisperAvailable()).not.toThrow();
  });
});

// ─────────────────────────────────────────────────────────────────────────────
// transcribe()
// ─────────────────────────────────────────────────────────────────────────────

describe("audioTranscriber — transcribe()", () => {
  let existsSyncSpy;
  let copyFileSyncSpy;
  let readFileSyncSpy;
  let unlinkSyncSpy;

  beforeEach(() => {
    childProcess.execSync.mockReset();
    existsSyncSpy   = jest.spyOn(fs, "existsSync");
    copyFileSyncSpy = jest.spyOn(fs, "copyFileSync").mockReturnValue(undefined);
    readFileSyncSpy = jest.spyOn(fs, "readFileSync");
    unlinkSyncSpy   = jest.spyOn(fs, "unlinkSync").mockReturnValue(undefined);
    // Mock low-level fd ops used for magic-byte detection
    jest.spyOn(fs, "openSync").mockReturnValue(42);
    jest.spyOn(fs, "readSync").mockReturnValue(12);
    jest.spyOn(fs, "closeSync").mockReturnValue(undefined);
  });

  afterEach(() => {
    jest.restoreAllMocks();
  });

  test("throws on invalid CID — too short", async () => {
    await expect(transcribe("short")).rejects.toThrow("Invalid audio CID");
  });

  test("throws on invalid CID — uppercase hex (64 chars)", async () => {
    await expect(transcribe("A".repeat(64))).rejects.toThrow("Invalid audio CID");
  });

  test("throws on invalid CID — 63 lowercase hex chars", async () => {
    await expect(transcribe("a".repeat(63))).rejects.toThrow("Invalid audio CID");
  });

  test("throws when blob file does not exist", async () => {
    existsSyncSpy.mockReturnValue(false);
    await expect(transcribe(VALID_CID)).rejects.toThrow("Audio blob not found locally");
  });

  test("calls whisper CLI and returns transcript on success", async () => {
    const expectedTranscript = "Hello from the test audio file.";

    existsSyncSpy.mockImplementation(() => true);
    childProcess.execSync.mockReturnValue(Buffer.from(""));
    readFileSyncSpy.mockImplementation((p, enc) => {
      if (p.endsWith(".txt")) return expectedTranscript;
      throw new Error("unexpected readFileSync: " + p);
    });

    const result = await transcribe(VALID_CID);

    expect(result).toBe(expectedTranscript);

    // Verify whisper was invoked with a .wav file argument
    const calls = childProcess.execSync.mock.calls.map((c) => c[0]);
    const whisperCall = calls.find((c) => c.includes("whisper") && c.includes(".wav"));
    expect(whisperCall).toBeDefined();
  });

  test("falls back to python3 -m whisper when whisper CLI fails", async () => {
    const expectedTranscript = "Fallback transcript via python.";

    existsSyncSpy.mockImplementation(() => true);
    childProcess.execSync.mockImplementation((cmd) => {
      // First call is plain "whisper <args>" — fail it
      if (!cmd.startsWith("python3")) throw new Error("whisper: command not found");
      // Second call is "python3 -m whisper <args>" — succeed
      return Buffer.from("");
    });
    readFileSyncSpy.mockImplementation((p, enc) => {
      if (p.endsWith(".txt")) return expectedTranscript;
      throw new Error("unexpected readFileSync: " + p);
    });

    const result = await transcribe(VALID_CID);

    expect(result).toBe(expectedTranscript);

    // python3 -m whisper must have been called
    const calls = childProcess.execSync.mock.calls.map((c) => c[0]);
    const pythonCall = calls.find((c) => c.startsWith("python3 -m whisper"));
    expect(pythonCall).toBeDefined();
  });

  test("cleans up temp files (tmpAudio and expectedTxt) even on whisper error", async () => {
    existsSyncSpy.mockImplementation((p) => {
      // blob exists, but .txt output never appears
      if (p.endsWith(".txt")) return false;
      return true;
    });
    childProcess.execSync.mockImplementation(() => { throw new Error("whisper failed"); });

    await expect(transcribe(VALID_CID)).rejects.toThrow();

    // The finally block should have attempted to unlink the .wav temp file
    expect(unlinkSyncSpy).toHaveBeenCalled();
    const unlinked = unlinkSyncSpy.mock.calls.map((c) => c[0]);
    const wavCleanup = unlinked.find((p) => p.endsWith(".wav"));
    expect(wavCleanup).toBeDefined();
  });

  test("cleans up temp files even when transcript is empty (throws 'empty')", async () => {
    existsSyncSpy.mockImplementation(() => true);
    childProcess.execSync.mockReturnValue(Buffer.from(""));
    // Whitespace-only — trims to "" → source throws "Whisper transcript is empty"
    readFileSyncSpy.mockImplementation((p, enc) => {
      if (p.endsWith(".txt")) return "   ";
      throw new Error("unexpected readFileSync: " + p);
    });

    await expect(transcribe(VALID_CID)).rejects.toThrow("empty");

    // Cleanup still ran
    expect(unlinkSyncSpy).toHaveBeenCalled();
  });

  test("throws when both whisper CLI and python3 -m whisper fail", async () => {
    existsSyncSpy.mockImplementation(() => true);
    childProcess.execSync.mockImplementation(() => { throw new Error("not found"); });

    await expect(transcribe(VALID_CID)).rejects.toThrow();
  });
});
