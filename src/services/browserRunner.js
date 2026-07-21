"use strict";

/**
 * Browser runner for computer-use jobs.
 * Shin Devlin
 *
 * Wraps Playwright to execute browser actions for computer-use marketplace jobs.
 * ALWAYS runs headless — browser sessions belong to buyers and showing them
 * would expose private activity to the miner. The miner's screen is never used.
 *
 * URL filtering is enforced before every navigation via browserContentFilter.
 * Hard blocks: SSRF/private IPs, CSAM patterns, non-http protocols.
 * Soft blocks: miner-configured (onion, adult, gambling, drugs, weapons).
 */

const path = require("path");
const fs = require("fs");
const crypto = require("crypto");
const os = require("os");

const HEADLESS = true; // always — buyer sessions are private; miner must not watch
const BLOB_DIR = process.env.HONE_BLOB_DIR || path.resolve(__dirname, "../../data/blobs");
const { assertUrlAllowed } = require("./browserContentFilter");
const SCREENSHOT_QUALITY = 80; // JPEG quality for screenshots
const DEFAULT_TIMEOUT_MS = 10000;

// Playwright is an optional dependency — miners who don't want browser jobs don't need it
function loadPlaywright() {
  try {
    return require("playwright");
  } catch (_) {
    return null;
  }
}

class BrowserSession {
  constructor(jobId, opts) {
    this.jobId = jobId;
    this.opts = opts || {};
    this.browser = null;
    this.page = null;
    this.viewport = opts.viewport || { width: 1280, height: 800 };
    this.closed = false;
  }

  async launch() {
    const pw = loadPlaywright();
    if (!pw) {
      throw new Error(
        "Playwright not installed. Run: npm install playwright && npx playwright install chromium"
      );
    }
    this.browser = await pw.chromium.launch({
      headless: HEADLESS,
      args: ["--no-sandbox", "--disable-setuid-sandbox"],
    });
    const ctx = await this.browser.newContext({
      viewport: this.viewport,
      userAgent: "Mozilla/5.0 (compatible; HONEBrowserAgent/1.0)",
    });
    this.page = await ctx.newPage();
    console.log(`[BrowserRunner] Job ${this.jobId} — launched (headless: ${HEADLESS})`);
  }

  async navigate(url) {
    // Content filter runs on every navigation — hard blocks (SSRF, CSAM) + miner soft blocks
    assertUrlAllowed(url);
    await this.page.goto(url, { waitUntil: "domcontentloaded", timeout: DEFAULT_TIMEOUT_MS });
  }

  async takeScreenshot() {
    const buf = await this.page.screenshot({ type: "jpeg", quality: SCREENSHOT_QUALITY });
    // Store as HONE-FS blob: sha256(content) → filename
    const cid = crypto.createHash("sha256").update(buf).digest("hex");
    if (!fs.existsSync(BLOB_DIR)) {
      fs.mkdirSync(BLOB_DIR, { recursive: true });
    }
    fs.writeFileSync(path.join(BLOB_DIR, cid), buf);
    return cid;
  }

  async getAccessibilityTree() {
    try {
      // Summarize interactive elements for the agent
      const elements = await this.page.evaluate(() => {
        const sel = "a[href], button, input, select, textarea, [role='button'], [role='link']";
        return Array.from(document.querySelectorAll(sel)).slice(0, 50).map((el) => ({
          tag: el.tagName.toLowerCase(),
          role: el.getAttribute("role") || el.tagName.toLowerCase(),
          text: (el.innerText || el.value || el.getAttribute("aria-label") || "").slice(0, 80),
          id: el.id || null,
          name: el.getAttribute("name") || null,
        }));
      });
      return elements;
    } catch (_) {
      return [];
    }
  }

  async executeAction(action) {
    const { action_type, coordinate, text, selector, scroll_direction, scroll_amount, wait_ms } = action;

    switch (action_type) {
      case "click":
        if (selector) {
          await this.page.click(selector, { timeout: DEFAULT_TIMEOUT_MS });
        } else if (coordinate) {
          await this.page.mouse.click(coordinate.x, coordinate.y);
        }
        break;

      case "type":
        if (selector) {
          await this.page.fill(selector, text || "", { timeout: DEFAULT_TIMEOUT_MS });
        } else if (coordinate) {
          await this.page.mouse.click(coordinate.x, coordinate.y);
          await this.page.keyboard.type(text || "");
        } else {
          await this.page.keyboard.type(text || "");
        }
        break;

      case "navigate":
        assertUrlAllowed(text || "");
        await this.page.goto(text || "", { waitUntil: "domcontentloaded", timeout: DEFAULT_TIMEOUT_MS });
        break;

      case "scroll":
        if (coordinate) {
          await this.page.mouse.move(coordinate.x, coordinate.y);
        }
        const delta = (scroll_amount || 300) * (scroll_direction === "up" ? -1 : 1);
        await this.page.mouse.wheel(0, delta);
        break;

      case "key":
        await this.page.keyboard.press(text || "Enter");
        break;

      case "wait":
        await new Promise((r) => setTimeout(r, Math.min(wait_ms || 1000, 5000)));
        break;

      case "done":
        break;

      default:
        throw new Error(`Unknown action type: ${action_type}`);
    }

    // Brief settle time after action
    await new Promise((r) => setTimeout(r, 300));
  }

  currentUrl() {
    return this.page ? this.page.url() : "";
  }

  async pageTitle() {
    return this.page ? this.page.title() : "";
  }

  async close() {
    if (this.closed) return;
    this.closed = true;
    try {
      if (this.browser) await this.browser.close();
    } catch (_) {}
    console.log(`[BrowserRunner] Job ${this.jobId} — browser closed`);
  }
}

// Active sessions keyed by job_id
const activeSessions = new Map();

async function createSession(jobId, opts) {
  if (activeSessions.has(jobId)) {
    throw new Error(`Session already exists for job ${jobId}`);
  }
  const session = new BrowserSession(jobId, opts);
  await session.launch();
  activeSessions.set(jobId, session);
  return session;
}

function getSession(jobId) {
  return activeSessions.get(jobId) || null;
}

async function closeSession(jobId) {
  const session = activeSessions.get(jobId);
  if (session) {
    await session.close();
    activeSessions.delete(jobId);
  }
}

/**
 * Delete screenshot blobs stored locally for a job.
 * Screenshots belong to the buyer — once transmitted (as CIDs in turn history),
 * the miner's local copy must be removed. Called at job settlement/close.
 */
function deleteSessionBlobs(screenshotCids) {
  if (!Array.isArray(screenshotCids)) return;
  for (const cid of screenshotCids) {
    if (!/^[a-f0-9]{64}$/.test(cid)) continue;
    const blobPath = path.join(BLOB_DIR, cid);
    try {
      fs.unlinkSync(blobPath);
    } catch (_) {
      // already gone or never written — not an error
    }
  }
}

function isPlaywrightAvailable() {
  return loadPlaywright() !== null;
}

module.exports = {
  BrowserSession,
  createSession,
  getSession,
  closeSession,
  deleteSessionBlobs,
  isPlaywrightAvailable,
};
