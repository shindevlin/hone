"use strict";

/**
 * BTCPC Gateway Server — v2.10.2
 * Shin Devlin
 *
 * HTTP server that turns the commerce stateStore into human-viewable
 * pages and a JSON resolve API. Runs alongside (or independently of)
 * the main BTCPC API server.
 *
 * Usage:
 *   BTCPC_GATEWAY_ENABLED=true node src/index.js   (starts with main API)
 *   BTCPC_GATEWAY_PORT=4343 node bin/btcpc-gateway (standalone)
 *
 * The gateway is READ-ONLY. It never writes to the chain. All mutations
 * happen via the existing /api/commerce routes. The gateway just
 * renders the current state.
 *
 * Trust model: local mode reads from the in-process stateStore. Future
 * remote mode (gated on BTCPC_GATEWAY_REMOTE_RPC_URL) will query a
 * remote full node via JSON-RPC.
 */

const express = require('express');
const resolver = require('./resolver');
const storefront = require('./storefront');

/**
 * Create the gateway router. Can be mounted into an existing Express
 * app or served by its own HTTP server.
 */
function createGatewayRouter(options) {
  options = options || {};
  var router = express.Router();
  var activeResolver = options.resolver || resolver;

  // Health check — always 200 if the process is up
  router.get('/health', function (_req, res) {
    res.json({ status: 'ok', gateway: 'btcpc', version: require('../../package.json').version });
  });

  // JSON resolve API — programmatic access to the same lookups the HTML
  // pages use. Useful for front-end SPAs that want to build their own UI.
  // Express 5 uses named parameters; {*subpath} captures the rest of the URL.
  router.get(/^\/api\/resolve(\/.*)?$/, function (req, res) {
    // req.path is like "/api/resolve/stores/alice" — strip the prefix
    var prefix = '/api/resolve';
    var rest = req.path.slice(prefix.length) || '/';
    var resolved = activeResolver.resolve(rest);
    res.status(resolved.status).json({
      kind: resolved.kind,
      data: resolved.data,
    });
  });

  // Shortcode redirect — /s/:shortcode → 302 to canonical product URL
  router.get('/s/:shortcode', function (req, res) {
    var resolved = activeResolver.resolveShortcode(req.params.shortcode);
    if (resolved.status === 302) {
      return res.redirect(302, resolved.data.location);
    }
    if (resolved.status === 300) {
      res
        .status(300)
        .type('text/html')
        .send(
          storefront.baseLayout(
            'Multiple matches',
            '<h2>Multiple products match this shortcode</h2><ul>' +
              resolved.data.matches
                .map(function (m) {
                  return (
                    '<li><a href="' +
                    storefront.esc(m.path) +
                    '">' +
                    storefront.esc(m.product_id) +
                    '</a></li>'
                  );
                })
                .join('') +
              '</ul>'
          )
        );
      return;
    }
    res
      .status(404)
      .type('text/html')
      .send(storefront.renderNotFound({ error: resolved.data.error }));
  });

  // Main HTML resolver — catch-all for /, /stores, /stores/:seller, etc.
  // Must be registered AFTER the more specific routes above.
  router.get(/^\/(|stores(\/.*)?)$/, function (req, res) {
    try {
      var resolved = activeResolver.resolve(req.path);
      var html = storefront.renderResolved(resolved);
      res.status(resolved.status).type('text/html').send(html);
    } catch (err) {
      console.error('[BTCPC Gateway] resolve error:', err);
      res
        .status(500)
        .type('text/html')
        .send(storefront.renderError(err.message || 'internal error'));
    }
  });

  return router;
}

/**
 * Start a standalone gateway HTTP server on the given port.
 * Returns the underlying http.Server so callers can close it for tests
 * or during graceful shutdown.
 */
function startGateway(options) {
  options = options || {};
  var port = options.port || parseInt(process.env.BTCPC_GATEWAY_PORT || '4343', 10);
  var host = options.host || process.env.BTCPC_GATEWAY_HOST || '0.0.0.0';

  var app = express();
  app.use(express.json());
  app.use(createGatewayRouter(options));

  return new Promise(function (resolve) {
    var server = app.listen(port, host, function () {
      var addr = server.address();
      console.log(
        '[BTCPC Gateway] listening on ' + addr.address + ':' + addr.port + ' (read-only)'
      );
      resolve({ server: server, app: app, port: addr.port });
    });
  });
}

module.exports = {
  createGatewayRouter: createGatewayRouter,
  startGateway: startGateway,
};
