"use strict";

const honeMcp = require("../src/mcp/honeMcpServer");

describe("HONE MCP server", () => {
  test("lists productized video presets", () => {
    const out = honeMcp.listVideoPresets();
    const ids = out.presets.map(p => p.id);
    expect(ids).toContain("simple_commercial");
    expect(ids).toContain("tiktok_short");
    expect(ids).toContain("youtube_video");
    expect(ids).toContain("tv_style_show_30m");
    expect(ids).toContain("cinematic_movie");
  });

  test("creates a simple user-facing production plan", () => {
    const { plan } = honeMcp.createVideoPlan({
      preset: "tiktok_short",
      topic: "a roofing company that fixes storm damage fast",
      audience: "homeowners",
      duration_seconds: 30,
      call_to_action: "book a roof inspection",
    });

    expect(plan.preset).toBe("tiktok_short");
    expect(plan.production.aspect_ratio).toBe("9:16");
    expect(plan.production.scenes.length).toBeGreaterThan(0);
    expect(plan.production.required_capabilities).toContain("video_generation");
    expect(plan.brief.call_to_action).toBe("book a roof inspection");
  });

  test("quotes a plan with minimum, recommended, and premium prices", () => {
    const { plan } = honeMcp.createVideoPlan({
      preset: "simple_commercial",
      topic: "HONE node hosting",
      duration_seconds: 45,
    });
    const { quote } = honeMcp.quoteVideoJob({ plan, resolution: "1080p" });

    expect(quote.recommended_hunits).toBeGreaterThan(0);
    expect(quote.minimum_hunits).toBeLessThan(quote.recommended_hunits);
    expect(quote.premium_fast_hunits).toBeGreaterThan(quote.recommended_hunits);
  });

  test("dry-run post creates a job and status can retrieve it", () => {
    const { plan } = honeMcp.createVideoPlan({
      preset: "youtube_video",
      topic: "why distributed GPU rendering matters",
    });
    const { quote } = honeMcp.quoteVideoJob({ plan });
    const { job } = honeMcp.postVideoJob({ plan, quote, dry_run: true });
    const status = honeMcp.getVideoJobStatus({ job_id: job.job_id });

    expect(job.dry_run).toBe(true);
    expect(status.found).toBe(true);
    expect(status.job_id).toBe(job.job_id);
    expect(status.preset).toBe("youtube_video");
  });

  test("live posting is disabled by default", () => {
    const { plan } = honeMcp.createVideoPlan({ preset: "simple_commercial", topic: "test" });
    expect(() => honeMcp.postVideoJob({ plan, dry_run: false })).toThrow(/disabled|not wired/);
  });

  test("handles MCP initialize, list, and tool call JSON-RPC", async () => {
    const init = await honeMcp.handleJsonRpc({ jsonrpc: "2.0", id: 1, method: "initialize", params: {} });
    expect(init.result.serverInfo.name).toBe("hone-mcp");

    const list = await honeMcp.handleJsonRpc({ jsonrpc: "2.0", id: 2, method: "tools/list", params: {} });
    expect(list.result.tools.some(t => t.name === "hone_video_plan_create")).toBe(true);

    const call = await honeMcp.handleJsonRpc({
      jsonrpc: "2.0",
      id: 3,
      method: "tools/call",
      params: {
        name: "hone_video_plan_create",
        arguments: { preset: "simple_commercial", topic: "local video generation" },
      },
    });
    expect(call.result.structuredContent.plan.preset).toBe("simple_commercial");
  });

  test("returns a Claude handoff prompt", () => {
    const out = honeMcp.createClaudeHandoffPrompt();
    expect(out.prompt).toMatch(/Claude continuing HONE video-generation MCP work/);
    expect(out.prompt).toMatch(/src\/mcp\/honeMcpServer\.js/);
  });
});
