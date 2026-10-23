"use strict";

/**
 * HONE MCP server for LLM orchestration.
 *
 * This is the first safe slice of the HONE-as-MCP surface: it lets Claude or
 * another MCP client plan video products, quote them, dry-run post jobs, and
 * inspect local job state. Live spending is intentionally disabled by default.
 */

const http = require("http");
const https = require("https");

const { compilePlan } = require("./videoPlanCompiler");
const mediaJob = require("./mediaJobSchema");

const SERVER_NAME = "hone-mcp";
const SERVER_VERSION = "0.1.0";
const DEFAULT_NODE_URL = process.env.HONE_MCP_NODE_URL || process.env.HONE_NODE_URL || "http://127.0.0.1:4242";
const LIVE_POSTING_ENABLED = process.env.HONE_MCP_ALLOW_SPEND === "1";

const CAPABILITIES = [
  { id: "text_inference", label: "Text inference", required: false },
  { id: "image_generation", label: "Image generation", required: false },
  { id: "video_generation", label: "Video generation", required: false },
  { id: "voice_synthesis", label: "Voice synthesis", required: false },
  { id: "music_generation", label: "Music generation", required: false },
  { id: "editing_assembly", label: "Editing and assembly", required: false },
  { id: "upscaling", label: "Upscaling", required: false },
  { id: "subtitles", label: "Subtitles", required: false },
  { id: "storage", label: "Artifact storage", required: false },
  { id: "review", label: "Review and dispute support", required: false },
];

const VIDEO_PRESETS = {
  simple_commercial: {
    id: "simple_commercial",
    label: "Simple commercial",
    duration_seconds: [15, 60],
    aspect_ratios: ["16:9", "9:16", "1:1"],
    default_aspect_ratio: "16:9",
    scene_count: [3, 8],
    milestones: ["brief", "script", "render", "final"],
    required_capabilities: ["video_generation", "voice_synthesis", "editing_assembly", "subtitles"],
    base_hunits: 2_500_000,
  },
  long_form_commercial: {
    id: "long_form_commercial",
    label: "Long-form commercial",
    duration_seconds: [120, 600],
    aspect_ratios: ["16:9", "9:16"],
    default_aspect_ratio: "16:9",
    scene_count: [8, 30],
    milestones: ["brief", "treatment", "script", "scene_batch", "assembly", "final"],
    required_capabilities: ["video_generation", "voice_synthesis", "music_generation", "editing_assembly", "subtitles"],
    base_hunits: 12_000_000,
  },
  tiktok_short: {
    id: "tiktok_short",
    label: "TikTok / short vertical video",
    duration_seconds: [8, 60],
    aspect_ratios: ["9:16"],
    default_aspect_ratio: "9:16",
    scene_count: [3, 10],
    milestones: ["hook", "script", "render", "captions", "final"],
    required_capabilities: ["video_generation", "voice_synthesis", "editing_assembly", "subtitles"],
    base_hunits: 2_000_000,
  },
  youtube_video: {
    id: "youtube_video",
    label: "YouTube video",
    duration_seconds: [300, 1200],
    aspect_ratios: ["16:9"],
    default_aspect_ratio: "16:9",
    scene_count: [10, 60],
    milestones: ["outline", "script", "asset_batch", "assembly", "thumbnail", "final"],
    required_capabilities: ["video_generation", "voice_synthesis", "music_generation", "editing_assembly", "subtitles"],
    base_hunits: 20_000_000,
  },
  tv_style_show_30m: {
    id: "tv_style_show_30m",
    label: "30-minute TV-style show",
    duration_seconds: [1500, 2100],
    aspect_ratios: ["16:9"],
    default_aspect_ratio: "16:9",
    scene_count: [40, 120],
    milestones: ["show_bible", "episode_outline", "script", "act_batches", "assembly", "final"],
    required_capabilities: ["video_generation", "voice_synthesis", "music_generation", "editing_assembly", "subtitles", "storage"],
    base_hunits: 90_000_000,
  },
  cinematic_movie: {
    id: "cinematic_movie",
    label: "Full cinematic movie",
    duration_seconds: [3600, 7200],
    aspect_ratios: ["16:9", "2.39:1"],
    default_aspect_ratio: "2.39:1",
    scene_count: [80, 240],
    milestones: ["treatment", "script", "storyboard", "scene_batches", "assembly", "grade_mix", "final"],
    required_capabilities: ["video_generation", "voice_synthesis", "music_generation", "editing_assembly", "upscaling", "subtitles", "storage", "review"],
    base_hunits: 350_000_000,
  },
};

