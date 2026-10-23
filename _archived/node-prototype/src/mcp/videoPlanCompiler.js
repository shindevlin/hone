"use strict";

/**
 * Preset → plan compiler for the HONE video vertical.
 *
 * A "plan" is the compiled production graph for a video product. Given a preset
 * (simple commercial, TikTok, YouTube, TV show, cinematic movie, …) plus rough
 * buyer intent, this module deterministically expands it into:
 *
 *   - a creative brief
 *   - a resolved production spec (duration, aspect ratio, scene count)
 *   - an ordered milestone list, each milestone carrying:
 *       * the concrete deliverable KINDS it produces (from the schema vocab)
 *       * the worker CAPABILITIES it requires
 *       * its dependency edges (depends_on) onto earlier milestones
 *   - a scene breakdown
 *   - an aggregate list of required capabilities / worker roles
 *
 * "Deterministic" matters: the same input must compile to the same plan_id so
 * quotes and jobs are reproducible and a plan can be re-derived on any node.
 * Nothing here does I/O or reads the clock.
 *
 * The compiler is the single place that knows how a preset decomposes into
 * work. The MCP server, the quote engine, and the job schema all consume its
 * output rather than re-deriving milestones independently.
 */

const crypto = require("crypto");

// Milestone name → the deliverable kinds it yields + the capabilities it needs.
// Deliverable kinds must be members of mediaJobSchema.DELIVERABLE_KINDS.
const MILESTONE_SPECS = Object.freeze({
  brief: { deliverable_kinds: ["creative_brief"], capabilities: ["text_inference"] },
  hook: { deliverable_kinds: ["creative_brief", "script"], capabilities: ["text_inference"] },
  outline: { deliverable_kinds: ["script"], capabilities: ["text_inference"] },
  treatment: { deliverable_kinds: ["creative_brief", "script"], capabilities: ["text_inference"] },
  show_bible: { deliverable_kinds: ["creative_brief"], capabilities: ["text_inference"] },
  episode_outline: { deliverable_kinds: ["script"], capabilities: ["text_inference"] },
  script: { deliverable_kinds: ["script"], capabilities: ["text_inference"] },
  storyboard: { deliverable_kinds: ["storyboard", "shot_list"], capabilities: ["image_generation"] },
  scene_batch: { deliverable_kinds: ["generated_scene"], capabilities: ["video_generation"] },
  scene_batches: { deliverable_kinds: ["generated_scene"], capabilities: ["video_generation"] },
  act_batches: { deliverable_kinds: ["generated_scene"], capabilities: ["video_generation"] },
  asset_batch: {
    deliverable_kinds: ["generated_scene", "voiceover", "music"],
    capabilities: ["video_generation", "voice_synthesis", "music_generation"],
  },
  render: {
    deliverable_kinds: ["generated_scene", "voiceover", "edit_assembly"],
    capabilities: ["video_generation", "voice_synthesis", "editing_assembly"],
  },
  captions: { deliverable_kinds: ["captions"], capabilities: ["subtitles"] },
  thumbnail: { deliverable_kinds: ["generated_scene"], capabilities: ["image_generation"] },
  assembly: { deliverable_kinds: ["edit_assembly"], capabilities: ["editing_assembly"] },
  grade_mix: {
    deliverable_kinds: ["color_grade", "sound_design"],
    capabilities: ["editing_assembly", "music_generation"],
  },
  final: {
    deliverable_kinds: ["final_render", "project_bundle", "provenance"],
    capabilities: ["editing_assembly", "storage"],
  },
});

/**
 * Compile a preset + intent into a full plan.
 *
 * @param {object} presets  The VIDEO_PRESETS map from the MCP server.
 * @param {object} input    Raw user intent (preset, topic, audience, …).
 * @returns {{ plan: object }}
 */
function compilePlan(presets, input) {
  const args = input || {};
  const preset = resolvePreset(presets, args.preset || args.type || "simple_commercial");

  const duration = clampNumber(
    numberOrDefault(args.duration_seconds, midpoint(preset.duration_seconds)),
    preset.duration_seconds[0],
    preset.duration_seconds[1]
  );
  const aspectRatio = preset.aspect_ratios.includes(args.aspect_ratio)
    ? args.aspect_ratio
    : preset.default_aspect_ratio;
  const sceneCount = estimateSceneCount(preset, duration);

  const topic = cleanText(
    args.topic || args.product || args.story || args.goal || "Untitled HONE video project",
    500
  );
  const audience = cleanText(args.audience || "general audience", 240);
  const tone = cleanText(args.tone || args.style || "clear, polished, commercially useful", 240);
  const callToAction = cleanText(args.call_to_action || args.cta || "learn more", 160);
  const resolution = cleanText(args.resolution || "1080p", 24);
  const budgetHunits = numberOrDefault(args.max_budget_hunits || args.budget_hunits, null);

  const milestones = compileMilestones(preset);
  const scenes = buildScenes(preset, sceneCount, topic, tone, callToAction);
  const requiredCapabilities = aggregateCapabilities(preset, milestones);
  const workerRoles = requiredCapabilities.map(capabilityToWorkerRole);

  const plan = {
    plan_id: stableId("plan", {
      preset: preset.id,
      topic,
      audience,
      tone,
      duration,
      aspectRatio,
      sceneCount,
    }),
    preset: preset.id,
    // Video presets compile to the "video" render modality. The job schema
    // reads this to generalize across image/audio/3D/composite renders (Wiiv).
    modality: "video",
    compiler_version: 1,
    title: makeTitle(preset, topic),
    brief: {
      topic,
      audience,
      tone,
      call_to_action: callToAction,
      safety_constraints: normalizeList(args.safety_constraints),
      references: normalizeList(args.references),
    },
    production: {
      duration_seconds: duration,
      aspect_ratio: aspectRatio,
      resolution,
      scene_count: sceneCount,
      required_capabilities: requiredCapabilities,
      worker_roles: workerRoles,
      milestones,
      scenes,
    },
    budget: {
      requested_max_hunits: budgetHunits,
      quote_required: true,
    },
    status: "planned",
  };
  return { plan };
}

