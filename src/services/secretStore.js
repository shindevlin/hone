"use strict";

/**
 * BTCPC Secret Store — per-installation secrets file.
 * Shin Devlin
 *
 * Holds data that is NOT chain state:
 *   - user password hashes (bcrypt)
 *   - TOTP secrets and backup codes
 *   - project API key hashes (sha256 of plaintext key)
 *   - JWT refresh tokens
 *   - Telegram link tokens
 *
 * Why a file and not MongoDB:
 *   - Per-installation secrets should never leave the machine
 *   - Mongo is being phased out as chain state — don't replace with a new
 *     Mongo dependency
 *   - JSON file is simpler, easier to back up, mode 0600 restricts access
 *
 * Why not the chain:
 *   - Passwords and TOTP seeds are secrets. Secrets on a public chain = game over
 *   - API keys must be per-machine (regenerable on reinstall)
 *
 * Format:
 *   {
 *     version: 1,
 *     created_at: timestamp,
 *     installation_id: uuid,
 *     users: {
 *       [username]: {
 *         user_id, email, password_hash, totp_secret, totp_backup_codes,
 *         totp_enabled, two_factor_enabled, auth_profile, telegram_id,
 *         mcp_servers, last_login, created_at
 *       }
 *     },
 *     projects: {
 *       [name]: {
 *         owner, repo_url, repo, wallet_address, created_at
 *       }
 *     },
 *     api_key_index: { [sha256_hash]: project_name }
 *   }
 *
 * Plaintext API keys are NEVER stored — only their sha256 hashes. Plaintext
 * is shown once on creation, then the user must save it themselves.
 */

var fs = require("fs");
var path = require("path");
var os = require("os");
var crypto = require("crypto");
var bcrypt = require("bcryptjs");

var SECRETS_PATH = process.env.BTCPC_SECRETS_PATH ||
  path.join(os.homedir(), ".btcpc", "secrets.json");

var state = null;

function _empty() {
  return {
    version: 1,
    created_at: Date.now(),
    installation_id: crypto.randomUUID(),
    users: {},
    projects: {},
    api_key_index: {},
    user_id_index: {},      // userId → username
    telegram_id_index: {},  // telegramId → username
  };
}

function _sha256(text) {
  return crypto.createHash("sha256").update(String(text)).digest("hex");
}

// ─────────────────────────────────────────────────────────────────
// Load / save
// ─────────────────────────────────────────────────────────────────

async function load() {
  var dir = path.dirname(SECRETS_PATH);
  if (!fs.existsSync(dir)) {
    fs.mkdirSync(dir, { recursive: true, mode: 0o700 });
  }

  if (fs.existsSync(SECRETS_PATH)) {
    try {
      var raw = fs.readFileSync(SECRETS_PATH, "utf8");
      state = JSON.parse(raw);
      if (!state.version) state.version = 1;
      if (!state.users) state.users = {};
      if (!state.projects) state.projects = {};
      if (!state.api_key_index) state.api_key_index = {};
      if (!state.user_id_index) state.user_id_index = {};
      if (!state.telegram_id_index) state.telegram_id_index = {};
      // Rebuild telegram_id_index from user records if missing (forward-compat)
      for (var username of Object.keys(state.users)) {
        var u = state.users[username];
        if (u.telegram_id && !state.telegram_id_index[u.telegram_id]) {
          state.telegram_id_index[u.telegram_id] = username;
        }
      }
      return state;
    } catch (e) {
      var bakPath = SECRETS_PATH + ".bak." + Date.now();
      try { fs.renameSync(SECRETS_PATH, bakPath); } catch (_) {}
      console.error("[secretStore] Corrupt secrets.json — backed up to " + bakPath + ", starting fresh");
      state = _empty();
      await save();
      return state;
    }
  }

  // First run — create empty
  state = _empty();
  await save();
  return state;
}

async function save() {
  if (!state) state = _empty();
  var dir = path.dirname(SECRETS_PATH);
  if (!fs.existsSync(dir)) {
    fs.mkdirSync(dir, { recursive: true, mode: 0o700 });
  }
  var tmp = SECRETS_PATH + ".tmp";
  fs.writeFileSync(tmp, JSON.stringify(state, null, 2), { mode: 0o600 });
  fs.renameSync(tmp, SECRETS_PATH);
}