const jobs = new Map();

function listCapabilities() {
  return {
    network: "hone",
    node_url: DEFAULT_NODE_URL,
    live_posting_enabled: LIVE_POSTING_ENABLED,
    capabilities: CAPABILITIES,
  };
}

function listVideoPresets() {
  return {
    presets: Object.values(VIDEO_PRESETS).map(p => ({
      id: p.id,
      label: p.label,
      duration_seconds: p.duration_seconds,
      aspect_ratios: p.aspect_ratios,
      default_aspect_ratio: p.default_aspect_ratio,
      required_capabilities: p.required_capabilities,
      milestones: p.milestones,
    })),
  };
}

function createVideoPlan(input) {
  // The preset→plan compiler is the single source of truth for how a preset
  // decomposes into a production graph (milestones, deliverable kinds,
  // capabilities, worker roles, dependencies). See videoPlanCompiler.js.
  return compilePlan(VIDEO_PRESETS, input || {});
}

function quoteVideoJob(input) {
  const plan = input && input.plan ? input.plan : createVideoPlan(input).plan;
  const preset = getPreset(plan.preset);
  const duration = numberOrDefault(plan.production && plan.production.duration_seconds, midpoint(preset.duration_seconds));
  const sceneCount = numberOrDefault(plan.production && plan.production.scene_count, estimateSceneCount(preset, duration));
  const resolutionMultiplier = resolutionFactor(input && input.resolution || plan.production && plan.production.resolution);
  const complexity = 1 + Math.max(0, sceneCount - midpoint(preset.scene_count)) / Math.max(1, preset.scene_count[1]);
  const durationFactor = duration / midpoint(preset.duration_seconds);
  const recommended = Math.round(preset.base_hunits * durationFactor * complexity * resolutionMultiplier);
  const minimum = Math.round(recommended * 0.65);
  const premium = Math.round(recommended * 1.85);
  const storageDays = numberOrDefault(input && input.storage_days, 30);

  return {
    quote: {
      preset: preset.id,
      minimum_hunits: minimum,
      recommended_hunits: recommended,
      premium_fast_hunits: premium,
      estimated_completion_minutes: estimateMinutes(preset, duration, sceneCount),
      storage_days: storageDays,
      assumptions: [
        "quote is an MVP estimate until live worker bids are connected",
        "long-form projects should settle by milestone",
        "subjective quality remains buyer/reviewer acceptance, not consensus",
      ],
    },
  };
}

function postVideoJob(input) {
  const args = input || {};
  const dryRun = args.dry_run !== false;
  const plan = args.plan || createVideoPlan(args).plan;
  const quote = args.quote || quoteVideoJob({ plan }).quote;
  const maxBudget = numberOrDefault(args.max_budget_hunits, plan.budget && plan.budget.requested_max_hunits);

  // ── Safety boundary — do NOT weaken without scoped auth + spend caps ──
  // Live posting stays disabled behind BOTH an env opt-in and a hard "not
  // wired yet" stop, so even HONE_MCP_ALLOW_SPEND=1 cannot move real value
  // until wallet-scoped auth and chain MediaJobPost routes exist.
  if (!dryRun && !LIVE_POSTING_ENABLED) {
    throw new Error("live HONE spending is disabled; set HONE_MCP_ALLOW_SPEND=1 after adding wallet-scoped auth");
  }
  if (!dryRun) {
    throw new Error("live media job posting is not wired to chain routes yet; use dry_run while protocol entries are added");
  }

  // Build the canonical A-to-Z job document. This is the shape a future
  // chain-backed MediaJobPost entry will serialize.
  const { job: canonical } = mediaJob.createMediaJob({
    plan,
    quote,
    maxBudgetHunits: maxBudget,
    buyer: cleanText(args.buyer || "", 128) || null,
    revision_policy: args.revision_policy,
    dryRun: true,
  });

  const job = {
    ...canonical,
    next_step: "review plan and quote, then implement live MediaJobPost route with scoped wallet auth",
  };
  jobs.set(job.job_id, job);
  return { job };
}

