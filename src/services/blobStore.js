"use strict";

/**
 * HONE-FS Blob Store — v2.11.0
 * Shin Devlin
 *
 * Disk-backed content-addressed storage. Files are stored at
 * ~/.hone/blobs/<cid[:2]>/<cid[2:4]>/<cid> where cid is the sha256 of
 * the file contents, hex-encoded. The 2-byte prefix sharding keeps
 * directory listings reasonable as the blob count grows.
 */

var fs = require("fs");
var path = require("path");
var os = require("os");
var crypto = require("crypto");

var DEFAULT_ROOT = path.join(os.homedir(), ".hone", "blobs");
// HONE_BLOB_DIR is the canonical env var; HONE_STORAGE_DIR is accepted as
// an alias for backwards compatibility with .env files that used the old name.
var BLOB_ROOT = process.env.HONE_BLOB_DIR || process.env.HONE_STORAGE_DIR || DEFAULT_ROOT;
var MAX_BLOB_BYTES = parseInt(process.env.HONE_MAX_BLOB_BYTES || String(100 * 1024 * 1024), 10);

var CID_PATTERN = /^[a-f0-9]{64}$/;

function isValidCid(cid) {
  return typeof cid === "string" && CID_PATTERN.test(cid);
}

function computeCid(buffer) {
  if (!Buffer.isBuffer(buffer)) {
    throw new Error("computeCid requires a Buffer");
  }
  return crypto.createHash("sha256").update(buffer).digest("hex");
}

function blobPath(cid) {
  if (!isValidCid(cid)) {
    throw new Error("invalid CID: " + cid);
  }
  return path.join(BLOB_ROOT, cid.slice(0, 2), cid.slice(2, 4), cid);
}

function ensureDirForCid(cid) {
  var dir = path.dirname(blobPath(cid));
  if (!fs.existsSync(dir)) {
    fs.mkdirSync(dir, { recursive: true, mode: 0o755 });
  }
}

function putBlob(buffer) {
  if (!Buffer.isBuffer(buffer)) {
    throw new Error("[blobStore] putBlob requires a Buffer");
  }
  if (buffer.length === 0) {
    throw new Error("[blobStore] putBlob: blob is empty");
  }
  if (buffer.length > MAX_BLOB_BYTES) {
    throw new Error("[blobStore] putBlob: blob exceeds max size of " + MAX_BLOB_BYTES + " bytes");
  }

  try {
    var cid = computeCid(buffer);
    var finalPath = blobPath(cid);

    if (fs.existsSync(finalPath)) {
      return { cid: cid, size: buffer.length, existed: true };
    }

    var dir = path.dirname(finalPath);
    fs.mkdirSync(dir, { recursive: true, mode: 0o755 });
    var tmpPath = finalPath + ".tmp." + process.pid + "." + Date.now();
    fs.writeFileSync(tmpPath, buffer, { mode: 0o644 });
    fs.renameSync(tmpPath, finalPath);

    return { cid: cid, size: buffer.length, existed: false };
  } catch (e) {
    console.error("[blobStore] putBlob error: " + e.message);
    return null;
  }
}

function getBlob(cid) {
  if (!isValidCid(cid)) {
    throw new Error("[blobStore] getBlob: invalid CID: " + cid);
  }
  try {
    var p = blobPath(cid);
    if (!fs.existsSync(p)) {
      throw new Error("[blobStore] getBlob: blob not found: " + cid);
    }
    return fs.readFileSync(p);
  } catch (e) {
    if (e.message.includes("not found") || e.message.includes("invalid")) throw e;
    console.error("[blobStore] getBlob error for " + cid + ": " + e.message);
    throw e;
  }
}

function hasBlob(cid) {
  if (!isValidCid(cid)) return false;
  return fs.existsSync(blobPath(cid));
}

function statBlob(cid) {
  if (!isValidCid(cid)) return null;
  var p = blobPath(cid);
  if (!fs.existsSync(p)) return null;
  var s = fs.statSync(p);
  return { size: s.size, mtime: s.mtimeMs };
}

