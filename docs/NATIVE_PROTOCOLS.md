# Native Protocol Businesses

## Overview

HONE ships with three protocol businesses baked in from genesis: Freeport, Verasens, and LinkGit. They are not add-ons, not third-party integrations, and not future roadmap items. They exist from block 0.

**Why native matters:**

- No bridge required — every transaction settles directly on the HONE chain
- No separate token — fees, escrow, and staking all flow through HONE
- Immediate liquidity — protocol activity generates real token demand from day one
- Every node is infrastructure — every node operator earns fees from all three protocols simultaneously

Each protocol is independently valuable. Each is also independently licensable. Together they give HONE three separate, non-correlated demand drivers, any one of which would justify the chain on its own.

---

## Freeport

**What it is:** Freeport is a sovereign marketplace protocol. It enables stores, products, orders, escrow, and encrypted digital product delivery entirely on-chain.

**What it does:**

- Merchants create storefronts and list products via `StoreUpdate` and `ProductCreate` / `ProductUpdate`
- Buyers place orders and pay into escrow via `OrderPlace`
- Sellers fulfill, and escrow releases automatically or via `EscrowRelease`
- Disputes handled via `OrderDispute` and `OrderCancel`
- Digital goods delivered encrypted via the `seek` / `hide` key roles
- Flash sales via `FlashSale` — time-bounded price drops committed on-chain

**Key entry types:** `StoreUpdate`, `ProductCreate`, `ProductUpdate`, `OrderPlace`, `OrderFulfill`, `OrderCancel`, `OrderDispute`, `EscrowRelease`, `FlashSale`

**Key roles:** `posting` (storefront), `memo` (purchase intent), `active` (payment key), `seek` (auto-delivery), `hide` (receive encrypted goods)

**Revenue model:**

- Escrow settlement fees charged on every completed order
- Storage fees for active product listings
- Every purchase locks HONE in escrow for the duration of the transaction — driving sustained token lock-up proportional to commerce volume

**Standalone potential:** Freeport could be extracted and deployed as a standalone sovereign marketplace chain or licensed to an existing L1 as a commerce module. The escrow mechanics, dispute resolution, and encrypted delivery pipeline are a complete product. See [FREEPORT_PROTOCOL_WHITEPAPER.md](FREEPORT_PROTOCOL_WHITEPAPER.md) for the full protocol specification.

---

## Verasens

**What it is:** Verasens is a decentralized IoT sensor and device data protocol. It turns the HONE chain into a verifiable ledger for real-world sensor data from any registered device.

**What it does:**

- Devices and sensors register on-chain via `SensorRegister` and `DeviceKeyRegister`
- Cryptographic keys are registered and vouched via `SensorKeyRegister` and `SensorVouch`
- Sensor readings are committed on-chain via `SensorReading` and `SensorDataCommit`
- Storage nodes stay accountable via `StorageHeartbeat`; gateways stay alive via `GatewayHeartbeat`
- Device operators stake HONE via `DeviceYieldStake` to earn emissions for data contributions

**Key entry types:** `SensorRegister`, `SensorKeyRegister`, `SensorVouch`, `SensorReading`, `SensorDataCommit`, `DeviceKeyRegister`, `DeviceYieldStake`, `GatewayHeartbeat`, `StorageHeartbeat`

**Revenue model:**

- Data query fees paid by anyone reading verified sensor history from the chain
- Sensor staking requirements — devices must hold HONE to participate, reducing circulating supply in proportion to the size of the sensor network
- Gateway and storage node operators earn fees for relaying and persisting sensor data

**Standalone potential:** Verasens could be licensed as a standalone IoT data integrity layer for supply chain, environmental monitoring, or industrial sensing use cases. The vouch-and-stake model provides Sybil resistance without a separate identity layer.

---

## LinkGit

**What it is:** LinkGit is a decentralized git protocol. It stores repositories as content-addressed objects on HONE-FS and records all ref updates on-chain.

**What it does:**

- Repositories created and anchored on-chain via `LinkGitRepoCreate`
- Branch and tag updates committed as on-chain refs via `LinkGitRefUpdate`
- Access control managed on-chain via `LinkGitAccessGrant` and `LinkGitAccessRevoke`
- Private repositories encrypted via the `hide` key — only holders of the corresponding `seek` key can read
- Storage extended or pruned via `LinkGitStorageExtend` and `LinkGitPruneProof`
- Mirror protocol: developers push simultaneously to LinkGit and GitHub — LinkGit is the canonical, censorship-resistant record; GitHub is the convenience mirror

**Key entry types:** `LinkGitRepoCreate`, `LinkGitRefUpdate`, `LinkGitAccessGrant`, `LinkGitAccessRevoke`, `LinkGitPruneProof`, `LinkGitStorageExtend`

**Revenue model:**

- Per-object storage fees paid in HONE for every blob, tree, and commit stored on HONE-FS
- Private repository fees for encrypted repos using the hide/seek key mechanism
- Storage extension fees for repos that remain active past their initial paid period

**Standalone potential:** LinkGit could be sold as a decentralized GitHub alternative or licensed to any project needing censorship-resistant code storage. The mirror protocol design means zero workflow disruption — teams keep using existing git tooling while the on-chain record provides permanence and auditability.

---

## Monetization Rails

The three protocols create three separate, independent demand drivers for HONE:

| Protocol | Demand Mechanism | Token Lock |
|----------|-----------------|------------|
| Freeport | Every purchase locks HONE in escrow | Duration of transaction |
| Verasens | Sensor staking requires HONE to participate | Duration of device operation |
| LinkGit | Storage fees paid in HONE per object | Ongoing per-repo |

Each of these operates independently. A collapse in e-commerce activity does not affect sensor data demand. A decline in IoT deployments does not affect code storage. The three demand drivers are non-correlated, which means the token has structural demand as long as any one of the three protocols has active users.

Combined, they make HONE a utility token in the strict sense: holding HONE is required to do things people want to do.

---

## Independence

Each protocol is native to HONE by choice, not by technical constraint.

Freeport's escrow mechanics, Verasens's device staking model, and LinkGit's content-addressed storage could each be deployed on any chain with sufficient storage and scripting capability. They run on HONE because HONE was designed to support exactly this kind of work-denominated, fee-generating protocol from the start.

This also means each protocol has standalone licensing value. An enterprise that wants on-chain sensor data integrity but does not want to run a full HONE node could license Verasens as a module. A company that wants censorship-resistant code storage could license LinkGit. A marketplace operator could license Freeport's escrow and delivery infrastructure.

Being native to HONE from genesis means these protocols are battle-tested on a live chain with real blocks, real fees, and real token mechanics — not whitepaper designs.
