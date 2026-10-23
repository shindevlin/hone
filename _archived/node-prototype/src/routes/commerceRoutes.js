"use strict";

/**
 * Commerce HTTP Routes — v2.10.1
 * Shin Devlin
 *
 * HTTP layer for the commerce primitive (stores, products, orders,
 * reputation, bonding curve). Wraps ledger.recordX functions with
 * authentication, input validation, and escrow coordination.
 *
 * Chain invariants are enforced in stateStore. These routes enforce
 * HIGHER-LEVEL invariants that benefit from pre-validation: slug format,
 * authenticated seller/buyer matching, stake-backed capacity checks,
 * etc. Bad input returns 400; chain-layer rejection returns 422.
 *
 * Auth model:
 *   - POST /api/commerce/stores         authenticateToken (JWT) — sellers open stores
 *   - POST /api/commerce/products       authenticateToken — sellers list products
 *   - POST /api/commerce/orders         authenticateToken — buyers place orders
 *   - POST /api/commerce/orders/:id/fulfill   authenticateToken — sellers ship
 *   - POST /api/commerce/orders/:id/deliver   authenticateToken — buyers confirm
 *   - POST /api/commerce/reputation/vote      authenticateToken
 *   - GET  endpoints are public (commerce data is public on chain)
 *
 * All mutations flow through ledger.recordX → stateStore. No direct
 * writes. No Mongoose. Block files remain the canonical source of truth.
 */

const express = require('express');
const router = express.Router();
const { authenticateToken } = require('../middlewares/auth');
const ledger = require('../services/ledger');
const stateStore = require('../chain/stateStore');
const bondingCurve = require('../services/stakeBondingCurve');
const escrow = require('../services/escrow');
const crypto = require('crypto');
const {
  sanitizeString,
  sanitizeAmount,
  sanitizePagination,
  rejectObjectInputs,
} = require('../middlewares/validate');

// Valid slug format for product_id slug portion: lowercase alphanumeric + dashes
const SLUG_PATTERN = /^[a-z0-9][a-z0-9-]{0,62}$/;

function isValidSlug(s) {
  return typeof s === 'string' && SLUG_PATTERN.test(s);
}

function buildProductId(seller, slug) {
  return seller + '/' + slug;
}

async function getCurrentEpoch() {
  return ledger.getCurrentEpoch();
}

// ─────────────────────────────────────────────────────────────────
// Stores
// ─────────────────────────────────────────────────────────────────

/**
 * POST /api/commerce/stores
 * Open a storefront for the authenticated user.
 * Body: {
 *   name, banner_cid?, description_cid?, categories?,
 *   initial_capacity, stake_paid_usd
 * }
 * The caller must have already paid the stable-coin bonding curve
 * fee to the treasury (handled by the bridge route, not here).
 * Stake amount in HONE is computed from capacity via the curve.
 */
