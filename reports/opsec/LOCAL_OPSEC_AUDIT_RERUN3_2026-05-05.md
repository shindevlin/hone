# Local OPSEC Audit Rerun #3

Date: 2026-05-05
Evidence: `reports/opsec/evidence-2026-05-05-run3/`
Compared against:
- Baseline: `reports/opsec/baseline/findings-baseline.json`
- Prior rerun: `reports/opsec/evidence-rerun-2026-05-05/`

## Scanner Status
- npm audit: fail (ENOLOCK; lockfile missing in current workspace root)
- cargo-audit: pass (exit 0)
- gitleaks: fail/findings (exit 1)
- osv-scanner: fail/findings (exit 1)
- slither: pass command, but no contract analyzed (compile/import errors)
- trivy: pass command (exit 0)

## Counts
- gitleaks: 48 (no change vs prior rerun)
- trivy: 15 total (13 high, 2 critical)

## Delta
Vs prior rerun:
- trivy: total -7 (high -7, critical 0)
- gitleaks: 0 change

Vs baseline:
- trivy: total -7 (high -7, critical 0)
- gitleaks: unchanged at 48

## Caveats
- npm delta cannot be compared in this run due ENOLOCK in current directory state.
- Slither contract coverage remains blocked by Solidity compile/import issues.
