// Chargeur des modules (modules/<id>/) : demande au noyau la liste des modules actifs (méthode modulesList), puis
// ajoute leurs feuilles de style et leurs scripts, dans l'ordre de la liste. Un module utilise l'API commune du HUD
// (J.cardHeader, J.widgets, J.CARD_NO, window.jarvisBridge) : il est chargé après tous les scripts du système.
// Module activé ou désactivé dans l'administration (événement modules_changed) : la page se recharge.
// La classe « mod-<id> » sur <html> indique qu'un module est actif (ex. barres de niveau de la card musique).
(function () {
  const J = (window.JARVIS = window.JARVIS || {});
  const bridge = window.jarvisBridge;
  J.modules = {};
  if (!bridge || !bridge.call) return;

  // es : module JavaScript (import / export), comme le scanner 3D du module Checkup.
  const loadScript = (src, es) => new Promise((resolve) => {
    const s = document.createElement('script');
    if (es) s.type = 'module';
    s.src = src;
    s.onload = () => resolve(true);
    s.onerror = () => { console.warn(`[modules] script introuvable : ${src}`); resolve(false); };
    document.body.appendChild(s);
  });
  const loadStyle = (href) => {
    const l = document.createElement('link');
    l.rel = 'stylesheet';
    l.href = href;
    document.head.appendChild(l);
  };

  let loaded = null; // ids des modules chargés
  async function load() {
    // Fenêtre ouverte avant que le noyau écoute (lancement) : l'appel est rejeté, on réessaie jusqu'à obtenir la liste.
    let res = null;
    for (let essai = 0; !res && essai < 60; essai++) {
      try { res = await bridge.call('modulesList'); } catch (e) {
        if (essai === 0) console.warn('[modules] liste indisponible, nouvelle tentative…', e);
        await new Promise((r) => setTimeout(r, 1000));
      }
    }
    const list = (res && Array.isArray(res.modules)) ? res.modules : [];
    for (const err of (res && res.erreurs) || []) console.warn(`[modules] ${err}`);
    const active = list.filter((m) => m.actif && (!m.noyau || m.lie));
    loaded = active.map((m) => m.id + ':' + (m.revision || '')).sort().join(',');
    for (const m of active) {
      // card.emplacement : card ouverte par le noyau (« note »…) ; card.cle : clé dans J.CARD_NO (« hw » pour Matériel).
      for (const c of [m.card, ...(m.autresCards || [])]) {
        if (c && Number.isFinite(c.numero) && J.setCardDefault) J.setCardDefault(c.emplacement || c.cle || m.id, c.numero);
      }
      document.documentElement.classList.add(`mod-${m.id}`);
      J.modules[m.id] = m;
      for (const href of m.styles || []) loadStyle(href);
    }
    for (const m of active) {
      for (const src of m.es || []) await loadScript(src, true);
      for (const src of m.scripts || []) await loadScript(src);
    }
    // Module actif ? (vrai tant que la liste n'est pas connue : rien ne disparaît au démarrage.)
    J.moduleActive = (id) => Boolean(J.modules[id]);
    // Card Conversation (dessinée par app.js) : module désactivé, les boutons du HUD (RANGER, PLACER…) l'ignorent.
    if (!J.modules.conversation && list.some((m) => m.id === 'conversation')) delete J.conversationCard;
    // inactifs : modules présents mais désactivés (leurs cards ne s'affichent pas : widgets.js › disabledKinds).
    document.dispatchEvent(new CustomEvent('jarvis:modules', { detail: { modules: active, inactifs: list.filter((m) => !active.includes(m)) } }));
  }

  bridge.onTaskEvent((ev) => {
    if (!ev) return;
    if (ev.type === 'card_expand' && ev.key) {
      const c = Boolean(ev.collapsed);
      if (ev.key === 'audio' && J.audioPanel) J.audioPanel.setCollapsed(c);
      else if ((ev.key === 'hw' || ev.key === 'materiel') && J.hwPanels) J.hwPanels.setAllCollapsed(c);
      else if (ev.key === 'conversation' && J.conversationCard) J.conversationCard.setCollapsed(c);
      else if (ev.key === 'actions' && J.actionsCard) J.actionsCard.setCollapsed(c);
      else {
        const el = document.getElementById(`mod-${ev.key}`);
        if (el) {
          if (c) el.classList.add('mod-collapsed');
          else { el.style.display = ''; el.removeAttribute('hidden'); el.classList.remove('mod-collapsed'); }
        }
      }
      return;
    }
    if (ev.type !== 'modules_changed' || loaded === null) return;
    bridge.call('modulesList').then((res) => {
      const ids = ((res && res.modules) || []).filter((m) => m.actif && (!m.noyau || m.lie)).map((m) => m.id + ':' + (m.revision || '')).sort().join(',');
      if (ids !== loaded) location.reload();
    }).catch(() => {});
  });

  load();
})();
