# BTCPC Sensor Data Monetization Plan

Version context: v3.0.96 working tree, Mongo disabled, block files are source of truth, stateStore is the rebuilt cache.

## Current Status

### Desktop App

The Electron wrapper exists locally at `desktop/main.js`, and Linux desktop binaries exist under `website/`:

- `website/BTCPC-desktop-linux.AppImage`
- `website/btcpc-desktop-linux.deb`

Problem: the currently tracked `package.json` does not include Electron dependencies or `desktop` scripts. The desktop folder is untracked in this worktree. Before this can be treated as a supported build, package wiring must be restored and committed with the desktop files.

Required next desktop work:

- Add `electron` to `devDependencies`.
- Add scripts: `desktop`, `desktop:dev`, and a packaging script if the existing binaries are expected to be reproducible.
- Decide whether committed desktop binaries belong in `website/` or should be release artifacts only.
- Run a clean install/build on a fresh checkout to confirm the wrapper works without local-only files.

### Android App

The Capacitor Android project exists locally under `android/`, with a debug APK at `android/app/build/outputs/apk/debug/app-debug.apk`.

Native sensor bridge exists at:

- `android/app/src/main/java/network/btcpc/app/BTCPCSensorsPlugin.java`

Current native bridge behavior:

- Uses Android `SensorManager`.
- Lists `Sensor.TYPE_ALL`.
- Streams known sensors as BTCPC sensor ids: `motion`, `orientation`, `light`, `magnetometer`, `barometer`, `proximity`, `steps`, `heart-rate`.
- Streams unknown native sensors as `android-<type>`, so APK-only sensors are not hidden.

Manifest permissions currently include:

- `INTERNET`
- `ACCESS_FINE_LOCATION`
- `ACCESS_COARSE_LOCATION`
- `ACTIVITY_RECOGNITION`
- `BODY_SENSORS`

Problem: the tracked `package.json` currently does not include Capacitor dependencies or Android scripts, and `android/` is untracked. This means the Android work is present locally but not reproducible from a fresh checkout.

Required next Android work:

- Add `@capacitor/core`, `@capacitor/cli`, and `@capacitor/android` to `devDependencies`.
- Add scripts: `android:sync`, `android:open`, `android:build`.
- Decide whether `android/local.properties` stays ignored, as it should.
- Rebuild with Java 21 JDK and Android SDK.
- Install APK on a real Android device and verify runtime permissions plus native sensor cards.

### Sensor API / Monetization

Tracked sensor API currently supports:

- `POST /api/sensors` register sensor.
- `POST /api/sensors/:id/readings` submit single numeric reading.
- `POST /api/sensors/:id/finalize` finalize readings and write `SENSOR_DATA_COMMIT`.
- `GET /api/sensors` list sensors.
- `GET /api/sensors/:id` inspect a sensor.
- Gateway registration, heartbeat, retire, and list routes.

Ledger/state already supports useful primitives:

- `TRANSFER`
- `ESCROW_LOCK`
- `ESCROW_RELEASE`
- `ESCROW_REFUND`
- `SENSOR_REGISTER`
- `SENSOR_READING`
- `SENSOR_DATA_COMMIT`

Missing billing layer:

- No query pricing engine.
- No paid sensor query endpoint.
- No per-sensor payment distribution.
- No protocol split to `shindevlin` and `natoshisakamoto`.
- No token-native wallet funded query flow.
- No enterprise rate-card/grant/pre-fund flow.
- No combined sensor query + inference analysis endpoint.

## Product Review

The idea is strong because it creates a direct market between useful real-world data and useful compute. The important differentiator is not just selling sensor data; it is selling sensor data plus immediate AI analysis through the same chain-native payment rail.

The clean user story is:

1. A consumer asks for a data shape, not a sensor.
2. BTCPC finds matching readings.
3. BTCPC prices the query before delivery.
4. BTCPC locks or charges funds.
5. BTCPC returns data.
6. BTCPC automatically pays sensor operators, protocol treasury accounts, and recycle.
7. Optional: BTCPC runs inference over the returned data and includes analysis.

