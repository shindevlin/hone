# Cross-Chain Identity Binding

**Version 0.1 — July 2026**

---

## Position

HONE **does not brand itself as a privacy chain** — but its identity layer is
*functionally* private by construction. A HONE account can bind external-chain
wallets (Bitcoin, Ethereum/EVM, Solana, …) to itself so that value, 2FA, and
provenance can span chains — **without publicly broadcasting "this founder controls
that wallet."**

Three principles, consistent with how HONE already treats its own role keys:

1. **Holder-initiated.** A cross-chain link is created only by the account holder,
   deliberately. Nothing links chains automatically, and nothing links them at
   genesis.
2. **Sign-request-gated.** Creating a binding is an authorized action routed through
   the [Sign-Request Protocol](SIGN_REQUEST_PROTOCOL.md) — the holder reviews and
   approves it locally, exactly like a transfer. An agent may *prepare* the binding;
   only the holder *authorizes* it.
3. **Private by default.** The chain stores a **commitment** (a hash), never a raw
   external address, and — for a private binding — not the verifying signature
   either. The account can *prove* a link when it chooses; observers cannot *derive*
   one.

This is the same stance HONE takes on its own keys: role keys (`posting`, `active`,
`owner`, …) come from one seed on the **hardened** SLIP-10 path `m/44'/6942'/role'/0'`,
so they are mutually **unlinkable from outside** even though one holder controls
them all. Cross-chain binding extends that: the holder can tie chains together for
their own use, without the tie being publicly legible.

---

## Not In Genesis

`genesis.json` pins **only each account's `posting` public key.** It does **not**
contain other HONE role keys, and it does **not** contain cross-chain addresses,
keys, or commitments. Reasons:

- Publicly listing every role key or external wallet under a named founder account
  at block 0 would hand-link identities that the hardened key derivation was
  specifically designed to keep unlinkable.
- Cross-chain binding is **per-account and opt-in**, added when the holder wants it —
  not a launch-time property forced on all founders.

**Consequence:** this design does not change genesis. Block-0 hash stays
`98e3c1b0e447bc99c8b566ae9f46359f0f87a8e95dbf747ed829c4cffa129b2e`; no re-genesis,
no re-smoke.

---

## Mechanism (builds on existing `VerifyChainLink`)

HONE already has the primitive: the `VerifyChainLink` ledger entry. The account
proves control of an external wallet by signing a challenge **with that wallet's own
key**; the node recovers the address from the signature, confirms it matches a
commitment, and stores the commitment.

Challenge (canonical): `hone:link:{account}:{chain}:{nonce}`
Commitment: `sha256(chain + ":" + address + ":" + nonce)`

The node's verification path (today):
- `recover_chain_address(sig_type, signed_message, signature)` → external address
- re-derive `sha256(chain:address:nonce)`, require it equals the submitted commitment
- store `chain_proofs[chain] = { commitment, mode, … }` — **address discarded**

### Two visibility modes

| Mode | What's stored on-chain | Who can verify | Use |
|---|---|---|---|
| **private** (default) | commitment only (`sha256(chain:addr:nonce)`) + `mode:"private"`. Signature + signed_message are **NOT** stored on-chain — kept by the holder. | the holder (can reveal address+nonce+signature out-of-band to prove) | founder wallets, personal binding — tie chains together privately |
| **public** (opt-in) | commitment + `signed_message` + `signature` (`mode:"hard"`) | anyone, independently | a service/vault that *wants* to be publicly provable |

The current implementation stores the signature+message (public/"hard" mode). This
spec adds the **private** mode as the default: store the commitment (and enough to
re-verify on later reveal), but withhold the signature/message from chain state so an
observer cannot recover the external address. The nonce and signature live with the
holder; presenting them later re-proves the commitment.

> Privacy caveat (honest): a commitment under a named account still reveals *that*
> the account bound *some* wallet on *that chain*. Private mode hides **which**
> wallet, not **that a binding exists**. An account wanting to hide even existence
> should not post a binding until it needs one.

---

## The Flow (holder-initiated, sign-request-gated)

```
Holder / agent                     Holder (approver)                Chain
   │                                     │                            │
   │ 1. build UNSIGNED binding request   │                            │
   │    kind=chain_link                  │                            │
   │    {account, chain, nonce,          │                            │
   │     commitment, visibility}         │                            │
   │ 2. sign-request ────────────────────▶ 3. review: which chain,    │
   │                                     │    which wallet, private?   │
   │                                     │ 4. APPROVE → sign the       │
   │                                     │    challenge with the       │
   │                                     │    EXTERNAL wallet's key     │
   │                                     │    (MetaMask/Ledger/Phantom) │
   │                                     │ 5. submit VerifyChainLink ──▶ verify + store
   │                                     │                            │   commitment
   │ 6. (private) holder keeps nonce+sig │                            │
```

- The **sign-request** carries the unsigned binding (account, chain, nonce,
  commitment, visibility) and a human-readable purpose. See
  [SIGN_REQUEST_PROTOCOL](SIGN_REQUEST_PROTOCOL.md). The agent builds it; the holder
  approves.
- Approval means the holder signs the `hone:link:…:{nonce}` challenge with the
  **external wallet's** key — the proof of control. That signature never requires the
  HONE active key and never exposes any private key to the agent.
- For **private** bindings, the holder retains `{address, nonce, signature}`; the
  chain keeps only the commitment. Later, the holder can reveal those three to prove
  the link to any verifier, on demand.

---

## Relationship to Other Specs

- **Sign-Request Protocol** — binding creation is one `kind` of sign-request
  (`chain_link`), so the "agent proposes, holder disposes" rule covers it. No
  autonomous cross-chain binding.
- **Address Scheme** — the HONE side of an account is its `hh1…` address (bech32 of
  the posting/account pubkey). Cross-chain binding attaches *external* wallets to that
  HONE identity; it does not change the `hh1…` address.
- **Unlinkable role keys** — same privacy logic, extended across chains: a holder can
  correlate their own identities; outsiders cannot.
- **2FA (`TwoFactor`)** — a bound external wallet can back a per-slot 2FA policy
  (`SetKeyPolicy`) without revealing which address maps to the slot — the same
  commitment-not-address principle.

---

## Build Notes (after sign-off)

- Add a `visibility` / `mode` field to `VerifyChainLink` (`"private"` | `"public"`),
  default `"private"`. Private mode: store commitment + mode, **omit**
  `signed_message` + `signature` from persisted `chain_proofs[chain]`.
- Keep `recover_chain_address` verification at submit time (proof still required to
  create the binding) — private mode changes only what is *retained*, not what is
  *checked*.
- `hone sign-request` gains a `chain_link` kind: build the challenge + commitment,
  route to the holder for external-wallet signing, submit the resulting
  `VerifyChainLink`.
- No genesis change. No new keys. No wallet regeneration.

---

_Not a privacy chain — but your keys and your chains are yours to link, privately,
when you choose._
