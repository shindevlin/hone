"use strict";

/**
 * HONE-FS blob store unit tests — v2.11.0
 * Tests the disk-backed CAS layer in isolation using a temp directory.
 */

const fs = require('fs');
const path = require('path');
const os = require('os');
const crypto = require('crypto');

const TEST_ROOT = path.join(os.tmpdir(), 'hone-test-blobs-' + process.pid);
process.env.HONE_BLOB_DIR = TEST_ROOT;

const blobStore = require('../src/services/blobStore');

describe('blobStore (v2.11.0)', () => {
  beforeEach(() => {
    if (fs.existsSync(TEST_ROOT)) fs.rmSync(TEST_ROOT, { recursive: true, force: true });
  });
  afterAll(() => {
    if (fs.existsSync(TEST_ROOT)) fs.rmSync(TEST_ROOT, { recursive: true, force: true });
  });

  describe('CID computation', () => {
    test('computeCid is deterministic sha256 hex', () => {
      const cid = blobStore.computeCid(Buffer.from('hello world'));
      expect(cid).toBe('b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9');
      expect(blobStore.isValidCid(cid)).toBe(true);
    });
    test('computeCid of same content matches', () => {
      const a = blobStore.computeCid(Buffer.from('identical'));
      const b = blobStore.computeCid(Buffer.from('identical'));
      expect(a).toBe(b);
    });
    test('computeCid rejects non-Buffer', () => {
      expect(() => blobStore.computeCid('string')).toThrow();
    });
    test('isValidCid recognizes format', () => {
      expect(blobStore.isValidCid('b94d27b9934d3e08a52e52d7da7dabfac484efe37a5380ee9088f7ace2efcde9')).toBe(true);
      expect(blobStore.isValidCid('UPPER'.repeat(13))).toBe(false);
      expect(blobStore.isValidCid('short')).toBe(false);
      expect(blobStore.isValidCid(null)).toBe(false);
    });
  });

  describe('putBlob / getBlob', () => {
    test('round-trip works', () => {
      const content = Buffer.from('test content');
      const result = blobStore.putBlob(content);
      expect(result.cid).toBe(blobStore.computeCid(content));
      expect(result.size).toBe(content.length);
      expect(result.existed).toBe(false);
      expect(blobStore.getBlob(result.cid).equals(content)).toBe(true);
    });
    test('binary content preserved', () => {
      const content = crypto.randomBytes(1024);
      const { cid } = blobStore.putBlob(content);
      expect(blobStore.getBlob(cid).equals(content)).toBe(true);
    });
    test('putBlob is idempotent', () => {
      const content = Buffer.from('same');
      const first = blobStore.putBlob(content);
      const second = blobStore.putBlob(content);
      expect(first.cid).toBe(second.cid);
      expect(second.existed).toBe(true);
    });
    test('rejects empty buffer', () => {
      expect(() => blobStore.putBlob(Buffer.alloc(0))).toThrow('empty');
    });
    test('rejects non-Buffer', () => {
      expect(() => blobStore.putBlob('string')).toThrow();
    });
    test('getBlob throws on invalid CID', () => {
      expect(() => blobStore.getBlob('bad')).toThrow('invalid');
    });
    test('getBlob throws on missing CID', () => {
      expect(() => blobStore.getBlob('a'.repeat(64))).toThrow('not found');
    });
  });

  describe('hasBlob / statBlob', () => {
    test('hasBlob false before put', () => {
      expect(blobStore.hasBlob('a'.repeat(64))).toBe(false);
    });
    test('hasBlob true after put', () => {
      const { cid } = blobStore.putBlob(Buffer.from('exists'));
      expect(blobStore.hasBlob(cid)).toBe(true);
    });
    test('statBlob returns size + mtime', () => {
      const content = Buffer.from('stat test');
      const { cid } = blobStore.putBlob(content);
      const stat = blobStore.statBlob(cid);
      expect(stat.size).toBe(content.length);
      expect(typeof stat.mtime).toBe('number');
    });
    test('statBlob null for missing', () => {
      expect(blobStore.statBlob('b'.repeat(64))).toBeNull();
    });
  });

  describe('deleteBlob', () => {
    test('removes file', () => {
      const { cid } = blobStore.putBlob(Buffer.from('delete me'));
      expect(blobStore.deleteBlob(cid)).toBe(true);
      expect(blobStore.hasBlob(cid)).toBe(false);
    });
    test('false for missing', () => {
      expect(blobStore.deleteBlob('c'.repeat(64))).toBe(false);
    });
  });

  describe('listBlobs / totalBytesStored', () => {
    test('empty initially', () => {
      expect(blobStore.listBlobs()).toEqual([]);
      expect(blobStore.totalBytesStored()).toBe(0);
    });
    test('lists all with metadata', () => {
      blobStore.putBlob(Buffer.from('alpha'));
      blobStore.putBlob(Buffer.from('beta gamma'));
      blobStore.putBlob(Buffer.from('delta epsilon zeta'));
      const list = blobStore.listBlobs();
      expect(list.length).toBe(3);
      const total = list.reduce((s, b) => s + b.size, 0);
      expect(blobStore.totalBytesStored()).toBe(total);
    });
  });

  describe('byte range operations', () => {
    test('readBlobRange returns correct slice', () => {
      const content = Buffer.from('0123456789abcdef');
      const { cid } = blobStore.putBlob(content);
      expect(blobStore.readBlobRange(cid, 4, 6).toString()).toBe('456789');
    });
    test('hashBlobRange matches manual hash', () => {
      const content = Buffer.from('the quick brown fox');
      const { cid } = blobStore.putBlob(content);
      const rangeHash = blobStore.hashBlobRange(cid, 4, 5);
      const manual = crypto.createHash('sha256').update('quick').digest('hex');
      expect(rangeHash).toBe(manual);
    });
    test('readBlobRange throws on missing', () => {
      expect(() => blobStore.readBlobRange('d'.repeat(64), 0, 10)).toThrow('not found');
    });
  });

  describe('path sharding', () => {
    test('blobPath uses 2-level hex prefix', () => {
      const content = Buffer.from('sharding');
      const cid = blobStore.computeCid(content);
      const expected = path.join(TEST_ROOT, cid.slice(0, 2), cid.slice(2, 4), cid);
      expect(blobStore.blobPath(cid)).toBe(expected);
    });
    test('blobPath throws on bad CID', () => {
      expect(() => blobStore.blobPath('bad')).toThrow();
    });
  });
});