/**
 * Apply a lifecycle transition to a dry-run job held in this process. Job-level
 * transitions are legal-checked by the schema; milestone transitions require a
 * milestone_id. This lets Claude walk a job through its lifecycle end-to-end in
 * dry-run before any chain-backed jobs exist.
 */
function advanceVideoJob(input) {
  const args = input || {};
  const jobId = args.job_id;
  if (!jobId || !jobs.has(jobId)) {
    return { found: false, job_id: jobId || null, note: "unknown job in this MCP process" };
  }
  const current = jobs.get(jobId);
  let updated;
  if (args.milestone_id) {
    ({ job: updated } = mediaJob.transitionMilestone(current, args.milestone_id, args.to, {
      note: cleanText(args.note || "", 400),
      artifact: args.artifact,
    }));
  } else {
    ({ job: updated } = mediaJob.transitionJob(current, args.to, {
      note: cleanText(args.note || "", 400),
    }));
  }
  jobs.set(updated.job_id, updated);
  return {
    found: true,
    job_id: updated.job_id,
    status: updated.status,
    milestones: updated.milestones.map(m => ({ milestone_id: m.milestone_id, status: m.status })),
    all_milestones_accepted: mediaJob.allMilestonesAccepted(updated),
  };
}

/** Describe the lifecycle state machine so clients can reason about legal moves. */
function describeJobLifecycle() {
  return {
    job_states: mediaJob.JOB_STATES,
    job_transitions: mediaJob.JOB_TRANSITIONS,
    terminal_states: mediaJob.TERMINAL_JOB_STATES,
    milestone_states: mediaJob.MILESTONE_STATES,
    milestone_transitions: mediaJob.MILESTONE_TRANSITIONS,
    deliverable_kinds: mediaJob.DELIVERABLE_KINDS,
    note: "Subjective quality is buyer/reviewer acceptance, never chain consensus.",
  };
}

function getVideoJobStatus(input) {
  const jobId = input && input.job_id;
  if (!jobId || !jobs.has(jobId)) {
    return {
      found: false,
      job_id: jobId || null,
      status: "unknown",
      note: "Only jobs created in this MCP process are available until chain-backed media jobs are implemented.",
    };
  }
  const job = jobs.get(jobId);
  return {
    found: true,
    job_id: job.job_id,
    status: job.status,
    dry_run: job.dry_run,
    preset: job.plan.preset,
    title: job.plan.title || (job.plan && job.plan.title),
    quote: job.quote,
    milestones: (job.milestones || []).map(m => ({
      milestone_id: m.milestone_id,
      name: m.name,
      status: m.status,
      required_capabilities: m.required_capabilities,
    })),
    next_step: job.next_step,
    artifacts: job.artifacts,
  };
}

function getVideoArtifact(input) {
  const jobId = input && input.job_id;
  if (!jobId || !jobs.has(jobId)) {
    return { found: false, job_id: jobId || null, artifacts: [] };
  }
  const job = jobs.get(jobId);
  return {
    found: true,
    job_id: job.job_id,
    artifacts: job.artifacts,
    expected_artifacts: [
      "creative_brief.json",
      "script.md",
      "shot_list.json",
      "preview.mp4",
      "final.mp4",
      "captions.vtt",
      "provenance.json",
    ],
    note: "Dry-run jobs do not produce artifacts. Live workers will return CIDs here.",
  };
}

function createClaudeHandoffPrompt() {
  return {
    prompt: [
      "You are Claude continuing HONE video-generation MCP work.",
      "Read docs/reports/HONE_VIDEO_GENERATION_VERTICAL_STUDY.md and docs/reports/HONE_MCP_FOR_CLAUDE_DESIGN.md.",
      "The MCP server is src/mcp/honeMcpServer.js; the canonical A-to-Z job schema + lifecycle is src/mcp/mediaJobSchema.js; the preset->plan compiler is src/mcp/videoPlanCompiler.js.",
      "Done: canonical media-job document shape, job + milestone lifecycle state machines with legal-transition enforcement, and the deterministic preset->plan compiler (milestones carry deliverable kinds, capabilities, worker roles, and dependencies).",
      "Preserve the opt-in design and the safety boundary: dry-run only, live spending gated behind HONE_MCP_ALLOW_SPEND=1 AND a hard 'not wired yet' stop until wallet-scoped auth + chain routes exist.",
      "Next implementation targets: wallet-scoped MCP auth and spend caps, chain-backed MediaJobPost entries + media API routes, worker capability discovery, a provider adapter for local video generation, and live artifact CIDs.",
    ].join("\n"),
  };
}

