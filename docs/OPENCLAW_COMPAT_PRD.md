# BTCPC ↔ OpenClaw Compatibility PRD

> **Goal:** make BTCPC and OpenClaw (github.com/openclaw/openclaw — a local-first,
> multi-channel personal AI assistant, Node/TS monorepo, Gateway control plane,
> plugin system, sandboxed tool execution) interoperate, in increasing depth.
> OpenClaw brings users, channels, and distribution. BTCPC brings mining-backed
> inference, a wallet, escrow, and a job marketplace. Neither has to build the
> other's hard part.

This file is the canonical backlog for this integration. Work the phases in
order — each phase should be independently shippable and prove real value
before the next is attempted. Agents/sessions pick the next `[ ]` item, build
it, test it, commit, and tick the box.

---

## Phase 0 — Groundwork (do this first, before any phase below)

- [ ] **Get OpenClaw running locally** — clone `github.com/openclaw/openclaw`,
  read its actual plugin API and inference-provider config (README describes
  it as Node/TS pnpm monorepo with a Gateway; confirm by reading code, not
  assumption). Document findings in this file under "OpenClaw Architecture
  Notes" below.
- [ ] **Confirm BTCPC's OpenAI-compatible endpoint works standalone** —
  `curl -X POST https://btcpc.net/v1/chat/completions -H "Authorization:
  Bearer btcpc_..." -d '{"model":"auto","messages":[...]}'`. If this endpoint
  is only documented and not live/tested, get a real API key via the faucet
  flow and prove one request round-trips.
- [ ] **Confirm `/api/bot/*` surface is stable enough to build on** — this is
  what `bots/btcpcwalletbot` already uses; an OpenClaw plugin would call the
  same routes. Read `src/routes/botRoutes.js` and note which endpoints exist
  today (create, login, balance, transfer, faucet, stake) vs. which would need
  to be added for Phase 2.

## Phase 1 — BTCPC as OpenClaw's inference backend

The shortest path. Wires OpenClaw's LLM calls through BTCPC's mining-backed
inference. No new BTCPC code — this is entirely OpenClaw-side configuration
(and maybe a small BTCPC-side adapter if `model: auto` doesn't map cleanly
onto whatever provider interface OpenClaw expects).

