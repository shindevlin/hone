# BTCPC Content Inconsistencies — Handoff for Resolution

These items conflict across public-facing files. Each one needs a canonical decision
before the content can be finalized. Do not guess — flag to the operator.

---

## 1. Genesis date and timestamp

**Conflict:**

| Source | Value |
|--------|-------|
| `website/index.html` countdown JS | `G=1776236400000` → **April 15, 2026, midnight California** |
| `website/index.html` countdown text | "Midnight California · April 15, 2026" |
| `README.md` | `BTCPC_GENESIS_TIMESTAMP=1777590000000` → **May 1, 2026, midnight Ireland** |
| `CLAUDE.md` | `1776236400000 (2026-04-15T07:00:00.000Z). Do not change.` |
| `memory/project_genesis_launch.md` | "Midnight Ireland genesis (1777590000000 ms)" |

**Resolution needed:** Which timestamp is canonical on the live chain? The content team has
used both. All public pages must agree on one date. The homepage countdown JS and
README must use the same millisecond value.

**Note:** As of today (2026-05-01) the countdown is past either date and hides itself,
but the embedded text "April 15" is still wrong if the canonical date is May 1.

---

## 2. Pool reward percentages — fixed vs. demand-driven

**Conflict:**

| Source | Claim |
|--------|-------|
| `README.md` | "6-pool rewards: 55% miners, 10% verifiers, 5% clocks, 12% storage, 8% services, 10% IoT" |
| `website/index.html` | "No fixed % → demand drives the split" |
| `rust/btcpc-node/src/main.rs` | Calibration-normalized utilization with dynamic allocation — no fixed percentages |

**Resolution needed:** The README states fixed percentages that do not match the live
Rust implementation. The homepage is correct. The README must be updated to remove
fixed percentages or the implementation must define them as starting-point calibration
targets with plain-language explanation.

---

## 3. Canonical install path — Node.js vs. Rust

**Conflict:**

| Source | Claim |
|--------|-------|
| `README.md` (top) | "⚠ Node.js layer deprecated — canonical chain is now `rust/btcpc-node`" |
| `website/index.html` Get Started | Shows `node bin/btcpc-all` commands |
| `website/index.html` Clock Node card | Shows `node bin/btcpc-all` |
| `README.md` install section | Shows `curl ... install.sh | sudo bash` (installs Rust binary) |

**Resolution needed:** The homepage install commands still show Node.js. These must be
replaced with the Rust install path. Confirm that `btcpc-node` (Rust) is the only
supported path for new installs.

---

## 4. JavaScript SDK in README — still supported?

**Conflict:**

| Source | Claim |
|--------|-------|
| `README.md` | Shows `require('@btcpc/sdk')` and OpenAI-compatible `baseURL: 'https://btcpc.net/v1'` |
| `rust/btcpc-node/src/api.rs` | Rust Axum API — no `/v1/` route visible |

**Resolution needed:** Is `@btcpc/sdk` published on npm? Does `/v1/chat/completions`
exist on the live node? If not, remove from README until implemented.

---

## 5. Bridge — "Coming at launch" but genesis has launched

The homepage bridge section says "Base — Coming at launch," "Arbitrum — Coming at launch,"
etc. Genesis has launched. Either:
- Change these to a roadmap status ("In development," "Q3 2026," etc.), or
- Remove if no timeline is set

**Resolution needed:** What is the honest status of the EVM bridge? Replace
"Coming at launch" with accurate language.

---

## 6. TOTP / consensus-level 2FA

The homepage security section states: "TOTP (Google Authenticator compatible) enforced
at the consensus level." This is a strong claim. Verify this is implemented in
`rust/btcpc-node` and not planned/carried from a prior spec. If planned, change to
"planned" or remove until live.

---

## 7. `/docs` link

The footer links to `/docs`. The analysis report notes a redirect loop on this URL.
Verify the route exists and resolves correctly, or remove the link until it does.

---

## 8. Windows install path

`website/index.html` links to `/windows` in a comment. Verify `/windows` exists or
remove the reference.

---

## Resolution format

For each item, respond with:
```
ITEM N: [canonical answer]
Source of truth: [which file/code to follow]
Action: [what to change in which file]
```
