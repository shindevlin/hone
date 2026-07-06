# BTCPC Private Authorization Stack

Date: 2026-04-23

## Goal

Add a BTCPC-native private authorization system for transfers and other spends that can use external chains as the approval source.

The policy owner chooses which chain authorizes the spend. BTCPC executes the spend only after it receives a valid approval receipt from the selected chain.

This is designed as a defense-in-depth layer:

- execution can happen on BTCPC or any supported target chain
- policy can live on another chain
- approval identity can be hidden from BTCPC if the verification backend supports it
- the protocol should not lock BTCPC to one chain or one wallet vendor

## Design Principles

1. Keep policy chain-agnostic.
2. Keep approval receipts chain-specific.
3. Normalize all approvals to one BTCPC challenge format.
4. Treat the verifier as a pluggable backend.
5. Prefer existing wallet ecosystems first.
6. Leave BTCPC-native ZK as a future backend slot, not a separate product path.

## Terms

- `execution chain`: where the transfer happens.
- `approval chain`: where the user authorizes the spend.
- `policy chain`: the chain that is treated as the trust anchor for a given account or transfer.
- `receipt`: the artifact BTCPC verifies before executing.
- `factor`: one hidden approval source enrolled for an account.
- `policy`: threshold and chain settings for private authorization.

## Supported Approval Modes

### Bitcoin

Use a signed challenge as the initial verifier mode.

Why this first:

- existing wallets already support signing
- simple to understand
- strong optics
- no custom wallet required

Approval characteristics:

- chain: `bitcoin`
- verifier: signature recovery or equivalent witness
- privacy: low to moderate
- implementation difficulty: medium

### Lightning

Use a small invoice payment or invoice settlement as the approval signal.

Why this fits:

- existing Lightning wallets already support paying invoices
- the approval can be represented as a paid invoice receipt
- good second-factor feel for users

Approval characteristics:

- chain: `lightning`
- verifier: invoice settlement receipt
- privacy: moderate
- implementation difficulty: medium

### zkVM

Use a portable proof backend for the future ZK verifier path.

The verifier should accept a generic proof object, not a BTCPC-specific circuit format.

Good backend candidates:

- SP1-style zkVM
- RISC Zero-style zkVM
- Noir-compatible proof backend where appropriate

Approval characteristics:

- chain: `zkvm`
- verifier: external proof verifier
- privacy: high
- implementation difficulty: high

### Existing Signature Chains

Support the existing signature-based chains already familiar to BTCPC:

- `evm`
- `solana`
- `ton`

These are useful as a bridge while BTCPC rolls out the new verifier modes.

## Policy Model

Each BTCPC account can enroll hidden approval factors.

Policy fields:

- `enabled`: whether private authorization is active
- `threshold`: required approval count
- `factors`: enrolled hidden factors
- `chains`: unique supported approval chains in use
- `updatedAt`: policy timestamp

Each factor should store only:

- `factorId`
- `chain`
- `commitment`
- optional label
- creation timestamp

BTCPC should avoid storing raw approval wallet addresses in the policy record when the approval mode supports hidden verification.

## Enrollment Flow

1. User requests an enrollment challenge for a chosen approval chain.
2. BTCPC returns a chain-specific enrollment artifact:
   - Bitcoin: challenge string
   - Lightning: invoice or payment request
   - zkVM: proof-friendly challenge object
3. User completes the action in the selected wallet or verifier app.
4. BTCPC verifies the receipt.
5. BTCPC stores a commitment for the factor.
6. BTCPC marks the factor as enrolled.

Enrollment must bind to:

- account
- factor id
- chain
- challenge id

## Transfer Flow

1. User starts a transfer on BTCPC.
2. BTCPC creates a transfer challenge.
3. The challenge includes:
   - sender
   - recipient
   - amount
   - token
   - memo
   - nonce / request id
   - expiry
   - approval chain
   - proof backend if needed
4. BTCPC hands the challenge to the approval chain.
5. User completes the approval in the chosen chain.
6. BTCPC verifies the receipt.
7. BTCPC records the transfer only after threshold approval passes.

## Chain Adapter Contract

Every approval chain should expose the same logical contract:

- `requestEnrollment(account, chain, metadata)`
- `verifyEnrollment(challengeId, receipt)`
- `requestTransferAuthorization(account, challenge)`
- `verifyTransferAuthorization(account, challenge, receipt)`

BTCPC code should not care whether the backend is:

- a Bitcoin signature
- a Lightning invoice
- a zkVM proof
- a future chain-specific proof format

## Receipt Normalization

All verifier backends should map to a normalized approval receipt:

- `approvalChain`
- `requestId`
- `factorId`
- `challengeHash`
- `verifiedAt`
- `result`
- `proofBackend`
- `provider`

The normalized receipt is what the ledger records.

## Security Invariants

BTCPC must ensure:

- challenge replay is rejected
- challenge expiry is enforced
- transfer amount matches the signed or proven challenge
- recipient matches the challenge
- sender matches the challenge
- approval chain matches the enrolled factor
- threshold is met before execution
- the verifier result is bound to the exact challenge

## BTCPC-Native ZK Roadmap

BTCPC-native ZK should be added as a verifier backend, not as a new authorization model.

That means the public interface stays the same:

- request challenge
- get receipt
- verify receipt
- execute transfer

Only the proof backend changes.

Recommended progression:

1. Bitcoin signature verifier
2. Lightning invoice verifier
3. zkVM verifier backend
4. BTCPC-native proof generation tooling
5. optional privacy-preserving factor enrollment

## User Experience

Users should be able to choose their approval chain from a list:

- Bitcoin
- Lightning
- zkVM-backed proof chain
- EVM
- Solana
- TON

UI should clearly show:

- which chain will approve the spend
- what amount and recipient are being authorized
- when the challenge expires
- how many approvals are required

## Operational Requirements

Environment variables:

- `HONE_PRIVATE_AUTH_ENABLED`
- `HONE_LIGHTNING_PROVIDER_URL`
- `HONE_LIGHTNING_PROVIDER_KEY`
- `HONE_LIGHTNING_AUTH_SATS`
- `HONE_ZK_VERIFIER_URL`
- `HONE_ZK_VERIFIER_KEY`
- `HONE_ZK_PROOF_BACKEND`

Operational requirements:

- leave `HONE_PRIVATE_AUTH_ENABLED` off by default until the future rollout is explicitly approved
- keep read-only preview endpoints available so users can inspect the future approval shape without activating it
- keep approval challenge TTL short
- log only normalized receipt metadata
- avoid writing raw approval wallet addresses into public logs
- support chain rotation without reformatting the policy store

## Non-Goals For The First Pass

- custom BTCPC wallet design
- BTCPC-only cryptographic proofs
- chain-specific lock-in
- hidden-signature privacy on Bitcoin or Lightning in the first release
- on-chain proof verification on Bitcoin itself

## Implementation Contract

The implementation should follow this document exactly:

- add adapters, not special cases
- preserve the same challenge format where possible
- keep BTCPC execution logic separate from approval logic
- treat BTCPC-native ZK as one backend among several
- require explicit chain selection for each private-auth spend
