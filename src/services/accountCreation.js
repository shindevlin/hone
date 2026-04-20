"use strict";

const crypto = require("crypto");
const bcrypt = require("bcryptjs");
const keyManager = require("../wallet/keyManager");
const ledger = require("./ledger");
const stateStore = require("../chain/stateStore");
const secretStore = require("./secretStore");
const { RESERVED_OWNER } = require("./reservedNames");
const {
  validAccountName,
  blockedAccountNameReason,
} = require("../middlewares/validate");

function normalizeUsername(username) {
  return String(username || "").trim().toLowerCase();
}

function validateNewUsername(username) {
  const clean = normalizeUsername(username);
  if (!clean) return { ok: false, error: "username required" };
  if (!validAccountName(clean)) {
    return { ok: false, error: "Username must be 3-20 chars, lowercase letters/numbers/.-_ only" };
  }
  const blocked = blockedAccountNameReason(clean);
  if (blocked) return { ok: false, error: blocked };
  return { ok: true, username: clean };
}

function btcpcAddressFromOwnerPublicKey(ownerPublicKey) {
  const hash = crypto.createHash("sha256").update(Buffer.from(ownerPublicKey, "hex")).digest();
  return "BTCPC" + hash.subarray(0, 20).toString("hex");
}

function chainValue(wallet, field) {
  return wallet && wallet[field] ? wallet[field] : null;
}

function buildWalletRows(username, mnemonic, roleKeys, chainWallets, btcpcAddress) {
  return [
    {
      chain: "btcpc",
      label: "BTCPC owner key",
      purpose: "Rotate keys and recover account authority.",
      account: username,
      address: btcpcAddress,
      public_key: roleKeys.owner.publicKey,
      private_key: roleKeys.owner.privateKey,
    },
    {
      chain: "btcpc",
      label: "BTCPC active key",
      purpose: "Move tokens and authorize financial actions.",
      account: username,
      address: btcpcAddress,
      public_key: roleKeys.active.publicKey,
      private_key: roleKeys.active.privateKey,
    },
    {
      chain: "btcpc",
      label: "BTCPC posting key",
      purpose: "Mine, post proofs, and run node operations.",
      account: username,
      address: btcpcAddress,
      public_key: roleKeys.posting.publicKey,
      private_key: roleKeys.posting.privateKey,
    },
    {
      chain: "btcpc",
      label: "BTCPC memo key",
      purpose: "Reserved for encrypted memos and future message encryption.",
      account: username,
      address: btcpcAddress,
      public_key: roleKeys.memo.publicKey,
      private_key: roleKeys.memo.privateKey,
    },
    {
      chain: "evm",
      label: "EVM wallet",
      purpose: "Ethereum-compatible chains such as Base, Arbitrum, Optimism, Polygon, and BSC.",
      address: chainValue(chainWallets.evm, "address"),
      public_key: chainValue(chainWallets.evm, "publicKey"),
      private_key: chainValue(chainWallets.evm, "privateKey"),
    },
    {
      chain: "solana",
      label: "Solana wallet",
      purpose: "Solana imports.",
      address: chainValue(chainWallets.solana, "address"),
      public_key: chainValue(chainWallets.solana, "publicKey"),
      private_key: chainValue(chainWallets.solana, "privateKey"),
    },
    {
      chain: "ton",
      label: "TON wallet",
      purpose: "TON imports. Some wallets use the link address.",
      address: chainValue(chainWallets.ton, "address"),
      link_address: chainValue(chainWallets.ton, "linkAddress"),
      public_key: chainValue(chainWallets.ton, "publicKey"),
      private_key: chainValue(chainWallets.ton, "privateKey"),
    },
    {
      chain: "bitcoin",
      label: "Bitcoin wallet",
      purpose: "Native SegWit Bitcoin wallet.",
      address: chainValue(chainWallets.bitcoin, "address"),
      public_key: chainValue(chainWallets.bitcoin, "publicKey"),
      private_key: chainValue(chainWallets.bitcoin, "privateKey"),
    },
    {
      chain: "hive",
      label: "Hive identity",
      purpose: "Hive-compatible identity name. No separate Hive private key is derived here.",
      account: username,
      address: username,
      public_key: null,
      private_key: null,
    },
  ];
}

