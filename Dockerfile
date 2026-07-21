# ── Stage 1: build Rust binaries ─────────────────────────────────────────────
FROM rust:1.80-bookworm AS builder

WORKDIR /build

# Cache dependency layer
COPY rust/hone-node/Cargo.toml rust/hone-node/Cargo.lock* ./rust/hone-node/
COPY rust/hone-node/crates ./rust/hone-node/crates
COPY rust/hone-cli/Cargo.toml rust/hone-cli/Cargo.lock* ./rust/hone-cli/
COPY rust/hone-contract-runtime/Cargo.toml ./rust/hone-contract-runtime/
RUN mkdir -p rust/hone-node/src && echo "fn main(){}" > rust/hone-node/src/main.rs && \
    mkdir -p rust/hone-cli/src && echo "fn main(){}" > rust/hone-cli/src/main.rs && \
    mkdir -p rust/hone-contract-runtime/src && echo "" > rust/hone-contract-runtime/src/lib.rs && \
    cd rust/hone-node && cargo build --release 2>/dev/null; true

# Full source build
COPY rust ./rust
RUN cd rust/hone-node && cargo build --release && \
    cd /build/rust/hone-cli && cargo build --release

# ── Stage 2: minimal runtime image ───────────────────────────────────────────
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    ca-certificates curl libssl3 \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /build/rust/hone-node/target/release/hone-node /usr/local/bin/hone-node
COPY --from=builder /build/rust/hone-cli/target/release/hone      /usr/local/bin/hone

COPY rust/hone-node/genesis.json         /app/genesis.json
COPY rust/hone-node/testnet-genesis.json /app/testnet-genesis.json
COPY website /app/website

RUN mkdir -p /app/data /app/.hone

# API 4242 | explorer 4243 | P2P 6942 | testnet API 4246 | testnet P2P 6946
EXPOSE 4242 4243 6942 4246 6946

ENV HONE_DATA_DIR=/app/data \
    HONE_GENESIS_FILE=/app/genesis.json \
    HONE_API_PORT=4242 \
    HONE_P2P_PORT=6942

CMD ["/usr/local/bin/hone-node"]
