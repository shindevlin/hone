"use strict";

/**
 * Tests for BTCPC stablecoin purchase system.
 * Covers quote math, fulfillment logic, deduplication, validation, order flow, and status.
 * Shin Devlin
 */

const fs = require("fs");
const path = require("path");
const os = require("os");
const crypto = require("crypto");

// ── Helpers ──────────────────────────────────────────────────────────────────

function tmpDir() {
  return fs.mkdtempSync(path.join(os.tmpdir(), "btcpc-purchase-test-"));
}

function readJsonl(file) {
  if (!fs.existsSync(file)) return [];
  return fs.readFileSync(file, "utf8")
    .split("\n").filter(Boolean)
    .map(l => { try { return JSON.parse(l); } catch { return null; } })
    .filter(Boolean);
}

function appendJsonl(file, rec) {
  fs.appendFileSync(file, JSON.stringify(rec) + "\n");
}

// ── Extracted logic (mirrors purchaseRoutes.js / btcpc-purchase-watcher) ─────

const BTCPC_PRICE_USD = 0.001;
const MIN_USD = 10;
const MAX_USD = 10000;

const ETH_CONTRACTS = {
  USDC: "0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48",
  USDT: "0xdAC17F958D2ee523a2206206994597C13D831ec7",
  DAI:  "0x6B175474E89094C44Da98b954EedeAC495271d0F",
};
const SOL_MINTS = { USDC: "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v" };
const SUPPORTED_CHAINS = ["ethereum", "solana", "ton"];
const SUPPORTED_TOKENS = { ethereum: ["USDC","USDT","DAI"], solana: ["USDC"], ton: ["USDC"] };

function calcBtcpc(usdAmount, price) {
  return usdAmount / price;
}

function validateAmount(amount) {
  if (isNaN(amount) || amount < MIN_USD) return `Minimum purchase is $${MIN_USD} USD`;
  if (amount > MAX_USD) return `Maximum purchase is $${MAX_USD} USD per transaction`;
  return null;
}

function isAlreadyProcessed(historyFile, txHash) {
  return readJsonl(historyFile).some(h => h.tx_hash === txHash);
}

function findOrderInFile(ordersFile, purchaseId) {
  return readJsonl(ordersFile).find(o => o.purchase_id === purchaseId) || null;
}

function getStatusFromFiles(ordersFile, historyFile, purchaseId) {
  const history = readJsonl(historyFile);
  const fulfilled = history.find(h => h.purchase_id === purchaseId);
  if (fulfilled) {
    return { status: "fulfilled", btcpc_amount: fulfilled.btcpc_amount, tx_hash: fulfilled.tx_hash, fulfilled_epoch: fulfilled.fulfilled_epoch };
  }
  const order = readJsonl(ordersFile).find(o => o.purchase_id === purchaseId);
  if (!order) return null;
  if (order.expires_at && Date.now() > order.expires_at && order.status === "pending") {
    return { status: "expired" };
  }
  return { status: order.status || "pending" };
}

function matchEthOrder(orders, token, usdAmount, paymentTs) {
  const THIRTY_MIN = 30 * 60 * 1000;
  return orders.find(o =>
    o.chain === "ethereum" &&
    o.token === token &&
    Math.abs(o.usd_amount - usdAmount) <= 0.01 &&
    paymentTs - o.created_at < THIRTY_MIN &&
    paymentTs >= o.created_at
  ) || null;
}

function matchMemoOrder(orders, chain, memoOrComment) {
  if (!memoOrComment) return null;
  return orders.find(o =>
    o.chain === chain &&
    memoOrComment.includes(o.purchase_id)
  ) || null;
}

function isLocalhostAddr(addr) {
  return addr === "127.0.0.1" || addr === "::1" || addr === "::ffff:127.0.0.1";
}

