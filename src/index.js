#!/usr/bin/env node
/**
 * Express Server
 */

require('dotenv').config();
const path = require('path');
const fs = require('fs');
const express = require('express');
const helmet = require('helmet');
const cors = require('cors');
const rateLimit = require('express-rate-limit');
const morgan = require('morgan');
const mongoose = require('mongoose');
const { version } = require('../package.json');

// Accept the persisted BTCPC_JWT_SECRET alias written by older self-heal code.
if (!process.env.JWT_SECRET && process.env.BTCPC_JWT_SECRET) {
  process.env.JWT_SECRET = process.env.BTCPC_JWT_SECRET;
}

// Auto-generate JWT_SECRET if not set (self-heal: never ask user to configure)
if (!process.env.JWT_SECRET) {
  const crypto = require('crypto');
  process.env.JWT_SECRET = crypto.randomBytes(32).toString('hex');
  console.log('[BTCPC] JWT_SECRET auto-generated (will change on restart — set BTCPC_JWT_SECRET in .env for persistence)');
  // Try to persist to .env so subsequent restarts keep the same secret
  try {
    const envPath = path.resolve(__dirname, '..', '.env');
    const envLine = '\nBTCPC_JWT_SECRET=' + process.env.JWT_SECRET + '\nJWT_SECRET=' + process.env.JWT_SECRET + '\n';
    fs.appendFileSync(envPath, envLine);
    console.log('[BTCPC] JWT_SECRET saved to .env');
  } catch (_) {
    // Container filesystem may be read-only — that's fine, just volatile
  }
}

// Phase F: MongoDB is optional. BTCPC_MONGO_MODE=disabled skips connection entirely.
let mongoEnabled = false;

// Purchase address startup check
if (!process.env.BTCPC_PURCHASE_ETH_ADDRESS) {
  console.warn('[BTCPC] WARNING: BTCPC_PURCHASE_ETH_ADDRESS not set — ETH purchase route will return 503');
}

const app = express();

// Trust loopback proxies only (website static server runs on 127.0.0.1)
// This is permissive enough for express-rate-limit's strict validation
app.set('trust proxy', 'loopback');

// Middleware — security first
app.use(cors({
  origin: process.env.CORS_ORIGINS ? process.env.CORS_ORIGINS.split(',') : [
    'https://btcpc.net', 'https://scan.btcpc.net', 'https://docs.btcpc.net',
    'http://localhost:4242', 'http://localhost:3000', 'http://localhost:3100',
    'https://localhost',   // Capacitor Android app
    'capacitor://localhost' // Capacitor iOS app
  ],
  credentials: true
}));
app.use(helmet());
app.use(express.json({ limit: '1mb' }));

const apiLimiter = rateLimit({
  windowMs: 60 * 1000, // 1 minute
  max: 100, // 100 requests per minute per IP
  standardHeaders: true,
  legacyHeaders: false,
  message: { error: 'Too many requests, slow down' }
});
app.use('/api/', apiLimiter);

const createLimiter = rateLimit({
  windowMs: 60 * 60 * 1000, // 1 hour
  max: 5, // 5 account creations per hour per IP
  message: { error: 'Account creation rate limit — try again later' }
});
app.use('/api/bot/create', createLimiter);

const onboardLimiter = rateLimit({
  windowMs: 60 * 60 * 1000, // 1 hour
  max: 10, // 10 onboard calls per hour per IP
  message: { error: 'Onboarding rate limit — try again later' }
});
app.use('/api/bot/onboard', onboardLimiter);

// Health routes must be mounted before inference auth middleware.
app.get('/health', (_req, res) => {
  res.json({ status: 'OK', timestamp: new Date().toISOString() });
});

