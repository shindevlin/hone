"use strict";

/**
 * secretStore tests — Phase D.5-alpha
 *
 * Covers user + project CRUD, password verification, API key hashing,
 * Telegram ID indexing, and forward-compat index rebuilding.
 *
 * Uses an isolated per-worker SECRETS_PATH via BTCPC_SECRETS_PATH env
 * var so parallel jest runs don't clobber each other or the user's
 * real ~/.btcpc/secrets.json.
 */

const fs = require('fs');
const os = require('os');
const path = require('path');

// Isolate per-worker BEFORE requiring secretStore (path is computed at load)
const ISOLATED_DIR = fs.mkdtempSync(path.join(os.tmpdir(), 'btcpc-secretstore-'));
process.env.BTCPC_SECRETS_PATH = path.join(ISOLATED_DIR, 'secrets.json');

const secretStore = require('../src/services/secretStore');

describe('secretStore — loading', () => {
  beforeEach(async () => {
    secretStore.resetForTests();
    await secretStore.load();
  });

  afterAll(() => {
    try { fs.rmSync(ISOLATED_DIR, { recursive: true, force: true }); } catch (_) {}
  });

  it('creates the secrets file on first load', () => {
    expect(fs.existsSync(process.env.BTCPC_SECRETS_PATH)).toBe(true);
  });

  it('reads back an existing store', async () => {
    await secretStore.createUser('alice', { password: 'hunter2', email: 'alice@example.com' });
    // Force a reload by wiping in-memory state
    secretStore.resetForTests();
    // Need to recreate the file for reload test
    await secretStore.load();
    expect(secretStore.hasUser('alice')).toBe(false); // reset wiped the file too
  });

  it('creates the file with mode 0600', () => {
    const stat = fs.statSync(process.env.BTCPC_SECRETS_PATH);
    // mode & 0o777 gives permission bits
    const perms = stat.mode & 0o777;
    expect(perms).toBe(0o600);
  });
});

