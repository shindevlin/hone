---
title: Secret Store Design
description: Design decision — local file is the source of truth, not RocksDB
captured_at: 2026-05-09
author: Shin Devlin
---

# Secret Store Design

## Decision
Local file (`~/.hone/secrets.enc`) is the **source of truth**. RocksDB CF_META is the runtime index only.

## Rationale
User must be able to delete or encrypt secrets with their own passphrase. A DB-only store is opaque.

## Spec
- File format: AES-256-GCM encrypted JSON, key from hw fingerprint by default.
- If `HONE_SECRETS_PASSPHRASE` env is set: use that instead — node prompts at startup.
- Delete the file → secrets gone (clean slate on next start).
- RocksDB prefix `secret:{key}` populated from file at boot, kept in sync on every write.
- Methods: `get(key)`, `set(key, val)`, `delete(key)`, `scan_prefix(pfx)`.
