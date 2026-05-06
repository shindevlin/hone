# Local OPSEC Audit Rerun (Delta vs Baseline)

Date: 2026-05-05
Baseline: `reports/opsec/baseline/findings-baseline.json`
Evidence dir: `reports/opsec/evidence-rerun-2026-05-05/`

## Scanner Status
- npm audit: fail (exit 1)
- cargo-audit: pass (exit 0)
- gitleaks: fail (exit 1)
- osv-scanner: fail/findings (exit 1)
- slither: pass command, but no contracts analyzed due compile/import errors
- trivy fs: pass command (exit 0)

## Findings Counts (Now)
- npm: high 2, critical 0, total 5
- gitleaks: 48
- trivy: high 20, critical 2, total 22

## Delta vs Baseline
- npm: no change (high 2, total 5)
- gitleaks: no change (48)
- trivy: improved by 26 total findings
  - high: 34 -> 20 (down 14)
  - critical: 14 -> 2 (down 12)

## Notes
- OSV local command now runs with correct syntax and reports findings (non-zero exit).
- Slither still has coverage gap because Solidity sources do not compile cleanly; static-analysis confidence for contracts remains limited.
- `gitleaks` repo config is malformed (`paths` key duplicated). Rerun used explicit default config override to produce comparable output.
