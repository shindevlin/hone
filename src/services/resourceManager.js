"use strict";

/**
 * BTCPC Resource Manager — Smart mining throttle
 * Shin Devlin
 *
 * Four layers of resource management:
 * 1. Auto-detect idle — full speed when user is away, throttle when active
 * 2. Manual toggle — /pause or --low flag
 * 3. CPU/GPU cap — max percentage (e.g. 50%)
 * 4. Schedule — cron-style active hours
 *
 * The miner checks getWorkItems() before each epoch to determine how many
 * inference jobs to run. Full mode = configured max. Reduced = 1 or 0.
 */

var { execSync } = require("child_process");
var platform = process.platform;
var { notifyThrottle } = require("./systemNotify");

// ── State ────────────────────────────────────────────────────────

var mode = "full";          // "full", "reduced", "paused"
var manualOverride = null;  // null = auto, "full", "reduced", "paused"
var maxCpuPercent = parseInt(process.env.BTCPC_MAX_CPU) || 100;
var maxGpuPercent = parseInt(process.env.BTCPC_MAX_GPU) || 100;
var idleThresholdMs = parseInt(process.env.BTCPC_IDLE_THRESHOLD_MS) || 120000; // 2 min
var fullWorkItems = parseInt(process.env.BTCPC_WORK_PER_EPOCH) || 3;
var reducedWorkItems = 1;
var lastMode = "full";

// Schedule: "HH:MM-HH:MM" for reduced hours (e.g. "09:00-17:00")
var scheduleReduced = process.env.BTCPC_REDUCED_HOURS || null;

// ── Idle Detection ───────────────────────────────────────────────

/**
 * Get user idle time in milliseconds.
 * Returns Infinity if can't detect (headless server = always "idle" = full speed).
 */
