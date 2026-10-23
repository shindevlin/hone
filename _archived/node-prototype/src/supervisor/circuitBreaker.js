"use strict";

/**
 * Circuit breaker for the hone-all supervisor.
 *
 * If a role crashes more than MAX_CRASHES times within WINDOW_MS, the breaker
 * trips for that role. The supervisor stops restarting it and keeps all other
 * roles running normally.
 *
 * Pure functions — no timers, no globals, no side effects. State lives in a
 * Map<roleName, number[]> that the caller owns.
 */

var WINDOW_MS   = parseInt(process.env.HONE_CB_WINDOW_MS)    || 60 * 60 * 1000; // 1 hour
var MAX_CRASHES = parseInt(process.env.HONE_CB_MAX_CRASHES)  || 20;

/**
 * Record one crash for `role` and return true if the breaker just tripped.
 * Prunes timestamps outside the window before deciding.
 *
 * @param {Map}    state  — caller-owned Map<role, number[]>
 * @param {string} role
 * @param {number} [nowMs]  — override for testing
 * @returns {boolean}  true = tripped (stop restarting), false = keep going
 */
function recordCrash(state, role, nowMs) {
  nowMs = nowMs != null ? nowMs : Date.now();
  var times = state.get(role) || [];
  var windowStart = nowMs - WINDOW_MS;
  times = times.filter(function (t) { return t > windowStart; });
  times.push(nowMs);
  state.set(role, times);
  return times.length > MAX_CRASHES;
}

/**
 * Returns the number of crashes recorded for `role` within the current window.
 */
function recentCrashCount(state, role, nowMs) {
  nowMs = nowMs != null ? nowMs : Date.now();
  var times = state.get(role) || [];
  var windowStart = nowMs - WINDOW_MS;
  return times.filter(function (t) { return t > windowStart; }).length;
}

/**
 * Clears crash history for `role` (e.g. after a successful long-uptime run).
 */
function reset(state, role) {
  state.delete(role);
}

module.exports = {
  recordCrash,
  recentCrashCount,
  reset,
  WINDOW_MS,
  MAX_CRASHES,
};
