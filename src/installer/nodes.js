"use strict";

// Node types the installer can set up. Each has a plain-English ask, a
// description of what it does, hardware requirements, and ordered setup steps.

const NODE_TYPES = [
  {
    id: "inference",
    name: "Inference Node",
    ask: "Earn BTCPC by running AI models on this device?",
    explain: [
      "Your device solves AI work requests from the network.",
      "Each completed job earns BTCPC tokens — more compute, more rewards.",
      "Requires: Ollama, 4 GB+ RAM. GPU is optional but earns more.",
    ],
    requires: { ram_gb: 4, ollama: true },
    env_keys: ["BTCPC_MINER", "BTCPC_MODEL"],
    bin: "btcpc-mine",
    systemd: "btcpc-miner",
  },
  {
    id: "clock",
    name: "Clock Node",
    ask: "Keep the network's time honest from this device? (runs on anything — phone, Pi, laptop)",
    explain: [
      "Clock nodes participate in epoch timing consensus.",
      "No GPU, no Ollama — just an internet connection.",
      "Earns a small stream of BTCPC for staying online.",
    ],
    requires: { ram_gb: 0.5, ollama: false },
    env_keys: ["BTCPC_CLOCK_ACCOUNT"],
    bin: "btcpc-clock",
    systemd: "btcpc-clock",
  },
  {
    id: "storage",
    name: "Storage Node",
    ask: "Host files for the network and earn storage rewards?",
    explain: [
      "You store content-addressed blobs for other network users.",
      "Paid per successful delivery — never penalized for downtime.",
      "Requires: disk space (you choose how much to offer).",
    ],
    requires: { disk_gb: 10, ollama: false },
    env_keys: ["BTCPC_STORAGE_DIR", "BTCPC_STORAGE_CAPACITY_GB"],
    bin: "btcpc-storage",
    systemd: "btcpc-storage",
  },
  {
    id: "verifier",
    name: "Verifier Node",
    ask: "Verify other miners' inference work to earn verifier rewards?",
    explain: [
      "You spot-check inference jobs submitted by miners.",
      "Helps the network detect bad actors.",
      "Requires moderate CPU — no GPU needed, but helps.",
    ],
    requires: { ram_gb: 2, ollama: false },
    env_keys: ["BTCPC_MINER"],
    bin: "btcpc-mine",
    systemd: null,
    role_flag: "verifier",
  },
  {
    id: "sensor",
    name: "Sensor Node",
    ask: "Stream real-world data to the chain? (GPS, weather, IoT, GNSS)",
    explain: [
      "Publishes sensor readings as signed chain transactions.",
      "Works with GPS dongles, Meshtastic radios, Flipper Zero, weather stations.",
      "Earns sensor data rewards for valid, verified readings.",
    ],
    requires: { ram_gb: 0.5, ollama: false },
    env_keys: ["BTCPC_SENSOR_TYPE"],
    bin: "btcpc-gnss-bridge",
    systemd: null,
  },
  {
    id: "flipper",
    name: "Flipper Zero Bridge",
    ask: "Auto-sync Flipper Zero sensor readings to the chain when plugged in? (harmless when no Flipper present)",
    explain: [
      "Polls /dev/ttyACM* every 10s and flushes Flipper readings to the chain.",
      "Supports Sub-GHz, IR, NFC and any sensor your Flipper app exports.",
      "Buffers offline readings and drains them when the chain is reachable.",
      "Safe to run on any device — exits cleanly when no Flipper is connected.",
    ],
    requires: { ram_gb: 0.5, ollama: false },
    env_keys: [],
    bin: "btcpc-flipper-listener",
    systemd: "btcpc-flipper",
  },
];

module.exports = { NODE_TYPES };
