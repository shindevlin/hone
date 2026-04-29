# LinkGit Protocol

Decentralized git on btcpc-fs. Version-controlled repositories stored as content-addressed objects in btcpc-fs, with branch/tag refs recorded on-chain.

## Overview

LinkGit is a protocol layer for hosting git repositories on BTCPC. Repository objects (commits, trees, blobs) are stored as content-addressed blobs in btcpc-fs. Branch and tag refs are written as on-chain ledger entries so every chain replica holds a verifiable, append-only history of all ref changes. Private repos are encrypted to the owner's hide key before storage — no storage node can read the content without the symmetric key.

Dead objects are pruned by default. When a `LinkGitRefUpdate` moves a branch head, storage nodes garbage-collect objects that are no longer reachable from any live ref in the repo. Owners who need to retain orphaned objects beyond the default prune window (for example, to preserve an abandoned branch tip) must submit a `LinkGitStorageExtend` entry paying a fee to keep the specified CIDs alive until a given epoch.

## Entry Types

| Entry type | Who signs | What it does |
|---|---|---|
| `LinkGitRepoCreate` | owner (posting key) | Registers a new repository. Sets visibility (public/private) and optionally the owner's hide public key for private repo encryption. |
| `LinkGitRefUpdate` | owner (posting key) | Records a new commit hash for a branch or tag ref. Triggers storage nodes to prune objects no longer reachable from any live ref. |
| `LinkGitAccessGrant` | grantor (posting key) | Shares the repo's symmetric key encrypted to the grantee's hide public key, granting read access to a private repo. |
| `LinkGitAccessRevoke` | grantor (posting key) | Revokes a previously granted access. Storage nodes stop serving encrypted objects to the grantee after this entry is applied. |
| `LinkGitPruneProof` | storage node (posting key) | Declares that a storage node has pruned unreachable objects after a ref update. Earns a small reward for confirmed GC work. Includes a Merkle root of pruned CIDs and bytes freed. |
| `LinkGitStorageExtend` | owner (posting key) | Pays a fee to retain specific CIDs beyond the default prune window. Specifies the CIDs to keep and the epoch until which they must be preserved. |
| `LinkGitAccessRevoke` (repeat) | grantor | See above — used for both individual revocations and full repo access teardown. |

## Private Repo Flow

1. Owner generates a repo symmetric key (AES-256 or ChaCha20-Poly1305).
2. All objects are encrypted with the symmetric key before being uploaded to btcpc-fs.
3. The owner's hide public key is registered on-chain in `LinkGitRepoCreate`.
4. To grant access: owner encrypts the symmetric key to the grantee's hide public key and submits a `LinkGitAccessGrant` entry containing the `encrypted_key` field.
5. Grantee decrypts the `encrypted_key` with their hide private key to recover the symmetric key, then uses it to decrypt objects fetched from btcpc-fs.
6. To revoke access: owner submits `LinkGitAccessRevoke`. Storage nodes enforce the revocation — subsequent fetch requests from the revoked grantee are rejected.

Hide keys are registered on-chain via `AccountUpdateKey` with `role = "hide"`. The hide private key never leaves the owner's device.

## Storage Model

- **Default behavior**: after each `LinkGitRefUpdate`, storage nodes compute the set of objects no longer reachable from any live ref and prune them. A `LinkGitPruneProof` entry is submitted to claim the GC reward.
- **Retaining objects**: submit `LinkGitStorageExtend` with the list of CIDs to preserve and a `keep_until_epoch`. The entry fee compensates storage nodes for extended retention. After `keep_until_epoch` passes, normal GC rules apply.
- **Cost model**: storage fees are denominated in dreams. The fee for a `LinkGitStorageExtend` is proportional to the number of CIDs and the number of epochs of retention.

## Reserved Accounts

| Account | Purpose |
|---|---|
| `linkgit` | Protocol-level operations account for the LinkGit sidecar service. |
| `linkgit-registry` | On-chain registry for repo metadata and access control state. |

Both accounts are provisioned in `genesis.json` with no keys. Keys are registered at sidecar startup via `AccountUpdateKey`.

## CLI

Repositories are addressed by the URI scheme `linkgit://owner/repo`.

```bash
# Add a LinkGit remote
git remote add origin linkgit://shindevlin/btcpc

# Push (LinkGit git-remote helper required)
git push origin main

# Clone
git clone linkgit://shindevlin/btcpc
```

The `git-remote-linkgit` helper translates standard git remote operations into btcpc-fs object uploads and on-chain `LinkGitRefUpdate` entries.
