# BTCPC Marketing Plan

## Narrative Architecture

Three characters. One arc.

| Character | Role | Public presence | End state |
|-----------|------|-----------------|-----------|
| **shindevlin** | Technical founder | Posts protocol design, build updates, mining data. Never personal. | Disappears when governance activates. Becomes chain lore. |
| **natoshisakamoto** | Skeptic-turned-believer | Posts "my CPU does real work" angle. Organic advocate. | Fades as real community fills this role. |
| **josh** | Legal advisor, post-handover voice | Responds to legal/governance questions. Becomes public face after Shin exits. | Permanent. The chain's spokesperson. |

The Satoshi arc: Shin builds in public, hands off, disappears. Josh inherits the mic. Natoshi was the proof that a skeptic couldn't argue with his own machine.

---

## Platforms by Persona

### shindevlin — The Builder

| Platform | Audience | Cadence | Content type |
|----------|----------|---------|-------------|
| Twitter/X | Crypto + AI devs | 3-4x/week | Protocol updates, data, technical threads |
| r/LocalLLaMA | Local AI community | 2x/week | Inference routing, Ollama integration posts |
| HackerNews | Builders | Monthly | Show HN launch, Ask HN architecture questions |
| Substack | Builders + technical readers | Bi-weekly | Deep dives: consensus design, work proofs, Lucid Pruning |
| GitHub | Devs | Continuous | Public repo, commit cadence, issues open |

**Voice:** Direct. No hype. Shows math and code. "Here is what the chain did today." Doesn't argue, just ships. Think Satoshi's bitcoin-dev list posts — dense, factual, self-evident.

---

### natoshisakamoto — The Skeptic

| Platform | Audience | Cadence | Content type |
|----------|----------|---------|-------------|
| Twitter/X | Crypto natives | 2-3x/week | CPU mining results, "was skeptical, changed my mind" |
| r/gpumining | GPU miners | 2x/week | Earnings comparisons, hardware setup posts |
| r/CryptoCurrency | Broad crypto | 1-2x/week | "Another chain? No. Here's why this one is different." |
| r/Bitcoin | Bitcoin maxis | Occasional | Careful engagement — "BTCPC doesn't compete with BTC" framing |

**Voice:** Skeptical. Practical. Talks in numbers, not vision. "My i5 earned X BTCPC last week running qwen3:4b. That's a real inference job somebody paid for." Natoshi doesn't evangelize — he reports. That's the credibility.

**Sunset plan:** As real community members start posting their own earnings and setups, Natoshi gradually goes quiet. He doesn't announce a departure — he just posts less. The community absorbed his role. This is the intended outcome.

---

### josh — The Voice (post-handover)

**Pre-handover:** Responds only to legal/governance/compliance questions. Never initiates. Never speaks as a project authority — only as "the advisor they brought in."

**Post-handover:** Becomes the primary public voice. Explains governance decisions, interfaces with media, handles formal communications. By this point, Shin is gone and Natoshi is quiet.

---

## Content Pillars

### Pillar 1: Proof of Work Is Not Wasteful — BTCPC Is the Next Step
- Bitcoin's PoW is elegant and necessary. SHA-256 puzzles are the security model.
- BTCPC doesn't criticize Bitcoin. It extends the insight: what if the work also produced something people wanted?
- Use this framing in r/Bitcoin, with Bitcoin maxis, anywhere PoW is defended.

### Pillar 2: My Machine Does Real Work
- The headline number: how many real inference jobs did the chain complete today?
- Natoshi's i5 CPU earning BTCPC for serving qwen3:4b requests is the proof-of-concept story.
- CPU vs GPU is a feature, not a bug — heterogeneous hardware = real decentralization.

### Pillar 3: Ways to Earn, Any Device
- GPU → mine inference
- Any device → clock node (phone, laptop, Raspberry Pi)
- NAS or spare disk → storage rewards
- LoRa/Helium → IoT sensor rewards
- Cloud VM → service hosting
- Entry point for every hardware type = wide top of funnel.

### Pillar 4: No Burn, No Punishment, No Gatekeeping
- No staking requirement to mine
- No slashing for going offline
- No burned tokens — 42M forever
- No synthetic work — if nobody needs inference today, pool recycles
- These are differentiators vs. every other chain. Lead with them.

### Pillar 5: Built to Hand Off
- The founder's exit is designed into the protocol from day one
- Genesis Operator NFT transfers to governance automatically
- Shin's philosophy: "build it so well that you become unnecessary"
- This is the story that makes BTCPC credible long-term.

---

## Launch Sequence

