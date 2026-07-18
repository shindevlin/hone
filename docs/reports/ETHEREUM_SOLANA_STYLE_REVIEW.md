# HONE Review Report: Logic, Security, and Documentation

Date: 2026-04-28

## Scope

This report covers three things:

1. Logic holes and security threats found in the current codebase.
2. Documentation gaps compared with the structure used by Ethereum and Solana developer docs.
3. A concrete documentation shape HONE can adopt so new contributors can understand the system and extend it safely.

The review used the generated code wiki plus the current docs tree as the local source of truth, and compared HONE documentation structure against the official Ethereum and Solana developer docs.

## Executive Summary

HONE already has a large body of product, roadmap, and security documentation, but it is not organized like a protocol reference. Ethereum and Solana docs start with primitives, then execution, then security and composition. HONE docs are still mostly narrative, roadmap-driven, and feature-first.

The most important code issues found are:

1. `privateAuthorization` is a stub that always returns `verified: true`, which disables the intended high-value transfer gate entirely.
2. `POST /api/storage/heartbeat` is unauthenticated, so any caller can spoof storage host liveness and influence reward/selection logic.
3. `POST /api/storage/files` appears broken as written because the server generates `storage_id` after requiring a signature over that same `storage_id`.
4. Clock reward anti-self-credit relies on witness bookkeeping that is not obviously cryptographically bound to the heartbeat sender, so the trust boundary is weaker than the comment implies.
5. Replayed finality snapshots can restore a negative spendable HONE balance for a non-system wallet. That is a chain integrity bug, not a display issue, because `stateStore` accepts persisted balances during hydration without a non-negative invariant check.

The documentation recommendation is to reorganize HONE docs into the same conceptual sequence used by Ethereum and Solana:

- What is an account?
- What is a transaction / entry?
- What are the execution primitives?
- How do security, signing, replay protection, and authorization work?
- How do you extend the system safely?

## Findings

### 1. Private authorization is bypassed completely

File: [`src/services/privateAuthorization.js`](/mnt/btcpc-storage/repos/hone/src/services/privateAuthorization.js)

The service is explicitly a stub and `verifyTransferAuthorization()` always returns `verified: true`.

Impact:

- Any route or controller that relies on this service gets no actual protection.
- The system advertises a high-value transfer gate, but the current implementation does not enforce one.
- This is a direct security bypass, not just a missing feature.

Relevant callers:

- [`src/explorer/server.js`](/mnt/btcpc-storage/repos/hone/src/explorer/server.js)
- [`src/controllers/walletController.js`](/mnt/btcpc-storage/repos/hone/src/controllers/walletController.js)
- [`src/routes/walletRoutes.js`](/mnt/btcpc-storage/repos/hone/src/routes/walletRoutes.js)
- [`src/routes/botRoutes.js`](/mnt/btcpc-storage/repos/hone/src/routes/botRoutes.js)

### 2. Storage heartbeats are spoofable

File: [`src/routes/storageRoutes.js`](/mnt/btcpc-storage/repos/hone/src/routes/storageRoutes.js)

`POST /heartbeat` accepts a `host` in the request body and has no authentication.

Impact:

- Any remote caller can impersonate a storage host.
- Active-host lists can be polluted.
- Reward or host-selection logic that depends on heartbeat freshness can be manipulated.

This is especially concerning because other heartbeat routes in the repo are authenticated, so this endpoint stands out as an inconsistent trust boundary.

### 3. File creation signature flow is likely broken

File: [`src/routes/storageRoutes.js`](/mnt/btcpc-storage/repos/hone/src/routes/storageRoutes.js)

`POST /files` generates `storageId` on the server, then validates a signature over `{ owner, storage_id: storageId, timestamp }`.

Impact:

- The client cannot know the generated `storageId` ahead of time.
- Unless some other client path precomputes the same ID, the request cannot be signed correctly.
- This looks like a functional dead end, not a subtle edge case.

This should be treated as a blocking logic bug.

### 4. Clock self-credit protection depends on weakly bound witness data

Files:

- [`src/chain/blockProposal.js`](/mnt/btcpc-storage/repos/hone/src/chain/blockProposal.js)
- [`src/p2p/protocol.js`](/mnt/btcpc-storage/repos/hone/src/p2p/protocol.js)

The code tries to prevent a proposer from counting its own heartbeat unless there is at least one witness. That is directionally correct, but the witness record is populated from P2P message metadata and the current flow does not make the witness identity as strong as the comment suggests.

Impact:

- If the witness source is spoofable or can be replayed by a relay, clock eligibility may be inflated.
- The reward logic depends on an assumption that the witness is independent, which is not obviously enforced at the transport layer.

This is a medium-to-high risk design concern because it affects reward correctness.

### 5. Negative spendable balances can survive replay

Files:

- [`src/chain/stateStore.js`](/mnt/btcpc-storage/repos/hone/src/chain/stateStore.js)
- [`src/chain/replay.js`](/mnt/btcpc-storage/repos/hone/src/chain/replay.js)

The replay/finality path can hydrate a negative HONE balance into a normal wallet account and keep it in memory. In the current chain data, `natoshisakamoto` replays to a negative spendable balance even though the account is not a system account.

Impact:

