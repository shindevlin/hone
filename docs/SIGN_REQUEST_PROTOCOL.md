# Sign-Request Protocol

**Version 0.1 — July 2026**

---

## Purpose

Value movement on HONE — funding a service account, providing liquidity, any
`Transfer` — must be authorized by a **human with the active key**, never by an
agent autonomously. This protocol is the safe bridge between "an agent prepared
a transaction" and "Shin authorized it with one deliberate approval."

The rule it enforces (standing, non-negotiable):

> **An agent never holds, sees, or signs with an active/owner key. An agent
> proposes; a human disposes. Every token transfer is an explicit, reviewed
> human approval.**

This is the same model hardware wallets use (you confirm on-device) and multisig
(a human quorum signs). The agent can be maximally useful — computing amounts,
building the exact transaction, routing it — right up to the signing line, and
hand the human the pen.

---

## Roles

| Role | Does | Never does |
|---|---|---|
| **Requester** (agent / node / vertical) | Builds an UNSIGNED transaction: recipient, amount, token, purpose, nonce, and the canonical signing message. Posts it as a sign-request. | Signs. Holds the active key. Submits a signed tx. |
| **Approver** (Shin, active-key holder) | Reviews the request (what / to whom / how much / why), then approves — signing locally with the active key. | — |
| **Submitter** | Broadcasts the signed transaction to the chain. May be the approver's own tooling. | Signs on the approver's behalf. |

The approver's active key **never leaves the approver's machine.** The requester
never receives it, and the request never contains a private key.

---

## The Flow

```
Requester                         Approver (Shin)                Chain
   │                                    │                          │
   │ 1. build unsigned tx               │                          │
   │    (to, amount, purpose, nonce,    │                          │
   │     canonical signing message)     │                          │
   │ 2. post sign-request ──────────────▶ 3. review: what/who/     │
   │                                    │    how much/why           │
   │                                    │ 4. APPROVE ("I want to    │
   │                                    │    send this") →          │
   │                                    │    sign locally with      │
   │                                    │    active key             │
   │                                    │ 5. submit signed tx ──────▶ applied
   │ 6. read outcome ◀──────────────────┴──────────────────────────┘
```

Nothing moves between steps 2 and 4 without the human. If the approver does
nothing, nothing happens — the safe default is inaction.

---

## Sign-Request Document

A structured, human-readable request. It is UNSIGNED — it carries everything
needed to review and sign, and NO private key.

| Field | Meaning |
|---|---|
| `request_id` | Unique id for this request. |
| `kind` | `transfer` (extendable: `stake`, `account_transfer`, …). |
| `from` | The account that will sign/pay — the approver's account (e.g. `shindevlin`). |
| `to` | Recipient account. |
| `amount_hunits` | Amount in hunits (1 HONE = 10^10). Shown in HONE too for review. |
| `token` | Token (native HONE by default). |
| `nonce` | The `from` account's next nonce (fetched from chain). |
| `purpose` | Human-readable reason ("fund hone-market for LP", "service key top-up"). Shown to the approver. |
| `canonical_signing_message` | The exact bytes the active key will sign — computed by the requester via the chain's `canonical_signing_message`, so the approver signs precisely this and nothing else. |
| `requested_by` | Who/what built the request (agent id, node, vertical). |
| `expires` | Optional: a nonce/epoch after which the request is stale (nonce reuse makes it naturally single-use). |

The requester computes `canonical_signing_message` from the same function the
node uses to verify (`tx::canonical_signing_message`), so **what the approver
signs is exactly what the chain will accept** — no substitution between review
and signature.

---

## Approval

The approver reviews and approves with the active key, locally. Reference form
(CLI, no key ever transmitted):

```
hone sign-request review  <request>     # prints from→to, amount (HONE + hunits),
                                         # purpose, nonce — human reads it
hone sign-request approve <request>     # re-shows details, prompts a deliberate
                                         # confirmation, signs the canonical
                                         # message with the ACTIVE key from the
                                         # local keystore (HONE_WALLET_PASSWORD),
                                         # produces the signed Transfer entry
```

- **Approve** = a deliberate human action ("I want to send this"), after seeing
  the amount and recipient. A future confirm UI can replace the CLI prompt with
  a literal "I want to send this" button — the protocol is the same.
- The active key is loaded from the approver's local keystore at approve time and
  never leaves the machine. The signed entry is what gets submitted.
- **Reject** / ignore = nothing happens. Inaction is safe.

---

## Security Properties

- **No autonomous transfer, structurally.** The requester has no active key and
  produces only an unsigned request. It is *impossible* for the agent to move
  value — not by policy, by construction.
- **What-you-see-is-what-you-sign.** The approver signs the exact
  `canonical_signing_message` shown; the requester can't swap the recipient or
  amount between review and signature (the signed bytes ARE the reviewed bytes).
- **Nonce = single-use.** Each request pins the account nonce; once applied, the
  same request can't replay.
- **Auditable.** Every value movement traces to a specific reviewed request +
  human approval, with a stated purpose.
- **Least privilege.** Only funding/LP/transfers go through this. Read-only and
  non-value operations don't need it.

---

## Relationship to Roles & Keys

- **Posting keys** (day-to-day chain entries: sensor commits, storefront, git
  push, render-worker registration) are wired into each **node's own local
  config** (`HONE_POSTING_KEY` / the node's `wallet.key`) by the operator. An
  agent never sees them; the node signs its own routine entries. This protocol is
  NOT needed for those — only for value movement.
- **Active/owner keys** (transfers, funding, LP, key rotation) are the
  approver's, gated behind THIS protocol.

---

## Post-Launch Use

The immediate driver: after re-genesis, service accounts (bullship, hone-market,
freeport, …) need funding, and Shin funds other wallets / LP. Every one of those
is a sign-request — the agent prepares the exact transfer, Shin approves. This is
the mechanism the re-genesis directive already referenced ("re-fund service keys
= sign-request to Shin, never autonomous").

---

_The agent proposes; the human disposes. The active key never leaves your hand._
