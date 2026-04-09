# BTCPC On-Chain Governance

## Principle

Every protocol parameter is changeable by network consensus. When the project is handed off, no single person or entity should need to modify code to adjust the chain's behavior. The chain governs itself.

## What Can Be Changed By Consensus

| Parameter | Current Value | Description |
|-----------|--------------|-------------|
| Dormancy grace period | 5 years | Time before token decay begins |
| Dormancy decay rate | 10% per year | Annual decay of dormant tokens |
| Cross-chain freeze extension | 5 years | Extra grace for cross-chain-active accounts |
| Clock reward percentage | 2% | Share of block reward to clock nodes |
| Minimum stake (permissionless) | 100 BTCPC | Required stake for permissionless consensus participation |
| Minimum stake (mining) | 1000 BTCPC | Required stake for mining |
| Epoch duration | 5 minutes | Time between blocks |
| Proposal window | 30 seconds | Time for finalization proposals |
| Token creation fee | 42 BTCPC | Cost to create a custom token |
| Inference verification count | 1 (genesis) / 3 (consensus) | Number of miners that must verify each inference |
| Minimum version | 2.0.75 | Oldest software version allowed on the network |
| Escrow timeout | 10 minutes | Auto-refund for stuck escrows |
| Finality interval | 100 epochs | How often finality blocks are written |

## How Governance Works

### 1. Proposal

Any staked account can submit a `GOVERNANCE_PROPOSAL` ledger entry:

```json
{
  "type": "GOVERNANCE_PROPOSAL",
  "from": "shindevlin",
  "epoch": 5000,
  "memo": "Change dormancy_grace_years from 5 to 7",
  "account_data": {
    "parameter": "dormancy_grace_years",
    "current_value": 5,
    "proposed_value": 7,
    "rationale": "5 years is too aggressive for long-term holders",
    "voting_ends_epoch": 5100
  }
}
```

### 2. Voting

Staked accounts vote by submitting `GOVERNANCE_VOTE` ledger entries:

```json
{
  "type": "GOVERNANCE_VOTE",
  "from": "natoshisakamoto",
  "memo": "proposal:<proposal_hash>:yes",
  "epoch": 5050
}
```

Voting power = staked BTCPC. One token = one vote. Delegated stake votes with the delegator's choice unless the delegatee overrides.

### 3. Resolution

After the voting period ends:
- **Quorum**: at least 10% of total staked BTCPC must vote
- **Threshold**: 66% supermajority required to pass
- **Timelock**: changes take effect 100 epochs (~8 hours) after passing — gives nodes time to update

### 4. Execution

The chain reads protocol parameters from the governance state (stored on-chain), not from hardcoded values. When a proposal passes and the timelock expires, the new value is active for all nodes automatically.

## Everything Is Governable

There are no hardcoded sacred cows. If the network reaches consensus, anything can change — including the total supply.

Bitcoin's 21 million cap is a social contract, not a technical constraint. A hard fork could change it tomorrow. BTCPC makes this explicit: the 42M supply is the **default**, and the network can vote to change it, the same way it can vote to change any other parameter.

Some changes require a **higher bar** than others:

| Change Type | Required Supermajority | Quorum |
|-------------|----------------------|--------|
| Normal parameters (reward %, stake minimums, etc.) | 66% | 10% of staked |
| Economic parameters (supply, emission schedule, decay rates) | 80% | 25% of staked |
| Structural changes (key hierarchy, ledger format, epoch model) | 90% | 40% of staked |

The higher the impact, the more agreement is needed. But nothing is off the table.

### Genesis Block

The genesis block (block 0) is historical record — it can't be rewritten because that would invalidate the entire hash chain. This isn't a governance restriction, it's a mathematical one. You can't change the past without breaking the proofs.

## Governance Ledger Types