router.post('/stores', authenticateToken, async (req, res) => {
  try {
    const seller = req.user.username;
    if (!seller) return res.status(401).json({ error: 'unauthenticated' });

    const objErr = rejectObjectInputs(req.body, ['name', 'banner_cid', 'description_cid']);
    if (objErr) return res.status(400).json({ error: objErr });

    const name = sanitizeString(req.body.name, 64);
    if (!name) return res.status(400).json({ error: 'name required' });

    const bannerCid = sanitizeString(req.body.banner_cid, 128) || null;
    const descriptionCid = sanitizeString(req.body.description_cid, 128) || null;
    const categories = Array.isArray(req.body.categories)
      ? req.body.categories.map((c) => sanitizeString(c, 32)).filter(Boolean).slice(0, 16)
      : [];

    const initialCapacity = parseInt(req.body.initial_capacity || 10, 10);
    if (!Number.isFinite(initialCapacity) || initialCapacity < 1 || initialCapacity > 10000) {
      return res.status(400).json({ error: 'initial_capacity must be 1-10000' });
    }

    // Chain invariant: one active store per account
    const existing = stateStore.getStore(seller);
    if (existing && existing.status === 'active') {
      return res.status(409).json({ error: 'store already open for this account' });
    }

    // Bonding curve: USD cost for the requested capacity
    const costUsd = bondingCurve.costForCapacity(0, initialCapacity);
    const stakeHone = bondingCurve.stakeForCapacity(initialCapacity);

    // Balance precheck: seller must have enough HONE to cover stake
    const sellerBalance = stateStore.getBalance(seller, 'HONE');
    if (sellerBalance < stakeHone) {
      return res.status(402).json({
        error: 'insufficient HONE balance to cover stake',
        required_stake_hone: stakeHone,
        current_balance_hone: sellerBalance,
        capacity_usd_cost: costUsd,
      });
    }

    const epoch = await getCurrentEpoch();

    // Lock the HONE stake (transfer seller → staking pool)
    await ledger.recordStake(seller, stakeHone, 'store:' + seller, epoch);

    // Record the store opening (stateStore dispatcher creates the record)
    await ledger.recordStoreOpen(
      seller,
      {
        name,
        banner_cid: bannerCid,
        description_cid: descriptionCid,
        categories,
      },
      initialCapacity,
      stakeHone,
      costUsd,
      epoch
    );

    const store = stateStore.getStore(seller);
    res.status(201).json({ success: true, store, cost_usd: costUsd, stake_hone: stakeHone });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

/**
 * PUT /api/commerce/stores/:seller
 * Update store metadata. Only the seller can update their own store.
 */
router.put('/stores/:seller', authenticateToken, async (req, res) => {
  try {
    const seller = req.params.seller;
    if (req.user.username !== seller) {
      return res.status(403).json({ error: 'can only update your own store' });
    }
    const store = stateStore.getStore(seller);
    if (!store) return res.status(404).json({ error: 'store not found' });

    const objErr = rejectObjectInputs(req.body, ['name', 'banner_cid', 'description_cid']);
    if (objErr) return res.status(400).json({ error: objErr });

    const updates = {};
    if (req.body.name !== undefined) updates.name = sanitizeString(req.body.name, 64);
    if (req.body.banner_cid !== undefined) updates.banner_cid = sanitizeString(req.body.banner_cid, 128);
    if (req.body.description_cid !== undefined) updates.description_cid = sanitizeString(req.body.description_cid, 128);
    if (Array.isArray(req.body.categories)) {
      updates.categories = req.body.categories.map((c) => sanitizeString(c, 32)).filter(Boolean).slice(0, 16);
    }

    const epoch = await getCurrentEpoch();
    await ledger.recordStoreUpdate(seller, updates, epoch);

    res.json({ success: true, store: stateStore.getStore(seller) });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

/**
 * DELETE /api/commerce/stores/:seller
 * Close a storefront. Only the seller can close their own store.
 * Does NOT return the stake — unbonding happens via the staking module
 * with a 14-epoch unbonding period.
 */
router.delete('/stores/:seller', authenticateToken, async (req, res) => {
  try {
    const seller = req.params.seller;
    if (req.user.username !== seller) {
      return res.status(403).json({ error: 'can only close your own store' });
    }
    const store = stateStore.getStore(seller);
    if (!store) return res.status(404).json({ error: 'store not found' });
    if (store.status === 'closed') return res.status(409).json({ error: 'store already closed' });

    const epoch = await getCurrentEpoch();
    await ledger.recordStoreClose(seller, epoch);

    res.json({
      success: true,
      store: stateStore.getStore(seller),
      unbonding_note: 'Stake will be released after 14 epochs via the staking module',
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

/**
 * POST /api/commerce/stores/:seller/capacity
 * Expand a store's capacity via the bonding curve.
 * Body: { additional_capacity }
 */
router.post('/stores/:seller/capacity', authenticateToken, async (req, res) => {
  try {
    const seller = req.params.seller;
    if (req.user.username !== seller) {
      return res.status(403).json({ error: 'can only expand your own store' });
    }
    const store = stateStore.getStore(seller);
    if (!store || store.status !== 'active') {
      return res.status(404).json({ error: 'active store not found' });
    }

    const additional = parseInt(req.body.additional_capacity, 10);
    if (!Number.isFinite(additional) || additional < 1 || additional > 10000) {
      return res.status(400).json({ error: 'additional_capacity must be 1-10000' });
    }

    const currentCapacity = store.capacity || 0;
    const costUsd = bondingCurve.costForCapacity(currentCapacity, additional);
    const additionalStakeHone = bondingCurve.stakeForCapacity(additional);

    const sellerBalance = stateStore.getBalance(seller, 'HONE');
    if (sellerBalance < additionalStakeHone) {
      return res.status(402).json({
        error: 'insufficient HONE balance to cover additional stake',
        required_stake_hone: additionalStakeHone,
        current_balance_hone: sellerBalance,
      });
    }

    const epoch = await getCurrentEpoch();

    // Lock the additional stake
    await ledger.recordStake(seller, additionalStakeHone, 'store:' + seller, epoch);

    // Record the capacity expansion
    await ledger.recordStakePurchase(
      seller,
      additional,
      'wUSDC',
      costUsd,
      additionalStakeHone,
      epoch
    );

    res.json({
      success: true,
      store: stateStore.getStore(seller),
      capacity_delta: additional,
      cost_usd: costUsd,
      stake_hone: additionalStakeHone,
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

/**
 * GET /api/commerce/stores
 * List all active stores. Public, paginated.
 */
router.get('/stores', (req, res) => {
  const { page, limit } = sanitizePagination(req.query.page, req.query.limit, 100);
  const all = stateStore.getAllStores({ status: 'active' });
  const total = all.length;
  const start = (page - 1) * limit;
  const stores = all.slice(start, start + limit);
  res.json({ stores, page, limit, total });
});

/**
 * GET /api/commerce/stores/:seller
 * Get a single store by seller account name.
 */
router.get('/stores/:seller', (req, res) => {
  const store = stateStore.getStore(req.params.seller);
  if (!store) return res.status(404).json({ error: 'store not found' });
  const products = stateStore.getProductsBySeller(req.params.seller);
  res.json({ store, products });
});

// ─────────────────────────────────────────────────────────────────
// Products
// ─────────────────────────────────────────────────────────────────

/**
 * POST /api/commerce/products
 * Create a product listing.
 * Body: { slug, title, description_snippet?, content_cid?, category?, price, stock }
 * product_id is computed server-side as <authenticated_seller>/<slug>
 */
router.post('/products', authenticateToken, async (req, res) => {
  try {
    const seller = req.user.username;
    if (!seller) return res.status(401).json({ error: 'unauthenticated' });

    const objErr = rejectObjectInputs(req.body, ['slug', 'title', 'description_snippet', 'content_cid', 'category']);
    if (objErr) return res.status(400).json({ error: objErr });

    const slug = sanitizeString(req.body.slug, 63);
    if (!isValidSlug(slug)) {
      return res.status(400).json({
        error: 'slug must match ^[a-z0-9][a-z0-9-]{0,62}$ (lowercase alphanumeric + dashes, start alphanumeric, max 63 chars)',
      });
    }

    const title = sanitizeString(req.body.title, 128);
    if (!title) return res.status(400).json({ error: 'title required' });

    const descriptionSnippet = sanitizeString(req.body.description_snippet, 256) || '';
    const contentCid = sanitizeString(req.body.content_cid, 128) || null;
    const category = sanitizeString(req.body.category, 32) || 'uncategorized';

    const price = sanitizeAmount(req.body.price);
    if (price === null) return res.status(400).json({ error: 'price must be a positive number' });

    const stock = parseInt(req.body.stock, 10);
    if (!Number.isFinite(stock) || stock < 0) {
      return res.status(400).json({ error: 'stock must be a non-negative integer' });
    }

    // Chain invariant preconditions (stateStore also enforces)
    const store = stateStore.getStore(seller);
    if (!store || store.status !== 'active') {
      return res.status(422).json({ error: 'active store required to list products' });
    }
    const activeCount = stateStore.getStoreActiveProductCount(seller);
    if (activeCount >= (store.capacity || 0)) {
      return res.status(422).json({
        error: 'store capacity exhausted — expand capacity via POST /stores/:seller/capacity',
        active_products: activeCount,
        capacity: store.capacity,
      });
    }

    const productId = buildProductId(seller, slug);
    if (stateStore.getProduct(productId)) {
      return res.status(409).json({ error: 'product already exists', product_id: productId });
    }

    const epoch = await getCurrentEpoch();
    await ledger.recordProductCreate(
      seller,
      {
        product_id: productId,
        title,
        description_snippet: descriptionSnippet,
        content_cid: contentCid,
        category,
        price,
        token: 'HONE',
        stock,
      },
      epoch
    );

    res.status(201).json({ success: true, product: stateStore.getProduct(productId) });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

/**
 * PUT /api/commerce/products/:productId
 * productId is URL-encoded, e.g., "alice%2Fwidget"
 */
router.put('/products/:productId', authenticateToken, async (req, res) => {
  try {
    const productId = decodeURIComponent(req.params.productId);
    const existing = stateStore.getProduct(productId);
    if (!existing) return res.status(404).json({ error: 'product not found' });
    if (existing.seller !== req.user.username) {
      return res.status(403).json({ error: 'can only update your own products' });
    }

    const objErr = rejectObjectInputs(req.body, ['title', 'description_snippet', 'content_cid', 'category']);
    if (objErr) return res.status(400).json({ error: objErr });

    const updates = {};
    if (req.body.title !== undefined) updates.title = sanitizeString(req.body.title, 128);
    if (req.body.description_snippet !== undefined) {
      updates.description_snippet = sanitizeString(req.body.description_snippet, 256);
    }
    if (req.body.content_cid !== undefined) updates.content_cid = sanitizeString(req.body.content_cid, 128);
    if (req.body.category !== undefined) updates.category = sanitizeString(req.body.category, 32);
    if (req.body.price !== undefined) {
      const p = sanitizeAmount(req.body.price);
      if (p === null) return res.status(400).json({ error: 'price must be a positive number' });
      updates.price = p;
    }
    if (req.body.stock !== undefined) {
      const s = parseInt(req.body.stock, 10);
      if (!Number.isFinite(s) || s < 0) return res.status(400).json({ error: 'stock must be a non-negative integer' });
      updates.stock = s;
    }

    const epoch = await getCurrentEpoch();
    await ledger.recordProductUpdate(req.user.username, productId, updates, epoch);

    res.json({ success: true, product: stateStore.getProduct(productId) });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

/**
 * DELETE /api/commerce/products/:productId
 * Delist a product. Only the seller can delist their own products.
 */
router.delete('/products/:productId', authenticateToken, async (req, res) => {
  try {
    const productId = decodeURIComponent(req.params.productId);
    const existing = stateStore.getProduct(productId);
    if (!existing) return res.status(404).json({ error: 'product not found' });
    if (existing.seller !== req.user.username) {
      return res.status(403).json({ error: 'can only delist your own products' });
    }

    const epoch = await getCurrentEpoch();
    await ledger.recordProductDelist(req.user.username, productId, epoch);

    res.json({ success: true, product: stateStore.getProduct(productId) });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

/**
 * GET /api/commerce/products
 * List products. Public, paginated, filterable.
 * Query: ?category=&seller=&status=active&page=&limit=
 */
router.get('/products', (req, res) => {
  const { page, limit } = sanitizePagination(req.query.page, req.query.limit, 100);
  const filter = { status: req.query.status || 'active' };
  if (req.query.seller) filter.seller = sanitizeString(req.query.seller, 20);
  if (req.query.category) filter.category = sanitizeString(req.query.category, 32);
  const all = stateStore.getAllProducts(filter);
  const total = all.length;
  const start = (page - 1) * limit;
  const products = all.slice(start, start + limit);
  res.json({ products, page, limit, total });
});

/**
 * GET /api/commerce/products/:productId
 */
router.get('/products/:productId', (req, res) => {
  const productId = decodeURIComponent(req.params.productId);
  const product = stateStore.getProduct(productId);
  if (!product) return res.status(404).json({ error: 'product not found' });
  const store = stateStore.getStore(product.seller);
  res.json({ product, store });
});

// ─────────────────────────────────────────────────────────────────
// Orders
// ─────────────────────────────────────────────────────────────────

/**
 * POST /api/commerce/orders
 * Place an order. Locks escrow from buyer, records ORDER_PLACE.
 * Body: { product_id, quantity }
 */
router.post('/orders', authenticateToken, async (req, res) => {
  try {
    const buyer = req.user.username;
    if (!buyer) return res.status(401).json({ error: 'unauthenticated' });

    const productId = sanitizeString(req.body.product_id, 128);
    if (!productId) return res.status(400).json({ error: 'product_id required' });

    const quantity = parseInt(req.body.quantity, 10);
    if (!Number.isFinite(quantity) || quantity < 1) {
      return res.status(400).json({ error: 'quantity must be a positive integer' });
    }

    const product = stateStore.getProduct(productId);
    if (!product) return res.status(404).json({ error: 'product not found' });
    if (product.status !== 'active') return res.status(422).json({ error: 'product is not active' });
    if (product.seller === buyer) return res.status(422).json({ error: 'cannot order your own product' });
    if ((product.stock || 0) < quantity) {
      return res.status(422).json({
        error: 'insufficient stock',
        stock: product.stock,
        requested: quantity,
      });
    }

    const total = parseFloat((product.price * quantity).toFixed(10));
    const buyerBalance = stateStore.getBalance(buyer, product.token || 'HONE');
    if (buyerBalance < total) {
      return res.status(402).json({
        error: 'insufficient balance',
        required: total,
        current: buyerBalance,
        token: product.token || 'HONE',
      });
    }

    const epoch = await getCurrentEpoch();
    const orderId = 'order-' + crypto.randomBytes(8).toString('hex');
    const escrowId = 'escrow-' + orderId;

    // Lock the escrow (buyer pays into escrow)
    await ledger.recordEscrowLock(buyer, escrowId, total, epoch);

    // Record the order
    await ledger.recordOrderPlace(
      buyer,
      product.seller,
      orderId,
      productId,
      quantity,
      product.price,
      product.token || 'HONE',
      escrowId,
      epoch
    );

    res.status(201).json({ success: true, order: stateStore.getOrder(orderId) });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

/**
 * POST /api/commerce/orders/:orderId/fulfill
 * Seller marks the order as shipped/fulfilled.
 * Body: { fulfillment_cid? } - optional CID pointing to tracking/proof blob
 */
router.post('/orders/:orderId/fulfill', authenticateToken, async (req, res) => {
  try {
    const order = stateStore.getOrder(req.params.orderId);
    if (!order) return res.status(404).json({ error: 'order not found' });
    if (order.seller !== req.user.username) {
      return res.status(403).json({ error: 'only the seller can fulfill' });
    }
    if (order.status !== 'placed') {
      return res.status(409).json({ error: 'order is not in placed status', current: order.status });
    }

    const fulfillmentCid = sanitizeString(req.body.fulfillment_cid, 128) || null;
    const epoch = await getCurrentEpoch();
    await ledger.recordOrderFulfill(req.user.username, req.params.orderId, fulfillmentCid, epoch);

    res.json({ success: true, order: stateStore.getOrder(req.params.orderId) });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

/**
 * POST /api/commerce/orders/:orderId/deliver
 * Buyer confirms delivery. Releases escrow to seller and settles
 * the 1% platform fee split (0.5% recycle / 0.4% stakers / 0.1% rep pool).
 */
router.post('/orders/:orderId/deliver', authenticateToken, async (req, res) => {
  try {
    const order = stateStore.getOrder(req.params.orderId);
    if (!order) return res.status(404).json({ error: 'order not found' });
    if (order.buyer !== req.user.username) {
      return res.status(403).json({ error: 'only the buyer can confirm delivery' });
    }
    if (order.status !== 'fulfilled') {
      return res.status(409).json({ error: 'order must be fulfilled before delivery confirmation', current: order.status });
    }

    const epoch = await getCurrentEpoch();

    // Release escrow to seller (full amount)
    await ledger.recordEscrowRelease(
      order.seller,
      order.escrow_id,
      order.total,
      epoch,
      'Order delivered: ' + req.params.orderId
    );

    // Record delivery confirmation (also triggers platform fee split
    // in recordOrderDelivered via the settlement helper)
    await ledger.recordOrderDelivered(req.user.username, req.params.orderId, epoch);

    res.json({
      success: true,
      order: stateStore.getOrder(req.params.orderId),
      platform_fee_note: '1% platform fee deducted from seller: 0.5% recycle, 0.4% stakers, 0.1% reputation pool',
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

/**
 * POST /api/commerce/orders/:orderId/cancel
 * Either party can cancel a PLACED (not yet fulfilled) order.
 * Refunds escrow to buyer.
 */
router.post('/orders/:orderId/cancel', authenticateToken, async (req, res) => {
  try {
    const order = stateStore.getOrder(req.params.orderId);
    if (!order) return res.status(404).json({ error: 'order not found' });
    if (order.buyer !== req.user.username && order.seller !== req.user.username) {
      return res.status(403).json({ error: 'only buyer or seller can cancel' });
    }
    if (order.status !== 'placed') {
      return res.status(409).json({ error: 'can only cancel orders in placed status', current: order.status });
    }

    const epoch = await getCurrentEpoch();

    // Refund escrow to buyer
    await ledger.recordEscrowRefund(order.buyer, order.escrow_id, order.total, epoch);

    // Record cancellation (releases stock)
    await ledger.recordOrderCancel(req.user.username, req.params.orderId, epoch);

    res.json({ success: true, order: stateStore.getOrder(req.params.orderId) });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

/**
 * POST /api/commerce/orders/:orderId/dispute
 * Buyer opens a dispute on an order. Triggers dispute resolution flow
 * (verifier panel review — wired up in a later phase).
 * Body: { memo? }
 */
router.post('/orders/:orderId/dispute', authenticateToken, async (req, res) => {
  try {
    const order = stateStore.getOrder(req.params.orderId);
    if (!order) return res.status(404).json({ error: 'order not found' });
    if (order.buyer !== req.user.username) {
      return res.status(403).json({ error: 'only the buyer can dispute' });
    }
    if (!['placed', 'fulfilled'].includes(order.status)) {
      return res.status(409).json({ error: 'can only dispute placed or fulfilled orders', current: order.status });
    }

    const memo = sanitizeString(req.body.memo, 256) || null;
    const epoch = await getCurrentEpoch();
    await ledger.recordOrderDispute(req.user.username, req.params.orderId, memo, epoch);

    res.json({
      success: true,
      order: stateStore.getOrder(req.params.orderId),
      note: 'Dispute opened. Escrow remains locked pending resolution.',
    });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

/**
 * GET /api/commerce/orders/my
 * List the authenticated user's orders (as buyer or seller).
 * Query: ?role=buyer|seller (defaults to both)
 */
router.get('/orders/my', authenticateToken, (req, res) => {
  const user = req.user.username;
  const role = req.query.role;
  let orders;
  if (role === 'buyer') {
    orders = stateStore.getOrdersByBuyer(user);
  } else if (role === 'seller') {
    orders = stateStore.getOrdersBySeller(user);
  } else {
    const buyer = stateStore.getOrdersByBuyer(user);
    const seller = stateStore.getOrdersBySeller(user);
    orders = buyer.concat(seller);
  }
  res.json({ orders, count: orders.length });
});

/**
 * GET /api/commerce/orders/:orderId
 */
router.get('/orders/:orderId', (req, res) => {
  const order = stateStore.getOrder(req.params.orderId);
  if (!order) return res.status(404).json({ error: 'order not found' });
  res.json({ order });
});

// ─────────────────────────────────────────────────────────────────
// Reputation
// ─────────────────────────────────────────────────────────────────

/**
 * POST /api/commerce/reputation/vote
 * Vote on a store, product, or miner.
 * Body: { target_type, target_id, vote: +1|-1, memo? }
 * Weight is computed from the voter's stake (anti-sybil).
 */
router.post('/reputation/vote', authenticateToken, async (req, res) => {
  try {
    const voter = req.user.username;
    const targetType = sanitizeString(req.body.target_type, 16);
    const targetId = sanitizeString(req.body.target_id, 128);
    const voteInput = parseInt(req.body.vote, 10);
    const memo = sanitizeString(req.body.memo, 256) || null;

    if (!['store', 'miner', 'product'].includes(targetType)) {
      return res.status(400).json({ error: 'target_type must be store, miner, or product' });
    }
    if (!targetId) return res.status(400).json({ error: 'target_id required' });
    if (voteInput !== 1 && voteInput !== -1) {
      return res.status(400).json({ error: 'vote must be +1 or -1' });
    }

    // Compute vote weight from voter's stake: weight = min(100, 1 + sqrt(stake))
    const voterStakePool = stateStore.getStakePool(voter);
    const voterStake = voterStakePool ? (voterStakePool.total_staked || 0) : 0;
    const weight = Math.max(1, Math.min(100, Math.floor(1 + Math.sqrt(voterStake))));

    const epoch = await getCurrentEpoch();
    await ledger.recordReputationVote(voter, targetType, targetId, voteInput, weight, memo, epoch);

    const rep = stateStore.getReputation(targetType, targetId);
    res.json({ success: true, reputation: rep, vote_weight: weight });
  } catch (err) {
    res.status(500).json({ error: err.message });
  }
});

/**
 * GET /api/commerce/reputation/:targetType/:targetId
 */
router.get('/reputation/:targetType/:targetId', (req, res) => {
  const rep = stateStore.getReputation(req.params.targetType, req.params.targetId);
  if (!rep) return res.json({ reputation: null, message: 'no votes recorded yet' });
  res.json({ reputation: rep });
});

// ─────────────────────────────────────────────────────────────────
// Bonding curve quote (helper endpoint)
// ─────────────────────────────────────────────────────────────────

/**
 * GET /api/commerce/quote/capacity?current=0&additional=10
 * Returns the USD cost to add capacity via the bonding curve.
 */
router.get('/quote/capacity', (req, res) => {
  const current = parseInt(req.query.current, 10) || 0;
  const additional = parseInt(req.query.additional, 10) || 1;
  if (current < 0 || additional < 1 || additional > 10000) {
    return res.status(400).json({ error: 'invalid parameters' });
  }
  const costUsd = bondingCurve.costForCapacity(current, additional);
  const stakeHone = bondingCurve.stakeForCapacity(additional);
  res.json({
    current_capacity: current,
    additional_capacity: additional,
    cost_usd: costUsd,
    stake_hone: stakeHone,
  });
});

module.exports = router;
