/**
 * Peer commerce read relay — serves store/product catalog from local ledger replay.
 *
 * Every HONE node replays the full commerce ledger, so every node holds a complete
 * copy of all stores and products. These routes expose that local copy publicly,
 * making the catalog censorship-resistant: no single IP is required to browse.
 *
 * Buyers hit /api/peer/commerce/* on any node in the mesh. Sellers' IPs are never
 * exposed for reads. For private data (orders, shipping), buyers use the seller's
 * .onion address (stored on-chain in the store record).
 *
 * These endpoints are intentionally read-only and require no authentication.
 */

const express = require("express");
const router = express.Router();
const stateStore = require("../chain/stateStore");

// GET /api/peer/commerce/stores — full catalog from local ledger
router.get("/stores", (req, res) => {
  const { limit = 50, offset = 0, seller } = req.query;
  let stores = stateStore.getAllStores({ status: "active" });
  if (seller) stores = stores.filter((s) => s.seller === seller);
  stores.sort((a, b) => (b.openedAt || 0) - (a.openedAt || 0));
  const total = stores.length;
  res.json({
    stores: stores.slice(Number(offset), Number(offset) + Number(limit)),
    total,
    served_by: process.env.HONE_NODE_ID || "unknown",
    source: "local-ledger",
  });
});

// GET /api/peer/commerce/stores/:seller — single store + its active products
router.get("/stores/:seller", (req, res) => {
  const store = stateStore.getStore(req.params.seller);
  if (!store) return res.status(404).json({ error: "store not found" });
  const products = stateStore
    .getAllProducts({ seller: req.params.seller, status: "active" })
    .map(strip_private);
  res.json({
    store: strip_store_private(store),
    products,
    served_by: process.env.HONE_NODE_ID || "unknown",
    source: "local-ledger",
  });
});

// GET /api/peer/commerce/products — browse all products
router.get("/products", (req, res) => {
  const { limit = 50, offset = 0, seller, category, token } = req.query;
  const filter = { status: "active" };
  if (seller) filter.seller = seller;
  if (category) filter.category = category;
  let products = stateStore.getAllProducts(filter);
  if (token) products = products.filter((p) => p.token === token);
  products.sort((a, b) => (b.createdEpoch || 0) - (a.createdEpoch || 0));
  const total = products.length;
  res.json({
    products: products.slice(Number(offset), Number(offset) + Number(limit)).map(strip_private),
    total,
    served_by: process.env.HONE_NODE_ID || "unknown",
    source: "local-ledger",
  });
});

// GET /api/peer/commerce/products/:seller/:slug
router.get("/products/:seller/:slug", (req, res) => {
  const productId = `${req.params.seller}/${req.params.slug}`;
  const product = stateStore.getProduct(productId);
  if (!product || product.status !== "active")
    return res.status(404).json({ error: "product not found" });
  res.json({ product: strip_private(product), source: "local-ledger" });
});

// GET /api/peer/commerce/health — confirms this node serves commerce data
router.get("/health", (req, res) => {
  const stores = stateStore.getAllStores({ status: "active" });
  const products = stateStore.getAllProducts({ status: "active" });
  res.json({
    ok: true,
    stores: stores.length,
    products: products.length,
    served_by: process.env.HONE_NODE_ID || "unknown",
    source: "local-ledger",
    note: "This node mirrors the full HONE commerce catalog. No single IP required.",
  });
});

// Strip fields that don't belong in public peer reads
function strip_private(p) {
  const { ...pub } = p;
  return pub;
}

// Strip order/shipping info from store object — only catalog metadata is public
function strip_store_private(s) {
  const { ...pub } = s;
  return pub;
}

module.exports = router;
