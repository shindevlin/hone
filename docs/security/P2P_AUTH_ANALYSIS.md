# HONE P2P Message Authentication Analysis

## Scope

This document analyzes the authentication and integrity guarantees of HONE's WebSocket-based P2P layer. The P2P network handles block gossip, peer discovery, chain sync, account announcements, and inference job routing.

---

## Current Architecture Summary

- Transport: WebSocket (ws://)
- Peer discovery: bootstrap list + peer exchange
- Message format: JSON over WebSocket
- Authentication: None at transport layer (as of analysis date)
- Signing: Block-level Ed25519 signatures exist; message-level signing is absent for most message types

---

## Issue Inventory

### P2P-01: No Transport Encryption

**Description:** All P2P traffic is transmitted in plaintext over WebSocket. Any network observer (ISP, co-located attacker, exit node) can read full message contents, including account names, inference job payloads, and block data.

**Impact:** Privacy violation; inference job content exposure; chain state surveillance; facilitates targeted eclipse and replay attacks.

**Priority:** Critical

**Recommendation:**
- Migrate to WSS (WebSocket Secure / TLS) for all peer connections.
- For peer-to-peer connections without a CA-issued cert, use self-signed certs with public key pinning derived from the peer's staked identity.
- Short-term mitigation: run P2P behind a WireGuard mesh between known nodes.

---

### P2P-02: Timestamp Replay Window Too Large

**Description:** Messages include a timestamp field used to detect replays, but the acceptance window is overly broad (implementation-dependent; estimated at minutes to hours based on current code).

**Impact:** Attacker can capture a valid message and resubmit it within the replay window. For account announcements and inference job results, this could trigger duplicate processing.

**Priority:** High

**Recommendation:**
- Reduce the replay window to 30 seconds (one epoch).
- Pair the timestamp with a per-session nonce; the combination must be unique within the window.
- Nodes maintain a seen-nonce cache for the current and previous epoch windows, evicting entries on epoch boundary.

---

### P2P-03: No Peer Identity Binding to Stake

**Description:** Any node can connect and send messages without proving it holds a staked HONE account. There is no challenge-response handshake that ties a peer's WebSocket connection to an on-chain identity.

**Impact:** Unauthenticated peers can submit fake work proofs, spam gossip, and trigger processing overhead without any economic stake at risk.

**Priority:** High

**Recommendation:**
- Implement a connection handshake: the connecting peer signs a challenge (random nonce + timestamp) with the private key corresponding to their staked account.
- The receiving node verifies the signature against the on-chain account's public key.
- Unstaked peers are allowed in read-only mode (chain sync, block download) but cannot submit work proofs or account announcements.

---

### P2P-04: Gossip Amplification

**Description:** The gossip protocol forwards messages to all connected peers without a seen-message deduplication TTL. A single message can be re-forwarded multiple times by different relay paths, causing amplification.

**Impact:** Network bandwidth exhaustion; CPU overhead from redundant message processing; potential DoS vector for small nodes.

**Priority:** Medium

**Recommendation:**
- Maintain a per-node seen-message cache keyed by message hash, with a TTL of 2 epochs.
- Before forwarding, check if the message hash is in the seen cache; discard if present.
- Cap the number of peers a node forwards a given message to (fan-out cap, e.g., 8 peers).
- Apply per-peer rate limits on inbound message volume.

---

### P2P-05: Eclipse Attacks

**Description:** An attacker fills a target node's peer slots with attacker-controlled nodes, isolating it from the honest network. The victim node receives only attacker-supplied blocks and messages.

**Impact:** The victim can be fed a forked chain, have its transactions censored, or be denied awareness of slashing evidence against the attacker.

**Priority:** High

**Recommendation:**
- Maintain a minimum number of outbound connections to peers discovered through independent channels (e.g., DNS seeds, hardcoded bootstrap list).
- Diversify peer selection by autonomous system (AS) number to reduce co-location risk.
- Periodically evict and re-discover peers to prevent long-term capture.
- Staked peers (with verified on-chain identity) are preferred over unauthenticated peers for outbound slots.

---

## Implementation Priority Table

| Issue | ID | Priority | Effort | Blocks Mainnet? |
|-------|----|----------|--------|-----------------|
| No transport encryption | P2P-01 | Critical | Medium | Yes |
| Replay window too large | P2P-02 | High | Low | Yes |
| No peer identity binding | P2P-03 | High | Medium | Yes |
| Gossip amplification | P2P-04 | Medium | Low | No (pre-production) |
| Eclipse attacks | P2P-05 | High | Medium | Yes |

---

## Recommended Implementation Order

1. **P2P-02** (replay window): Low effort, high impact. Fix immediately.
2. **P2P-01** (TLS): Required before any public testnet expansion.
3. **P2P-03** (peer identity binding): Required before mainnet; pairs with staking infrastructure.
4. **P2P-05** (eclipse resistance): Required before mainnet; implement with peer diversity logic.
5. **P2P-04** (gossip amplification): Address before production load; not a mainnet blocker but degrades performance under load.

---

## Revision History

| Version | Date | Notes |
|---------|------|-------|
| 1.0 | 2026-04-15 | Initial P2P authentication analysis, P2P-01 through P2P-05 |
