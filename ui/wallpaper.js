// Bureau JARVIS, réglage « Fond d'écran de Windows » (Administration › Bureau JARVIS) : l'image de fond
// de Windows remplace le fond noir, sous les effets, les cards et le cœur. Les icônes du bureau restent
// cachées : la fenêtre reste opaque, c'est l'image elle-même que la coquille relit (desk.rs › desk_wallpaper).
// Page « desk » (HUD) et pages « desk-bg-N » (autres écrans) seulement. Relu au changement de réglage
// (jarvis:wallpaper), quand la page redevient visible et toutes les 20 s (diaporama, Windows à la une).

(function () {
  const cfg = window.__JARVIS__;
  const tauri = window.__TAURI__;
  const label = cfg && cfg.label ? String(cfg.label) : '';
  if (!tauri || !tauri.core || !(label === 'desk' || label.startsWith('desk-bg-'))) return;

  const J = (window.JARVIS = window.JARVIS || {});
  J.wallpaperOn = false; // lu par app.js : plus de fond plein sous le canvas
  const FITS = {
    cover: ['cover', 'no-repeat', 'center'],
    contain: ['contain', 'no-repeat', 'center'],
    stretch: ['100% 100%', 'no-repeat', 'center'],
    center: ['auto', 'no-repeat', 'center'],
    tile: ['auto', 'repeat', 'left top']
  };
  let layer = null;
  let veil = null;
  let sig = '';
  let busy = false;

  function ensureLayer() {
    if (layer) return;
    layer = document.createElement('div');
    layer.id = 'wallpaper';
    layer.style.cssText = 'position:fixed;inset:0;z-index:-1;pointer-events:none;';
    veil = document.createElement('div');
    veil.style.cssText = 'position:absolute;inset:0;';
    layer.appendChild(veil);
    document.body.prepend(layer);
  }

  function apply(res) {
    const on = Boolean(res && res.on);
    J.wallpaperOn = on;
    document.documentElement.classList.toggle('wallpaper-on', on);
    if (!on) {
      if (layer) layer.remove();
      layer = veil = null;
      sig = '';
      return;
    }
    ensureLayer();
    const [size, repeat, pos] = FITS[res.fit] || FITS.cover;
    layer.style.backgroundColor = res.color || '#000';
    layer.style.backgroundSize = size;
    layer.style.backgroundRepeat = repeat;
    layer.style.backgroundPosition = pos;
    if (res.url) layer.style.backgroundImage = `url("${res.url}")`;
    else if (!res.sig) layer.style.backgroundImage = 'none';
    sig = res.sig || '';
    let theme = 'cyan';
    try { theme = localStorage.getItem('jarvis.theme') || 'cyan'; } catch {}
    const t = J.THEMES && (J.THEMES[theme] || J.THEMES.cyan);
    veil.style.background = (t && t.bgHex) || '#00040a'; // voile teinté par le thème
    veil.style.opacity = String(Math.max(0, Math.min(0.9, Number(res.dim) || 0)));
  }

  function refresh() {
    if (busy) return;
    busy = true;
    tauri.core.invoke('desk_wallpaper', { sig: sig || null })
      .then(apply)
      .catch(() => {})
      .finally(() => { busy = false; });
  }

  const style = document.createElement('style');
  style.textContent = 'html.wallpaper-on, html.wallpaper-on body { background-color: transparent !important; }';
  document.head.appendChild(style);

  if (tauri.event) tauri.event.listen('jarvis:wallpaper', () => { sig = ''; refresh(); });
  document.addEventListener('visibilitychange', () => { if (!document.hidden) refresh(); });
  setInterval(() => { if (!document.hidden) refresh(); }, 20000);
  if (document.body) refresh();
  else document.addEventListener('DOMContentLoaded', refresh);
})();
