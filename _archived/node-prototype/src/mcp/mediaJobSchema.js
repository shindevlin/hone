"use strict";

/**
 * Canonical A-to-Z media job schema and lifecycle for the HONE video vertical.
 *
 * This module is the single source of truth for the *shape* of a HONE media
 * production job and the *rules* by which it moves through its lifecycle. It is
 * deliberately pure (no I/O, no chain calls, no wall-clock beyond an injectable
 * `now`) so that:
 *
 *   1. The MCP server can build and validate job documents in-process today.
 *   2. The same document shape can be serialized into a chain-backed
 *      `MediaJobPost` ledger entry later without a second schema.
 *   3. Tests can drive the state machine deterministically.
 *
 * A HONE media job models the full production supply chain — brief, script,
 * storyboard, generated assets, voice, music, edit, captions, revisions, final
 * render, project bundle, storage, and settlement — as an ordered list of
 * milestones, each carrying deliverable slots and the worker capabilities it
 * needs. Subjective quality is intentionally NOT a consensus input: a milestone
 * is "accepted" by the buyer (or a reviewer market), never by chain validators.
 */

const crypto = require("crypto");

const SCHEMA_VERSION = 1;

// ── Lifecycle states ────────────────────────────────────────────────────────
//
// A job document has one top-level `status`. Milestones have their own smaller
// state machine (see MILESTONE_STATES) so long-form work can settle piecewise.

const JOB_STATES = Object.freeze({
  DRAFTED: "drafted", // plan compiled, not yet quoted/funded
  QUOTED: "quoted", // a quote is attached; awaiting funding decision
  FUNDED: "funded", // escrow committed (dry-run: simulated); ready to start
  IN_PRODUCTION: "in_production", // at least one milestone is active
  IN_REVIEW: "in_review", // all milestones delivered; awaiting buyer acceptance
  ACCEPTED: "accepted", // buyer accepted the full deliverable
  SETTLED: "settled", // escrow released to workers; terminal success
  DISPUTED: "disputed", // buyer opened a dispute; reviewer market engaged
  CANCELLED: "cancelled", // cancelled before settlement; terminal
});

// Legal top-level transitions. Anything not listed is rejected by `transition`.
const JOB_TRANSITIONS = Object.freeze({
  [JOB_STATES.DRAFTED]: [JOB_STATES.QUOTED, JOB_STATES.CANCELLED],
  [JOB_STATES.QUOTED]: [JOB_STATES.FUNDED, JOB_STATES.DRAFTED, JOB_STATES.CANCELLED],
  [JOB_STATES.FUNDED]: [JOB_STATES.IN_PRODUCTION, JOB_STATES.CANCELLED],
  [JOB_STATES.IN_PRODUCTION]: [JOB_STATES.IN_REVIEW, JOB_STATES.DISPUTED, JOB_STATES.CANCELLED],
  [JOB_STATES.IN_REVIEW]: [JOB_STATES.ACCEPTED, JOB_STATES.IN_PRODUCTION, JOB_STATES.DISPUTED],
  [JOB_STATES.ACCEPTED]: [JOB_STATES.SETTLED, JOB_STATES.DISPUTED],
  [JOB_STATES.DISPUTED]: [JOB_STATES.IN_PRODUCTION, JOB_STATES.SETTLED, JOB_STATES.CANCELLED],
  [JOB_STATES.SETTLED]: [],
  [JOB_STATES.CANCELLED]: [],
});

const TERMINAL_JOB_STATES = Object.freeze([JOB_STATES.SETTLED, JOB_STATES.CANCELLED]);

// ── Milestone states ────────────────────────────────────────────────────────

const MILESTONE_STATES = Object.freeze({
  PENDING: "pending", // not started; may be blocked by dependencies
  ACTIVE: "active", // a worker is engaged
  DELIVERED: "delivered", // artifact submitted; awaiting acceptance
  ACCEPTED: "accepted", // buyer/reviewer accepted this milestone
  REVISION_REQUESTED: "revision_requested", // sent back with notes
  REJECTED: "rejected", // terminal fail for this milestone
});