function buildWalletExport(username, mnemonic, walletRows, balances) {
  const lines = [];
  lines.push("BTCPC WALLET EXPORT");
  lines.push("Shown once. Store offline. Anyone with these private keys or mnemonic can control the wallet.");
  lines.push("");
  lines.push("Account: " + username);
  lines.push("Mnemonic: " + mnemonic);
  lines.push("Wallet balance BTCPC: " + (balances.wallet || 0));
  lines.push("Delegated BTCPC for inference: " + (balances.delegated || 0));
  lines.push("");
  for (const row of walletRows) {
    lines.push(row.label);
    if (row.purpose) lines.push("Purpose: " + row.purpose);
    if (row.account) lines.push("Account: " + row.account);
    if (row.address) lines.push("Address: " + row.address);
    if (row.link_address) lines.push("Link address: " + row.link_address);
    if (row.public_key) lines.push("Public key: " + row.public_key);
    if (row.private_key) lines.push("Private key: " + row.private_key);
    lines.push("");
  }
  return lines.join("\n").trim();
}

function flattenWalletFields(rows) {
  const flat = {};
  for (const row of rows) {
    const prefix = row.chain + "_" + row.label.toLowerCase().replace(/^btcpc /, "").replace(/[^a-z0-9]+/g, "_").replace(/^_|_$/g, "");
    if (row.address) flat[prefix + "_address"] = row.address;
    if (row.link_address) flat[prefix + "_link_address"] = row.link_address;
    if (row.public_key) flat[prefix + "_public_key"] = row.public_key;
    if (row.private_key) flat[prefix + "_private_key"] = row.private_key;
  }
  return flat;
}

async function buildWalletMaterial(username, mnemonic, balances) {
  const roleKeys = await keyManager.mnemonicToKeys(mnemonic);
  const publicKeys = {
    owner: roleKeys.owner.publicKey,
    active: roleKeys.active.publicKey,
    posting: roleKeys.posting.publicKey,
    memo: roleKeys.memo.publicKey,
  };
  const chainWallets = await keyManager.deriveChainWallets(mnemonic);
  const btcpcAddress = btcpcAddressFromOwnerPublicKey(publicKeys.owner);
  const chainAddresses = {
    btcpc: btcpcAddress,
    evm: chainWallets.evm.address,
    solana: chainWallets.solana.address,
    bitcoin: chainWallets.bitcoin.address,
    ton: chainWallets.ton.address,
    ton_link: chainWallets.ton.linkAddress || null,
    hive: username,
  };
  const rows = buildWalletRows(username, mnemonic, roleKeys, chainWallets, btcpcAddress);
  const walletExport = buildWalletExport(username, mnemonic, rows, balances || { wallet: 0, delegated: 0 });
  const flat = flattenWalletFields(rows);

  return {
    mnemonic,
    wallet_export: walletExport,
    wallet_rows: rows,
    public_keys: publicKeys,
    chain_addresses: chainAddresses,
    chain_wallets: chainWallets,
    role_private_keys: {
      owner: roleKeys.owner.privateKey,
      active: roleKeys.active.privateKey,
      posting: roleKeys.posting.privateKey,
      memo: roleKeys.memo.privateKey,
    },
    ...flat,
  };
}

async function accountExists(username) {
  if (stateStore.getAccount && stateStore.getAccount(username)) return true;
  try {
    await secretStore.load();
    if (secretStore.hasUser && secretStore.hasUser(username)) return true;
  } catch (_) {}
  try {
    const User = require("../models/User");
    const existing = await User.findOne({ username });
    return !!existing;
  } catch (_) {
    return false;
  }
}

async function maybeCreateMongoUser(fields) {
  try {
    const User = require("../models/User");
    if (await User.findOne({ username: fields.username })) return;
    await User.create({
      username: fields.username,
      email: fields.email || fields.username + "@btcpc.local",
      password: fields.password_hash || bcrypt.hashSync(fields.password || crypto.randomBytes(32).toString("hex"), 10),
      twoFactorEnabled: !!fields.two_factor_public_key,
      authProfile: fields.auth_profile || "password",
      telegramId: fields.telegram_id || null,
      telegramUsername: fields.telegram_username || null,
      ownerPublicKey: fields.owner_public_key,
      activePublicKey: fields.active_public_key,
      postingPublicKey: fields.posting_public_key,
      memoPublicKey: fields.memo_public_key,
      twoFactorPublicKey: fields.two_factor_public_key || null,
    });
  } catch (_) {}
}