function _ensureLoaded() {
  if (!state) {
    throw new Error("secretStore not loaded — call secretStore.load() at startup");
  }
}

// ─────────────────────────────────────────────────────────────────
// Users
// ─────────────────────────────────────────────────────────────────

function getUser(username) {
  _ensureLoaded();
  if (!username) return null;
  return state.users[username] || null;
}

function getUserById(userId) {
  _ensureLoaded();
  var username = state.user_id_index[userId];
  return username ? state.users[username] : null;
}

async function createUser(username, fields) {
  _ensureLoaded();
  if (!username) throw new Error("username required");
  if (state.users[username]) throw new Error("user exists");
  fields = fields || {};

  // Accept either plaintext 'password' (hashed here) OR a pre-hashed
  // 'password_hash' (for migration from Mongo or restoring backups).
  var passwordHash = fields.password_hash || null;
  if (fields.password) {
    passwordHash = await bcrypt.hash(fields.password, 10);
  }

  var userId = fields.user_id || crypto.randomUUID();
  state.users[username] = {
    user_id: userId,
    email: fields.email || null,
    password_hash: passwordHash,
    totp_secret: fields.totp_secret || null,
    totp_backup_codes: fields.totp_backup_codes || [],
    totp_enabled: fields.totp_enabled || false,
    two_factor_enabled: fields.two_factor_enabled || false,
    auth_profile: fields.auth_profile || (passwordHash ? "password" : "none"),
    telegram_id: fields.telegram_id || null,
    telegram_username: fields.telegram_username || null,
    owner_public_key: fields.owner_public_key || null,
    active_public_key: fields.active_public_key || null,
    posting_public_key: fields.posting_public_key || null,
    memo_public_key: fields.memo_public_key || null,
    two_factor_public_key: fields.two_factor_public_key || null,
    mcp_servers: fields.mcp_servers || [],
    last_login: null,
    created_at: Date.now(),
  };
  state.user_id_index[userId] = username;
  if (fields.telegram_id) {
    state.telegram_id_index[fields.telegram_id] = username;
  }
  await save();
  return state.users[username];
}

async function updateUser(username, patch) {
  _ensureLoaded();
  var user = state.users[username];
  if (!user) throw new Error("user not found");
  patch = patch || {};

  // Hash plaintext password if supplied
  if (patch.password) {
    patch.password_hash = await bcrypt.hash(patch.password, 10);
    delete patch.password;
    if (!user.auth_profile || user.auth_profile === "none") {
      user.auth_profile = "password";
    }
  }

  // Telegram id re-indexing
  if (Object.prototype.hasOwnProperty.call(patch, "telegram_id")) {
    if (user.telegram_id && state.telegram_id_index[user.telegram_id] === username) {
      delete state.telegram_id_index[user.telegram_id];
    }
    if (patch.telegram_id) {
      state.telegram_id_index[patch.telegram_id] = username;
    }
  }

  Object.assign(user, patch);
  await save();
  return user;
}

async function deleteUser(username) {
  _ensureLoaded();
  var user = state.users[username];
  if (!user) return false;
  delete state.user_id_index[user.user_id];
  if (user.telegram_id && state.telegram_id_index[user.telegram_id] === username) {
    delete state.telegram_id_index[user.telegram_id];
  }
  delete state.users[username];
  await save();
  return true;
}

function hasUser(username) {
  _ensureLoaded();
  if (!username) return false;
  return !!state.users[username];
}

function getUserByTelegramId(telegramId) {
  _ensureLoaded();
  if (!telegramId) return null;
  var username = state.telegram_id_index[String(telegramId)];
  return username ? state.users[username] : null;
}

function stats() {
  _ensureLoaded();
  return {
    users: Object.keys(state.users).length,
    projects: Object.keys(state.projects).length,
    telegram_linked: Object.keys(state.telegram_id_index).length,
    installation_id: state.installation_id,
    created_at: state.created_at,
    file: SECRETS_PATH,
  };
}

