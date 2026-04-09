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

### Automatic Heartbeat — Cross-Chain Proof of Life

The most elegant approach: if you're active on ANY linked blockchain, your BTCPC dormancy clock resets automatically. You never need to think about BTCPC specifically.

**How it works:**

Every BTCPC account has deterministic cross-chain addresses (EVM, Solana, Bitcoin, TON, Hive) derived from the same mnemonic. The BTCPC network already monitors these addresses for cross-chain mining claims. The same watcher can detect activity:

```
User makes an ETH transfer on Ethereum
  → BTCPC cross-chain watcher sees activity on the linked EVM address
  → Automatically writes a HEARTBEAT entry on the BTCPC ledger
  → Dormancy clock resets for 5 years
  → User never touched BTCPC directly
```

**Supported chains for automatic heartbeat:**
- Ethereum / EVM (any transaction on linked address)
- Solana (any transaction on linked address)
- Bitcoin (any UTXO movement on linked address)
- TON (any transaction on linked address)
- Hive (any custom_json or transfer on linked account)

**Cross-chain activity protects your tokens — but doesn't unlock spending.**

If your linked ETH wallet is active but you haven't signed with your BTCPC key, your tokens are **frozen** (no decay, but can't spend). This protects against the scenario where someone lost their BTCPC mnemonic but their ETH wallet is still active — the tokens are preserved but locked.

To spend frozen tokens: sign any BTCPC transaction (heartbeat, transfer, stake) with the native BTCPC key.

### Three-Tier Dormancy Timeline

| Period | Cross-Chain Active | All Chains Inactive |
|--------|-------------------|-------------------|
| Years 0-5 | Full access | Full access |
| Years 5-10 | **Frozen** — safe, no decay, can't spend until BTCPC native sign | **Decaying** — 10%/year of remaining |
| Years 10+ | **Decaying** — cross-chain alone isn't enough anymore, 10%/year | Still decaying |

At ANY point: one native BTCPC signature (heartbeat, transfer, anything) → unfrozen, clock fully resets, keep whatever remains.

```
Year 0:   1000 BTCPC, active
Year 5:   No BTCPC activity, ETH wallet active → frozen (1000, locked)
Year 8:   Still frozen, still 1000 BTCPC — cross-chain protecting it
Year 10:  Cross-chain grace expires → decay starts (1000 → 900)
Year 11:  900 → 810
Year 15:  590 BTCPC remaining
Year 12:  User signs with BTCPC key → unfrozen, 810 BTCPC, clock resets for 5 years
```

### Other Heartbeat Methods

**1. On-chain recurring contract (set-and-forget)**
Register a `HEARTBEAT_CONTRACT` once. The protocol auto-heartbeats your account every 4 years during epoch finalization. Costs 0.01 BTCPC one-time fee. Cancel anytime.

**2. Delegated heartbeat (estate planning)**
Authorize another account to heartbeat on your behalf using your posting key. Use case: inheritance — "If I'm gone, my family can keep my tokens alive." The delegate can heartbeat but can never move funds (posting key, not active key).

### Summary of Heartbeat Options

| Method | Effort | Setup | Use Case |
|--------|--------|-------|----------|
| Manual `/heartbeat` | 1 tap per 5 years | None | Simple, for active users |
| Cross-chain activity | Zero — automatic | Link chains (already done at account creation) | Crypto-active users |
| Recurring contract | Zero after setup | 0.01 BTCPC one-time | Set-and-forget |
| Delegated | Zero for owner | Share posting key | Estate planning / inheritance |
| Any wallet activity | Zero — automatic | None | Send, receive, stake, inference all count |

### Status

**Not yet implemented.** This is a protocol design for a future version. The chain is only days old — the 5-year clock hasn't started for anyone. Implementation can happen anytime before year 5. The cross-chain watcher infrastructure already exists for mining claims — extending it to heartbeats is straightforward.
