# BTCPC API Catalog Library

## Purpose
`btcpc-api-catalog` turns `public-apis/public-apis` into a local, versioned BTCPC library that LLM models, agents, runtimes, and service workers can query internally.

The GitHub repo remains the mother hub. BTCPC snapshots it, verifies links, records the upstream commit, and serves the result locally. Consensus logic should not call arbitrary third-party APIs directly.

## Rust Crate
Path:

```text
rust/btcpc-api-catalog
```

Capabilities:
- Parse `public-apis` `README.md` into structured records.
- Preserve source repo, source commit, source line, and snapshot hash.
- Flag basic risk conditions: auth required, HTTP-only, unknown CORS, tracking params.
- Search by text, category, HTTPS-only, and secretless-only filters.
- Verify links with a Rust HTTP verifier.
- Save/load JSON snapshots for local runtime use.

## Update Flow
1. Keep a local checkout of `https://github.com/public-apis/public-apis` outside the main BTCPC repo.
2. Periodically `git fetch` and fast-forward it.
3. Run the Rust snapshotter against that checkout.
4. Verify links in batches.
5. Store the signed/hash-addressed snapshot where BTCPC runtimes and LLM agents can query it.

Example:

```bash
git clone https://github.com/public-apis/public-apis /mnt/btcpc-storage/mirrors/public-apis
cd /mnt/btcpc-storage/mirrors/public-apis && git pull --ff-only

cargo run --manifest-path rust/btcpc-api-catalog/Cargo.toml -- \
  snapshot /mnt/btcpc-storage/mirrors/public-apis \
  /mnt/btcpc-storage/catalogs/public-apis.snapshot.json \
  --verify --limit 250
```

Remove `--limit` when running the full verifier. The first full pass may be slow because the upstream catalog has more than 1,400 entries.

## First Orchestrator Smoke Test
After creating a snapshot, run one API-tool job through the Rust sidecar:

```bash
cargo run --manifest-path rust/btcpc-orchestrator/Cargo.toml -- \
  run-api-tool \
  --catalog /mnt/btcpc-storage/catalogs/public-apis.snapshot.json \
  --category Weather \
  --query Open-Meteo \
  --out /tmp/btcpc-api-tool-report.json
```

A successful report should show:
- `job.status = "Succeeded"`
- `attempt.status = "Succeeded"`
- `span.attributes.http.status = 200`
- non-null `attestation`

## LLM / Runtime Use
The catalog is useful as a tool-selection index. A model can ask for:
- public weather APIs with no auth
- blockchain explorer APIs
- open-data APIs
- APIs that support HTTPS and CORS
- APIs with known bad verification status excluded

The runtime should pass only the selected API metadata to the model, not the full catalog.

## Reward Boundary
A node should not earn BTCPC merely for listing an API. Rewards should require work:
- verified API availability checks
- successful proxied service execution
- reproducible response hash commitments
- signed span/attestation submission
- challengeable failure evidence

## Security Rules
- Treat catalog contents as untrusted external metadata.
- Never place API keys in the catalog snapshot.
- Do not let model-selected URLs bypass allowlists for high-risk workflows.
- Do not make catalog snapshots consensus-critical until verification is deterministic and bounded.
- Keep source commit and snapshot hash with every model/tool execution for replay and audit.
