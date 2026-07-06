# BTCPC Relaunch and Genesis Reset Plan

Date: 2026-04-28

This is a planning document, not an implementation.

It records the intended next phase:

- fix the accounting and replay bug first
- reset the chain into a clean new genesis once the logic is verified
- start every account at zero in the new chain
- rewrite the whitepaper, README, and public website to match the relaunch

The goal is to make BTCPC “born again” from a clean epoch zero with validated issuance and reward logic.

## Why a reset is being proposed

The replayed chain state is currently inconsistent:

- one wallet replays to a negative spendable balance
- the visible balances do not match the emission schedule
- the current state does not cleanly explain where the supply lives

If the team wants a fresh, auditable launch with confirmed issuance behavior, a new genesis is the cleanest path.

## Core policy for the relaunch

1. Fix the replay / balance-floor logic first.
2. Verify issuance and recycle flows in a controlled environment.
3. Start a new chain at epoch zero.
4. Start all wallet balances at zero.
5. Update public docs so the whitepaper, README, and website all describe the new launch state.

## What should happen to unearned tokens

Any value that is not legitimately earned under the new rules should flow to `btcpc_recycle`, not be burned.

This should remain a BTCPC principle:

- no burn
- recycle instead
- slow, predictable release back into rewards

The relaunch may choose a slower recycle cadence than the current chain if that is needed to keep emissions stable and auditable. The important part is that recycle remains the recovery path for unearned or unallocated value.

## Proposed relaunch sequence

### Phase 1: fix and verify

- Patch the non-negative balance invariant during replay and finality hydration.
- Reconcile the current chain state against the emission schedule.
- Verify the issuance math and reward splits in tests.
- Confirm the recycle path works as intended when pools are unearned or empty.

### Phase 2: prepare the reset

- Define the last old-chain block that will remain historical reference only.
- Confirm that the new genesis starts with zero wallet balances.
- Identify any invalid or corrupt state that must be documented as retired history rather than carried forward.
- Freeze the old chain state and publish a final pre-reset audit note.
- Choose the new genesis timestamp and epoch zero.

### Phase 3: relaunch

- Generate a new genesis block.
- Seed the chain with zeroed wallet balances and fresh account state.
- Start epoch zero with the corrected emission schedule.
- Begin public mining from the clean state.

### Phase 4: publish the new public docs

Update these surfaces together:

- `docs/HONE_WHITEPAPER.md`
- `README.md`
- `website/index.html`
- `website/app.html`
- `website/install.html`
- `website/vendor.html`
- `website/openclaw.html`

## Documentation changes required

### Whitepaper

Rewrite the whitepaper to describe the new genesis era:

- epoch zero timestamp
- reward schedule and per-epoch issuance
- recycle policy
- wallet migration rules
- finality and replay rules
- what counts as earned versus unearned value

### README

Rewrite the README introduction so it matches the relaunch:

- what BTCPC is at launch
- how to install and join the new chain
- how rewards work in the new epoch-zero system
- how recycle behaves

### Website

Update the public site copy so it reflects the relaunch:

- launch status
- reward language
- genesis/epoch-zero messaging
- recycle policy
- corrected emission values

## Open decisions

These must be decided before the relaunch is committed:

- whether the recycle wallet starts empty or with a bootstrap amount
- whether the new emission schedule changes pool weights or only fixes the current math
- whether any old-chain identities are re-used as zeroed accounts on the new chain or treated as entirely new accounts

## Recommendation

Do not publish the relaunch until:

- the reward math passes replay tests
- the negative balance bug is fixed
- the recycle flow is confirmed
- the whitepaper, README, and web pages all agree on epoch zero and the updated emission story
- the zero-balance genesis state is verified in a fresh chain
