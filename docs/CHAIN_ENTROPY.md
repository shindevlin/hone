# Chain Entropy — Liveness Rewards and Entropy Decay

> **Status: `LIVENESS_REWARDS_ENABLED = false` / `ENTROPY_DECAY_ENABLED = false`**
>
> This feature is implemented and code-ready. It will be activated by governance
> after the documentation reaches users (website page + whitepaper section).
> No funds move until the flag is flipped.

---

## What Is Chain Entropy?

Chain entropy is a mechanism that keeps tokens in active circulation rather than
locked in dormant wallets indefinitely. It does this by slowly returning long-idle
balances to the active economy — funding rewards for participants who are proving
liveness each epoch.

The goal is positive: **reward activity, not punish inactivity.**

No token is ever destroyed. Every dream that enters circulation via chain entropy
is credited to an active wallet or the recycle fund, and flows back through the
normal reward distribution model.

---

## Dormancy Timeline

| Period | Behaviour |
|--------|-----------|
| Years 0–3 | No dormancy rules. Accounts may be idle indefinitely. |
| Years 3–5 | Warning period only. No tokens move. Wallets may see UI countdown. |
| Year 5+ | Entropy decay begins. Dormant accounts contribute 10%/year. |

"Dormant" means: no `LivenessProof`, `Mine`, `SensorDataCommit`, transfer sent,
or any other account-initiated entry in over 12 months.

---

## How Decay Works

Each epoch, the protocol checks whether any account has been dormant for ≥ 365
days (measured in era-0 epoch counts). If so, 10%/year of that account's balance
enters the entropy pool for that epoch.

The epoch allocation is tiny per account — divide 10% by the number of epochs per
year to get the per-epoch take:

```
per_epoch_take = dormant_balance × 0.10 / epochs_per_year
```

At era-0 (2880 epochs/day, ~1M epochs/year):
```
per_epoch_take ≈ dormant_balance × 0.10 / 1_051_200
                ≈ dormant_balance × 9.5e-8 per epoch
```

This is imperceptible to any individual epoch but meaningfully moves tokens over
the full year.

---

## Entropy Pool Distribution

Each epoch's entropy pool is split 50/50:

| Destination | Amount | Mechanism |
|-------------|--------|-----------|
| Active live wallets | 50% | Pro-rata to accounts with a `LivenessProof` in this epoch |
| `__recycle_fund__` | 50% | Flows back through the normal block-reward distribution |

The `LivenessProof` entry is a zero-cost (free) signed heartbeat any account can
submit once per epoch to claim their share of the liveness pool. It costs no HONE
— only the posting key signature (free to generate). Wallets will auto-submit
these in the background.

---

## Chain Entropy Constants

| Constant | Value | Meaning |
|----------|-------|---------|
| `LIVENESS_REWARDS_ENABLED` | `false` (locked until docs) | Master switch |
| `ENTROPY_DECAY_ENABLED` | `false` (locked until docs) | Decay sub-switch |
| Dormancy threshold | 365 days | Account must be idle this long before decay starts |
| Annual rate | 10%/year | Fraction of dormant balance entering entropy pool per year |
| Live wallet share | 50% | Fraction of entropy pool going to active liveness provers |
| Recycle share | 50% | Fraction to `__recycle_fund__` |

---

## The `LivenessProof` Entry

```json
{
  "type": "LivenessProof",
  "account": "shindevlin",
  "epoch": 12345,
  "nonce": 0,
  "signed_by": "shindevlin"
}
```

- Signed by the account's posting key.
- Zero balance change — only proves the account is alive.
- One per epoch per account (duplicates rejected).
- Wallets will auto-submit this in the background (opt-in default).

---

## `EntropyWitness` Entry

The `EntropyWitness` entry is submitted by nodes contributing to the entropy VRF
in Phase 7 (Stage 2 epoch entropy). It is not required for liveness rewards —
`LivenessProof` is the user-facing instrument.

---

## Why This Is Fair

1. **Opt-out anytime**: Submitting any account-initiated entry resets the dormancy
   clock. One `LivenessProof` per year is enough to permanently exempt an account.
2. **No surprise**: Wallets will show a dormancy countdown 2 years before decay
   begins. Users have plenty of time to act.
3. **No destruction**: Tokens never vanish. They move from dormant to active hands.
4. **Proportional**: The 10%/year rate applies to the full balance but is spread
   across ~1M era-0 epochs, so each epoch's take is negligible.
5. **Fair split**: Active users who submit `LivenessProof` collectively share 50%
   of entropy. This incentivises engagement without punishing holders who simply
   stay invested.

---

## Activation Checklist

Before `LIVENESS_REWARDS_ENABLED` is set to `true` via governance:

- [ ] This document is published on the HONE website
- [ ] Whitepaper section "Chain Entropy" is complete and public
- [ ] Wallet UI shows dormancy countdown for accounts approaching threshold
- [ ] Explorer shows per-epoch `LivenessProof` participation rate
- [ ] Governance vote passes (2-of-3 council) after 2-epoch timelock

See `D2` in `docs/ROADMAP.md` for the locked design decision.

---

## Related

- `docs/ROADMAP.md` — D2 (Liveness Rewards), T6-4 (docs required before enable)
- `rust/hone-node/crates/hone-types/src/emission.rs` — `LIVENESS_REWARDS_ENABLED`, `ENTROPY_DECAY_ENABLED`
- `rust/hone-node/crates/hone-types/src/entry.rs` — `LivenessProof`, `EntropyWitness`
