# Android repository completeness review

Date: 2026-07-31  
Branch: `codex/android-repository-complete`  
Scope: branch-only; no live node, state, account, or key access

## Outcome

The native Android source, Gradle wrapper, UI, foreground service, sensor wire client,
AndroidKeyStore device signer, and tests are tracked together under
`clients/hone-android-native`.

The legacy raw owner-key path was removed. The repository no longer reads or writes an
`owner_posting.seed` file, and the Bouncy Castle dependency used only for that exportable raw-seed
signer was removed. The foreground service no longer embeds an owner account or Grouchly node URL;
it performs read-only sensor capture.

`SensorNodeSettings` now models the required public configuration without defaults. It validates
that chain, owner, node URL, and sensor prefix are explicit. Device submission remains available
through `AndroidSensorKeyStore` and `SensorSubmissionCoordinator`; an owner signer is accepted only
as an operation-scoped callback for enrollment, revocation, or the explicitly selected fallback.

## Verification

- `git diff --check`: pass
- static scan for the deleted signer and hard-coded runtime owner/node defaults: pass
- Android tests added for explicit settings and existing enrollment/device-signing tests retained
- `./gradlew :app:testDebugUnitTest --no-daemon`: not run to completion because this machine has no
  Android SDK location. No `local.properties` was created or modified.

## Remaining product hooks

Submission stays disabled until app state provides an operator-confirmed settings screen, connects
the wallet's one-operation signing callback for enrollment, persists only public enrollment state,
and connects captured readings to the enrolled device signer. Those changes require a product flow
decision and must be exercised against a disposable local node with fresh test accounts.

This is a proposal for review. It is not deployed and does not authorize use against live clocks.