const MILESTONE_TRANSITIONS = Object.freeze({
  [MILESTONE_STATES.PENDING]: [MILESTONE_STATES.ACTIVE],
  [MILESTONE_STATES.ACTIVE]: [MILESTONE_STATES.DELIVERED, MILESTONE_STATES.REJECTED],
  [MILESTONE_STATES.DELIVERED]: [
    MILESTONE_STATES.ACCEPTED,
    MILESTONE_STATES.REVISION_REQUESTED,
    MILESTONE_STATES.REJECTED,
  ],
  [MILESTONE_STATES.REVISION_REQUESTED]: [MILESTONE_STATES.ACTIVE],
  [MILESTONE_STATES.ACCEPTED]: [],
  [MILESTONE_STATES.REJECTED]: [],
});

// ── Deliverable classes ─────────────────────────────────────────────────────
//
// The A-to-Z artifact vocabulary. A finished job's artifact bundle is drawn
// from these; the compiler decides which a given preset actually requires.

const DELIVERABLE_KINDS = Object.freeze([
  "creative_brief",
  "script",
  "storyboard",
  "shot_list",
  "generated_scene",
  "generated_image",
  "generated_model",
  "voiceover",
  "music",
  "sound_design",
  "edit_assembly",
  "captions",
  "color_grade",
  "upscale",
  "final_render",
  "project_bundle",
  "provenance",
]);

// Wiiv render modalities. A job targets one; the protocol is identical across
// them (see docs/WIIV_PROTOCOL.md). Kept in sync with rust/wiiv RenderModality.
const RENDER_MODALITIES = Object.freeze(["image", "video", "audio", "threed", "composite"]);

// ── Construction ────────────────────────────────────────────────────────────

/**
 * Build a canonical, validated media job document from a compiled plan.
 *
 * @param {object} opts
 * @param {object} opts.plan            A plan from the preset→plan compiler.
 * @param {object} [opts.quote]         An optional attached quote.
 * @param {number} [opts.maxBudgetHunits] Buyer spend cap for this job.
 * @param {string} [opts.buyer]         Buyer account (never trusted for auth here).
 * @param {boolean} [opts.dryRun=true]  Dry-run jobs never touch chain/escrow.
 * @param {() => number} [opts.now]     Injectable clock for deterministic tests.
 * @returns {{ job: object }}
 */
function createMediaJob(opts) {
  const args = opts || {};
  if (!args.plan || typeof args.plan !== "object") {
    throw new Error("createMediaJob requires a compiled plan");
  }
  const now = typeof args.now === "function" ? args.now : Date.now;
  const dryRun = args.dryRun !== false;
  const plan = args.plan;

  const milestones = (plan.production && Array.isArray(plan.production.milestones)
    ? plan.production.milestones
    : []
  ).map((m, index) => ({
    milestone_id: m.id || `${index + 1}-milestone`,
    name: m.name || `milestone_${index + 1}`,
    deliverable: m.deliverable || "",
    deliverable_kinds: Array.isArray(m.deliverable_kinds) ? m.deliverable_kinds.slice() : [],
    required_capabilities: Array.isArray(m.required_capabilities) ? m.required_capabilities.slice() : [],
    depends_on: Array.isArray(m.depends_on) ? m.depends_on.slice() : [],
    status: MILESTONE_STATES.PENDING,
    artifacts: [],
    revisions: 0,
  }));

  const timestamp = new Date(now()).toISOString();
  const job = {
    schema_version: SCHEMA_VERSION,
    kind: "hone.media_job",
    job_id: stableId("media-job", {
      plan_id: plan.plan_id,
      preset: plan.preset,
      buyer: args.buyer || null,
    }),
    status: JOB_STATES.DRAFTED,
    dry_run: dryRun,
    buyer: args.buyer || null,
    // Render modality — a video preset compiles to modality "video"; the field
    // generalizes the job to image/audio/3D/composite renders (Wiiv). Defaults to
    // the plan's modality, else video for back-compat with the first slice.
    modality: RENDER_MODALITIES.includes(plan.modality) ? plan.modality : "video",
    preset: plan.preset,
    title: plan.title || "Untitled HONE media job",
    plan,
    quote: args.quote || null,
    budget: {
      max_hunits: numberOrNull(args.maxBudgetHunits),
      committed_hunits: 0,
      released_hunits: 0,
    },
    revision_policy: normalizeRevisionPolicy(args.revision_policy),
    milestones,
    artifacts: [],
    history: [
      { at: timestamp, from: null, to: JOB_STATES.DRAFTED, note: "job drafted from compiled plan" },
    ],
    created_at: timestamp,
    updated_at: timestamp,
  };

  const problems = validateMediaJob(job);
  if (problems.length) {
    throw new Error("invalid media job document: " + problems.join("; "));
  }
  return { job };
}

