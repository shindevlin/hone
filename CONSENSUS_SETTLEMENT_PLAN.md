# Consensus and Settlement Plan

Branch: `security-hardening-audit`

## Goal
- Separate timekeeping, work execution, verification, and settlement.
- Keep clocks narrow: they seal time, not rewards.
- Keep workers narrow: they propose work, not finalization.
- Keep verifiers narrow: they inspect work and propose acceptance/rejection.
- Keep reviewers narrow: they arbitrate disputes and rare human-reviewed edge cases.

## Intended Flow
1. Clock nodes publish epoch seals.
2. A requester opens an inference request with an attached fee budget.
3. A worker completes the job asynchronously and submits a work product later.
4. Computer verifiers inspect the submission and produce signed verdicts.
5. If there is disagreement or the requester explicitly escalates, reviewer nodes adjudicate.
6. Settlement finalizes against the request/submission/verdict state, not against the original request epoch.

## Economic Rules
- Inference is paid separately from review.
- A challenge requires payment from the challenger/requester.
- Review is paid by the requester and is non-refundable.
- If a challenge is upheld:
  - inference fee is refunded according to the policy for the request
  - review fee is not refunded
  - challenge bond may be returned if the challenge was valid, or partially forfeited depending on policy
- If a challenge is denied:
  - requester loses both inference and review fees
  - challenge bond is forfeited

## Reviewer Model
- Default path: computer reviewers handle most cases.
- Human reviewers are reserved for expensive or ambiguous edge cases.
- Reviewer eligibility requires stake.
- Reviewer selection should be weighted against concentration:
  - larger stake should not dominate selection linearly
  - use inverse-stake or diminishing-return weighting
  - combine with reputation and anti-sybil controls
- Reviewer verdicts should be signed and auditable.
- Committee dissent should be event-scoped:
  - reviewers who vote against the winning committee verdict lose stake only for that review event
  - dissenter count should feed reviewer reputation so repeated contrarians are weighted down in future selection

## Anti-Sybil / Anti-Capture Rules
- Require locked stake to participate in reviewer selection.
- Penalize false or negligent verdicts with slashing and reputation loss.
- Use weighted random selection from eligible reviewers instead of open self-selection.
- Prevent self-review and same-controller review.
- Prefer multiple independent reviewer nodes for high-value disputes.

## Consensus Drift Prevention
- Introduce a frozen per-request/per-epoch snapshot that contains:
  - sealed epoch state
  - request metadata
  - worker submission hash
  - verifier verdict hash
  - reviewer verdict hash if any
  - fee pool state
- Settlement must hash and replay against that snapshot.
- Any proposal that references a different snapshot is invalid.

## Immediate Implementation Targets
- Add a formal request/submission/verdict state machine.
- Remove reward logic from clock consensus.
- Make reward/finalization proposals consume frozen snapshots.
- Add challenge fee accounting and review-fee handling.
- Add reviewer selection policy with stake weighting and slashing hooks.
- Add tests for:
  - asynchronous completion across many epochs
  - challenge fee payment
  - upheld vs denied challenge accounting
  - reviewer selection bias
  - self-review rejection
  - snapshot mismatch rejection

## Current Branch Progress
- Implemented the first pass of the inference lifecycle refactor in `src/services/inferenceMarket.js`.
- Added explicit job review and challenge routes in `src/routes/inferenceMarketRoutes.js`.
- Added delayed finality sweeping in `src/services/marketplaceSweep.js`.
- Job open now stores review fee, challenge fee, review mode, and a challenge window.
- Miner submission no longer auto-settles a job.
- Reviewers now have a separate review step; appeal review can finalize a challenged job.
- Challenges are paid separately from the job escrow and are only locked when the challenge is filed.
- Self-review is blocked at the service layer.
- Finalized inference jobs now flow into cross-chain finality announcements in `src/services/crossChainFinality.js`, which batch sealed work into per-target-chain files under `anchors/cross-chain/`.
- The reason for the cross-chain announcement layer is to give the challenge window a real protocol boundary: once the deadline passes, the finalized work set becomes a frozen commitment that external chains can consume without reopening disputes.

## Open Follow-Ups
- Wire reviewer eligibility to stake and reputation.
- Add explicit verifier-node selection rules instead of any authenticated user acting as reviewer.
- Decide whether challenge bonds should be refunded on upheld disputes or partially retained.
- Add regression tests for the new accepted / challenged / appealed payout branches.