| Type | Purpose |
|------|---------|
| `GOVERNANCE_PROPOSAL` | Submit a parameter change proposal |
| `GOVERNANCE_VOTE` | Vote on a proposal (yes/no/abstain) |
| `GOVERNANCE_EXECUTE` | Auto-generated when a proposal passes and timelock expires |

## Chain Rollback & Block Invalidation

The network can vote to undo history — like Ethereum did with the DAO hack in 2016.

### What the Network Can Vote To Do

| Action | What Happens | Use Case |
|--------|-------------|----------|
| **Invalidate a block** | Block marked void, state recomputed as if it never happened | Malicious block accepted by mistake |
| **Reverse a transaction** | Tokens moved back to original owner | Exploit, stolen funds |
| **Blacklist an address** | Account frozen, cannot send (can still receive) | Compromised account, active attacker |
| **Reorg from checkpoint** | Roll back to a finality block, replay forward skipping bad blocks | Major chain corruption |

### How It Works

1. `GOVERNANCE_PROPOSAL` submitted with type `CHAIN_INTERVENTION`
2. Must specify: which block(s) or transaction(s), what action, detailed rationale
3. **90% supermajority** required (structural tier — highest bar)
4. **40% quorum** of staked tokens must vote
5. Voting period: 200 epochs (~16 hours) — longer than normal to allow deliberation
6. If passed: next finalization consensus includes the intervention
7. All nodes recompute state from the nearest finality block before the affected block

### Why Finality Blocks Make This Practical

Without finality blocks, a rollback means replaying from genesis — impractical as the chain grows. With finality blocks every 100 epochs:

```
Bad block at epoch 5,432
  → Nearest finality block: epoch 5,400
  → Roll back to epoch 5,400 state snapshot
  → Replay epochs 5,401-5,431, skip 5,432
  → Continue from epoch 5,433
  → Only 32 epochs replayed, not 5,432
```

### The Ethereum Precedent

Ethereum's DAO rollback in 2016 proved that "code is law" is aspirational, not absolute. When 3.6M ETH was stolen, the network voted to roll back. Those who disagreed forked to Ethereum Classic.

BTCPC builds this into the protocol explicitly — not as an emergency hack, but as a governed, transparent, vote-based mechanism. The community decides, not one developer.

### Safeguards

- **Highest threshold**: 90% supermajority + 40% quorum — near-unanimous agreement required
- **Extended voting**: 200 epochs instead of 100 — more time for community debate
- **Public rationale**: proposal must include detailed justification visible to all nodes
- **Transparency**: the CHAIN_INTERVENTION entry is permanently on the ledger — the rollback itself is recorded history
- **No secret rollbacks**: every intervention is visible on-chain forever

## Emergency Proposals

For critical security fixes, a separate fast-track path:
- Requires 90% supermajority (not 66%)
- No timelock — takes effect immediately
- Can only be submitted by accounts with 10,000+ BTCPC staked
- Must include a detailed security rationale

## Genesis Phase

During the genesis phase (before sufficient stake distribution), governance is effectively controlled by the genesis operator (shindevlin) since they hold the majority of staked tokens. As the network grows and stake distributes, governance becomes truly decentralized.

The goal is to make the genesis operator irrelevant — the chain should run itself.

## Implementation Status

**Not yet implemented.** This is the governance design for when the project is handed off. Current protocol parameters are in code. The transition path:

1. Move all configurable parameters to a `protocolParams` on-chain state
2. Add GOVERNANCE_PROPOSAL and GOVERNANCE_VOTE ledger types
3. Add voting logic to epoch finalization
4. Add timelock execution
5. Remove hardcoded values, read from governance state

This can be built incrementally — start with one parameter (e.g., clock reward percentage), prove the mechanism works, then extend to all parameters.

## Activation Timeline

Governance is designed but not active during genesis phase. It activates when:
1. Shin passes operational control to the network
2. At least 10 unique staked accounts exist
3. At least 1000 BTCPC is staked across the network

Until then, the genesis operator (shindevlin) manages protocol parameters.
After activation, all changes require network consensus votes.
