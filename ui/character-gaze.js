// Regard du personnage du cœur : il tourne la tête (et les yeux) vers la card qui retient l'attention.
//
// Qu'est-ce qui retient l'attention, par ordre de priorité :
//  1. la card que tu survoles (2,5 s après) ou sur laquelle tu cliques / que tu déplaces (7 s après) ;
//  2. une card qui vient de s'activer ou de s'ouvrir (6 s) ;
//  3. la card musique tant qu'une musique joue (regard plus discret, permanent).
// Cards prises en compte : les cards d'action autour du cœur (J.getActionCards, dessinées sur le canvas)
// et les widgets (J.getWidgetBoxes : Bureau, Terminal, mails…). Le rig du personnage (characters/rig.js)
// reçoit { x, y, w } : direction (−1 gauche … +1 droite, −1 haut … +1 bas) et poids (0 = regard devant).
(function () {
  const J = (window.JARVIS = window.JARVIS || {});

  const HOVER_MS = 2500;
  const CLICK_MS = 7000;
  const NEW_MS = 6000;
  const REACH_X = 420; // distance (px logiques) pour un regard à fond sur le côté
  const REACH_Y = 300;
  const DEAD_ZONE = 70; // card sous le personnage : il ne regarde pas

  let pointer = null; // { key, at, hard }
  const seenCards = new Map(); // type de card d'action -> dernier état
  const seenWidgets = new Set(); // clés de widgets déjà vus actifs
  let attention = null; // { key, until, w }
  const startedAt = performance.now();

  const clamp = (v, a, b) => Math.max(a, Math.min(b, v));
  const now = () => performance.now();

  function widgetBoxes() {
    try {
      return J.getWidgetBoxes ? Object.values(J.getWidgetBoxes()) : [];
    } catch {
      return [];
    }
  }

  function center(b) {
    return { x: b.x + b.w / 2, y: b.y + b.h / 2 };
  }

  // --- Pointeur sur les widgets (DOM) --------------------------------------------------------------
  function onPointer(e, hard) {
    if (!e.target || !e.target.closest) return;
    const hit = widgetBoxes().find((b) => b.el && b.el.contains(e.target));
    if (!hit) return;
    pointer = { key: `w:${hit.key}`, at: now(), hard: hard || (pointer && pointer.key === `w:${hit.key}` && pointer.hard) };
  }
  document.addEventListener('pointerover', (e) => onPointer(e, false), true);
  document.addEventListener('pointerdown', (e) => onPointer(e, true), true);

  // --- Cibles ----------------------------------------------------------------------------------------
  function targets() {
    const out = new Map(); // key -> { c: {x, y}, kind }
    if (J.getActionCards) {
      for (const card of J.getActionCards()) {
        if (!card.panelBox || (J.cardVisible && !J.cardVisible(card))) continue;
        out.set(`a:${card.type}`, { c: center(card.panelBox), card });
      }
    }
    for (const b of widgetBoxes()) {
      if (b.state === 'active' && b.w > 0) out.set(`w:${b.key}`, { c: center(b), widget: b });
    }
    return out;
  }

  function musicPlaying() {
    const m = J.currentMedia;
    return Boolean(m && m.active && m.status === 'playing');
  }

  // Met à jour l'attention : renvoie { key, w } ou null.
  function choose(t, cand) {
    // 1. Pointeur : widgets survolés / cliqués, cards d'action survolées ou déplacées.
    let best = null;
    if (pointer && cand.has(pointer.key) && t - pointer.at < (pointer.hard ? CLICK_MS : HOVER_MS)) {
      best = { key: pointer.key, until: pointer.at + (pointer.hard ? CLICK_MS : HOVER_MS), w: 1 };
    }
    for (const [key, v] of cand) {
      if (!v.card) continue;
      const prev = seenCards.get(v.card.type);
      seenCards.set(v.card.type, v.card.state);
      if (v.card.isHovered || v.card.isDragging) {
        pointer = { key, at: t, hard: Boolean(v.card.isDragging) || (pointer && pointer.key === key && pointer.hard) };
        best = { key, until: t + (pointer.hard ? CLICK_MS : HOVER_MS), w: 1 };
      } else if (prev !== undefined && prev !== v.card.state && (v.card.state === 'activating' || v.card.state === 'active') && prev !== 'activating') {
        // Nouvelle activation : on y jette un œil quelques secondes.
        if (!best) attention = { key, until: t + NEW_MS, w: 0.9 };
      }
    }
    // Widget qui vient de s'ouvrir.
    for (const [key, v] of cand) {
      if (!v.widget) continue;
      if (!seenWidgets.has(key)) {
        seenWidgets.add(key);
        // Au démarrage, les widgets déjà là ne comptent pas comme des nouveautés.
        if (t - startedAt > 5000 && !best && !(attention && attention.until > t)) attention = { key, until: t + NEW_MS, w: 0.9 };
      }
    }
    for (const k of [...seenWidgets]) if (!cand.has(k)) seenWidgets.delete(k);
    if (best) {
      attention = best;
      return best;
    }
    if (attention && attention.until > t && cand.has(attention.key)) return attention;
    // 3. Musique en cours : regard discret et permanent vers la card musique.
    if (musicPlaying()) {
      const key = cand.has('a:music') ? 'a:music' : cand.has('w:music') ? 'w:music' : null;
      if (key) return { key, until: t + 1000, w: 0.7 };
    }
    return null;
  }

  // Direction du regard depuis le centre (cx, cy) du cœur, en pixels logiques du HUD.
  function get(cx, cy) {
    const t = now();
    const cand = targets();
    const pick = choose(t, cand);
    if (!pick) return null;
    const v = cand.get(pick.key);
    if (!v) return null;
    const dx = v.c.x - cx;
    const dy = v.c.y - cy;
    if (Math.hypot(dx, dy) < DEAD_ZONE) return null;
    const ax = Math.pow(Math.min(1, Math.abs(dx) / REACH_X), 0.8);
    const ay = Math.min(1, Math.abs(dy) / REACH_Y);
    return { x: Math.sign(dx) * ax, y: Math.sign(dy) * ay, w: pick.w, key: pick.key };
  }

  J.characterGaze = { get, clamp };
})();
