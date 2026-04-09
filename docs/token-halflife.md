# Token Dormancy Recycling — No Lost Tokens, Ever

## The Problem

In Bitcoin, ~4 million BTC are estimated to be permanently lost (dead wallets, lost keys, Satoshi's stash). This reduces effective supply but the tokens are gone forever — no one benefits.

BTCPC should never have permanently lost tokens.

## The Solution: Dormancy Recycling

Wallets that have had no activity (no sends, no receives, no staking, no inference) for 5+ years begin a gradual half-life decay. The decayed tokens flow back into the mining reward pool — extending the emission schedule and benefiting active participants.

### How It Works

1. **Activity clock**: every wallet tracks its last activity timestamp (any ledger entry)
2. **Grace period**: 5 years of zero activity before decay begins
3. **Decay**: after the grace period, 10% of the remaining balance recycles per year
4. **Decay destination**: decayed tokens go to `btcpc_recycle` — a protocol address that feeds back into the block reward pool
5. **Wake up anytime**: any activity (even receiving 1 dream) resets the clock. No penalty for being dormant — just come back before the decay eats too much.

### Example

```
Year 0:  Alice has 1000 BTCPC, goes inactive
Year 5:  Grace period ends. Balance still 1000 BTCPC.
Year 6:  10% decay: 900 BTCPC remains, 100 recycled
Year 7:  10% decay: 810 BTCPC remains, 90 recycled
Year 8:  10% decay: 729 BTCPC remains, 81 recycled
Year 10: 590 BTCPC remains (59% of original)
Year 15: 349 BTCPC remains (35%)
Year 20: 206 BTCPC remains (21%)
Year 30: 72 BTCPC remains (7.2%)

If Alice comes back at year 10: she has 590 BTCPC.
She taps /heartbeat → clock resets. 590 BTCPC is hers for another 5 years.
```

### Why 10% Decay, Not Burn

- **Gentle**: 10% of remaining balance per year — takes decades to meaningfully erode
- **Not a penalty**: this is recycling, not punishment
- **Never zero**: mathematically, the balance approaches zero but never reaches it
- **Reversible**: any activity (or one heartbeat tap) stops the decay instantly
- **Fair**: only truly abandoned wallets lose tokens — and slowly
- **Economic benefit**: recycled tokens extend the emission schedule for active miners
- **59% after 10 years**: even a decade of absence leaves most tokens intact

### Implementation

This would be computed during epoch finalization:
1. Scan all accounts in the SMT
2. For accounts with `lastActivity < now - 5 years`:
   - Compute decay: `decayAmount = balance * 0.10` (10% of remaining per year)
   - Record ledger entry: `DORMANCY_DECAY from: account, to: btcpc_recycle`
   - Update SMT
3. `btcpc_recycle` balance is added to the next epoch's block reward

### Keeping Your Wallet Alive

Any of these resets the 5-year clock:
- Send or receive tokens
- Stake, unstake, delegate
- Use inference
- **Tap "I'm still here" once** (heartbeat — zero cost, zero fee)

The heartbeat is the simplest option. One tap every 5 years. That's it. This is not a penalty — it's proof you still exist. The wallet notifies you at 4.5 years so you never forget.

Available via:
- Telegram: `/heartbeat`
- Explorer settings page: "Keep Alive" button
- CLI: `node bin/btcpc-cli wallet heartbeat`

### Protocol Rules

- Grace period: 5 years (configurable via governance later)
- Decay rate: 10% per year of remaining balance (gentle, not a penalty)
- Minimum activity: any ledger entry, or a zero-cost HEARTBEAT entry
- Burn address (`btcpc_burn`) tokens also decay → recycle (even burns aren't permanent)
- Genesis accounts (shindevlin, reserved names) follow the same rules — no exceptions
- Notification at 4.5 years: Telegram, email, explorer — "tap to keep your BTCPC active"

### Status

**Not yet implemented.** This is a protocol design for a future version. The chain is only days old — the 5-year clock hasn't started for anyone. Implementation can happen anytime before year 5.