This aligns with BTCPC because sensor operators earn by producing requested data, not just by existing.

Main risk: privacy and data quality. GPS and device data cannot leak exact physical locations by default. All query APIs should support fuzzed/metro-level data unless the sensor explicitly publishes precise readings. Query results should carry quality metadata: source count, time range, freshness, aggregation level, and confidence.

## Proposed Economic Model

Use basis points for all splits to avoid floating-point policy bugs.

Default split for data fee:

- Sensor operators: 70%
- Protocol treasury: 20%
- Recycle: 10%

Protocol treasury split:

- `shindevlin`: 50% of protocol share
- `natoshisakamoto`: 50% of protocol share

Example for 100 BTCPC data fee:

- 70 BTCPC split among sensor owners by returned reading count.
- 10 BTCPC to `shindevlin`.
- 10 BTCPC to `natoshisakamoto`.
- 10 BTCPC to `btcpc_recycle`.

This should be configurable in one policy object, not scattered across routes.

## Pricing Dimensions

Base formula:

```text
data_fee = reading_count
  * base_price_by_type
  * age_multiplier
  * resolution_multiplier
  * account_discount
```

Sensor type multiplier:

- `gps`: high
- `barometer`, `magnetometer`, `heart-rate`, `android-*`: medium/high depending on scarcity.
- `temperature`, `humidity`, `load`, `battery`: low.
- `custom`: default medium until classified.

Time depth multiplier:

- Recent, last 1 hour: cheapest.
- Last 24 hours: normal.
- Historical older than 24 hours: higher because it depends on durable storage and indexing.

Resolution multiplier:

- Aggregated per epoch: cheap.
- Raw readings: full price.
- Raw plus metadata: premium.

Volume discounts:

- Token-native users get deterministic per-query pricing.
- Enterprise users can get discounted `account_discount` based on prefunded tier or negotiated rate card.

## Access Tiers

### Token-Native

Goal: no signup beyond having a BTCPC wallet/account.

Authentication options:

- Bearer session token tied to account.
- Signed request using active/posting key.
- Existing JWT where available.

Flow:

1. Client calls quote endpoint.
2. Client submits query with payer account/signature.
3. System locks or directly debits the quoted amount.
4. System returns data and writes payout ledger entries.
5. If result count is lower than quoted max, refund overage automatically.

### Enterprise

Goal: stable API usage for high-volume consumers.

Enterprise account fields should live in chain-backed project/account metadata, not Mongo:

- `account`
- `rate_card_id`
- `discount_bps`
- `monthly_commitment`
- `prefunded_balance`
- `webhook_url`
- `rate_limit_class`

Flow:

1. Enterprise pre-funds account/project wallet.
2. API key or session token identifies project/account.
3. Queries debit from prefunded BTCPC balance.
4. Webhook can receive pushed datasets or query completion events.
5. Invoice endpoint can summarize on-chain debits, not act as source of truth.

## API Design

### Quote

`POST /v1/sensor-data/quote`

Body:

```json
{
  "type": "gps",
  "region": "sv-san-salvador",
  "from": "2026-04-15T10:00:00Z",
  "to": "2026-04-15T16:00:00Z",
  "resolution": "raw",
  "limit": 1000
}
```

Response:

```json
{
  "quote_id": "sdq_...",
  "estimated_readings": 742,
  "max_cost": 0.742,
  "price_per_reading": 0.001,
  "currency": "BTCPC",
  "expires_in_seconds": 60,
  "rate_card": {
    "type_multiplier": 1.5,
    "age_multiplier": 1,
    "resolution_multiplier": 1
  }
}
```

### Query

`POST /v1/sensor-data/query`

Body:

```json
{
  "quote_id": "sdq_...",
  "payer": "consumer",
  "type": "gps",
  "region": "sv-san-salvador",
  "from": "2026-04-15T10:00:00Z",
  "to": "2026-04-15T16:00:00Z",
  "resolution": "raw",
  "limit": 1000
}
```