function normalizeRevisionPolicy(policy) {
  const p = policy && typeof policy === "object" ? policy : {};
  return {
    included_revisions: numberOrDefault(p.included_revisions, 1),
    max_revisions: numberOrDefault(p.max_revisions, 3),
    revision_hunits: numberOrDefault(p.revision_hunits, 0),
  };
}

// ── Validation ──────────────────────────────────────────────────────────────

/**
 * Structural validation. Returns an array of human-readable problems; empty
 * means valid. Kept strict enough to be a real gate for a future chain entry
 * but tolerant of forward-compatible extra fields.
 *
 * @param {object} job
 * @returns {string[]}
 */
function validateMediaJob(job) {
  const problems = [];
  if (!job || typeof job !== "object") return ["job is not an object"];

  if (job.schema_version !== SCHEMA_VERSION) {
    problems.push(`schema_version must be ${SCHEMA_VERSION}`);
  }
  if (job.kind !== "hone.media_job") problems.push("kind must be 'hone.media_job'");
  if (typeof job.job_id !== "string" || !job.job_id) problems.push("job_id is required");
  if (!Object.values(JOB_STATES).includes(job.status)) {
    problems.push(`status '${job.status}' is not a known job state`);
  }
  if (typeof job.dry_run !== "boolean") problems.push("dry_run must be a boolean");
  if (job.modality !== undefined && !RENDER_MODALITIES.includes(job.modality)) {
    problems.push(`modality '${job.modality}' is not a known render modality`);
  }
  if (!job.plan || typeof job.plan !== "object") problems.push("plan is required");

  const budget = job.budget || {};
  const maxH = budget.max_hunits;
  if (maxH !== null && maxH !== undefined && (!Number.isFinite(maxH) || maxH < 0)) {
    problems.push("budget.max_hunits must be null or a non-negative number");
  }
  if (Number.isFinite(budget.committed_hunits) && Number.isFinite(maxH) && budget.committed_hunits > maxH) {
    problems.push("budget.committed_hunits exceeds max_hunits");
  }

  if (!Array.isArray(job.milestones)) {
    problems.push("milestones must be an array");
  } else {
    const ids = new Set();
    for (const m of job.milestones) {
      if (!m || typeof m !== "object") {
        problems.push("milestone entries must be objects");
        continue;
      }
      if (!m.milestone_id) problems.push("each milestone needs a milestone_id");
      if (ids.has(m.milestone_id)) problems.push(`duplicate milestone_id: ${m.milestone_id}`);
      ids.add(m.milestone_id);
      if (!Object.values(MILESTONE_STATES).includes(m.status)) {
        problems.push(`milestone ${m.milestone_id} has unknown status '${m.status}'`);
      }
      for (const kind of m.deliverable_kinds || []) {
        if (!DELIVERABLE_KINDS.includes(kind)) {
          problems.push(`milestone ${m.milestone_id} references unknown deliverable kind '${kind}'`);
        }
      }
    }
    // Dependencies must reference real milestones.
    for (const m of job.milestones) {
      for (const dep of (m && m.depends_on) || []) {
        if (!ids.has(dep)) problems.push(`milestone ${m.milestone_id} depends on unknown '${dep}'`);
      }
    }
  }

  return problems;
}

// ── Transitions ─────────────────────────────────────────────────────────────

/**
 * Apply a top-level job state transition, returning a new job document.
 * Rejects illegal transitions. Pure: does not mutate the input.
 *
 * @param {object} job
 * @param {string} nextStatus
 * @param {object} [meta]
 * @param {string} [meta.note]
 * @param {() => number} [meta.now]
 * @returns {{ job: object }}
 */
