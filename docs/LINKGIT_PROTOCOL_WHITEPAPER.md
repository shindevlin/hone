# LinkGit Protocol

### Decentralized Version Control Anchored to a Proof-of-Compute Chain

**Shin Devlin**
**Version 1.0 — April 2026**

---

> **Note (2026):** LinkGit is a native protocol within BTCPC, deployed at genesis block 0. All entry types described in this whitepaper are natively supported by the BTCPC chain. No separate deployment or smart contract required. See [NATIVE_PROTOCOLS.md](NATIVE_PROTOCOLS.md) for the full native protocol overview.

---

## Abstract

LinkGit is a decentralized version control protocol built into the BTCPC chain at genesis. Repository objects — commits, trees, blobs — are stored as content-addressed data in btcpc-fs. Branch and tag refs are written as on-chain ledger entries, making every ref change append-only, tamper-evident, and replicated across the entire chain network. Private repositories are encrypted to the owner's hide key; access is granted and revoked on-chain without exposing the symmetric key to any storage node.

The protocol is designed to coexist with existing git infrastructure. The `git-remote-linkgit` helper translates standard git remote operations into chain entries and btcpc-fs uploads, so users interact with `git push` and `git clone` as normal. A built-in mirror protocol allows simultaneous push to LinkGit and GitHub (or any other git host), so teams do not need to choose between decentralized sovereignty and platform reach.

Protocol ownership is held by the `linkgit` account (controlled by `shindevlin` at genesis) and is transferable via standard key rotation.

---

## 1. Introduction

### 1.1 The Problem with Centralized Git Hosting

The vast majority of the world's source code is hosted on three platforms: GitHub, GitLab, and Bitbucket. All three are centralized services with a familiar set of structural weaknesses.

**Platform risk.** GitHub can suspend any repository, any organization, or any account at any time, for any reason, without advance notice. This has happened to open-source projects, to entire national coding communities under sanctions, and to individual developers for policy violations that were never clearly defined. The code is yours — but the hosting is the platform's, and the platform's rules govern accessibility.

**No cryptographic ownership.** When you push to GitHub, you are uploading content to GitHub's servers under GitHub's terms. There is no cryptographic proof that the version of `main` at commit `abc123` is the version you authored. GitHub can silently modify, rewrite, or replace repository contents. The `git` hash you see is computed over your objects — but those objects live in GitHub's storage, not yours.

**DMCA and legal censorship.** Repositories that host security research, cryptographic tools, or content that any jurisdiction finds objectionable are routinely removed following DMCA takedowns or government requests. The repository owner receives no meaningful recourse. The content is gone.

**Vendor-specific features.** GitHub Actions, GitLab CI/CD, GitHub Issues, GitHub Discussions — these features are powerful, but they create deep coupling to the platform. Migrating away means losing workflow, issue history, CI pipelines, and all the metadata that has accumulated around the code.

**Metadata is not portable.** Pull requests, code review comments, issue discussions, deployment records — none of this is in the git history. It lives in GitHub's database. When a project migrates off GitHub, it takes only the commits; everything else is left behind.

### 1.2 The LinkGit Insight

The core of git is already decentralized: every clone is a complete, cryptographically consistent replica of the repository's object graph. The problem is not git itself — it is the layer of infrastructure that hosts the canonical ref pointer ("`main` is at commit `abc123`") and makes that pointer globally discoverable and highly available.

LinkGit replaces that centralized ref pointer with an on-chain ledger entry. When you push to a LinkGit remote, two things happen:

1. Your git objects (commits, trees, blobs) are uploaded to btcpc-fs as content-addressed storage, replicated across the BTCPC storage network.
2. The updated branch ref (`main` → `abc123`) is written as a `LinkGitRefUpdate` entry to the BTCPC ledger, replicated across every node in the network.

The result is a repository where:
- The objects are content-addressed and distributed — no single node holds the authoritative copy
- The refs are on-chain — any chain replica can tell you the current state of `main`
- The entire history is append-only — ref updates are ledger entries, not overwrites
- Private repos are encrypted before upload — storage nodes cannot read the content
- Access control is on-chain — grant and revoke are ledger entries, enforced by storage nodes

### 1.3 Design Goals

