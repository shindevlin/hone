# Token Half-Life — No Lost Tokens, Ever

## The Problem

In Bitcoin, ~4 million BTC are estimated to be permanently lost (dead wallets, lost keys, Satoshi's stash). This reduces effective supply but the tokens are gone forever — no one benefits.

BTCPC should never have permanently lost tokens.

## The Solution: Dormancy Half-Life

Wallets that have had no activity (no sends, no receives, no staking, no inference) for 5+ years begin a gradual half-life decay. The decayed tokens flow back into the mining reward pool — extending the emission schedule and benefiting active participants.

### How It Works

1. **Activity clock**: every wallet tracks its last activity timestamp (any ledger entry)
2. **Grace period**: 5 years of zero activity before decay begins
3. **Half-life**: after the grace period, the wallet loses 50% of its balance per year
4. **Decay destination**: decayed tokens go to `btcpc_recycle` — a protocol address that feeds back into the block reward pool
5. **Wake up anytime**: any activity (even receiving 1 dream) resets the clock. No penalty for being dormant — just come back before the decay eats too much.

### Example

```
Year 0: Alice has 1000 BTCPC, goes inactive
Year 5: Grace period ends. Balance still 1000 BTCPC.
Year 6: Half-life decay: 500 BTCPC remains, 500 recycled to mining pool
Year 7: Half-life decay: 250 BTCPC remains, 250 recycled
Year 8: Half-life decay: 125 BTCPC remains, 125 recycled
...

If Alice comes back at year 7: she has 250 BTCPC.
She sends 1 dream → activity clock resets. No more decay.
```

### Why Half-Life, Not Full Burn

- **Gradual**: gives people years to notice and act
- **Never zero**: mathematically, the balance approaches zero but never reaches it
- **Reversible**: any activity stops the decay
- **Fair**: only truly abandoned wallets lose tokens
- **Economic benefit**: recycled tokens extend the emission schedule for active miners

### Implementation

This would be computed during epoch finalization:
1. Scan all accounts in the SMT
2. For accounts with `lastActivity < now - 5 years`:
   - Compute decay: `decayAmount = balance * (1 - 0.5^(yearsSinceGrace))`
   - Record ledger entry: `DORMANCY_DECAY from: account, to: btcpc_recycle`
   - Update SMT
3. `btcpc_recycle` balance is added to the next epoch's block reward

### Protocol Rules

- Grace period: 5 years (configurable via governance later)
- Half-life: 1 year (50% per year after grace)
- Minimum activity: any ledger entry (send, receive, stake, unstake, delegate)
- Burn address (`btcpc_burn`) tokens also decay → recycle (even burns aren't permanent)
- Genesis accounts (shindevlin, reserved names) follow the same rules — no exceptions

### Status

**Not yet implemented.** This is a protocol design for a future version. The chain is only days old — the 5-year clock hasn't started for anyone. Implementation can happen anytime before year 5.