Response:

```json
{
  "query_id": "sdqrun_...",
  "readings_returned": 742,
  "charged": 0.742,
  "refund": 0,
  "data": [],
  "payouts": [
    { "account": "sensor-owner-a", "amount": 0.42, "readings": 420 },
    { "account": "sensor-owner-b", "amount": 0.322, "readings": 322 }
  ]
}
```

### AI-Integrated Query

`POST /v1/sensor-data/analyze`

Body:

```json
{
  "payer": "consumer",
  "query": {
    "type": "gps",
    "network": "nebra",
    "from": "now-6h",
    "resolution": "raw"
  },
  "prompt": "Tell me if any units are showing irregular movement.",
  "model": "auto"
}
```

System behavior:

1. Price sensor data.
2. Estimate inference cost.
3. Lock escrow for `data_max_cost + inference_max_cost`.
4. Execute sensor query.
5. Run inference with the returned data as context.
6. Charge actual data cost and actual inference cost.
7. Pay sensor operators from data cost.
8. Pay inference miner through existing inference settlement.
9. Refund unused escrow.

Response:

```json
{
  "query_id": "sdqrun_...",
  "inference_id": "btcpc-...",
  "charged": {
    "data": 0.742,
    "inference": 0.031
  },
  "readings_returned": 742,
  "data": [],
  "analysis": "Two units show drift inconsistent with their normal fixed base station behavior..."
}
```

## Billing Service Design

Create `src/services/sensorDataBilling.js`.

Responsibilities:

- Normalize query filters.
- Estimate matching reading count.
- Calculate quote.
- Lock escrow or direct debit.
- Calculate actual charge from returned readings.
- Calculate per-owner splits.
- Write ledger entries.
- Refund overage.
- Return an auditable billing summary.

Suggested exports:

```js
quoteSensorQuery(params, accountContext)
executePaidSensorQuery(params, accountContext)
executeSensorAnalysis(params, accountContext)
calculateRate(params, accountContext)
calculatePayouts(readings, totalFee)
settleSensorDataPayment(queryId, payer, totalFee, payouts, epoch)
```

## Payment Settlement Mechanics

Use escrow for multi-step queries and AI-integrated queries.

Recommended settlement entries:

1. `ESCROW_LOCK` from consumer to `btcpc_escrow`.
2. `ESCROW_RELEASE` to each sensor owner.
3. `ESCROW_RELEASE` to `shindevlin`.
4. `ESCROW_RELEASE` to `natoshisakamoto`.
5. `ESCROW_RELEASE` to `btcpc_recycle`.
6. `ESCROW_REFUND` to payer for unused max quote.

Reason: this keeps all payment movements chain-visible and uses existing ledger/stateStore mechanics.

If direct pay-per-query is needed for simple token-native queries, use `recordTransfer` from payer to each recipient. Escrow is safer for quotes, limits, and AI calls because returned reading count can differ from estimated count.

## Reading Attribution

Each returned reading must include internal attribution fields used for settlement:

- `sensor_id`
- `owner`
- `type`
- `epoch`
- `timestamp`
- `quality_score`

The public response can omit physical sensor identity by default:

```json
{
  "source": "anonymous",
  "type": "gps",
  "region": "sv-san-salvador",
  "timestamp": 1776250000000,
  "value": {},
  "quality": 0.98
}
```

Settlement still uses owner internally.

## Privacy Rules

Default GPS mode must be privacy-preserving:

- Return metro/region-level or fuzzed coordinates by default.
- Require explicit `precision: "raw"` and sensor opt-in for precise coordinates.
- Never expose account names in public data unless `include_attribution: true` and the caller has elevated access.

Builder access exception:

- `shindevlin` and `natoshisakamoto` can view operational network data for build/debug work.
- This should be authenticated and audited, not query-param based.

## Implementation Phases

### Phase 1: Reproducible App Wiring

Goal: make desktop and Android buildable from clean checkout.

