# BTCPC Private Authorization Implementation Plan

Created: 2026-04-23

This plan turns the private authorization stack in `docs/PRIVATE_AUTH_STACK.md` into an execution order.

## Goal

Build a BTCPC-native private authorization system where:

- a spend on BTCPC or any supported target chain can require approval from a separate supported chain,
- the approval chain is chosen by the user from an allowed list,
- BTCPC verifies a normalized approval receipt before executing the spend,
- the approval wallet can remain undisclosed to BTCPC when the backend supports it,
- Bitcoin and Lightning ship first as existing-wallet verifiers,
- portable ZK comes next through a backend-agnostic verifier slot,
- BTCPC-native ZK is the later privacy upgrade, not the initial dependency.

## Design Constraints

1. Do not lock the protocol to one wallet vendor or one chain ecosystem.
2. Keep the external approval format chain-neutral.
3. Make Bitcoin and Lightning usable with existing wallets first.
4. Keep chain-specific verification in adapters, not in the policy model.
5. Preserve the current BTCPC execution path until each verifier mode is covered by tests.
6. Add BTCPC-native ZK only after the receipt-based protocol is stable.

## Current Starting Point

The repo already has:

- a stack contract in `docs/PRIVATE_AUTH_STACK.md`,
- a roadmap entry in `docs/ROADMAP.md`,
- initial private-auth service/controller/route scaffolding,
- tests for the current adapter layer.

This plan assumes the stack contract is the source of truth and focuses on completing the full rollout.

## Runtime Staging

The code path is intentionally staged off by default.

- `HONE_PRIVATE_AUTH_ENABLED=false` keeps the approval flow discoverable in code without activating it.
- The future update should turn that flag on only after the chain adapters and UI are approved for release.

## Wave 0 — Policy Core and API Contract

### Objective

Make private authorization a first-class account policy, independent of the verifier chain.

### Deliverables

- stable policy schema for:
  - enabled / disabled
  - threshold
  - approved chains
  - hidden approval factors
- normalized transfer challenge schema
- normalized approval receipt schema
- policy read/write endpoints that do not expose raw approval wallet identities
- transfer gating in every BTCPC execution path that can move funds

### Implementation notes

- Store only commitments or equivalent redacted factor metadata.
- Treat the challenge id and nonce as mandatory replay protections.
- Keep the API shape chain-neutral so the UI can swap verifier chains without changing the transfer model.

### Exit criteria

- a transfer cannot execute if private auth is enabled and no receipt is present,
- a transfer receipt can be validated against the exact sender, recipient, amount, policy id, and expiry,
- policy storage does not require chain-specific branches outside the adapter layer.

### Tests

- policy create/update/read
- challenge creation
- replay rejection
- expiry rejection
- missing-receipt rejection

## Wave 1 — Bitcoin Verifier Path

### Objective

Ship Bitcoin as the first external approval chain using existing wallets and a simple verifier mode.

### Deliverables

- Bitcoin approval enrollment flow
- signed challenge verification path
- account linkage for Bitcoin approval factors
- BTCPC receipt normalization for Bitcoin approvals

### Implementation notes

- Use Bitcoin message signing or the closest existing-wallet signature flow available to the linked wallet.
- Keep privacy out of scope for this wave; the purpose is security, compatibility, and optics.
- Do not require Bitcoin to participate in the BTCPC spend directly.
- BTCPC should only require a valid Bitcoin approval receipt before executing the spend.

### Exit criteria

- a user can enroll a Bitcoin approval factor,
- a user can authorize a BTCPC spend with a Bitcoin signature,
- BTCPC rejects Bitcoin receipts that do not match the current challenge.

### Tests

- enrollment challenge/verification
- valid Bitcoin approval receipt
- invalid signer rejection
- mismatched challenge rejection

## Wave 2 — Lightning Verifier Path

### Objective

Add Lightning as a second approval chain using invoice-based approval rather than forcing arbitrary wallet signatures.

### Deliverables

- Lightning approval enrollment flow
- invoice creation and settlement verification
- normalized Lightning receipt format
- BTCPC policy support for Lightning approvals

### Implementation notes

- Prefer invoice settlement / paid receipt as the verifier primitive.
- Keep BOLT11 support first if it is the easiest wallet-compatible path.
- Add BOLT12 / blinded-path support as the privacy upgrade path later, not as a blocker for launch.

### Exit criteria

- a user can enroll a Lightning approval factor,
- BTCPC can issue an approval invoice for a challenge,
- BTCPC can confirm the invoice was paid and bind that payment to the exact challenge.

### Tests

