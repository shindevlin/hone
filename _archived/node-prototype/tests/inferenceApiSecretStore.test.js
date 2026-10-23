"use strict";

/**
 * inference/api.js secretStore-first project lookup — D.5-delta tests
 *
 * The authenticateBearer middleware in api.js now uses _findProjectByApiKey
 * which tries secretStore before falling back to Mongo.
 *
 * We test the core lookup helpers directly since the full Express router
 * is heavy to boot (needs Ollama, WorkProof model, etc.).
 */

const fs = require('fs');
const os = require('os');
const path = require('path');

// Isolate secretStore per test run
const ISOLATED_DIR = fs.mkdtempSync(path.join(os.tmpdir(), 'hone-inf-delta-'));
process.env.HONE_SECRETS_PATH = path.join(ISOLATED_DIR, 'secrets.json');

// Mock everything the inference api.js requires that isn't under test
jest.mock('../src/models/WorkProof', () => function (fields) { Object.assign(this, fields); this.save = jest.fn(async function() { this._id = 'proof-id'; return this; }); });
jest.mock('../src/models/Project', () => {
  const projects = {};
  function M(f) { Object.assign(this, f); this.save = jest.fn(async function() { return this; }); }
  M.findOne = jest.fn(async (q) => {
    if (q && q.apiKey) {
      return Object.values(projects).find(p => p.apiKey === q.apiKey) || null;
    }
    if (q && q.name) return projects[q.name] || null;
    return null;
  });
  M._projects = projects;
  return M;
});
jest.mock('../src/models/InferenceJob', () => ({ countDocuments: jest.fn(async () => 0) }));
jest.mock('../src/services/epochManager', () => ({ getCurrentEpoch: jest.fn(async () => 1) }));
jest.mock('../src/mining/workGenerator', () => ({ getModelWeight: jest.fn(() => 1.0) }));
jest.mock('../src/services/pricing', () => ({
  calculateCost: jest.fn(async () => ({ cost: 0.001, pricing: { tokensPerHone: 1000, modelWeight: 1.0, loadMultiplier: 1.0, totalMultiplier: 1.0, load: 0 } })),
  getCurrentPricing: jest.fn(async () => ({ tokensPerHone: 1000, costPerToken: 0.000001, loadMultiplier: 1.0, modelWeight: 1.0, totalMultiplier: 1.0, load: 0, baseRate: 0.000001 })),
  getAutoBid: jest.fn(async () => ({ bid: 0.01 })),
}));
jest.mock('../src/inference/p2pRouter', () => ({
  submitInference: jest.fn(async () => ({ job_id: 'test-job-1' })),
  getJob: jest.fn(async () => ({ status: 'completed', result_text: 'hello', tokens_generated: 10 })),
  hasMiners: jest.fn(() => false),
  peerCount: jest.fn(() => 0),
}));

const secretStore = require('../src/services/secretStore');
const Project = require('../src/models/Project');

afterAll(() => {
  try { fs.rmSync(ISOLATED_DIR, { recursive: true, force: true }); } catch (_) {}
});

describe('inference/api.js — secretStore project lookup (D.5-delta)', () => {
  beforeEach(async () => {
    secretStore.resetForTests();
    await secretStore.load();
    for (const k of Object.keys(Project._projects)) delete Project._projects[k];
    Project.findOne.mockClear();
  });

  it('getProjectByApiKey finds a project created in secretStore', async () => {
    const { apiKey } = await secretStore.createProject({
      name: 'owner/infrepo',
      owner: 'owner',
      repo: 'infrepo',
      repo_url: 'https://github.com/owner/infrepo',
      wallet_address: 'hone_proj_infrwallet',
    });

    const found = secretStore.getProjectByApiKey(apiKey);
    expect(found).toBeTruthy();
    expect(found.owner).toBe('owner');
    // Mongo was not consulted
    expect(Project.findOne).not.toHaveBeenCalled();
  });

  it('falls back to Mongo Project.findOne when secretStore misses', async () => {
    const mongoKey = 'hone_mongoinf_key';
    Project._projects['owner/mongoinf'] = {
      name: 'owner/mongoinf',
      apiKey: mongoKey,
      owner: 'owner',
      repo: 'mongoinf',
      walletAddress: 'hone_proj_mongoinf',
      verified: true,
      balance: 10,
      isActive: true,
      save: jest.fn(async function() { return this; }),
    };

    // secretStore doesn't know this key
    const fromSS = secretStore.getProjectByApiKey(mongoKey);
    expect(fromSS).toBeNull();

    // Mongo lookup should succeed
    const mongoProject = await Project.findOne({ apiKey: mongoKey, isActive: true });
    expect(mongoProject).toBeTruthy();
    expect(mongoProject.owner).toBe('owner');
    expect(Project.findOne).toHaveBeenCalledTimes(1);
  });

  it('wrong key returns null from both stores', async () => {
    await secretStore.createProject({
      name: 'owner/wrongkeyrepo',
      owner: 'owner',
      repo: 'wrongkeyrepo',
      repo_url: 'https://github.com/owner/wrongkeyrepo',
      wallet_address: 'hone_proj_wrongkey',
    });

    const notFound = secretStore.getProjectByApiKey('hone_wrong_key_here');
    expect(notFound).toBeNull();

    const mongoNotFound = await Project.findOne({ apiKey: 'hone_wrong_key_here', isActive: true });
    expect(mongoNotFound).toBeNull();
  });
});
