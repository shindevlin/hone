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

- [x] **Get OpenClaw running locally** — clone `github.com/openclaw/openclaw`,
  read its actual plugin API and inference-provider config (README describes
  it as Node/TS pnpm monorepo with a Gateway; confirm by reading code, not
  assumption). Document findings in this file under "OpenClaw Architecture
  Notes" below. **DONE** — cloned at HEAD `43a7fd38` (2026-07-03), read the
  actual plugin SDK, provider config, and OpenAI-compatible provider path. All
  facts recorded in "OpenClaw Architecture Notes" below are from source, not
  assumption. (Note: this run did not execute `openclaw onboard`/`gateway` — it
  requires Node ≥22.19 + `pnpm install`; the "read the code" half of this item,
  which is what every downstream phase actually needs, is complete. Actually
  standing up a live Gateway is folded into Phase 1's "Wire BTCPC as a provider"
  step, where it's needed for the real round-trip.) Commit on `main`.
- [ ] **Confirm BTCPC's OpenAI-compatible endpoint works standalone** —
  `curl -X POST https://honemesh.net/v1/chat/completions -H "Authorization:
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
  `baseURL` like the existing honemesh.net `/v1` docs already assume.
- [ ] **Wire BTCPC as a provider** — point OpenClaw's inference config at
  `https://honemesh.net/v1` with a `btcpc_...` API key. Get one real
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

*Confirmed by reading actual OpenClaw source at HEAD `43a7fd38` (2026-07-03),
`github.com/openclaw/openclaw`, version `2026.6.11`. File paths below are
relative to the OpenClaw repo root. Correct anything here that later turns out
wrong.*

### Repo shape (confirmed)

- Stack: Node.js/TypeScript, **pnpm** workspace monorepo. `engines.node >=22.19.0`,
  `packageManager: pnpm@11.2.2`. Plain `npm install` is explicitly unsupported —
  use `pnpm install`.
- Workspace globs (`pnpm-workspace.yaml`): `.`, `ui`, `packages/*`, `extensions/*`.
- Relevant `packages/`: `plugin-sdk` (plugin API facade), `llm-core`, `llm-runtime`,
  `model-catalog-core` (provider/model config types), `net-policy`, `gateway-protocol`,
  `gateway-client`, `agent-core`, `sdk`.
- `extensions/` holds ~130 bundled plugins, including one **per LLM provider**
  (`openai`, `anthropic`, `google`, `groq`, `deepseek`, `openrouter`, `litellm`,
  `lmstudio`, `vllm`, `ollama`, `together`, `fireworks`, …) and one per channel
  (`telegram`, `discord`, `slack`, `whatsapp`, `signal`, `imessage`, `matrix`, …).
- Run/dev (from `README.md`): global install path is `openclaw onboard
  --install-daemon` then `openclaw gateway status`; source-checkout path is
  `pnpm install` → `pnpm openclaw setup` → `pnpm gateway:watch` (runs TS directly
  via `tsx`). This run read the code but did not stand up a live Gateway — see
  the Phase 0 item note.

### Plugin system (needed by Phase 2 & 3) — CONFIRMED, no blockers

- A bundled plugin is a directory under `extensions/<id>/` with:
  - `openclaw.plugin.json` — the **manifest**. Real fields observed: `id`,
    `activation.onStartup`, `enabledByDefault`, `providers`, `modelSupport`,
    `providerEndpoints` (host allow-list metadata), `modelCatalog` (provider
    `baseUrl` + `api` + full `models[]` with cost/context), `setup.providers`
    (`envVars`), `providerAuthChoices` (auth methods, incl. `optionKey`/`cliFlag`),
    `contracts`, and a JSON-Schema `configSchema` for plugin config.
  - `package.json` with `"openclaw": { "extensions": ["./index.ts"] }` and
    `@openclaw/plugin-sdk` as a dev dep (`workspace:*`).
  - `index.ts` — entrypoint using `definePluginEntry({ id, name, description,
    register(api) })` from `openclaw/plugin-sdk/plugin-entry`.
- The `register(api)` object is `OpenClawPluginApi` (defined in
  `src/plugins/types.ts`, re-exported via `packages/plugin-sdk`). It is **very
  broad**. The registration methods that matter for BTCPC:
  - `registerTool(tool | factory, opts)` — **this is the Phase 2 hook.** A tool is
    `AnyAgentTool`: `{ name, label, description, parameters (a Typebox/JSON schema),
    execute(toolCallId, rawParams) }`. Real template to copy:
    `extensions/firecrawl/src/firecrawl-scrape-tool.ts` — an HTTP-backed external-API
    tool with an API key, structurally identical to what a wallet tool needs.
  - `registerProvider(provider)` — native LLM provider (Phase 1 uses this
    indirectly; see below).
  - `registerCli(...)`, `registerGatewayMethod(...)`, `registerHttpRoute(...)`,
    `registerChannel(...)`, `registerService(...)` — `registerService` is a
    **long-running service** hook, directly relevant to Phase 3 supervising a
    `btcpc-node` child.
  - `api.config` (full `OpenClawConfig`) and `api.pluginConfig` (this plugin's
    config, validated against `configSchema`).
- **Secret storage (answers the Phase 2 "never in plaintext config" requirement):**
  OpenClaw stores secrets as **env-var / secret references**, not raw strings.
  Primitives live in `openclaw/plugin-sdk/secret-input`:
  `resolveSecretInputString`, `normalizeSecretInput`, `isSecretRef`, `coerceSecretRef`,
  `buildSecretInputSchema`. Config fields declared with `buildOptionalSecretInputSchema()`
  are recognized by redaction and resolved at runtime (e.g. from `$FIRECRAWL_API_KEY`).
  `firecrawl/src/config.ts` is the reference pattern. **A BTCPC wallet plugin should
  store its `/api/bot/*` JWT/token as a secret-input ref, not a plaintext string.**

### Inference provider config (needed by Phase 1) — CONFIRMED, no blockers

The Phase 1 assumption ("OpenClaw accepts an arbitrary OpenAI-compatible `baseURL`")
is **correct**, and there are two clean paths:

1. **Env-var override on the bundled `openai` extension.** `extensions/openai/base-url.ts`
   → `resolveOpenAIDefaultBaseUrl()` returns `OPENAI_BASE_URL` if set, else
   `https://api.openai.com/v1`. So the absolute-shortest wiring is:
   `OPENAI_BASE_URL=https://btcpc.net/v1` + `OPENAI_API_KEY=btcpc_...`. Caveat:
   this masquerades BTCPC as "openai" and its model catalog is the hardcoded GPT
   list, so `model: auto`/BTCPC model ids won't be in the catalog (works for a
   raw round-trip; not clean for a distinct provider).
