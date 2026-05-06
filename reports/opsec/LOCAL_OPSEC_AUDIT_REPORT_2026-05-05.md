# Local OPSEC Audit Report (Soft Gate)

Date: 2026-05-05
Mode: soft (informational, non-blocking)
Scope: local equivalent of `.github/workflows/opsec-audit.yml`

## Scanner Status
- npm audit: **fail** (exit 1)
- cargo-audit: **pass** (exit 0)
- gitleaks: **fail** (exit 1)
- osv-scanner: **error** (exit 127, CLI arg mismatch)
- slither: **pass with analysis gap** (exit 0, no contracts analyzed due compilation/import issues)
- trivy fs: **pass command / findings present** (exit 0, vulnerabilities detected)

## Findings Snapshot (Baseline Run)
- npm audit vulnerabilities: total 5, high 2, critical 0
- npm high packages: `@solana/web3.js`, `axios`
- gitleaks findings: 48 potential secrets
- gitleaks top rules: `generic-api-key` (32), `curl-auth-header` (16)
- trivy vulnerabilities: total 48, high 34, critical 14

## New vs Existing Delta
No previous baseline file was present.
- New findings: all current findings are marked **new** for this initial baseline.
- Existing findings: 0
- Resolved findings: 0

## Reliability Notes
- OSV scanner invocation in local docker run used an invalid flag (`-skip-git`).
- Slither could not compile current contracts due syntax/import issues, so smart-contract static coverage is incomplete.

## Evidence
- `reports/opsec/evidence/npm-audit.json`
- `reports/opsec/evidence/cargo-audit.txt`
- `reports/opsec/evidence/gitleaks.json`
- `reports/opsec/evidence/osv-scanner.stderr`
- `reports/opsec/evidence/slither.stderr`
- `reports/opsec/evidence/trivy.json`
