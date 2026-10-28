// Numéro de version de Jarvis, en bas du HUD. Un clic ouvre l'historique des modifications
// (ui/changelog.html) dans le navigateur. La version vient de ui/changelog.json, via le noyau.
(function () {
  const bridge = window.jarvisBridge;
  if (!bridge || !bridge.appVersion) return;

  const el = document.createElement('button');
  el.id = 'version-badge';
  el.type = 'button';
  el.className = 'hidden';
  document.body.appendChild(el);

  function load() {
    bridge.appVersion().then((v) => {
      if (!v || !v.version) { el.className = 'hidden'; return; }
      el.textContent = `v${v.version}`;
      el.title = `Jarvis ${v.version}${v.title ? ` — ${v.title}` : ''}${v.date ? ` (${v.date})` : ''}\nCliquer pour voir l'historique des modifications`;
      el.className = '';
    }).catch(() => {});
  }

  el.addEventListener('click', () => { bridge.openChangelog().catch(() => {}); });

  load();
  // Le noyau (re)démarre avec la coquille : la version est relue quand il revient.
  if (bridge.onTaskEvent) bridge.onTaskEvent((ev) => { if (ev && ev.type === 'desk_config') load(); });
})();
