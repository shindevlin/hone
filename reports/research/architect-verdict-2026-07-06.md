# Architect Verdict — HONE Improvement Scan (2026-07-06)

**Purpose:** This is the actionable distillation of [`scan-2026-07-06.md`](scan-2026-07-06.md),
filtered through an adversarial review that read the **actual source tree** (not just the
public README). Most of the scan's ~25 suggestions were rejected. What remains below is
what survived scrutiny against the real code, plus one genuine latent bug the scan missed.

**For the orchestrator:** items in §1 are ready to action, in the stated order. §2 lists
rejected suggestions with reasons so they are not re-proposed. Every claim marked ✅ was
verified against the file/line cited; claims marked ⚠️ are design-judgment, not code facts.

---

## §0. The one that must be fixed first — a real fork bug (not from the scan)

**Contract-initiated token transfers mutate consensus balance state on a single node and
are never broadcast or sealed.** ✅ Verified.

- `rust/hone-node/src/contracts.rs:174-199` — `ContractEngine::call()` iterates
  `result.pending_transfers`, builds a `LedgerEntry::Transfer`, and applies it directly via
  `self.chain.apply_entry(&entry)` (line 188). Balances are mutated locally.
- Contrast: every user-facing mutation (`/api/transfer`, `/api/stake`, …) routes through
  `apply_and_broadcast` in `rust/hone-node/src/api.rs` (e.g. lines 1663, 1681, 1699),
  which is where the **"zero peers MUST NOT apply an entry"** hardline gate lives.
- The consensus state root is the **balance** Merkle root. A contract that moves a token
  therefore advances the originating node's state root while the network never sees the
  entry — the exact silent-fork failure mode the no-local-submission hardline exists to
  prevent, entering through a path the hardline does not guard.

**Why the scan missed it:** the scan claimed "consensus divergence" for the wrong reason
(JIT non-determinism, see §1.2) in an execution model HONE does not run. The real, sitting
divergence is this un-broadcast contract transfer.

**Fix (decision required):** contract-emitted balance effects must flow through the same
broadcast + epoch-seal path as every other consensus entry — either by emitting the
`Transfer` into the pending pool for the network to seal, or by gating `execute_call` behind
the same peer-count check and broadcast that `apply_and_broadcast` enforces. Do **not** keep
applying contract `Transfer`s via a bare `apply_entry`.

---

## §1. Action list — what survived, in order

### 1. Deterministic partition test for the no-fork hardline  *(was scan #8: turmoil/madsim)*
**Verdict: BUILD (highest value in the entire scan).**
- ✅ `rust/hone-node/src/sim.rs` is a **testnet fake-activity generator** (seeds founder
  accounts, fires random transfers every 30s), **not** deterministic simulation testing.
  There is no existing test that drives a network partition and asserts non-divergence.
- The most safety-critical invariant in the system (zero-peers-never-apply) is currently
  defended by a single runtime `peer_count == 0` branch and zero adversarial tests.
- **Scope:** do NOT "adopt madsim" wholesale. Build a deterministic seam around
  `Chain::apply_entry` + the pending-pool → `drain_pending_sorted` → seal path, with an
  injectable clock and injectable peer/gossip inbox, driven by a seeded scheduler. Reach for
  `turmoil` only if the seam needs a real network model; `loom`/`shuttle` on
  `drain_pending_sorted` is the higher-value half.
- **Acceptance test that justifies the work:** a replayable test expressing "node is
  partitioned" / "node rejoins with a divergent pending pool" that **fails today if the
  `peer_count == 0` guard is deleted.** If that test can be written, it ships.

### 2. Pin WASM codegen determinism  *(was scan #1: wasmtime → wasmi — DOWNGRADED)*
**Verdict: take the free one-liner now; defer the wasmi swap pending a design decision.**
- ✅ `rust/hone-contract-runtime/src/executor.rs` builds its engine with `Config::new()` +
  only `consume_fuel(true)` — **no** `cranelift_nan_canonicalization`, no strategy pin.
- ⚠️ BUT the scan's headline premise ("every node must reach byte-identical contract
  results") is false for HONE: ✅ `ContractCall`/`ContractDeploy` are base-layer **no-ops on
  replay** (`chain.rs` ~1112) — contract WASM executes **once**, on the receiving node, and
  contract storage (`contract:{id}:{key}` in CF_META) is **not** in the balance state root.
  Contracts are single-executor, not replicated-deterministic. The JIT-divergence risk the
  scan describes is not currently exposed.
- **Action now:** add `config.cranelift_nan_canonicalization(true)` and pin codegen settings
  in `executor.rs`. Nearly free; closes the surface *if* contracts ever become replicated.
- **Deferred decision:** `wasmi` (deterministic interpreter, native fuel replacing the
  bolted-on gas, no per-version codegen drift) is the correct choice **only if** you decide
  contract execution becomes part of consensus replay (every node re-executes on seal). Until
  that decision, wasmi is a ~15% perf regression defending against a non-exposed divergence.
- **Note:** this item and §0 are the same subsystem. Resolve §0 (are contract balance
  effects consensus state? — they are) alongside the replay-vs-single-executor decision.

