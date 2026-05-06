"use strict";

/**
 * BTCPC Rust Chain IPC Client
 * Shin Devlin
 *
 * Synchronous JSON-RPC helper for btcpc-chain over a Unix domain socket.
 * Intended for blockStore integration where the existing callers are sync.
 */

var fs = require("fs");
var childProcess = require("child_process");

var DEFAULT_SOCKET_PATH = "/tmp/btcpc-chain.sock";
var DEFAULT_TIMEOUT_MS = 5000;
var MAX_BUFFER_BYTES = 70 * 1024 * 1024;

var _requestId = 1;
var _preferredTransport = null; // "nc" | "python3"

var PYTHON_BRIDGE = [
  "import socket, sys",
  "sock_path = sys.argv[1]",
  "timeout_s = float(sys.argv[2])",
  "request = sys.stdin.buffer.read()",
  "sock = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)",
  "sock.settimeout(timeout_s)",
  "sock.connect(sock_path)",
  "sock.sendall(request)",
  "chunks = []",
  "while True:",
  "    try:",
  "        chunk = sock.recv(65536)",
  "    except socket.timeout:",
  "        break",
  "    if not chunk:",
  "        break",
  "    chunks.append(chunk)",
  "    if b'\\n' in chunk:",
  "        break",
  "sys.stdout.buffer.write(b''.join(chunks))"
].join("\n");

function isEnabled() {
  return process.env.BTCPC_USE_RUST_CHAIN === "true";
}

function socketPath() {
  return process.env.BTCPC_CHAIN_SOCK || DEFAULT_SOCKET_PATH;
}

function timeoutMs() {
  var raw = Number(process.env.BTCPC_CHAIN_IPC_TIMEOUT_MS || DEFAULT_TIMEOUT_MS);
  if (!Number.isFinite(raw) || raw <= 0) return DEFAULT_TIMEOUT_MS;
  return Math.floor(raw);
}

function isSocketAvailable() {
  return fs.existsSync(socketPath());
}

function _toEpoch(epoch) {
  var n = Number(epoch);
  if (!Number.isInteger(n) || n < 0) {
    throw new Error("[BTCPC Chain IPC] epoch must be a non-negative integer");
  }
  return n;
}

function _missingBinary(err) {
  return !!(err && (err.code === "ENOENT" || /ENOENT/.test(err.message || "")));
}

function _sendWithNc(requestLine) {
  var res = childProcess.spawnSync("nc", ["-U", socketPath()], {
    input: requestLine + "\n",
    encoding: "utf8",
    timeout: timeoutMs(),
    maxBuffer: MAX_BUFFER_BYTES
  });
  return _unwrapTransportResult("nc", res);
}

function _sendWithPython(requestLine) {
  var res = childProcess.spawnSync("python3", ["-c", PYTHON_BRIDGE, socketPath(), String(timeoutMs() / 1000)], {
    input: requestLine + "\n",
    encoding: "utf8",
    timeout: timeoutMs(),
    maxBuffer: MAX_BUFFER_BYTES
  });
  return _unwrapTransportResult("python3", res);
}

function _unwrapTransportResult(transport, result) {
  if (result.error) {
    var transportErr = new Error(
      "[BTCPC Chain IPC] " + transport + " transport failed: " + result.error.message
    );
    transportErr.code = result.error.code;
    throw transportErr;
  }
  if (result.status !== 0) {
    throw new Error(
      "[BTCPC Chain IPC] " + transport + " exited with code " + result.status +
      (result.stderr ? (": " + String(result.stderr).trim()) : "")
    );
  }
  var out = String(result.stdout || "").trim();
  if (!out) {
    throw new Error(
      "[BTCPC Chain IPC] " + transport + " returned an empty response" +
      (result.stderr ? (": " + String(result.stderr).trim()) : "")
    );
  }
  var lines = out.split(/\r?\n/).filter(function (line) { return line.trim().length > 0; });
  return lines[lines.length - 1];
}