function buildQuoteResponse(usdAmount, chain, token) {
  const price = BTCPC_PRICE_USD;
  const addrMap = { ethereum: "0xEthAddr", solana: "SolAddr", ton: "TonAddr" };
  const contractMap = {
    ethereum: { USDC: ETH_CONTRACTS.USDC, USDT: ETH_CONTRACTS.USDT, DAI: ETH_CONTRACTS.DAI },
    solana: { USDC: SOL_MINTS.USDC },
    ton: { USDC: "native" },
  };
  return {
    btcpc_amount: usdAmount / price,
    price_per_btcpc_usd: price,
    payment_address: addrMap[chain] || null,
    token_contract: (contractMap[chain] || {})[token] || null,
    min_usd: MIN_USD,
    max_usd: MAX_USD,
  };
}

// ── Tests ─────────────────────────────────────────────────────────────────────

describe("Quote calculation", () => {
  test("calculates correct BTCPC at $0.001/BTCPC", () => {
    expect(calcBtcpc(100, 0.001)).toBe(100000);
    expect(calcBtcpc(10, 0.001)).toBe(10000);
    expect(calcBtcpc(10000, 0.001)).toBe(10000000);
  });

  test("calculates correctly at non-default price", () => {
    expect(calcBtcpc(50, 0.005)).toBe(10000);
    expect(calcBtcpc(1000, 0.01)).toBe(100000);
  });

  test("quote response has all required fields", () => {
    const q = buildQuoteResponse(100, "ethereum", "USDC");
    expect(q).toHaveProperty("btcpc_amount", 100000);
    expect(q).toHaveProperty("price_per_btcpc_usd", 0.001);
    expect(q).toHaveProperty("payment_address");
    expect(q).toHaveProperty("token_contract");
    expect(q).toHaveProperty("min_usd", 10);
    expect(q).toHaveProperty("max_usd", 10000);
  });

  test("ETH contract addresses are correct", () => {
    expect(ETH_CONTRACTS.USDC).toBe("0xA0b86991c6218b36c1d19D4a2e9Eb0cE3606eB48");
    expect(ETH_CONTRACTS.USDT).toBe("0xdAC17F958D2ee523a2206206994597C13D831ec7");
    expect(ETH_CONTRACTS.DAI).toBe("0x6B175474E89094C44Da98b954EedeAC495271d0F");
  });

  test("Solana USDC mint is correct", () => {
    expect(SOL_MINTS.USDC).toBe("EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v");
  });
});

describe("Amount validation", () => {
  test("rejects amounts below minimum", () => {
    expect(validateAmount(5)).toMatch(/Minimum/);
    expect(validateAmount(0)).toMatch(/Minimum/);
  });

  test("rejects amounts above maximum", () => {
    expect(validateAmount(15000)).toMatch(/Maximum/);
    expect(validateAmount(99999)).toMatch(/Maximum/);
  });

  test("accepts boundary minimum ($10)", () => {
    expect(validateAmount(10)).toBeNull();
  });

  test("accepts boundary maximum ($10000)", () => {
    expect(validateAmount(10000)).toBeNull();
  });

  test("accepts mid-range amount", () => {
    expect(validateAmount(500)).toBeNull();
  });
});

describe("Order file persistence", () => {
  let dir, ordersFile;

  beforeEach(() => {
    dir = tmpDir();
    ordersFile = path.join(dir, "purchase-orders.jsonl");
  });

  afterEach(() => { fs.rmSync(dir, { recursive: true, force: true }); });

  test("writes purchase order to jsonl", () => {
    const order = { purchase_id: "abc123", btcpc_account: "testuser", usd_amount: 100, status: "pending", created_at: Date.now() };
    appendJsonl(ordersFile, order);
    const orders = readJsonl(ordersFile);
    expect(orders).toHaveLength(1);
    expect(orders[0].purchase_id).toBe("abc123");
    expect(orders[0].btcpc_account).toBe("testuser");
  });

  test("multiple orders are written and readable", () => {
    for (let i = 0; i < 3; i++) {
      appendJsonl(ordersFile, { purchase_id: `id-${i}`, usd_amount: 100 + i, status: "pending" });
    }
    const orders = readJsonl(ordersFile);
    expect(orders).toHaveLength(3);
    expect(orders[2].purchase_id).toBe("id-2");
  });

  test("findOrderInFile returns correct order", () => {
    appendJsonl(ordersFile, { purchase_id: "find-me", usd_amount: 200, status: "pending" });
    appendJsonl(ordersFile, { purchase_id: "other", usd_amount: 50, status: "pending" });
    const found = findOrderInFile(ordersFile, "find-me");
    expect(found).not.toBeNull();
    expect(found.usd_amount).toBe(200);
  });

  test("findOrderInFile returns null for unknown id", () => {
    appendJsonl(ordersFile, { purchase_id: "known", usd_amount: 100, status: "pending" });
    expect(findOrderInFile(ordersFile, "unknown")).toBeNull();
  });
});