app.get('/', (_req, res) => {
  res.json({
    name: 'BTCPC API',
    version,
    endpoints: [
      '/health',
      '/install.sh',
      '/api/user',
      '/api/wallet',
      '/api/staking',
      '/api/sensor-data/rate-card',
      '/api/sensor-data/quote',
      '/api/sensor-data/query',
      '/api/node',
      '/api/dreams/:account',
      '/api/dream/:blockNumber',
      '/api/dream/:blockNumber/inscribe',
      '/api/dream/:blockNumber/transfer',
      '/api/faucet/claim',
      '/api/projects/register',
      '/api/projects/verify',
      '/api/projects/me',
      '/api/projects/fund',
      '/api/delegation',
      '/api/totp',
      '/api/recovery',
      '/api/appeal',
      '/api/appeal/resolve',
      '/v1/chat/completions',
      '/v1/models'
    ]
  });
});

app.get('/install.sh', (_req, res) => {
  const scriptPath = path.resolve(__dirname, '..', 'scripts', 'install-user.sh');
  if (!fs.existsSync(scriptPath)) {
    return res.status(404).type('text/plain').send('install script not found\n');
  }
  res.type('text/plain');
  res.setHeader('Content-Disposition', 'inline; filename="install.sh"');
  return res.sendFile(scriptPath);
});

app.get('/install-termux.sh', (_req, res) => {
  const scriptPath = path.resolve(__dirname, '..', 'scripts', 'install-termux.sh');
  if (!fs.existsSync(scriptPath)) {
    return res.status(404).type('text/plain').send('install script not found\n');
  }
  res.type('text/plain');
  res.setHeader('Content-Disposition', 'inline; filename="install-termux.sh"');
  return res.sendFile(scriptPath);
});

// Import routes
const userRoutes = require("./routes/userRoutes");
const walletRoutes = require("./routes/walletRoutes");
const stakingRoutes = require("./routes/stakingRoutes");
const nodeRoutes = require("./routes/nodeRoutes");
const delegationRoutes = require("./routes/delegationRoutes");
const inferenceApi = require("./inference/api");
const encryptedInference = require("./inference/encrypted");
const dreamRoutes = require("./routes/dreamRoutes");
const recoveryRoutes = require("./routes/recoveryRoutes");
const faucetRoutes = require("./routes/faucetRoutes");
const projectRoutes = require("./routes/projectRoutes");
const botRoutes = require("./routes/botRoutes");
const totpRoutes = require("./routes/totpRoutes");
const appealRoutes = require("./routes/appealRoutes");
const publicRoutes = require("./routes/publicRoutes");
const sessionRoutes = require("./routes/sessionRoutes");
const commerceRoutes = require("./routes/commerceRoutes");
const amberPillRoutes = require("./routes/amberPillRoutes");
const { sensorsRouter, gatewaysRouter, devicesRouter } = require("./routes/sensorRoutes");
const { router: sensorDataRouter } = require("./routes/sensorDataRoutes");
const explorerRoutes = require("./routes/explorerRoutes");
const toolRoutes = require("./routes/toolRoutes");
const storageRoutes = require("./routes/storageRoutes");
const blobRoutes = require("./routes/blobRoutes");
const modelRoutes = require("./routes/modelRoutes");
const phoneMiningRoutes = require("./routes/phoneMiningRoutes");
const phoneStorageRoutes = require("./routes/phoneStorageRoutes");
const auctionRoutes = require("./routes/auctionRoutes");
const purchaseRoutes = require("./routes/purchaseRoutes");
const inferenceMarketRoutes = require("./routes/inferenceMarketRoutes");
app.use("/api/user", userRoutes);
app.use("/api/wallet", walletRoutes);
app.use("/api/faucet", faucetRoutes);
app.use("/api/projects", projectRoutes);
app.use("/api/staking", stakingRoutes);
app.use("/api/node", nodeRoutes);
app.use("/api/delegation", delegationRoutes);
app.use("/api/recovery", recoveryRoutes);
app.use("/api/totp", totpRoutes);
app.use("/api/appeal", appealRoutes);
app.use("/api/commerce", commerceRoutes);
app.use("/api/amber-pills", amberPillRoutes);
app.use("/api/sensors", sensorsRouter);
app.use("/api/gateways", gatewaysRouter);
app.use("/api/devices", devicesRouter);
app.use("/api/sensor-data", sensorDataRouter);
app.use("/api/storage", storageRoutes);
app.use("/api/blobs", blobRoutes);
app.use("/api/models", modelRoutes);
app.use("/api/auctions", auctionRoutes);
app.use("/api/jobs", inferenceMarketRoutes);
app.use("/api/purchase", purchaseRoutes);
app.use("/api/mining/phone", phoneMiningRoutes);
app.use("/api/storage/phone", phoneStorageRoutes);
app.use("/public", publicRoutes);
app.use("/api/auth", sessionRoutes);
app.use("/api", dreamRoutes);
app.use("/api/onboard", onboardLimiter, (req, res, next) => {
  req.url = "/onboard";
  botRoutes(req, res, next);
});
app.use("/api/bot", botRoutes);
app.use("/api/explorer", explorerRoutes);
app.use(toolRoutes);
app.use(inferenceApi);
app.use(encryptedInference);

