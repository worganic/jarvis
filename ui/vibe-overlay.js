// Overlay d'alignement visuel Vibe Coding : surligne en direct l'élément ciblé à l'écran
// avec un cadre lumineux néon cyan/émeraude et un badge d'identification.
(function () {
  const bridge = window.jarvisBridge;
  if (!bridge || typeof bridge.onEvent !== 'function') return;

  let overlay = null;
  let clearTimer = null;

  function ensureOverlay() {
    if (overlay) return overlay;
    overlay = document.createElement('div');
    overlay.id = 'vibe-highlight-box';
    overlay.style.position = 'fixed';
    overlay.style.pointerEvents = 'none';
    overlay.style.zIndex = '999999';
    overlay.style.border = '2px solid #00f0ff';
    overlay.style.borderRadius = '6px';
    overlay.style.boxShadow = '0 0 15px rgba(0, 240, 255, 0.6), inset 0 0 10px rgba(0, 240, 255, 0.3)';
    overlay.style.transition = 'all 0.2s ease-out';
    overlay.style.display = 'none';

    const badge = document.createElement('div');
    badge.className = 'vibe-badge';
    badge.style.position = 'absolute';
    badge.style.top = '-26px';
    badge.style.left = '0px';
    badge.style.background = 'rgba(0, 20, 30, 0.9)';
    badge.style.color = '#00f0ff';
    badge.style.border = '1px solid #00f0ff';
    badge.style.borderRadius = '4px';
    badge.style.padding = '2px 8px';
    badge.style.fontSize = '12px';
    badge.style.fontWeight = 'bold';
    badge.style.letterSpacing = '0.5px';
    badge.style.whiteSpace = 'nowrap';
    badge.textContent = '🎯 Cible Vibe';
    overlay.appendChild(badge);

    document.body.appendChild(overlay);
    return overlay;
  }

  function highlight(rect, label, ttl = 30000) {
    const el = ensureOverlay();
    clearTimeout(clearTimer);

    if (!rect || rect.width <= 0 || rect.height <= 0) return;

    el.style.left = `${rect.x}px`;
    el.style.top = `${rect.y}px`;
    el.style.width = `${rect.width}px`;
    el.style.height = `${rect.height}px`;

    const badge = el.querySelector('.vibe-badge');
    if (badge) badge.textContent = `🎯 ${label || 'Composant'}`;

    el.style.display = 'block';
    el.style.opacity = '1';

    clearTimer = setTimeout(clear, ttl);
  }

  function clear() {
    clearTimeout(clearTimer);
    if (!overlay) return;
    overlay.style.opacity = '0';
    setTimeout(() => {
      if (overlay) overlay.style.display = 'none';
    }, 200);
  }

  bridge.onEvent((ev) => {
    if (!ev) return;
    if (ev.type === 'vibe_highlight') {
      highlight(ev.rect, ev.label, ev.ttl || 30000);
    } else if (ev.type === 'vibe_clear') {
      clear();
    }
  });
})();
