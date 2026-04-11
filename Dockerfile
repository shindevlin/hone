FROM node:20-alpine

WORKDIR /app

# System deps for native modules + healthchecks
RUN apk add --no-cache python3 make g++ git curl

# Install dependencies first (cached layer)
COPY package.json package-lock.json* ./
RUN npm install --omit=dev

# Copy source
COPY bin ./bin
COPY src ./src
COPY contracts ./contracts
COPY scripts ./scripts
COPY docs ./docs

# Create runtime dirs
RUN mkdir -p /app/data /app/blocks /app/.btcpc-inference

# Expose ports: API (3000), explorer (4242), storage HTTP (4243), P2P (6942), clock P2P (6943)
EXPOSE 3000 4242 4243 6942 6943

# Multi-role supervisor. Override which roles run via BTCPC_ROLES env var:
#   BTCPC_ROLES=all  (default) — api + miner + clock + storage
#   BTCPC_ROLES=miner,clock     — just the earning roles
#   BTCPC_ROLES=api             — HTTP API only
CMD ["node", "bin/btcpc-all"]