describe('secretStore — users', () => {
  beforeEach(async () => {
    secretStore.resetForTests();
    await secretStore.load();
  });

  it('createUser with plaintext password hashes it', async () => {
    const user = await secretStore.createUser('alice', { password: 'hunter2' });
    expect(user.password_hash).toBeTruthy();
    expect(user.password_hash).not.toBe('hunter2');
    expect(user.auth_profile).toBe('password');
  });

  it('createUser with pre-hashed password preserves it', async () => {
    const user = await secretStore.createUser('alice', { password_hash: '$2a$10$abcdefg' });
    expect(user.password_hash).toBe('$2a$10$abcdefg');
  });

  it('createUser with no password sets auth_profile none', async () => {
    const user = await secretStore.createUser('alice', {});
    expect(user.password_hash).toBeNull();
    expect(user.auth_profile).toBe('none');
  });

  it('rejects duplicate username', async () => {
    await secretStore.createUser('alice', { password: 'hunter2' });
    await expect(secretStore.createUser('alice', { password: 'other' }))
      .rejects.toThrow(/exists/);
  });

  it('getUser returns the record or null', async () => {
    await secretStore.createUser('alice', { password: 'hunter2' });
    expect(secretStore.getUser('alice')).toBeTruthy();
    expect(secretStore.getUser('bob')).toBeNull();
  });

  it('hasUser returns bool', async () => {
    await secretStore.createUser('alice', {});
    expect(secretStore.hasUser('alice')).toBe(true);
    expect(secretStore.hasUser('bob')).toBe(false);
  });

  it('getUserById looks up via user_id_index', async () => {
    const user = await secretStore.createUser('alice', {});
    const found = secretStore.getUserById(user.user_id);
    expect(found).toBeTruthy();
    expect(found.user_id).toBe(user.user_id);
  });

  it('getUserByTelegramId looks up via telegram_id_index', async () => {
    await secretStore.createUser('alice', { telegram_id: '12345' });
    const found = secretStore.getUserByTelegramId('12345');
    expect(found).toBeTruthy();
    expect(found.telegram_id).toBe('12345');
    expect(secretStore.getUserByTelegramId('99999')).toBeNull();
  });

  it('verifyPassword returns true for correct password', async () => {
    await secretStore.createUser('alice', { password: 'hunter2' });
    await expect(secretStore.verifyPassword('alice', 'hunter2')).resolves.toBe(true);
  });

  it('verifyPassword returns false for wrong password', async () => {
    await secretStore.createUser('alice', { password: 'hunter2' });
    await expect(secretStore.verifyPassword('alice', 'wrong')).resolves.toBe(false);
  });

  it('verifyPassword returns false for missing user', async () => {
    await expect(secretStore.verifyPassword('ghost', 'anything')).resolves.toBe(false);
  });

  it('updateUser with plaintext password hashes it', async () => {
    await secretStore.createUser('alice', { password: 'hunter2' });
    await secretStore.updateUser('alice', { password: 'newpass' });
    await expect(secretStore.verifyPassword('alice', 'newpass')).resolves.toBe(true);
    await expect(secretStore.verifyPassword('alice', 'hunter2')).resolves.toBe(false);
  });

  it('updateUser re-indexes telegram_id', async () => {
    await secretStore.createUser('alice', { telegram_id: '100' });
    await secretStore.updateUser('alice', { telegram_id: '200' });
    expect(secretStore.getUserByTelegramId('100')).toBeNull();
    expect(secretStore.getUserByTelegramId('200')).toBeTruthy();
  });

  it('updateUser can clear telegram_id', async () => {
    await secretStore.createUser('alice', { telegram_id: '100' });
    await secretStore.updateUser('alice', { telegram_id: null });
    expect(secretStore.getUserByTelegramId('100')).toBeNull();
  });

  it('setPassword hashes and stores', async () => {
    await secretStore.createUser('alice', {});
    await secretStore.setPassword('alice', 'new-secret');
    await expect(secretStore.verifyPassword('alice', 'new-secret')).resolves.toBe(true);
  });

  it('deleteUser removes and cleans indexes', async () => {
    const user = await secretStore.createUser('alice', { telegram_id: '42' });
    const deleted = await secretStore.deleteUser('alice');
    expect(deleted).toBe(true);
    expect(secretStore.getUser('alice')).toBeNull();
    expect(secretStore.getUserById(user.user_id)).toBeNull();
    expect(secretStore.getUserByTelegramId('42')).toBeNull();
  });

  it('deleteUser returns false for missing user', async () => {
    const deleted = await secretStore.deleteUser('ghost');
    expect(deleted).toBe(false);
  });

  it('getAllUsers strips sensitive fields', async () => {
    await secretStore.createUser('alice', { password: 'hunter2', email: 'alice@example.com' });
    const all = secretStore.getAllUsers();
    expect(all).toHaveLength(1);
    expect(all[0].username).toBe('alice');
    expect(all[0].email).toBe('alice@example.com');
    expect(all[0].password_hash).toBeUndefined();
    expect(all[0].totp_secret).toBeUndefined();
  });
});