### 3. Config-driven inference arch dispatch registry  *(was scan #3: qwen wiring)*
**Verdict: BUILD (study Crane's pattern, do not depend on it) — and it must retire Ollama.**
- ✅ `rust/hone-android/src/llm.rs` loads `Qwen2.5-0.5B-Instruct-GGUF` via
  `candle_transformers::models::quantized_llama::ModelWeights` — the "wire qwen through the
  llama arch" hack. It works for **dense** Qwen only because dense GGUF is llama-shaped; it
  will **not** carry `qwen2_moe`/`qwen3_moe`, which is exactly where the "multi-arch unsolved"
  pain lives. So the scan is right: the missing piece is a config-driven dispatch registry,
  not a candle capability gap.
- ⚠️ **Larger finding the scan missed:** ✅ the node worker path
  (`rust/hone-node/src/worker.rs`, `call_ollama` / `OLLAMA_URL`) shells out to **Ollama** —
  an external inference daemon. This contradicts the **embedded-candle-is-mandatory**
  anti-cheat mandate (an external service is the supervised-fallback hole the mandate closes).
- **Scope:** build a registry routing `{llama, qwen2, qwen2_moe, qwen3, qwen3_moe}` GGUF to
  the correct `candle-transformers` model, **inside the embedded-candle path.** Read Crane
  (MIT, pure-candle) as a reference for the "add a model in <100 LOC" pattern; own the code.
- **Acceptance:** the registry must let the node path **retire the Ollama shell-out**, not
  run alongside it. If it doesn't kill Ollama, it hasn't solved the problem.

### 4. Proof-of-inference design track  *(was scan #5 — keep as a spec, not a swap)*
**Verdict: STUDY / design-track, sequence after 1–3.**
- The viable design is optimistic re-execution + fraud-proof dispute game (OPML pattern),
  **gated on batch-invariant deterministic kernels** — without batch-invariance, honest
  miners produce different outputs and consensus breaks. This depends on the §3 registry and
  the §0/§2 contract-execution decision landing first. No dependency to adopt; it's a spec.

---

## §2. Rejected — do not re-propose (with the reason each died)

| Scan item | Verdict | Why it dies |
|-----------|---------|-------------|
| **#6 VRF → schnorrkel/fastcrypto** | ❌ REJECT | ✅ There is no custom keyed VRF. `vrf.rs` is a commit-reveal **XOR randomness beacon** (RANDAO-shaped, 1-of-n unbiasable), not a per-node keyed VRF. ECVRF is a single-party primitive answering a different question. The finding pattern-matched the word "VRF" in a doc comment and never opened the file — it discredits the scan's crypto section. |
| **#2 FROST 2-of-3 founder vault** | ❌ REJECT | ⚠️ The "never auto-sign" rule is deliberately a **human intent gate**. FROST removes the human review beat and reintroduces machine-coordination cost. It solves multisig mechanics; HONE's bottleneck is intent verification, which FROST does not provide. |
| **#4 storage trait + redb/fjall** | ❌ NOT NOW | ⚠️ Sound someday, but `store.rs` is threaded as a concrete type through the chain, and `balance_merkle_root()` must be **byte-identical** across backends or a RocksDB server and a redb phone fork the state root on any iteration-order/encoding difference. That's a consensus-equivalence obligation the scan waves at with "benchmark both." Not a corroboration-count away from safe. |
| **#7 iroh transport + pkarr** | ❌ NOT NOW | ⚠️ Genuinely better NAT traversal, but it's the **ninth** transport (already have libp2p + tor + i2p + nostr + matrix + ton + udp_gossip + freeport), it stacks a second QUIC engine, and its relay fallback defaults to a third party — the scan's own "self-host relays or inherit centralization" caveat is the whole ballgame for a sovereignty chain. STUDY at best. |
| Observability/tooling tail: **utoipa, metrics.rs, tokio-console, cargo-zigbuild** | ❌ DEFER | Operator ergonomics, not improvements to HONE. utoipa is the best of them but retrofitting compile-time OpenAPI across ~9000 lines of hand-wired `api.rs` is a multi-week yak-shave a hand-maintained `openapi.json` beats in an afternoon. Adopt on felt pain, not on a scan. |
| Feature crates: **k256, passkey-rs, rust-tss-esapi (TPM), ssi/DID, uniffi** | ❌ DEFER | Additive hardening/interop for unfinished subsystems. TPM can't be load-bearing anti-sybil (phones lack accessible TPM — scan admits it). k256 would replace audited C `secp256k1` for a speculative `no_std` niche. ssi/DID is interop for an ecosystem HONE isn't courting. |
| Storage/CID stack: **ipld-core, iroh-blobs, bao-tree, fastcdc, reed-solomon** | ❌ DEFER | All contingent on a sha256-vs-BLAKE3 content-addressing decision not yet made, hanging off a Wiiv CID TODO that isn't on the mainnet path. The honest core — *make that decision once, pre-mainnet* — stands; the crate adoptions are premature until it's made. |
| Explicit rejects: **Nym (AGPL), Yggdrasil/hyperdht (wrong lang), poanetwork/vdf (stale+GMP), polkavm (not-for-prod), ZenGo/webb GG-family (unaudited)** | ✅ AGREE | Scan agreeing with positions already held. Noted, no action. |
| **Replace clock consensus (GRANDPA/Tendermint/HotStuff)** | ✅ CORRECTLY NOT PROPOSED | They assume a mostly-online validator set; HONE's phones are intermittent full nodes by design. |

---

## §3. Method note for the orchestrator

The scan's repeated "surfaced from N of 6 agents" is **not evidence about HONE's code** —
it's one model reading one README, sampled six times. Weigh the arguments, not the vote
count. The findings that survived did so because they were checked against the source; the
findings that died (VRF especially) died because they weren't. Apply the same standard to
future monthly scans: a suggestion is only actionable once it's been grounded in the tree.

**Priority order for action:** §0 (contract-transfer fork) → §1.2 NaN-canonicalization
one-liner → §1.1 partition test → §1.3 candle registry (+ retire Ollama) → §1.4 PoI spec.
