---
title: HONE Service Host — v2.14 Spec (Stateful Multi-Service Hosting)
description: Turning the runtime job market into a real hosting platform, specified against Bullship as the first customer
author: Shin Devlin
status: draft
supersedes_prototype: src/services/serviceHostRunner.js (v2.13-beta, deprecated Node.js)
---

# HONE Service Host — v2.14

> **First customer, first spec.** Bullship (a Telegram trivia game,
> `github.com/estejosh/bullship`) tried to host itself on HONE and could not.
> It runs today as a Docker Compose stack tunneled from a home PC. Bullship is
> HONE's most realistic first Service-Host customer, so **its requirements are
> the v2.14 spec.** Every milestone below has a Bullship acceptance test.

This document is written against the **canonical Rust node**. The
`serviceHostRunner.js` referenced in the original problem statement is the
**deprecated Node.js prototype** (`src/`, do not extend it — see CLAUDE.md).
The real substrate is already richer than that prototype implied.

---

## 0. What already exists (verified in `rust/hone-node`)

The canonical node ships a **complete runtime job-market lifecycle** — this is
not greenfield. The `LedgerEntry` variants (`crates/hone-types/src/entry.rs`)
and their API routes (`src/api.rs`) already present:

| Entry | Route | Purpose |
|-------|-------|---------|
| `RuntimeRegister` | `POST /api/runtime/register` | Register a runtime: `runtime_id`, `owner`, `manifest_cid`, `runtime_class`, `bond` (dreams → escrow), `nonce`. |
| `RuntimeDeploy` | `POST /api/runtime/deploy` | Place a registered runtime on a `host_id`. |
| `RuntimeUndeploy` | `POST /api/runtime/undeploy` | Remove a deployment. |
| `RuntimeJobEnqueue` | `POST /api/runtime/job/enqueue` | Durable job with `fee` escrowed until attest; `payload_cid`, `due_epoch`, `job_class`. |
| `RuntimeClaim` | `POST /api/runtime/job/claim` | Host claims a lease (`lease_id`, `expires_epoch`). |
| `RuntimeAttest` | `POST /api/runtime/job/attest` | Host publishes `output_commitment` + `runtime_sha` (must match `manifest_cid` — proves it ran the registered code). |
| `RuntimeChallenge` | `POST /api/runtime/challenge` | Challenge an attestation with `evidence_cid`. |
| `RuntimeSlash` | (adjudicated) | Slash a host's bond after a successful challenge. |
| `RuntimeReward` | (system, epoch seal) | Credit a host for completed work. |

Also present and reusable:

- **Encrypted secrets** — `src/secret_store.rs`: AES-256-GCM at rest
  (`~/.hone/secrets.enc`), key from hardware fingerprint or
  `HONE_SECRETS_PASSPHRASE`, RocksDB `secret:` index. Currently used
  node-locally (e.g. TOTP). **Not yet a deploy-time sealed-secret channel.**
- **Node hosting market** — `GET/POST /api/service/node-hosting*`: buyers
  purchase hosting capacity from nodes. This is the commercial layer a stateful
  deploy plugs into.
- **Contract engine** — `src/contracts.rs`, `POST /api/contract/deploy`, with
  per-deployer nonces.

**Conclusion:** HONE has a bonded, escrowed, challenge-slashable *stateless*
compute market. v2.14 is the set of deltas that make it *stateful,
multi-service, secret-bearing, and publicly reachable* — i.e. able to run
Bullship.

---

## 1. The five gaps (Bullship, verified) → v2.14 deltas

Each gap maps to concrete new ledger entries + routes that extend the existing
model rather than replacing it.

### Gap A — Persistent volumes bound to a service
**Bullship needs:** PostgreSQL (accounts, scores, balances) + Redis
(queues/cache). Stateless = no game.

**Delta:**
- New entry `RuntimeVolumeCreate { volume_id, owner, runtime_id, size_bytes, class, replication, bond, epoch, signed_by }` — `class` ∈ `{block, object}`; `replication` = copies to keep. Bond escrows storage rent.
- New entry `RuntimeVolumeAttach { volume_id, runtime_id, mount_path, mode, epoch, signed_by }`.
- Volume data lives in **HONE-FS/Hive replica** (already the binary-distribution layer). A volume is a replicated, content-addressed dataset with a mutable HEAD pointer committed on-chain each snapshot epoch.
- Host-side: the runner mounts the volume before spawning the runtime; on graceful stop it snapshots and commits the new HEAD.
- **Acceptance:** deploy a single Postgres runtime with a 1 GB volume; write a row; kill the host; redeploy; the row is still there.

