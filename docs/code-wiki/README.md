# Code Wiki

This is the generated map of the BTCPC codebase.

Use it when you want to find where a feature lives, what tests cover it, or how a request moves through the system.

**Pages available**: 178

## If You're New

- [BTCPC docs index](../INDEX.md) - the human-written overview of the project.
- [Full generated index](index.md) - every generated community page in one place.

If you want the shortest path to understanding BTCPC, start with the docs index and then come back here when you need the code-level map.

## If You Are Debugging Or Building

- [P2P server](p2p-server.md) - peer connections, relay bootstrap, and inbound handling.
- [P2P message handling](p2p-handle.md) - announcements, heartbeat gossip, and message routing.
- [Clock / epoch logic](chain-epoch.md) - epoch timing, authority rotation, and chain-time behavior.
- [Node registry](chain-node.md) - miners, clocks, storage hosts, sensors, and device roles.
- [Chain state](chain-state.md) - derived balances, state store shape, and on-disk chain truth.
- [Mining finalization](mining-finalization.md) - work proofs, rewards, and block finalization.
- [Stateful hosting](services-stateful.md) - lifecycle for long-lived services.
- [Heartbeat tests](tests-heartbeat.md) - clock liveness, witness checks, and reward eligibility.
- [Stateful tests](tests-stateful.md) - storage and stateful service behavior.

## By Topic

- `p2p-*` - network sockets, relay bootstrap, gossip, and message routing.
- `chain-*` - epoch sealing, validation, state, and node registry.
- `services-*` - hosting, rewards, storage, sensors, and payouts.
- `tests-*` - the fastest way to see what is covered today.

The pages below are generated from the code knowledge graph and are meant to be a technical navigation aid, not a product guide.