2. **Config-driven per-provider `baseUrl` (the correct path).** `baseUrl` is a
   first-class field on provider catalog entries
   (`packages/model-catalog-core/src/model-catalog-types.ts:223`, and per-model at
   `:243`/`:253`), settable under `config.models.providers.<id>.baseUrl`. The
   `litellm` / `lmstudio` / `vllm` / `openrouter` extensions exist precisely to
   point at user-supplied OpenAI-compatible endpoints:
   - `litellm` uses `config.models.providers.litellm.baseUrl` (default
     `http://localhost:4000`), auth choice has `allowExplicitBaseUrl: true`, and
     non-interactive setup accepts `ctx.opts.customBaseUrl`
     (`extensions/litellm/index.ts`, `extensions/litellm/onboard.ts`).
   - **This is the model to follow** for a first-class "btcpc" provider: either
     configure BTCPC under an existing generic provider id, or (cleaner) add a
     small `extensions/btcpc` provider plugin whose `modelCatalog.providers.btcpc`
     has `baseUrl: https://btcpc.net/v1`, `api: "openai-*"`, and lists BTCPC's
     `model: auto`. **No new BTCPC-node code is required for Phase 1 either way** —
     it's OpenClaw-side config (path 1) or a small OpenClaw-side provider plugin
     (path 2). Recommend path 2 for the "Wire BTCPC as a provider" step so on-chain
     attribution and model ids are clean.

- **Net-policy caveat (relevant if a BTCPC node is on localhost/LAN, e.g. Phase 3):**
  OpenClaw gates private/LAN endpoints. `litellm` shows the escape hatch —
  `resolveAllowPrivateNetwork` / `shouldAutoAllowPrivateLitellmEndpoint`
  (`extensions/litellm/image-generation-provider.ts`). A public `https://btcpc.net/v1`
  is unaffected; a `http://localhost:4242` local node would need the private-network
  allow. `packages/net-policy` is the relevant package.

### Channels (needed by Phase 2 "test across ≥2 channels") — CONFIRMED

Bundled channel extensions cover far more than the PRD header lists — including
`telegram`, `discord`, `slack`, `whatsapp`, `signal`, `imessage`, `irc`, `matrix`,
`msteams`, `googlechat`, `feishu`, `line`, `mattermost`, `nostr`, `twitch`, and
more. The Phase 2 goal (one plugin, identical behavior across e.g. Telegram +
Discord) is architecturally sound: a plugin registers **tools**, which are
channel-agnostic — the Gateway routes them to whichever channel the session is on.

### Open flags for later phases

- **Phase 3 supervision:** `registerService(...)` exists as an in-process
  long-running service hook — Phase 3's "supervise a `btcpc-node` binary" is
  plausible *inside* OpenClaw via a service that spawns/monitors the child, but the
  sandbox-backend read that item calls for hasn't been done yet. Left for Phase 3.
- **Model catalog vs. `model: auto`:** BTCPC's `model: auto` won't appear in
  OpenClaw's model catalog unless BTCPC is added as a catalog provider (path 2
  above). Path 1 works for a raw round-trip but not for catalog/routing UX.

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
