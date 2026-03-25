"use strict";

/**
 * BTCPC Content Filter
 * Applies automated redaction to public dream inscriptions.
 * Inference is NEVER filtered — only public inscription text.
 */

const REDACTION = "XXXXXXXXX";

// Minimal prohibited pattern list — CSAM-related terms
const PROHIBITED_PATTERNS = [
  /\bchild\s*(porn|pornograph|sexual|abuse|exploit)/gi,
  /\bminor\s*(porn|pornograph|sexual|abuse|exploit)/gi,
  /\bunderage\s*(porn|pornograph|sexual|abuse|exploit)/gi,
  /\bpedophil/gi,
  /\bpaedophil/gi,
  /\bjailbait/gi,
  /\bcsam\b/gi,
  /\bchild\s*sex/gi,
  /\bkiddie\s*porn/gi,
  /\bloli\b/gi,
  /\bshota\b/gi
];

// URL detection pattern
const URL_PATTERN = /https?:\/\/[^\s]+|www\.[^\s]+|[a-zA-Z0-9-]+\.(com|org|net|io|co|xyz|dev|app|ai|me|info|biz|us|uk|de|fr|jp|cn|ru|br|in|au)\b(\/[^\s]*)?/gi;

const BASE_FEE = 0.01; // BTCPC
const URL_FEE_MULTIPLIER = 10;

/**
 * Scan text for prohibited patterns and replace with redaction string.
 * @param {string} text - inscription text to filter
 * @returns {{ filtered_text: string, was_redacted: boolean, has_url: boolean, fee_multiplier: number }}
 */
function filterInscription(text) {
  if (!text || typeof text !== "string") {
    return {
      filtered_text: text || "",
      was_redacted: false,
      has_url: false,
      fee_multiplier: 1
    };
  }

  let filtered = text;
  let wasRedacted = false;

  for (const pattern of PROHIBITED_PATTERNS) {
    // Reset lastIndex for global patterns
    pattern.lastIndex = 0;
    if (pattern.test(filtered)) {
      wasRedacted = true;
      pattern.lastIndex = 0;
      filtered = filtered.replace(pattern, REDACTION);
    }
  }

  const hasUrl = containsURL(filtered);
  const feeMultiplier = hasUrl ? URL_FEE_MULTIPLIER : 1;

  return {
    filtered_text: filtered,
    was_redacted: wasRedacted,
    has_url: hasUrl,
    fee_multiplier: feeMultiplier
  };
}

/**
 * Detect external URLs/links in text.
 * @param {string} text
 * @returns {boolean}
 */
function containsURL(text) {
  if (!text || typeof text !== "string") return false;
  URL_PATTERN.lastIndex = 0;
  return URL_PATTERN.test(text);
}

/**
 * Calculate inscription fee based on content.
 * @param {string} text - inscription text
 * @returns {number} fee in BTCPC
 */
function getInscriptionFee(text) {
  const hasUrl = containsURL(text);
  return hasUrl ? BASE_FEE * URL_FEE_MULTIPLIER : BASE_FEE;
}

module.exports = {
  filterInscription,
  containsURL,
  getInscriptionFee,
  REDACTION,
  BASE_FEE,
  URL_FEE_MULTIPLIER
};
