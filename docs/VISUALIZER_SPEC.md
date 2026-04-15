# The Global Nervous System — Visualizer Spec

Real-time, interactive bridge between raw blockchain data and cinematic UX.
Visualizes storage and inference across the decentralized network.

## Architecture

- WebGL-based geospatial visualizer
- 3D Cinematic Globe + 2D Command Map toggle
- Coordinate privacy: map nodes to metro-area buckets, then place each account
  deterministically in a 15-50km metro-sector ring. This is intentionally
  wider than exact-coordinate jitter so users physically next door do not render
  as neighbors.
- Transaction aggregation: burst events, not individual dots
- Engine: Three.js
- Theme: Midnight (deep black, charcoal, neon accents, no borders)

## Visual Identity

- Hubs (Nodes): glowing hexagonal anchors, pulse rate = activity level
- Inference flows: ultra-fast neon-magenta laser arcs
- Storage flows: thicker cyan liquid trails
- Sensor data: amber particle streams

## Interaction

- Globe View (Story Mode): auto-rotate 0.05 RPM, great circle arcs
- Flat View (Audit Mode): top-down, hexagonal heatmap, direct pulse lines
- Live Ledger: translucent sidebar with block hashes and latency pings

## Current Implementation Notes

- `website/globe.html` is the public btcpc.net globe.
- It currently uses Three.js, Natural Earth coastlines, bloom, 3D globe mode,
  2D command-map mode, glowing hex hubs, magenta inference arcs, cyan storage
  trails, amber sensor pulses, and a matrix-style event log.
- Account positions are derived client-side from account names. Known genesis
  accounts can be assigned to broad metro buckets, but the rendered coordinate
  is still sector-spread and not an exact location.
- Continue improving aggregation by replacing random visual events with real
  burst events from `/public/network`, block data, inference jobs, and storage
  proofs as those public endpoints become available.

## Effects

- Exponential decay on light trails
- Bloom post-processing shader for hub glow
- Camera inertia with damping