const TOOL_HANDLERS = {
  hone_capabilities_list: listCapabilities,
  hone_video_presets_list: listVideoPresets,
  hone_video_plan_create: createVideoPlan,
  hone_video_job_quote: quoteVideoJob,
  hone_video_job_post: postVideoJob,
  hone_video_job_status: getVideoJobStatus,
  hone_video_job_advance: advanceVideoJob,
  hone_video_job_lifecycle: describeJobLifecycle,
  hone_video_artifact_get: getVideoArtifact,
  hone_claude_handoff_prompt: createClaudeHandoffPrompt,
};

function listTools() {
  return [
    tool("hone_capabilities_list", "List HONE capability classes and local MCP safety state.", {}),
    tool("hone_video_presets_list", "List productized video presets such as commercials, TikToks, YouTube videos, shows, and movies.", {}),
    tool("hone_video_plan_create", "Create a structured HONE video production plan from user intent.", {
      preset: { type: "string" },
      topic: { type: "string" },
      audience: { type: "string" },
      duration_seconds: { type: "number" },
      aspect_ratio: { type: "string" },
      tone: { type: "string" },
      call_to_action: { type: "string" },
      max_budget_hunits: { type: "number" },
      references: { type: "array", items: { type: "string" } },
      safety_constraints: { type: "array", items: { type: "string" } },
    }),
    tool("hone_video_job_quote", "Estimate cost and completion time for a video plan.", {
      plan: { type: "object" },
      resolution: { type: "string" },
      storage_days: { type: "number" },
    }),
    tool("hone_video_job_post", "Create a safe dry-run media job record. Live spending is disabled until wallet-scoped auth is wired.", {
      plan: { type: "object" },
      quote: { type: "object" },
      max_budget_hunits: { type: "number" },
      dry_run: { type: "boolean" },
    }),
    tool("hone_video_job_status", "Get status for a media job created by this MCP process.", {
      job_id: { type: "string" },
    }),
    tool("hone_video_job_advance", "Advance a dry-run job or one of its milestones through the lifecycle state machine. Rejects illegal transitions.", {
      job_id: { type: "string" },
      to: { type: "string" },
      milestone_id: { type: "string" },
      note: { type: "string" },
      artifact: { type: "object" },
    }),
    tool("hone_video_job_lifecycle", "Describe the canonical media-job lifecycle: states, legal transitions, and deliverable kinds.", {}),
    tool("hone_video_artifact_get", "Get artifact metadata for a media job.", {
      job_id: { type: "string" },
    }),
    tool("hone_claude_handoff_prompt", "Return a prompt for Claude to continue this implementation.", {}),
  ];
}

function tool(name, description, properties) {
  return {
    name,
    description,
    inputSchema: {
      type: "object",
      additionalProperties: true,
      properties,
    },
  };
}

async function callTool(name, args) {
  const handler = TOOL_HANDLERS[name];
  if (!handler) throw new Error("unknown tool: " + name);
  return await handler(args || {});
}

async function handleJsonRpc(message) {
  if (!message || message.jsonrpc !== "2.0") {
    return errorResponse(message && message.id, -32600, "invalid JSON-RPC request");
  }
  if (!Object.prototype.hasOwnProperty.call(message, "id")) {
    return null;
  }

  try {
    if (message.method === "initialize") {
      return resultResponse(message.id, {
        protocolVersion: "2024-11-05",
        capabilities: { tools: {} },
        serverInfo: { name: SERVER_NAME, version: SERVER_VERSION },
      });
    }
    if (message.method === "tools/list") {
      return resultResponse(message.id, { tools: listTools() });
    }
    if (message.method === "tools/call") {
      const params = message.params || {};
      const output = await callTool(params.name, params.arguments || {});
      return resultResponse(message.id, {
        content: [{ type: "text", text: JSON.stringify(output, null, 2) }],
        structuredContent: output,
      });
    }
    return errorResponse(message.id, -32601, "method not found: " + message.method);
  } catch (err) {
    return errorResponse(message.id, -32000, err.message);
  }
}

function resultResponse(id, result) {
  return { jsonrpc: "2.0", id, result };
}