### Gap B — App bundles (multi-service, private network)
**Bullship is:** API gateway, queue worker, news fetcher, AI-headline service,
blockchain relay, **+ Postgres + Redis**. No way to express "these N services
are one app."

**Delta:**
- New entry `AppBundleRegister { bundle_id, owner, manifest_cid, service_count, bond, epoch, signed_by }`. The bundle manifest (stored in FS at `manifest_cid`) is a **compose-compiled** document: a list of services, each mapping to a `RuntimeRegister`-shaped record, plus a private-network declaration and inter-service DNS names.
- A `hone bundle from-compose docker-compose.yml` tool compiles Compose → bundle manifest (images → runtime_class + binary CID; `depends_on` → start order; named volumes → `RuntimeVolumeCreate`; `environment` → secret refs, see Gap C).
- Deploy is atomic: `RuntimeDeploy` gains an optional `bundle_id`; all services in a bundle place together (or on a placement group, Gap E) and share a private overlay network with stable service DNS (`postgres`, `redis`, `gateway`).
- **Acceptance:** `hone bundle from-compose` on Bullship's real compose file produces a bundle that deploys all services with the gateway able to reach `postgres:5432` and `redis:6379` by name.

### Gap C — Sealed secrets
**Bullship needs at runtime:** `JWT_SECRET`, `TELEGRAM_BOT_TOKEN`, later a Hive
posting key. Handing these to anonymous hosts is unacceptable.

**Delta (reuses `secret_store.rs`):**
- Deploy-time sealing: secrets are encrypted **to the target host's public key** (or to a placement group's threshold key) at deploy time — the chain only ever sees ciphertext CIDs, never plaintext.
- New entry `SealedSecretGrant { grant_id, bundle_id, secret_ref, ciphertext_cid, sealed_to_host, epoch, signed_by }`. `secret_ref` is the env var name; the runtime receives it decrypted **only inside the process**, injected by the runner after it decrypts with its host key inside `secret_store`.
- **Trusted/staked host tier:** a host may `RuntimeRegister` with a `secret_tier` flag backed by a **larger bond**. Only such hosts are eligible targets for `SealedSecretGrant`. Leaking a secret is a slashable `RuntimeChallenge` reason (`secret_exfiltration`), forfeiting the elevated bond.
- **Acceptance:** deploy Bullship's gateway with `JWT_SECRET` + `TELEGRAM_BOT_TOKEN` as sealed secrets to a staked host; the secrets never appear in any on-chain entry, FS object, or non-target host; the gateway boots and validates a JWT.

### Gap D — Ingress (stable public HTTPS per service)
**Bullship is a Telegram Mini App:** needs a stable public HTTPS URL. The runner
binds a host port; there is no routing/TLS layer.

**Delta:**
- New entry `IngressRoute { route_id, bundle_id, service, hostname, target_port, tls, epoch, signed_by }`.
- An **ingress gateway node** (a new node role, opt-in, staked) advertises a wildcard-TLS public hostname (e.g. `*.svc.honemesh.net`) and does **SNI routing** to the current host of the named service, following it across redeploys via the on-chain deployment record. A single gateway node satisfies Bullship day one; multiple gateways = HA later.
- TLS via ACME on the gateway; per-service subdomain `bullship.svc.honemesh.net`.
- **Acceptance:** Bullship's gateway gets a stable `https://<name>.svc.honemesh.net` that survives a host migration, with no home-PC tunnel.

### Gap E — Placement / affinity
Even with volumes, a database must stay on (or follow) its data.

**Delta:**
- `RuntimeDeploy` gains `placement_group` + `affinity` hints. A **placement group** co-locates a bundle's services (or pins stateful ones).
- Volume affinity: a service with an attached `RuntimeVolume` is scheduled to a host that already holds a replica of that volume; if none, the volume replicates to the chosen host first (`RuntimeVolumeAttach` blocks until a local replica exists).
- Migration primitive: `RuntimeMigrate { runtime_id, from_host, to_host, volume_ids, epoch, signed_by }` — drains, snapshots volumes, re-places, re-seals secrets to the new host, updates ingress.
- **Acceptance:** kill Bullship's Postgres host; the placement layer redeploys Postgres to a host holding a volume replica, re-seals secrets, and re-points ingress — game recovers without operator action.