- invoice request generation
- paid invoice verification
- unpaid invoice rejection
- wrong-challenge invoice rejection

## Wave 3 — Existing Signature Chains

### Objective

Keep compatibility with the existing signature-based chains already present in BTCPC while normalizing them into the new approval model.

### Supported chains

- `evm`
- `solana`
- `ton`

### Deliverables

- adapter contract parity across existing signature chains
- factor commitments stored in the same policy model
- one transfer authorization flow regardless of chain

### Implementation notes

- These chains are bridge support while the new private-auth stack matures.
- Do not let the old signature model leak into the public API once the new receipt format is active.

### Exit criteria

- each supported signature chain can produce a valid private-auth receipt,
- the transfer path does not need chain-specific branching outside the adapter layer.

### Tests

- chain-specific approval receipt verification
- threshold satisfaction
- chain mismatch rejection

## Wave 4 — Portable zkVM Backend

### Objective

Add a portable ZK verifier slot so BTCPC can verify a proof backend without being trapped in one chain.

### Recommended direction

Use a portable zkVM-style backend first, then wire it into BTCPC through the same receipt contract:

- SP1-style backend where appropriate,
- RISC Zero-style backend where appropriate,
- Noir-compatible verifier path where it fits the target chain.

### Deliverables

- `zkvm` approval chain/backend support in the adapter layer
- verifier interface that accepts a generic proof object
- proof receipt normalization
- proof backend configuration in environment and policy metadata

### Implementation notes

- BTCPC should verify a generic proof receipt, not a proof format tied to a single chain.
- Keep proof generation outside BTCPC at first.
- The first proof backend should prove only the minimum receipt semantics:
  - valid approval source
  - exact challenge binding
  - non-replay

### Exit criteria

- BTCPC can verify one portable proof backend end-to-end,
- the same policy and transfer schema still works with the Bitcoin and Lightning paths.

### Tests

- proof verification success path
- invalid proof rejection
- backend mismatch rejection
- replay rejection

## Wave 5 — BTCPC-Native ZK

### Objective

Move from external proof verification to BTCPC-native proof generation and validation.

### Deliverables

- BTCPC-native proof generation tooling
- BTCPC-native verifier backend
- hidden-approval factor enrollment with stronger unlinkability
- optional privacy-preserving factor rotation / re-enrollment

### Implementation notes

- This wave should reuse the same challenge, receipt, and policy interfaces.
- Only the proof backend changes.
- BTCPC-native ZK should not force a protocol redesign.

### Exit criteria

- BTCPC can verify proofs produced by its own ZK tooling,
- users can migrate from external approval modes without changing the spend UX.

### Tests

- BTCPC-native proof generation
- verifier compatibility with the existing receipt model
- re-enrollment and factor rotation

## Wave 6 — UX, Policy Management, and Safety Hardening

### Objective

Expose the policy system cleanly to users and make failure modes obvious.

### Deliverables

- UI for choosing the approval chain per policy or per transfer
- UI for threshold and factor management
- clear challenge details:
  - sender
  - recipient
  - amount
  - expiry
  - approval chain
  - threshold required
- audit-safe logging with redacted approval identity data

### Implementation notes

- The user should be able to choose Bitcoin, Lightning, or a ZK-backed verifier without learning protocol internals.
- The UI should make it clear when the approval wallet is hidden from BTCPC and when it is not.

### Exit criteria

- a user can enable, inspect, and disable private auth without a support ticket,
- the transfer flow is understandable and hard to misread,
- logs do not leak raw approval wallet identities.

### Tests

- policy UI integration
- challenge UI rendering
- redaction checks
- basic end-to-end transfer happy path

## Wave Ordering Rationale

1. Get the policy and receipt contract right first.
2. Ship Bitcoin because it is the most valuable existing-wallet verifier and easiest to explain.
3. Add Lightning because it gives another recognizable approval source and fits the “verify on another chain” model.
4. Keep the existing signature chains working as bridge support.
5. Add portable ZK once the protocol is stable.
6. Add BTCPC-native ZK last so the interface stays stable while the backend gets stronger.

## Non-Goals For The First Rollout

- No custom wallet build.
- No BTCPC-native ZK as the first verifier.
- No single-chain lock-in.
- No requirement that the approval chain and execution chain be the same.
- No privacy-preserving Bitcoin or Lightning scheme until the basic verifier path is stable.

## Definition of Done

The private authorization stack is ready for implementation once:

- the policy schema is stable,
- the receipt schema is stable,
- Bitcoin approval works,
- Lightning approval works,
- existing signature chains are normalized,
- one portable ZK backend is wired,
- and the BTCPC-native ZK wave can be added without changing the public API.
