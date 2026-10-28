// Module Actions : la card 5, qui liste les cards qu'on peut ouvrir. Ses lignes viennent des fiches des autres
// modules actifs (module.json › actions) ; un clic ouvre la card, un second la referme. Type 1 (toujours active) :
// réduite en bouton jaune tant qu'on ne l'ouvre pas. Le bouton « ☰ ACTIONS » du HUD (ui/cards-dock.js) l'ouvre aussi.
(function () {
  const J = (window.JARVIS = window.JARVIS || {});
  const bridge = window.jarvisBridge;
  const root = document.getElementById('widgets');
  if (!root || !J.cardHeader) return;

  const MARGIN = 12; // même marge que widgets.js › keepInView
  const ui = () => parseFloat(document.documentElement.style.getPropertyValue('--ui')) || 1;
  function el(tag, cls, text) {
    const e = document.createElement(tag);
    if (cls) e.className = cls;
    if (text !== undefined) e.textContent = text;
    return e;
  }

  // Seules les cards de type 2 (temporaires) sont listées : celles de type 1 (Bureau, Audio, Matériel…) sont toujours
  // présentes, réduites en boutons. run : phrase envoyée à Jarvis (comme une ligne de card cliquée), qui fabrique la card.
  const sayIt = (say) => { if (bridge && bridge.utterance) bridge.utterance(say, { from: 'widget' }); };
  // Les lignes viennent des fiches des modules actifs (module.json › actions) : voir plus bas.
  const ACTIONS = [];

  // La card Actions est de type 1 (toujours active) : toujours présente, réduite en bouton jaune tant qu'on ne l'ouvre
  // pas, avec le même en-tête que les autres (numéro, titre, 📍 fixer, 📌 figer, – réduire). État et place mémorisés.
  const PREFIX = () => window.JARVIS_POS_PREFIX || 'jarvis.';
  const POS_KEY = () => PREFIX() + 'widgetPos.actions';
  const DEFAULT_KEY = () => PREFIX() + 'defaultPos.actions';
  const readPos = (key) => { try { const p = JSON.parse(localStorage.getItem(key) || 'null'); return p && Number.isFinite(p.x) && Number.isFinite(p.y) ? p : null; } catch { return null; } };
  let collapsed = true;
  let pinned = false;
  try {
    collapsed = localStorage.getItem('jarvis.actionsCollapsed') !== '0';
    pinned = localStorage.getItem('jarvis.actionsPinned') === '1';
  } catch {}

  const card = el('div', 'widget w-tool w-card-list w-active w-actions-card');
  const hdr = J.cardHeader({ key: 'actions', no: J.CARD_NO.actions, title: 'ACTIONS', onCollapse: () => setCollapsed(true) });
  const list = el('div', 'w-rows');
  const rows = new Map(); // action → ligne

  // Choix de l'utilisateur : lignes affichées et leur ordre (⚙ dans l'en-tête). Identifiant d'une ligne : module|titre.
  // order : identifiants dans l'ordre voulu (une ligne inconnue va à la fin, dans l'ordre d'arrivée) ; hidden : masquées.
  const PREFS_KEY = 'jarvis.actionsPrefs';
  let prefs = { order: [], hidden: [] };
  try {
    const p = JSON.parse(localStorage.getItem(PREFS_KEY) || 'null');
    if (p) prefs = { order: Array.isArray(p.order) ? p.order : [], hidden: Array.isArray(p.hidden) ? p.hidden : [] };
  } catch {}
  const savePrefs = () => { try { localStorage.setItem(PREFS_KEY, JSON.stringify(prefs)); } catch {} };
  let editing = false;
  const editBtn = el('button', 'w-pin w-act-edit', '⚙');
  editBtn.title = 'Personnaliser : choisir les lignes affichées et leur ordre';
  hdr.head.insertBefore(editBtn, hdr.fix);
  const editBar = el('div', 'w-act-editbar');
  const resetBtn = el('button', 'w-act-reset', 'Tout afficher · ordre d’origine');
  editBar.append(el('span', 'w-act-hint', 'Coche les lignes à afficher, ▲ ▼ pour l’ordre'), resetBtn);
  // Ordre courant de toutes les lignes (affichées ou non).
  function orderedActions() {
    const all = [...rows.keys()];
    const rank = (a) => { const i = prefs.order.indexOf(a.id); return i < 0 ? prefs.order.length + all.indexOf(a) : i; };
    return all.sort((x, y) => rank(x) - rank(y));
  }
  function applyPrefs() {
    for (const a of orderedActions()) {
      const row = rows.get(a);
      const hidden = prefs.hidden.includes(a.id);
      row.classList.toggle('w-act-hidden', hidden);
      row.style.display = hidden && !editing ? 'none' : '';
      const box = row.querySelector('.w-act-show');
      if (box) box.checked = !hidden;
      list.appendChild(row); // réinsertion = nouvel ordre
    }
  }
  function moveAction(a, step) {
    const ids = orderedActions().map((x) => x.id);
    const i = ids.indexOf(a.id);
    const j = i + step;
    if (i < 0 || j < 0 || j >= ids.length) return;
    [ids[i], ids[j]] = [ids[j], ids[i]];
    prefs.order = ids;
    savePrefs();
    applyPrefs();
  }
  function setEditing(on) {
    editing = Boolean(on);
    card.classList.toggle('w-act-editing', editing);
    editBtn.classList.toggle('on', editing);
    applyPrefs();
  }
  editBtn.onclick = (e) => { e.stopPropagation(); setEditing(!editing); };
  resetBtn.onclick = (e) => { e.stopPropagation(); prefs = { order: [], hidden: [] }; savePrefs(); applyPrefs(); };

  function addActionRow(a) {
    a.id = `${a.mod || ''}|${a.title}`;
    const row = el('div', 'w-row clickable');
    row.title = `Ouvrir : ${a.title}`;
    const text = el('span', 'w-row-text');
    // Numéro de la card, après le titre (il sert à la voix : « ferme la card 102 »).
    const no = el('span', 'w-row-no');
    no.dataset.slot = a.slot;
    const titleLine = el('span', 'w-row-title', a.title);
    titleLine.append(no);
    text.append(titleLine, el('span', 'w-row-sub', a.sub));
    // Nombre d'éléments de la card (listes, notes, dossiers…), à droite : méthode « compteurs » du noyau.
    const count = el('span', 'w-row-count');
    count.dataset.mod = a.mod || '';
    // Réglages de la ligne, visibles en mode « personnaliser » : affichée ou non, monter, descendre.
    const tools = el('span', 'w-act-tools');
    const show = el('input', 'w-act-show');
    show.type = 'checkbox';
    show.title = 'Afficher cette ligne';
    show.onclick = (e) => e.stopPropagation();
    show.onchange = () => {
      prefs.hidden = prefs.hidden.filter((id) => id !== a.id);
      if (!show.checked) prefs.hidden.push(a.id);
      savePrefs();
      applyPrefs();
    };
    const up = el('button', 'w-act-move', '▲');
    up.title = 'Monter';
    up.onclick = (e) => { e.stopPropagation(); moveAction(a, -1); };
    const down = el('button', 'w-act-move', '▼');
    down.title = 'Descendre';
    down.onclick = (e) => { e.stopPropagation(); moveAction(a, 1); };
    tools.append(show, up, down);
    row.append(tools, el('span', 'w-row-ico', a.ico), text, count);
    row.onclick = () => { if (!editing) toggleAction(a, row); };
    rows.set(a, row);
    list.appendChild(row);
  }
  for (const a of ACTIONS) addActionRow(a);
  // Actions déclarées par les modules actifs (module.json › actions : ico, titre, sous, dire ou appel, emplacement = card ouverte).
  document.addEventListener('jarvis:modules', (e) => {
    for (const m of (e.detail && e.detail.modules) || []) {
      for (const a of Array.isArray(m.actions) ? m.actions : []) {
        if (!a || !a.titre || !(a.dire || Array.isArray(a.appel))) continue;
        // dire : phrase envoyée à Jarvis ; appel : [méthode, …arguments] d'une méthode du noyau.
        const run = a.dire ? () => sayIt(a.dire) : () => { if (bridge && bridge.call) bridge.call(...a.appel).catch(() => {}); };
        addActionRow({ slot: a.emplacement || (m.card && m.card.emplacement) || m.id, mod: m.id, ico: a.ico || '▤', title: a.titre, sub: a.sous || m.nom, run });
      }
    }
    applyPrefs();
    paintNumbers();
  });
  // Un clic sur une action ouvre sa card ; un second clic la referme. On retient les cards apparues juste après le
  // clic (le noyau les fabrique) pour pouvoir les fermer ; la ligne reste allumée tant qu'elles sont ouvertes.
  const opened = new Map(); // action → clés des cards ouvertes
  const liveKeys = () => {
    const out = new Set();
    const cards = J.widgets && J.widgets.cards;
    if (cards) for (const [key, rec] of cards) if (rec.state === 'active' && rec.el && rec.el.isConnected) out.add(key);
    return out;
  };
  function toggleAction(a, row) {
    const keys = [...(opened.get(a) || [])].filter((k) => liveKeys().has(k));
    if (keys.length) { // déjà ouverte : on la ferme
      for (const k of keys) J.widgets.close(k);
      opened.delete(a);
      paintRows();
      return;
    }
    const before = liveKeys();
    a.run();
    let tries = 0;
    const watch = setInterval(() => { // la card arrive un instant plus tard : on la repère
      const fresh = [...liveKeys()].filter((k) => !before.has(k));
      if (fresh.length) { opened.set(a, fresh); clearInterval(watch); paintRows(); }
      else if (++tries > 16) clearInterval(watch);
    }, 250);
  }
  const paintNumbers = () => { for (const no of list.querySelectorAll('.w-row-no')) { const n = J.cardSlotNo && J.cardSlotNo(no.dataset.slot); no.textContent = Number.isFinite(n) ? String(n) : ''; } };
  document.addEventListener('jarvis:cardnumbers', paintNumbers);
  setTimeout(paintNumbers, 0);
  function paintRows() {
    const live = liveKeys();
    for (const [a, row] of rows) row.classList.toggle('on', (opened.get(a) || []).some((k) => live.has(k)));
  }
  setInterval(() => { if (!collapsed) paintRows(); }, 600);

  // Alerte sur la ligne Nomade : listes à valider, dépôts reçus, modifications à vérifier, données ajoutées depuis le site.
  // L'état vient du noyau (méthode « nomade », op « etat », en local) ; le bouton réduit s'allume aussi.
  const pill = el('div', 'w-mini-pill', '▤ ACTIONS');
  const plural = (n, one, many) => `${n} ${n > 1 ? many : one}`;
  function paintNomadeAlert(st) {
    const depots = (st && Array.isArray(st.depots)) ? st.depots : [];
    const lists = depots.filter((d) => d.kind === 'nouvelle_liste').length;
    const others = depots.length - lists;
    const check = (st && Array.isArray(st.conflicts)) ? st.conflicts.length : 0;
    const sum = st && st.summary;
    const added = Boolean(sum && sum.recus > 0 && Date.now() - (sum.at || 0) < 10 * 60000);
    const parts = [
      lists ? plural(lists, 'liste à valider', 'listes à valider') : '',
      others ? plural(others, 'dépôt reçu', 'dépôts reçus') : '',
      check ? `${check} à vérifier` : '',
      added ? 'données ajoutées depuis le site' : '',
    ].filter(Boolean);
    for (const [a, row] of rows) {
      if (a.slot !== 'nomade_view') continue;
      let tag = row.querySelector('.w-row-alert');
      if (!parts.length) { if (tag) tag.remove(); row.classList.remove('alert'); continue; }
      if (!tag) { tag = el('span', 'w-row-alert'); row.querySelector('.w-row-text').append(tag); }
      tag.textContent = `⚠ ${parts.join(' · ')}`;
      tag.title = 'Ouvre la card Nomade pour traiter';
      row.classList.add('alert');
    }
    pill.classList.toggle('has-alert', parts.length > 0);
  }
  let alertBusy = false;
  async function refreshAlert() {
    if (alertBusy || !bridge || !bridge.call || document.hidden) return;
    alertBusy = true;
    try { paintNomadeAlert(await bridge.call('nomade', 'etat')); } catch {} finally { alertBusy = false; }
  }
  setInterval(refreshAlert, 30000);
  setTimeout(refreshAlert, 4000);
  document.addEventListener('jarvis:modules', () => setTimeout(refreshAlert, 500));
  if (bridge && bridge.onTaskEvent) {
    bridge.onTaskEvent((ev) => { if (ev && (ev.type === 'nomade_depot' || (ev.type === 'widget' && ev.kind === 'nomade_view'))) setTimeout(refreshAlert, 300); });
  }

  // Compteurs à droite des lignes (nombre de listes, de notes, de projets, de réunions, de dossiers Data).
  let countBusy = false;
  async function refreshCounts() {
    if (countBusy || !bridge || !bridge.call || document.hidden) return;
    countBusy = true;
    try {
      const c = (await bridge.call('compteurs')) || {};
      for (const span of list.querySelectorAll('.w-row-count')) {
        const n = c[span.dataset.mod];
        span.textContent = Number.isFinite(n) ? String(n) : '';
        span.title = Number.isFinite(n) ? `${n} élément${n > 1 ? 's' : ''}` : '';
      }
    } catch {} finally { countBusy = false; }
  }
  setInterval(refreshCounts, 30000);
  setTimeout(refreshCounts, 4500);
  document.addEventListener('jarvis:modules', () => setTimeout(refreshCounts, 600));
  if (bridge && bridge.onTaskEvent) {
    const CHANGES = ['savoir_changed', 'collections_changed', 'note', 'note_state', 'meeting_update', 'agents_changed'];
    let soon = null;
    bridge.onTaskEvent((ev) => {
      if (!ev || !CHANGES.includes(ev.type)) return;
      clearTimeout(soon);
      soon = setTimeout(refreshCounts, 800);
    });
  }

  card.append(hdr.head, editBar, list, pill);
  root.appendChild(card);

  function place(x, y) {
    const maxX = Math.max(MARGIN, window.innerWidth - card.offsetWidth - MARGIN);
    const maxY = Math.max(MARGIN, window.innerHeight - card.offsetHeight - MARGIN);
    card.style.left = `${Math.min(Math.max(MARGIN, x), maxX)}px`;
    card.style.top = `${Math.min(Math.max(MARGIN, y), maxY)}px`;
  }
  const savePos = () => { try { localStorage.setItem(POS_KEY(), JSON.stringify({ x: parseFloat(card.style.left), y: parseFloat(card.style.top) })); } catch {} };
  function moveTo(x, y) { place(x, y); savePos(); }

  function setCollapsed(on) {
    collapsed = Boolean(on);
    try { localStorage.setItem('jarvis.actionsCollapsed', collapsed ? '1' : '0'); } catch {}
    card.classList.toggle('w-collapsed', collapsed);
    card.style.width = collapsed ? '' : `${260 * ui()}px`; // réduite : la largeur des autres boutons (feuille de style)
    if (!collapsed && J.cardFront && J.cardFront.front) J.cardFront.front(card); // ouverte : devant les autres cards
    if (J.dockActionsOn) J.dockActionsOn(!collapsed);
    requestAnimationFrame(() => place(parseFloat(card.style.left) || 0, parseFloat(card.style.top) || 0));
  }

  // 📍 : l'emplacement actuel devient l'emplacement par défaut (Maj + clic : l'oublier) ; 📌 : la card ne se déplace plus.
  const paintFix = () => hdr.fix.classList.toggle('on', Boolean(readPos(DEFAULT_KEY())));
  hdr.fix.onclick = (e) => {
    e.stopPropagation();
    const p = readPos(DEFAULT_KEY());
    try {
      if (e.shiftKey) localStorage.removeItem(DEFAULT_KEY());
      else if (!p) localStorage.setItem(DEFAULT_KEY(), JSON.stringify({ x: Math.round(card.offsetLeft), y: Math.round(card.offsetTop) })); // pas fixé : on fixe ici
      else if (Math.abs(card.offsetLeft - p.x) <= 12 && Math.abs(card.offsetTop - p.y) <= 12) localStorage.removeItem(DEFAULT_KEY()); // déjà là : on désactive
      else moveTo(p.x, p.y); // ailleurs : on la replace
    } catch {}
    paintFix();
  };
  hdr.pin.classList.toggle('on', pinned);
  hdr.pin.onclick = (e) => {
    e.stopPropagation();
    pinned = !pinned;
    hdr.pin.classList.toggle('on', pinned);
    try { localStorage.setItem('jarvis.actionsPinned', pinned ? '1' : '0'); } catch {}
  };
  paintFix();

  // Glisser par la card (en-tête ou bouton réduit), sauf sur un bouton ; un simple clic sur le bouton réduit l'ouvre,
  // sur le titre la réduit. Le pointeur n'est capturé qu'une fois glissé : le clic simple arrive au bon élément.
  let from = null;
  let moved = false;
  card.addEventListener('pointerdown', (e) => {
    if (e.button !== 0 || e.target.closest('button, .w-row')) return;
    moved = false;
    if (!pinned) from = { dx: e.clientX - card.offsetLeft, dy: e.clientY - card.offsetTop, x: e.clientX, y: e.clientY };
  });
  card.addEventListener('pointermove', (e) => {
    if (!from) return;
    if (!moved && Math.hypot(e.clientX - from.x, e.clientY - from.y) < 5) return;
    if (!moved) { moved = true; card.classList.add('is-dragging'); try { card.setPointerCapture(e.pointerId); } catch {} }
    place(e.clientX - from.dx, e.clientY - from.dy);
  });
  const end = () => {
    if (from && moved) savePos();
    from = null;
    card.classList.remove('is-dragging');
  };
  card.addEventListener('pointerup', end);
  card.addEventListener('pointercancel', end);
  card.addEventListener('click', (e) => {
    if (moved) { moved = false; return; }
    if (collapsed) setCollapsed(false);
    else if (e.target.closest('.w-title, .w-num')) setCollapsed(true);
  });

  // Place de départ : celle enregistrée, sinon sous les boutons du HUD, à droite.
  function startPlace() {
    const S = ui();
    const p = readPos(POS_KEY());
    place(p ? p.x : window.innerWidth - 34 * S - card.offsetWidth, p ? p.y : (J.hudButtonsBottom || 90 * S) + 40 * S);
  }
  card.style.width = collapsed ? '' : `${260 * ui()}px`;
  card.classList.toggle('w-collapsed', collapsed);
  startPlace();
  setTimeout(startPlace, 1500); // la mise à l'échelle et la hauteur des boutons du HUD ne sont connues qu'après le premier tracé

  // Pour RANGER et PLACER : même interface que les autres cards de type 1.
  J.actionsCard = {
    element: card,
    isCollapsed: () => collapsed,
    setCollapsed,
    moveTo,
    // Vue figée (bouton FIGER / RESTAURER) : emplacement et état.
    layoutSnapshot() { return { x: Math.round(card.offsetLeft), y: Math.round(card.offsetTop), collapsed }; },
    layoutApply(s) {
      if (!s || !Number.isFinite(s.x) || !Number.isFinite(s.y)) return 0;
      moveTo(s.x, s.y);
      setCollapsed(Boolean(s.collapsed));
      return 1;
    },
    defaultState() {
      const p = readPos(DEFAULT_KEY());
      if (!p) return { total: 0, there: 0 };
      return { total: 1, there: Math.abs(card.offsetLeft - p.x) <= 12 && Math.abs(card.offsetTop - p.y) <= 12 ? 1 : 0 };
    },
    openDefault() {
      if (!readPos(DEFAULT_KEY()) || !collapsed) return 0;
      setCollapsed(false);
      return 1;
    },
    restoreDefault() {
      const p = readPos(DEFAULT_KEY());
      if (!p) return 0;
      moveTo(p.x, p.y);
      return 1;
    }
  };
  if (J.dockActionsOn) J.dockActionsOn(!collapsed);
  window.addEventListener('resize', () => place(parseFloat(card.style.left) || 0, parseFloat(card.style.top) || 0));
})();
