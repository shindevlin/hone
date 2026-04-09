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

## What Cannot Be Changed

Some things are immutable — even governance can't change them:

- **Total supply**: 42,000,000 BTCPC — forever
- **Genesis block**: block 0 is sacred
- **Ledger permanence**: past entries can never be modified or deleted
- **Key hierarchy**: owner/active/posting/memo structure
- **Smallest unit**: 1 dream = 0.00000001 BTCPC

## Governance Ledger Types

| Type | Purpose |
|------|---------|
| `GOVERNANCE_PROPOSAL` | Submit a parameter change proposal |
| `GOVERNANCE_VOTE` | Vote on a proposal (yes/no/abstain) |
| `GOVERNANCE_EXECUTE` | Auto-generated when a proposal passes and timelock expires |

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
