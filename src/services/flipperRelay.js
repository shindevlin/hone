"use strict";

var http = require("http");
var https = require("https");

function normalizeUrl(url) {
  if (typeof url !== "string") return "";
  var trimmed = url.trim();
  if (!trimmed) return "";
  return trimmed.replace(/\/+$/, "");
}

function parseUrlList(raw) {
  if (!raw) return [];

  var parts = Array.isArray(raw) ? raw : String(raw).split(/[, \t\r\n]+/);
  var urls = [];
  var seen = Object.create(null);

  for (var i = 0; i < parts.length; i++) {
    var url = normalizeUrl(parts[i]);
    if (!url || seen[url]) continue;
    seen[url] = true;
    urls.push(url);
  }

  return urls;
}

function isSuccessfulStatus(status) {
  return (status >= 200 && status < 300) || status === 409;
}

function postJson(urlStr, body, options) {
  options = options || {};

  return new Promise(function (resolve, reject) {
    var payload = typeof body === "string" ? body : JSON.stringify(body);
    var headers = Object.assign({
      "Content-Type": "application/json",
      "Content-Length": Buffer.byteLength(payload),
    }, options.headers || {});

    var urlObj;
    try {
      urlObj = new URL(urlStr);
    } catch (err) {
      return reject(new Error("invalid URL: " + urlStr));
    }

    var mod = urlObj.protocol === "https:" ? https : http;
    var req = mod.request({
      hostname: urlObj.hostname,
      port: urlObj.port || (urlObj.protocol === "https:" ? 443 : 80),
      path: urlObj.pathname + (urlObj.search || ""),
      method: "POST",
      headers: headers,
    }, function (res) {
      var chunks = [];
      res.on("data", function (chunk) { chunks.push(chunk); });
      res.on("end", function () {
        var raw = Buffer.concat(chunks).toString("utf8");
        var parsed = {};
        if (raw) {
          try { parsed = JSON.parse(raw); } catch (_) {}
        }
        resolve({
          status: res.statusCode,
          body: parsed,
          raw: raw,
          url: urlStr,
        });
      });
    });

    req.on("error", reject);
    req.setTimeout(options.timeoutMs || 8000, function () {
      req.destroy(new Error("request timeout"));
    });
    req.end(payload);
  });
}

async function postJsonToAny(urls, body, options) {
  options = options || {};
  var list = parseUrlList(urls);
  var lastError = null;
  var lastResult = null;

  for (var i = 0; i < list.length; i++) {
    try {
      var result = await postJson(list[i], body, options);
      if (isSuccessfulStatus(result.status)) {
        return result;
      }
      lastResult = result;
    } catch (err) {
      lastError = err;
    }
  }

  var error = lastError || new Error("all relay targets failed");
  if (lastResult) error.lastResult = lastResult;
  throw error;
}

module.exports = {
  isSuccessfulStatus: isSuccessfulStatus,
  normalizeUrl: normalizeUrl,
  parseUrlList: parseUrlList,
  postJson: postJson,
  postJsonToAny: postJsonToAny,
};
