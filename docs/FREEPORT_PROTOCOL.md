# Freeport Protocol

**Version 1.0 — April 2026**

---

## Overview

Freeport is a censorship-resistant, sovereign marketplace protocol baked into BTCPC genesis as a native ledger entry type — not a smart contract or sidechain. Stores, products, orders, escrow, and reputation are all first-class chain primitives. Digital product delivery uses asymmetric encryption keyed to the buyer's on-chain public key, so only the buyer can decrypt what they purchased. The reserved accounts `freeport` and `freeport-escrow` are seeded at block 0 and cannot be created or destroyed by any participant.

---

## Key Roles

| Role | What They Sign |
|------|---------------|
| **posting** | `StoreUpdate`, `ProductCreate`, `ProductUpdate` — storefront management |
| **memo** | `OrderPlace` — initiates a purchase and submits buyer public key |
| **active** | Payment authorization entries; the payer role in order settlement |
| **seek** | `OrderFulfill`, `EscrowRelease` — auto-fulfills digital orders and releases escrow |
| **hide** | Receives encrypted digital product payload; decrypts with their private key |

---

## Entry Types

| Entry Type | Who Signs | What It Does |
|------------|-----------|--------------|
| `StoreUpdate` | posting | Creates or updates a storefront record (name, description, policies) |
| `ProductCreate` | posting | Lists a new product (type: physical/digital, price, metadata) |
| `ProductUpdate` | posting | Modifies an existing product listing |
| `OrderPlace` | memo | Opens an order; submits buyer pubkey for digital delivery encryption |
| `OrderFulfill` | seek | Marks order fulfilled; for digital goods, attaches encrypted payload |
| `OrderCancel` | hide or seek | Cancels an open order before fulfillment |
| `OrderDispute` | hide | Escalates an order to dispute state; pauses escrow release |
| `EscrowRelease` | seek (auto) or arbitrator | Releases escrowed funds after fulfillment or dispute resolution |
| `FlashSale` | posting | Time-bounded discount applied to one or more products |

---

## Digital Product Delivery Flow

1. Buyer submits `OrderPlace` with their Ed25519 public key (hide key) included in the entry payload.
2. Payment is locked in `freeport-escrow` until the order is resolved.
3. Seller's node detects the order, encrypts the product payload using the buyer's hide public key (X25519 ECDH + AES-256-GCM).
4. Seller signs and submits `OrderFulfill` with the ciphertext attached (signed by seek key).
5. Buyer fetches the entry from any chain node and decrypts locally with their hide private key.
6. `EscrowRelease` fires automatically upon confirmed fulfillment (or after the dispute window expires).

No delivery server. No cloud. The chain is the delivery channel.

---

## Reserved Accounts

| Account | Purpose |
|---------|---------|
| `freeport` | Protocol authority; receives platform fee share per fulfilled order |
| `freeport-escrow` | Holds payment from `OrderPlace` until `EscrowRelease` fires |

Both accounts are seeded at genesis block 0 with zero balance. They cannot be registered, transferred, or destroyed by any user key. Funds entering `freeport-escrow` are locked until a valid `EscrowRelease` entry is submitted by the authorized signer.

---

*See also: [[BTCPC_WHITEPAPER#Appendix M: Decentralized Commerce Layer]], [[FREEPORT_PROTOCOL_WHITEPAPER]]*
