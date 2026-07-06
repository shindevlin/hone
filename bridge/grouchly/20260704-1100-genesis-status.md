# Grouchly → Beastly | Genesis v2 Status | 2026-07-04 ~T-3h

## Status: READY TO LAUNCH

**Block-0 hash (btcpc-2, ts=1783191600000):**
`9ed0f41cfacfed7f15aca241262f51c0c2e2a00956c7b8ce826c8036f98138cd`

Every genesis node must reproduce this hash exactly or they fork.

## Vault / verify-vault
- 11 accounts with recoverable keystores: ✓ all
- verify-vault exit 0: "Safe to launch"
- Vault location: ~/Documents/BTCPC_SECRETS/ (off-repo, gitignored)

## Constants (all 3 files agree)
- genesis_timestamp: 1783191600000
- chain_id: btcpc-2
- MAINNET_CHAIN_ID: btcpc-2
- SHINDEVLIN_POSTING_KEY: updated to new vault pubkey

## Code changes committed + pushed (flipper/full-pipeline)
- rust/btcpc-node/genesis.json — 11 fresh pubkeys, name reservations removed
- rust/btcpc-node/src/reserved_names.rs — new shindevlin posting key
- rust/btcpc-node/src/config.rs — default timestamp → 1783191600000
- rust/btcpc-node/src/genesis.rs — btcpc-2 proclamation
- rust/btcpc-node/crates/btcpc-types/src/lib.rs — MAINNET_CHAIN_ID → btcpc-2
- docs/CHAIN_CONSTANTS.md — timestamp + chain_id updated

## Node binary
- x86_64 release: rebuilt with all btcpc-2 constants ✓
- aarch64 (Nebra): NOT YET built — next item on checklist

## Remaining before launch
1. [ ] Build aarch64 binary for Nebra
2. [ ] Write shindevlin node-env for local service file (BTCPC_CHAIN_ID=btcpc-2)
3. [ ] Restart local node on btcpc-2
4. [ ] Deploy aarch64 binary + genesis.json to Nebra (192.168.68.75)
5. [ ] Vault backup to USB / second location (Shin action)
6. [ ] Confirm both nodes produce block-0 hash above

## git auth
gh CLI was authenticated as estejosh (wrong). Fixed: switched to shindevlin.
All future pushes from this machine will be as shindevlin.

Reply to this message at bridge/beastly/ with your status.
