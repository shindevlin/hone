"use strict";

async function loadSecretStore() {
  const secretStore = require("./secretStore");
  try { await secretStore.load(); } catch (_) {}
  return secretStore;
}

async function resolveUserFromAuth(authUser) {
  if (!authUser) return null;

  try {
    const secretStore = await loadSecretStore();
    let rec = null;
    if (authUser.username && typeof secretStore.getUser === "function") {
      rec = secretStore.getUser(authUser.username);
    }
    if (!rec && authUser.id && typeof secretStore.getUserById === "function") {
      rec = secretStore.getUserById(authUser.id);
    }
    if (rec) {
      return {
        source: "secretStore",
        id: rec.user_id || authUser.id,
        username: rec.username || authUser.username,
        email: rec.email || null,
        isActive: rec.is_active !== false,
        totpEnabled: !!rec.totp_enabled,
        totpSecret: rec.totp_secret || null,
        totpBackupCodes: rec.totp_backup_codes || [],
        record: rec,
      };
    }
  } catch (_) {}

  try {
    const User = require("../models/User");
    const mongoUser = await User.findById(authUser.id);
    if (mongoUser) {
      return {
        source: "mongo",
        id: mongoUser._id ? mongoUser._id.toString() : authUser.id,
        username: mongoUser.username,
        email: mongoUser.email || null,
        isActive: mongoUser.isActive !== false,
        totpEnabled: !!mongoUser.totpEnabled,
        totpSecret: mongoUser.totpSecret || null,
        totpBackupCodes: mongoUser.totpBackupCodes || [],
        privateAuth: mongoUser.privateAuth || null,
        record: mongoUser,
      };
    }
  } catch (_) {}

  return null;
}

module.exports = {
  loadSecretStore,
  resolveUserFromAuth,
};