### Stage 1 — Build in Public (Now → 10 miners)
**Goal:** Establish shindevlin as a credible technical builder. Seed the Natoshi narrative.

- Shin posts real chain data weekly: inference jobs completed, miners active, epoch stats
- Natoshi posts his CPU setup and first earnings
- GitHub repo public under shindevlin
- Show HN post: "I built a blockchain where miners run Ollama instead of SHA-256"
- No hype claims. Show the work.

### Stage 2 — Community Seeding (10 → 100 miners)
**Goal:** First wave of real community. Start organic Natoshi-replacement process.

- Shin publishes Substack deep dives: consensus design, Lucid Pruning, four-tier finality
- Natoshi's posts start getting replies from real users with their own setups
- r/LocalLLaMA thread: "BTCPC routes inference jobs to Ollama miners — anyone building on this?"
- First real community members start posting their earnings
- Josh answers governance questions publicly for the first time

### Stage 3 — Governance Handoff (100+ miners, governance active)
**Goal:** Shin exits. Josh takes over.

- Shin's final post: "The Genesis Operator NFT has transferred. I'm just another node now."
- No announcement of departure after that. He just stops posting.
- Josh publishes governance explainer, becomes the public point of contact
- Natoshi posts final "my machine is just one of thousands now" post, then fades
- Chain governs itself. The founders doc is historical record.

---

## The Show HN Post (shindevlin voice)

> **Show HN: I built a blockchain where GPU miners run Ollama instead of solving SHA-256**
>
> BTCPC is a sovereign chain where block rewards are earned by machines doing real AI inference.
>
> The mining loop: a user submits an inference job via the API → the job is broadcast to miners → miners run it through Ollama (any model) → the result is returned to the user → the miner earns tokens proportional to the verified parameter count of their model.
>
> No proof-of-work puzzle. The inference result IS the proof. If nobody submits a job, the miner pool recycles to the next epoch — no make-work.
>
> The chain also has clock nodes (any device, 5% of rewards), storage hosts (15%), and IoT sensor bridges (10%). You can mine on a phone if you just want to keep the clock.
>
> Six reward pools, 42M supply, 10 decimals, 30-second epochs. The original whitepaper is inscribed on block 0, while the living docs continue to evolve and the latest whitepaper is re-inscribed natively on BTCPC every 10,000 epochs.
>
> I'm not going to be here forever. The Genesis Operator NFT transfers to governance automatically when enough miners stake. That was the design from day one.
>
> Happy to answer questions about the consensus model, the inference routing, or the reward math.
>
> [honemesh.net] [whitepaper] [github]

---

## Hard Rules

1. **Shin never reveals he will disappear** — that's the arc, not the announcement. He just ships and eventually stops.
2. **Natoshi never claims to be independent of the project** if directly and sincerely asked — he deflects or is vague, but doesn't lie to a sincere direct question.
3. **Josh never posts as a founder** — always the advisor angle.
4. **No hype. No price predictions. No "to the moon."** Shin speaks in data. Natoshi speaks in personal experience. Josh speaks in governance and law.
5. **Bitcoin's PoW is not wasteful** — BTCPC extends it, never criticizes it. Memory: `feedback_bitcoin_respect.md`.
6. **No agent posts without human review** — both personas are human-in-the-loop. A ZeroClaw agent drafts; the poster reviews and publishes.

---

## ZeroClaw Agent Architecture (drafting only)

The marketing agent drafts content. Josh reviews and posts manually.

```
btcpc-marketing/
  SOUL.md          — persona instructions for shin + natoshi
  tools/
    reddit_draft.py    — drafts post for given subreddit + pillar
    twitter_draft.py   — drafts tweet thread
    substack_draft.py  — drafts technical deep dive
    notify_josh.py     — Telegram alert when legal/governance question detected
  schedules/
    shin_weekly.yaml
    natoshi_weekly.yaml
  memory/
    posted.json        — tracks what's been posted (no repeats)
```

Inference runs through BTCPC at https://honemesh.net/testnet (qwen3:4b or qwen3.5:9b).
Model for draft generation: qwen3.5:9b (better writing quality for Shin's technical voice).

---

## Metrics to Track

| Metric | Target (Stage 1) | Target (Stage 2) |
|--------|-----------------|-----------------|
| Active miners | 10 | 100 |
| Inference jobs/day | 50 | 500 |
| GitHub stars (shindevlin/btcpc) | 100 | 1,000 |
| r/LocalLLaMA subscribers following | 50 | 500 |
| Twitter/X followers (combined) | 500 | 5,000 |

The only metric that matters long-term: **inference jobs per day**. That's the chain's actual output. Everything else is noise.
