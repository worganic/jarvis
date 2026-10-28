// Jarvis Nomade — coquille de l'application en cache (lecture hors ligne ; les données sont gardées par app.js).
const CACHE = 'nomade-v10';
const IMAGES = 'nomade-images';
const SHELL = ['./', 'index.html', 'app.css', 'app.js', 'manifest.json', 'icon.svg'];

self.addEventListener('install', (e) => {
  e.waitUntil(caches.open(CACHE).then((c) => c.addAll(SHELL)).then(() => self.skipWaiting()));
});

self.addEventListener('activate', (e) => {
  e.waitUntil(caches.keys().then((keys) => Promise.all(keys.filter((k) => k !== CACHE && k !== IMAGES).map((k) => caches.delete(k)))).then(() => self.clients.claim()));
});

// Réseau d'abord pour la coquille (mise à jour immédiate), cache si hors ligne ; l'API n'est jamais mise en cache.
self.addEventListener('fetch', (e) => {
  const url = new URL(e.request.url);
  if (e.request.method !== 'GET' || url.origin !== location.origin) return;
  // Images des fiches : nommées par leur contenu, donc jamais périmées (cache d'abord, lisibles hors ligne).
  if (url.pathname.includes('/api/') && url.searchParams.get('r') === 'file') {
    e.respondWith(caches.open(IMAGES).then((c) => c.match(e.request).then((hit) => hit || fetch(e.request).then((res) => {
      if (res.ok) c.put(e.request, res.clone());
      return res;
    }))));
    return;
  }
  if (url.pathname.includes('/api/')) return;
  e.respondWith(
    fetch(e.request)
      .then((res) => {
        const copy = res.clone();
        caches.open(CACHE).then((c) => c.put(e.request, copy));
        return res;
      })
      .catch(() => caches.match(e.request).then((r) => r || caches.match('index.html')))
  );
});
