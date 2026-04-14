#!/usr/bin/env node
"use strict";

/**
 * Static file server for btcpc.net — serves the website/ directory on port 4243.
 * Supports extensionless URLs (/install → install.html, /clock → clock.html).
 */

const express = require("express");
const path = require("path");
const http = require("http");

const app = express();
const PORT = process.env.PORT || 4243;
const ROOT = __dirname;
const API_PORT = process.env.BTCPC_API_PORT || 3100;

// Permissions-Policy header for PWA sensor access on Android Chrome
app.use((req, res, next) => {
  res.setHeader(
    "Permissions-Policy",
    "accelerometer=*, gyroscope=*, magnetometer=*, ambient-light-sensor=*"
  );
  next();
});

// Proxy /api/* to the BTCPC API for sensor nodes and bot endpoints
app.use("/api", (req, res) => {
  const options = {
    hostname: "127.0.0.1",
    port: API_PORT,
    path: "/api" + req.url,
    method: req.method,
    headers: req.headers,
  };
  const proxyReq = http.request(options, (proxyRes) => {
    res.writeHead(proxyRes.statusCode, proxyRes.headers);
    proxyRes.pipe(res);
  });
  proxyReq.on("error", (err) => {
    res.status(502).json({ error: "API unreachable: " + err.message });
  });
  if (req.method === "POST" || req.method === "PUT") {
    let body = "";
    req.on("data", (chunk) => (body += chunk));
    req.on("end", () => proxyReq.end(body));
  } else {
    proxyReq.end();
  }
});

// Proxy /public/* to the BTCPC API for browser clock nodes
app.use("/public", (req, res) => {
  const options = {
    hostname: "127.0.0.1",
    port: API_PORT,
    path: "/public" + req.url,
    method: req.method,
    headers: req.headers,
  };
  const proxyReq = http.request(options, (proxyRes) => {
    res.writeHead(proxyRes.statusCode, proxyRes.headers);
    proxyRes.pipe(res);
  });
  proxyReq.on("error", (err) => {
    res.status(502).json({ error: "API unreachable: " + err.message });
  });
  if (req.method === "POST" || req.method === "PUT") {
    let body = "";
    req.on("data", (chunk) => (body += chunk));
    req.on("end", () => proxyReq.end(body));
  } else {
    proxyReq.end();
  }
});

// Extensionless URLs: /install → /install.html, /clock → /clock.html, etc.
app.use((req, res, next) => {
  if (req.path === "/" || req.path.includes(".")) return next();
  const htmlPath = path.join(ROOT, req.path + ".html");
  require("fs").stat(htmlPath, (err, stats) => {
    if (!err && stats.isFile()) {
      res.sendFile(htmlPath);
    } else {
      next();
    }
  });
});

// Serve static files from website/
app.use(express.static(ROOT, {
  extensions: ["html"],
  etag: true,
  lastModified: true,
}));

// Fallback: 404
app.use((req, res) => {
  res.status(404).sendFile(path.join(ROOT, "index.html"));
});

app.listen(PORT, "127.0.0.1", () => {
  console.log(`[website] serving ${ROOT} on http://127.0.0.1:${PORT}`);
});
