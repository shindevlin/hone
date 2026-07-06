---
title: BTCPC Post-Launch — project analysis, forward goal, and autonomous collaboration protocol
description: Now that hone is live, where the project stands, the goal to keep advancing, and how Beastly + Grouchly build together without human intervention (except gated actions)
author: Shin Devlin
status: living charter
---

# Post-Launch Charter

> **Context:** hone launched July 4 2026, live and in consensus (epoch 150+,
> 20+ peers, 4 founder nodes). The keys-lost failure that caused 4 relaunches is
> permanently fixed (recoverable keystores). This charter shifts the two agents
> (Beastly = dev/chain, Grouchly = devices) from launch-firefighting to
> **sustained, autonomous forward progress.**

---

## 1. Project analysis — where we actually stand (2026-07-04)

### Live and working
- **Chain:** hone live, 4 founder nodes (shindevlin/beastly, natoshisakamoto +
  Nebra/josh via Grouchly), sealing epochs, rewards flowing. shindevlin ~1.3B
  dreams earned.
- **Keys:** all 11 accounts recoverable (Argon2id keystores), full multi-chain key
  records exported + encrypted. Relaunch cycle broken.
- **Bullship (first customer):** funded, inference key registered, real
  completions billed. Runs on real hone inference.
- **Roadmap:** 78/81 done. Only open item: **T4-5** (HardwareClaim TEE
  attestation, Tier 4).
- **Integration manifest:** self-updating consumer-repo understanding, shipped.

### Built-but-not-fully-live (the real work queue)
- **Verasens:** sensor pipeline exists; Grouchly fixed a `sort_keys=True` sig bug
  that was failing ALL submissions. With josh funded, needs live verification that
  SensorDataCommit → SensorReward flows.
- **Flipper → chain:** `flipper_rx.rs` (parse+verify+submit) is BUILT in
  `android/rust/btcpc-miner`, but NOT wired into the running Android client
  (btcpc-android/www is a stub). Gap = JNI wiring, not missing code. Don't
  rewrite in TS.
- **Phone node:** can be a full node (miner qwen2.5-0.5b + sensors + clock); needs
  the APK rebuilt (Beastly has the Android SDK) and josh-funded submission live.
- **Inference engine:** migrating off external Ollama → embedded candle GGUF
  (`inference_engine.rs`, candle compiles on 1.90). Removes the daemon-down
  fragility that 503'd bullship. Needs: wire the ~6 Ollama call sites, warm_up at
  boot, streaming variant.
- **Verticals not yet activated:** freeport, linkgit have accounts + keys but no
  live service. Bullship is the template for turning each on.

### Debt / cleanup
- Two phone inference paths (candle in btcpc-android, tract-onnx in btcpc-miner) —
  converge on candle.
- Ollama still the default inference path until the candle migration lands.

## 2. The forward goal

> **GOAL: Make every BTCPC vertical demonstrably EARN on the live chain, with the
> node stack non-fragile and the two agents advancing it autonomously.**

Measured by these milestones, in rough priority:

1. **Verasens earns** — a phone/Flipper/Nebra submits SensorDataCommit and
   receives SensorReward on hone, verified on-chain. (Closest to done —
   Grouchly's sig fix + josh funding unblock it.)
2. **Phone is a live earning node** — APK rebuilt, phone mines (qwen2.5-0.5b) +
   sensors, visibly earning in-app. The Proof-of-Compute demo made real.
3. **Inference non-fragile** — embedded candle default; no node 503s for a
   missing daemon; bullship runs on any node.
4. **Flipper → chain** — flipper_rx wired via JNI into the Android client;
   sub-GHz/NFC data flows to Verasens.
5. **Second + third verticals live** — freeport and linkgit turned on using the
   bullship template (fund → register → point service at a node).
6. **Close T4-5** — hardware attestation, the last roadmap item.

Each milestone = one demonstrable "this vertical earns" proof. Keep a running
scoreboard on the channel.

## 3. Autonomous collaboration protocol (channel robustness)

The channel transport works. What's missing is a *protocol* so Beastly + Grouchly
build together without a human in the loop — except for gated actions. Rules:

### 3.1 Roles (fixed)
- **Beastly** — chain/node code, SDK, inference engine, APK builds (has Android
  SDK), key/sign-request preparation, git.
- **Grouchly** — physical devices (phone, Flipper, Nebra), on-device runtime,
  live sensor/Verasens verification.

### 3.2 Work handoff (so nothing stalls waiting on a human)
- **Claim before starting:** `channel send --type directive "CLAIM: <milestone
  #N> <what> — mine, hands off"`. Check `recv` for the other's claims first;
  if both want it, the earlier claim wins, the other picks the next item.
- **Report on finish:** `--type response "DONE: <#N> <what>, <sha/result>. Next I
  claim <#M>."` — always announce the next claim so momentum never stops.
- **Blocked → hand off, don't wait:** if blocked on the other's lane, send a
  `--type directive "BLOCKED on <X> — need you to <Y>; meanwhile I'm on <#M>."`
  and move to other work. Never idle waiting.
- **Heartbeat:** each agent runs `channel watch --interval 30` so replies are seen
  within 30s. No polling the human.

### 3.3 The scoreboard (shared state)
- A file `bridge/SCOREBOARD.md` (committed to the channel repo) tracks the 6
  milestones: owner, status (todo/in-progress/blocked/done), last update, proof.
  Either agent updates it and pushes. This is the single source of "what's the
  state" so neither agent nor the human has to reconstruct it.

### 3.4 Permission gates (the ONLY things that require the human)
Autonomy is the default. These specific actions STOP and require Shin's explicit
approval via a sign-request or ask — never done autonomously:
- **Token transfers / any value movement** (fund, transfer, stake, purchase).
  Prepare an unsigned sign-request, route to Shin or a triumvirate founder wallet
  (shindevlin/natoshisakamoto/josh). See memory: no-autonomous-token-transfer.
- **Pushing to `main`** (feature branches only; PRs opened by Shin).
- **Anything outward-facing / irreversible** (publishing, external posts,
  destructive ops on prod, deploying to accounts we don't control).
- **Secrets** never go in channel messages or the git tier — vault only.

Everything else — building, testing, wiring, APK builds, config, code review,
deploying to OUR nodes, verifying on-chain — proceeds without asking.

### 3.5 Escalation
If both agents are blocked on a gated action (e.g. a needed transfer), post a
single consolidated `--type directive` addressed to Shin with the exact
sign-requests batched, so one human touch unblocks multiple items.

## 4. Immediate next actions (from this charter)
- **Beastly:** finish the inference-engine wiring (call sites + streaming) → commit;
  rebuild the phone APK when Grouchly sends inputs; verify flipper_rx JNI wiring.
- **Grouchly:** verify Verasens earns with josh funded (milestone 1); Nebra GNSS
  sensor live; report to SCOREBOARD.
- **Both:** stand up `bridge/SCOREBOARD.md` and run `watch --interval 30`.
- **Shin (gated):** approve any funding sign-requests for freeport/linkgit when
  those verticals are ready to activate.
