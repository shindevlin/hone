# Sensor Data Market Build Notes

Branch: `feature/sensor-data-market-apps`

Started from a dirty `main` worktree containing:

- v3.0.96 live-log fixes.
- BTCPC logo integration.
- Local Android Capacitor scaffold.
- Local Electron desktop wrapper.
- Sensor data monetization plan at `docs/SENSOR_DATA_MONETIZATION_PLAN.md`.

## Build Goals

1. Make desktop and Android work reproducible from a clean checkout.
2. Build the missing sensor data billing layer:
   - query filters
   - quote calculation
   - escrow/payment settlement
   - per-sensor-owner payout distribution
   - protocol treasury split to `shindevlin` and `natoshisakamoto`
   - recycle split to `btcpc_recycle`
3. Expose paid sensor data API routes.
4. Add AI-integrated sensor-data + inference endpoint after the billing path is safe.
5. Keep notes detailed enough that another LLM can resume without conversation context.

## Constraints

- MongoDB remains disabled.
- State source of truth is block files plus ledger entries; `stateStore` is cache.
- No burn. All remainder routes to `btcpc_recycle`.
- Always bump `package.json` and `package-lock.json` together before commit.
- Do not rely on local-only generated build outputs as source.
- Fail paths should self-heal or refund automatically.

## Current Split

- App packaging worker `019d9221-8fff-7a71-bf46-764b38285060` owns `package.json`, `package-lock.json`, `capacitor.config.json`, `android/`, `desktop/`, and app build ignores/scripts.
- Billing worker `019d9222-0368-7163-8481-7de1815597bc` owns `src/services/sensorDataBilling.js` and `tests/sensorDataBilling.test.js`.
- API worker `019d9222-21bf-7ec0-88a7-9f5478a52007` owns `src/routes/sensorDataRoutes.js` and `tests/sensorDataRoutes.test.js`.
- Main thread owns coordination, final integration, route mounting, version checks, and final verification.

## Packaging Progress

- `package.json` scripts restored for Electron and Capacitor app entrypoints.
- `package-lock.json` is in sync with the tracked version number and the app package additions.
- `.gitignore` now excludes Android build outputs, local SDK config, desktop release artifacts, and the generated Electron/Capacitor caches that should not be committed.
- Remaining packaging follow-up is to decide whether the untracked `android/` and `desktop/` trees are being committed as source or regenerated as part of release packaging.

## Open Decisions

- Default data-fee split currently planned as 70% sensor operators, 20% protocol, 10% recycle.
- Protocol share currently planned as 50/50 to `shindevlin` and `natoshisakamoto`.
- Empty query fee policy unresolved. Safer default: no data fee, but allow a small quoted planning fee later.
- Raw GPS should not be default. Default response should anonymize source and use privacy-preserving coordinates unless precise access is explicitly requested and the sensor opted in.

## Verification Log

- `src/routes/sensorDataRoutes.js` now exists as a thin router shell.
- It currently returns `503` when `src/services/sensorDataBilling.js` is absent.
- The route is not mounted in `src/index.js` yet by design; that should wait until the billing service lands.
- Pending: implement `src/services/sensorDataBilling.js` and mount `/v1/sensor-data` in the API entrypoint.
- Billing core is now implemented in `src/services/sensorDataBilling.js`.
- It covers quote generation, proportional owner payouts, protocol split, recycle remainder, and escrow settlement with refund handling.
- Focused tests exist in `tests/sensorDataBilling.test.js` and pass in-band.
