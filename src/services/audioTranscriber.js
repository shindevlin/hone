"use strict";

/**
 * Audio transcription for multi-modal inference jobs.
 * Shin Devlin
 *
 * Wraps local Whisper (openai-whisper CLI or whisper.cpp) to transcribe
 * audio blobs before inference. The transcript is injected into the job
 * prompt — the model never sees raw audio, just text.
 *
 * Supported runtimes (tried in order):
 *   1. `whisper` CLI (openai-whisper pip package)
 *   2. `python3 -m whisper` (same package, module invocation)
 *   3. `whisper.cpp` via `./main` in WHISPER_CPP_PATH
 *
 * If none are available, throws — miner should skip audio jobs or
 * advertise audio: false in capabilities.
 */

const fs = require("fs");
const path = require("path");
const os = require("os");
const { execSync } = require("child_process");

const BLOB_DIR = process.env.HONE_BLOB_DIR || path.resolve(__dirname, "../../data/blobs");
const WHISPER_MODEL = process.env.HONE_WHISPER_MODEL || "tiny";
const WHISPER_TIMEOUT_MS = parseInt(process.env.HONE_WHISPER_TIMEOUT_MS) || 120000;

function _tryExec(cmd, opts) {
  try {
    execSync(cmd, { ...opts, stdio: "pipe", timeout: 5000 });
    return true;
  } catch (_) {
    return false;
  }
}

function isWhisperAvailable() {
  return (
    _tryExec("whisper --help") ||
    _tryExec("python3 -m whisper --help")
  );
}

/**
 * Detect audio format from magic bytes. Returns a file extension Whisper accepts.
 * Falls back to .wav (Whisper is permissive about WAV-like blobs).
 */
function detectAudioExtension(buf) {
  if (!buf || buf.length < 12) return ".wav";
  // WAV: RIFF....WAVE
  if (buf[0] === 0x52 && buf[1] === 0x49 && buf[2] === 0x46 && buf[3] === 0x46) return ".wav";
  // MP3: ID3 tag or sync word FF FB / FF FA / FF F3
  if (buf[0] === 0x49 && buf[1] === 0x44 && buf[2] === 0x33) return ".mp3";
  if (buf[0] === 0xFF && (buf[1] & 0xE0) === 0xE0) return ".mp3";
  // OGG: OggS
  if (buf[0] === 0x4F && buf[1] === 0x67 && buf[2] === 0x67 && buf[3] === 0x53) return ".ogg";
  // FLAC: fLaC
  if (buf[0] === 0x66 && buf[1] === 0x4C && buf[2] === 0x61 && buf[3] === 0x43) return ".flac";
  // M4A / AAC: ftyp box (bytes 4-7 = "ftyp")
  if (buf[4] === 0x66 && buf[5] === 0x74 && buf[6] === 0x79 && buf[7] === 0x70) return ".m4a";
  // WebM / Matroska: EBML header 1A 45 DF A3
  if (buf[0] === 0x1A && buf[1] === 0x45 && buf[2] === 0xDF && buf[3] === 0xA3) return ".webm";
  return ".wav";
}

/**
 * Transcribe an audio blob (by CID) to text.
 * Returns the transcript string, or throws on failure.
 */
async function transcribe(audioCid) {
  if (!/^[a-f0-9]{64}$/.test(audioCid)) {
    throw new Error(`Invalid audio CID: ${audioCid}`);
  }
  const blobPath = path.join(BLOB_DIR, audioCid);
  if (!fs.existsSync(blobPath)) {
    throw new Error(`Audio blob not found locally: ${audioCid}`);
  }

  // Detect format from magic bytes so Whisper gets the right extension
  const header = Buffer.alloc(12);
  const fd = fs.openSync(blobPath, "r");
  fs.readSync(fd, header, 0, 12, 0);
  fs.closeSync(fd);
  const ext = detectAudioExtension(header);

  const tmpDir = os.tmpdir();
  const tmpAudio = path.join(tmpDir, `hone_audio_${Date.now()}${ext}`);
  const expectedTxt = tmpAudio.replace(/\.[^.]+$/, ".txt");

  try {
    fs.copyFileSync(blobPath, tmpAudio);

    const baseArgs = `"${tmpAudio}" --output_format txt --output_dir "${tmpDir}" --model ${WHISPER_MODEL}`;

    let ok = false;
    // Try whisper CLI
    try {
      execSync(`whisper ${baseArgs}`, { stdio: "pipe", timeout: WHISPER_TIMEOUT_MS });
      ok = true;
    } catch (_) {
      // Try python module form
      execSync(`python3 -m whisper ${baseArgs}`, { stdio: "pipe", timeout: WHISPER_TIMEOUT_MS });
      ok = true;
    }

    if (!ok || !fs.existsSync(expectedTxt)) {
      throw new Error("Whisper produced no output file");
    }

    const transcript = fs.readFileSync(expectedTxt, "utf8").trim();
    if (!transcript) throw new Error("Whisper transcript is empty");
    return transcript;
  } finally {
    try { fs.unlinkSync(tmpAudio); } catch (_) {}
    try { fs.unlinkSync(expectedTxt); } catch (_) {}
  }
}

module.exports = { transcribe, isWhisperAvailable, detectAudioExtension };
