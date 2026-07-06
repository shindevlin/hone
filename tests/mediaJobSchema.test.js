"use strict";

const schema = require("../src/mcp/mediaJobSchema");
const { compilePlan } = require("../src/mcp/videoPlanCompiler");
const honeMcp = require("../src/mcp/honeMcpServer");

// A fixed clock makes job_id / timestamps deterministic in tests.
const fixedNow = () => 1783191600000;

function planFor(preset, extra) {
  return compilePlan(honeMcp.VIDEO_PRESETS, Object.assign({ preset, topic: "test topic" }, extra || {}));
}

describe("media job schema + lifecycle", () => {
  test("createMediaJob produces a valid canonical document", () => {
    const { plan } = planFor("tiktok_short");
    const { job } = schema.createMediaJob({ plan, now: fixedNow });

    expect(job.kind).toBe("hone.media_job");
    expect(job.schema_version).toBe(schema.SCHEMA_VERSION);
    expect(job.status).toBe(schema.JOB_STATES.DRAFTED);
    expect(job.dry_run).toBe(true);
    expect(job.milestones.length).toBeGreaterThan(0);
    expect(schema.validateMediaJob(job)).toEqual([]);
  });

  test("job_id is deterministic for the same plan and buyer", () => {
    const { plan } = planFor("simple_commercial");
    const a = schema.createMediaJob({ plan, buyer: "alice", now: fixedNow }).job;
    const b = schema.createMediaJob({ plan, buyer: "alice", now: fixedNow }).job;
    expect(a.job_id).toBe(b.job_id);
  });

  test("validateMediaJob rejects a bad schema version and unknown deliverable kind", () => {
    const { plan } = planFor("simple_commercial");
    const { job } = schema.createMediaJob({ plan, now: fixedNow });

    const badVersion = Object.assign({}, job, { schema_version: 999 });
    expect(schema.validateMediaJob(badVersion)).toContain("schema_version must be 1");

    const badKind = JSON.parse(JSON.stringify(job));
    badKind.milestones[0].deliverable_kinds = ["not_a_real_kind"];
    expect(schema.validateMediaJob(badKind).join(" ")).toMatch(/unknown deliverable kind/);
  });

  test("legal job transitions are allowed, illegal ones throw", () => {
    const { plan } = planFor("simple_commercial");
    let { job } = schema.createMediaJob({ plan, now: fixedNow });

    ({ job } = schema.transitionJob(job, schema.JOB_STATES.QUOTED, { now: fixedNow }));
    expect(job.status).toBe(schema.JOB_STATES.QUOTED);
    ({ job } = schema.transitionJob(job, schema.JOB_STATES.FUNDED, { now: fixedNow }));
    ({ job } = schema.transitionJob(job, schema.JOB_STATES.IN_PRODUCTION, { now: fixedNow }));
    expect(job.status).toBe(schema.JOB_STATES.IN_PRODUCTION);

    // drafted -> settled is not a legal edge
    const fresh = schema.createMediaJob({ plan, now: fixedNow }).job;
    expect(() => schema.transitionJob(fresh, schema.JOB_STATES.SETTLED)).toThrow(/illegal job transition/);
  });

  test("milestone dependency gating blocks activating a downstream milestone early", () => {
    const { plan } = planFor("simple_commercial");
    const { job } = schema.createMediaJob({ plan, now: fixedNow });
    const first = job.milestones[0].milestone_id;
    const second = job.milestones[1].milestone_id;

    // Second milestone depends on the first; cannot go active until first accepted.
    expect(() => schema.transitionMilestone(job, second, schema.MILESTONE_STATES.ACTIVE)).toThrow(
      /blocked by unaccepted dependencies/
    );

    // Walk the first milestone to accepted, then the second may activate.
    let updated = job;
    ({ job: updated } = schema.transitionMilestone(updated, first, schema.MILESTONE_STATES.ACTIVE, { now: fixedNow }));
    ({ job: updated } = schema.transitionMilestone(updated, first, schema.MILESTONE_STATES.DELIVERED, {
      now: fixedNow,
      artifact: { kind: "creative_brief", cid: "bafyfake" },
    }));
    ({ job: updated } = schema.transitionMilestone(updated, first, schema.MILESTONE_STATES.ACCEPTED, { now: fixedNow }));
    ({ job: updated } = schema.transitionMilestone(updated, second, schema.MILESTONE_STATES.ACTIVE, { now: fixedNow }));

    const secondM = updated.milestones.find(m => m.milestone_id === second);
    expect(secondM.status).toBe(schema.MILESTONE_STATES.ACTIVE);
    const firstM = updated.milestones.find(m => m.milestone_id === first);
    expect(firstM.artifacts.length).toBe(1);
  });

  test("revision requests increment the milestone revision counter", () => {
    const { plan } = planFor("tiktok_short");
    const { job } = schema.createMediaJob({ plan, now: fixedNow });
    const id = job.milestones[0].milestone_id;
    let updated = job;
    ({ job: updated } = schema.transitionMilestone(updated, id, schema.MILESTONE_STATES.ACTIVE, { now: fixedNow }));
    ({ job: updated } = schema.transitionMilestone(updated, id, schema.MILESTONE_STATES.DELIVERED, { now: fixedNow }));
    ({ job: updated } = schema.transitionMilestone(updated, id, schema.MILESTONE_STATES.REVISION_REQUESTED, {
      now: fixedNow,
      note: "tighten the hook",
    }));
    const m = updated.milestones.find(x => x.milestone_id === id);
    expect(m.revisions).toBe(1);
    expect(m.status).toBe(schema.MILESTONE_STATES.REVISION_REQUESTED);
  });

  test("transitions do not mutate the input document", () => {
    const { plan } = planFor("simple_commercial");
    const { job } = schema.createMediaJob({ plan, now: fixedNow });
    const before = JSON.stringify(job);
    schema.transitionJob(job, schema.JOB_STATES.QUOTED, { now: fixedNow });
    expect(JSON.stringify(job)).toBe(before);
  });
});

