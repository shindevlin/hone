"use strict";

/**
 * BTCPC Inference Engine (Browser) -- WebLLM wrapper
 * Shin Devlin
 *
 * Runs LLM inference directly in the browser using WebGPU + WebLLM.
 * Downloads models to IndexedDB and runs them on the device GPU.
 */

(function () {
  const WEBLLM_CDN = "https://cdn.jsdelivr.net/npm/@mlc-ai/web-llm@0.2.73/+esm";

  // Default model: small enough for phones, good quality
  const DEFAULT_MODEL = "Qwen2.5-0.5B-Instruct-q4f16_1-MLC";

  let webllm = null;
  let engine = null;
  let loadedModel = null;
  let loading = false;
  let downloadProgress = 0;
  let progressCallback = null;

  // ---------------------------------------------------------------------------
  // WebGPU check
  // ---------------------------------------------------------------------------

  function isWebGPUSupported() {
    return !!(navigator.gpu);
  }

  // ---------------------------------------------------------------------------
  // Engine init
  // ---------------------------------------------------------------------------

  /**
   * Initialize the WebLLM engine with a given model.
   * Downloads the model to IndexedDB if not cached, then creates the WebGPU pipeline.
   *
   * @param {string} [modelId] - WebLLM model identifier (default: Qwen2.5-0.5B)
   * @returns {Promise<void>}
   */
  async function initEngine(modelId) {
    modelId = modelId || DEFAULT_MODEL;

    if (loading) {
      throw new Error("Model is already loading");
    }

    if (engine && loadedModel === modelId) {
      console.log("[BTCPC Engine] Model " + modelId + " already loaded");
      return;
    }

    if (!isWebGPUSupported()) {
      throw new Error("WebGPU is not supported in this browser. Try Chrome 113+ or Edge 113+.");
    }

    loading = true;
    downloadProgress = 0;

    try {
      // Lazy-load WebLLM from CDN
      if (!webllm) {
        console.log("[BTCPC Engine] Loading WebLLM runtime...");
        emitProgress("loading_runtime", 0);
        webllm = await import(WEBLLM_CDN);
        console.log("[BTCPC Engine] WebLLM runtime loaded");
      }

      // Destroy previous engine if switching models
      if (engine) {
        try {
          await engine.unload();
        } catch (_) {
          // ignore
        }
        engine = null;
        loadedModel = null;
      }

      console.log("[BTCPC Engine] Initializing model: " + modelId);
      emitProgress("downloading", 0);

      // Create engine with progress callback
      engine = await webllm.CreateMLCEngine(modelId, {
        initProgressCallback: function (report) {
          // report: { progress: 0-1, text: string, timeElapsed: number }
          downloadProgress = Math.round((report.progress || 0) * 100);
          const phase = report.progress < 1 ? "downloading" : "ready";
          emitProgress(phase, downloadProgress, report.text);

          if (report.progress >= 1) {
            console.log("[BTCPC Engine] Model ready: " + modelId);
          }
        },
      });

      loadedModel = modelId;
      downloadProgress = 100;
      emitProgress("ready", 100);
      console.log("[BTCPC Engine] Engine initialized with " + modelId);
    } catch (err) {
      console.error("[BTCPC Engine] Init failed:", err.message || err);
      emitProgress("error", 0, err.message || String(err));
      throw err;
    } finally {
      loading = false;
    }
  }

  // ---------------------------------------------------------------------------
  // Run inference
  // ---------------------------------------------------------------------------

  /**
   * Run a chat completion on the loaded model.
   *
   * @param {string} prompt - User prompt text
   * @param {object} [opts] - Options: max_tokens, temperature
   * @returns {Promise<{text: string, tokens: number, elapsed_ms: number}>}
   */
  async function runInference(prompt, opts) {
    if (!engine || !loadedModel) {
      throw new Error("No model loaded. Call initEngine() first.");
    }

    opts = opts || {};
    const maxTokens = opts.max_tokens || 1024;
    const temperature = opts.temperature || 0.7;

    const startTime = performance.now();

    // Build messages array
    const messages = [];
    // Check for System: prefix in prompt (matches server handler)
    const systemMatch = prompt.match(/^System:\s*([\s\S]*?)(?=\n\n|$)/);
    if (systemMatch) {
      messages.push({ role: "system", content: systemMatch[1].trim() });
      const userContent = prompt.slice(systemMatch[0].length).trim();
      if (userContent) messages.push({ role: "user", content: userContent });
    } else {
      messages.push({ role: "user", content: prompt });
    }

    const response = await engine.chat.completions.create({
      messages: messages,
      max_tokens: maxTokens,
      temperature: temperature,
      stream: false,
    });

    const elapsed = Math.round(performance.now() - startTime);

    const choice = response.choices && response.choices[0];
    const text = choice ? (choice.message ? choice.message.content : "") : "";
    const tokens =
      response.usage && response.usage.completion_tokens
        ? response.usage.completion_tokens
        : Math.ceil(text.length / 4);

    return {
      text: text,
      tokens: tokens,
      elapsed_ms: elapsed,
    };
  }

  // ---------------------------------------------------------------------------
  // Model info
  // ---------------------------------------------------------------------------

  /**
   * Get list of currently loaded models.
   * @returns {string[]}
   */
  function getLoadedModels() {
    if (loadedModel) return [loadedModel];
    return [];
  }

  /**
   * Get current download progress (0-100).
   * @returns {number}
   */
  function getDownloadProgress() {
    return downloadProgress;
  }

  // ---------------------------------------------------------------------------
  // Progress events
  // ---------------------------------------------------------------------------

  function emitProgress(phase, percent, detail) {
    if (progressCallback) {
      progressCallback({
        phase: phase,
        percent: percent || 0,
        detail: detail || "",
        model: loadedModel || DEFAULT_MODEL,
      });
    }
  }

  // ---------------------------------------------------------------------------
  // Public API
  // ---------------------------------------------------------------------------

  window.btcpcEngine = {
    initEngine: initEngine,
    runInference: runInference,
    isWebGPUSupported: isWebGPUSupported,
    getLoadedModels: getLoadedModels,
    getDownloadProgress: getDownloadProgress,
    onProgress: function (cb) {
      progressCallback = cb;
    },
  };

  console.log("[BTCPC Engine] Browser inference engine ready (WebGPU: " + isWebGPUSupported() + ")");
})();