describe("Fulfillment deduplication", () => {
  let dir, historyFile;

  beforeEach(() => {
    dir = tmpDir();
    historyFile = path.join(dir, "purchase-history.jsonl");
  });

  afterEach(() => { fs.rmSync(dir, { recursive: true, force: true }); });

  test("same tx_hash not processed twice", () => {
    const txHash = "0xabc123def456";
    appendJsonl(historyFile, { tx_hash: txHash, purchase_id: "pid1", btcpc_amount: 100000 });
    expect(isAlreadyProcessed(historyFile, txHash)).toBe(true);
  });

  test("different tx_hash is not a duplicate", () => {
    appendJsonl(historyFile, { tx_hash: "0xone", purchase_id: "pid1" });
    expect(isAlreadyProcessed(historyFile, "0xtwo")).toBe(false);
  });

  test("returns false when history file is empty", () => {
    expect(isAlreadyProcessed(historyFile, "0xanything")).toBe(false);
  });

  test("multiple entries — finds correct duplicate", () => {
    for (let i = 0; i < 5; i++) {
      appendJsonl(historyFile, { tx_hash: `0x${i}00`, purchase_id: `pid-${i}` });
    }
    expect(isAlreadyProcessed(historyFile, "0x300")).toBe(true);
    expect(isAlreadyProcessed(historyFile, "0x999")).toBe(false);
  });
});

describe("Status endpoint logic", () => {
  let dir, ordersFile, historyFile;

  beforeEach(() => {
    dir = tmpDir();
    ordersFile = path.join(dir, "orders.jsonl");
    historyFile = path.join(dir, "history.jsonl");
  });

  afterEach(() => { fs.rmSync(dir, { recursive: true, force: true }); });

  test("returns pending for new order", () => {
    const pid = "pending-pid-1";
    appendJsonl(ordersFile, { purchase_id: pid, status: "pending", created_at: Date.now(), expires_at: Date.now() + 60000 });
    const result = getStatusFromFiles(ordersFile, historyFile, pid);
    expect(result.status).toBe("pending");
  });

  test("returns fulfilled when history record exists", () => {
    const pid = "fulfilled-pid-1";
    appendJsonl(ordersFile, { purchase_id: pid, status: "pending", created_at: Date.now() });
    appendJsonl(historyFile, { purchase_id: pid, tx_hash: "0xfulfilled", btcpc_amount: 100000, fulfilled_epoch: 42 });
    const result = getStatusFromFiles(ordersFile, historyFile, pid);
    expect(result.status).toBe("fulfilled");
    expect(result.btcpc_amount).toBe(100000);
    expect(result.tx_hash).toBe("0xfulfilled");
  });

  test("returns null for unknown purchase_id", () => {
    expect(getStatusFromFiles(ordersFile, historyFile, "does-not-exist")).toBeNull();
  });

  test("returns expired for past-deadline order", () => {
    const pid = "expired-pid-1";
    appendJsonl(ordersFile, { purchase_id: pid, status: "pending", created_at: Date.now() - 3600000, expires_at: Date.now() - 1000 });
    const result = getStatusFromFiles(ordersFile, historyFile, pid);
    expect(result.status).toBe("expired");
  });
});