- [ ] **Identify OpenClaw's provider/model config surface** — find where
  OpenClaw configures its LLM provider (likely an OpenAI-compatible
  `baseURL`/`apiKey` config, given it's TS). Confirm it accepts an arbitrary
  `baseURL` like the existing btcpc.net `/v1` docs already assume.
- [ ] **Wire BTCPC as a provider** — point OpenClaw's inference config at
  `https://btcpc.net/v1` with a `btcpc_...` API key. Get one real
  conversation turn to complete through BTCPC's mining pipeline.
- [ ] **Verify the round trip on-chain** — confirm the request actually
  produced an `INFERENCE_CHARGE` entry and rewarded a miner (check via
  explorer or `/api/node/info` on a node that served the job). This is the
  proof that OpenClaw traffic = real BTCPC token demand, not just an API
  passthrough that happens to work.
- [ ] **Document the setup** — a short guide (`docs/OPENCLAW_INFERENCE_SETUP.md`
  or a section here) so any OpenClaw user can point their instance at BTCPC.

## Phase 2 — Conversational wallet plugin

An OpenClaw plugin that lets a user manage their BTCPC wallet from any
channel OpenClaw supports (Telegram, Discord, iMessage, Slack, etc.) by
talking to the existing `/api/bot/*` HTTP surface — the same API
`bots/btcpcwalletbot` already uses. No new chain logic; this is a thin
client, same as the existing bots.

- [ ] **Scope the plugin's command surface** — balance, transfer, faucet
  claim, stake/unstake, account creation. Reuse the same request/response
  shapes as `bots/btcpcwalletbot` so both clients stay consistent.
- [ ] **Build the OpenClaw plugin** — following OpenClaw's plugin API
  (confirmed in Phase 0), implement it as HTTP calls to `/api/bot/*`. Tokens/
  JWTs stored per-OpenClaw-user via whatever secret-storage OpenClaw's plugin
  system provides — never in plaintext config.
- [ ] **Test across at least 2 channels** — prove the same plugin works
  identically from two different OpenClaw-supported channels (e.g. Telegram
  and Discord), since that cross-channel behavior is the actual value OpenClaw
  adds over the existing single-purpose bots.
- [ ] **Decide fate of the existing Telegram-only bots** — once the OpenClaw
  plugin covers wallet actions, `bots/btcpcwalletbot` may become redundant for
  OpenClaw users (but should stay for non-OpenClaw Telegram users). Document
  the decision, don't silently deprecate.

## Phase 3 — OpenClaw device runs a BTCPC node (the flywheel)

The deep integration: every OpenClaw install becomes a potential BTCPC
earning node. If the user's hardware has Ollama/GPU, their assistant mines
while idle — potentially offsetting/paying for its own inference cost.

- [ ] **Determine supervision model** — can OpenClaw's existing sandbox/
  plugin backends supervise a long-running child process (a `btcpc-node`
  binary), or does this need a companion daemon alongside OpenClaw rather
  than inside it? Read OpenClaw's sandbox backend code before deciding.
- [ ] **Bundle or fetch `btcpc-node`** — decide whether OpenClaw ships the
  binary, downloads it on first run (reusing `website/install.sh` self-heal
  logic), or only integrates with an already-running node.
- [ ] **Auto-provision a node identity** — on first run, generate keys /
  create an account the same way `website/install.sh` does today for a
  fresh install, surfaced through OpenClaw's onboarding flow instead of a
  terminal prompt.
- [ ] **Surface node status in OpenClaw's UI/Canvas** — epoch, peers, earnings,
  mining role — so the user sees their assistant is earning, not just running.
- [ ] **Self-heal parity** — the node started this way must follow the same
  self-heal rule as everything else in this repo (see
  `docs/SELF_HEAL_PRD.md`): never crash-loop visibly, never require the user
  to run a command to recover.

## Phase 4 — Agent-to-agent settlement (frontier)

OpenClaw agents pay each other in BTCPC for compute or goods, using
`INFERENCE_CHARGE` and escrow primitives that already exist on-chain.

- [ ] **Design the settlement flow** — write a short spec: when does one
  OpenClaw agent owe another BTCPC (compute delegation? Freeport purchase
  made on a user's behalf? LinkGit pull with a paid tier?). This needs a
  design pass before any code — the existing escrow/`INFERENCE_CHARGE` entry
  types were built for human-initiated jobs, not necessarily agent-initiated
  ones. Confirm they generalize or note what's missing.
- [ ] **Prototype one concrete settlement scenario end-to-end** — pick the
  single most obviously valuable case from the design pass and build only
  that one first.
- [ ] **Security review before shipping** — agent-initiated payments are a
  new trust boundary (an agent spending a user's BTCPC without a human
  clicking "confirm" each time). Needs explicit spend limits / confirmation
  policy design, reviewed before this goes live for real users.

---

## OpenClaw Architecture Notes

*(Fill in during Phase 0 as facts are confirmed by reading actual OpenClaw
code — do not guess here, and correct anything below that turns out wrong.)*

- Repo: `github.com/openclaw/openclaw`
- Stack: Node.js/TypeScript, pnpm monorepo
- Core component: "Gateway" — control plane for sessions, channels, tools, events
- Companion apps: macOS, iOS, Android
- Plugin system with bundled extensions; multiple sandbox backends
- Multi-channel: WhatsApp, Telegram, Slack, Discord, Google Chat, Signal,
  iMessage, IRC, Microsoft Teams, Matrix, and others
- Voice on macOS/iOS/Android; live Canvas UI

---

## How agents work this PRD

1. Pick the highest-priority unticked item, starting with Phase 0.
2. Do not skip ahead to a later phase's items until the current phase's items
   are ticked — each phase is meant to prove the previous one was worth
   building on.
3. Commit atomically per item where possible; update this file, tick the box,
   note the commit hash or PR.
4. If an item reveals the plan is wrong (e.g. OpenClaw's plugin API can't do
   what Phase 2 assumes), stop and rewrite the affected item(s) rather than
   forcing a bad fit — note what changed and why.