1. **Git-compatible.** `git push`, `git clone`, `git fetch` work without modification. The `git-remote-linkgit` helper handles the translation.
2. **Coexist with GitHub.** The mirror protocol pushes to LinkGit and GitHub simultaneously. Teams can publish to GitHub for discoverability while maintaining sovereign storage on-chain.
3. **Private by default for encrypted repos.** No storage node can read a private repository without the symmetric key. Access control is enforced cryptographically, not by server-side policy.
4. **Lean chain footprint.** The chain stores only refs and access control entries — not git objects. Objects live in btcpc-fs. This keeps chain state compact and replication fast.
5. **Economic sustainability.** Storage nodes earn BTCPC for hosting git objects. Dead objects are pruned by default; retention beyond the default window costs a fee.

---

## 2. Architecture

### 2.1 Components

```
Developer Workstation
     │
     │  git push linkgit://shindevlin/btcpc
     ▼
git-remote-linkgit  (git remote helper, installed as part of btcpc-node toolchain)
     │
     ├── Upload git objects → btcpc-fs (content-addressed blob storage)
     │
     └── Write ref update → BTCPC Chain (LinkGitRefUpdate entry)
                                  │
                                  ▼
                       All BTCPC nodes replicate the ref
                                  │
                                  ▼
                       Storage nodes serve objects
                       Storage nodes prune dead objects after ref update
                       Storage nodes submit LinkGitPruneProof entries
```

### 2.2 Mirror Protocol

The mirror protocol runs alongside the git-remote-linkgit helper. When configured, a push to a LinkGit remote fans out to all configured mirror remotes in parallel:

```
git push linkgit://shindevlin/btcpc main
     │
     ├── git-remote-linkgit: upload objects + write LinkGitRefUpdate
     │
     └── mirror apply:
          ├── git push github:shindevlin/btcpc main (via standard git)
          └── git push gitlab:shindevlin/btcpc main (via standard git)
```

Mirror configuration lives in `.linkgit/mirrors` at the repo root:

```
[mirror "github"]
  url = https://github.com/shindevlin/btcpc
  push = refs/heads/*
  push = refs/tags/*

[mirror "gitlab"]
  url = https://gitlab.com/shindevlin/btcpc
  push = refs/heads/*
```

The `linkgit mirror apply` command writes the mirror URLs as `url.<url>.pushInsteadOf` entries in the local git config, so all pushes automatically fan out without any extra commands. The mirror protocol is transparent to the developer — `git push` handles everything.

### 2.3 Private Repository Encryption

Private repos use the owner's hide key, an ed25519 keypair derived from the BIP-39 seed at the SLIP-10 hardened path `m/44'/6942'/4'/0'` (role index 4 = hide). The hide key is designed specifically for content encryption — its private key never leaves the owner's device, and its public key is published on-chain.

**Write path (owner pushes to private repo):**
1. Owner generates a repo symmetric key (AES-256-GCM, 32 bytes random).
2. All git objects are encrypted with the symmetric key before upload to btcpc-fs.
3. The owner's hide public key is registered on-chain in `LinkGitRepoCreate`.
4. The symmetric key is encrypted to the owner's hide public key using ECIES (secp256k1) and stored locally.