function getIdleTimeMs() {
  try {
    if (platform === "linux") {
      // xprintidle returns milliseconds since last input
      var ms = parseInt(execSync("xprintidle", { timeout: 2000, encoding: "utf8" }).trim());
      return isNaN(ms) ? Infinity : ms;
    } else if (platform === "darwin") {
      // ioreg returns nanoseconds
      var output = execSync(
        'ioreg -c IOHIDSystem | awk \'/HIDIdleTime/ {print $NF; exit}\'',
        { timeout: 2000, encoding: "utf8" }
      ).trim();
      var ns = parseInt(output);
      return isNaN(ns) ? Infinity : ns / 1000000;
    } else if (platform === "win32") {
      // PowerShell: get last input time
      var ps = 'Add-Type @"using System;using System.Runtime.InteropServices;public class IdleTime{[DllImport("user32.dll")]static extern bool GetLastInputInfo(ref LASTINPUTINFO p);public struct LASTINPUTINFO{public uint cbSize;public uint dwTime;}public static uint Get(){LASTINPUTINFO i=new LASTINPUTINFO();i.cbSize=(uint)Marshal.SizeOf(i);GetLastInputInfo(ref i);return((uint)Environment.TickCount-i.dwTime);}}";[IdleTime]::Get()';
      var msStr = execSync('powershell -command "' + ps.replace(/"/g, '\\"') + '"',
        { timeout: 5000, encoding: "utf8", windowsHide: true }).trim();
      var msWin = parseInt(msStr);
      return isNaN(msWin) ? Infinity : msWin;
    }
  } catch (e) {
    // Can't detect idle — treat as idle (full speed)
    return Infinity;
  }
  return Infinity;
}

/**
 * Check if user is idle (no keyboard/mouse for threshold).
 */
function isUserIdle() {
  return getIdleTimeMs() >= idleThresholdMs;
}

// ── Schedule ─────────────────────────────────────────────────────

/**
 * Check if current time is within reduced hours.
 * Format: "HH:MM-HH:MM" (24h). e.g. "09:00-17:00"
 */
function isReducedSchedule() {
  if (!scheduleReduced) return false;

  var parts = scheduleReduced.split("-");
  if (parts.length !== 2) return false;

  var now = new Date();
  var nowMinutes = now.getHours() * 60 + now.getMinutes();

  var startParts = parts[0].split(":");
  var endParts = parts[1].split(":");
  var start = parseInt(startParts[0]) * 60 + parseInt(startParts[1]);
  var end = parseInt(endParts[0]) * 60 + parseInt(endParts[1]);

  if (start < end) {
    return nowMinutes >= start && nowMinutes < end;
  } else {
    // Wraps midnight (e.g. "22:00-06:00")
    return nowMinutes >= start || nowMinutes < end;
  }
}

// ── Mode Calculation ─────────────────────────────────────────────

/**
 * Calculate the current mining mode based on all inputs.
 * Priority: manual override > schedule > idle detection
 */
function calculateMode() {
  // Manual override takes priority
  if (manualOverride) return manualOverride;

  // Schedule
  if (isReducedSchedule()) return "reduced";

  // Idle detection
  if (!isUserIdle()) return "reduced";

  return "full";
}

/**
 * Get the number of work items for this epoch.
 * Full mode = configured max. Reduced = 1. Paused = 0.
 */
function getWorkItems() {
  var currentMode = calculateMode();

  // Notify on mode change
  if (currentMode !== lastMode) {
    lastMode = currentMode;
    notifyThrottle(currentMode);
    console.log("[BTCPC] Mining mode: " + currentMode +
      (currentMode === "full" ? " (" + fullWorkItems + " work items)" :
       currentMode === "reduced" ? " (" + reducedWorkItems + " work item)" :
       " (paused)"));
  }

  mode = currentMode;

  switch (currentMode) {
    case "full": return fullWorkItems;
    case "reduced": return reducedWorkItems;
    case "paused": return 0;
    default: return fullWorkItems;
  }
}

/**
 * Get the current mode string.
 */
function getMode() {
  return calculateMode();
}

// ── Manual Controls ──────────────────────────────────────────────

/**
 * Set manual override. Pass null to return to auto.
 */
function setManualMode(newMode) {
  if (newMode === "auto" || newMode === null) {
    manualOverride = null;
    console.log("[BTCPC] Mining mode: auto (idle detection + schedule)");
  } else if (["full", "reduced", "paused"].includes(newMode)) {
    manualOverride = newMode;
    console.log("[BTCPC] Mining mode: " + newMode + " (manual override)");
  }
}

/**
 * Pause mining completely.
 */
function pause() {
  setManualMode("paused");
}

/**
 * Resume mining (return to auto mode).
 */
function resume() {
  setManualMode(null);
}

/**
 * Set max CPU percentage (affects Ollama process priority).
 */
function setCpuCap(percent) {
  maxCpuPercent = Math.max(10, Math.min(100, percent));
  console.log("[BTCPC] CPU cap: " + maxCpuPercent + "%");
}

/**
 * Set max GPU percentage.
 * Controls how much VRAM/compute Ollama can use.
 * Applied via CUDA_VISIBLE_DEVICES or Ollama's num_gpu parameter.
 */
function setGpuCap(percent) {
  maxGpuPercent = Math.max(0, Math.min(100, percent));
  console.log("[BTCPC] GPU cap: " + maxGpuPercent + "%");

  // Apply via Ollama environment — num_gpu layers
  // 0% = CPU only, 100% = all layers on GPU
  if (maxGpuPercent === 0) {
    process.env.OLLAMA_NUM_GPU = "0";
  } else {
    delete process.env.OLLAMA_NUM_GPU; // let Ollama use default (all)
  }
}

/**
 * Get the effective Ollama parameters based on current caps.
 * Miner should pass these to Ollama requests.
 */
function getOllamaOptions() {
  var opts = {};

  // GPU cap: control num_gpu (number of layers offloaded to GPU)
  // 100% = default (all), 50% = half layers, 0% = CPU only
  if (maxGpuPercent < 100) {
    // Approximate: most models have ~32-80 layers
    // Scale proportionally
    opts.num_gpu = Math.max(0, Math.round(40 * maxGpuPercent / 100));
  }

  // CPU cap: control num_thread
  if (maxCpuPercent < 100) {
    var os = require("os");
    var totalCores = os.cpus().length;
    opts.num_thread = Math.max(1, Math.round(totalCores * maxCpuPercent / 100));
  }

  return opts;
}

/**
 * Get status for display.
 */
function getStatus() {
  var idleMs = getIdleTimeMs();
  return {
    mode: calculateMode(),
    manualOverride: manualOverride,
    userIdle: idleMs >= idleThresholdMs,
    idleTimeMs: idleMs === Infinity ? null : idleMs,
    idleThresholdMs: idleThresholdMs,
    cpuCap: maxCpuPercent,
    gpuCap: maxGpuPercent,
    schedule: scheduleReduced,
    inScheduledReduction: isReducedSchedule(),
    workItems: getWorkItems(),
    fullWorkItems: fullWorkItems,
    reducedWorkItems: reducedWorkItems,
    ollamaOptions: getOllamaOptions()
  };
}

module.exports = {
  getWorkItems: getWorkItems,
  getMode: getMode,
  setManualMode: setManualMode,
  pause: pause,
  resume: resume,
  setCpuCap: setCpuCap,
  setGpuCap: setGpuCap,
  getOllamaOptions: getOllamaOptions,
  getStatus: getStatus,
  isUserIdle: isUserIdle
};
