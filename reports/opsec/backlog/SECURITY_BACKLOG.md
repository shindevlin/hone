# Security Backlog (From OPSEC Baseline 2026-05-05)

## OPSEC-2026-001
Severity: High
Title: JS dependency advisories in production tree
Source: npm audit
Status: Resolved — 2026-05-05
Resolution: Root `package.json` and `package-lock.json` deleted. The Node.js prototype
  was deprecated; there is no npm surface in production. `npm audit` no longer applicable.

## OPSEC-2026-002
Severity: High
Title: Potential secret material in repository history/content
Source: gitleaks (48 findings)
Status: Resolved — 2026-05-05
Resolution: All 48 findings were false positives (SHA-256 file hashes in
  `data/model-registry.json` and `Authorization: Bearer` examples in docs). Fixed
  `.gitleaks.toml`: merged duplicate `paths` keys into one block, anchored hash regex
  to `^[0-9a-f]{64}$`. Subsequent scans: 0 findings.
Open follow-up (OPSEC-2026-006): `.env` contents committed in git history at
  4af4a768–2b66a8c2 (2026-03-30/31). Credentials should be rotated; history scrub
  deferred until pre-public-launch.

## OPSEC-2026-003
Severity: High
Title: Trivy reported HIGH/CRITICAL vulnerabilities in filesystem dependencies
Source: trivy fs scan
Status: Resolved — 2026-05-05
Resolution:
  - CVE-2026-33040 / CVE-2026-34219 (gossipsub): libp2p 0.55→0.56 (gossipsub 0.49.4)
  - GHSA-82j2-j2ch-gfr8 (rustls-webpki): reqwest 0.11→0.12 in btcpc-miner; cargo
    update in ort-patched (0.103.5→0.103.13)
  - CVE-2026-41676/78/81 / GHSA-hppc (openssl): cargo update 0.10.73→0.10.79
  - CVE-2026-32314 (yamux): upstream-blocked, accepted risk, suppressed in
    `.trivyignore`; mitigated by noise auth layer and tokio supervision loop in net.rs
  - btcpc/ dashboard CVEs: pre-deploy, suppressed in `.trivyignore` pending dep bumps
  Subsequent scans: trivy 0 findings (suppressions valid).

## OPSEC-2026-004
Severity: Medium
Title: Solidity audit blind spot (contracts not analyzable)
Source: slither compile/import failures
Status: Open
Owner: Unassigned
Target milestone: Pre-mainnet
Action:
  - Fix Solidity import paths so Slither can parse contracts
  - Gate on CI once analyzable

## OPSEC-2026-005
Severity: Medium
Title: OSV local invocation mismatch caused scan failure
Source: osv-scanner runtime
Status: Resolved — 2026-05-05
Resolution: Scanner invocation fixed; osv-scanner now runs cleanly in audit script.

## OPSEC-2026-006
Severity: High
Title: Credentials committed in git history (.env, 2026-03-30/31)
Source: Manual finding during OPSEC audit
Status: Open
Owner: Shin Devlin
Target milestone: Pre-public-launch (before GitHub/Codeberg repo goes public)
Action:
  - Rotate all credentials that were in .env at commits 4af4a768–2b66a8c2:
    TELEGRAM_BOT_TOKEN, HIVE_PRIVATE_KEY, TON_PASSPHRASE, JWT_SECRET, MONGODB_URI
  - Run `git filter-repo --path .env --invert-paths` to scrub history
  - Force-push cleaned history to all remotes
  Note: repo is currently private; rotation is the priority, history scrub can follow.

## OPSEC-2026-007
Severity: High
Title: Plaintext credentials in git remote URL
Source: Manual finding during OPSEC audit
Status: Resolved — 2026-05-05
Resolution: Codeberg token was embedded in remote URL in `.git/config`. Removed by
  resetting both remote URLs to bare HTTPS (no credentials). Token should be rotated
  in Codeberg account settings.
