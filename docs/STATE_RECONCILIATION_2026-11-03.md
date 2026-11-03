# State reconciliation, 2026-11-03 (self-paid storage)

**Authorised by:** Josh, 2026-11-03 11:37 CST.
**Decision:** the canonical state is the grouchly/beastly lineage minus the storage rewards
the nodes paid to themselves.

## Cause
Before v2.1-live-identity 768af935, storage heartbeats were applied locally and gossiped
unsigned. Peers rejected them at seal, so each node held only its own `storage_beat:` rows,
and the reward driver paid the storage pool to that node alone. The same self-paid beats
also skewed the node's `ema_critical_mass:storage`, which over-paid in 4 epochs after the
fix (349100–349126). Since 349,127, grouchly and beastly have paid storage identically.

## Rule (exact)
For every epoch ≤ 351,090 and every `storage_reward` account, the canonical payout is the
**minimum** of the amounts grouchly and beastly paid, read from each node's `txhist`
(`hone-node state-audit`). Each node's excess is removed. Because the 0.4% testnet and 0.1%
treasury earnings shares are computed on what was paid out, their excess (`floor` per epoch)
is removed too. All removed hunits go to `__recycle_fund__`, so the deltas sum to zero.
Clock and sensor history were not changed; they take grouchly's values.

## Grouchly deltas (`deploy/corrections/correction-storage-selfpay.json`)
| account | delta (hunits) | HONE |
|---|---:|---:|
| natoshisakamoto | −215,101,717,959 | −21.5101717959 |
| shindevlin | −589,666,308 | −0.0589666308 |
| `__testnet_fund__` | −862,765,216 | −0.0862765216 |
| `__treasury__` | −215,691,102 | −0.0215691102 |
| `__recycle_fund__` | +216,769,840,585 | +21.6769840585 |

**Cross-check against beastly.** Beastly's own excess was shindevlin −71.7337076484 HONE
(self-paid). Applying each node's correction to the shared 351120/351090 checkpoint leaves
residual differences only in clock rewards (role-backer split) and josh's sensor rewards,
resolved to grouchly's values.
**Rounding:** the per-epoch floor on the testnet/treasury shares is exact to within
1 hunit per epoch.

## Procedure
1. Back up every data dir.
2. Stop grouchly, run `hone-node state-correct <data_dir> correction-storage-selfpay.json`
   once (atomic, conservation-checked, recorded as `operator_correction:<id>`), copy
   `state/`, then start grouchly.
3. One node at a time, stop beastly and then nebra, replace `state/` with that copy (all
   column families: balances, accounts, stakes, clock registrations, meta), clear the
   node's old `checkpoints/`, and start it. `node:*`, `monitor:*` and `current_epoch` are
   rewritten at startup.
4. Confirm the three state roots match over the next 2 healer checkpoints, then set
   `HONE_STATE_HEALER=heal`.