async function createAccountForUser(options) {
  options = options || {};
  const checked = validateNewUsername(options.username);
  if (!checked.ok) {
    const err = new Error(checked.error);
    err.statusCode = 400;
    throw err;
  }
  const username = checked.username;
  const password = options.password || null;

  if (options.requirePassword && (!password || typeof password !== "string" || password.length < 8 || password.length > 200)) {
    const err = new Error("password must be 8-200 characters");
    err.statusCode = 400;
    throw err;
  }
  if (password && (typeof password !== "string" || password.length > 200)) {
    const err = new Error("invalid password");
    err.statusCode = 400;
    throw err;
  }

  if (await accountExists(username)) {
    const err = new Error('Account "' + username + '" already exists');
    err.statusCode = 409;
    throw err;
  }
  if (stateStore.hasAccount && stateStore.hasAccount(RESERVED_OWNER + "/" + username)) {
    const err = new Error('Account "' + username + '" is reserved. Use the reserved-name claim flow.');
    err.statusCode = 409;
    err.reserved = true;
    err.reserved_wallet = RESERVED_OWNER + "/" + username;
    throw err;
  }

  const mnemonic = options.mnemonic || keyManager.generateMnemonic();
  if (!keyManager.validateMnemonic(mnemonic)) {
    const err = new Error("Invalid BIP-39 mnemonic");
    err.statusCode = 400;
    throw err;
  }

  const material = await buildWalletMaterial(username, mnemonic, { wallet: 0, delegated: 0 });
  const publicKeys = material.public_keys;
  const chainAddresses = material.chain_addresses;

  let secondFactor = null;
  if (password) secondFactor = await keyManager.deriveSecondFactor(password, username);

  await secretStore.load();
  await secretStore.createUser(username, {
    password,
    telegram_id: options.telegramId || null,
    telegram_username: options.telegramUsername || null,
    auth_profile: password ? "password" : "none",
    two_factor_enabled: !!password,
    owner_public_key: publicKeys.owner,
    active_public_key: publicKeys.active,
    posting_public_key: publicKeys.posting,
    memo_public_key: publicKeys.memo,
    two_factor_public_key: secondFactor ? secondFactor.publicKey : null,
  });

  await maybeCreateMongoUser({
    username,
    password,
    email: options.email || username + "@btcpc.local",
    telegram_id: options.telegramId || null,
    telegram_username: options.telegramUsername || null,
    auth_profile: password ? "password" : "none",
    owner_public_key: publicKeys.owner,
    active_public_key: publicKeys.active,
    posting_public_key: publicKeys.posting,
    memo_public_key: publicKeys.memo,
    two_factor_public_key: secondFactor ? secondFactor.publicKey : null,
  });

  const epoch = stateStore.getChainHeight ? stateStore.getChainHeight() : 0;
  await ledger.recordAccountCreate(username, publicKeys, chainAddresses, epoch);

  let delegatedBalance = 0;
  let faucetClaimed = false;
  if (options.claimFaucet !== false) {
    try {
      const amount = epoch <= 1000 ? 1 : epoch <= 10000 ? 0.1 : 0.01;
      const faucetBalance = stateStore.getBalance("btcpc_faucet", "BTCPC");
      if (faucetBalance >= amount) {
        await ledger.recordDelegate("btcpc_faucet", username, amount, "faucet", epoch);
        delegatedBalance = amount;
        faucetClaimed = true;
      }
    } catch (_) {}
  }

  const walletBalance = stateStore.getBalance ? stateStore.getBalance(username, "BTCPC") : 0;
  const balances = { wallet: walletBalance, delegated: delegatedBalance };
  const finalMaterial = await buildWalletMaterial(username, mnemonic, balances);

  return {
    success: true,
    username,
    mnemonic,
    wallet_export: finalMaterial.wallet_export,
    wallet_rows: finalMaterial.wallet_rows,
    wallet_balance: walletBalance,
    delegated_balance: delegatedBalance,
    faucet_claimed: faucetClaimed,
    faucet_note: delegatedBalance > 0
      ? "Delegated BTCPC is for inference only. It cannot be transferred or sold."
      : null,
    public_keys: finalMaterial.public_keys,
    chain_addresses: finalMaterial.chain_addresses,
    chain_wallets: finalMaterial.chain_wallets,
    role_private_keys: finalMaterial.role_private_keys,
    ...flattenWalletFields(finalMaterial.wallet_rows),
  };
}

module.exports = {
  createAccountForUser,
  validateNewUsername,
  buildWalletMaterial,
  buildWalletExport,
};
