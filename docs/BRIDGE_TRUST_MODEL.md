# BTCPC Bridge Trust Model

## Overview

The BTCPC bridge connects native BTCPC on the sovereign chain to wrapped wBTCPC on EVM
chains (Ethereum, Base, Arbitrum, Optimism, BSC, Polygon).

The bridge uses a **lock-and-release model** — no tokens are burned or minted.
Locked wBTCPC stays in the contract pool for the reverse direction.

---

## Current Architecture: V2 (3-of-5 Multisig)

### Signers

| Slot | Identity | Role |
|------|----------|------|
| 0 | shindevlin | Core developer |
| 1 | natoshisakamoto | Core developer |
| 2 | josh | Core developer |
| 3 | hardware-node-1 | Dedicated signing hardware (offline HSM) |
| 4 | hardware-node-2 | Dedicated signing hardware (offline HSM) |

**Threshold: 3-of-5.** Any three signers can approve a release, rotation, or unpause.

### Trust Assumptions

- **Release direction (BTCPC → EVM):** Requires 3-of-5 signer agreement on the amount, recipient,
  and nonce. An attacker must compromise three separate key holders simultaneously.
- **Lock direction (EVM → BTCPC):** Trustless. User sends tokens to the contract; the BTCPC chain
  watches the `Lock` event and credits native BTCPC. No signer involvement required.
- **Account binding:** A single signer authorises binding a BTCPC username to an EVM address.
  This is acceptable because binding data originates from on-chain state — forging it requires
  compromising the BTCPC chain itself, not just the bridge.

### EIP-712 Typed Data

All signatures use EIP-712 structured data signing. MetaMask and hardware wallets display
human-readable fields (recipient, amount, nonce) rather than an opaque hash.

**Domain separator:** `wBTCPCBridge v2` with `chainId` and contract address. Cross-chain
replay is impossible (different chain ID → different domain separator).

**Signed structs:**

| Operation | Struct |
|-----------|--------|
| Release | `Release(address recipient, uint256 amount, uint256 nonce)` |
| Bind account | `Bind(string btcpcUsername, address evmAddress)` |
| Rotate signer | `RotateSigner(address oldSigner, address newSigner, uint256 nonce)` |
| Update limits | `SetLimits(uint256 dailyReleaseLimit, uint256 dailyLockLimit, uint256 nonce)` |

### Daily Volume Limits

| Direction | Default Cap | Governance |
|-----------|-------------|-----------|
| Release (out) | Set at deploy | 3-of-5 to update |
| Lock (in) | Set at deploy | 3-of-5 to update |

Limits reset at UTC midnight (`block.timestamp / 86400`). A limit of `0` disables the cap.

### Pause

Any single signer can pause the bridge (emergency halt, no coordination required).
Three-of-five must agree to unpause.

### Signer Rotation

Old signer is replaced in-place. Requires 3-of-5 signatures over
`RotateSigner(oldSigner, newSigner, rotateNonce)`. The `rotateNonce` prevents replay
of an old rotation approval.

---

## Risk Register

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|-----------|
| Single key compromise | Medium | Low (V2) | Threshold requires 3-of-5; one key alone does nothing |
| 3 simultaneous key compromises | Very low | Critical | Hardware signers on separate air-gapped devices |
| Release nonce replay | Negligible | Critical | `processedNonces` mapping; each nonce used once |
| Lock event replay (BTCPC side) | Low | High | BTCPC chain validates event nonce monotonicity |
| Daily limit exhaustion | Low | Medium | Volume caps limit single-epoch drain |
| Smart contract bug | Low | Critical | Audit planned at Phase 9; contract is minimal and auditable |

---

## Recommended Deployment: Safe Multisig

Rather than relying on the custom 3-of-5 logic inside `wBTCPCBridge.sol`, the
recommended deployment pattern is to **use a Gnosis Safe as the bridge admin**:

1. Deploy a [Safe](https://app.safe.global) on Ethereum (or target EVM chain) with a **3-of-5 threshold** and the five signer addresses listed above.
2. Set the Safe's address as the `owner` / admin of the bridge contract.
3. All bridge admin operations (release approvals, signer rotation, limit updates, pause/unpause) are proposed as Safe transactions — signers review human-readable fields in the Safe UI and approve with their hardware wallet.

**Why Safe over custom multisig:**
- Independently audited by multiple security firms; holds $100B+ in historic TVL
- Native Ledger hardware wallet support via WalletConnect — no browser extension required
- Web UI at `app.safe.global` shows the decoded transaction before signing
- Built-in nonce and replay protection; signature tracking is on-chain and transparent
- Timelock guards (Safe{Snap} or custom `Guard` contract) can enforce a delay on high-value releases

**Hardware wallets — one per signer:**

A single Ledger device can generate unlimited accounts at different BIP32 paths
(`m/44'/60'/N'/0/0`). However, each of the **three threshold signers must hold their
own physical device**. A single Ledger holding all five keys means one device
compromise = full bridge compromise, defeating the purpose of the multisig. The
standard setup:

| Signer | Device |
|--------|--------|
| shindevlin | Ledger Nano X (personal) |
| natoshisakamoto | Ledger Nano X (personal) |
| josh | Ledger Nano X (personal) |
| hardware-node-1 | Ledger stored in cold storage location A |
| hardware-node-2 | Ledger stored in cold storage location B |

The two hardware-node keys serve as backup signers — the three personal devices are
the operational threshold for day-to-day bridge use.

---

## Planned Upgrades

### V3 — Light Client Verification (Post-Audit)

Replace the multisig trust model with on-chain verification of BTCPC chain state proofs.

- BTCPC implements a Patricia Merkle Trie state root (Phase 7, D13)
- Bridge contract on EVM verifies Merkle proofs against a committed state root
- Signers become "state root relayers" — they cannot forge releases, only attest to chain state
- A committee of 21 relayers (staked on BTCPC chain) vote on the canonical state root
- Wrong attestation = stake slash on BTCPC chain

This removes the multisig trust entirely: the security model becomes "trust the BTCPC chain
consensus, not a fixed set of signers."

---

## TON and Bitcoin Ordinals Peer Discovery

TON and Bitcoin Ordinals registries are implemented but not yet deployed.

| Registry | Status | File | Activation |
|----------|--------|------|-----------|
| TON | Code complete, not deployed | `discovery.rs:TON_REGISTRY_CONTRACT` | Deploy contract, set constant |
| Bitcoin Ordinals | Code complete, not deployed | `discovery.rs:BTC_REGISTRY_WALLET` | Fund wallet, write first inscription |

Both fall back to Hive peer discovery while their respective constants are empty.

**Deployment checklist (TON):**
1. Deploy the registry smart contract on TON mainnet
2. Set `TON_REGISTRY_CONTRACT` to the deployed address
3. Write initial peer list via `set_peers()` getter

**Deployment checklist (Bitcoin Ordinals):**
1. Fund registry wallet with enough BTC for Ordinals inscriptions (~$50)
2. Inscribe the initial peer list JSON array from the registry wallet
3. Set `BTC_REGISTRY_WALLET` to the wallet's Bitcoin address

---

## Audit Plan

Target audit firms: Zellic, OtterSec, or Trail of Bits (D14).
Audit gate: after Phase 0–6 complete and V3 light-client path is designed.
Scope: `wBTCPCBridge.sol`, `wBTCPC.sol`, consensus finality, bridge event handling on BTCPC chain.