/**
 * Expand a preset's milestone names into fully-specified milestone objects with
 * deliverable kinds, capabilities, and a linear dependency chain (each milestone
 * depends on the one before it — long-form presets can be re-parallelized later
 * once worker discovery exists, but a linear chain is the safe default).
 */
function compileMilestones(preset) {
  const names = preset.milestones || [];
  return names.map((name, index) => {
    const spec = MILESTONE_SPECS[name] || { deliverable_kinds: [], capabilities: [] };
    const id = `${index + 1}-${name}`;
    return {
      id,
      name,
      deliverable: milestoneDeliverable(name, preset),
      deliverable_kinds: spec.deliverable_kinds.slice(),
      required_capabilities: spec.capabilities.slice(),
      depends_on: index === 0 ? [] : [`${index}-${names[index - 1]}`],
    };
  });
}

/**
 * Union of the preset's declared required_capabilities and everything the
 * compiled milestones actually need, stable-ordered by first appearance.
 */
function aggregateCapabilities(preset, milestones) {
  const seen = new Set();
  const ordered = [];
  const add = cap => {
    if (cap && !seen.has(cap)) {
      seen.add(cap);
      ordered.push(cap);
    }
  };
  (preset.required_capabilities || []).forEach(add);
  for (const m of milestones) {
    (m.required_capabilities || []).forEach(add);
  }
  return ordered;
}

function capabilityToWorkerRole(capability) {
  const map = {
    text_inference: "writer_model",
    image_generation: "image_worker",
    video_generation: "video_worker",
    voice_synthesis: "voice_worker",
    music_generation: "music_worker",
    editing_assembly: "editor_worker",
    upscaling: "upscale_worker",
    subtitles: "caption_worker",
    storage: "storage_node",
    review: "reviewer",
  };
  return { capability, role: map[capability] || capability };
}

// ── scene + title helpers (kept behaviourally identical to the first slice) ──

function buildScenes(preset, count, topic, tone, cta) {
  const scenes = [];
  for (let i = 0; i < count; i++) {
    const role = sceneRole(preset, i, count);
    scenes.push({
      scene_id: `s${String(i + 1).padStart(2, "0")}`,
      role,
      prompt: `${role} for ${topic}; tone: ${tone}`,
      audio: i === count - 1 ? `end with call to action: ${cta}` : "voiceover or natural sound as needed",
    });
  }
  return scenes;
}

function sceneRole(preset, index, total) {
  if (index === 0) return preset.id === "tiktok_short" ? "hook" : "opening";
  if (index === total - 1) return "call_to_action";
  if (preset.id === "cinematic_movie" || preset.id === "tv_style_show_30m") return `story beat ${index}`;
  if (index < total / 2) return "problem_or_setup";
  return "proof_or_solution";
}

function milestoneDeliverable(name, preset) {
  const map = {
    brief: "creative brief",
    hook: "opening hook and caption direction",
    treatment: "narrative treatment",
    outline: "structured outline",
    script: "script draft",
    storyboard: "storyboard and shot list",
    scene_batch: "generated scene batch",
    scene_batches: "generated scene batches",
    act_batches: "episode act batches",
    asset_batch: "visual/audio asset batch",
    render: "preview render",
    captions: "caption pass",
    thumbnail: "thumbnail concepts",
    assembly: "edited assembly",
    grade_mix: "color grade and audio mix",
    final: `final ${preset.label} package`,
    show_bible: "show bible",
    episode_outline: "episode outline",
  };
  return map[name] || name.replace(/_/g, " ");
}

function makeTitle(preset, topic) {
  const shortTopic = topic.length > 80 ? topic.slice(0, 77) + "..." : topic;
  return `${preset.label}: ${shortTopic}`;
}

// ── numeric + text helpers ──────────────────────────────────────────────────

function resolvePreset(presets, id) {
  const preset = presets && presets[id];
  if (!preset) throw new Error("unknown video preset: " + id);
  return preset;
}

function estimateSceneCount(preset, duration) {
  const target = Math.round(duration / (preset.id === "tiktok_short" ? 6 : 12));
  return clampNumber(target, preset.scene_count[0], preset.scene_count[1]);
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

function normalizeList(value) {
  if (!value) return [];
  if (Array.isArray(value)) return value.map(v => cleanText(v, 300)).filter(Boolean);
  return [cleanText(value, 300)].filter(Boolean);
}

function cleanText(value, max) {
  return String(value || "").replace(/\s+/g, " ").trim().slice(0, max);
}

function stableId(prefix, value) {
  const hash = crypto.createHash("sha256").update(JSON.stringify(value)).digest("hex").slice(0, 16);
  return `${prefix}_${hash}`;
}

module.exports = {
  MILESTONE_SPECS,
  compilePlan,
  compileMilestones,
  aggregateCapabilities,
  capabilityToWorkerRole,
};