- A wallet can enter the live state in an invalid negative-balance condition.
- Any downstream logic that assumes non-negative spendable balances can make incorrect decisions.
- Because the state is loaded from persisted chain data, the issue can survive restarts and propagate into new API responses.

This should be treated as a must-fix chain integrity issue.

## Documentation Assessment

### What HONE already does well

- There is a strong product narrative in `README.md` and `docs/START_HERE.md`.
- Security-specific notes already exist in `docs/security/`.
- The generated code wiki gives a useful map from feature names to code communities.
- The docs mention important domain concepts: roles, keys, tokenomics, storage, commerce, and finality.

### What is missing compared with Ethereum and Solana docs

Ethereum and Solana both lead with primitives and operational rules:

- Ethereum: accounts, transactions, smart contracts, security, verification.
- Solana: accounts, programs, instructions, transactions, PDAs, CPIs, and limits.

HONE documentation does not yet present the system in that same order. It is harder than it should be for a new contributor to answer:

- What is the HONE equivalent of an account?
- What is the HONE equivalent of a transaction or instruction?
- What is canonical state?
- Which actions are signed by which key?
- Which operations are on-chain versus off-chain?
- Which routes are public, authenticated, or owner-only?
- What parts are consensus-critical versus convenience features?

### Recommended HONE documentation shape

#### 1. Core concepts

Create a conceptual reference that answers:

- Accounts and identities
- Keys and key roles
- Entries, blocks, and epochs
- Chain state versus derived state
- Role model: miner, clock, verifier, storage host, sensor, gateway, service host
- Finality tiers and anchoring

#### 2. Execution model

Explain how HONE actually processes work:

- How requests become ledger entries
- How entries become blocks
- How blocks are replayed into state
- How rewards are computed and distributed
- How storage and commerce fit into state replay

#### 3. Security model

Document the trust boundaries clearly:

- What requires posting-key signing
- What requires active-key signing
- What is intentionally public
- What is authenticated by session or token
- What is currently stubbed or experimental

This section should explicitly call out any current stub implementations so readers do not confuse them with production security controls.

#### 4. Extension guides

Add “how to build on HONE” pages for:

- Running a miner or clock node
- Adding a storage feature
- Adding a new ledger entry type
- Extending commerce
- Integrating with wallets and bots
- Reading chain state safely from external apps

#### 5. Reference pages

Add API-style docs for the major subsystems:

- Chain state
- P2P protocol
- Storage
- Mining and reward distribution
- Wallet / auth flows
- Cross-chain anchoring

## Documentation Rework Plan

The fastest useful restructure is:

1. Keep `README.md` as the product landing page.
2. Make `docs/START_HERE.md` the onboarding path for humans and agents.
3. Add a new conceptual reference section in `docs/` for protocol primitives.
4. Link the generated code wiki from the docs index as the code-level map.
5. Keep security docs separate, but link them from the conceptual reference.

## What To Fix First

Prioritized implementation fixes:

1. Replace the `privateAuthorization` stub with a real authorization path or remove the feature from public routes until it is real.
2. Add authentication or signed proof to storage heartbeats.
3. Fix the file creation signature flow so the client can sign a stable, known payload.
4. Tighten clock witness validation so reward eligibility cannot be inflated by relay metadata.
5. Enforce a non-negative spendable balance invariant during replay and finality hydration.

## Source References

Official docs reviewed for structure and conceptual ordering:

- Ethereum accounts: https://ethereum.org/developers/docs/accounts
- Ethereum transactions: https://ethereum.org/developers/docs/transactions
- Ethereum smart contract intro: https://ethereum.org/developers/docs/smart-contracts/
- Ethereum smart contract security: https://ethereum.org/developers/docs/smart-contracts/security/
- Ethereum contract verification: https://ethereum.org/developers/docs/smart-contracts/verifying/
- Solana core concepts: https://solana.com/docs/core
- Solana accounts: https://solana.com/docs/core/accounts
- Solana programs: https://solana.com/docs/core/programs
- Solana instructions: https://solana.com/docs/core/instructions/instruction-structure
- Solana transactions: https://solana.com/docs/core/transactions
- Solana CPI: https://solana.com/docs/core/cpi

## Repo References

- [`README.md`](/mnt/btcpc-storage/repos/hone/README.md)
- [`docs/START_HERE.md`](/mnt/btcpc-storage/repos/hone/docs/START_HERE.md)
- [`docs/INDEX.md`](/mnt/btcpc-storage/repos/hone/docs/INDEX.md)
- [`docs/TECHNICAL_DEEP_DIVE.md`](/mnt/btcpc-storage/repos/hone/docs/TECHNICAL_DEEP_DIVE.md)
- [`docs/security/SECURITY_CHECKLIST.md`](/mnt/btcpc-storage/repos/hone/docs/security/SECURITY_CHECKLIST.md)
- [`docs/security/P2P_AUTH_ANALYSIS.md`](/mnt/btcpc-storage/repos/hone/docs/security/P2P_AUTH_ANALYSIS.md)
- [`docs/code-wiki/README.md`](/mnt/btcpc-storage/repos/hone/docs/code-wiki/README.md)
- [`docs/code-wiki/index.md`](/mnt/btcpc-storage/repos/hone/docs/code-wiki/index.md)