function _transportSend(requestLine) {
  var candidates = _preferredTransport ? [_preferredTransport] : ["nc", "python3"];
  var lastError = null;

  for (var i = 0; i < candidates.length; i++) {
    var t = candidates[i];
    try {
      var line = t === "nc" ? _sendWithNc(requestLine) : _sendWithPython(requestLine);
      _preferredTransport = t;
      return line;
    } catch (err) {
      lastError = err;
      if (_preferredTransport === t) _preferredTransport = null;
      if (!_missingBinary(err)) break;
    }
  }

  if (lastError) throw lastError;
  throw new Error("[BTCPC Chain IPC] no transport available");
}

function _parseRpcResponse(line, method) {
  var msg;
  try {
    msg = JSON.parse(line);
  } catch (e) {
    throw new Error(
      "[BTCPC Chain IPC] Invalid JSON response for " + method + ": " +
      line.slice(0, 240)
    );
  }

  if (msg && msg.error) {
    var code = msg.error.code;
    var text = msg.error.message || "unknown error";
    throw new Error("[BTCPC Chain IPC] " + method + " failed (" + code + "): " + text);
  }

  if (!msg || !Object.prototype.hasOwnProperty.call(msg, "result")) {
    throw new Error("[BTCPC Chain IPC] " + method + " returned malformed response");
  }

  return msg.result;
}

function call(method, params) {
  if (!isSocketAvailable()) {
    throw new Error("[BTCPC Chain IPC] socket unavailable: " + socketPath());
  }

  var request = {
    id: _requestId++,
    method: method,
    params: params || {}
  };

  var line = _transportSend(JSON.stringify(request));
  return _parseRpcResponse(line, method);
}

function writeBlock(epoch, dataBuf) {
  if (!Buffer.isBuffer(dataBuf)) throw new Error("[BTCPC Chain IPC] writeBlock expects a Buffer");
  return call("block_write", { epoch: _toEpoch(epoch), data_b64: dataBuf.toString("base64") }) === true;
}

function writeFinality(epoch, dataBuf) {
  if (!Buffer.isBuffer(dataBuf)) throw new Error("[BTCPC Chain IPC] writeFinality expects a Buffer");
  return call("block_write_finality", { epoch: _toEpoch(epoch), data_b64: dataBuf.toString("base64") }) === true;
}

function readBlock(epoch) {
  var b64 = call("block_read", { epoch: _toEpoch(epoch) });
  if (b64 === null || b64 === undefined) return null;
  return Buffer.from(String(b64), "base64");
}

function readFinality(epoch) {
  var b64 = call("block_read_finality", { epoch: _toEpoch(epoch) });
  if (b64 === null || b64 === undefined) return null;
  return Buffer.from(String(b64), "base64");
}

function hasBlock(epoch) {
  return !!call("block_has", { epoch: _toEpoch(epoch) });
}

function hasFinality(epoch) {
  return !!call("block_has_finality", { epoch: _toEpoch(epoch) });
}

function latestBlock() {
  var v = Number(call("block_latest", {}));
  return Number.isFinite(v) ? v : -1;
}

function latestFinality() {
  var v = Number(call("block_latest_finality", {}));
  return Number.isFinite(v) ? v : -1;
}

function pruneBefore(epoch) {
  var v = Number(call("block_prune", { epoch: _toEpoch(epoch) }));
  return Number.isFinite(v) ? v : 0;
}

module.exports = {
  isEnabled: isEnabled,
  isSocketAvailable: isSocketAvailable,
  socketPath: socketPath,
  call: call,
  writeBlock: writeBlock,
  writeFinality: writeFinality,
  readBlock: readBlock,
  readFinality: readFinality,
  hasBlock: hasBlock,
  hasFinality: hasFinality,
  latestBlock: latestBlock,
  latestFinality: latestFinality,
  pruneBefore: pruneBefore
};