Tasks:

- Restore Capacitor/Electron dependencies and scripts to `package.json`.
- Ensure `package-lock.json` version/deps match.
- Commit `capacitor.config.json`, `android/`, and `desktop/` as source, excluding generated build artifacts.
- Add/update `.gitignore` to exclude APK build outputs, Gradle caches, local SDK config, AppImage/deb unless intentionally released.
- Verify `npm run android:build`.
- Verify `npm run desktop` launches locally.

### Phase 2: Sensor Query Index

Goal: query data by type/time/region without scanning everything blindly.

Tasks:

- Add read APIs in `sensorRegistry` or a new `sensorQueryStore`.
- Return readings by filters: type, region, network, sensor owner, epoch range, timestamp range.
- Support `raw` and `epoch_aggregate`.
- Include internal attribution for settlement.
- Add tests with multi-owner readings.

### Phase 3: Billing Engine

Goal: quote and settle sensor data queries.

Tasks:

- Create `src/services/sensorDataBilling.js`.
- Implement rate card constants.
- Implement quote calculation.
- Implement split calculation by owner reading count.
- Implement protocol/recycle splits.
- Implement escrow lock/release/refund path.
- Add tests for exact split math and rounding.

### Phase 4: Paid API Routes

Goal: expose token-native and enterprise APIs.

Tasks:

- Create `src/routes/sensorDataRoutes.js`.
- Mount under `/v1/sensor-data` in `src/index.js`.
- Add `POST /quote`.
- Add `POST /query`.
- Add `POST /analyze`.
- Add `GET /rate-card`.
- Add `GET /usage` for enterprise/project accounts.
- Authenticate with existing JWT/session/project key patterns.
- Rate limit by tier.

### Phase 5: AI-Integrated Endpoint

Goal: combine data and inference in one paid call.

Tasks:

- Reuse existing inference submission path rather than duplicating miner logic.
- Build compact prompt context from returned readings.
- Cap data size passed to model; summarize/aggregate if too large.
- Lock combined escrow.
- Settle data fee first, inference fee second.
- Refund unused escrow on inference failure.
- Add tests for inference unavailable path: data escrow must refund or only charge data if explicitly configured.

### Phase 6: Enterprise Features

Goal: make this usable for real data buyers.

Tasks:

- Rate-card metadata per account/project.
- Prefunded enterprise balance.
- Higher rate limits.
- Optional webhook pushes.
- Usage export endpoint.
- Monthly summary generated from ledger entries.

## Tests Required

Add test files:

- `tests/sensorDataBilling.test.js`
- `tests/sensorDataRoutes.test.js`
- `tests/sensorDataAnalyze.test.js`

Minimum coverage:

- Quote by sensor type.
- Historical multiplier.
- Raw vs aggregate multiplier.
- Bulk discount.
- Multi-owner payout proportional to returned readings.
- Protocol split to `shindevlin` and `natoshisakamoto`.
- Remainder to `btcpc_recycle`.
- Escrow overquote refund.
- Empty result costs zero or minimum query fee, depending policy.
- AI endpoint refunds inference escrow on model failure.
- Public response hides sensor identity by default.
- Builder access requires authenticated `shindevlin` or `natoshisakamoto`.

## Open Decisions

- Exact basis point split. Current recommendation: 70% sensors, 20% protocol, 10% recycle.
- Whether protocol fee goes directly to `shindevlin`/`natoshisakamoto` or through `btcpc_treasury`.
- Minimum charge for empty queries.
- Whether raw GPS precision is ever sellable by default. Recommendation: no, require opt-in.
- Whether enterprise invoices are off-chain PDFs or on-chain summaries only.
- Whether AppImage/deb/APK artifacts should be committed or released separately.

## First Safe Build Step

Implement Phase 1 and Phase 3 first.

Reason:

- Phase 1 makes app work reproducible.
- Phase 3 creates the missing billing core without exposing a public paid endpoint prematurely.
- Once billing tests pass, adding routes is straightforward and lower risk.
