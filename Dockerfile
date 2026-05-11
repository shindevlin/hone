# ── Stage 1: build Rust binaries ─────────────────────────────────────────────
FROM rust:1.80-bookworm AS builder

WORKDIR /build

# Cache dependency layer
COPY rust/btcpc-node/Cargo.toml rust/btcpc-node/Cargo.lock* ./rust/btcpc-node/
COPY rust/btcpc-node/crates ./rust/btcpc-node/crates
COPY rust/btcpc-cli/Cargo.toml rust/btcpc-cli/Cargo.lock* ./rust/btcpc-cli/
COPY rust/btcpc-contract-runtime/Cargo.toml ./rust/btcpc-contract-runtime/
RUN mkdir -p rust/btcpc-node/src && echo "fn main(){}" > rust/btcpc-node/src/main.rs && \
    mkdir -p rust/btcpc-cli/src && echo "fn main(){}" > rust/btcpc-cli/src/main.rs && \
    mkdir -p rust/btcpc-contract-runtime/src && echo "" > rust/btcpc-contract-runtime/src/lib.rs && \
    cd rust/btcpc-node && cargo build --release 2>/dev/null; true

# Full source build
COPY rust ./rust
RUN cd rust/btcpc-node && cargo build --release && \
    cd /build/rust/btcpc-cli && cargo build --release

# ── Stage 2: minimal runtime image ───────────────────────────────────────────
FROM debian:bookworm-slim

RUN apt-get update && apt-get install -y \
    ca-certificates curl libssl3 \
    && rm -rf /var/lib/apt/lists/*

WORKDIR /app

COPY --from=builder /build/rust/btcpc-node/target/release/btcpc-node /usr/local/bin/btcpc-node
COPY --from=builder /build/rust/btcpc-cli/target/release/btcpc      /usr/local/bin/btcpc

COPY rust/btcpc-node/genesis.json         /app/genesis.json
COPY rust/btcpc-node/testnet-genesis.json /app/testnet-genesis.json
COPY website /app/website

RUN mkdir -p /app/data /app/.btcpc

# API 4242 | explorer 4243 | P2P 6942 | testnet API 4246 | testnet P2P 6946
EXPOSE 4242 4243 6942 4246 6946

ENV BTCPC_DATA_DIR=/app/data \
    BTCPC_GENESIS_FILE=/app/genesis.json \
    BTCPC_API_PORT=4242 \
    BTCPC_P2P_PORT=6942

CMD ["/usr/local/bin/btcpc-node"]
