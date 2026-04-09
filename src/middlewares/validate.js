"use strict";

function isPlainString(val) {
  return typeof val === 'string';
}

function rejectObjectInputs(obj, fields) {
  for (var i = 0; i < fields.length; i++) {
    var val = obj[fields[i]];
    if (val !== undefined && val !== null && typeof val === 'object') {
      return fields[i] + ' must be a string';
    }
  }
  return null;
}

function validAccountName(name) {
  if (!isPlainString(name)) return false;
  return /^[a-z0-9][a-z0-9._-]{2,19}$/.test(name);
}

function validAmount(val) {
  var n = Number(val);
  return typeof val !== 'object' && !isNaN(n) && isFinite(n) && n > 0;
}

function sanitizeAmount(val) {
  var n = Number(val);
  if (typeof val === 'object' || isNaN(n) || !isFinite(n) || n <= 0) return null;
  return n;
}

function validPositiveInt(val) {
  var n = Number(val);
  return typeof val !== 'object' && Number.isInteger(n) && n > 0;
}

function sanitizePagination(page, limit, maxLimit) {
  maxLimit = maxLimit || 100;
  var p = parseInt(page, 10);
  var l = parseInt(limit, 10);
  if (isNaN(p) || p < 1) p = 1;
  if (isNaN(l) || l < 1) l = 25;
  if (l > maxLimit) l = maxLimit;
  return { page: p, limit: l };
}

function sanitizeString(val, maxLen) {
  if (!isPlainString(val)) return null;
  maxLen = maxLen || 1000;
  return val.slice(0, maxLen);
}

function validTelegramId(val) {
  if (!isPlainString(val) && typeof val !== 'number') return false;
  var s = String(val);
  return /^\d{1,20}$/.test(s);
}

function sanitizeTelegramId(val) {
  if (val === undefined || val === null) return null;
  if (typeof val === 'object') return null;
  var s = String(val);
  if (!/^\d{1,20}$/.test(s)) return null;
  return s;
}

function validHexString(val, maxLen) {
  if (!isPlainString(val)) return false;
  maxLen = maxLen || 128;
  return /^[0-9a-fA-F]+$/.test(val) && val.length <= maxLen;
}

function validAddress(val) {
  if (!isPlainString(val)) return false;
  return val.length >= 10 && val.length <= 200 && /^[a-zA-Z0-9_]+$/.test(val);
}

function validUrl(val) {
  if (!isPlainString(val)) return false;
  if (val.length > 2048) return false;
  try {
    new URL(val);
    return true;
  } catch (_) {
    return false;
  }
}

function validEndpoint(val) {
  if (!isPlainString(val)) return false;
  return val.length <= 500;
}

function validModel(val) {
  if (!isPlainString(val)) return false;
  return val.length <= 100 && /^[a-zA-Z0-9._:/-]+$/.test(val);
}

function validChain(val) {
  if (!isPlainString(val)) return false;
  var allowed = ['btcpc', 'evm', 'solana', 'bitcoin', 'ton', 'hive', 'eth', 'bsc', 'polygon', 'base', 'arbitrum'];
  return allowed.includes(val.toLowerCase());
}

function validMinerMode(val) {
  if (!isPlainString(val)) return false;
  return ['full', 'reduced', 'paused'].includes(val);
}

module.exports = {
  isPlainString,
  rejectObjectInputs,
  validAccountName,
  validAmount,
  sanitizeAmount,
  validPositiveInt,
  sanitizePagination,
  sanitizeString,
  validTelegramId,
  sanitizeTelegramId,
  validHexString,
  validAddress,
  validUrl,
  validEndpoint,
  validModel,
  validChain,
  validMinerMode
};
