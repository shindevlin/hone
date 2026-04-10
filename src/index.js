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

// Fail-closed: require critical env vars at startup
if (!process.env.JWT_SECRET) { console.error('FATAL: JWT_SECRET not set'); process.exit(1); }
if (!process.env.MONGODB_URI) { console.error('FATAL: MONGODB_URI not set'); process.exit(1); }

const app = express();

// Trust loopback proxies only (website static server runs on 127.0.0.1)
// This is permissive enough for express-rate-limit's strict validation
app.set('trust proxy', 'loopback');

// Middleware — security first
app.use(cors({
  origin: process.env.CORS_ORIGINS ? process.env.CORS_ORIGINS.split(',') : ['https://btcpc.net', 'https://scan.btcpc.net', 'https://docs.btcpc.net', 'http://localhost:4242', 'http://localhost:3100'],
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
const commerceRoutes = require("./routes/commerceRoutes");
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
app.use("/public", publicRoutes);
app.use("/api", dreamRoutes);
app.use("/api/onboard", onboardLimiter, (req, res, next) => {
  req.url = "/onboard";
  botRoutes(req, res, next);
});
app.use("/api/bot", botRoutes);
app.use(inferenceApi);
app.use(encryptedInference);

app.use(morgan('combined'));

// Rate limiting
const limiter = rateLimit({
  windowMs: parseInt(process.env.RATE_LIMIT_WINDOW_MS) || 900000,
  max: parseInt(process.env.RATE_LIMIT_MAX_REQUESTS) || 100,
  message: 'Too many requests from this IP, please try again later.'
});
app.use('/api/', limiter);

// Database connection
async function connectDB() {
  try {
    await mongoose.connect(process.env.MONGODB_URI);
    console.log('MongoDB connected successfully');
  } catch (err) {
    console.error('MongoDB connection error:', err.message);
    process.exit(1);
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

const PORT = process.env.PORT || 3000;
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

  app.listen(PORT, () => {
    console.log(`BTCPC server running on port ${PORT}`);
  });

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

      // Initialize P2P inference router (listens for results)
      const { initP2PRouter } = require('./inference/p2pRouter');
      initP2PRouter();
    } catch (err) {
      console.error('[BTCPC] Failed to start P2P network:', err.message);
    }
  }
}).catch(err => {
  console.error('Failed to start server:', err);
});

module.exports = app;