app.use(morgan('combined'));

// Epoch Bandwidth error handler — 429 with machine-readable body
app.use((err, req, res, next) => {
  if (err && err.code === 'INSUFFICIENT_EB') {
    return res.status(429).json({
      error: err.message,
      code: 'INSUFFICIENT_EB',
      eb_remaining: err.eb_remaining,
      eb_cost: err.eb_cost,
      max_eb: err.max_eb,
    });
  }
  next(err);
});

// Rate limiting
const limiter = rateLimit({
  windowMs: parseInt(process.env.RATE_LIMIT_WINDOW_MS) || 900000,
  max: parseInt(process.env.RATE_LIMIT_MAX_REQUESTS) || 100,
  message: 'Too many requests from this IP, please try again later.'
});
app.use('/api/', limiter);

// Database connection — Phase F: non-fatal, optional
async function connectDB() {
  const mongoMode = (process.env.BTCPC_MONGO_MODE || '').toLowerCase();
  if (mongoMode === 'disabled') {
    console.log('[BTCPC] MongoDB: DISABLED via BTCPC_MONGO_MODE=disabled');
    return;
  }
  if (!process.env.MONGODB_URI) {
    console.warn('[BTCPC] MongoDB: enabled but unreachable — MONGODB_URI not set, falling back to secretStore + stateStore');
    return;
  }
  try {
    await mongoose.connect(process.env.MONGODB_URI, { serverSelectionTimeoutMS: 3000 });
    mongoEnabled = true;
    console.log('[BTCPC] MongoDB: enabled and connected');
  } catch (err) {
    console.warn('[BTCPC] MongoDB: enabled but unreachable — falling back to secretStore + stateStore (' + err.message + ')');
  }
}

// Error handling middleware
app.use((err, req, res, next) => {
  console.error(err.stack);
  res.status(err.status || 500).json({ error: err.message });
});

// BTCPC Epoch Manager
const { startEpochLoop } = require('./services/epochManager');

// BTCPC P2P Network
const p2pNetwork = require('./p2p/network');
const { loadFromDatabase } = require('./p2p/chainSync');

