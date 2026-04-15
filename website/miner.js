"use strict";

/**
 * BTCPC Browser Miner -- P2P relay client for phone inference mining
 * Shin Devlin
 *
 * Connects to the BTCPC relay via WebSocket and handles inference jobs
 * using the in-browser WebLLM engine (inference-engine.js).
 */

(function () {
  const RELAY_URL = "wss://btcpc-relay.shindevlin.workers.dev/ws";
  const CLAIM_JITTER_MIN = 200;
  const CLAIM_JITTER_MAX = 800;
  const RECONNECT_BASE_MS = 1000;
  const RECONNECT_MAX_MS = 30000;
  const SEEN_MAX = 1000;
  const JOB_TIMEOUT_MS = 5 * 60 * 1000; // 5 min max per job on phone

  let ws = null;
  let nodeId = null;
  let connected = false;
  let reconnectAttempts = 0;
  let reconnectTimer = null;
  let clockTimer = null;
  let intentionalClose = false;

  // Job state
  let jobInFlight = null;
  const seenRequests = new Set();
  const claimedJobs = new Map();

  // Stats
  const stats = {
    jobsCompleted: 0,
    tokensGenerated: 0,
    totalElapsedMs: 0,
    earnings: 0,
    connected: false,
    lastJobAt: null,
  };

  // Status callback
  let statusCallback = null;

  // ---------------------------------------------------------------------------
  // Helpers
  // ---------------------------------------------------------------------------

  function generateNodeId() {
    const stored = localStorage.getItem("btcpc_node_id");
    if (stored) return stored;
    const id =
      "phone-" +
      Date.now().toString(36) +
      "-" +
      Math.random().toString(36).slice(2, 8);
    localStorage.setItem("btcpc_node_id", id);
    return id;
  }

  function createMessage(type, data) {
    return JSON.stringify({
      id: nodeId + "-" + Date.now() + "-" + Math.random().toString(36).slice(2, 8),
      type: type,
      data: data,
      timestamp: Date.now(),
      nodeId: nodeId,
    });
  }

  function emitStatus(status, detail) {
    if (statusCallback) statusCallback({ status, detail, stats: { ...stats } });
  }

  async function sha256hex(text) {
    const encoded = new TextEncoder().encode(text);
    const buf = await crypto.subtle.digest("SHA-256", encoded);
    return Array.from(new Uint8Array(buf))
      .map((b) => b.toString(16).padStart(2, "0"))
      .join("");
  }

  function trimSeen(set) {
    if (set.size > SEEN_MAX) {
      const first = set.values().next().value;
      set.delete(first);
    }
  }

  // ---------------------------------------------------------------------------
  // WebSocket connection
  // ---------------------------------------------------------------------------

  function connect(opts) {
    opts = opts || {};
    const minerName = opts.minerName || localStorage.getItem("btcpc_miner_name") || "phone-miner";
    const models = opts.models || [];

    if (ws && (ws.readyState === WebSocket.OPEN || ws.readyState === WebSocket.CONNECTING)) {
      console.log("[BTCPC Miner] Already connected or connecting");
      return;
    }

    nodeId = generateNodeId();
    intentionalClose = false;

    try {
      ws = new WebSocket(RELAY_URL);
    } catch (err) {
      console.error("[BTCPC Miner] WebSocket creation failed:", err.message);
      emitStatus("error", err.message);
      scheduleReconnect();
      return;
    }

    ws.onopen = function () {
      connected = true;
      stats.connected = true;
      reconnectAttempts = 0;
      console.log("[BTCPC Miner] Connected to relay");
      emitStatus("connected");

      // Send handshake
      ws.send(
        createMessage("HANDSHAKE", {
          nodeId: nodeId,
          node_name: minerName,
          capabilities: ["inference", "clock", "sensor"],
          models: models,
          version: "phone-1.0",
          platform: "browser",
        })
      );

      // Start clock heartbeat — every 30 seconds while connected
      if (clockTimer) clearInterval(clockTimer);
      clockTimer = setInterval(function () {
        if (!connected) return;
        var epoch = Math.floor((Date.now() - 1776236400000) / 30000);
        if (epoch < 0) epoch = 0;
        send(
          createMessage("CLOCK_HEARTBEAT", {
            account: minerName,
            epoch_number: epoch,
            timestamp: Date.now(),
            source: "phone",
          })
        );
        stats.heartbeats = (stats.heartbeats || 0) + 1;
      }, 30000);
    };

    ws.onmessage = function (event) {
      try {
        const msg = JSON.parse(event.data);
        handleMessage(msg, minerName);
      } catch (err) {
        console.error("[BTCPC Miner] Bad message:", err.message);
      }
    };

    ws.onclose = function () {
      connected = false;
      stats.connected = false;
      if (clockTimer) { clearInterval(clockTimer); clockTimer = null; }
      console.log("[BTCPC Miner] Disconnected from relay");
      emitStatus("disconnected");
      if (!intentionalClose) scheduleReconnect();
    };

    ws.onerror = function (err) {
      console.error("[BTCPC Miner] WebSocket error:", err);
      emitStatus("error", "WebSocket error");
    };
  }

  function disconnect() {
    intentionalClose = true;
    if (reconnectTimer) {
      clearTimeout(reconnectTimer);
      reconnectTimer = null;
    }
    if (ws) {
      ws.close();
      ws = null;
    }
    connected = false;
    stats.connected = false;
    jobInFlight = null;
    emitStatus("disconnected");
  }

  function scheduleReconnect() {
    if (intentionalClose) return;
    const delay = Math.min(
      RECONNECT_BASE_MS * Math.pow(2, reconnectAttempts),
      RECONNECT_MAX_MS
    );
    reconnectAttempts++;
    console.log("[BTCPC Miner] Reconnecting in " + Math.round(delay / 1000) + "s...");
    emitStatus("reconnecting", { delay_ms: delay, attempt: reconnectAttempts });
    reconnectTimer = setTimeout(function () {
      reconnectTimer = null;
      connect();
    }, delay);
  }

  function send(msg) {
    if (ws && ws.readyState === WebSocket.OPEN) {
      ws.send(typeof msg === "string" ? msg : JSON.stringify(msg));
    }
  }

  // ---------------------------------------------------------------------------
  // Message handling
  // ---------------------------------------------------------------------------

  function handleMessage(msg, minerName) {
    if (!msg || !msg.type) return;

    switch (msg.type) {
      case "INFERENCE_REQUEST":
        handleInferenceRequest(msg, minerName);
        break;
      case "INFERENCE_CLAIM":
        handleInferenceClaim(msg);
        break;
      case "INFERENCE_PAYLOAD":
        handlePayload(msg, minerName);
        break;
      case "INFERENCE_RESULT":
      case "INFERENCE_REVEAL":
      case "INFERENCE_COMMIT":
        handleCompletion(msg);
        break;
      case "INFERENCE_RECLAIM":
        handleReclaim(msg);
        break;
    }
  }

  // ---------------------------------------------------------------------------
  // Inference request -> claim flow
  // ---------------------------------------------------------------------------

  async function handleInferenceRequest(msg, minerName) {
    const data = msg.data || msg;
    const reqId = data.request_id;
    if (!reqId || seenRequests.has(reqId)) return;

    seenRequests.add(reqId);
    trimSeen(seenRequests);

    // Already claimed by someone?
    if (claimedJobs.has(reqId)) return;

    // Already busy with a job?
    if (jobInFlight) {
      console.log("[BTCPC Miner] Busy, skipping " + reqId.slice(0, 8));
      return;
    }

    // Check if engine has a model loaded
    if (!window.btcpcEngine) {
      console.log("[BTCPC Miner] No inference engine loaded, skipping");
      return;
    }

    const loadedModels = window.btcpcEngine.getLoadedModels();
    if (!loadedModels || loadedModels.length === 0) {
      console.log("[BTCPC Miner] No model loaded, skipping");
      return;
    }

    // Random jitter before claiming
    const jitter = CLAIM_JITTER_MIN + Math.random() * (CLAIM_JITTER_MAX - CLAIM_JITTER_MIN);
    await new Promise(function (r) { setTimeout(r, jitter); });

    // Re-check after jitter
    if (claimedJobs.has(reqId) || jobInFlight) return;

    // Claim it
    jobInFlight = {
      request_id: reqId,
      started_at: Date.now(),
      model: loadedModels[0],
      prompt: data.prompt || null,
      data: data,
    };

    claimedJobs.set(reqId, { miner: minerName, claimed_at: Date.now() });

    send(
      createMessage("INFERENCE_CLAIM", {
        request_id: reqId,
        node_id: nodeId,
        node_name: minerName,
        model: loadedModels[0],
        price: Math.min(data.max_fee || 10, 5),
      })
    );

    console.log("[BTCPC Miner] Claimed " + reqId.slice(0, 8) + " (jitter: " + Math.round(jitter) + "ms)");
    emitStatus("claimed", { request_id: reqId });

    // If prompt is already in the request (not encrypted), process immediately
    if (data.prompt && !data.encrypted) {
      await processJob(reqId, data.prompt, data, minerName);
    }
    // Otherwise wait for INFERENCE_PAYLOAD
  }

  function handleInferenceClaim(msg) {
    const data = msg.data || {};
    const reqId = data.request_id;
    if (!reqId) return;

    claimedJobs.set(reqId, { miner: data.node_name || "unknown", claimed_at: Date.now() });

    // If someone else claimed our pending job, back off
    if (
      data.node_name &&
      data.node_id !== nodeId &&
      jobInFlight &&
      jobInFlight.request_id === reqId &&
      !jobInFlight.processing
    ) {
      console.log("[BTCPC Miner] " + reqId.slice(0, 8) + " claimed by " + data.node_name + ", backing off");
      jobInFlight = null;
    }
  }

  async function handlePayload(msg, minerName) {
    const data = msg.data || msg;
    const reqId = data.request_id;
    if (!reqId) return;

    // Only process if this is our claimed job
    if (!jobInFlight || jobInFlight.request_id !== reqId) return;

    let prompt = data.prompt;

    // Handle encrypted payloads
    if (data.encrypted && data.prompt_encrypted && window.btcpcCrypto) {
      try {
        const memoPriv = localStorage.getItem("btcpc_memo_priv");
        if (!memoPriv) {
          console.error("[BTCPC Miner] Encrypted job but no memo private key stored");
          clearJob("no memo key");
          return;
        }
        const sharedSecret = await window.btcpcCrypto.computeSharedSecret(
          memoPriv,
          data.user_memo_pubkey
        );
        prompt = await window.btcpcCrypto.decrypt(data.prompt_encrypted, sharedSecret);
        jobInFlight.sharedSecret = sharedSecret;
        console.log("[BTCPC Miner] Decrypted prompt for " + reqId.slice(0, 8));
      } catch (err) {
        console.error("[BTCPC Miner] Decrypt failed for " + reqId.slice(0, 8) + ": " + err.message);
        clearJob("decrypt failed");
        return;
      }
    }

    if (!prompt) {
      console.log("[BTCPC Miner] No prompt in payload for " + reqId.slice(0, 8));
      clearJob("no prompt");
      return;
    }

    await processJob(reqId, prompt, data, minerName);
  }

  // ---------------------------------------------------------------------------
  // Run inference and broadcast results
  // ---------------------------------------------------------------------------

  async function processJob(reqId, prompt, data, minerName) {
    if (!jobInFlight || jobInFlight.request_id !== reqId) return;
    jobInFlight.processing = true;

    emitStatus("processing", { request_id: reqId, prompt_length: prompt.length });
    console.log("[BTCPC Miner] Processing " + reqId.slice(0, 8) + " (" + prompt.length + " chars)");

    // Timeout guard
    const timeoutId = setTimeout(function () {
      if (jobInFlight && jobInFlight.request_id === reqId) {
        console.log("[BTCPC Miner] Job " + reqId.slice(0, 8) + " timed out");
        clearJob("timeout");
      }
    }, JOB_TIMEOUT_MS);

    try {
      const result = await window.btcpcEngine.runInference(prompt, {
        max_tokens: data.max_tokens || 1024,
        temperature: data.temperature || 0.7,
      });

      clearTimeout(timeoutId);

      if (!jobInFlight || jobInFlight.request_id !== reqId) return; // cleared while running

      const resultText = result.text || "";
      const tokensGenerated = result.tokens || Math.ceil(resultText.length / 4);
      const elapsed = result.elapsed_ms || 0;

      const resultHash = await sha256hex(resultText);
      const promptHash = await sha256hex(prompt);

      // Broadcast INFERENCE_COMMIT
      send(
        createMessage("INFERENCE_COMMIT", {
          request_id: reqId,
          node_id: nodeId,
          node_name: minerName,
          result_hash: resultHash,
          tokens_generated: tokensGenerated,
        })
      );

      // Broadcast INFERENCE_REVEAL
      send(
        createMessage("INFERENCE_REVEAL", {
          request_id: reqId,
          node_id: nodeId,
          node_name: minerName,
          result_hash: resultHash,
          result_text: resultText,
          tokens_generated: tokensGenerated,
          model: jobInFlight.model,
          elapsed_ms: elapsed,
          work_proof: { prompt_hash: promptHash, result_hash: resultHash },
        })
      );

      // Broadcast INFERENCE_RESULT (encrypt if needed)
      let resultEncrypted = null;
      const sharedSecret = jobInFlight.sharedSecret;
      if (sharedSecret && window.btcpcCrypto) {
        try {
          resultEncrypted = await window.btcpcCrypto.encrypt(resultText, sharedSecret);
        } catch (encErr) {
          console.error("[BTCPC Miner] Failed to encrypt result: " + encErr.message);
        }
      }

      send(
        createMessage("INFERENCE_RESULT", {
          request_id: reqId,
          result_text: sharedSecret ? undefined : resultText,
          result_encrypted: resultEncrypted || undefined,
          encrypted: sharedSecret ? true : undefined,
          result_hash: resultHash,
          tokens_generated: tokensGenerated,
          model: jobInFlight.model,
          elapsed_ms: elapsed,
          node_name: minerName,
          platform: "browser",
        })
      );

      // Update stats
      stats.jobsCompleted++;
      stats.tokensGenerated += tokensGenerated;
      stats.totalElapsedMs += elapsed;
      stats.lastJobAt = new Date().toISOString();

      console.log(
        "[BTCPC Miner] Completed " +
          reqId.slice(0, 8) +
          ": " +
          tokensGenerated +
          " tokens, " +
          elapsed +
          "ms"
      );
      emitStatus("completed", {
        request_id: reqId,
        tokens: tokensGenerated,
        elapsed_ms: elapsed,
      });
    } catch (err) {
      clearTimeout(timeoutId);
      console.error("[BTCPC Miner] Failed " + reqId.slice(0, 8) + ": " + err.message);

      send(
        createMessage("INFERENCE_RESULT", {
          request_id: reqId,
          error: err.message,
          node_name: minerName,
          platform: "browser",
        })
      );

      emitStatus("error", { request_id: reqId, error: err.message });
    } finally {
      clearJob("processing complete");
    }
  }

  // ---------------------------------------------------------------------------
  // Completion / reclaim
  // ---------------------------------------------------------------------------

  function handleCompletion(msg) {
    const data = msg.data || {};
    const reqId = data.request_id;
    if (!reqId) return;
    claimedJobs.delete(reqId);
    if (jobInFlight && jobInFlight.request_id === reqId && !jobInFlight.processing) {
      clearJob("completion broadcast");
    }
  }

  function handleReclaim(msg) {
    const data = msg.data || {};
    const reqId = data.request_id;
    if (!reqId) return;
    claimedJobs.delete(reqId);
    seenRequests.delete(reqId);
    if (jobInFlight && jobInFlight.request_id === reqId) {
      clearJob("reclaimed by network");
    }
  }

  function clearJob(reason) {
    if (jobInFlight) {
      console.log("[BTCPC Miner] Job " + (jobInFlight.request_id || "").slice(0, 8) + " cleared (" + reason + ")");
      jobInFlight = null;
    }
  }

  // ---------------------------------------------------------------------------
  // Public API
  // ---------------------------------------------------------------------------

  window.btcpcMiner = {
    connect: connect,
    disconnect: disconnect,
    isConnected: function () {
      return connected;
    },
    getStats: function () {
      return { ...stats };
    },
    onStatusChange: function (cb) {
      statusCallback = cb;
    },
  };

  console.log("[BTCPC Miner] Browser P2P miner ready");
})();
