"use strict";

/**
 * HONE Agent Events
 * Shin Devlin
 *
 * Lightweight EventEmitter bridge between the P2P TOOL_CALL handler and the
 * REST API streaming response. When a miner emits a tool_call via P2P, the
 * event lands here so the /v1/agent/turn SSE stream can surface it to the client.
 */

const EventEmitter = require("events");

const agentEvents = new EventEmitter();
agentEvents.setMaxListeners(200); // one listener per active session turn

module.exports = agentEvents;
