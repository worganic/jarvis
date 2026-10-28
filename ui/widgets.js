// Widgets d'outils du HUD (chantier v3, étape 7 & v3-14) : musique, mail, agenda, connecteurs.
// Mode fantôme permanent en fond transparent, activation avec effet d'écriture, estompage lent,
// liaisons avec les cards d'action et déplacement libre à la souris pour toutes les cartes.
(function () {
  const J = (window.JARVIS = window.JARVIS || {});
  const bridge = window.jarvisBridge;
  const root = document.getElementById('widgets');
  if (!root) return;

  const cards = new Map(); // clé -> { el, timer, kind, state, fadeTimer }
  let eventNo = 0; // numéro de card de l'événement en cours de rendu (normalizeHead)

  const ICONS = { music: '♫', mail: '✉', agenda: '▦', browser: '◍', workspace: '▤', tool: '⬡' };
  const AUTO_HIDE = { mail: 60000, agenda: 60000, browser: 45000, workspace: 45000, tool: 45000 };

  function h(tag, attrs = {}, ...children) {
    const el = document.createElement(tag);
    for (const [k, v] of Object.entries(attrs || {})) {
      if (v === undefined || v === null || v === false) continue;
      if (k === 'class') el.className = v;
      else if (k.startsWith('on') && typeof v === 'function') el.addEventListener(k.slice(2), v);
      else el.setAttribute(k, v === true ? '' : v);
    }
    for (const child of children.flat()) {
      if (child === null || child === undefined || child === false) continue;
      const isNode = (typeof Node !== 'undefined' && child instanceof Node) || (child && typeof child === 'object' && child.nodeType);
      el.append(isNode ? child : document.createTextNode(String(child)));
    }
    return el;
  }

  function applyTheme(themeId) {
    const theme = J.THEMES && (J.THEMES[themeId] || J.THEMES.cyan);
    if (theme) root.style.setProperty('--w-rgb', theme.primaryRgb);
  }
  applyTheme(localStorage.getItem('jarvis.theme') || 'cyan');

  function fmtTime(ms) {
    if (!Number.isFinite(ms) || ms < 0) return '0:00';
    const s = Math.floor(ms / 1000);
    return `${Math.floor(s / 60)}:${String(s % 60).padStart(2, '0')}`;
  }

  function appName(app) {
    const a = String(app || '').toLowerCase();
    if (a.includes('spotify')) return 'SPOTIFY';
    if (a.includes('chrome')) return 'CHROME';
    if (a.includes('msedge') || a.includes('edge')) return 'EDGE';
    if (a.includes('firefox')) return 'FIREFOX';
    if (a.includes('zune') || a.includes('media')) return 'LECTEUR';
    const base = a.split(/[\\/!]/).filter(Boolean).pop() || '';
    return base.replace(/\.exe$/, '').split('.').pop().toUpperCase().slice(0, 14);
  }

  const SPOTIFY_SVG = '<svg class="w-source-icon" viewBox="0 0 24 24" width="12" height="12"><circle cx="12" cy="12" r="12" fill="#1db954"/><path d="M17.5 10.8C14 8.7 8.2 8.5 4.8 9.5c-.6.2-1.1-.2-1.3-.7-.2-.6.2-1.1.7-1.3 4-1.2 10.5-1 14.6 1.4.5.3.7 1 .4 1.5-.3.5-1 .6-1.7.4zm-.2 2.9c-.3.4-.8.5-1.2.3-3-1.8-7.5-2.4-11-1.3-.5.2-1-.2-1.1-.6-.2-.5.2-1 .6-1.1 4.1-1.2 9.2-.6 12.5 1.5.4.2.5.7.2 1.2zm-1.4 2.8c-.2.3-.6.4-.9.2-2.6-1.6-5.8-2-9.6-1.1-.4.1-.7-.1-.8-.5-.1-.4.1-.7.5-.8 4.2-1 7.8-.5 10.6 1.2.4.3.4.7.2 1z" fill="#121212"/></svg>';

  function updateSourceBadge(el, app) {
    if (!el) return;
    const name = appName(app);
    if (!name) {
      el.style.display = 'none';
      el.innerHTML = '';
      return;
    }
    el.style.display = 'inline-flex';
    const isSpotify = name === 'SPOTIFY';
    el.className = 'w-source' + (isSpotify ? ' spotify' : '');
    if (isSpotify) {
      el.innerHTML = `${SPOTIFY_SVG}<span>SPOTIFY</span>`;
    } else {
      el.textContent = name;
    }
  }

  // Card entièrement visible dans la fenêtre : recalée à l'intérieur (marges), et, plus haute que la
  // fenêtre, limitée à sa hauteur (sa liste défile, l'en-tête reste visible — voir style.css). Appelé
  // au placement, quand la card change de taille, quand la fenêtre change de taille et après un
  // glisser-déposer. La position enregistrée n'est pas modifiée : elle resservira dans une fenêtre
  // plus grande.
  const VIEW_MARGIN = 12;
  function keepInView(el) {
    if (!el || !el.isConnected || el.classList.contains('is-dragging')) return;
    const vw = window.innerWidth;
    const vh = window.innerHeight;
    // Card Bureau : 60 % de la hauteur de l'écran, la même en miniature et agrandie.
    const compact = el.classList.contains('w-compact');
    const cap = parseFloat(el.dataset.capH) || 0; // hauteur réduite par autoPlace pour tenir dans une place libre
    const maxH = compact ? `${Math.round(vh * 0.6)}px` : `${Math.max(120, cap ? Math.min(cap, vh - VIEW_MARGIN * 2) : vh - VIEW_MARGIN * 2)}px`;
    if (el.style.maxHeight !== maxH) el.style.maxHeight = maxH;
    if (compact && !el.classList.contains('w-collapsed') && el.style.height !== maxH) el.style.height = maxH;
    if (compact && el.classList.contains('w-collapsed') && el.style.height) el.style.height = '';
    const w = el.offsetWidth;
    const hgt = el.offsetHeight;
    const x = parseFloat(el.style.left) || 0;
    const y = parseFloat(el.style.top) || 0;
    const nx = Math.min(Math.max(VIEW_MARGIN, x), Math.max(VIEW_MARGIN, vw - w - VIEW_MARGIN));
    const ny = Math.min(Math.max(VIEW_MARGIN, y), Math.max(VIEW_MARGIN, vh - hgt - VIEW_MARGIN));
    if (Math.abs(nx - x) > 0.5) el.style.left = `${nx}px`;
    if (Math.abs(ny - y) > 0.5) el.style.top = `${ny}px`;
    const rec = [...cards.values()].find((c) => c.el === el);
    if (rec) rec._cachedBox = null;
  }
  // Une card qui grandit après coup (liste chargée, texte ajouté) est recalée aussi.
  const sizeWatch = typeof ResizeObserver === 'function'
    ? new ResizeObserver((entries) => { for (const e of entries) keepInView(e.target); })
    : null;

  // Card qui s'ouvre sans emplacement choisi (ni déplacée, ni fixée par 📍) : elle prend la place libre la plus proche de son
  // emplacement par défaut. Sans place libre, elle se pose là où elle recouvre le moins de cartes et, surtout, ne cache le
  // titre d'aucune autre card. Son emplacement n'est pas mémorisé : il est recalculé à chaque ouverture.
  function hasChosenPos(key) {
    try {
      if (localStorage.getItem((window.JARVIS_POS_PREFIX || 'jarvis.') + 'widgetPos.' + key)) return true;
    } catch {}
    return Boolean(defaultPos(key));
  }
  function autoPlace(el, key) {
    if (!el.isConnected || document.body.classList.contains('booting') || el.classList.contains('w-compact') || hasChosenPos(key)) return;
    const S = parseFloat(document.documentElement.style.getPropertyValue('--ui')) || 1;
    const vw = window.innerWidth - (J.hwPanels ? 224 * S : 0);
    const vh = window.innerHeight;
    const w = el.offsetWidth;
    const hgt = el.offsetHeight;
    if (!w || !hgt) return;
    const headOf = (e) => { const hd = e.querySelector('.w-head'); return (hd && hd.offsetHeight) || 44 * S; };
    const others = [...cards.values()].map((c) => c.el).filter((e) => e !== el && e.isConnected && e.offsetWidth > 0 && !e.classList.contains('w-compact')).map((e) => {
      const x = parseFloat(e.style.left) || e.offsetLeft;
      const y = parseFloat(e.style.top) || e.offsetTop;
      return { x, y, w: e.offsetWidth, h: e.offsetHeight, head: headOf(e) };
    });
    const inter = (ax, ay, aw, ah, b) => Math.max(0, Math.min(ax + aw, b.x + b.w) - Math.max(ax, b.x)) * Math.max(0, Math.min(ay + ah, b.y + b.h) - Math.max(ay, b.y));
    const score = (x, y, hh) => {
      let s = 0;
      for (const o of others) {
        s += inter(x, y, w, hh, o);
        if (inter(x, y, w, hh, { x: o.x, y: o.y, w: o.w, h: o.head })) s += 1e7; // cacherait le titre d'une autre card
      }
      return s;
    };
    const x0 = parseFloat(el.style.left) || 0;
    const y0 = parseFloat(el.style.top) || 0;
    const clampX = (x) => Math.min(Math.max(VIEW_MARGIN, x), Math.max(VIEW_MARGIN, vw - w - VIEW_MARGIN));
    const clampY = (y, hh) => Math.min(Math.max(VIEW_MARGIN, y), Math.max(VIEW_MARGIN, vh - hh - VIEW_MARGIN));
    // Meilleure place pour une card de hauteur hh : celle qui en recouvre le moins, la plus proche de l'emplacement par défaut.
    const search = (hh) => {
      let best = { x: clampX(x0), y: clampY(y0, hh), s: 0, dist: 0 };
      best.s = score(best.x, best.y, hh);
      if (best.s === 0) return best;
      best = { x: best.x, y: best.y, s: Infinity, dist: Infinity };
      const step = 24 * S;
      for (let y = VIEW_MARGIN; y <= Math.max(VIEW_MARGIN, vh - hh - VIEW_MARGIN); y += step) {
        for (let x = VIEW_MARGIN; x <= Math.max(VIEW_MARGIN, vw - w - VIEW_MARGIN); x += step) {
          const sc = score(x, y, hh);
          const dist = Math.hypot(x - x0, y - y0);
          if (sc < best.s || (sc === best.s && dist < best.dist)) best = { x, y, s: sc, dist };
        }
      }
      return best;
    };
    let best = search(hgt);
    // Pas de place pour la card entière : on la raccourcit (sa liste défile) pour qu'elle tienne dans un espace libre.
    if (best.s > 0) {
      for (const f of [0.8, 0.65, 0.5]) {
        const hh = Math.round(hgt * f);
        if (hh < 240 * S) break;
        const r = search(hh);
        if (r.s === 0) {
          best = r;
          el.dataset.capH = String(hh);
          el.style.maxHeight = `${hh}px`;
          break;
        }
      }
    }
    el.style.left = `${best.x}px`;
    el.style.top = `${best.y}px`;
  }

  // Positionnement par défaut harmonieux ou restauré depuis localStorage
  function getWidgetPos(key) {
    const S = parseFloat(document.documentElement.style.getPropertyValue('--ui')) || 1;
    // La colonne des cards matériel (hw-panels.js) occupe le bord droit : les widgets se placent avant elle.
    const W = window.innerWidth - (J.hwPanels ? 224 * S : 0);
    const wWidth = 340 * S;
    const marginX = 36 * S;
    const saved = localStorage.getItem((window.JARVIS_POS_PREFIX || 'jarvis.') + 'widgetPos.' + key);
    if (saved) {
      try {
        const p = JSON.parse(saved);
        if (Number.isFinite(p.x) && Number.isFinite(p.y)) return p;
      } catch {}
    }
    const dflt = defaultPos(key); // emplacement fixé par le bouton 📍 de la card
    if (dflt) return dflt;
    // Listes à fiches (module Listes, modules/listes/ui/listes.js) : leur propre emplacement par défaut.
    if (J.collectionsUI && J.collectionsUI.pos) {
      const p = J.collectionsUI.pos(key, S, W);
      if (p) return p;
    }
    switch (key) {
      case 'mail':
        return { x: W - wWidth - marginX, y: 92 * S };
      case 'agenda':
        return { x: W - wWidth - marginX, y: 390 * S };
      case 'music':
        return { x: Math.max(20, W - (wWidth * 2) - marginX * 1.6), y: 92 * S };
      case 'browser':
        return { x: Math.max(20, W - (wWidth * 2) - marginX * 1.6), y: 390 * S };
      case 'note':
        return { x: Math.max(20, W - 480 * S - wWidth - marginX * 1.6), y: 92 * S };
      case 'mail_view':
        return { x: Math.max(20, W - 460 * S - wWidth - marginX * 1.6), y: 120 * S };
      case 'meeting':
        return { x: Math.max(20, W - 520 * S - wWidth - marginX * 1.6), y: 72 * S };
      case 'tool:Devant':
      case 'devant':
        return { x: 20 * S, y: 120 * S };
      case 'tool':
      default:
        return { x: W - wWidth - marginX, y: 640 * S };
    }
  }

  // Déplacement universel fluide à la souris (Drag & Drop)
  function makeDraggable(el, key) {
    let isDragging = false;
    let startX = 0, startY = 0;
    let initialLeft = 0, initialTop = 0;

    // Le pointeur n'est capturé qu'une fois la souris vraiment déplacée : capturé dès l'appui, le navigateur
    // enverrait le « click » à la card entière et les lignes cliquables (div) ne le recevraient jamais.
    const DRAG_THRESHOLD = 5;
    let pending = null;

    const beginDrag = (e, from) => {
      isDragging = true;
      el.classList.add('is-dragging'); if (J.cardFront && J.cardFront.front) J.cardFront.front(el);
      delete el.dataset.capH;
      startX = from.x;
      startY = from.y;
      initialLeft = parseFloat(el.style.left) || el.offsetLeft;
      initialTop = parseFloat(el.style.top) || el.offsetTop;
      // Décalage automatique en cours (placement sans chevauchement, app.js) : intégré à la
      // position, le widget part de là où on le voit.
      const rec = cards.get(key);
      if (rec && (rec.layoutX || rec.layoutY)) {
        const S = parseFloat(document.documentElement.style.getPropertyValue('--ui')) || 1;
        initialLeft += (rec.layoutX || 0) * S;
        initialTop += (rec.layoutY || 0) * S;
        el.style.left = `${initialLeft}px`;
        el.style.top = `${initialTop}px`;
        rec.layoutX = rec.layoutY = rec.layoutTX = rec.layoutTY = 0;
        el.style.translate = '';
        rec._cachedBox = null;
      }
      try { el.setPointerCapture(from.id); } catch {}
      onPointerMove(e);
    };

    const stopPending = () => {
      pending = null;
      document.removeEventListener('pointermove', onPendingMove);
      document.removeEventListener('pointerup', stopPending);
      document.removeEventListener('pointercancel', stopPending);
    };

    function onPendingMove(e) {
      if (!pending || e.pointerId !== pending.id) return;
      if (Math.hypot(e.clientX - pending.x, e.clientY - pending.y) <= DRAG_THRESHOLD) return;
      const from = pending;
      stopPending();
      beginDrag(e, from);
    }

    const onPointerDown = (e) => {
      // Ne pas initier de drag si clic sur un bouton interactif
      if (e.target.closest('button, input, textarea, select, label, a, img, .w-close, .w-btn, .w-mail-body')) return;
      if (el.classList.contains('w-pinned')) return; // card figée : elle ne bouge plus
      if (J.cardFront && J.cardFront.front) J.cardFront.front(el); pending = { x: e.clientX, y: e.clientY, id: e.pointerId };
      document.addEventListener('pointermove', onPendingMove);
      document.addEventListener('pointerup', stopPending);
      document.addEventListener('pointercancel', stopPending);
      e.stopPropagation();
    };

    const onPointerMove = (e) => {
      if (!isDragging) return;
      const dx = e.clientX - startX;
      const dy = e.clientY - startY;
      const newX = Math.max(10, Math.min(window.innerWidth - el.offsetWidth - 10, initialLeft + dx));
      const newY = Math.max(40, Math.min(window.innerHeight - el.offsetHeight - 40, initialTop + dy));
      el.style.left = `${newX}px`;
      el.style.top = `${newY}px`;
      const card = cards.get(key);
      if (card) card._cachedBox = null;
    };

    const onPointerUp = (e) => {
      if (!isDragging) return;
      isDragging = false;
      el.classList.remove('is-dragging');
      try { el.releasePointerCapture(e.pointerId); } catch {}
      const curX = parseFloat(el.style.left) || el.offsetLeft;
      const curY = parseFloat(el.style.top) || el.offsetTop;
      localStorage.setItem((window.JARVIS_POS_PREFIX || 'jarvis.') + 'widgetPos.' + key, JSON.stringify({ x: curX, y: curY }));
      keepInView(el);
    };

    el.addEventListener('pointerdown', onPointerDown);
    el.addEventListener('pointermove', onPointerMove);
    el.addEventListener('pointerup', onPointerUp);
    el.addEventListener('pointercancel', onPointerUp);
  }

  // Effet d'écriture progressive (typewriter holographique)
  function typewrite(element, text, speed = 22, onComplete) {
    if (!element || !text) return;
    element.textContent = '';
    element.classList.add('w-typing');
    let i = 0;
    const timer = setInterval(() => {
      if (i < text.length) {
        element.textContent += text.charAt(i);
        i++;
      } else {
        clearInterval(timer);
        setTimeout(() => element.classList.remove('w-typing'), 500);
        if (onComplete) onComplete();
      }
    }, speed);
  }

  // Cards de type 2 toujours affichées : fermées, elles restent en mode inactif (fantôme) à leur place ;
  // la card revient à cet endroit quand elle est de nouveau utilisée (place() réveille le fantôme).
  const GHOST_KEYS = new Set(); // rempli par les modules (K.ghostKey : card Terminal…)
  function ghostStub(key, from) {
    const title = from.querySelector('.w-head .w-title');
    const stub = h('div', { class: 'widget w-tool w-ghost' },
      h('div', { class: 'w-head' }, gripHandle(), h('span', { class: 'w-icon' }, ICONS.tool), h('span', { class: 'w-title' }, title ? title.textContent : String(key).replace(/^tool:/, ''))));
    stub.style.left = from.style.left;
    stub.style.top = from.style.top;
    makeDraggable(stub, key);
    return stub;
  }

  // Estompage fluide vers le mode fantôme transparent
  // --- Registre des modules (modules/<id>/ui) -----------------------------------------------------------
  // Un module déclare l'affichage de ses cards (kind de l'événement « widget »), les événements qu'il écoute et ce
  // qu'il faut faire quand sa card se ferme. Un événement « widget » arrivé avant le chargement des modules attend ;
  // celui d'un module désactivé est ignoré (fiche › widgets).
  const renderers = new Map(); // kind → fn(ev)
  const toolRenderers = new Map(); // drapeau d'une card « tool » (collection, checkup…) → fn(ev) ; fiche : « tool.<drapeau> »
  const evHandlers = new Map(); // type d'événement → [fn(ev)]
  const closeHooks = new Map(); // clé de card → fn()
  const vanishKeys = new Set(['note', 'meeting', 'mail_view', 'product_view']); // cards qui disparaissent à la fermeture (pas de fantôme)
  const disabledKinds = new Set(); // kinds des modules désactivés
  let modulesReady = false;
  const pendingWidgets = [];
  function registerRenderer(kind, fn) { renderers.set(kind, fn); }
  function registerTool(flag, fn) { toolRenderers.set(flag, fn); }
  function onEvent(type, fn) {
    if (!evHandlers.has(type)) evHandlers.set(type, []);
    evHandlers.get(type).push(fn);
  }
  function onClose(key, fn) { closeHooks.set(key, fn); }
  // guardClose : vrai = la card fermée reste visible et active (musique en lecture) ; ownKey : card numérotée par le
  // HUD (J.CARD_NO), pas par le noyau ; onIdle : { busy() → vrai = jamais inactive, idle() → au lieu de l'estomper }.
  const closeGuards = new Map();
  const ownKeys = new Set();
  const idleHooks = new Map();
  // Décorateurs de la card de liste (renderRows) : fn(ev, el, key) → élément(s) ajoutés dans l'en-tête (après le titre)
  // ou au pied de la card ; null si la card ne les concerne pas.
  const headDecorators = [];
  const noAppClick = new Set(); // valeurs de ev.tool dont la card ne s'affiche pas d'un clic
  const footDecorators = [];
  const decorate = (list, ev, el, key) => list.flatMap((fn) => { try { const r = fn(ev, el, key); return Array.isArray(r) ? r : [r]; } catch (e) { console.error(e); return []; } }).filter(Boolean);

  function close(key) {
    const card = cards.get(key);
    if (!card) return;
    // Numéro de card rendu au cœur (« ferme la card 2 » ne la trouvera plus).
    if (!ownKeys.has(key) && card.state === 'active' && bridge && bridge.cardClosed) bridge.cardClosed(key);
    card._cachedBox = null;
    clearTimeout(card.timer);
    clearTimeout(card.fadeTimer);
    card.el.classList.remove('w-blink'); // la card qui se ferme ne clignote plus

    // Card de liste ou d'action (clé « tool:… ») ou de note : elle disparaît vraiment. Seuls les
    // widgets permanents (musique, mails, agenda…) restent en mode fantôme.
    const hook = closeHooks.get(key);
    if (hook) { try { hook(); } catch (e) { console.error(e); } }
    if (GHOST_KEYS.has(key) && card.el.classList.contains('w-foldable')) { // type 2 permanente : reste affichée, inactive
      if (sizeWatch) sizeWatch.unobserve(card.el);
      const stub = ghostStub(key, card.el);
      card.el.replaceWith(stub);
      card.el = stub;
      card.state = 'ghost';
      card._cachedBox = null;
      return;
    }
    if (String(key).startsWith('tool:') || vanishKeys.has(key)) {
      card.state = 'leaving';
      card.el.classList.remove('w-active');
      card.el.classList.add('w-leaving');
      card.fadeTimer = setTimeout(() => {
        if (sizeWatch) sizeWatch.unobserve(card.el);
        card.el.remove();
        if (cards.get(key) === card) cards.delete(key);
      }, 420);
      return;
    }

    // Card gardée par son module (musique en lecture) : fermée mais active, donc visible (pas d'estompage).
    const guard = closeGuards.get(key);
    if (guard && guard()) return;

    // Transition en mode fading : le fond devient transparent, puis les textes secondaires s'effacent
    card.state = 'fading';
    card.el.classList.remove('w-active');
    card.el.classList.add('w-fading');

    card.fadeTimer = setTimeout(() => {
      card.state = 'ghost';
      card.el.classList.remove('w-fading');
      card.el.classList.add('w-ghost');
    }, 6000); // 6 secondes pour finaliser l'estompage
  }

  function gripHandle() {
    return h('div', { class: 'w-grip', title: 'Déplacer le widget' },
      h('div', { class: 'w-grip-row' }, h('div', { class: 'w-grip-dot' }), h('div', { class: 'w-grip-dot' })),
      h('div', { class: 'w-grip-row' }, h('div', { class: 'w-grip-dot' }), h('div', { class: 'w-grip-dot' })));
  }

  // Numéro de la card, à citer pour la fermer à la voix (« ferme la card 2 »).
  function numBadge(ev) {
    if (ev && ev.compact && J.CARD_NO) return h('span', { class: 'w-num', 'data-cardno': 'bureau', title: `Card n° ${J.CARD_NO.bureau} (type 1, toujours active)` }, String(J.CARD_NO.bureau)); // Bureau : type 1
    return ev && ev.cardNo ? h('span', { class: 'w-num', title: `Card n° ${ev.cardNo} : « ferme la card ${ev.cardNo - 100} »` }, String(ev.cardNo)) : null;
  }

  // × d'une card : si elle représente une application, la fermer aussi (le cœur demande l'accord).
  // Card verrouillée (ev.locked : terminal, tmux, Claude ouvert) : pas de × — elle part avec l'application.
  function closeButton(ev, key) {
    if (ev && ev.locked) {
      return h('span', { class: 'w-close w-lock', title: `Verrouillée : se ferme avec ${ev.appLabel || 'l’application'} (✕ sur la ligne pour fermer les deux)` }, '🔒');
    }
    const withApp = Boolean(ev && ev.appId && ev.cardNo && bridge && bridge.cardClose);
    return h('button', {
      class: 'w-close',
      title: withApp ? `Fermer la card et ${ev.appLabel || 'l’application'}` : 'Fermer',
      onclick: () => {
        if (withApp) bridge.cardClose(ev.cardNo);
        // Card d'une application qui tourne encore (persist) mais qu'on ne ferme pas : elle reste visible, réduite en bouton.
        else if (ev && ev.persist && cards.get(key) && cards.get(key).el.classList.contains('w-foldable')) { setFolded(key, true); return; }
        close(key);
      }
    }, '×');
  }

  // Clic sur une card liée à une application : l'afficher (pas après un glisser-déposer).
  function openOnClick(el, ev) {
    if (!ev || !ev.appId || !bridge || !bridge.cardAppOpen) return;
    el.classList.add('w-has-app');
    el.title = `Clic : afficher ${ev.appLabel || 'l’application'}`;
    let down = null;
    el.addEventListener('pointerdown', (e) => { down = { x: e.clientX, y: e.clientY }; });
    el.addEventListener('click', (e) => {
      if (el.classList.contains('w-collapsed')) return;
      if (e.target.closest('button, a, input, .w-row-app')) return;
      if (down && Math.hypot(e.clientX - down.x, e.clientY - down.y) > 5) return;
      bridge.cardAppOpen(ev.appId);
    });
  }

  function header(kind, title, source, key) {
    const sourceEl = h('span', { class: 'w-source' });
    if (source) updateSourceBadge(sourceEl, source);
    else sourceEl.style.display = 'none';
    return h('div', { class: 'w-head' },
      gripHandle(),
      h('span', { class: 'w-icon' }, ICONS[kind] || ICONS.tool),
      h('span', { class: 'w-title' }, title),
      sourceEl,
      h('button', { class: 'w-close', title: 'Fermer', onclick: () => close(key) }, '×'));
  }

  // Ouverture depuis le circuit imprimé du fond (circuit.js) : un éclair part du cœur le long des pistes
  // jusqu'au widget, qui se déplie depuis le bord tourné vers le cœur. Pas pendant l'allumage (sa propre cascade).
  function unfoldFromCircuit(el) {
    try {
      if (document.body && document.body.classList.contains('boot-widgets')) return;
      const r = el.getBoundingClientRect();
      if (!r.width || !r.height) return;
      const dx = r.left + r.width / 2 - window.innerWidth / 2;
      const dy = r.top + r.height / 2 - window.innerHeight / 2;
      const side = Math.abs(dx) >= Math.abs(dy) ? (dx > 0 ? 'l' : 'r') : (dy > 0 ? 't' : 'b');
      el.classList.add('w-unfold', `w-unfold-${side}`);
      setTimeout(() => el.classList.remove('w-unfold', `w-unfold-${side}`), 800);
      const S = parseFloat(document.documentElement.style.getPropertyValue('--ui')) || 1;
      if (J.flashToBox) J.flashToBox({ x: r.left / S, y: r.top / S, w: r.width / S, h: r.height / S });
    } catch { /* animation seulement */ }
  }

  function place(key, el, kind, ttl) {
    const existing = cards.get(key);
    const opening = !existing || existing.state !== 'active'; // nouvelle card, ou réveil d'un widget fantôme
    const pos = getWidgetPos(key);

    if (existing) {
      clearTimeout(existing.timer);
      clearTimeout(existing.fadeTimer);
      // Préserver la position actuelle
      el.style.left = existing.el.style.left || `${pos.x}px`;
      el.style.top = existing.el.style.top || `${pos.y}px`;
      if (existing.el.style.zIndex) el.style.zIndex = existing.el.style.zIndex; // card passée devant par un clic (card-front.js)
      existing.el.replaceWith(el);
    } else {
      el.style.left = `${pos.x}px`;
      el.style.top = `${pos.y}px`;
      root.appendChild(el);
    }

    makeDraggable(el, key);
    normalizeHead(key, el);
    el.classList.remove('w-ghost', 'w-fading');
    el.classList.add('w-active');
    keepInView(el);
    if (!existing) { try { autoPlace(el, key); } catch (e) { console.error(e); } }
    if (sizeWatch) {
      if (existing && existing.el !== el) sizeWatch.unobserve(existing.el);
      sizeWatch.observe(el);
    }

    const card = { el, kind, state: 'active', timer: null, fadeTimer: null, lastActive: performance.now(), persist: Boolean(existing && existing.persist) };
    if (ttl) card.timer = setTimeout(() => close(key), ttl);
    cards.set(key, card);
    if (opening) unfoldFromCircuit(el);
    // Card qui s'ouvre : devant toutes les autres (pas à chaque rafraîchissement d'une card déjà ouverte).
    if (opening && J.cardFront && J.cardFront.front) J.cardFront.front(el);
    // Card de type 2 (temporaire) qui s'ouvre : le cadre et le titre clignotent 5 secondes pour la repérer.
    if (opening && !document.body.classList.contains('booting') && !el.classList.contains('w-compact') && !el.classList.contains('w-ghost')) {
      el.classList.add('w-blink');
      setTimeout(() => el.classList.remove('w-blink'), 5000);
    }
  }

  // --- Mail, agenda, connecteurs -------------------------------------------------------------------

  function renderList(ev) {
    const key = `${ev.kind}`;
    const items = ev.items || [];
    const titleText = ev.title || (ev.kind === 'mail' ? 'Derniers emails' : 'Agenda du jour');
    const titleEl = h('span', { class: 'w-title' }, titleText);

    const listItems = items.map((it) => {
      const itTitle = h('div', { class: 'w-item-title' }, ev.kind === 'mail' ? it.subject : it.title);
      const itSub = h('div', { class: 'w-item-sub' }, ev.kind === 'mail'
        ? `${it.from}${it.date ? ` · ${String(it.date).slice(0, 22)}` : ''}`
        : [it.time || (it.start ? String(it.start).replace('T', ' ').slice(0, 16) : ''), it.place].filter(Boolean).join(' · '));
      const mailRef = ev.kind === 'mail' ? (it.uid || it.id) : null;
      return h('div', {
        class: `w-item${mailRef ? ' clickable' : ''}`,
        role: mailRef ? 'button' : undefined,
        tabindex: mailRef ? '0' : undefined,
        title: mailRef ? 'Clic : ouvrir ce mail' : undefined,
        onclick: mailRef && bridge && bridge.mailOpen ? () => bridge.mailOpen(mailRef) : undefined
      }, itTitle, itSub);
    });

    const el = h('div', { class: `widget w-${ev.kind}` },
      h('div', { class: 'w-head' },
        gripHandle(),
        h('span', { class: 'w-icon' }, ICONS[ev.kind] || ICONS.tool),
        titleEl,
        h('span', { class: 'w-source' }, ev.kind === 'mail' ? 'GMAIL' : 'AGENDA'),
        numBadge(ev),
        closeButton(ev, key)),
      items.length
        ? h('div', { class: 'w-list' }, ...listItems)
        : ev.error
          ? h('div', { class: 'w-error' }, ev.error, ev.setup ? h('div', { class: 'w-item-sub' }, `→ ${ev.setup}`) : null)
          : h('div', { class: 'w-empty' }, ev.kind === 'mail' ? 'Aucun email.' : 'Aucun rendez-vous.'));

    place(key, el, ev.kind, AUTO_HIDE[ev.kind]);
  }

  // Liste en lignes (projets, Claude Code ouverts, commandes d'aide, fichiers…) : chaque ligne a son
  // icône, un libellé, un détail et une info à droite ; `say` la rend cliquable (la phrase est
  // envoyée à Jarvis comme si on l'avait tapée). { section } : intertitre.
  // Un double-clic sur une ligne n'envoie la phrase qu'une fois.
  let lastRowSay = { say: '', at: 0 };
  function sayRow(say) {
    const now = Date.now();
    if (say === lastRowSay.say && now - lastRowSay.at < 1500) return;
    lastRowSay = { say, at: now };
    bridge.utterance(say, { from: 'widget' });
  }

  // Ligne avec un appel direct au noyau ({ method, args } : « mailOpen » pour lire un mail) plutôt
  // qu'une phrase envoyée à Jarvis. Même anti double-clic que sayRow.
  let lastRowCall = { key: '', at: 0 };
  function activateRow(r) {
    if (r.call && bridge && typeof bridge[r.call.method] === 'function') {
      const key = JSON.stringify(r.call);
      const now = Date.now();
      if (key === lastRowCall.key && now - lastRowCall.at < 1500) return;
      lastRowCall = { key, at: now };
      Promise.resolve(bridge[r.call.method](...(r.call.args || []))).catch(() => {});
      return;
    }
    if (r.say && bridge && bridge.utterance) sayRow(r.say);
  }

  // Couleur d'un terminal (celle de son onglet dans Windows Terminal) : bande et bordure de la ligne.
  const isHex = (c) => typeof c === 'string' && /^#[0-9a-f]{6}$/i.test(c);
  function accentCls(r) { return isHex(r.accent) ? ' w-accent' : ''; }
  function accentStyle(r) { return isHex(r.accent) ? `--accent: ${r.accent}` : null; }
  // Fenêtre à plusieurs onglets : une pastille par onglet, dans sa couleur.
  function accentDots(r) {
    if (!Array.isArray(r.accents) || r.accents.length < 2) return null;
    return h('span', { class: 'w-accent-dots', title: `${r.accents.length} onglets` },
      ...r.accents.filter(isHex).map((c) => h('span', { class: 'w-accent-dot', style: `background: ${c}` })));
  }

  // Mode mini (card Bureau au démarrage) : « numéro + 3 premières lettres », sans boutons ni sections.
  function miniRows(rows) {
    return h('div', { class: 'w-rows' }, ...rows.filter((r) => !r.section && /^\d+\.\s/.test(r.title || '')).map((r) => {
      const m = r.title.match(/^(\d+)\.\s*(.*)$/);
      const active = Boolean(r.call && bridge && typeof bridge[r.call.method] === 'function');
      return h('div', {
        class: `w-row w-row-mini${active ? ' clickable' : ''}${r.tone ? ` tone-${r.tone}` : ''}${accentCls(r)}`,
        style: accentStyle(r),
        title: `${m[2]}${r.sub ? ` · ${r.sub}` : ''}`,
        onclick: active ? () => activateRow(r) : undefined
      }, h('span', { class: 'w-mini-n' }, m[1]), h('span', { class: 'w-mini-t' }, m[2].slice(0, 3)));
    }));
  }

  // Pastille de la ligne : miniature (r.thumb, https seulement, images des marchands) ou icône ; si
  // l'image ne charge pas, l'icône reprend sa place.
  function rowIco(r) {
    const fallback = r.icon || '▸';
    if (typeof r.thumb !== 'string' || !/^https:\/\//i.test(r.thumb)) return h('span', { class: 'w-row-ico' }, fallback);
    const img = h('img', { class: 'w-row-img', src: r.thumb, alt: '', loading: 'lazy', referrerpolicy: 'no-referrer', draggable: 'false' });
    const ico = h('span', { class: 'w-row-ico w-row-thumb' }, img);
    img.addEventListener('error', () => { ico.classList.remove('w-row-thumb'); ico.textContent = fallback; });
    return ico;
  }

  // Emplacements de la card Bureau : quatre pastilles 1-4. Clic : y aller (ou enregistrer si vide) ; clic
  // droit : réenregistrer ; Maj+clic : effacer. hwnd = 0 : la disposition entière (toutes ses fenêtres).
  function slotButtons(hwnd, filled, what) {
    if (!bridge || !bridge.desktopAction) return null;
    const pre = hwnd ? 'slot' : 'layout';
    return h('span', { class: 'w-slots' }, ...filled.map((on, i) => {
      const n = i + 1;
      const run = (verb) => Promise.resolve(bridge.desktopAction(hwnd, `${verb}:${n}`)).catch(() => {});
      const btn = h('button', {
        class: `w-slot${on ? ' on' : ''}`,
        title: on
          ? `Emplacement ${n} : y placer ${what} · clic droit : réenregistrer ici · Maj+clic : effacer`
          : `Emplacement ${n} vide : clic pour y enregistrer ${what} telle${hwnd ? '' : 's'} qu’${hwnd ? 'elle est' : 'elles sont'}`,
        onclick: (e) => { e.stopPropagation(); run(e.shiftKey ? `${pre}_clear` : pre); }
      }, String(n));
      btn.addEventListener('contextmenu', (e) => { e.preventDefault(); e.stopPropagation(); run(`${pre}_save`); });
      return btn;
    }));
  }

  function rowsList(rows) {
    return h('div', { class: 'w-rows' }, ...rows.slice(0, 250).map((r) => {
      if (r.section) return h('div', { class: 'w-rows-section' }, r.section);
      if (Array.isArray(r.slotbar)) {
        return h('div', { class: 'w-slotbar' },
          h('span', { class: 'w-slotbar-label', title: 'Dispositions : place de toutes les fenêtres enregistrées' }, 'Dispositions'),
          slotButtons(0, r.slotbar.map((c) => c > 0), 'les fenêtres'));
      }
      // Mini-card horizontale (card Terminal : une par agent lancé) : icône, titre et détail, état en badge.
      if (r.card) {
        const go = Boolean(r.call && bridge && typeof bridge[r.call.method] === 'function');
        return h('div', { class: `w-row w-row-card${go ? ' clickable' : ''}${r.tone ? ` tone-${r.tone}` : ''}${accentCls(r)}`, style: accentStyle(r), title: r.hint || '', onclick: go ? () => activateRow(r) : undefined },
          rowIco(r),
          h('span', { class: 'w-row-text' },
            h('span', { class: 'w-row-title' }, r.title || ''),
            r.sub ? h('span', { class: 'w-row-sub' }, r.sub) : null),
          h('span', { class: 'w-row-side' },
            r.state ? h('span', { class: 'w-row-state' }, r.state) : null,
            r.meta ? h('span', { class: 'w-row-meta' }, r.meta) : null));
      }
      // Ligne de boutons d'action (terminal, Claude : « Valider », « Changer de modèle »…) : chaque bouton
      // appelle le noyau ({ call }) ; la ligne garde son titre et son détail (confirmation d'une action sensible).
      if (Array.isArray(r.chips) && r.chips.length) {
        return h('div', { class: `w-row w-row-chips${r.actionList ? ' w-actions-list' : ''}${r.tone ? ` tone-${r.tone}` : ''}` },
          r.sub ? h('div', { class: 'w-chips-note' }, h('strong', {}, r.title || ''), ` · ${r.sub}`) : null,
          h('div', { class: 'w-chips' }, ...r.chips.map((c) => h('button', {
            class: `w-chip${c.tone ? ` ${c.tone}` : ''}`, title: c.title || '',
            onclick: () => activateRow(c)
          }, c.icon ? `${c.icon} ${c.label}` : c.label))));
      }
      // Ligne liée à une application : clic = l'afficher ; petits boutons « parler » et « fermer ».
      if (r.appId && bridge && bridge.cardAppOpen) {
        return h('div', {
          class: `w-row clickable w-row-app${r.tone ? ` tone-${r.tone}` : ''}`,
          role: 'button', tabindex: '0',
          title: `Clic : afficher ${r.appLabel || r.title || 'l’application'}`,
          onclick: (e) => { if (!e.target.closest('.w-row-btn')) bridge.cardAppOpen(r.appId); }
        },
        h('span', { class: 'w-row-ico' }, r.icon || '▸'),
        h('span', { class: 'w-row-text' },
          h('span', { class: 'w-row-title' }, r.title || ''),
          r.sub ? h('span', { class: 'w-row-sub' }, r.sub) : null),
        r.meta ? h('span', { class: 'w-row-meta' }, r.meta) : null,
        h('span', { class: 'w-row-actions' },
          // Terminal ou Claude : dicter dedans (sans « Jarvis »), puis rendre la main à Jarvis.
          r.voice && bridge.cardAppAction ? h('button', {
            class: `w-row-btn${r.voice.active ? ' on' : ''}`,
            title: r.voice.active ? 'Dictée active : ce que tu dis est écrit ici' : 'Dicter ici : ce que tu dis sera écrit dans ce terminal, sans dire « Jarvis »',
            onclick: () => bridge.cardAppAction(r.appId, 'dicter')
          }, r.voice.active ? '🎙 Dictée' : '🎙 Dicter ici') : null,
          r.voice && r.voice.active && bridge.cardAppAction ? h('button', {
            class: 'w-row-btn', title: 'Jarvis reprend la main : il écoute de nouveau tes demandes',
            onclick: () => bridge.cardAppAction(r.appId, 'reprendre')
          }, '✋ Jarvis') : null,
          !r.voice && r.say && bridge.utterance ? h('button', { class: 'w-row-btn', title: `Dire : « ${r.say} »`, onclick: () => sayRow(r.say) }, r.sayLabel || '🎙') : null,
          bridge.cardAppClose ? h('button', { class: 'w-row-btn danger', title: `Fermer ${r.appLabel || 'l’application'}`, onclick: () => bridge.cardAppClose(r.appId) }, '✕') : null));
      }
      // Ligne avec des boutons d'action (🚫 « ne plus annoncer cet expéditeur », ↺ « réactiver »…) :
      // chaque bouton envoie sa phrase à Jarvis ; la ligne est alors marquée comme traitée.
      if (Array.isArray(r.actions) && r.actions.length && bridge) {
        const active = Boolean((r.call && typeof bridge[r.call.method] === 'function') || (r.say && bridge.utterance));
        const row = h('div', {
          class: `w-row${active ? ' clickable' : ''} w-row-app${r.tone ? ` tone-${r.tone}` : ''}${accentCls(r)}`,
          style: accentStyle(r),
          role: active ? 'button' : null, tabindex: active ? '0' : null,
          title: r.hint || (r.say ? `Dire : « ${r.say} »` : ''),
          onclick: (e) => { if (active && !e.target.closest('.w-row-btn') && !row.classList.contains('w-row-done')) activateRow(r); }
        },
        rowIco(r),
        h('span', { class: 'w-row-text' },
          h('span', { class: 'w-row-title' }, accentDots(r), r.title || ''),
          r.sub ? h('span', { class: 'w-row-sub' }, r.sub) : null),
        r.meta ? h('span', { class: 'w-row-meta' }, r.meta) : null,
        Array.isArray(r.slots) && r.hwnd ? slotButtons(r.hwnd, r.slots, 'cette fenêtre') : null,
        h('span', { class: 'w-row-actions' }, ...r.actions.map((a) => h('button', {
          class: `w-row-btn${a.tone ? ` ${a.tone}` : ''}`,
          title: a.title || `Dire : « ${a.say} »`,
          onclick: (e) => {
            e.stopPropagation();
            if (row.classList.contains('w-row-done')) return;
            activateRow(a); // appel direct (a.call : fenêtre, onglet…) ou phrase dite (a.say)
            // Action définitive (expéditeur exclu…) : la ligne est grisée ; sinon (▶ lancer, ☆ favori)
            // la ligne reste active — le noyau réaffiche la card si besoin.
            if (!a.done) return;
            row.classList.add('w-row-done');
            for (const b of row.querySelectorAll('.w-row-btn')) b.disabled = true;
            (row.querySelector('.w-row-sub') || row.querySelector('.w-row-title')).insertAdjacentText('beforeend', ` · ${a.done}`);
          }
        }, a.icon || '•'))));
        return row;
      }
      const clickable = Boolean((r.say && bridge && bridge.utterance) || (r.call && bridge && bridge[r.call.method]));
      return h(clickable ? 'button' : 'div', {
        class: `w-row${clickable ? ' clickable' : ''}${r.tone ? ` tone-${r.tone}` : ''}`,
        title: r.hint || (clickable && r.say ? `Dire : « ${r.say} »` : ''),
        onclick: clickable ? () => activateRow(r) : undefined
      },
      rowIco(r),
      h('span', { class: 'w-row-text' },
        h('span', { class: 'w-row-title' }, r.title || ''),
        r.sub ? h('span', { class: 'w-row-sub' }, r.sub) : null),
      r.meta ? h('span', { class: 'w-row-meta' }, r.meta) : null);
    }));
  }

  // Card de liste en lignes (même design partout : nouveaux mails, derniers mails, notes, projets…).
  // Cards figées (bouton 📌) : taille (mini → grande), place et durée de vie ne changent plus ;
  // gardé entre deux rafraîchissements de la card.
  const pinnedCards = new Set();
  // Card Bureau réductible en un seul bouton déplaçable (état mémorisé).
  let bureauCollapsed = false;
  try { bureauCollapsed = localStorage.getItem('jarvis.bureauCollapsed') === '1'; } catch {}
  function setBureauCollapsed(on) {
    bureauCollapsed = on;
    try { localStorage.setItem('jarvis.bureauCollapsed', on ? '1' : '0'); } catch {}
    for (const [k, rec] of cards) if (rec.el && rec.el.classList.contains('w-compact')) rec.el.classList.toggle('w-collapsed', on);
  }
  // Card Bureau : miniature (numéros des fenêtres) ou agrandie, au clic sur ⇔ (plus au survol). État mémorisé.
  let bureauExpanded = false;
  try { bureauExpanded = localStorage.getItem('jarvis.bureauExpanded') === '1'; } catch {}
  function expandButton() {
    return h('button', {
      class: 'w-expand', title: 'Agrandir / réduire en miniature',
      onclick: (e) => {
        e.stopPropagation();
        bureauExpanded = !bureauExpanded;
        try { localStorage.setItem('jarvis.bureauExpanded', bureauExpanded ? '1' : '0'); } catch {}
        for (const [, rec] of cards) if (rec.el && rec.el.classList.contains('w-compact')) rec.el.classList.toggle('w-expanded', bureauExpanded);
      }
    }, '⇔');
  }
  function collapseButton() {
    // stopPropagation : le clic remonterait à la card, déjà réduite, et expandOnTap la redéplierait aussitôt.
    return h('button', { class: 'w-collapse', title: 'Réduire en un bouton', onclick: (e) => { e.stopPropagation(); setBureauCollapsed(true); } }, '–');
  }
  function miniPill(label, ev) {
    // Le clic n'arrive pas ici : makeDraggable capture le pointeur sur la card entière, le navigateur
    // envoie donc le « click » à la card. Le dépliage est géré par expandOnTap (card réduite).
    const badge = ev && ev.badge ? ` ${ev.badge}` : '';
    const tone = ev && ev.badgeTone ? ` tone-${ev.badgeTone}` : '';
    return h('div', { class: `w-mini-pill${tone}`, title: 'Clic : afficher la card (glisser : déplacer)' }, `${ev && ev.foldable ? '▣' : '▤'} ${label}${badge}`);
  }
  // Card réduite : un clic (sans glisser) sur la pastille la déplie.
  function expandOnTap(el, onExpand) {
    let down = null;
    el.addEventListener('pointerdown', (e) => { down = { x: e.clientX, y: e.clientY }; });
    el.addEventListener('click', (e) => {
      if (!el.classList.contains('w-collapsed')) return;
      if (down && Math.hypot(e.clientX - down.x, e.clientY - down.y) > 5) return;
      onExpand();
    });
  }
  // Cards repliables en pastille (card Terminal…) : état mémorisé par card, comme la card Audio.
  const foldedCards = new Set();
  try { for (const k of JSON.parse(localStorage.getItem('jarvis.foldedCards') || '[]')) foldedCards.add(k); } catch {}
  function setFolded(key, on) {
    if (on) foldedCards.add(key); else foldedCards.delete(key);
    try { localStorage.setItem('jarvis.foldedCards', JSON.stringify([...foldedCards])); } catch {}
    const rec = cards.get(key);
    if (rec && rec.el) rec.el.classList.toggle('w-collapsed', on);
    if (!on && rec && rec.el && J.cardFront && J.cardFront.front) J.cardFront.front(rec.el); // dépliée : devant les autres
  }
  function foldButton(key) {
    return h('button', { class: 'w-collapse', title: 'Réduire en un bouton', onclick: (e) => { e.stopPropagation(); setFolded(key, true); } }, '–');
  }
  function pinButton(el, key) {
    const btn = h('button', { class: 'w-pin', title: 'Figer la card (taille, place, pas de fermeture automatique)' }, '📌');
    const apply = (on) => {
      if (on) pinnedCards.add(key); else pinnedCards.delete(key);
      el.classList.toggle('w-pinned', on);
      el.classList.toggle('w-locked', on);
      btn.classList.toggle('on', on);
      const rec = cards.get(key);
      if (rec) {
        clearTimeout(rec.timer);
        rec.timer = null;
      }
    };
    btn.onclick = () => apply(!pinnedCards.has(key));
    if (pinnedCards.has(key)) { el.classList.add('w-pinned', 'w-locked'); btn.classList.add('on'); }
    return btn;
  }
  function renderRows(ev, key, ttl) {
    // Cards de type 2 (temporaires : Terminal, mails, notes…) : toutes se réduisent en pastille violette, hors card Bureau (type 1, jaune).
    ev = { ...ev, foldable: Boolean(ev.foldable || !ev.compact) };
    const el = h('div', { class: `widget w-tool w-card-list${ev.kind === 'mail' ? ' w-mail' : ''}${ev.wide ? ' w-wide' : ''}${ev.compact ? ' w-compact' : ''}${ev.compact && bureauExpanded ? ' w-expanded' : ''}${ev.compact && bureauCollapsed ? ' w-collapsed' : ''}${ev.foldable ? ' w-foldable' : ''}${ev.foldable && foldedCards.has(key) ? ' w-collapsed' : ''}` });
    const appendAll = (...kids) => el.append(...kids.filter(Boolean));
    appendAll(
      h('div', { class: 'w-head' },
        gripHandle(),
        numBadge(ev),
        h('span', { class: 'w-title' }, ev.title || 'Liste'),
        ev.count !== undefined && !ev.cardNo ? h('span', { class: 'w-count' }, String(ev.count)) : null, // numérotée : le numéro suffit
        ev.compact ? expandButton() : null, // bouton propre à la card Bureau, après le titre
        ...decorate(headDecorators, ev, el, key), // boutons des modules (Terminal : 🎙 ☰)
        pinButton(el, key),
        ev.compact ? collapseButton() : (ev.foldable ? foldButton(key) : null),
        closeButton(ev, key)),
      ev.rows.length ? (ev.compact ? h('div', { class: 'w-dual' }, miniRows(ev.rows), rowsList(ev.rows)) : rowsList(ev.rows)) : h('div', { class: 'w-empty' }, ev.empty || 'Rien à afficher.'),
      ev.footer ? h('div', { class: 'w-rows-foot' }, ev.footer) : null,
      ...decorate(footDecorators, ev, el, key),
      ev.compact || ev.foldable ? miniPill(String(ev.title || 'Bureau').toUpperCase(), ev) : null);
    // Card dont les lignes désignent chacune une application (Terminal : une ligne par agent) : un clic sur le reste de
    // la card n'affiche rien (fiche du module : noAppClick).
    if (!noAppClick.has(ev.tool)) openOnClick(el, ev);
    // Card Bureau reconstruite à chaque changement du bureau : on reprend la position de défilement.
    let keepScroll = 0;
    if (ev.compact) {
      expandOnTap(el, () => setBureauCollapsed(false));
      const prev = cards.get(key);
      const old = prev && prev.el && prev.el.isConnected ? prev.el : null;
      if (old) {
        const list = old.querySelector('.w-dual');
        keepScroll = list ? list.scrollTop : 0;
      }
    }
    if (ev.foldable) expandOnTap(el, () => setFolded(key, false));
    place(key, el, ev.kind, ttl);
    const placed = cards.get(key);
    if (placed) placed.persist = Boolean(ev.persist || ev.locked); // card d'une application qui tourne : jamais effacée pour inactivité
    if (keepScroll) {
      const list = el.querySelector('.w-dual');
      if (list) list.scrollTop = keepScroll;
    }
    if (ev.compact) { // ancrée en haut à gauche, sur toute la hauteur disponible
      let saved = null;
      try { saved = localStorage.getItem((window.JARVIS_POS_PREFIX || 'jarvis.') + 'widgetPos.' + key); } catch {}
      if (!saved) {
        el.style.left = '6px';
        el.style.top = '120px';
      } // sous « J.A.R.V.I.S. » et ses lignes d'état (jusqu'à y ≈ 102)
      el.style.maxHeight = '60vh';
    }
  }

  function renderTool(ev) {
    if (ev.devant || ev.title === 'Devant' || ev.tool === 'devant') {
      renderDevant(ev);
      return;
    }
    // Card « tool » d'un module (listes à fiches, checkup…) : son drapeau la confie au module, ou l'écarte s'il est désactivé.
    for (const [flag, fn] of toolRenderers) if (ev[flag]) { fn(ev); return; }
    for (const k of disabledKinds) if (k.startsWith('tool.') && (ev[k.slice(5)] || ev.tool === k.slice(5))) return;
    const key = `tool:${ev.title || ev.kind}`;
    if (Array.isArray(ev.rows)) {
      // persist : card d'une application (terminal, Claude) — elle reste tant que l'application tourne.
      renderRows(ev, key, ev.pinned || ev.persist || pinnedCards.has(key) ? 0 : (ev.ttl || 60000));
      return;
    }
    const input = ev.input && Object.keys(ev.input).length ? Object.entries(ev.input).slice(0, 3).map(([k, v]) => `${k} : ${typeof v === 'string' ? v : JSON.stringify(v)}`).join('\n') : '';
    const titleText = ev.title || 'Action Système';
    const titleEl = h('span', { class: 'w-title' }, titleText);
    const resultEl = h('div', { class: 'w-result' }, String(ev.result || '(sans résultat)').slice(0, 700));

    const el = h('div', { class: `widget w-tool${ev.error ? ' error' : ''}` },
      h('div', { class: 'w-head' },
        gripHandle(),
        h('span', { class: 'w-icon' }, ICONS[ev.kind] || ICONS.tool),
        titleEl,
        h('span', { class: 'w-source' }, String(ev.tool || '').replace(/_/g, ' ').toUpperCase().slice(0, 22)),
        numBadge(ev),
        closeButton(ev, key)),
      input ? h('div', { class: 'w-input' }, input.slice(0, 240)) : null,
      ev.pending ? h('div', { class: 'w-empty' }, 'en cours…') : resultEl);

    place(key, el, ev.kind, ev.pending ? 90000 : AUTO_HIDE[ev.kind] || 45000);
  }

  // --- Initialisation des 5 widgets permanents en mode fantôme transparent au fond ---
  function initPermanentWidgets() {
    const defs = [
      { key: 'mail', kind: 'mail', title: 'Derniers emails', source: 'GMAIL' },
      { key: 'agenda', kind: 'agenda', title: 'Agenda & Planning', source: 'AGENDA' },
      { key: 'browser', kind: 'browser', title: 'Navigateur Web', source: 'CHROME' },
      { key: 'tool', kind: 'tool', title: 'Outil & Système', source: 'JARVIS' }
    ];
    for (const d of defs) addGhost(d);
  }
  // Widget fantôme (transparent, au fond) qui attend d'être utilisé ; les modules en ajoutent (card Terminal…).
  function addGhost(d) {
    if (cards.has(d.key)) return;
    const pos = getWidgetPos(d.key);
    const el = h('div', { class: `widget w-${d.kind} w-ghost` },
      h('div', { class: 'w-head' },
        gripHandle(),
        h('span', { class: 'w-icon' }, ICONS[d.kind]),
        h('span', { class: 'w-title' }, d.title),
        h('span', { class: 'w-source' }, d.source)));

    el.style.left = `${pos.x}px`;
    el.style.top = `${pos.y}px`;
    makeDraggable(el, d.key);
    root.appendChild(el);
    cards.set(d.key, { el, kind: d.kind, state: 'ghost', timer: null, fadeTimer: null });
  }

  // Initialisation immédiate des widgets fantômes
  initPermanentWidgets();

  // Invalidation du cache des boîtes de widgets
  J.invalidateWidgetBoxes = function () {
    for (const card of cards.values()) {
      card._cachedBox = null;
    }
  };
  if (typeof window !== 'undefined' && typeof window.addEventListener === 'function') {
    window.addEventListener('resize', J.invalidateWidgetBoxes, { passive: true });
    // Fenêtre réduite (demi-écran, autre moniteur) : les cards ouvertes restent entièrement visibles.
    window.addEventListener('resize', () => {
      for (const card of cards.values()) if (card.state === 'active') keepInView(card.el);
    }, { passive: true });
  }

  // Export pour le tracé des liaisons dans branches.js (version avec mise en cache sans reflow)
  J.getWidgetBox = function (key) {
    const card = cards.get(key);
    if (!card || !card.el) return null;
    if (card._cachedBox && card._cachedState === card.state) {
      return card._cachedBox;
    }
    const S = parseFloat(document.documentElement.style.getPropertyValue('--ui')) || 1;
    const rect = card.el.getBoundingClientRect();
    const box = {
      key,
      kind: card.kind || key,
      state: card.state || 'ghost',
      x: rect.left / S,
      y: rect.top / S,
      w: rect.width / S,
      h: rect.height / S,
      el: card.el
    };
    card._cachedBox = box;
    card._cachedState = card.state;
    return box;
  };

  J.getWidgetBoxes = function () {
    const boxes = {};
    for (const key of cards.keys()) {
      const b = J.getWidgetBox(key);
      if (b) boxes[key] = b;
    }
    return boxes;
  };

  if (bridge) {
    bridge.onTaskEvent((ev) => {
      if (!ev) return;
      dispatchWithNo(ev);
    });
    function dispatchEv(ev) {
      for (const fn of evHandlers.get(ev.type) || []) { try { fn(ev); } catch (e) { console.error(e); } }
      if (ev.type === 'widget') {
        if (!modulesReady) pendingWidgets.push(ev); // les modules ne sont pas encore chargés : la card attend
        else if (renderers.has(ev.kind)) renderers.get(ev.kind)(ev);
        else if (disabledKinds.has(ev.kind)) return; // module désactivé : pas de card
        else if ((ev.kind === 'mail' || ev.kind === 'agenda') && Array.isArray(ev.rows)) renderRows(ev, ev.kind, AUTO_HIDE[ev.kind]);
        else if (ev.kind === 'mail' || ev.kind === 'agenda') renderList(ev);
        else renderTool(ev);
      } else if (ev.type === 'widget_close' && ev.key) {
        close(ev.key);
      } else if (ev.type === 'cards_sync' && Array.isArray(ev.keys)) {
        // Noyau (re)connecté : les cards de liste ou d'application qu'il ne connaît pas viennent d'un
        // noyau précédent (relance) — orphelines, elles partent ; le nouveau noyau réaffiche les siennes.
        const known = new Set(ev.keys);
        for (const [key, card] of [...cards]) {
          if (String(key).startsWith('tool:') && card.state === 'active' && !known.has(key)) close(key);
        }
      } else if (ev.type === 'theme_config' || (ev.type === 'status' && ev.theme)) {
        applyTheme(ev.theme);
      }
    }
  }

  function dispatchWithNo(ev) {
    eventNo = Number(ev.cardNo) || 0;
    try { dispatchEv(ev); } finally { eventNo = 0; }
  }
  // Modules chargés (ui/modules-loader.js) : les cards en attente s'affichent ; les kinds des modules désactivés sont ignorés.
  document.addEventListener('jarvis:modules', (e) => {
    for (const m of (e.detail && e.detail.inactifs) || []) for (const k of m.widgets || []) if (!renderers.has(k)) disabledKinds.add(k);
    // Widget fantôme d'un module désactivé (mails, agenda…) : retiré.
    for (const k of disabledKinds) {
      const rec = cards.get(k);
      if (rec && rec.state === 'ghost' && rec.el) { rec.el.remove(); cards.delete(k); }
    }
    modulesReady = true;
    for (const ev of pendingWidgets.splice(0)) dispatchWithNo(ev);
  });

  // Outils offerts aux modules (modules/<id>/ui) : cadre commun des cards, lignes, placement, registre.
  J.widgetKit = {
    h, place, close, gripHandle, numBadge, closeButton, header, openOnClick, keepInView, makeDraggable, typewrite,
    setFolded, foldButton, pinButton, renderRows, rowsList, activateRow, sayRow, updateSourceBadge, appName, fmtTime,
    ICONS, AUTO_HIDE, cards, getWidgetPos, normalizeHead,
    register: registerRenderer, registerTool, renderList, on: onEvent,
    headButtons: (fn) => headDecorators.push(fn), footButtons: (fn) => footDecorators.push(fn),
    noAppClick: (tool) => noAppClick.add(tool), ghostKey: (key) => GHOST_KEYS.add(key), addGhost,
    guardClose: (key, fn) => closeGuards.set(key, fn), ownKey: (key) => ownKeys.add(key), onIdle: (key, hooks) => idleHooks.set(key, hooks),
    isFolded: (key) => foldedCards.has(key), onClose, vanish: (key) => vanishKeys.add(key),
    eventNo: () => eventNo,
    // Affiche un événement comme s'il venait du noyau (card locale d'un module, essais).
    dispatch: (ev) => dispatchWithNo(ev)
  };
  // Bouton « ranger » du HUD (cards-dock.js) : réduit en bouton jaune les cards de type 1 (toujours actives : Bureau ici)
  // et rend leurs éléments (les cards de type 2 ouvertes sont rangées à part : collapseOpenType2).
  function collapseAll() {
    setBureauCollapsed(true);
    const out = [];
    for (const [key, rec] of cards) {
      if (rec.el && rec.el.isConnected && rec.el.classList.contains('w-compact') && rec.el.classList.contains('w-collapsed')) out.push({ key, el: rec.el });
    }
    return out;
  }
  // Bouton « ranger », suite : les cards de type 2 ouvertes (temporaires : Terminal, mails, notes, listes…) sont réduites en
  // bouton violet ; leurs éléments sont rendus pour être rangés sous les cards de type 1. Les cards inactives (fantômes)
  // ne bougent pas.
  function collapseOpenType2() {
    const out = [];
    for (const [key, rec] of cards) {
      const el = rec.el;
      if (!el || !el.isConnected || rec.state !== 'active' || el.classList.contains('w-compact') || el.classList.contains('w-ghost') || !el.classList.contains('w-foldable')) continue;
      if (!el.classList.contains('w-collapsed')) setFolded(key, true);
      out.push({ key, el });
    }
    return out;
  }
  // Rouvre la card Bureau, ou rend faux quand elle n'existe pas (le noyau doit alors la fabriquer).
  function showBureau() {
    const has = [...cards.values()].some((rec) => rec.el && rec.el.isConnected && rec.el.classList.contains('w-compact'));
    if (has) setBureauCollapsed(false);
    return has;
  }
  // Emplacement par défaut d'une card (bouton 📍 « fixer ») : mémorisé par card, rendu par « PLACER » (cards-dock.js).
  const DEFAULT_PREFIX = () => (window.JARVIS_POS_PREFIX || 'jarvis.') + 'defaultPos.';
  function defaultPos(key) {
    try {
      const p = JSON.parse(localStorage.getItem(DEFAULT_PREFIX() + key) || 'null');
      if (p && Number.isFinite(p.x) && Number.isFinite(p.y)) return p;
    } catch {}
    return null;
  }
  function fixButton(key, el) {
    const btn = h('button', { class: 'w-fix', title: 'Fixer : garde l’emplacement de la card. Quand il est actif : un clic la replace à cet emplacement, un second clic (elle y est déjà) désactive le fixage, puis on recommence (Maj + clic : l’oublier) · le bouton « PLACER » y remet toutes les cards' }, '📍');
    btn.classList.toggle('on', Boolean(defaultPos(key)));
    btn.onclick = (e) => {
      e.stopPropagation();
      const here = { x: parseFloat(el.style.left) || el.offsetLeft, y: parseFloat(el.style.top) || el.offsetTop };
      const p = defaultPos(key);
      try {
        if (e.shiftKey) localStorage.removeItem(DEFAULT_PREFIX() + key);
        else if (!p) localStorage.setItem(DEFAULT_PREFIX() + key, JSON.stringify(here)); // pas fixé : on fixe ici
        else if (Math.abs(here.x - p.x) <= 12 && Math.abs(here.y - p.y) <= 12) localStorage.removeItem(DEFAULT_PREFIX() + key); // déjà là : on désactive
        else { moveCard(key, p.x, p.y); keepInView(el); } // ailleurs : on la replace
      } catch {}
      btn.classList.toggle('on', Boolean(defaultPos(key)));
    };
    return btn;
  }
  function addFixButton(key, el) {
    const head = el.querySelector('.w-head');
    if (!head || head.querySelector('.w-fix')) return;
    // Même ordre partout : 📍 📌 – ×
    const next = head.querySelector('.w-pin') || head.querySelector('.w-collapse') || head.querySelector('.w-close');
    if (next) head.insertBefore(fixButton(key, el), next);
    else head.append(fixButton(key, el));
  }
  // En-tête identique pour toutes les cards : poignée, numéro, titre, puis 📍 📌 (– ×). Le numéro vient du noyau
  // (ev.cardNo, lu pendant l'événement en cours) ou, pour une card du HUD (ownKey : musique), de J.CARD_NO.
  function normalizeHead(key, el) {
    const head = el.querySelector('.w-head');
    if (!head) return;
    const no = eventNo || (ownKeys.has(key) && J.CARD_NO ? J.CARD_NO[key] : 0);
    if (no && !head.querySelector('.w-num')) {
      const badge = h('span', { class: 'w-num', title: `Card n° ${no}` }, String(no));
      if (ownKeys.has(key)) badge.dataset.cardno = key;
      const grip = head.querySelector('.w-grip');
      if (grip) grip.after(badge); else head.prepend(badge);
    }
    const label = head.querySelector('.w-label');
    if (label && !head.querySelector('.w-title')) label.classList.add('w-title');
    const titleEl = head.querySelector('.w-title');
    // Toutes les cards se réduisent en bouton (violet) : bouton « – » et pastille, comme les cards en lignes.
    if (!head.querySelector('.w-collapse') && !el.classList.contains('w-compact') && !el.classList.contains('w-ghost')) {
      el.classList.add('w-foldable', 'w-xfold');
      const btn = foldButton(key);
      const closeBtn = head.querySelector('.w-close');
      if (closeBtn) head.insertBefore(btn, closeBtn); else head.append(btn);
      el.append(miniPill(String(titleEl ? titleEl.textContent : key).toUpperCase(), { foldable: true }));
      expandOnTap(el, () => setFolded(key, false));
      if (foldedCards.has(key)) el.classList.add('w-collapsed');
    }
    // Clic sur le titre (ou le numéro) : la card se réduit en bouton, ou se ferme quand elle ne se réduit pas.
    // stopPropagation : le clic remonterait à la card, déjà réduite, et la redéplierait aussitôt (expandOnTap).
    let titleDown = null;
    for (const t of head.querySelectorAll('.w-title, .w-num')) {
      t.addEventListener('pointerdown', (e) => { titleDown = { x: e.clientX, y: e.clientY }; });
      t.addEventListener('click', (e) => {
        if (titleDown && Math.hypot(e.clientX - titleDown.x, e.clientY - titleDown.y) > 5) return; // glissé, pas cliqué
        e.stopPropagation();
        if (el.classList.contains('w-compact')) setBureauCollapsed(true);
        else if (el.classList.contains('w-foldable')) setFolded(key, true);
        else { const x = head.querySelector('button.w-close'); if (x) x.click(); }
      });
    }
    if (!head.querySelector('.w-pin')) {
      const next = head.querySelector('.w-collapse') || head.querySelector('.w-close');
      const pin = pinButton(el, key);
      if (next) head.insertBefore(pin, next); else head.append(pin);
    }
    addFixButton(key, el);
  }
  // 2e clic sur « PLACER » : les cards qui ont un emplacement par défaut, déjà à leur place, s'ouvrent toutes.
  function defaultsState() {
    let total = 0;
    let there = 0;
    for (const [key, rec] of cards) {
      const p = defaultPos(key);
      if (!p || !rec.el || !rec.el.isConnected) continue;
      total++;
      if (Math.abs((parseFloat(rec.el.style.left) || 0) - p.x) <= 12 && Math.abs((parseFloat(rec.el.style.top) || 0) - p.y) <= 12) there++;
    }
    return { total, there };
  }
  function openDefaults() {
    let n = 0;
    for (const [key, rec] of cards) {
      if (!defaultPos(key) || !rec.el || !rec.el.isConnected || !rec.el.classList.contains('w-collapsed')) continue;
      if (rec.el.classList.contains('w-compact')) setBureauCollapsed(false);
      else setFolded(key, false);
      n++;
    }
    return n;
  }
  // Vue figée (bouton « FIGER » / « RESTAURER », cards-dock.js) : emplacement et état (ouverte / réduite) de chaque card.
  function layoutSnapshot() {
    const out = {};
    for (const [key, rec] of cards) {
      if (!rec.el || !rec.el.isConnected) continue;
      out[key] = { x: parseFloat(rec.el.style.left) || 0, y: parseFloat(rec.el.style.top) || 0, collapsed: rec.el.classList.contains('w-collapsed') };
    }
    return out;
  }
  function layoutApply(snap) {
    let n = 0;
    for (const [key, s] of Object.entries(snap || {})) {
      if (!Number.isFinite(s.x) || !Number.isFinite(s.y)) continue;
      const rec = cards.get(key);
      if (!rec || !rec.el || !rec.el.isConnected) { // pas ouverte maintenant : elle s'ouvrira là, dans cet état
        try { localStorage.setItem(DEFAULT_PREFIX().replace('defaultPos.', 'widgetPos.') + key, JSON.stringify({ x: s.x, y: s.y })); } catch {}
        if (String(key).startsWith('tool:')) { if (s.collapsed) foldedCards.add(key); else foldedCards.delete(key); }
        continue;
      }
      moveCard(key, s.x, s.y);
      if (rec.el.classList.contains('w-compact')) setBureauCollapsed(Boolean(s.collapsed));
      else if (rec.el.classList.contains('w-foldable')) setFolded(key, Boolean(s.collapsed));
      keepInView(rec.el);
      n++;
    }
    return n;
  }
  // Remet chaque card ouverte qui a un emplacement par défaut à cet emplacement ; rend leur nombre.
  function restoreDefaults() {
    let n = 0;
    for (const [key, rec] of cards) {
      const p = defaultPos(key);
      if (!p || !rec.el || !rec.el.isConnected) continue;
      moveCard(key, p.x, p.y);
      keepInView(rec.el);
      n++;
    }
    return n;
  }

  // Position d'une card en pixels de la page, enregistrée comme après un glisser-déposer.
  function moveCard(key, x, y) {
    const rec = cards.get(key);
    if (!rec || !rec.el) return;
    rec.layoutX = rec.layoutY = rec.layoutTX = rec.layoutTY = 0; // décalage automatique (app.js) remis à zéro
    rec.el.style.translate = '';
    rec.el.style.left = `${x}px`;
    rec.el.style.top = `${y}px`;
    rec._cachedBox = null;
    try { localStorage.setItem((window.JARVIS_POS_PREFIX || 'jarvis.') + 'widgetPos.' + key, JSON.stringify({ x, y })); } catch {}
  }

  // Cards de type 2 inactives : une minute sans mise à jour, sans clic ni survol, sans musique en lecture, et la card
  // s'efface progressivement (même fondu que le ×) puis reste en fantôme. Exceptions : cards figées (📌), réduites en bouton,
  // cards d'une application qui tourne (Terminal…) et musique tant qu'elle joue.
  let idleFadeMs = 60000;
  function sweepIdle() {
    const now = performance.now();
    for (const [key, rec] of cards) {
      const el = rec.el;
      if (rec.state !== 'active' || !el || !el.isConnected) continue;
      if (el.classList.contains('w-compact') || el.classList.contains('w-ghost') || el.classList.contains('w-collapsed') || pinnedCards.has(key) || rec.persist) continue;
      const idle = idleHooks.get(key);
      if (idle && idle.busy && idle.busy()) { rec.lastActive = now; continue; }
      if (el.matches(':hover')) { rec.lastActive = now; continue; }
      if (now - (rec.lastActive || now) < idleFadeMs) continue;
      if (idle && idle.idle) idle.idle(); // ex. musique arrêtée depuis une minute : réduite en bouton
      else close(key);
    }
  }
  setInterval(sweepIdle, 5000);
  // Un clic dans une card compte comme une activité.
  root.addEventListener('pointerdown', (e) => {
    const target = e.target instanceof Element ? e.target.closest('.widget') : null;
    if (!target) return;
    for (const rec of cards.values()) if (rec.el === target) rec.lastActive = performance.now();
  }, true);

  J.widgets = { sweepIdle, setIdleMs: (ms) => { idleFadeMs = ms; }, close, cards, initPermanentWidgets, collapseAll, collapseOpenType2, showBureau, moveCard, restoreDefaults, defaultsState, openDefaults, layoutSnapshot, layoutApply };
})();
