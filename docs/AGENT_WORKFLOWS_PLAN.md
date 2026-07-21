---
title: HONE Agent Workflows — plan & build
description: What the agent-workflow engine is today, the multi-step-workflow gap, and the plan to make agent workflows run non-fragile on live hone
author: Shin Devlin
status: plan → building
---

# Agent Workflows via the Inference Engine

**Shin:** "agent workflows via the engine? plan them, build them, push them. use grouchly."

## 1. What exists today (verified 2026-07-04)
HONE already has a real agent layer — more built than the verticals:

- **Agent sessions** (`agent_session.rs`): persistent, encrypted, tool-capable AI
  sessions. Open → turns (ReAct) → close. Rolling 50-turn history + summary.
- **Agent task marketplace** (`agent_task.rs`, `/api/agent/task/*`):
  post/bid/assign/submit/verify. Agents do paid work for each other, with
  commit-reveal verification.
- **ReAct tool loop** (`agent_tools.rs`): a worker runs the model, parses
  `{"tool":...}` calls, executes them, feeds results back. Up to 8 iterations.
  Tools: `chain_read` (query the chain), `web_search` (DuckDuckGo), `code_exec`
  (sandboxed 30s).
- **Orchestrator** (`rust/hone-orchestrator`, `hone-orchestratord`): control
  plane — RuntimeJob/Worker/Attempt/Span/Attestation with signed attestations +
  verifier challenges. Runs API-tool jobs end-to-end today.
- **Registry + credits** (`agent_registry.rs`): agents register, hold credit,
  get paid.

**All of it runs on inference** — `agent_worker.rs::run_task` calls the model
(currently external Ollama at `/api/generate` + `/api/chat`).

## 2. The two real gaps
1. **Fragile inference.** Agent workflows call Ollama over HTTP — the same
   daemon-down failure that 503'd bullship. Fixed by the embedded-candle
   migration (`inference_engine.rs`): agent_worker becomes a caller of the
   unified engine, so workflows run in-process, no daemon.
2. **Single task, not multi-step workflows.** Today an agent task is one ReAct
   loop (≤8 tools) producing one output. A *workflow* chains tasks: output of
   step A feeds step B, with fan-out/verify/synthesize. The orchestrator has
   jobs but not workflow *composition* (its `sequence` is retry, not steps).

## 3. What we build
### Build 1 — Agent workflows run on the embedded engine (de-fragilize)
Rewrite `agent_worker.rs::run_task`'s two Ollama HTTP calls to use
`inference_engine::chat`. Result: agent tasks + sessions run on embedded candle
(or an external INFERENCE_URL), never on a separate daemon. This is part of the
inference migration (call sites 4-5 of ~6) and directly makes agent workflows
non-fragile.

### Build 2 — Multi-step workflow primitive
A workflow = an ordered set of agent tasks where a step can consume prior steps'
outputs. Minimal, chain-native design:
- New entry `AgentWorkflowPost { workflow_id, steps: Vec<WorkflowStep>, ... }`
  where a `WorkflowStep` = `{ task_template, depends_on: Vec<step_idx>,
  input_from: Vec<step_idx> }`.
- The orchestrator schedules a step once its `depends_on` steps have verified
  attestations, substituting their outputs into the step's input.
- Reuses the existing task post/bid/verify/settle machinery per step — a workflow
  is just a DAG *over* tasks, not a new execution engine.
- Fan-out (multiple independent steps) + join (a step depending on several) fall
  out of the DAG for free.
- Rewards: each step settles as a normal AgentTask; the workflow poster escrows
  the sum.

### Build 3 — A reference workflow that EARNS (the proof)
End-to-end on live hone: post a 3-step workflow (e.g. `web_search` →
`chain_read`/synthesize → `code_exec`/format), have a worker node execute each
step on embedded candle, verify, and settle rewards. This proves "agent workflows
via the engine" is real and earning — the same "vertical earns" bar as the
scoreboard.

## 4. Division of labor
- **Beastly:** Build 1 (wire agent_worker to inference_engine) + Build 2 (workflow
  entry + orchestrator DAG scheduling) + the code for Build 3. Push.
- **Grouchly:** run a worker node (phone/Nebra/his node) that CLAIMS + executes
  the reference workflow steps live, and verify the reward settles on-chain.
  Report to SCOREBOARD. This is the live-runtime half only he can do.

## 5. Scoreboard milestone
Add **#7 — Agent workflows earn**: a multi-step agent workflow runs on the
embedded engine and settles rewards on live hone. Owner: beastly (build) +
grouchly (live execution).

## 5b. AgentWorkflow DAG — precise design (Build 2, spec-before-code)

A workflow is a **DAG over the existing agent-task machinery** — NOT a new
execution engine. Each step becomes a normal `AgentTaskPost` when its
dependencies have verified; step outputs substitute into dependent step inputs.

### New ledger entry (crates/hone-types/src/entry.rs)
```rust
AgentWorkflowPost {
    workflow_id:  String,
    requester:    String,
    steps:        Vec<WorkflowStep>,   // ordered; index = step id
    max_fee:      u64,                  // total escrow = sum of step fees
    deadline_epoch: u64,
    epoch:        u64,
    nonce:        u64,
    signed_by:    String,
}
// where:
struct WorkflowStep {
    description:   String,             // may contain {{step:N}} placeholders
    tools_allowed: Vec<String>,
    step_fee:      u64,
    min_verifiers: u32,
    depends_on:    Vec<u32>,           // step indices that must VERIFY first
}
```
Touches (the ~4 files): `entry.rs` (variant + epoch() + weight), `tx.rs`
(validate: signed_by==requester, require_key, nonce, sig; DAG acyclic; fees sum ≤
max_fee; escrow from agent_credit), `api.rs` (`POST /api/agent/workflow` +
`GET /api/agent/workflow/:id`), `agent_task.rs`/orchestrator (scheduling).

### Scheduling (agent_task.rs / orchestrator, epoch-driven)
Store `agent_workflow:{id}` = { steps, statuses[], outputs[] }.
Each epoch, for a workflow:
1. For every step whose `depends_on` are all `Verified` and is itself `Pending`:
   substitute `{{step:N}}` in its description with output[N], emit an internal
   `AgentTaskPost` (reusing bid/assign/submit/verify/settle verbatim), mark
   step `Posted`.
2. When a step's task Settles, record its output, mark `Verified`.
3. When ALL steps `Verified`, workflow `Done`; refund unused escrow.
DAG acyclicity is validated at post time, so scheduling always terminates.
Fan-out (independent steps) + join (a step depending on several) fall out for
free. No new execution path — a workflow is orchestration *over* tasks.

### Why this shape
- Reuses the audited task verify/settle/reward economics per step — no new
  reward-safety surface.
- Outputs-as-inputs is the only genuinely new mechanic; it's a string
  substitution at post time, verifiable and simple.
- Matches the orchestrator's existing job/attempt/attestation model.

## 6. Order of operations
1. Beastly: finish inference_engine wiring incl. agent_worker (Build 1) → commit.
2. Beastly: add AgentWorkflow entry + orchestrator DAG scheduling (Build 2) →
   commit.
3. Beastly: write the reference 3-step workflow + a worker-execution path →
   commit.
4. Grouchly: run a worker, execute the reference workflow live, verify settle.
5. Both: mark #7 done on SCOREBOARD with the on-chain proof.