**Grant access path:**
1. Grantee publishes their hide public key on-chain via `AccountUpdateKey` with `role = "hide"`.
2. Owner fetches grantee's hide public key from the chain.
3. Owner encrypts the repo symmetric key to grantee's hide public key.
4. Owner submits `LinkGitAccessGrant` containing the `encrypted_key` field (ciphertext visible on-chain, only decryptable by grantee's hide private key).
5. Grantee fetches the `LinkGitAccessGrant` entry, decrypts `encrypted_key` with their hide private key, recovers the symmetric key, and can now decrypt all repo objects from btcpc-fs.

**Revoke access path:**
1. Owner submits `LinkGitAccessRevoke` for the grantee.
2. Storage nodes enforce revocation — they stop serving objects to the grantee.
3. To truly prevent future access, the owner rotates the repo symmetric key and re-encrypts all objects with the new key. (This is a storage-intensive operation; most revocations for non-sensitive repos skip re-encryption and rely on storage node enforcement.)

### 2.4 Storage and GC

LinkGit's storage model is designed for efficiency. The chain does not store git objects — only CID references (hashes) in `LinkGitRefUpdate` entries. The objects themselves live in btcpc-fs.

**Default GC:** After each `LinkGitRefUpdate`, storage nodes compute the set of git objects that are no longer reachable from any live ref in the repo (unreachable commits, trees, blobs). These are garbage collected. The storage node that performs GC submits a `LinkGitPruneProof` entry containing a Merkle root of pruned CIDs and total bytes freed, earning a small BTCPC reward.

**Retaining objects:** Repo owners who need to retain orphaned objects (for example, an abandoned feature branch, or a historical state for compliance purposes) submit a `LinkGitStorageExtend` entry. The entry specifies the CIDs to preserve and `keep_until_epoch`. The fee is proportional to the number of CIDs and the number of epochs of retention. After `keep_until_epoch`, normal GC rules apply.

---

## 3. Entry Types

All LinkGit operations are expressed as first-class BTCPC ledger entries. No WASM smart contract is required.

| Entry Type | Who Signs | What It Does |
|---|---|---|
| `LinkGitRepoCreate` | owner (posting key) | Registers a new repository. Sets visibility (public/private), optional hide public key for private repos, and description. |
| `LinkGitRefUpdate` | owner (posting key) | Records a new commit hash for a branch or tag ref. Triggers storage nodes to GC unreachable objects. |
| `LinkGitAccessGrant` | grantor (posting key) | Shares the repo's symmetric key encrypted to the grantee's hide public key. Grants read access to a private repo. |
| `LinkGitAccessRevoke` | grantor (posting key) | Revokes a previously granted access. Storage nodes stop serving objects to the grantee after this entry is applied. |
| `LinkGitPruneProof` | storage node (posting key) | Proves that a storage node has GC'd unreachable objects after a ref update. Includes Merkle root of pruned CIDs and bytes freed. Earns a small BTCPC reward. |
| `LinkGitStorageExtend` | owner (posting key) | Pays a fee to retain specific CIDs beyond the default prune window. Specifies CIDs to keep and `keep_until_epoch`. |

### 3.1 Entry Schema Details

#### `LinkGitRepoCreate`
```json
{
  "type": "LinkGitRepoCreate",
  "account": "shindevlin",
  "repo_id": "shindevlin/btcpc",
  "visibility": "public",
  "hide_pubkey": null,
  "description": "Bitcoin Proof of Compute",
  "epoch": 1,
  "sig": "..."
}
```

Private repo:
```json
{
  "type": "LinkGitRepoCreate",
  "account": "alice",
  "repo_id": "alice/private-project",
  "visibility": "private",
  "hide_pubkey": "02abc...",
  "description": "Internal work",
  "epoch": 1200,
  "sig": "..."
}
```

#### `LinkGitRefUpdate`
```json
{
  "type": "LinkGitRefUpdate",
  "account": "shindevlin",
  "repo_id": "shindevlin/btcpc",
  "ref": "refs/heads/main",
  "old_oid": "0000000000000000000000000000000000000000",
  "new_oid": "a7f4b3c9d0e1f2a3b4c5d6e7f8a9b0c1d2e3f4a5",
  "epoch": 4200,
  "sig": "..."
}
```

#### `LinkGitAccessGrant`
```json
{
  "type": "LinkGitAccessGrant",
  "account": "alice",
  "repo_id": "alice/private-project",
  "grantee": "bob",
  "encrypted_key": "04b7f2...AES-GCM ciphertext (encrypted repo sym key, to bob's hide pubkey)...",
  "epoch": 4201,
  "sig": "..."
}
```

#### `LinkGitPruneProof`
```json
{
  "type": "LinkGitPruneProof",
  "account": "storage-node-1",
  "repo_id": "shindevlin/btcpc",
  "pruned_cids_root": "sha256:b94d27b9...",
  "pruned_count": 47,
  "bytes_freed": 1247832,
  "epoch": 4201,
  "sig": "..."
}
```

#### `LinkGitStorageExtend`
```json
{
  "type": "LinkGitStorageExtend",
  "account": "alice",
  "repo_id": "alice/private-project",
  "cids": ["sha256:abc...", "sha256:def..."],
  "keep_until_epoch": 10000,
  "fee": 5000000000,
  "epoch": 4202,
  "sig": "..."
}
```

---

## 4. CLI Reference

### 4.1 Basic Usage

LinkGit repositories use the URI scheme `linkgit://owner/repo`.

```bash
# Create a new repo on-chain
linkgit repo create shindevlin/my-project

# Add a LinkGit remote to an existing git repo
git remote add origin linkgit://shindevlin/my-project

# Push (git-remote-linkgit handles upload + chain entry)
git push origin main

# Clone
git clone linkgit://shindevlin/btcpc

# Fetch
git fetch origin
```

### 4.2 Mirror Protocol

```bash
# Initialize mirror config for a repo
linkgit mirror init

# Add mirrors
linkgit mirror add github https://github.com/shindevlin/btcpc
linkgit mirror add gitlab https://gitlab.com/shindevlin/btcpc

# Apply mirrors to git config (sets up multi-push)
linkgit mirror apply

# Now git push fans out to all mirrors automatically
git push origin main
# pushes to: linkgit://shindevlin/btcpc, github, gitlab

# View configured mirrors
linkgit mirror list

# Remove a mirror
linkgit mirror remove gitlab
```

The `.linkgit/mirrors` config file:

```ini
[mirror "github"]
  url = https://github.com/shindevlin/btcpc
  push = refs/heads/*
  push = refs/tags/*

[mirror "gitlab"]
  url = https://gitlab.com/shindevlin/btcpc
  push = refs/heads/*
```

This file is checked into the repository and shared with all collaborators. When a collaborator clones the repo and runs `linkgit mirror apply`, their git config is updated to push to all configured mirrors.

### 4.3 Access Control

```bash
# Grant read access to a private repo
linkgit access grant alice/private-project bob

# Revoke access
linkgit access revoke alice/private-project bob

# List current access grants
linkgit access list alice/private-project
```

### 4.4 Storage Management

```bash
# View storage usage for a repo
linkgit storage status shindevlin/btcpc

# Extend storage for specific objects
linkgit storage extend alice/private-project --cid sha256:abc... --until-epoch 10000

# View prune history
linkgit storage pruned shindevlin/btcpc
```

---

## 5. Economic Model

### 5.1 Storage Fees

LinkGit storage fees are paid in BTCPC dreams when committing objects to btcpc-fs. The fee is charged by the btcpc-fs storage network, not by the LinkGit protocol layer. Storage nodes earn from the 12% storage reward pool for hosting git objects alongside other btcpc-fs blobs.

There is no per-push fee for writing on-chain entries. The standard BTCPC transaction fee (denominated in dreams) applies to each ledger entry: `LinkGitRepoCreate`, `LinkGitRefUpdate`, `LinkGitAccessGrant`, etc.

### 5.2 GC Rewards

Storage nodes earn BTCPC for proving garbage collection. When a `LinkGitRefUpdate` moves a branch head, the storage nodes that perform GC submit `LinkGitPruneProof` entries. The proof earns a small reward from the storage pool proportional to the bytes freed. This creates an economic incentive for storage nodes to actively prune dead objects rather than holding them indefinitely.

### 5.3 Protocol Revenue

The `linkgit` protocol account earns a small fee on each `LinkGitStorageExtend` entry — a percentage of the extension fee flows to `linkgit` as protocol maintenance revenue. Future governance may direct this to a community fund.

---

## 6. Privacy and Security

### 6.1 Threat Model

**What LinkGit protects against:**
- Platform censorship — objects are replicated across the btcpc-fs storage network; no single node can suppress access
- Silent content modification — all objects are content-addressed; the chain ref anchors a specific hash, not a mutable pointer
- Unauthorized access to private repos — objects are encrypted before upload; storage nodes cannot read the content
- Access revocation bypass — revocation is a chain entry; storage nodes enforce it at the serving layer

**What LinkGit does not protect against:**
- An attacker who compromises both the owner's private key AND the hide private key (full key compromise) — at this point, the attacker has access to both the symmetric key and the ability to write new access grants
- Traffic analysis — observers can see that a push occurred, the repo ID, and the new commit hash (for public repos); they cannot read the objects for private repos, but the timing and frequency of pushes is visible on-chain
- Storage network attacks — if the majority of storage nodes holding a specific repo's objects go offline, the objects may become unavailable (the chain ref still exists, but objects cannot be fetched). This is mitigated by replication factor in btcpc-fs.

### 6.2 Hide Key Architecture

The hide key is one of six role keys in the HONE key hierarchy. HONE role keys
are **ed25519** on the SLIP-10 hardened path `m/44'/6942'/role'/0'` (6942 is HONE's
coin index; `role` selects the key):

| Role | role index | Path | Purpose |
|---|---|---|---|
| owner | 0 | `m/44'/6942'/0'/0'` | Key rotation, account recovery |
| active | 1 | `m/44'/6942'/1'/0'` | Token transfers, escrow |
| posting | 2 | `m/44'/6942'/2'/0'` | Chain entries, storefront, git push |
| memo | 3 | `m/44'/6942'/3'/0'` | Purchase initiation, encrypted messages |
| **hide** | 4 | `m/44'/6942'/4'/0'` | **Decrypt inbound encrypted content** |
| seek | 5 | `m/44'/6942'/5'/0'` | Auto-deliver encrypted content outbound |

The hide private key is specifically designed for asymmetric encryption of inbound content. When another party wants to share a secret with you (a repo symmetric key, a digital product, an encrypted message), they encrypt it to your hide public key. Only your hide private key can decrypt it. The hide private key never leaves your device and is never submitted to the chain.

This architecture means LinkGit private repo access control is fully on-chain and cryptographically enforced, not dependent on any server-side access policy.

---

## 7. Protocol Ownership and Governance

### 7.1 Initial Ownership

The `linkgit` account is controlled by `shindevlin` at genesis. The `linkgit-registry` account serves as the on-chain anchor for all repository metadata.

**What shindevlin controls at genesis:**
- Protocol fee parameters (percentage of `LinkGitStorageExtend` fees flowing to `linkgit`)
- GC reward calibration (the BTCPC reward per byte freed in `LinkGitPruneProof`)
- Future protocol upgrades (new entry types, schema extensions)
- The `btcpc/btcpc` GitHub repository — the reference implementation of the LinkGit chain layer and the `git-remote-linkgit` helper

**What shindevlin does not control:**
- Individual repositories — once a `LinkGitRepoCreate` entry is written, the repo's ref history is on-chain and immutable
- Private repo objects — encrypted before upload; not readable by any account without the symmetric key
- Historical commit history — all git objects are content-addressed in btcpc-fs; no protocol account can modify or delete them outside the GC mechanism

### 7.2 Transfer Mechanism

Ownership of LinkGit can be transferred from shindevlin to any other party via two steps:

1. **On-chain key rotation**: submit `AccountUpdateKey` for the `linkgit` account, changing the owner key to the new controller's key.
2. **GitHub repository transfer**: transfer the `btcpc/btcpc` GitHub repository (which contains the LinkGit implementation — `rust/linkgit/`, chain entry types, API endpoints) to the new controller's GitHub account.

Both steps together constitute a complete transfer of the protocol. The BTCPC genesis block records shindevlin as the original protocol author for historical attribution.

### 7.3 Strategic Value

LinkGit is positioned as the native version control layer for the BTCPC ecosystem. Every BTCPC project — miners, validators, protocol implementations — can be hosted on LinkGit with sovereign ownership and simultaneous GitHub publishing.

Long-term, as the BTCPC ecosystem grows, LinkGit becomes the default code hosting layer for teams that want cryptographic proof of their codebase's history without platform dependency. The mirror protocol ensures no friction during the transition — teams can mirror to GitHub indefinitely and shift their primary hosting to LinkGit as the storage network matures.

---

## 8. Implementation

### 8.1 git-remote-linkgit

`git-remote-linkgit` is a git remote helper — a binary that git invokes when a remote URL uses the `linkgit://` scheme. The helper speaks the git remote helper protocol on stdin/stdout.

**Capabilities:**
- `fetch` — fetches git objects from btcpc-fs and writes them to the local object store
- `push` — uploads git objects to btcpc-fs, writes `LinkGitRefUpdate` entries to the chain
- `list` — lists all refs for a repo by querying the chain API

**Installation:** The helper is distributed as part of `btcpc-node`. After `cargo build`, the `git-remote-linkgit` binary is in `target/release/` and must be on PATH for git to find it.

### 8.2 Chain-Side State

The BTCPC node maintains LinkGit state in RocksDB:

```
linkgit:repo:{repo_id}          → JSON: { owner, visibility, hide_pubkey, description, created_epoch }
linkgit:ref:{repo_id}:{ref}     → JSON: { oid, epoch, updated_by }
linkgit:access:{repo_id}:{acct} → JSON: { encrypted_key, granted_epoch }
```

The node exposes LinkGit API endpoints for the `git-remote-linkgit` helper:

```
GET  /api/linkgit/repos                         — list repos
GET  /api/linkgit/repos/{repo_id}               — get repo metadata
GET  /api/linkgit/repos/{repo_id}/refs          — list all refs
POST /api/linkgit/repos                         — create repo (submits LinkGitRepoCreate entry)
POST /api/linkgit/repos/{repo_id}/refs          — update ref (submits LinkGitRefUpdate entry)
POST /api/linkgit/repos/{repo_id}/access/grant  — grant access (submits LinkGitAccessGrant entry)
POST /api/linkgit/repos/{repo_id}/access/revoke — revoke access (submits LinkGitAccessRevoke entry)
```

### 8.3 btcpc-fs Integration

Git objects uploaded during a push are chunked and stored in btcpc-fs at their SHA-256 content hash. The btcpc-fs CID for a git object is `sha256:<hex>`. Since git already identifies objects by SHA-1, the btcpc-fs layer adds SHA-256 content addressing on top for storage-layer deduplication and verification.

On fetch, `git-remote-linkgit` queries the chain for the current ref OID, then fetches the required objects from btcpc-fs by CID. The objects are written to the local git object store and git's standard pack negotiation handles delta compression.

---

## 9. Comparison

| Feature | GitHub | GitLab | Gitea (self-hosted) | IPFS-based | LinkGit |
|---|---|---|---|---|---|
| No central server | ✗ | ✗ | partial (self) | ✓ | ✓ |
| Cryptographic ref integrity | ✗ | ✗ | ✗ | partial | ✓ (on-chain) |
| Encrypted private repos (infra can't read) | ✗ | ✗ | ✗ | ✗ | ✓ |
| On-chain access control | ✗ | ✗ | ✗ | ✗ | ✓ |
| Dead blob GC with proof | ✗ | ✗ | ✗ | ✗ | ✓ |
| Mirror to other git hosts | partial | partial | partial | ✗ | ✓ (native) |
| Works with standard `git push` | ✓ | ✓ | ✓ | ✗ | ✓ |
| Censorship resistant | ✗ | ✗ | ✓ (if self-hosted) | ✓ | ✓ |
| Token incentives for storage | ✗ | ✗ | ✗ | ✗ | ✓ |
| DMCA / legal takedown surface | ✓ | ✓ | ✗ | ✗ | ✗ |

---

## 10. Roadmap

### Phase 1 (Genesis — Q2 2026)
- All 6 entry types live on mainnet
- `git-remote-linkgit` helper operational
- Mirror protocol implemented
- `linkgit` CLI for repo management and access control

### Phase 2 (Q3 2026)
- Code review protocol: on-chain PR/MR metadata (title, body, status) — review comments remain off-chain or in a separate sidecar
- Issue tracking: lightweight `LinkGitIssue` entry type for bug reports and feature requests anchored to the chain
- Notifications: subscriber accounts watch repos for `LinkGitRefUpdate` events

### Phase 3 (Q4 2026)
- LinkGit web explorer: public repos browsable at a web UI served by any BTCPC node
- CI/CD integration: trigger builds on `LinkGitRefUpdate` events via webhooks to off-chain build systems
- Organization accounts: multi-sig posting key for org-owned repos

### Phase 4 (2027)
- On-chain governance for repo forks (fork = `LinkGitRepoCreate` with `forked_from` field)
- Reputation system: storage nodes rated by uptime and fetch latency
- Cross-chain repo mirroring: `LinkGitRefUpdate` relayed to Nostr or ActivityPub for discoverability outside the BTCPC ecosystem

---

## Appendix A: Entry Type Reference

### `LinkGitRepoCreate`
| Field | Type | Description |
|---|---|---|
| `account` | string | Owner's BTCPC account name |
| `repo_id` | string | `owner/repo` format, globally unique |
| `visibility` | string | `"public"` or `"private"` |
| `hide_pubkey` | string? | Owner's hide public key (required for private repos) |
| `description` | string? | Human-readable description |
| `epoch` | u64 | Chain epoch at submission |
| `sig` | string | Hex-encoded signature (owner's posting key) |

### `LinkGitRefUpdate`
| Field | Type | Description |
|---|---|---|
| `account` | string | Owner's BTCPC account name |
| `repo_id` | string | `owner/repo` |
| `ref` | string | Full ref name (`refs/heads/main`, `refs/tags/v1.0.0`) |
| `old_oid` | string | Previous commit hash (all zeros for new ref) |
| `new_oid` | string | New commit hash |
| `epoch` | u64 | Chain epoch at submission |
| `sig` | string | Hex-encoded signature (owner's posting key) |

### `LinkGitAccessGrant`
| Field | Type | Description |
|---|---|---|
| `account` | string | Grantor's BTCPC account name |
| `repo_id` | string | `owner/repo` |
| `grantee` | string | Grantee's BTCPC account name |
| `encrypted_key` | string | Repo symmetric key encrypted to grantee's hide public key (hex ECIES ciphertext) |
| `epoch` | u64 | Chain epoch at submission |
| `sig` | string | Hex-encoded signature (grantor's posting key) |

### `LinkGitAccessRevoke`
| Field | Type | Description |
|---|---|---|
| `account` | string | Revoker's BTCPC account name |
| `repo_id` | string | `owner/repo` |
| `grantee` | string | BTCPC account name being revoked |
| `epoch` | u64 | Chain epoch at submission |
| `sig` | string | Hex-encoded signature (revoker's posting key) |

### `LinkGitPruneProof`
| Field | Type | Description |
|---|---|---|
| `account` | string | Storage node's BTCPC account name |
| `repo_id` | string | `owner/repo` |
| `pruned_cids_root` | string | Merkle root of pruned CIDs (hex SHA-256) |
| `pruned_count` | u32 | Number of objects pruned |
| `bytes_freed` | u64 | Total bytes freed by this GC pass |
| `epoch` | u64 | Chain epoch at submission |
| `sig` | string | Hex-encoded signature (storage node's posting key) |

### `LinkGitStorageExtend`
| Field | Type | Description |
|---|---|---|
| `account` | string | Owner's BTCPC account name |
| `repo_id` | string | `owner/repo` |
| `cids` | string[] | List of CIDs to preserve |
| `keep_until_epoch` | u64 | Last epoch at which CIDs must be retained |
| `fee` | u64 | Fee in dreams (must cover storage cost for specified period) |
| `epoch` | u64 | Chain epoch at submission |
| `sig` | string | Hex-encoded signature (owner's posting key) |

---

## Appendix B: Mirror Config Reference

`.linkgit/mirrors`:

```ini
[mirror "github"]
  url = https://github.com/owner/repo
  push = refs/heads/*
  push = refs/tags/*
  push = refs/heads/main:refs/heads/main

[mirror "gitlab"]
  url = https://gitlab.com/owner/repo
  push = refs/heads/*
```

Running `linkgit mirror apply` translates this config into git's native multi-push mechanism by setting `remote.origin.pushurl` entries in `.git/config`. After `apply`, a single `git push origin main` pushes to LinkGit and all configured mirrors in parallel.

---

## Appendix C: Reserved Accounts

| Account | Purpose | Controlled By |
|---|---|---|
| `linkgit` | Protocol authority account; receives protocol fee share | `shindevlin` at genesis; transferable via key rotation |
| `linkgit-registry` | On-chain anchor for repository metadata and access control state | Protocol; no user keys at genesis |

Both accounts are seeded at genesis block 0 with zero BTCPC balance. Keys are registered at sidecar startup via `AccountUpdateKey`.

---

*LinkGit Protocol — Version 1.0 — Shin Devlin — April 2026*
*Part of the BTCPC native protocol suite. See also: [Freeport Protocol Whitepaper](FREEPORT_PROTOCOL_WHITEPAPER.md), [Verasens Protocol Whitepaper](VERASENS_PROTOCOL_WHITEPAPER.md)*
