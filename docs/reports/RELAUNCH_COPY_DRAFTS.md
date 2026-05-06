# BTCPC Relaunch Copy Drafts and Notes

Date: 2026-04-28

This document keeps the launch notes together in one place so the reset rationale stays clear across the whitepaper, README, and website.

## Why the chain is being reset

The chain is being reset because the current replayed state is not internally trustworthy:

- at least one non-system wallet replays to a negative spendable balance
- the replayed account totals do not match the intended emission schedule cleanly
- the current state makes it hard to tell what was earned, what was recycled, and what is simply corrupted persistence

The goal of the reset is not to erase history for convenience. The goal is to relaunch from a clean epoch zero after proving the logic works, so issuance, recycle, and wallet balances can all be audited from day one. The new chain starts with zero balances for all accounts.

## Balance policy for the relaunch

This is the rule that should be repeated everywhere:

- all wallet balances start at zero on the new genesis
- actual token balances are not migrated or trimmed
- only invalid, corrupt, or unearned protocol-side state from the old chain is documented as retired history
- recycle is the recovery path, not burn

## Whitepaper draft notes

The whitepaper should say:

- BTCPC is relaunching from a new epoch zero
- the chain was reset because the prior state could not be trusted for issuance accounting
- the new launch starts from zero balances for every account
- old-chain balances are not migrated
- unearned or protocol-side residual value remains part of the old chain's retired history and is not carried into the new genesis state
- the emission schedule and recycle rules are explicit and testable

### Suggested replacement for the opening framing

> BTCPC relaunched from a clean genesis after validating its issuance and replay logic. All accounts start at zero in the new chain. Invalid or unearned protocol-side state from the prior chain remains retired on the old ledger, preserving the no-burn rule while restoring a clean epoch-zero accounting model.

### Suggested replacement for the reset rationale section

> The network reset was necessary because the prior replayed state contained an invalid spendable balance and did not cleanly reconcile with the intended emission schedule. Rather than carry corrupt accounting forward, BTCPC restarted from epoch zero with all balances reset to zero and a verified reward model.

## README draft notes

The README should explain the relaunch in plain language:

- BTCPC now starts from a clean genesis / epoch zero
- balances are not migrated
- the project keeps no-burn / recycle
- users can verify issuance by replaying from the new chain root

### Suggested replacement for the top-level description

> Bitcoin Proof of Compute is a sovereign blockchain for real work. BTCPC relaunched from a clean genesis after fixing its replay and issuance accounting. The new chain starts from zero balances, unearned value remains retired on the old ledger, and the chain now begins at a verifiable epoch zero.

### Suggested replacement for the “How it works” bullets

- epoch zero starts from the new genesis timestamp
- rewards are paid from the corrected schedule
- recycle captures fees and unearned state instead of burning it
- wallet balances start at zero
- mining, clocks, storage, and other roles earn from verified work

## Website draft notes

The public website should stop sounding like a pre-launch countdown and instead sound like a relaunch with a verified accounting model.

### Suggested homepage hero

Headline:

> Bitcoin Proof of Compute

Subheadline:

> Relaunched from a clean genesis. All wallet balances start at zero. Rewards, recycle, and epoch-zero accounting are now auditable from the start.

### Suggested site badge or stat line

> Epoch zero active · no burn · recycle instead · verified emission schedule

### Suggested install-page framing

> Join the new chain from epoch zero. Install a node, create a wallet, and verify the reward system on a clean accounting base.

## Communication rule

When describing the reset publicly, do not say:

- that balances were trimmed
- that the chain was “fixed by haircut”
- that user funds were reduced for convenience

Say instead:

- the chain was reset because the accounting was not trustworthy
- all balances start at zero in the new genesis
- invalid or unearned state remains retired on the old ledger
- the chain restarted from a clean epoch zero after validation

## Internal review checklist

Before any public release, confirm:

- whitepaper and README use the same epoch-zero story
- website hero copy matches the whitepaper
- tokenomics text still preserves no burn / recycle
- the new genesis starts with zero balances
- the explanation for the reset is consistent in every surface