---

## 2. Milestones (priority order = Bullship's a→e)

| # | Milestone | Delivers gap | Bullship value |
|---|-----------|--------------|----------------|
| **M0** | **Public inference endpoint** (works today, no chain change) | — | Restores real AI-written fake headlines immediately (§4). |
| **M1** | Persistent volumes (`RuntimeVolumeCreate/Attach`, FS-backed HEAD) | A | Postgres/Redis can persist. |
| **M2** | **Managed Postgres/Redis as network services** | A (mostly) + B (mostly) | *Likely the cheapest first real hosting win* — Bullship keeps its own app services but rents managed DB/cache, eliminating the hardest state work. |
| **M3** | App bundles + private network (`AppBundleRegister`, compose compiler) | B | Deploy all Bullship services as one unit. |
| **M4** | Sealed secrets + staked-host tier (`SealedSecretGrant`) | C | JWT/Telegram token safe on hosts. |
| **M5** | Ingress gateway (`IngressRoute`, SNI+ACME) | D | Public HTTPS, no tunnel. |
| **M6** | Placement/affinity + migrate (`RuntimeMigrate`) | E | Self-healing stateful placement. |

**M2 is the recommended first funded milestone**: managed DB/cache as a network
service collapses most of gaps A and B for Bullship (and every future
stateful customer) without shipping general volume orchestration first.

---

## 3. Non-negotiables (inherit from chain invariants)

- **No local submission without peers.** A host with zero peers must not accept
  or "confirm" any deploy/attest locally (CLAUDE.md hardline). Deploys are chain
  entries; they seal in an epoch or they didn't happen.
- **Signed + owner-bound.** Every new entry above carries `signed_by`, bound to
  `owner`/`host_id` via `check_signature`, exactly like the sensor/commerce
  entries (`signed_by == owner`). No unauthenticated deploy path.
- **Bonded + slashable.** State, secrets, and ingress all raise the trust
  surface, so each new capability is backed by a bond and a matching
  `RuntimeChallenge` reason + `RuntimeSlash` path.
- **Attestation matches code.** `RuntimeAttest.runtime_sha` must equal the
  registered `manifest_cid` — extend this to bundles (each service's binary) and
  to volume snapshots (snapshot hash committed on attest).

---

## 4. Near-term win — public inference endpoint (do first, no chain change)

Bullship already routes **all** AI generation (`ai-headline`, `oracle`,
`queue_worker` via `hone-sdk`) through the HONE inference API. Its host expects
`http://172.17.0.1:4242` and **falls back to canned mock headlines when the
endpoint is absent.**

**Action:** expose one reliable **public** inference endpoint Bullship can hit
(the node already serves `/v1/chat/completions`, `/v1/models`, `/v1/pricing`).
Point Bullship's SDK base URL at it. This restores real AI-written fake
headlines **before any hosting work lands** — the fastest possible proof that
HONE delivers value to this customer.

This is milestone **M0** and is independent of M1–M6.

---

## 5. Open questions

1. **Volume backend:** reuse HONE-FS/Hive replica for block volumes, or a
   dedicated volume CF? (Leaning: FS for object/snapshot, a thin block layer on
   top for Postgres.)
2. **Ingress trust:** is a single staked gateway acceptable for v2.14, with
   multi-gateway HA deferred? (Bullship: yes.)
3. **Secret threshold:** seal to a single staked host (simple) vs. a threshold
   group (survives one host loss)? M4 ships single-host; threshold is a
   follow-up.
4. **Compose coverage:** which Compose features does the compiler support in v1
   (networks, volumes, depends_on, healthcheck) vs. reject with a clear error?

---

*Cross-refs:* `docs/CONTRACTS.md`, `docs/HONE_FS_HIVE_EXTERNAL_REPLICA_PLAN.md`,
`docs/AGENT_INTEGRATION.md`, and the consumer-facing
`docs/HONE_INTEGRATION_MANIFEST.md` (how a repo like Bullship keeps a
self-updating understanding of this surface).
