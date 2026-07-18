"use strict";

/**
 * modelHealer.test.js — self-heal model verification tests
 *
 * Each test verifies the return value is a working model name (or null),
 * not just that an error was caught. Mocks axios and verifyModel so no
 * real Ollama or registry calls are made.
 */

jest.mock('axios');
jest.mock('../src/services/modelVerifier');

const axios = require('axios');
const { verifyModel } = require('../src/services/modelVerifier');
const { resolveWorkingModel, resetCache } = require('../src/mining/modelHealer');

// ─── helpers ──────────────────────────────────────────────────────────────────

function makeTagsResponse(modelNames) {
  return {
    data: {
      models: modelNames.map(name => ({
        name,
        digest: 'sha256:' + 'a'.repeat(64),
      })),
    },
  };
}

function makeVerified(name) {
  return { verified: true, localHash: 'sha256:' + 'a'.repeat(64), reason: 'Hash matches', model: name };
}

function makeUnverified(name, reason = 'Model not found locally') {
  return { verified: false, localHash: null, reason, model: name };
}

// ─── setup ────────────────────────────────────────────────────────────────────

beforeEach(() => {
  // Reset module cache between tests
  resetCache();
  jest.clearAllMocks();

  // Default: pull always succeeds (tests override per-case)
  axios.post.mockResolvedValue({ data: {} });

  // Default: /api/tags returns empty list
  axios.get.mockResolvedValue(makeTagsResponse([]));
});

// ─── Test group 1: preferred model verifies immediately ───────────────────────

describe('preferred model verifies immediately', () => {
  beforeEach(() => {
    verifyModel.mockResolvedValue(makeVerified('qwen3:4b'));
  });

  test('returns the preferred model name', async () => {
    const result = await resolveWorkingModel('qwen3:4b');
    expect(result).toBe('qwen3:4b');
  });

  test('does not call ollama pull', async () => {
    await resolveWorkingModel('qwen3:4b');
    // axios.post is used for both /api/show (inside verifyModel — mocked) and
    // /api/pull. Since verifyModel is fully mocked, the only axios.post calls
    // would be pull calls. With immediate success there should be none.
    const pullCalls = axios.post.mock.calls.filter(args => {
      const url = args[0] || '';
      return typeof url === 'string' && url.includes('/api/pull');
    });
    expect(pullCalls.length).toBe(0);
  });

  test('second call returns cached result without re-running resolution', async () => {
    const first = await resolveWorkingModel('qwen3:4b');
    const second = await resolveWorkingModel('qwen3:4b');
    expect(second).toBe(first);
    // verifyModel should only have been called once (first resolution)
    expect(verifyModel).toHaveBeenCalledTimes(1);
  });
});

// ─── Test group 2: preferred fails → pull succeeds → returns after retry ──────

describe('preferred fails → pull succeeds → returns after retry', () => {
  test('returns preferred model name after successful pull', async () => {
    // First call to verifyModel (pre-pull) fails; second (post-pull) succeeds
    verifyModel
      .mockResolvedValueOnce(makeUnverified('llama3:8b'))
      .mockResolvedValueOnce(makeVerified('llama3:8b'));

    // /api/tags returns empty (no other locals to fall through to)
    axios.get.mockResolvedValue(makeTagsResponse([]));

    // pull succeeds
    axios.post.mockResolvedValue({ data: {} });

    const result = await resolveWorkingModel('llama3:8b');
    expect(result).toBe('llama3:8b');

    // Confirm a pull was attempted
    const pullCalls = axios.post.mock.calls.filter(args =>
      typeof args[0] === 'string' && args[0].includes('/api/pull')
    );
    expect(pullCalls.length).toBeGreaterThan(0);
  });
});

// ─── Test group 3: preferred fails + pull fails → iterates locals → returns first verifiable

describe('preferred fails + pull fails → iterates locals → returns first verifiable', () => {
  test('returns the first local model that verifies', async () => {
    // preferred model never verifies
    verifyModel.mockImplementation(async (model) => {
      if (model === 'bad-model:7b') return makeUnverified(model);
      if (model === 'local-a:3b') return makeUnverified(model);
      if (model === 'local-b:1b') return makeVerified(model); // this one works
      return makeUnverified(model);
    });

    // pull always fails
    axios.post.mockImplementation(async (url) => {
      if (typeof url === 'string' && url.includes('/api/pull')) {
        throw new Error('pull failed');
      }
      // /api/show for param count
      return { data: { model_info: { 'general.parameter_count': 1000000000 } } };
    });

    // local Ollama has two models
    axios.get.mockResolvedValue(makeTagsResponse(['local-a:3b', 'local-b:1b']));

    const result = await resolveWorkingModel('bad-model:7b');
    expect(result).toBe('local-b:1b');
  });
});

