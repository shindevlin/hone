# The Global Nervous System — Visualizer Spec

Real-time, interactive bridge between raw blockchain data and cinematic UX.
Visualizes storage and inference across the decentralized network.

## Architecture

- WebGL-based geospatial visualizer
- 3D Cinematic Globe + 2D Command Map toggle
- Coordinate fuzzing: ±50km jitter for privacy
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

## Effects

- Exponential decay on light trails
- Bloom post-processing shader for hub glow
- Camera inertia with damping
