# Scanner Rebuild Verification

Date: 2026-05-05

## What was rebuilt
- Added deterministic local runner: `scripts/opsec-runner.sh`
- Updated CI workflow scanner invocations to pass explicit config/ignore files:
  - gitleaks: `--config=/repo/.gitleaks.toml --source=/repo`
  - trivy: `--ignorefile /repo/.trivyignore`

## Verification run
Command:
- `scripts/opsec-runner.sh reports/opsec/evidence-rebuild-2026-05-05`

Result:
- gitleaks findings: `0`
- trivy findings: `2 high`, `0 critical`

Remaining Trivy findings:
- `GHSA-82j2-j2ch-gfr8` in `android/rust/hone-miner/Cargo.lock` (`rustls-webpki 0.101.7`)
- `GHSA-82j2-j2ch-gfr8` in `android/rust/ort-patched/Cargo.lock` (`rustls-webpki 0.103.5`)

Conclusion:
- Suppression/config loading issue is fixed.
- gitleaks noise is eliminated.
- Two unsuppressed Trivy findings remain and are currently real output unless explicitly suppressed.