describe("Order matching — Ethereum (amount + time window)", () => {
  const now = Date.now();

  const orders = [
    { purchase_id: "eth-1", chain: "ethereum", token: "USDC", usd_amount: 100, status: "pending", created_at: now - 5 * 60 * 1000 },
    { purchase_id: "eth-dai", chain: "ethereum", token: "DAI",  usd_amount: 250, status: "pending", created_at: now - 10 * 60 * 1000 },
  ];

  test("matches by exact amount", () => {
    const m = matchEthOrder(orders, "USDC", 100.00, now);
    expect(m).not.toBeNull();
    expect(m.purchase_id).toBe("eth-1");
  });

  test("matches within $0.01 float tolerance", () => {
    const m = matchEthOrder(orders, "USDC", 100.005, now);
    expect(m).not.toBeNull();
  });

  test("does not match outside 30-minute window", () => {
    const latePayment = now + 35 * 60 * 1000;
    const m = matchEthOrder(orders, "USDC", 100.00, latePayment);
    expect(m).toBeNull();
  });

  test("does not match wrong token", () => {
    const m = matchEthOrder(orders, "USDT", 100.00, now);
    expect(m).toBeNull();
  });

  test("matches DAI order by token + amount", () => {
    const m = matchEthOrder(orders, "DAI", 250.00, now);
    expect(m).not.toBeNull();
    expect(m.purchase_id).toBe("eth-dai");
  });
});

describe("Order matching — Solana/TON (memo/comment)", () => {
  const orders = [
    { purchase_id: "sol-abc123", chain: "solana", token: "USDC", status: "pending" },
    { purchase_id: "ton-def456", chain: "ton", token: "USDC", status: "pending" },
  ];

  test("SOL: matches by exact purchase_id in memo", () => {
    const m = matchMemoOrder(orders, "solana", "sol-abc123");
    expect(m).not.toBeNull();
    expect(m.purchase_id).toBe("sol-abc123");
  });

  test("SOL: matches when purchase_id is embedded in longer memo", () => {
    const m = matchMemoOrder(orders, "solana", "Payment ref sol-abc123 done");
    expect(m).not.toBeNull();
  });

  test("TON: matches by comment", () => {
    const m = matchMemoOrder(orders, "ton", "ton-def456");
    expect(m).not.toBeNull();
    expect(m.purchase_id).toBe("ton-def456");
  });

  test("returns null for empty memo", () => {
    expect(matchMemoOrder(orders, "solana", null)).toBeNull();
    expect(matchMemoOrder(orders, "solana", "")).toBeNull();
  });

  test("returns null when memo has no matching purchase_id", () => {
    expect(matchMemoOrder(orders, "solana", "completely-random-memo")).toBeNull();
  });
});

describe("Localhost-only guard", () => {
  test("allows 127.0.0.1", () => {
    expect(isLocalhostAddr("127.0.0.1")).toBe(true);
  });

  test("allows ::1", () => {
    expect(isLocalhostAddr("::1")).toBe(true);
  });

  test("allows ::ffff:127.0.0.1", () => {
    expect(isLocalhostAddr("::ffff:127.0.0.1")).toBe(true);
  });

  test("rejects LAN address", () => {
    expect(isLocalhostAddr("192.168.1.100")).toBe(false);
  });

  test("rejects public IP", () => {
    expect(isLocalhostAddr("104.21.5.45")).toBe(false);
  });
});

describe("Supported chains and tokens", () => {
  test("ethereum supports USDC, USDT, DAI", () => {
    expect(SUPPORTED_TOKENS.ethereum).toContain("USDC");
    expect(SUPPORTED_TOKENS.ethereum).toContain("USDT");
    expect(SUPPORTED_TOKENS.ethereum).toContain("DAI");
  });

  test("solana supports USDC only", () => {
    expect(SUPPORTED_TOKENS.solana).toEqual(["USDC"]);
  });

  test("TON supports USDC only", () => {
    expect(SUPPORTED_TOKENS.ton).toEqual(["USDC"]);
  });

  test("all three chains are supported", () => {
    expect(SUPPORTED_CHAINS).toContain("ethereum");
    expect(SUPPORTED_CHAINS).toContain("solana");
    expect(SUPPORTED_CHAINS).toContain("ton");
  });
});
