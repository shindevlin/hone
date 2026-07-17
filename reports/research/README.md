# HONE Improvement Research

**What this is:** an ongoing, evidence-based scan of GitHub / the OSS ecosystem for
projects that would *objectively* make HONE better — faster, safer, more sovereign, or
less code to maintain. Every candidate carries a verdict: **use as a dependency**,
**fork & adapt**, or **study & rebuild in Rust**.

This is a living directory. Each scan is dated and additive; findings graduate into
tracked work when adopted, and get struck through when rejected or superseded.

## Index

| Date | Report | Focus |
|------|--------|-------|
| 2026-07-06 | [scan-2026-07-06.md](scan-2026-07-06.md) | Inaugural full-stack sweep: networking, consensus, inference, storage, identity/crypto, node infra |

## The fork-vs-rebuild question (the short answer)

The user's framing — *"what is better, a fork or a rebuild to Rust?"* — has a clear answer
for HONE, and it's **neither, mostly**:

1. **Depend, don't fork or rebuild** — for anything audited and security-critical
   (crypto, signing, VRF, WASM execution). Forking audited crypto throws away the audit;
   rebuilding it hand-rolls the exact primitives the project rules forbid rebuilding.
2. **Study & rebuild** — only for *patterns* that are language/chain-specific and can't be
   linked (proof-of-inference dispute games, stealth-address schemes, finality-gadget
   safety arguments, AGPL-licensed code like Nym).
3. **Fork** — the rare middle case: a small, permissively-licensed, near-fit project that
   you intend to own and diverge from (e.g. `libp2p-iroh` as a migration bridge).

A wholesale "rebuild HONE in a different form" is **not** on the table and nothing in this
scan argues for it — HONE's Rust single-binary architecture is sound. The wins are
**surgical component swaps**, ranked by leverage in each scan.

## Method

Each scan fans out parallel research agents, one per subsystem, each of which:
- verifies star counts, license, and maintenance status against the **live** GitHub repo
  (not from memory),
- maps each candidate to a specific, named HONE weakness,
- assigns a dependency / fork / study verdict **with the reason**,
- and is explicitly instructed to say when HONE's *existing* choice is already correct.

Findings are then cross-checked for convergence (the same project surfacing from
independent agents is a strong signal) and contradiction.
