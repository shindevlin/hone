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

1. **Self-attested at creation.** A HONE wallet derives its own external-chain wallets
   from the same seed (§ Auto-Binding), so at creation it already *holds* those keys
   and can **self-prove control** with no user friction. The binding is created by the
   wallet, for its own keys — not an agent acting on the holder's behalf, and never at
   genesis. (A holder can still add a binding to an *externally-held* wallet later; that
   path is holder-initiated and sign-request-gated — § Externally-Held Wallets.)
2. **HONE learns only "controls chain X."** The chain records that the account controls
   *a* wallet on a chain, plus a **permanent commitment** — never the raw address. This
   binding is durable: it is the anchor for cross-chain HONE claims (§ Claim Ledger).
3. **Private by default (ZK target).** The chain stores a **commitment** (a hash), never
   the address, and — in the ZK target — the node never sees the address even transiently.
   The account can *prove/open* a link when it chooses; observers cannot *derive* one.

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
- Cross-chain binding is **per-account**, self-attested at wallet creation — not a
  launch-time property forced on all founders in genesis.

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

## Auto-Binding at Wallet Creation

A HONE wallet is derived from **one BIP-39 mnemonic**, which yields both the six HONE
role keys *and* external-chain wallets (evm, bitcoin, solana — see the whitepaper key
table). So at the moment of creation the wallet **already holds the private keys** for
its own external wallets. It does not need the user, and it does not need an external
signer — it can prove control of its own keys to itself.

**Default-on, per enabled chain.** When a wallet is created, for each enabled external
chain it:

1. Derives the external wallet from the seed (already done as part of creation).
2. Self-signs the challenge `hone:link:{account}:{chain}:{nonce}` with that external
   wallet's key.
3. Submits a `VerifyChainLink` (private mode) that the node verifies and records as a
   **permanent** binding: `controls[chain] = true` + the commitment.

The result HONE stores is exactly *"this account controls a wallet on chain X"* — a
durable fact, no address. This is automatic (no per-chain approval prompts) because it
is the wallet attesting to **its own** keys at birth, which is categorically different
from an agent moving value or binding a wallet the holder must fetch a signature for.

A wallet may set a flag to **skip** auto-binding for a chain (or all chains) if it wants
zero cross-chain footprint — but the default is to bind every enabled chain at creation,
so cross-chain identity and claim-eligibility exist from the start.

> This differs from the ZK caveat above only in *who signs*: here the wallet signs its
> own derived key automatically; in the externally-held case (below) a human signs with
> an outside wallet through a sign-request.

---

## Claim Ledger (why the binding is permanent)

The binding is not just a privacy nicety — it is the **permanent anchor for cross-chain
HONE claims.** When HONE becomes claimable on other chains (e.g. an external-chain
distribution or bridge redemption), the chain must know **how much** each external
identity is owed. The binding is that record.

**Commit-now / reveal-at-claim** (the model that keeps it private *and* claimable):

1. **At creation (commit).** The wallet posts a **permanent commitment**
   `C = H(chain, address, claim_secret)` alongside `controls[chain]=true`. The node
   verifies control and stores `C` **forever**; it never learns the address (ZK target)
   — only that a valid, controlled wallet on that chain is committed. `claim_secret` is
   held by the wallet.
2. **Accrual.** Cross-chain entitlement accrues to the **HONE account** as normal (the
   account earns HONE). `C` permanently ties that account's cross-chain-claimable
   entitlement to an unrevealed external control-set on each bound chain.
3. **At claim (reveal).** When claims open, the holder **opens `C`** — presents
   `(address, claim_secret)` — proving the commitment was theirs. The redemption path
   (HONE claim contract / bridge) pays **exactly the accrued amount** to that address.
   Only at this moment, and only for the address being claimed, does an address surface —
   and only to the extent redemption requires it.

Properties:
- **Permanent.** `C` is written once and never mutated; the claim record cannot be lost
  or retroactively unbound.
- **Private until claim.** No address is stored; the holder controls if/when to reveal,
  per chain, at claim time.
- **Exact reconciliation.** Because entitlement accrues to the account and `C` ties the
  account to the external control-set, the owed amount per external identity is
  computable — the reveal just names *where* to pay.
- **Single-use per commitment.** Opening `C` consumes it for its claim; the nonce/secret
  make it non-replayable.

> This is the standard private-airdrop / stealth-claim shape: commit to an entitlement
> now, reveal the payout address only at redemption.

---

## The Flow — externally-held wallet (holder-initiated, sign-request-gated)

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

## Build Phasing

**Build nothing until the portable ZK verifier slot exists.** The privacy target —
node *never* sees the external address, even transiently — requires the ZK verifier
from the private-auth plan (Wave 4: a portable zkVM backend, SP1 / RISC-Zero / Noir,
verifying a generic proof receipt). Until that lands, this spec is a **design of
record only**; do not ship a commitment-only fallback that lets the node see the
address at verify time. When the ZK slot is available, the binding proof becomes
"I control a valid wallet on chain X, committed as `C`" with no address disclosure.

Phasing, once the ZK slot is ready:

1. **Claim-commitment ledger.** A permanent `chain_bindings[chain] = { controls:true,
   commitment: C }` on the account; write-once, never mutated. `C = H(chain, address,
   claim_secret)`.
2. **Auto-binding at creation.** Wallet creation, per enabled chain, ZK-proves control
   of its own derived external wallet and posts the commitment. Default-on; a
   `skip_chain_binding` flag opts a wallet out.
3. **`VerifyChainLink` ZK mode.** Accept a ZK proof receipt in place of the
   recover-address-from-signature path; store commitment + `controls[chain]`, never the
   address. Keep the legacy signature path only for explicitly-public bindings.
4. **Externally-held bindings** via `hone sign-request` `chain_link` kind (holder signs
   with an outside wallet through the sign-request flow).
5. **Claim/redemption path.** Opening `C` at claim time: holder reveals `(address,
   claim_secret)`, redemption pays the accrued amount to that address; commitment is
   consumed.

- No genesis change. No new keys. No wallet regeneration. Block-0 hash unchanged.

---

## Relationship to Other Specs (additions)

- **Portable ZK slot (PRIVATE_AUTH Wave 4/5)** — the binding proof and the claim
  reveal both verify through the generic proof-receipt interface, not a chain-specific
  format. This spec is a *consumer* of that slot and is build-gated on it.
- **Sign-Request Protocol** — used only for the **externally-held** binding path
  (a wallet the holder must fetch a signature for). Auto-binding at creation does **not**
  route through sign-request — the wallet signs its own derived keys automatically.

---

_Not a privacy chain — but your keys and your chains are yours. Bound at birth, private
until you claim._