function errorResponse(id, code, message) {
  return { jsonrpc: "2.0", id: id === undefined ? null : id, error: { code, message } };
}

function runStdio() {
  let buffer = Buffer.alloc(0);
  process.stdin.on("data", chunk => {
    buffer = Buffer.concat([buffer, chunk]);
    processBuffer().catch(err => {
      writeJsonRpc(errorResponse(null, -32000, err.message));
    });
  });

  async function processBuffer() {
    while (buffer.length > 0) {
      const headerEnd = buffer.indexOf("\r\n\r\n");
      if (headerEnd >= 0) {
        const header = buffer.slice(0, headerEnd).toString("utf8");
        const match = /Content-Length:\s*(\d+)/i.exec(header);
        if (!match) throw new Error("missing Content-Length");
        const length = parseInt(match[1], 10);
        const start = headerEnd + 4;
        if (buffer.length < start + length) return;
        const raw = buffer.slice(start, start + length).toString("utf8");
        buffer = buffer.slice(start + length);
        await dispatchRaw(raw);
        continue;
      }

      const newline = buffer.indexOf("\n");
      if (newline < 0) return;
      const rawLine = buffer.slice(0, newline).toString("utf8").trim();
      buffer = buffer.slice(newline + 1);
      if (rawLine) await dispatchRaw(rawLine);
    }
  }

  async function dispatchRaw(raw) {
    const request = JSON.parse(raw);
    const response = await handleJsonRpc(request);
    if (response) writeJsonRpc(response);
  }
}

function writeJsonRpc(message) {
  const body = JSON.stringify(message);
  process.stdout.write("Content-Length: " + Buffer.byteLength(body, "utf8") + "\r\n\r\n" + body);
}

function getPreset(id) {
  const preset = VIDEO_PRESETS[id];
  if (!preset) throw new Error("unknown video preset: " + id);
  return preset;
}

function midpoint(range) {
  return Math.round((range[0] + range[1]) / 2);
}

function numberOrDefault(value, fallback) {
  const n = Number(value);
  return Number.isFinite(n) ? n : fallback;
}

function clampNumber(value, min, max) {
  return Math.max(min, Math.min(max, Math.round(value)));
}

function cleanText(value, max) {
  return String(value || "").replace(/\s+/g, " ").trim().slice(0, max);
}

// Scene-count estimate is still used by the quote engine; scene/title/
// milestone text generation now lives in videoPlanCompiler.js.
function estimateSceneCount(preset, duration) {
  const target = Math.round(duration / (preset.id === "tiktok_short" ? 6 : 12));
  return clampNumber(target, preset.scene_count[0], preset.scene_count[1]);
}

function resolutionFactor(resolution) {
  const value = String(resolution || "1080p").toLowerCase();
  if (value.includes("4k") || value.includes("2160")) return 2.5;
  if (value.includes("720")) return 0.75;
  return 1.0;
}

function estimateMinutes(preset, duration, sceneCount) {
  const base = Math.ceil(duration / 10 + sceneCount * 2);
  if (preset.id === "tv_style_show_30m") return Math.max(base, 360);
  if (preset.id === "cinematic_movie") return Math.max(base, 1440);
  return Math.max(base, 10);
}

function requestNodeJson(path) {
  const url = new URL(path, DEFAULT_NODE_URL);
  const client = url.protocol === "https:" ? https : http;
  return new Promise((resolve, reject) => {
    const req = client.request(url, { method: "GET", timeout: 2000 }, res => {
      let raw = "";
      res.on("data", chunk => { raw += chunk; });
      res.on("end", () => {
        try { resolve(JSON.parse(raw)); } catch (_) { resolve(null); }
      });
    });
    req.on("error", reject);
    req.on("timeout", () => { req.destroy(); reject(new Error("node request timeout")); });
    req.end();
  });
}

module.exports = {
  SERVER_NAME,
  SERVER_VERSION,
  VIDEO_PRESETS,
  listCapabilities,
  listVideoPresets,
  createVideoPlan,
  quoteVideoJob,
  postVideoJob,
  getVideoJobStatus,
  advanceVideoJob,
  describeJobLifecycle,
  getVideoArtifact,
  createClaudeHandoffPrompt,
  listTools,
  callTool,
  handleJsonRpc,
  runStdio,
  requestNodeJson,
};

if (require.main === module) {
  runStdio();
}