const PORT = process.env.BTCPC_API_PORT || process.env.PORT || 3000;
connectDB().then(async () => {
  // Phase B: replay blocks into stateStore at startup so shadow reads have data
  try {
    const replay = require('./chain/replay');
    const result = await replay.replayFromDisk({ verbose: true });
    console.log('[BTCPC] stateStore replay: ' + result.replayed + ' blocks, ' +
      result.accounts + ' accounts, height=' + result.chainHeight + ', ' + result.durationMs + 'ms');
  } catch (err) {
    console.error('[BTCPC] stateStore replay error:', err.message);
  }

  // Bootstrap: if BTCPC_ACTIVE_KEY is set and the miner account has no active
  // public key in stateStore yet, inject it directly so block proposal signatures
  // from this node can be verified without waiting for an on-chain KEY_ROTATE.
  if (process.env.BTCPC_ACTIVE_KEY && process.env.BTCPC_MINER) {
    try {
      const stateStore = require('./chain/stateStore');
      const minerAcct = stateStore.getAccount(process.env.BTCPC_MINER);
      if (!minerAcct || !minerAcct.public_keys || !minerAcct.public_keys.active) {
        const { secp256k1 } = require('@noble/curves/secp256k1');
        const pubKeyBytes = secp256k1.getPublicKey(Buffer.from(process.env.BTCPC_ACTIVE_KEY, 'hex'), true);
        const activePubKey = Buffer.from(pubKeyBytes).toString('hex');
        stateStore.bootstrapAccountKey(process.env.BTCPC_MINER, 'active', activePubKey);
        console.log('[BTCPC] Bootstrap: injected active public key for ' + process.env.BTCPC_MINER + ': ' + activePubKey.slice(0, 16) + '...');
      }
    } catch (keyErr) {
      console.warn('[BTCPC] Bootstrap key inject failed:', keyErr.message);
    }
  }

  app.listen(PORT, () => {
    console.log(`BTCPC server running on port ${PORT}`);
  });

  // v2.10.2: optional gateway server for human-viewable commerce pages.
  // Off by default to avoid port-binding surprises; enable via env var.
  if (process.env.BTCPC_GATEWAY_ENABLED === 'true') {
    try {
      const { startGateway } = require('./gateway/server');
      startGateway().catch((err) => {
        console.error('[BTCPC Gateway] failed to start:', err.message);
      });
    } catch (err) {
      console.error('[BTCPC Gateway] module load error:', err.message);
    }
  }

  // Start auto-updater
  const { startAutoUpdater } = require('./services/autoUpdater');
  startAutoUpdater();

  // Start the BTCPC epoch loop after DB is connected
  if (process.env.BTCPC_EPOCH_ENABLED !== 'false') {
    startEpochLoop().catch(err => {
      console.error('[BTCPC] Failed to start epoch loop:', err.message);
    });
  }

  // Start the BTCPC P2P network
  if (process.env.BTCPC_P2P_ENABLED !== 'false') {
    try {
      await loadFromDatabase();
      p2pNetwork.startServer();
      p2pNetwork.connectToSeeds();
      console.log('[BTCPC] P2P network layer started');

      // Initialize fork resolver — self-heals back to main chain if we diverge
      const forkResolver = require('./chain/forkResolver');
      const blockStore = require('./chain/blockStore');
      const stateStore = require('./chain/stateStore');
      const ledger = require('./services/ledger');
      const settlement = require('./chain/rewardSettlement');
      forkResolver.init({ blockStore, stateStore, ledger, settlement });
      forkResolver.on('heal_started', (e) => console.log('[BTCPC ForkResolver] Healing — rolled back to epoch', e.commonAncestor));
      forkResolver.on('heal_complete', (e) => console.log('[BTCPC ForkResolver] Healed — replayed', e.replayed, 'blocks'));
      forkResolver.on('need_full_resync', (e) => console.warn('[BTCPC ForkResolver] Full resync needed from', e.peer, '—', e.reason));
      // Wire clock connectivity into isolation detection
      const clockConsensus = require('./chain/clockConsensus');
      clockConsensus.setPeerSource(() => p2pNetwork.getPeers());
      console.log('[BTCPC] Fork resolver and clock consensus initialized');

      // Initialize P2P inference router (listens for results)
      const { initP2PRouter } = require('./inference/p2pRouter');
      initP2PRouter();

      // Sync local models into the API's model registry so it knows what this node can serve
      const { syncLocalModels } = require('./services/modelRegistry');
      syncLocalModels('local').then(models => {
        if (models.length) console.log(`[BTCPC] API model registry synced: ${models.join(', ')}`);
      }).catch(() => {});
    } catch (err) {
      console.error('[BTCPC] Failed to start P2P network:', err.message);
    }
  }
}).catch(err => {
  console.error('Failed to start server:', err);
});

module.exports = app;
