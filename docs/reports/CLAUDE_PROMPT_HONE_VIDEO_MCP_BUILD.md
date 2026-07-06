# Claude Resume Prompt: HONE Video MCP Build

Use this prompt when Claude can work on the repo again.

```text
You are Claude continuing implementation in X:\hone.

Codex started the HONE video-generation MCP vertical. Read these files first:

- docs/reports/HONE_VIDEO_GENERATION_VERTICAL_STUDY.md
- docs/reports/HONE_MCP_FOR_CLAUDE_DESIGN.md
- docs/reports/CODEX_REVIEW_FOR_CLAUDE_2026-07-05.md
- src/mcp/honeMcpServer.js
- tests/honeMcpServer.test.js

What Codex built:

- Added `src/mcp/honeMcpServer.js`, a no-new-dependencies MCP-over-stdio server for HONE.
- Added productized video presets:
  - simple commercial
  - long-form commercial
  - TikTok / short vertical video
  - YouTube video
  - 30-minute TV-style show
  - full cinematic movie
- Added MCP tools:
  - `hone_capabilities_list`
  - `hone_video_presets_list`
  - `hone_video_plan_create`
  - `hone_video_job_quote`
  - `hone_video_job_post`
  - `hone_video_job_status`
  - `hone_video_artifact_get`
  - `hone_claude_handoff_prompt`
- Added `npm run mcp:hone` for local Claude/MCP clients.
- Added `tests/honeMcpServer.test.js`.
- Updated the video strategy notes to make the product direction explicit: HONE should be an A-to-Z video production tool, not a prompt box. The buyer should be able to arrive with rough intent and leave with a finished deliverable: brief, script, storyboard, generated assets, voice, music, edit, captions, revisions, final render, project bundle, storage, and settlement.
- Clarified the marketplace differentiator: HONE should coordinate local models, distributed GPU workers, human specialists, storage nodes, reviewers, milestones, escrow, disputes, and artifact provenance as one production supply chain.

Important safety boundary:

- Job posting is dry-run only.
- Live HONE spending is disabled unless `HONE_MCP_ALLOW_SPEND=1`, and even then the code currently refuses live posting until wallet-scoped auth and chain routes exist.
- Preserve the opt-in model: nodes advertise capabilities, Claude plans work, HONE executes and settles.

Verified by Codex:

- `npm test -- --runInBand tests/honeMcpServer.test.js` passed.
- `npm test -- --runInBand tests/toolRoutes.security.test.js tests/installerScripts.test.js` passed.
- Prior full suite with `--forceExit` passed before this MCP slice; rerun after your changes.

Next implementation targets:

1. Build the A-to-Z video job schema and lifecycle:
   - intake brief
   - approved creative plan
   - script
   - scene list
   - shot list
   - required assets
   - generation tasks
   - edit/assembly tasks
   - review checkpoints
   - revision requests
   - final delivery package
   - artifact CIDs/provenance
   - settlement state
2. Add a preset-to-plan compiler:
   - simple commercial
   - long-form commercial
   - TikTok / short vertical
   - YouTube
   - 30-minute TV-style show
   - cinematic movie
   - Each preset should expand user intent into a structured production plan without forcing the user to understand model details.
3. Add wallet-scoped auth for MCP actions:
   - scoped project token
   - max spend per job/day/project
   - explicit confirmation for spending and escrow release
4. Add chain/protocol entries or Rust API routes:
   - `MediaJobPost`
   - `MediaJobPlan`
   - `MediaAssetComplete`
   - `MediaJobAssemble`
   - `MediaJobAccept`
   - `MediaJobDispute`
   - `MediaJobPay`
5. Connect worker capability discovery:
   - video generation
   - voice synthesis
   - music generation
   - editing/assembly
   - upscaling
   - subtitles
   - storage
6. Connect local video generation through a provider adapter:
   - local model first
   - remote HONE worker later
   - external providers optional later
7. Connect live job posting only after auth exists.
8. Return real artifact CIDs from workers.
9. Keep subjective quality out of consensus; use buyer acceptance, reviewer market, reputation, and disputes.

Do not revert unrelated user/Codex work in the dirty tree. Keep changes additive and scoped.
```
