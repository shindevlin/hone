"use strict";

/**
 * BTCPC Gateway Resolver — v2.10.2
 * Shin Devlin
 *
 * Pure data-layer module for turning gateway URLs into structured resolved
 * objects. No HTTP, no HTML, no I/O beyond stateStore reads. Used by
 * both the server-side HTML renderer and the JSON /api/resolve endpoint.
 *
 * Resolves the following URL patterns:
 *
 *   /                            → landing (list of recent stores)
 *   /stores                      → directory (paginated)
 *   /stores/:seller              → seller profile (store + product grid)
 *   /stores/:seller/:slug        → single product page
 *   /s/:shortcode                → shortcode → canonical redirect target
 *   /api/resolve/<any-path>      → JSON form of the resolver output
 *
 * Trust model: always reads from the local stateStore. An optional
 * remoteClient.js (future) provides the same interface but queries a
 * remote BTCPC node via JSON-RPC for lightweight gateway deployments.
 */

const crypto = require('crypto');

// Dependency injection — default is the local stateStore, but callers can
// pass a remote client with the same interface. This is how we support
// both local-mode (read stateStore directly) and remote-mode (RPC to a
// full node) without duplicating the rendering logic.
const defaultStateStore = require('../chain/stateStore');

function createResolver(stateStore) {
  stateStore = stateStore || defaultStateStore;

  /**
   * Compute a deterministic 6-character shortcode for a product_id.
   * sha256(product_id)[:6] — no separate index needed because the
   * shortcode is derivable from the canonical ID.
   *
   * Collisions are possible but extremely rare (~1 in 16M per 6 chars).
   * The gateway resolves shortcode collisions by returning all matches.
   */
  function shortcodeFor(productId) {
    return crypto.createHash('sha256').update(productId).digest('hex').slice(0, 6);
  }

  /**
   * Parse a gateway URL path into a resolution descriptor.
   * Returns { kind, ...fields } where kind is one of:
   *   'landing', 'store_directory', 'store_profile', 'product', 'shortcode'
   * Returns { kind: 'unknown' } for unmatched paths.
   */
  function parsePath(urlPath) {
    if (typeof urlPath !== 'string') {
      return { kind: 'unknown' };
    }

    // Empty string maps to landing
    if (urlPath === '') return { kind: 'landing' };

    // Normalize: strip query string, ensure leading slash, remove trailing
    var cleanPath = urlPath.split('?')[0];
    if (!cleanPath.startsWith('/')) cleanPath = '/' + cleanPath;
    if (cleanPath.length > 1 && cleanPath.endsWith('/')) {
      cleanPath = cleanPath.slice(0, -1);
    }

    if (cleanPath === '' || cleanPath === '/') {
      return { kind: 'landing' };
    }

    if (cleanPath === '/stores') {
      return { kind: 'store_directory' };
    }

    // /stores/:seller/:slug — single product
    var productMatch = cleanPath.match(/^\/stores\/([a-z0-9][a-z0-9._-]*)\/([a-z0-9][a-z0-9-]{0,62})$/);
    if (productMatch) {
      return {
        kind: 'product',
        seller: productMatch[1],
        slug: productMatch[2],
        product_id: productMatch[1] + '/' + productMatch[2],
      };
    }

    // /stores/:seller — seller profile
    var sellerMatch = cleanPath.match(/^\/stores\/([a-z0-9][a-z0-9._-]*)$/);
    if (sellerMatch) {
      return { kind: 'store_profile', seller: sellerMatch[1] };
    }

    // /s/:shortcode — shortcode redirect
    var shortMatch = cleanPath.match(/^\/s\/([a-f0-9]{6})$/);
    if (shortMatch) {
      return { kind: 'shortcode', shortcode: shortMatch[1] };
    }

    return { kind: 'unknown', path: cleanPath };
  }

  /**
   * Resolve a parsed path descriptor into actual data from stateStore.
   * Returns { status, data } where status is an HTTP status code and
   * data is the resolved object (or error message).
   */
  function resolve(urlPath) {
    var parsed = parsePath(urlPath);

    switch (parsed.kind) {
      case 'landing':
        return resolveLanding();
      case 'store_directory':
        return resolveStoreDirectory();
      case 'store_profile':
        return resolveStoreProfile(parsed.seller);
      case 'product':
        return resolveProduct(parsed.seller, parsed.slug, parsed.product_id);
      case 'shortcode':
        return resolveShortcode(parsed.shortcode);
      default:
        return {
          status: 404,
          kind: 'not_found',
          data: { error: 'unknown path', path: parsed.path || urlPath },
        };
    }
  }

  function resolveLanding() {
    var stores = stateStore.getAllStores({ status: 'active' });
    // Sort by creation epoch descending (newest first), take top 12
    stores.sort(function (a, b) {
      return (b.created_epoch || 0) - (a.created_epoch || 0);
    });
    var recent = stores.slice(0, 12);
    return {
      status: 200,
      kind: 'landing',
      data: {
        recent_stores: recent,
        total_stores: stores.length,
      },
    };
  }

  function resolveStoreDirectory() {
    var stores = stateStore.getAllStores({ status: 'active' });
    stores.sort(function (a, b) {
      return (b.rep_score || 0) - (a.rep_score || 0);
    });
    return {
      status: 200,
      kind: 'store_directory',
      data: {
        stores: stores,
        total: stores.length,
      },
    };
  }

  function resolveStoreProfile(seller) {
    var store = stateStore.getStore(seller);
    if (!store) {
      return {
        status: 404,
        kind: 'not_found',
        data: { error: 'store not found', seller: seller },
      };
    }
    var products = stateStore.getProductsBySeller(seller)
      .filter(function (p) { return p.status === 'active'; });
    var reputation = stateStore.getReputation('store', seller);
    return {
      status: 200,
      kind: 'store_profile',
      data: {
        store: store,
        products: products,
        product_count: products.length,
        reputation: reputation,
      },
    };
  }

  function resolveProduct(seller, slug, productId) {
    var product = stateStore.getProduct(productId);
    if (!product) {
      return {
        status: 404,
        kind: 'not_found',
        data: { error: 'product not found', product_id: productId },
      };
    }
    var store = stateStore.getStore(seller);
    var productReputation = stateStore.getReputation('product', productId);
    var storeReputation = stateStore.getReputation('store', seller);
    return {
      status: 200,
      kind: 'product',
      data: {
        product: product,
        store: store,
        product_reputation: productReputation,
        store_reputation: storeReputation,
        shortcode: shortcodeFor(productId),
        canonical_path: '/stores/' + seller + '/' + slug,
      },
    };
  }

  /**
   * Shortcode resolution: iterate all active products, hash each product_id,
   * return matches. Linear scan is fine for a small number of products; can
   * be optimized to a dedicated shortcode index later if needed.
   */
  function resolveShortcode(shortcode) {
    var all = stateStore.getAllProducts({ status: 'active' });
    var matches = all.filter(function (p) {
      return shortcodeFor(p.product_id) === shortcode;
    });
    if (matches.length === 0) {
      return {
        status: 404,
        kind: 'not_found',
        data: { error: 'no product matches shortcode', shortcode: shortcode },
      };
    }
    if (matches.length === 1) {
      var p = matches[0];
      var parts = p.product_id.split('/');
      return {
        status: 302,
        kind: 'redirect',
        data: {
          location: '/stores/' + parts[0] + '/' + parts[1],
          product_id: p.product_id,
        },
      };
    }
    // Multiple matches — rare. Let caller disambiguate.
    return {
      status: 300,
      kind: 'multiple_matches',
      data: {
        matches: matches.map(function (p) {
          var parts = p.product_id.split('/');
          return { product_id: p.product_id, path: '/stores/' + parts[0] + '/' + parts[1] };
        }),
      },
    };
  }

  return {
    parsePath: parsePath,
    resolve: resolve,
    shortcodeFor: shortcodeFor,
    // Individual resolvers exposed for direct use
    resolveLanding: resolveLanding,
    resolveStoreDirectory: resolveStoreDirectory,
    resolveStoreProfile: resolveStoreProfile,
    resolveProduct: resolveProduct,
    resolveShortcode: resolveShortcode,
  };
}

// Default export: a resolver instance bound to the local stateStore
module.exports = createResolver();
// Also export the factory for tests or remote-mode gateways
module.exports.createResolver = createResolver;
