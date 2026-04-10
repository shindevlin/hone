"use strict";

/**
 * BTCPC Gateway Storefront — HTML rendering — v2.10.2
 * Shin Devlin
 *
 * Server-side HTML rendering for store/product pages. Pure functions:
 * take a resolved data object from resolver.js, return an HTML string.
 * No framework, no build step — template literals only.
 *
 * Escaping: all user-controlled strings (store names, product titles,
 * descriptions, CIDs) pass through esc() which escapes HTML special
 * characters. Any content_cid references are rendered as data attrs
 * and opaque hash strings, never as resolved content (which would
 * require trusting an external gateway).
 */

function esc(s) {
  if (s === null || s === undefined) return '';
  return String(s)
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;')
    .replace(/'/g, '&#39;');
}

function fmtBtcpc(amount) {
  if (amount === null || amount === undefined) return '0';
  var n = Number(amount);
  if (!isFinite(n)) return '0';
  // Show up to 4 decimal places for display, more only if needed
  if (n >= 1) return n.toFixed(2);
  if (n >= 0.01) return n.toFixed(4);
  return n.toFixed(8);
}

function fmtRep(score) {
  if (score === null || score === undefined) return 'new';
  var n = Number(score);
  if (n > 50) return '⭐ ' + n.toFixed(0);
  if (n > 0) return '👍 ' + n.toFixed(0);
  if (n === 0) return '— 0';
  return '👎 ' + n.toFixed(0);
}

// ─────────────────────────────────────────────────────────────────
// Base layout — shared head/body wrapper
// ─────────────────────────────────────────────────────────────────

function baseLayout(title, bodyHtml) {
  return [
    '<!DOCTYPE html>',
    '<html lang="en">',
    '<head>',
    '<meta charset="UTF-8">',
    '<meta name="viewport" content="width=device-width, initial-scale=1.0">',
    '<title>' + esc(title) + ' — BTCPC</title>',
    '<style>',
    'body{font-family:system-ui,-apple-system,sans-serif;max-width:960px;margin:0 auto;padding:24px;background:#0a0a0a;color:#e0e0e0;line-height:1.5}',
    'a{color:#f7931a;text-decoration:none}a:hover{text-decoration:underline}',
    'header{border-bottom:1px solid #333;padding-bottom:16px;margin-bottom:24px}',
    'header h1{margin:0;font-size:1.5rem}header .tagline{color:#888;font-size:0.875rem;margin-top:4px}',
    'nav{display:flex;gap:16px;margin-top:12px;font-size:0.875rem}',
    '.store-card,.product-card{background:#161616;border:1px solid #2a2a2a;border-radius:8px;padding:16px;margin-bottom:12px}',
    '.store-card h3,.product-card h3{margin:0 0 8px 0;font-size:1.125rem}',
    '.meta{color:#888;font-size:0.875rem}',
    '.badge{display:inline-block;background:#2a2a2a;color:#e0e0e0;padding:2px 8px;border-radius:12px;font-size:0.75rem;margin-right:6px}',
    '.badge.gold{background:#f7931a;color:#000}',
    '.badge.tarnished{background:#555;color:#ccc}',
    '.badge.new{background:#2a4a7a;color:#fff}',
    '.grid{display:grid;grid-template-columns:repeat(auto-fill,minmax(280px,1fr));gap:16px}',
    '.price{font-size:1.25rem;font-weight:600;color:#f7931a}',
    '.stock{color:#888;font-size:0.875rem}',
    '.cid{font-family:monospace;font-size:0.75rem;color:#666;word-break:break-all}',
    'footer{margin-top:48px;padding-top:16px;border-top:1px solid #333;color:#666;font-size:0.75rem;text-align:center}',
    '.notice{background:#1a1a2a;border:1px solid #334;border-radius:6px;padding:12px;margin:16px 0;font-size:0.875rem}',
    '.rep-positive{color:#4ade80}.rep-zero{color:#888}.rep-negative{color:#f87171}',
    '</style>',
    '</head>',
    '<body>',
    '<header>',
    '<h1><a href="/">BTCPC</a></h1>',
    '<div class="tagline">sovereign chain · commerce · compute · no burn, all recycle</div>',
    '<nav><a href="/">home</a><a href="/stores">stores</a></nav>',
    '</header>',
    '<main>',
    bodyHtml,
    '</main>',
    '<footer>BTCPC — Bitcoin Proof of Compute · <a href="https://btcpc.net">btcpc.net</a></footer>',
    '</body>',
    '</html>',
  ].join('\n');
}

// ─────────────────────────────────────────────────────────────────
// Landing page
// ─────────────────────────────────────────────────────────────────

function renderLanding(data) {
  var stores = data.recent_stores || [];
  var body = [
    '<h2>Welcome to BTCPC Commerce</h2>',
    '<p>Open storefronts backed by real stake, priced in a chain that recycles every fee back to the people doing the work. ' + data.total_stores + ' active store' + (data.total_stores === 1 ? '' : 's') + ' right now.</p>',
    stores.length === 0
      ? '<div class="notice">No stores open yet. Be the first — <a href="/stores">browse the directory</a> or POST to <code>/api/commerce/stores</code> to open your own.</div>'
      : '<h3>Recently opened stores</h3><div class="grid">' +
        stores
          .map(function (s) { return renderStoreCard(s); })
          .join('') +
        '</div>',
  ].join('\n');
  return baseLayout('Home', body);
}

function renderStoreCard(store) {
  var repClass = 'rep-zero';
  if ((store.rep_score || 0) > 0) repClass = 'rep-positive';
  if ((store.rep_score || 0) < 0) repClass = 'rep-negative';
  return [
    '<a href="/stores/' + esc(store.seller) + '" class="store-card">',
    '<h3>' + esc(store.name || store.seller) + '</h3>',
    '<div class="meta">by <strong>' + esc(store.seller) + '</strong></div>',
    '<div class="meta">',
    '<span class="' + repClass + '">' + esc(fmtRep(store.rep_score)) + '</span> · ',
    (store.total_sales || 0) + ' sale' + (store.total_sales === 1 ? '' : 's') + ' · ',
    (store.capacity || 0) + ' product slot' + (store.capacity === 1 ? '' : 's'),
    '</div>',
    '</a>',
  ].join('');
}

// ─────────────────────────────────────────────────────────────────
// Store directory
// ─────────────────────────────────────────────────────────────────

function renderStoreDirectory(data) {
  var stores = data.stores || [];
  var body = [
    '<h2>All stores (' + data.total + ')</h2>',
    stores.length === 0
      ? '<div class="notice">No stores open yet.</div>'
      : '<div class="grid">' +
        stores.map(function (s) { return renderStoreCard(s); }).join('') +
        '</div>',
  ].join('\n');
  return baseLayout('Stores', body);
}

// ─────────────────────────────────────────────────────────────────
// Single store profile
// ─────────────────────────────────────────────────────────────────

function renderStoreProfile(data) {
  var store = data.store;
  var products = data.products || [];

  var statusBadge = '';
  if (store.status === 'active') statusBadge = '<span class="badge gold">active</span>';
  else if (store.status === 'closed') statusBadge = '<span class="badge tarnished">closed</span>';

  var repClass = 'rep-zero';
  if ((store.rep_score || 0) > 0) repClass = 'rep-positive';
  if ((store.rep_score || 0) < 0) repClass = 'rep-negative';

  var body = [
    '<article>',
    '<h2>' + esc(store.name || store.seller) + ' ' + statusBadge + '</h2>',
    '<div class="meta">operated by <strong>' + esc(store.seller) + '</strong></div>',
    store.description_cid
      ? '<div class="meta">description: <span class="cid">' + esc(store.description_cid) + '</span></div>'
      : '',
    store.banner_cid
      ? '<div class="meta">banner: <span class="cid">' + esc(store.banner_cid) + '</span></div>'
      : '',
    '<div class="meta" style="margin-top:12px">',
    '<span class="' + repClass + '">' + esc(fmtRep(store.rep_score)) + '</span> · ',
    '<span class="badge">' + (store.rep_votes_up || 0) + ' ↑</span>',
    '<span class="badge">' + (store.rep_votes_down || 0) + ' ↓</span>',
    '</div>',
    '<div class="meta">',
    (store.total_sales || 0) + ' sales · ',
    (store.total_fulfilled || 0) + ' fulfilled · ',
    (store.total_disputed || 0) + ' disputed',
    '</div>',
    '<div class="meta">staked: ' + fmtBtcpc(store.stake_amount) + ' BTCPC · capacity: ' + (store.capacity || 0) + ' slots</div>',
    '</article>',
    '<h3 style="margin-top:32px">Products (' + products.length + ')</h3>',
    products.length === 0
      ? '<div class="notice">No products listed yet.</div>'
      : '<div class="grid">' +
        products.map(function (p) { return renderProductCard(p); }).join('') +
        '</div>',
  ].join('\n');

  return baseLayout(store.name || store.seller, body);
}

function renderProductCard(product) {
  var parts = (product.product_id || '').split('/');
  var path = parts.length === 2 ? '/stores/' + parts[0] + '/' + parts[1] : '#';
  return [
    '<a href="' + esc(path) + '" class="product-card">',
    '<h3>' + esc(product.title || product.product_id) + '</h3>',
    '<div class="price">' + fmtBtcpc(product.price) + ' ' + esc(product.token || 'BTCPC') + '</div>',
    '<div class="stock">' + (product.stock || 0) + ' in stock</div>',
    product.category ? '<div class="meta"><span class="badge">' + esc(product.category) + '</span></div>' : '',
    product.description_snippet
      ? '<div class="meta" style="margin-top:8px">' + esc(product.description_snippet).slice(0, 140) + '</div>'
      : '',
    '</a>',
  ].join('');
}

// ─────────────────────────────────────────────────────────────────
// Single product page
// ─────────────────────────────────────────────────────────────────

function renderProduct(data) {
  var product = data.product;
  var store = data.store;
  var productRep = data.product_reputation;
  var shortcode = data.shortcode;

  var body = [
    '<article>',
    '<div class="meta"><a href="/stores/' + esc(product.seller) + '">← ' + esc(store ? store.name || store.seller : product.seller) + '</a></div>',
    '<h2>' + esc(product.title || product.product_id) + '</h2>',
    '<div class="price" style="font-size:2rem;margin:16px 0">' + fmtBtcpc(product.price) + ' ' + esc(product.token || 'BTCPC') + '</div>',
    '<div class="stock" style="font-size:1rem">' + (product.stock || 0) + ' in stock</div>',
    product.category
      ? '<div class="meta" style="margin-top:12px"><span class="badge">' + esc(product.category) + '</span></div>'
      : '',
    '<div class="meta" style="margin-top:16px">',
    productRep && (productRep.votes_up + productRep.votes_down) > 0
      ? esc(fmtRep(productRep.score)) + ' · ' + productRep.votes_up + ' ↑ / ' + productRep.votes_down + ' ↓'
      : 'no reviews yet',
    '</div>',
    product.description_snippet
      ? '<p style="margin-top:24px;background:#161616;padding:16px;border-radius:8px;border:1px solid #2a2a2a">' + esc(product.description_snippet) + '</p>'
      : '',
    product.content_cid
      ? '<div class="meta" style="margin-top:16px">content: <span class="cid">' + esc(product.content_cid) + '</span></div>'
      : '',
    '<div class="meta" style="margin-top:8px">product id: <span class="cid">' + esc(product.product_id) + '</span></div>',
    '<div class="meta">shortcode: <a href="/s/' + esc(shortcode) + '">/s/' + esc(shortcode) + '</a></div>',
    '<div class="notice">To buy this product, authenticate via /api/commerce/orders with a BTCPC account holding sufficient balance. Order flow: place → seller fulfill → buyer confirm delivery.</div>',
    '</article>',
  ].join('\n');

  return baseLayout(product.title || product.product_id, body);
}

// ─────────────────────────────────────────────────────────────────
// Not found / errors
// ─────────────────────────────────────────────────────────────────

function renderNotFound(data) {
  var body = [
    '<h2>Not found</h2>',
    '<p>' + esc(data.error || 'The page you requested does not exist.') + '</p>',
    '<p><a href="/">Back to home</a></p>',
  ].join('\n');
  return baseLayout('Not Found', body);
}

function renderError(message) {
  var body = [
    '<h2>Something went wrong</h2>',
    '<p>' + esc(message) + '</p>',
    '<p><a href="/">Back to home</a></p>',
  ].join('\n');
  return baseLayout('Error', body);
}

// ─────────────────────────────────────────────────────────────────
// Dispatcher — given a resolver result, return the right HTML
// ─────────────────────────────────────────────────────────────────

function renderResolved(resolved) {
  switch (resolved.kind) {
    case 'landing':
      return renderLanding(resolved.data);
    case 'store_directory':
      return renderStoreDirectory(resolved.data);
    case 'store_profile':
      return renderStoreProfile(resolved.data);
    case 'product':
      return renderProduct(resolved.data);
    case 'not_found':
      return renderNotFound(resolved.data);
    default:
      return renderError('Unknown resolution kind: ' + resolved.kind);
  }
}

module.exports = {
  esc: esc,
  fmtBtcpc: fmtBtcpc,
  fmtRep: fmtRep,
  baseLayout: baseLayout,
  renderLanding: renderLanding,
  renderStoreCard: renderStoreCard,
  renderStoreDirectory: renderStoreDirectory,
  renderStoreProfile: renderStoreProfile,
  renderProductCard: renderProductCard,
  renderProduct: renderProduct,
  renderNotFound: renderNotFound,
  renderError: renderError,
  renderResolved: renderResolved,
};
