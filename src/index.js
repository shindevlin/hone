#!/usr/bin/env node
/**
 * Express Server
 */

require('dotenv').config();
const express = require('express');
const helmet = require('helmet');
const cors = require('cors');
const rateLimit = require('express-rate-limit');
const morgan = require('morgan');
const mongoose = require('mongoose');

const app = express();

// Middleware
app.use(cors());
app.use(helmet());
app.use(express.json());
// Import routes
const userRoutes = require("./routes/userRoutes");
const walletRoutes = require("./routes/walletRoutes");
const stakingRoutes = require("./routes/stakingRoutes");
const nodeRoutes = require("./routes/nodeRoutes");
const delegationRoutes = require("./routes/delegationRoutes");
const inferenceApi = require("./inference/api");
const dreamRoutes = require("./routes/dreamRoutes");
const recoveryRoutes = require("./routes/recoveryRoutes");
app.use("/api/user", userRoutes);
app.use("/api/wallet", walletRoutes);
app.use("/api/staking", stakingRoutes);
app.use("/api/node", nodeRoutes);
app.use("/api/delegation", delegationRoutes);
app.use("/api/recovery", recoveryRoutes);
app.use("/api", dreamRoutes);
app.use(inferenceApi);

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

// Routes
app.get('/health', (req, res) => {
  res.json({ status: 'OK', timestamp: new Date().toISOString() });
});

app.get('/', (req, res) => {
  res.json({
    name: 'URSNode API',
    version: '1.0.0',
    endpoints: [
      '/health',
      '/api/user',
      '/api/wallet',
      '/api/staking',
      '/api/node',
      '/api/dreams/:account',
      '/api/dream/:blockNumber',
      '/api/dream/:blockNumber/inscribe',
      '/api/dream/:blockNumber/transfer',
      '/api/delegation',
      '/api/recovery',
      '/v1/chat/completions',
      '/v1/models'
    ]
  });
});

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
  app.listen(PORT, () => {
    console.log(`URSNode server running on port ${PORT}`);
  });

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
    } catch (err) {
      console.error('[BTCPC] Failed to start P2P network:', err.message);
    }
  }
}).catch(err => {
  console.error('Failed to start server:', err);
});

module.exports = app;