// Tests-only: wipe in-memory state and on-disk file
function resetForTests() {
  state = null;
  try {
    if (fs.existsSync(SECRETS_PATH)) fs.unlinkSync(SECRETS_PATH);
  } catch (_) {}
}

async function verifyPassword(username, plaintext) {
  var user = getUser(username);
  if (!user || !user.password_hash) return false;
  return bcrypt.compare(plaintext, user.password_hash);
}

async function setPassword(username, plaintext) {
  var hash = await bcrypt.hash(plaintext, 10);
  return updateUser(username, { password_hash: hash });
}

function getAllUsers() {
  _ensureLoaded();
  // Strip sensitive fields
  return Object.entries(state.users).map(function (entry) {
    return {
      username: entry[0],
      email: entry[1].email,
      created_at: entry[1].created_at,
      auth_profile: entry[1].auth_profile,
      totp_enabled: entry[1].totp_enabled,
    };
  });
}

// ─────────────────────────────────────────────────────────────────
// Projects
// ─────────────────────────────────────────────────────────────────

function getProject(name) {
  _ensureLoaded();
  return state.projects[name] || null;
}

function getProjectByApiKey(plaintextKey) {
  _ensureLoaded();
  var hash = _sha256(plaintextKey);
  var name = state.api_key_index[hash];
  return name ? state.projects[name] : null;
}

async function createProject(fields) {
  _ensureLoaded();
  if (!fields.name) throw new Error("name required");
  if (state.projects[fields.name]) throw new Error("project exists");

  // Generate a fresh API key (plaintext shown ONCE to the caller)
  var plaintextKey = "btcpc_" + crypto.randomBytes(16).toString("hex");
  var hash = _sha256(plaintextKey);

  state.projects[fields.name] = {
    owner: fields.owner || null,
    repo_url: fields.repo_url || null,
    repo: fields.repo || null,
    wallet_address: fields.wallet_address || null,
    api_key_hash: hash,
    created_at: Date.now(),
  };
  state.api_key_index[hash] = fields.name;
  await save();

  return {
    project: state.projects[fields.name],
    apiKey: plaintextKey, // show once, never stored in plaintext
  };
}

async function rotateProjectKey(name) {
  _ensureLoaded();
  var project = state.projects[name];
  if (!project) throw new Error("project not found");

  // Remove old hash from index
  if (project.api_key_hash) {
    delete state.api_key_index[project.api_key_hash];
  }

  // Generate new
  var plaintextKey = "btcpc_" + crypto.randomBytes(16).toString("hex");
  var hash = _sha256(plaintextKey);
  project.api_key_hash = hash;
  state.api_key_index[hash] = name;

  await save();
  return { project: project, apiKey: plaintextKey };
}

async function deleteProject(name) {
  _ensureLoaded();
  var project = state.projects[name];
  if (!project) return false;
  if (project.api_key_hash) delete state.api_key_index[project.api_key_hash];
  delete state.projects[name];
  await save();
  return true;
}

function getAllProjects() {
  _ensureLoaded();
  // Strip api_key_hash
  return Object.entries(state.projects).map(function (entry) {
    var p = entry[1];
    return {
      name: entry[0],
      owner: p.owner,
      repo_url: p.repo_url,
      repo: p.repo,
      wallet_address: p.wallet_address,
      created_at: p.created_at,
    };
  });
}

// ─────────────────────────────────────────────────────────────────
// Export
// ─────────────────────────────────────────────────────────────────

module.exports = {
  load: load,
  save: save,
  SECRETS_PATH: SECRETS_PATH,
  // Users
  getUser: getUser,
  getUserById: getUserById,
  getUserByTelegramId: getUserByTelegramId,
  hasUser: hasUser,
  createUser: createUser,
  updateUser: updateUser,
  deleteUser: deleteUser,
  verifyPassword: verifyPassword,
  setPassword: setPassword,
  getAllUsers: getAllUsers,
  // Projects
  getProject: getProject,
  getProjectByApiKey: getProjectByApiKey,
  createProject: createProject,
  rotateProjectKey: rotateProjectKey,
  deleteProject: deleteProject,
  getAllProjects: getAllProjects,
  // Introspection
  stats: stats,
  resetForTests: resetForTests,
};