function deleteBlob(cid) {
  if (!isValidCid(cid)) return false;
  try {
    var p = blobPath(cid);
    if (!fs.existsSync(p)) return false;
    fs.unlinkSync(p);
    return true;
  } catch (e) {
    console.error("[blobStore] deleteBlob error for " + cid + ": " + e.message);
    return false;
  }
}

function listBlobs() {
  var result = [];
  if (!fs.existsSync(BLOB_ROOT)) return result;

  var level1 = fs.readdirSync(BLOB_ROOT);
  for (var i = 0; i < level1.length; i++) {
    var l1 = level1[i];
    if (!/^[a-f0-9]{2}$/.test(l1)) continue;
    var l1Path = path.join(BLOB_ROOT, l1);
    if (!fs.statSync(l1Path).isDirectory()) continue;

    var level2 = fs.readdirSync(l1Path);
    for (var j = 0; j < level2.length; j++) {
      var l2 = level2[j];
      if (!/^[a-f0-9]{2}$/.test(l2)) continue;
      var l2Path = path.join(l1Path, l2);
      if (!fs.statSync(l2Path).isDirectory()) continue;

      var files = fs.readdirSync(l2Path);
      for (var k = 0; k < files.length; k++) {
        var cid = files[k];
        if (!isValidCid(cid)) continue;
        var filePath = path.join(l2Path, cid);
        var st = fs.statSync(filePath);
        result.push({ cid: cid, size: st.size, mtime: st.mtimeMs });
      }
    }
  }
  return result;
}

function totalBytesStored() {
  var total = 0;
  var blobs = listBlobs();
  for (var i = 0; i < blobs.length; i++) {
    total += blobs[i].size;
  }
  return total;
}

function readBlobRange(cid, start, length) {
  if (!isValidCid(cid)) {
    throw new Error("[blobStore] readBlobRange: invalid CID: " + cid);
  }
  var p = blobPath(cid);
  if (!fs.existsSync(p)) {
    throw new Error("[blobStore] readBlobRange: blob not found: " + cid);
  }
  try {
    var buf = Buffer.alloc(length);
    var fd = fs.openSync(p, "r");
    try {
      var bytesRead = fs.readSync(fd, buf, 0, length, start);
      return buf.slice(0, bytesRead);
    } finally {
      fs.closeSync(fd);
    }
  } catch (e) {
    if (e.message.includes("not found") || e.message.includes("invalid")) throw e;
    console.error("[blobStore] readBlobRange error for " + cid + ": " + e.message);
    throw e;
  }
}

function hashBlobRange(cid, start, length) {
  var chunk = readBlobRange(cid, start, length);
  return crypto.createHash("sha256").update(chunk).digest("hex");
}

// Streaming upload — writes body stream directly to disk, no memory limit.
// Returns Promise<{cid, size, existed}> or rejects on error.
function putBlobStream(readableStream) {
  return new Promise(function(resolve, reject) {
    var tmpPath = path.join(BLOB_ROOT, '.tmp.' + process.pid + '.' + Date.now());
    fs.mkdirSync(BLOB_ROOT, { recursive: true, mode: 0o755 });
    var out = fs.createWriteStream(tmpPath, { mode: 0o644 });
    var hash = crypto.createHash('sha256');
    var size = 0;

    readableStream.on('data', function(chunk) {
      size += chunk.length;
      hash.update(chunk);
      out.write(chunk);
    });

    readableStream.on('end', function() {
      out.end(function() {
        var cid = hash.digest('hex');
        var finalPath = blobPath(cid);
        if (fs.existsSync(finalPath)) {
          fs.unlinkSync(tmpPath);
          return resolve({ cid: cid, size: size, existed: true });
        }
        var dir = path.dirname(finalPath);
        fs.mkdirSync(dir, { recursive: true, mode: 0o755 });
        fs.renameSync(tmpPath, finalPath);
        resolve({ cid: cid, size: size, existed: false });
      });
    });

    readableStream.on('error', function(err) {
      out.destroy();
      try { fs.unlinkSync(tmpPath); } catch(_) {}
      reject(err);
    });
  });
}

