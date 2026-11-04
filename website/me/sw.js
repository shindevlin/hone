/* HONE My Birds: keeps the page shell for offline/home-screen starts. Never caches /api. */
var CACHE = "hone-me-v1";
var SHELL = ["/me/", "/me/manifest.webmanifest", "/icon-192.png", "/apple-touch-icon.png"];
self.addEventListener("install", function (e) {
  e.waitUntil(caches.open(CACHE).then(function (c) { return c.addAll(SHELL); }).then(function () { return self.skipWaiting(); }));
});
self.addEventListener("activate", function (e) {
  e.waitUntil(caches.keys().then(function (ks) {
    return Promise.all(ks.filter(function (k) { return k !== CACHE; }).map(function (k) { return caches.delete(k); }));
  }).then(function () { return self.clients.claim(); }));
});
self.addEventListener("fetch", function (e) {
  var u = new URL(e.request.url);
  if (e.request.method !== "GET" || u.origin !== location.origin || u.pathname.indexOf("/api/") === 0) return;
  if (e.request.mode === "navigate" || SHELL.indexOf(u.pathname) >= 0) {
    e.respondWith(fetch(e.request).then(function (r) {
      var copy = r.clone(); caches.open(CACHE).then(function (c) { c.put(u.pathname === "/me" ? "/me/" : e.request, copy); });
      return r;
    }).catch(function () { return caches.match(e.request).then(function (m) { return m || caches.match("/me/"); }); }));
  }
});