describe("preset -> plan compiler", () => {
  test("compiles milestones with deliverable kinds, capabilities, and dependencies", () => {
    const { plan } = compilePlan(honeMcp.VIDEO_PRESETS, {
      preset: "cinematic_movie",
      topic: "a lighthouse keeper's last night",
    });
    const ms = plan.production.milestones;
    expect(ms.length).toBeGreaterThan(0);
    // first milestone has no dependency; later ones chain to the prior.
    expect(ms[0].depends_on).toEqual([]);
    expect(ms[1].depends_on).toEqual([ms[0].id]);
    // every milestone declares kinds + capabilities.
    for (const m of ms) {
      expect(Array.isArray(m.deliverable_kinds)).toBe(true);
      expect(Array.isArray(m.required_capabilities)).toBe(true);
    }
    // aggregate capabilities + worker roles are present.
    expect(plan.production.required_capabilities).toContain("video_generation");
    expect(plan.production.worker_roles.some(r => r.role === "video_worker")).toBe(true);
  });

  test("compilation is deterministic (same input -> same plan_id)", () => {
    const a = compilePlan(honeMcp.VIDEO_PRESETS, { preset: "youtube_video", topic: "x" }).plan;
    const b = compilePlan(honeMcp.VIDEO_PRESETS, { preset: "youtube_video", topic: "x" }).plan;
    expect(a.plan_id).toBe(b.plan_id);
  });

  test("every compiled deliverable kind is a member of the schema vocabulary", () => {
    for (const presetId of Object.keys(honeMcp.VIDEO_PRESETS)) {
      const { plan } = compilePlan(honeMcp.VIDEO_PRESETS, { preset: presetId, topic: "t" });
      for (const m of plan.production.milestones) {
        for (const kind of m.deliverable_kinds) {
          expect(schema.DELIVERABLE_KINDS).toContain(kind);
        }
      }
    }
  });
});

describe("MCP server lifecycle integration", () => {
  test("a dry-run job can be advanced through job and milestone states", () => {
    const { plan } = honeMcp.createVideoPlan({ preset: "simple_commercial", topic: "hone node hosting" });
    const { job } = honeMcp.postVideoJob({ plan, dry_run: true });
    expect(job.status).toBe(schema.JOB_STATES.DRAFTED);

    const quoted = honeMcp.advanceVideoJob({ job_id: job.job_id, to: schema.JOB_STATES.QUOTED });
    expect(quoted.status).toBe(schema.JOB_STATES.QUOTED);

    const firstMilestone = job.milestones[0].milestone_id;
    const active = honeMcp.advanceVideoJob({
      job_id: job.job_id,
      milestone_id: firstMilestone,
      to: schema.MILESTONE_STATES.ACTIVE,
    });
    expect(active.found).toBe(true);
    expect(active.milestones.find(m => m.milestone_id === firstMilestone).status).toBe(
      schema.MILESTONE_STATES.ACTIVE
    );
  });

  test("advancing an unknown job reports not found rather than throwing", () => {
    const out = honeMcp.advanceVideoJob({ job_id: "media-job_deadbeef", to: schema.JOB_STATES.QUOTED });
    expect(out.found).toBe(false);
  });

  test("lifecycle description exposes states and transitions", () => {
    const out = honeMcp.describeJobLifecycle();
    expect(out.job_states.DRAFTED).toBe("drafted");
    expect(out.job_transitions.drafted).toContain("quoted");
    expect(out.deliverable_kinds).toContain("final_render");
  });

  test("hone_video_job_lifecycle and hone_video_job_advance are registered tools", async () => {
    const list = await honeMcp.handleJsonRpc({ jsonrpc: "2.0", id: 1, method: "tools/list", params: {} });
    const names = list.result.tools.map(t => t.name);
    expect(names).toContain("hone_video_job_lifecycle");
    expect(names).toContain("hone_video_job_advance");
  });
});

describe("Wiiv render modality generalization", () => {
  test("video presets compile to modality 'video' and the job carries it", () => {
    const { plan } = planFor("simple_commercial");
    expect(plan.modality).toBe("video");
    const { job } = schema.createMediaJob({ plan, now: fixedNow });
    expect(job.modality).toBe("video");
  });

  test("RENDER_MODALITIES covers image/video/audio/3D/composite", () => {
    expect(schema.RENDER_MODALITIES).toEqual(
      expect.arrayContaining(["image", "video", "audio", "threed", "composite"])
    );
  });

  test("validation rejects an unknown modality", () => {
    const { plan } = planFor("tiktok_short");
    const { job } = schema.createMediaJob({ plan, now: fixedNow });
    const bad = Object.assign({}, job, { modality: "hologram" });
    expect(schema.validateMediaJob(bad).join(" ")).toMatch(/not a known render modality/);
  });

  test("deliverable vocabulary includes image and 3D kinds for non-video renders", () => {
    expect(schema.DELIVERABLE_KINDS).toContain("generated_image");
    expect(schema.DELIVERABLE_KINDS).toContain("generated_model");
  });
});