// Split buffer into chunkSize pieces, store each as a blob.
// Returns { chunk_size, total_size, chunks: [{cid, size, existed}, ...] }
function putBlobChunked(buffer, chunkSize) {
  chunkSize = chunkSize || (16 * 1024 * 1024);
  var chunks = [];
  var offset = 0;
  while (offset < buffer.length) {
    var end = Math.min(offset + chunkSize, buffer.length);
    var chunk = buffer.slice(offset, end);
    var result = putBlob(chunk);
    if (!result) throw new Error('putBlob failed at offset ' + offset);
    chunks.push({ cid: result.cid, size: chunk.length, existed: result.existed });
    offset = end;
  }
  return { chunk_size: chunkSize, total_size: buffer.length, chunks: chunks };
}

// Stream a readable into 16MB chunks stored as separate blobs.
// Returns Promise<{ chunk_size, total_size, chunks: [{cid, size, existed}] }>
function putBlobStreamChunked(readableStream, chunkSize) {
  chunkSize = chunkSize || (16 * 1024 * 1024);
  return new Promise(function(resolve, reject) {
    var parts = [];
    readableStream.on('data', function(c) { parts.push(c); });
    readableStream.on('end', function() {
      try { resolve(putBlobChunked(Buffer.concat(parts), chunkSize)); }
      catch(e) { reject(e); }
    });
    readableStream.on('error', reject);
  });
}

// Create a Readable that yields chunks in order — for serving chunked files.
function createChunkedReadStream(chunkCids) {
  var Readable = require('stream').Readable;
  var stream = new Readable({ read: function() {} });
  (function push(i) {
    if (i >= chunkCids.length) { stream.push(null); return; }
    var buf = getBlob(chunkCids[i]);
    if (!buf) { stream.destroy(new Error('chunk not found: ' + chunkCids[i])); return; }
    stream.push(buf);
    setImmediate(function() { push(i + 1); });
  })(0);
  return stream;
}

// Read a byte range across multiple chunks. Returns Buffer or null.
function readChunkedRange(chunks, start, length) {
  // chunks: [{cid, size}, ...]
  var parts = [];
  var pos = 0;
  var remaining = length;
  for (var i = 0; i < chunks.length && remaining > 0; i++) {
    var chunkEnd = pos + chunks[i].size;
    if (chunkEnd <= start) { pos = chunkEnd; continue; }
    var chunkStart = Math.max(0, start - pos);
    var chunkLen = Math.min(chunks[i].size - chunkStart, remaining);
    var buf = readBlobRange(chunks[i].cid, chunkStart, chunkLen);
    if (!buf) return null;
    parts.push(buf);
    remaining -= chunkLen;
    pos = chunkEnd;
  }
  return parts.length ? Buffer.concat(parts) : Buffer.alloc(0);
}

// Returns true if every chunk CID is present locally.
function hasAllChunks(chunkManifest) {
  return chunkManifest.chunks.every(function(c) { return hasBlob(c.cid); });
}

module.exports = {
  BLOB_ROOT: BLOB_ROOT,
  MAX_BLOB_BYTES: MAX_BLOB_BYTES,
  CID_PATTERN: CID_PATTERN,
  isValidCid: isValidCid,
  computeCid: computeCid,
  blobPath: blobPath,
  putBlob: putBlob,
  getBlob: getBlob,
  hasBlob: hasBlob,
  statBlob: statBlob,
  deleteBlob: deleteBlob,
  listBlobs: listBlobs,
  totalBytesStored: totalBytesStored,
  readBlobRange: readBlobRange,
  hashBlobRange: hashBlobRange,
  putBlobStream: putBlobStream,
  putBlobChunked: putBlobChunked,
  putBlobStreamChunked: putBlobStreamChunked,
  createChunkedReadStream: createChunkedReadStream,
  readChunkedRange: readChunkedRange,
  hasAllChunks: hasAllChunks,
};
