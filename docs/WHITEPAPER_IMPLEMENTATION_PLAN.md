# BTCPC Whitepaper Implementation Plan

Created: 2026-04-14

## Scope

This plan reconciles the BTCPC v3.0 whitepaper with the current codebase and records implementation work so another engineer or LLM can resume without redoing the audit.

## User Clarifications

- Epochs are 30-second consensus epochs. There must be no 5-minute fallback.
- "Doublings" are emission periods that should double in length while reward-per-epoch remains the same within the period. Working recovered schedule: 1 week, then 2 weeks, 4 weeks, 8 weeks, and so on.
- P2P signatures matter most for spend and authority-bearing actions. The whitepaper currently says all P2P messages are signed; implementation should enforce signatures at minimum for spend, account, clock, storage, sensor, and block/finality authority messages.
- Finality anchoring to external chains should be wallet-funded or sponsor-funded. Users/projects should be able to fund chain-specific anchor wallets, and local BTCPC rewards for facilitating external finality should be documented in the whitepaper.
- TOTP should exist as protocol/app infrastructure and SDK support so projects can enforce protected actions without a human operator.
- Human wallets should also be able to opt into TOTP when they choose; this must protect local secretStore-backed wallets, not only Mongo-backed accounts.
- TOTP should use standard authenticator apps such as Google Authenticator/Authy/Aegis: BTCPC shows an `otpauth://` QR/code during setup, the user enters the 6-digit code for sensitive actions, and chain storage should contain only policy/commitment data plus the protected action list, never the raw seed.
- Verifier strictness should scale with adoption. Early network: low verifier quorum. Larger network: opt-in verifier nodes, larger panels, higher verification coverage.

## Priority Fixes

1. Storage challenges
   - Replace placeholder "any 32 bytes is valid" response checking with expected range-hash verification.
   - Keep the API testable with injected response bytes and deterministic offsets where useful.

2. Epoch duration
   - Remove 5-minute protocol fallbacks.
   - Centralize epoch duration as 30 seconds.
   - Treat clock-node consensus as authoritative for epoch close, not local fallback constants.

3. P2P signature enforcement
   - Default required signatures to true for protected message classes.
   - Keep a clearly named legacy/test opt-out only for local tests if needed.
   - Ensure MEMPOOL_ENTRY protected types validate account signatures by role.

4. Account names
   - Align username validation with whitepaper.
   - Proposed protocol rule: 3-20 chars, lowercase letters/numbers/hyphens, no leading/trailing hyphen.
   - Reserve dangerous/confusing names including `wallet`.

5. TOTP and SDK support
   - Locate existing TOTP flows and define the exact transaction/auth envelope.
   - Ensure human wallets can opt in and have TOTP enforced on protected wallet/staking/delegation actions.
   - Store only on-chain public policy/commitment for TOTP, not the TOTP secret. The secret belongs in the user's authenticator/wallet-secure storage, along with the 6-digit authenticator flow for protected actions.
   - Add SDK helpers for challenge generation, TOTP verification payloads, and project-side enforcement.
   - Document which actions require TOTP by default.

6. External finality and bridge
   - Convert placeholder anchor payloads into real Merkle-batched finality payloads.
   - Add sponsor wallet funding model and anchor-fee accounting.
   - Add chain-specific submitter interfaces for Base, Arbitrum, Ethereum, Bitcoin, and future chains.
   - Document user-funded finality as an optional service with BTCPC rewards for finalizers.

7. Verifier scaling
   - Add network-size-aware verifier quorum/panel sizing.
   - Make verifier role opt-in.
   - Increase verification coverage as miner count/job volume grows.

8. TON wallet/address derivation
   - Replace simplified TON placeholder address derivation with a real TON wallet address implementation or clearly gate TON as experimental. Current implementation should stay documented as a raw public-key link target derived from the shared BIP-39 mnemonic.

9. Service/oracle chain integration
   - Finish dispatcher wiring where modules are currently standalone primitives.
   - Add tests that ledger entries route through block finalization, not only local service calls.

10. Emission doublings
    - Implement the recovered weekly doubling schedule: 1 week, 2 weeks, 4 weeks, 8 weeks, etc.
    - Keep reward-per-epoch constant within each period so each period's total allotment doubles, then truncate the final period at the 42M cap.
    - Update `emissionSchedule` and whitepaper together so they describe the same mechanics and make clear the consensus path only does reward lookup by epoch.

## Open Design Decisions

- Exact final cap/truncation behavior for the weekly doubling schedule, though the current working assumption is that the cap simply truncates the last period.
- Which P2P messages are mandatory-signed from day one versus allowed unsigned gossip.
- Whether finality funders receive only BTCPC rewards, off-chain fee rebates, or both.
- Exact verifier panel formula by miner count, active job count, and model value.
- Whether more reserved names should be added beyond the current system/confusing-name list.

## Initial Implementation Track

- Start with low-risk correctness fixes that do not touch mining/reward/P2P topology internals unnecessarily.
- Avoid modifying `src/mining/miner.js`, `src/chain/finalizationConsensus.js`, `src/services/escrow.js`, and `src/services/ledger.js` unless explicitly approved.
- Maintain `docs/WHITEPAPER_IMPLEMENTATION_NOTES.md` as the handoff log.