// ─── Test group 4: no local models verify → pulls qwen3:4b → returns it ──────

describe('no local models verify → pulls qwen3:4b → returns it', () => {
  test('returns qwen3:4b after pulling it', async () => {
    verifyModel.mockImplementation(async (model) => {
      if (model === 'qwen3:4b') return makeVerified('qwen3:4b');
      return makeUnverified(model);
    });

    // pull succeeds
    axios.post.mockResolvedValue({ data: {} });

    // local Ollama has one model that doesn't verify; qwen3:4b is not local yet
    axios.get.mockResolvedValue(makeTagsResponse(['bad:1b']));

    const result = await resolveWorkingModel('bad:1b');
    expect(result).toBe('qwen3:4b');
  });
});

// ─── Test group 5: all fallbacks fail → returns null ──────────────────────────

describe('all fallbacks fail → returns null', () => {
  test('returns null when every model fails verification', async () => {
    verifyModel.mockResolvedValue(makeUnverified('any'));

    // All pulls fail
    axios.post.mockImplementation(async (url) => {
      if (typeof url === 'string' && url.includes('/api/pull')) {
        throw new Error('no internet');
      }
      return { data: {} };
    });

    // /api/tags returns empty list (no local models)
    axios.get.mockResolvedValue(makeTagsResponse([]));

    const result = await resolveWorkingModel('bad-model:7b');
    expect(result).toBeNull();
  });
});

// ─── Test group 6: HONE_MODEL unset → picks largest local model ──────────────

describe('HONE_MODEL unset → auto-picks largest verified local model', () => {
  test('returns the largest verified local model by param count', async () => {
    // Two local models: small-a (1B params) and large-b (27B params)
    axios.get.mockResolvedValue(makeTagsResponse(['small-a:1b', 'large-b:27b']));

    // /api/show returns param counts
    axios.post.mockImplementation(async (url, body) => {
      if (typeof url === 'string' && url.includes('/api/show')) {
        const paramMap = {
          'small-a:1b': 1000000000,
          'large-b:27b': 27000000000,
        };
        const name = body?.name;
        return { data: { model_info: { 'general.parameter_count': paramMap[name] || 0 } } };
      }
      return { data: {} };
    });

    // Both verify, but large-b should be picked first
    verifyModel.mockImplementation(async (model) => makeVerified(model));

    const result = await resolveWorkingModel(null);
    expect(result).toBe('large-b:27b');
  });

  test('falls through to pull fallback if no locals verify', async () => {
    axios.get.mockResolvedValue(makeTagsResponse(['bad:3b']));
    axios.post.mockImplementation(async (url, body) => {
      if (typeof url === 'string' && url.includes('/api/show')) {
        return { data: { model_info: { 'general.parameter_count': 3000000000 } } };
      }
      // pull succeeds
      return { data: {} };
    });

    verifyModel.mockImplementation(async (model) => {
      if (model === 'qwen3:4b') return makeVerified('qwen3:4b');
      return makeUnverified(model);
    });

    const result = await resolveWorkingModel(null);
    expect(result).toBe('qwen3:4b');
  });
});

// ─── Test group 7: caching ─────────────────────────────────────────────────────

describe('resolution is cached', () => {
  test('second call with same model returns immediately (verifyModel called once)', async () => {
    verifyModel.mockResolvedValue(makeVerified('qwen3:4b'));

    const first = await resolveWorkingModel('qwen3:4b');
    const second = await resolveWorkingModel('qwen3:4b');

    expect(first).toBe('qwen3:4b');
    expect(second).toBe('qwen3:4b');
    expect(verifyModel).toHaveBeenCalledTimes(1);
  });

  test('forceRefresh=true bypasses cache and re-runs resolution', async () => {
    verifyModel.mockResolvedValue(makeVerified('qwen3:4b'));

    await resolveWorkingModel('qwen3:4b');
    await resolveWorkingModel('qwen3:4b', true); // force refresh

    // Should have been called twice — once per resolution
    expect(verifyModel).toHaveBeenCalledTimes(2);
  });

  test('null result is also cached (avoids hammering Ollama when everything is broken)', async () => {
    verifyModel.mockResolvedValue(makeUnverified('broken:7b'));
    axios.post.mockImplementation(async (url) => {
      if (typeof url === 'string' && url.includes('/api/pull')) throw new Error('no internet');
      return { data: {} };
    });
    axios.get.mockResolvedValue(makeTagsResponse([]));

    const first = await resolveWorkingModel('broken:7b');
    const callsAfterFirst = verifyModel.mock.calls.length;

    const second = await resolveWorkingModel('broken:7b');

    expect(first).toBeNull();
    expect(second).toBeNull();
    // No additional verifyModel calls after the cache was populated
    expect(verifyModel.mock.calls.length).toBe(callsAfterFirst);
  });
});
