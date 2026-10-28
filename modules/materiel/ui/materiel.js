// Card MATÉRIEL du HUD (à droite) : une seule card, un seul en-tête, avec les sections processeur, GPU, mémoire
// et stockage (jauges circulaires et barres néon). Elle se réduit en un bouton (« ▤ MATÉRIEL 23 % ») et se
// rouvre d'un clic ; l'état est mémorisé. Module Matériel (modules/materiel/, interface seule) : alimenté par l'événement
// « telemetry » du cœur (mesures et sondes GPU / disques : orch/core_start.rs, orch/hardware.rs, partagées avec le
// mode éco automatique et le diagnostic).
(function () {
  const J = (window.JARVIS = window.JARVIS || {});
  const NS = 'http://www.w3.org/2000/svg';
  const KEY = 'jarvis.hwPanels';

  const KEY2 = 'jarvis.hwCollapsed';
  let collapsed = false;
  try { collapsed = localStorage.getItem(KEY2) === '1'; } catch {}
  const save = () => { try { localStorage.setItem(KEY2, collapsed ? '1' : '0'); } catch {} };

  const col = document.createElement('div');
  col.id = 'hw-panels';
  col.style.display = 'flex'; // visible dès le départ ; les valeurs arrivent avec la télémétrie
  document.body.appendChild(col);

  const card = document.createElement('div');
  card.className = 'hw-card hw-single';
  col.appendChild(card);
  let pill = null;
  function syncAll() {
    card.classList.toggle('collapsed', collapsed);
    col.classList.toggle('all-collapsed', collapsed);
  }
  // L'en-tête et le bouton réduit servent de poignée : glissés, ils déplacent la card (position mémorisée).
  const POS = (window.JARVIS_POS_PREFIX || 'jarvis.') + 'hwPanels.pos';
  function placeCol(x, y) {
    const w = col.offsetWidth || 206;
    const h = Math.min(col.offsetHeight || 40, window.innerHeight);
    col.style.right = 'auto';
    col.style.left = Math.max(0, Math.min(window.innerWidth - w, x)) + 'px';
    col.style.top = Math.max(0, Math.min(window.innerHeight - Math.min(h, 60), y)) + 'px';
  }
  try {
    const s = JSON.parse(localStorage.getItem(POS) || 'null');
    if (s && Number.isFinite(s.x) && Number.isFinite(s.y)) placeCol(s.x, s.y);
  } catch {}
  window.addEventListener('resize', () => {
    if (col.style.left) placeCol(parseFloat(col.style.left) || 0, parseFloat(col.style.top) || 0);
  });
  let drag = null;
  let moved = false;
  function handle(h) {
    h.addEventListener('pointerdown', (e) => {
      if (e.button !== 0 || pinned || e.target.closest('.w-fix, .w-pin, .w-collapse, .hw-toggle')) return;
      const r = col.getBoundingClientRect();
      drag = { dx: e.clientX - r.left, dy: e.clientY - r.top, x: e.clientX, y: e.clientY };
      moved = false;
    });
    h.addEventListener('pointermove', (e) => {
      if (!drag) return;
      if (!moved && Math.hypot(e.clientX - drag.x, e.clientY - drag.y) < 5) return;
      if (!moved) { moved = true; try { h.setPointerCapture(e.pointerId); } catch {} } // capturé seulement une fois glissé : le clic simple arrive au bon élément
      placeCol(e.clientX - drag.dx, e.clientY - drag.dy);
    });
    h.addEventListener('pointerup', () => {
      if (drag && moved) {
        try { localStorage.setItem(POS, JSON.stringify({ x: parseFloat(col.style.left), y: parseFloat(col.style.top) })); } catch {}
      }
      drag = null;
    });
  }
  // Emplacement par défaut de la colonne (bouton 📍 de chaque card) et retour à celui-ci (« PLACER », cards-dock.js).
  const DKEY = () => (window.JARVIS_POS_PREFIX || 'jarvis.') + 'defaultPos.hw';
  const savedDefault = () => { try { const p = JSON.parse(localStorage.getItem(DKEY()) || 'null'); return p && Number.isFinite(p.x) && Number.isFinite(p.y) ? p : null; } catch { return null; } };
  const fixes = [];
  // Colonne figée (📌) : elle ne se déplace plus ; état mémorisé, commun aux cards.
  const pins = [];
  let pinned = false;
  try { pinned = localStorage.getItem('jarvis.hwPinned') === '1'; } catch {}
  function setPinned(on) {
    pinned = on;
    try { localStorage.setItem('jarvis.hwPinned', on ? '1' : '0'); } catch {}
    for (const p of pins) p.classList.toggle('on', on);
  }
  const paintFix = () => { for (const f of fixes) f.classList.toggle('on', Boolean(savedDefault())); };
  function fixDefault(clear) {
    const r = col.getBoundingClientRect();
    const p = savedDefault();
    try {
      if (clear) localStorage.removeItem(DKEY());
      else if (!p) localStorage.setItem(DKEY(), JSON.stringify({ x: Math.round(r.left), y: Math.round(r.top) })); // pas fixé : on fixe ici
      else if (Math.abs(r.left - p.x) <= 12 && Math.abs(r.top - p.y) <= 12) localStorage.removeItem(DKEY()); // déjà là : on désactive
      else J.hwPanels.moveTo(p.x, p.y); // ailleurs : on la replace
    } catch {}
    paintFix();
  }

  function applyTheme() {
    let id = 'cyan';
    try { id = localStorage.getItem('jarvis.theme') || 'cyan'; } catch {}
    const theme = J.THEMES && (J.THEMES[id] || J.THEMES.cyan);
    if (theme) col.style.setProperty('--w-rgb', theme.primaryRgb);
  }
  applyTheme();

  function el(tag, cls, text) {
    const e = document.createElement(tag);
    if (cls) e.className = cls;
    if (text !== undefined) e.textContent = text;
    return e;
  }

  const R = 30;
  const CIRC = 2 * Math.PI * R;

  // Jauge circulaire : anneau, arc de valeur, pourcentage au centre.
  function ring() {
    const svg = document.createElementNS(NS, 'svg');
    svg.setAttribute('viewBox', '0 0 80 80');
    svg.setAttribute('class', 'hw-ring');
    const mk = (cls, r) => {
      const c = document.createElementNS(NS, 'circle');
      c.setAttribute('cx', '40'); c.setAttribute('cy', '40'); c.setAttribute('r', String(r));
      c.setAttribute('class', cls);
      svg.appendChild(c);
      return c;
    };
    mk('hw-ring-outer', 37);
    mk('hw-ring-track', R);
    const arc = mk('hw-ring-arc', R);
    arc.setAttribute('stroke-dasharray', `0 ${CIRC}`);
    arc.setAttribute('transform', 'rotate(-90 40 40)');
    const txt = document.createElementNS(NS, 'text');
    txt.setAttribute('x', '40'); txt.setAttribute('y', '45'); txt.setAttribute('class', 'hw-ring-txt');
    txt.setAttribute('text-anchor', 'middle');
    svg.appendChild(txt);
    return { svg, arc, txt };
  }

  const level = (pct) => (pct >= 90 ? 'crit' : pct >= 75 ? 'warn' : '');

  // Un seul en-tête pour toute la card : numéro, titre, 📍 📌 – (réduire en un bouton).
  const hdr = J.cardHeader({ key: 'hw', no: J.CARD_NO.hw, title: 'MATÉRIEL' });
  const head = hdr.head;
  head.classList.add('hw-head');
  const flip = () => { collapsed = !collapsed; save(); syncAll(); };
  const tap = () => { if (moved) { moved = false; return; } flip(); }; // fin d'un glisser : pas de bascule
  hdr.collapse.onclick = (e) => { e.stopPropagation(); flip(); };
  hdr.title.onclick = tap; // clic sur le titre : la card se réduit
  // (infobulle commune posée par card-header.js)
  hdr.fix.onclick = (e) => { e.stopPropagation(); fixDefault(e.shiftKey); };
  fixes.push(hdr.fix);
  hdr.pin.onclick = (e) => { e.stopPropagation(); setPinned(!pinned); };
  pins.push(hdr.pin);
  const body = el('div', 'hw-body hw-all');
  pill = el('button', 'hw-pill');
  pill.title = 'Afficher la card Matériel (glisser : déplacer)';
  const pillVal = el('span', 'hw-pill-v', '–');
  pill.append(el('span', 'hw-pill-l', '▤ MATÉRIEL'), pillVal);
  pill.onclick = tap;
  handle(head);
  handle(pill);
  card.append(head, body, pill);

  // Une section de la card : petit intitulé, puis son contenu.
  function panel(id, title) {
    const box = el('div', 'hw-sec');
    box.append(el('div', 'hw-sec-t', title));
    const inner = el('div', 'hw-sec-b');
    box.appendChild(inner);
    body.appendChild(box);
    return { box, body: inner, setLevel: (l) => { box.dataset.level = l || ''; } };
  }

  const line = (k, v) => {
    const row = el('div', 'hw-line');
    const kk = el('span', 'hw-k', k);
    const vv = el('span', 'hw-v', v);
    row.append(kk, vv);
    return { row, set: (t) => { vv.textContent = t; } };
  };

  // Card à jauge : anneau + lignes de détail.
  function gaugePanel(id, title, lineLabels) {
    const p = panel(id, title);
    const r = ring();
    const lines = lineLabels.map((k) => line(k, '–'));
    const side = el('div', 'hw-side');
    for (const l of lines) side.appendChild(l.row);
    p.body.classList.add('hw-gauge');
    p.body.append(r.svg, side);
    return {
      set(pct, texts) {
        const v = Math.max(0, Math.min(100, Number(pct) || 0));
        r.arc.setAttribute('stroke-dasharray', `${(CIRC * v) / 100} ${CIRC}`);
        r.txt.textContent = `${Math.round(v)}%`;
        if (id === 'cpu') pillVal.textContent = `${Math.round(v)} %`;
        p.setLevel(level(v));
        texts.forEach((t, i) => lines[i] && lines[i].set(t));
      },
      hide(on) { p.box.style.display = on ? 'none' : ''; }
    };
  }

  const cpu = gaugePanel('cpu', 'PROCESSEUR', ['Charge', 'Jarvis']);
  const gpu = gaugePanel('gpu', 'GPU', ['Modèle', 'VRAM', 'Température']);
  const ram = gaugePanel('ram', 'MÉMOIRE', ['Utilisée', 'Totale']);

  // Card disques : une barre néon par lecteur (occupé / total).
  const disks = panel('disks', 'STOCKAGE');
  disks.body.classList.add('hw-disks');
  let disksSig = '';
  function setDisks(list) {
    const sig = JSON.stringify(list);
    if (sig === disksSig) return;
    disksSig = sig;
    disks.body.textContent = '';
    for (const d of list) {
      const row = el('div', `hw-disk ${level(d.pct)}`);
      const top = el('div', 'hw-disk-top');
      top.append(el('span', 'hw-disk-n', d.name), el('span', 'hw-disk-p', `${d.pct}%`));
      const bar = el('div', 'hw-bar');
      const fill = el('div', 'hw-bar-fill');
      fill.style.width = `${d.pct}%`;
      bar.appendChild(fill);
      row.append(top, bar, el('div', 'hw-disk-c', `${d.usedGb} G / ${d.totalGb} G · libre ${d.totalGb - d.usedGb} G`));
      disks.body.appendChild(row);
    }
    const worst = list.reduce((m, d) => Math.max(m, d.pct), 0);
    disks.setLevel(level(worst));
  }

  syncAll();
  paintFix();
  setPinned(pinned);

  const gb = (n) => `${n} G`;

  J.hwPanels = {
    // Bouton « ranger » et card d'actions (cards-dock.js) : tout réduire / tout rouvrir, et poser la colonne.
    setAllCollapsed(on) {
      collapsed = Boolean(on);
      save();
      syncAll();
    },
    element: col,
    // Vue figée (cards-dock.js) : emplacement et état.
    layoutSnapshot() {
      const r = col.getBoundingClientRect();
      return { x: Math.round(r.left), y: Math.round(r.top), collapsed };
    },
    layoutApply(s) {
      if (!s || !Number.isFinite(s.x) || !Number.isFinite(s.y)) return 0;
      J.hwPanels.moveTo(s.x, s.y);
      J.hwPanels.setAllCollapsed(Boolean(s.collapsed));
      return 1;
    },
    defaultState() {
      const p = savedDefault();
      if (!p) return { total: 0, there: 0 };
      return { total: 1, there: Math.abs((parseFloat(col.style.left) || 0) - p.x) <= 12 && Math.abs((parseFloat(col.style.top) || 0) - p.y) <= 12 ? 1 : 0 };
    },
    openDefault() {
      if (!savedDefault() || !collapsed) return 0;
      J.hwPanels.setAllCollapsed(false);
      return 1;
    },
    restoreDefault() {
      const p = savedDefault();
      if (!p) return 0;
      J.hwPanels.moveTo(p.x, p.y);
      return 1;
    },
    moveTo(x, y) {
      placeCol(x, y);
      try { localStorage.setItem(POS, JSON.stringify({ x: parseFloat(col.style.left), y: parseFloat(col.style.top) })); } catch {}
    },
    update(t) {
      col.style.display = 'flex'; // visible dès la première mesure (rien si la surveillance est coupée)
      cpu.set(t.cpu, [`${t.cpu || 0} %`, `${t.procMb || 0} Mo`]);
      ram.set(t.ramPct, [gb(t.ramUsedGb), gb(t.ramTotalGb)]);
      gpu.hide(!t.gpu);
      syncAll();
      if (t.gpu) gpu.set(t.gpu.pct, [t.gpu.name, `${t.gpu.vramUsedGb} / ${t.gpu.vramTotalGb} G`, t.gpu.temp != null ? `${t.gpu.temp} °C` : '–']);
      if (Array.isArray(t.disks)) setDisks(t.disks);
    }
  };
  const bridge = window.jarvisBridge;
  if (bridge && bridge.onTaskEvent) bridge.onTaskEvent((ev) => { if (ev && ev.type === 'telemetry') J.hwPanels.update(ev); });
})();
