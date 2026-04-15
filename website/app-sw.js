// BTCPC Sensor — Service Worker
const CACHE_NAME = 'btcpc-node-v3';
const ASSETS = ['/app', '/app.html', '/inference-crypto.js', '/inference-engine.js', '/miner.js'];

self.addEventListener('install', e => {
  e.waitUntil(
    caches.open(CACHE_NAME).then(c => c.addAll(ASSETS)).then(() => self.skipWaiting())
  );
});

self.addEventListener('activate', e => {
  e.waitUntil(
    caches.keys().then(keys =>
      Promise.all(keys.filter(k => k !== CACHE_NAME).map(k => caches.delete(k)))
    ).then(() => self.clients.claim())
  );
});

self.addEventListener('fetch', e => {
  // Network-first for API calls, cache-first for app shell
  if (e.request.url.includes('/api/')) {
    e.respondWith(
      fetch(e.request).catch(() => new Response(JSON.stringify({ error: 'offline' }), {
        headers: { 'Content-Type': 'application/json' }
      }))
    );
  } else {
    e.respondWith(
      caches.match(e.request).then(r => r || fetch(e.request).then(resp => {
        const clone = resp.clone();
        caches.open(CACHE_NAME).then(c => c.put(e.request, clone));
        return resp;
      }))
    );
  }
});

// Background sync for buffered readings
self.addEventListener('sync', e => {
  if (e.tag === 'btcpc-sensor-sync') {
    e.waitUntil(syncReadings());
  }
});

async function syncReadings() {
  // Open IndexedDB and push any buffered readings
  try {
    const db = await openDB();
    const tx = db.transaction('readings', 'readonly');
    const store = tx.objectStore('readings');
    const all = await idbGetAll(store);
    if (!all.length) return;

    const account = await getStoredAccount(db);
    if (!account) return;

    // Group by sensor type
    const groups = {};
    for (const r of all) {
      const key = r.sensorType || 'unknown';
      if (!groups[key]) groups[key] = [];
      groups[key].push(r);
    }

    for (const [type, readings] of Object.entries(groups)) {
      try {
        const resp = await fetch(`/api/sensors/${account}/phone-${type}/readings`, {
          method: 'POST',
          headers: { 'Content-Type': 'application/json' },
          body: JSON.stringify({ readings })
        });
        if (resp.ok) {
          // Remove synced readings
          const delTx = db.transaction('readings', 'readwrite');
          const delStore = delTx.objectStore('readings');
          for (const r of readings) {
            delStore.delete(r.id);
          }
        }
      } catch (_) { /* will retry on next sync */ }
    }
  } catch (_) { /* IndexedDB not available in SW context on some browsers */ }
}

function openDB() {
  return new Promise((resolve, reject) => {
    const req = indexedDB.open('btcpc-sensor', 1);
    req.onupgradeneeded = () => {
      const db = req.result;
      if (!db.objectStoreNames.contains('readings')) {
        db.createObjectStore('readings', { keyPath: 'id', autoIncrement: true });
      }
      if (!db.objectStoreNames.contains('config')) {
        db.createObjectStore('config', { keyPath: 'key' });
      }
    };
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(req.error);
  });
}

function idbGetAll(store) {
  return new Promise((resolve, reject) => {
    const req = store.getAll();
    req.onsuccess = () => resolve(req.result);
    req.onerror = () => reject(req.error);
  });
}

function getStoredAccount(db) {
  return new Promise((resolve) => {
    try {
      const tx = db.transaction('config', 'readonly');
      const store = tx.objectStore('config');
      const req = store.get('account');
      req.onsuccess = () => resolve(req.result ? req.result.value : null);
      req.onerror = () => resolve(null);
    } catch (_) { resolve(null); }
  });
}
