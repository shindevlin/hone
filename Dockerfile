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

# Expose ports: API, explorer, P2P
EXPOSE 3000 4242 6942

# Default command runs the miner; override with `command:` in compose to run a clock node
CMD ["node", "bin/btcpc-mine"]
