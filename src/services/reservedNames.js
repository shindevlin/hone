"use strict";

const fs = require("fs");
const path = require("path");

const RESERVED_NAMES_PATH = path.resolve(__dirname, "../../data/reserved-names.json");
const RESERVED_OWNER = "shindevlin";

function pushLetterCombos(out, length, prefix) {
  prefix = prefix || "";
  if (prefix.length === length) {
    out.push(prefix);
    return;
  }
  for (let i = 97; i <= 122; i++) {
    pushLetterCombos(out, length, prefix + String.fromCharCode(i));
  }
}

function loadCuratedNames() {
  try {
    return JSON.parse(fs.readFileSync(RESERVED_NAMES_PATH, "utf8"));
  } catch (_) {
    return [];
  }
}

function getReservedNamesForShin() {
  const names = new Set();
  for (const raw of loadCuratedNames()) {
    const name = String(raw || "").trim().toLowerCase();
    if (/^[a-z0-9][a-z0-9._-]{0,19}$/.test(name) && name !== RESERVED_OWNER) {
      names.add(name);
    }
  }

  const combos = [];
  pushLetterCombos(combos, 1);
  pushLetterCombos(combos, 2);
  pushLetterCombos(combos, 3);
  for (const name of combos) {
    if (name !== RESERVED_OWNER) names.add(name);
  }

  return Array.from(names).sort();
}

module.exports = {
  RESERVED_OWNER,
  getReservedNamesForShin,
};
