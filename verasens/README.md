# verasens

Sensor verification protocol for BTCPC — authenticates physical sensor devices
(GNSS, IoT, environmental) against the chain's hardware identity registry.

Device serial numbers are anchored on-chain via the hardware claim stake model.
Verasens validates sensor data provenance before it enters the reward pipeline.

## Status

The Verasens aggregation service is implemented — see
[`rust/verasens/`](../rust/verasens/) for the actual crate (Rust, Axum HTTP
API). This directory is kept as a stable doc pointer; no code lives here.

- Ingests `SensorReading` events (chain type: `crates/btcpc-types/src/entry.rs`)
- Groups by sensor class + geography + time window per the ratified
  `metadata` JSON convention (see `docs/PLATFORM_PRD.md` Phase 1.2)
- Exposes a query API returning aggregated stats, verified/unverified
  breakdown, and per-result provenance — not raw per-reading dumps

See [docs/](../docs/) for hardware device architecture and the Phase 1.2
audit/decision record.