function transitionJob(job, nextStatus, meta) {
  if (!job || typeof job !== "object") throw new Error("transitionJob requires a job");
  const from = job.status;
  const allowed = JOB_TRANSITIONS[from];
  if (!allowed) throw new Error(`unknown current job status '${from}'`);
  if (!allowed.includes(nextStatus)) {
    throw new Error(`illegal job transition ${from} -> ${nextStatus}`);
  }
  const m = meta || {};
  const now = typeof m.now === "function" ? m.now : Date.now;
  const at = new Date(now()).toISOString();
  const next = {
    ...job,
    status: nextStatus,
    updated_at: at,
    history: [...(job.history || []), { at, from, to: nextStatus, note: m.note || "" }],
  };
  return { job: next };
}

/**
 * Apply a milestone-level transition, returning a new job document.
 * Enforces the milestone state machine and dependency gating (a milestone may
 * only become ACTIVE once all `depends_on` milestones are ACCEPTED).
 *
 * @param {object} job
 * @param {string} milestoneId
 * @param {string} nextStatus
 * @param {object} [meta] { note, artifact, now }
 * @returns {{ job: object }}
 */
function transitionMilestone(job, milestoneId, nextStatus, meta) {
  if (!job || typeof job !== "object") throw new Error("transitionMilestone requires a job");
  const idx = (job.milestones || []).findIndex(m => m.milestone_id === milestoneId);
  if (idx < 0) throw new Error(`no such milestone: ${milestoneId}`);
  const milestone = job.milestones[idx];
  const from = milestone.status;
  const allowed = MILESTONE_TRANSITIONS[from];
  if (!allowed) throw new Error(`unknown milestone status '${from}'`);
  if (!allowed.includes(nextStatus)) {
    throw new Error(`illegal milestone transition ${from} -> ${nextStatus} for ${milestoneId}`);
  }

  if (nextStatus === MILESTONE_STATES.ACTIVE) {
    const unmet = (milestone.depends_on || []).filter(dep => {
      const d = job.milestones.find(x => x.milestone_id === dep);
      return !d || d.status !== MILESTONE_STATES.ACCEPTED;
    });
    if (unmet.length) {
      throw new Error(`milestone ${milestoneId} blocked by unaccepted dependencies: ${unmet.join(", ")}`);
    }
  }

  const m = meta || {};
  const now = typeof m.now === "function" ? m.now : Date.now;
  const at = new Date(now()).toISOString();

  const updatedMilestone = {
    ...milestone,
    status: nextStatus,
    artifacts:
      m.artifact && nextStatus === MILESTONE_STATES.DELIVERED
        ? [...milestone.artifacts, m.artifact]
        : milestone.artifacts,
    revisions:
      nextStatus === MILESTONE_STATES.REVISION_REQUESTED ? milestone.revisions + 1 : milestone.revisions,
  };

  const milestones = job.milestones.slice();
  milestones[idx] = updatedMilestone;

  const next = {
    ...job,
    milestones,
    updated_at: at,
    history: [
      ...(job.history || []),
      { at, milestone_id: milestoneId, from, to: nextStatus, note: m.note || "" },
    ],
  };
  return { job: next };
}

/** True when every milestone is in a terminal-accepted or rejected state. */
function allMilestonesResolved(job) {
  return (job.milestones || []).every(
    m => m.status === MILESTONE_STATES.ACCEPTED || m.status === MILESTONE_STATES.REJECTED
  );
}

/** True when every milestone has been accepted. */
function allMilestonesAccepted(job) {
  const ms = job.milestones || [];
  return ms.length > 0 && ms.every(m => m.status === MILESTONE_STATES.ACCEPTED);
}

// ── helpers ─────────────────────────────────────────────────────────────────

function stableId(prefix, value) {
  const hash = crypto.createHash("sha256").update(JSON.stringify(value)).digest("hex").slice(0, 16);
  return `${prefix}_${hash}`;
}

function numberOrNull(value) {
  const n = Number(value);
  return Number.isFinite(n) ? n : null;
}

function numberOrDefault(value, fallback) {
  const n = Number(value);
  return Number.isFinite(n) ? n : fallback;
}

module.exports = {
  SCHEMA_VERSION,
  JOB_STATES,
  JOB_TRANSITIONS,
  TERMINAL_JOB_STATES,
  MILESTONE_STATES,
  MILESTONE_TRANSITIONS,
  DELIVERABLE_KINDS,
  RENDER_MODALITIES,
  createMediaJob,
  validateMediaJob,
  transitionJob,
  transitionMilestone,
  allMilestonesResolved,
  allMilestonesAccepted,
};
