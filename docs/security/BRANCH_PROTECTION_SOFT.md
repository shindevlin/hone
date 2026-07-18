# Soft Branch Protection Plan (HONE)

This is a soft rollout. We run OPSEC checks and review results before hard enforcement.

## Current Mode
- Workflow: `.github/workflows/opsec-audit.yml`
- Default behavior: report findings without blocking merges (`enforce_gate=false`)

## Recommended GitHub Settings (Soft)
- Enable branch protection on `main`
- Do **not** require OPSEC checks yet
- Add `OPSEC Gate Summary` to visible status checks for reviewer awareness

## Hardening Path
1. Stabilize scanner noise and false positives for 1-2 weeks.
2. Triage existing backlog to acceptable baseline.
3. Flip `enforce_gate=true` in reusable workflow calls for selected repos.
4. Mark `OPSEC Gate Summary` as required in branch protection.