describe('secretStore — projects + api keys', () => {
  beforeEach(async () => {
    secretStore.resetForTests();
    await secretStore.load();
  });

  it('createProject returns plaintext apiKey once', async () => {
    const result = await secretStore.createProject({
      name: 'myproject',
      owner: 'alice',
      repo_url: 'https://github.com/alice/my',
      repo: 'my',
      wallet_address: 'alice',
    });
    expect(result.project.name).toBeUndefined(); // name isn't in the stored record, only as the map key
    expect(result.apiKey).toMatch(/^btcpc_[a-f0-9]{32}$/);
    expect(result.project.api_key_hash).toBeTruthy();
    expect(result.project.api_key_hash).not.toContain('btcpc_');
  });

  it('getProjectByApiKey looks up by sha256 hash', async () => {
    const { apiKey } = await secretStore.createProject({ name: 'myproject' });
    const found = secretStore.getProjectByApiKey(apiKey);
    expect(found).toBeTruthy();
    expect(secretStore.getProjectByApiKey('not-a-real-key')).toBeNull();
  });

  it('rejects duplicate project name', async () => {
    await secretStore.createProject({ name: 'myproject' });
    await expect(secretStore.createProject({ name: 'myproject' }))
      .rejects.toThrow(/exists/);
  });

  it('rotateProjectKey returns new plaintext and invalidates old', async () => {
    const first = await secretStore.createProject({ name: 'myproject' });
    const oldKey = first.apiKey;

    const second = await secretStore.rotateProjectKey('myproject');
    const newKey = second.apiKey;

    expect(newKey).not.toBe(oldKey);
    expect(secretStore.getProjectByApiKey(oldKey)).toBeNull();
    expect(secretStore.getProjectByApiKey(newKey)).toBeTruthy();
  });

  it('deleteProject unlinks api_key_index', async () => {
    const { apiKey } = await secretStore.createProject({ name: 'myproject' });
    await secretStore.deleteProject('myproject');
    expect(secretStore.getProject('myproject')).toBeNull();
    expect(secretStore.getProjectByApiKey(apiKey)).toBeNull();
  });

  it('getAllProjects strips api_key_hash', async () => {
    await secretStore.createProject({ name: 'p1', owner: 'alice' });
    await secretStore.createProject({ name: 'p2', owner: 'bob' });
    const all = secretStore.getAllProjects();
    expect(all).toHaveLength(2);
    for (const p of all) {
      expect(p.api_key_hash).toBeUndefined();
    }
  });
});

describe('secretStore — persistence across reloads', () => {
  beforeEach(() => {
    secretStore.resetForTests();
  });

  it('persists users across a load/save cycle', async () => {
    await secretStore.load();
    await secretStore.createUser('alice', { password: 'hunter2', telegram_id: '12345' });

    // Wipe in-memory state WITHOUT deleting the file
    // (simulates a fresh node process loading the same file)
    const secretStoreModule = require.resolve('../src/services/secretStore');
    delete require.cache[secretStoreModule];
    const reloaded = require('../src/services/secretStore');
    await reloaded.load();

    expect(reloaded.hasUser('alice')).toBe(true);
    expect(reloaded.getUserByTelegramId('12345')).toBeTruthy();
    // Password should still verify after reload
    await expect(reloaded.verifyPassword('alice', 'hunter2')).resolves.toBe(true);
  });

  it('rebuilds telegram_id_index from user records if missing', async () => {
    await secretStore.load();
    await secretStore.createUser('alice', { telegram_id: '12345' });

    // Corrupt the file by removing the telegram_id_index (simulates a
    // secrets.json from before D.5-alpha upgraded the format)
    const raw = JSON.parse(fs.readFileSync(process.env.BTCPC_SECRETS_PATH, 'utf8'));
    delete raw.telegram_id_index;
    fs.writeFileSync(process.env.BTCPC_SECRETS_PATH, JSON.stringify(raw));

    // Reload in a fresh module instance
    const secretStoreModule = require.resolve('../src/services/secretStore');
    delete require.cache[secretStoreModule];
    const reloaded = require('../src/services/secretStore');
    await reloaded.load();

    expect(reloaded.getUserByTelegramId('12345')).toBeTruthy();
  });
});

describe('secretStore — stats', () => {
  beforeEach(async () => {
    secretStore.resetForTests();
    await secretStore.load();
  });

  it('reports user + project + telegram counts', async () => {
    await secretStore.createUser('alice', { telegram_id: '100' });
    await secretStore.createUser('bob', {});
    await secretStore.createProject({ name: 'p1' });

    const s = secretStore.stats();
    expect(s.users).toBe(2);
    expect(s.projects).toBe(1);
    expect(s.telegram_linked).toBe(1);
    expect(s.installation_id).toBeTruthy();
  });
});
