"use strict";

/**
 * BTCPC-FS Blob Store — v2.11.0
 * Shin Devlin
 *
 * Disk-backed content-addressed storage. Files are stored at
 * ~/.btcpc/blobs/<cid[:2]>/<cid[2:4]>/<cid> where cid is the sha256 of
 * the file contents, hex-encoded. The 2-byte prefix sharding keeps
 * directory listings reasonable as the blob count grows.
 */

var fs = require("fs");
var path = require("path");
var os = require("os");
var crypto = require("crypto");

var DEFAULT_ROOT = path.join(os.homedir(), ".btcpc", "blobs");
// BTCPC_BLOB_DIR is the canonical env var; BTCPC_STORAGE_DIR is accepted as
// an alias for backwards compatibility with .env files that used the old name.
var BLOB_ROOT = process.env.BTCPC_BLOB_DIR || process.env.BTCPC_STORAGE_DIR || DEFAULT_ROOT;
var MAX_BLOB_BYTES = parseInt(process.env.BTCPC_MAX_BLOB_BYTES || String(100 * 1024 * 1024), 10);

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
    console.error("[blobStore] putBlob requires a Buffer");
    return null;
  }
  if (buffer.length === 0) {
    console.error("[blobStore] putBlob: blob is empty");
    return null;
  }
  if (buffer.length > MAX_BLOB_BYTES) {
    console.error("[blobStore] putBlob: blob exceeds max size of " + MAX_BLOB_BYTES + " bytes");
    return null;
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
    console.error("[blobStore] getBlob: invalid CID: " + cid);
    return null;
  }
  try {
    var p = blobPath(cid);
    if (!fs.existsSync(p)) {
      return null;
    }
    return fs.readFileSync(p);
  } catch (e) {
    console.error("[blobStore] getBlob error for " + cid + ": " + e.message);
    return null;
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
    console.error("[blobStore] readBlobRange: invalid CID: " + cid);
    return null;
  }
  try {
    var p = blobPath(cid);
    if (!fs.existsSync(p)) {
      console.error("[blobStore] readBlobRange: blob not found: " + cid);
      return null;
    }
    var buf = Buffer.alloc(length);
    var fd = fs.openSync(p, "r");
    try {
      var bytesRead = fs.readSync(fd, buf, 0, length, start);
      return buf.slice(0, bytesRead);
    } finally {
      fs.closeSync(fd);
    }
  } catch (e) {
    console.error("[blobStore] readBlobRange error for " + cid + ": " + e.message);
    return null;
  }
}

function hashBlobRange(cid, start, length) {
  var chunk = readBlobRange(cid, start, length);
  return crypto.createHash("sha256").update(chunk).digest("hex");
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
};
