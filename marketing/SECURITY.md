# How BTCPC Keeps Users Safe

## For Non-Technical Readers

BTCPC is designed so that no single person, company, or computer can cheat, steal, or shut down the network. Here's how:

## Multiple Miners Verify Every Answer

When a user asks an AI question on BTCPC, the answer isn't trusted from a single source. Multiple miners independently compute the same answer. If they all agree, the answer is accepted. If someone tries to submit a wrong answer, the honest miners catch it.

**Real-world analogy:** Instead of trusting one accountant, you hire three and only accept the result they all agree on.

## Miners Have Skin in the Game (Staking)

Before a miner can earn rewards, they must deposit BTCPC tokens as a stake — essentially a security deposit. If they cheat (submit wrong answers), they lose their stake. The potential loss is always greater than the potential gain from cheating.

**Penalty schedule:**
- First offense: 10% of stake
- Second offense: 25% of stake
- Third offense: 50% + 24-hour ban
- Data leak: 100% of stake + permanent ban

## Decentralized — No Single Point of Failure

- **Multiple mining nodes** across different locations
- **Independent clock nodes** keep timing synchronized
- **No central server** — the network is the computer
- **If one node goes down**, others continue seamlessly

## Cryptographic Key Security

Every account uses a 4-key hierarchy:

| Key | What It Does | Risk If Compromised |
|-----|-------------|-------------------|
| **Owner key** | Full account control | Full risk — keep offline |
| **Active key** | Move money | Financial risk — use carefully |
| **Posting key** | Social operations, bot linking | Low risk — can't move money |
| **Memo key** | Encrypted messages | Privacy risk only |

**Why this matters:** Users can connect Telegram bots using their posting key without risking their funds. Even if a bot is compromised, the attacker can't steal tokens.

## Permanent, Unalterable Record

Every transaction, every mining reward, every AI inference result is recorded on the blockchain permanently. Once recorded, it cannot be changed, deleted, or hidden.

- **Block files** stored on every node's disk
- **Hash chain** — each block's fingerprint includes the previous block's fingerprint, creating an unbreakable chain back to the first block
- **State proofs** — any account balance can be mathematically verified without trusting anyone

## How BTCPC Prevents Common Attacks

### Double-Spending
Every transfer passes through a mempool that checks the sender's balance (including pending transactions). You can't spend the same tokens twice — the network catches it before the transaction is even processed.

### 51% Attack
Unlike Bitcoin (where 51% of hashrate = control), BTCPC has multiple defense layers:
- Verification consensus requires multiple independent miners to agree
- Staking creates economic cost for attackers
- State root in every block header means fraud is instantly detectable
- Finality blocks create checkpoints that can't be rewritten

### Sybil Attack (Fake Nodes)
Permissionless clock and mining nodes must stake BTCPC tokens to participate. Creating thousands of fake nodes requires proportionally large capital — making the attack economically irrational.

### Censorship
No single entity controls which requests get processed. Any miner can claim any request. Users can submit directly to the P2P network — no gatekeeper.

## Open Source

The entire BTCPC codebase is open source. Anyone can audit the code, verify the security model, and independently confirm that the system works as described. There are no hidden servers, no closed-source components, and no backdoors.
