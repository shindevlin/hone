"use strict";

/**
 * Global Jest setup.
 *
 * MongoDB is optional and disabled by default (see CLAUDE.md). When a test
 * exercises code that calls a Mongoose model without a live connection,
 * Mongoose *buffers* the operation and waits `bufferTimeoutMS` (default
 * 10_000ms, but 30_000 in some op paths) before rejecting — turning a
 * should-be-instant no-op into a multi-second hang that blows the Jest
 * timeout, especially under parallel load.
 *
 * Disable command buffering globally so any model call without a connection
 * fails fast. Code that genuinely needs Mongo guards on connection state
 * already (HONE_MONGO_MODE); code that doesn't will catch the fast rejection
 * exactly as it would catch a buffer timeout — just in milliseconds, not 30s.
 */
try {
  const mongoose = require("mongoose");
  mongoose.set("bufferCommands", false);
  mongoose.set("bufferTimeoutMS", 1000);
} catch (_) {
  // mongoose not installed in this environment — nothing to configure.
}
