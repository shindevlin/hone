---
title: HONE Read-Leak Security Audit (2026-07-04)
description: Swept all data-returning endpoints for the "returns private data without checking who asks" bug class. Findings + fixes.
author: Shin Devlin
status: audit complete (2 fixes shipped, follow-ups noted)
---

# Read-Leak Audit

**Trigger:** adding a LinkGit repo-discovery endpoint surfaced that `visibility`
(public/private) was stored but NOT enforced on reads — a private repo would leak.
Shin: "private means only the keyholder can see it." Same-class bugs travel
together, so all data-returning endpoints were swept for the pattern: *does this
return owner/buyer/private data to a caller who hasn't proven they're allowed?*

## Fixed (shipped)

### 1. LinkGit private repos — keyholder-gated (commits d78c3193, 99af60df)
- Discovery `/api/linkgit/repos` + owner list → PUBLIC repos only.
- Single repo GET → private returns a minimal stub, not the ref list.
- **`POST /api/linkgit/repo/:owner/:repo/read`** → full private data ONLY when the
  caller signs the challenge `hone:linkgit:read:{repo_id}:{caller}` with their
  HIDE key (verified against on-chain hide pubkey) AND is owner/grantee
  (`linkgit_server::can_read`). 401 not-keyholder, 403 not-authorized. Read API +
  git-serve layer share one access predicate.

### 2. Agent sessions — owner-gated (commit f169355c)
- `GET /api/agent-session/:id` was returning the full session + all turns to
  anyone with the id (leaking client identity, model, fee; turns are E2E
  ciphertext but metadata leaked). Now: full record ONLY to the client who signs
  the session_id with their session key (`client_pubkey`, via `?sig=`); others get
  an existence/status stub. Added `verify_pubkey_sig`.

## Audited CLEAN (no change needed)

- **Freeport / hone-market orders** — `get_order` already checks
  `caller == buyer || seller` and requires `AuthUser`; `license_key`/fulfillment
  only go to the parties. Node `/api/commerce/*` routes are public storefront
  listings only.
- **TOTP / 2FA** — the secret is returned only in the enroll SETUP response (by
  design, to configure the authenticator). Verify endpoints take a code and check
  it; they never return the secret.
- **Tracker route** — `get_tracker_route` requires a Verified `tracker_claim` for
  the named account before returning location waypoints (403 otherwise). A lost
  item's location history is not exposed to the public.
- **Purchase** — payment flows; status is order-scoped, no private-key/secret
  exposure found.

## Follow-ups (noted, not blocking)

- **Tracker route caller-auth:** `get_tracker_route` verifies a claim EXISTS for
  `?account=`, but does not cryptographically verify the caller IS that account
  (no signature). Lower risk (needs the serial_commitment + a verified claim), but
  should get the same signature gate as LinkGit for defence-in-depth.
- **LinkGit authenticated owner list:** the unauth owner-list returns only public
  repos — even to the owner. A signed variant should return the caller's own +
  granted private repos.

## Pattern / principle established
Any endpoint returning private/owner/buyer data must either (a) require a
signature proving the caller is the authorized party, or (b) return only a
redacted/public view. Default-deny. New read endpoints must be checked against
this before merge.
