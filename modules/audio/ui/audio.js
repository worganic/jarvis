// Card AUDIO du HUD : sortie (enceintes / casque) et entrée (micro) de Windows — volume, sourdine,
// choix du périphérique, niveau en direct — et voix de Jarvis (volume propre, voix coupée, écoute).
// Même comportement que les cards matériel et Bureau : réduite en un bouton miniature jaune
// (« ▤ AUDIO 28 % ») qui se glisse et se rouvre d'un clic ; état et position mémorisés.
// Module Audio (modules/audio/) : chargé par ui/modules-loader.js quand le module est actif. Données : partie noyau
// du module (modules/audio/core, méthodes audioWatch, audioVolume, audioMute, audioDefault) → processus
// « jarvis-core audio-mixer serve » (événements audio_mixer, audio_peak) ; voix de Jarvis : cœur (jarvis_volume).
(function () {
  const J = (window.JARVIS = window.JARVIS || {});
  const bridge = window.jarvisBridge;
  const KEY = 'jarvis.audioCollapsed';
  const POS = (window.JARVIS_POS_PREFIX || 'jarvis.') + 'audioPanel.pos';

  let collapsed = false;
  try { collapsed = localStorage.getItem(KEY) === '1'; } catch {}

  function el(tag, cls, text) {
    const e = document.createElement(tag);
    if (cls) e.className = cls;
    if (text !== undefined) e.textContent = text;
    return e;
  }

  const root = el('div');
  root.id = 'audio-panel';
  const card = el('div', 'hw-card au-card');
  const hdr = J.cardHeader({ key: 'audio', no: J.CARD_NO.audio, title: 'AUDIO', onCollapse: () => setCollapsed(true) });
  const head = hdr.head;
  head.classList.add('hw-head', 'au-head');
  head.title = 'Glisser : déplacer la card';
  const toggle = hdr.collapse;
  const titleEl = hdr.title;
  const fix = hdr.fix;
  const pinBtn = hdr.pin;
  // Card figée (📌) : elle ne se déplace plus ; état mémorisé.
  let pinned = false;
  try { pinned = localStorage.getItem('jarvis.audioPinned') === '1'; } catch {}
  pinBtn.classList.toggle('on', pinned);
  pinBtn.onclick = (e) => {
    e.stopPropagation();
    pinned = !pinned;
    pinBtn.classList.toggle('on', pinned);
    try { localStorage.setItem('jarvis.audioPinned', pinned ? '1' : '0'); } catch {}
  };
  const body = el('div', 'hw-body au-body');
  const pill = el('button', 'au-pill');
  pill.title = 'Clic : afficher la card audio (glisser : déplacer)';
  const pillLabel = el('span', 'au-pill-l', '▤ AUDIO');
  const pillVal = el('span', 'au-pill-v', '–');
  pill.append(pillLabel, pillVal);
  card.append(head, body, pill);
  root.appendChild(card);
  document.body.appendChild(root);

  function applyTheme(id) {
    try { id = id || localStorage.getItem('jarvis.theme') || 'cyan'; } catch { id = 'cyan'; }
    const theme = J.THEMES && (J.THEMES[id] || J.THEMES.cyan);
    if (theme) root.style.setProperty('--w-rgb', theme.primaryRgb);
  }
  applyTheme();

  // --- Sections : sortie, entrée, Jarvis -------------------------------------------------------------

  // Curseur de volume : envoi limité pendant le glisser, valeur finale au relâchement ; l'état reçu
  // du noyau ne le fait pas sauter tant qu'on le tient.
  function slider(onSet) {
    const row = el('div', 'au-slide');
    const input = el('input', 'au-range');
    input.type = 'range'; input.min = '0'; input.max = '100'; input.step = '1'; input.value = '0';
    const val = el('span', 'au-val', '–');
    row.append(input, val);
    let holding = false;
    let timer = null;
    let lastSent = 0;
    const paint = (v) => {
      input.value = String(v);
      input.style.setProperty('--p', `${v}%`);
      val.textContent = `${v}%`;
    };
    const send = () => {
      clearTimeout(timer);
      timer = null;
      lastSent = Date.now();
      onSet(Number(input.value));
    };
    input.addEventListener('pointerdown', () => { holding = true; });
    input.addEventListener('input', () => {
      paint(Number(input.value));
      if (Date.now() - lastSent > 90) send();
      else if (!timer) timer = setTimeout(send, 90);
    });
    input.addEventListener('change', () => { holding = false; send(); });
    input.addEventListener('pointerup', () => { holding = false; });
    input.addEventListener('wheel', (e) => { // molette : ±2 %
      e.preventDefault();
      paint(Math.max(0, Math.min(100, Number(input.value) + (e.deltaY < 0 ? 2 : -2))));
      if (!timer) timer = setTimeout(send, 150);
    }, { passive: false });
    return {
      row,
      set(v) { if (!holding && !timer) paint(Math.round(Number(v) || 0)); },
      disable(on) { input.disabled = on; row.classList.toggle('off', on); }
    };
  }

  function iconButton(title, onclick) {
    const b = el('button', 'au-btn');
    b.title = title;
    b.onclick = onclick;
    return b;
  }

  // Section d'un périphérique Windows (« out » : sortie, « in » : micro).
  function deviceSection(kind, label) {
    const sec = el('div', `au-sec au-${kind}`);
    const top = el('div', 'au-sec-head');
    const mute = iconButton('', () => bridge && bridge.call('audioMute', kind, 'toggle').then(render).catch(() => {}));
    top.append(el('span', 'au-sec-t', label), mute);
    const select = el('select', 'au-select');
    select.title = kind === 'out' ? 'Sortie son par défaut de Windows' : 'Micro par défaut de Windows';
    select.onchange = () => {
      if (!bridge || !select.value) return;
      select.disabled = true;
      bridge.call('audioDefault', select.value).then(render).catch(() => {}).finally(() => { select.disabled = false; });
    };
    const vol = slider((v) => bridge && bridge.call('audioVolume', kind, Number(v)).catch(() => {}));
    const meter = el('div', 'au-meter');
    const fill = el('div', 'au-meter-fill');
    meter.appendChild(fill);
    sec.append(top, select, vol.row, meter);
    body.appendChild(sec);
    let listSig = '';
    return {
      set(ep, list) {
        const sig = JSON.stringify(list || []);
        if (sig !== listSig && document.activeElement !== select) {
          listSig = sig;
          select.textContent = '';
          for (const d of list || []) {
            const o = el('option', null, d.name);
            o.value = d.id;
            select.appendChild(o);
          }
          if (!(list || []).length) select.appendChild(el('option', null, 'Aucun périphérique'));
        }
        if (ep && ep.id && document.activeElement !== select) select.value = ep.id;
        const ok = Boolean(ep && ep.available);
        vol.disable(!ok);
        if (ok) vol.set(ep.volume);
        const muted = Boolean(ep && ep.muted);
        sec.classList.toggle('muted', muted);
        mute.textContent = kind === 'out' ? (muted ? '🔇' : '🔊') : (muted ? '🚫' : '🎙');
        mute.classList.toggle('on', muted);
        mute.title = kind === 'out'
          ? (muted ? 'Son de Windows coupé. Clic : rétablir' : 'Couper le son de Windows')
          : (muted ? 'Micro coupé dans Windows. Clic : rétablir' : 'Couper le micro dans Windows');
      },
      peak(p) {
        // Échelle perceptive : un murmure reste visible, un son fort ne sature pas tout de suite.
        const pct = Math.min(100, Math.round(Math.pow(Math.max(0, Number(p) || 0), 0.6) * 100));
        fill.style.width = `${sec.classList.contains('muted') ? 0 : pct}%`;
      }
    };
  }

  const out = deviceSection('out', 'SORTIE · ENCEINTES');
  const inp = deviceSection('in', 'ENTRÉE · MICRO');

  // Voix de Jarvis : volume propre (moteur vocal), voix coupée (bouton « son »), écoute (bouton « micro »).
  const jsec = el('div', 'au-sec au-jarvis');
  const jtop = el('div', 'au-sec-head');
  let silence = { mode: 'auto', isMuted: false };
  let micMuted = false;
  const voiceBtn = iconButton('', async () => {
    if (!bridge || !bridge.voiceSilence) return;
    const r = await bridge.voiceSilence().catch(() => null);
    if (r) { silence = { mode: r.mode, isMuted: Boolean(r.muted) }; paintJarvis(); }
  });
  const earBtn = iconButton('', async () => {
    if (!bridge || !bridge.voiceMute) return;
    const r = await bridge.voiceMute(!micMuted).catch(() => null);
    if (r && typeof r.muted === 'boolean') { micMuted = r.muted; paintJarvis(); }
  });
  const testBtn = iconButton('Faire parler Jarvis à ce volume', () => bridge && bridge.jarvisVolume(jvol.value(), true).catch(() => {}));
  testBtn.textContent = '▶';
  const jbtns = el('span', 'au-btns');
  jbtns.append(testBtn, earBtn, voiceBtn);
  jtop.append(el('span', 'au-sec-t', 'VOIX DE JARVIS'), jbtns);
  let jarvisValue = 100;
  const jslide = slider((v) => { jarvisValue = v; if (bridge) bridge.jarvisVolume(v).catch(() => {}); });
  const jvol = { ...jslide, value: () => jarvisValue };
  jsec.append(jtop, jslide.row);
  body.appendChild(jsec);

  function paintJarvis() {
    const silenced = silence.mode === 'on' || silence.isMuted;
    voiceBtn.textContent = silenced ? '🔇' : '🔊';
    voiceBtn.classList.toggle('on', silenced);
    voiceBtn.title = silence.mode === 'on'
      ? 'Voix de Jarvis coupée (il écrit ses réponses). Clic : rétablir'
      : silence.isMuted ? 'Sourdine automatique en cours. Clic : couper la voix en permanence' : 'Couper la voix de Jarvis';
    earBtn.textContent = micMuted ? '🚫' : '🎙';
    earBtn.classList.toggle('on', micMuted);
    earBtn.title = micMuted ? 'Jarvis n’écoute plus. Clic : rétablir l’écoute' : 'Couper l’écoute de Jarvis (le micro reste actif pour Windows)';
    jsec.classList.toggle('muted', silenced);
  }
  paintJarvis();

  // --- État reçu du noyau ----------------------------------------------------------------------------

  let last = null;
  function render(s) {
    if (!s) return;
    if (Number.isFinite(s.jarvisVolume)) { jarvisValue = s.jarvisVolume; jslide.set(s.jarvisVolume); }
    if (!s.ok || !s.out) {
      if (s.error && !last) pillVal.textContent = '—';
      return;
    }
    last = s;
    out.set(s.out, s.outputs);
    inp.set(s.in, s.inputs);
    pillVal.textContent = s.out.muted ? 'MUET' : `${s.out.volume} %`;
    pill.classList.toggle('muted', Boolean(s.out.muted));
  }

  // --- Réduction, glisser-déposer ----------------------------------------------------------------------

  function place(x, y) {
    const w = root.offsetWidth || 150;
    const h = root.offsetHeight || 26;
    root.style.left = `${Math.max(0, Math.min(window.innerWidth - w, x))}px`;
    root.style.top = `${Math.max(0, Math.min(window.innerHeight - Math.min(h, 60), y))}px`;
  }
  // Tant que l'utilisateur ne l'a pas déplacée, la card est « amarrée » : juste sous la card Bureau (ou à son
  // emplacement, en haut à gauche, tant que celle-ci n'existe pas), jamais par-dessus la conversation ni une autre card.
  // Elle suit la card Bureau quand celle-ci grandit, rétrécit ou est déplacée.
  const BUREAU = '.widget.w-card-list.w-compact';
  const BUREAU_TOP = 120; // ancrage de la card Bureau (ui/widgets.js › renderRows)
  const GAP = 8;
  let docked = true;
  function dock() {
    if (!docked) return;
    const b = document.querySelector(BUREAU);
    const y = b ? Math.round(b.getBoundingClientRect().bottom) + GAP : BUREAU_TOP;
    const left = b ? Math.round(b.getBoundingClientRect().left) : 6;
    const cx = parseFloat(root.style.left);
    const cy = parseFloat(root.style.top);
    if (!Number.isFinite(cx) || !Number.isFinite(cy) || Math.abs(cy - y) >= 1 || Math.abs(cx - left) >= 1) place(left, y);
    fitBelow();
  }
  // Amarrée et trop haute pour la place restante : défilement interne plutôt que de recouvrir la card Bureau.
  function fitBelow() {
    const room = window.innerHeight - (parseFloat(root.style.top) || 0) - GAP;
    card.style.maxHeight = docked && !collapsed && room > 120 ? `${Math.floor(room)}px` : '';
    card.style.overflowY = docked && !collapsed ? 'auto' : '';
  }
  function restore() {
    let s = null;
    try { s = JSON.parse(localStorage.getItem(POS) || 'null'); } catch {}
    if (s && Number.isFinite(s.x) && Number.isFinite(s.y)) {
      docked = false; // position choisie par l'utilisateur
      place(s.x, s.y);
    } else {
      dock();
    }
  }
  setInterval(() => { if (!document.hidden) dock(); }, 400);
  // Card ouverte qui déborde en bas : remontée pour rester entière (la position enregistrée ne change pas).
  function keepInView() {
    if (docked) return fitBelow();
    const r = root.getBoundingClientRect();
    if (r.bottom > window.innerHeight - 8) root.style.top = `${Math.max(0, window.innerHeight - 8 - r.height)}px`;
  }

  function sync() {
    card.classList.toggle('collapsed', collapsed);
    root.classList.toggle('collapsed', collapsed);
    if (!collapsed) requestAnimationFrame(keepInView);
    lease();
  }
  function setCollapsed(on) {
    collapsed = on;
    try { localStorage.setItem(KEY, on ? '1' : '0'); } catch {}
    if (on) { out.peak(0); inp.peak(0); }
    sync();
  }

  // Poignées : l'en-tête (card ouverte) et le bouton miniature (card réduite).
  function draggable(handle, onClick) {
    let drag = null;
    let moved = false;
    handle.addEventListener('pointerdown', (e) => {
      if (e.button !== 0 || pinned || e.target.closest('button')) return;
      const r = root.getBoundingClientRect();
      drag = { dx: e.clientX - r.left, dy: e.clientY - r.top, x: e.clientX, y: e.clientY };
      moved = false;
    });
    handle.addEventListener('pointermove', (e) => {
      if (!drag) return;
      if (!moved && Math.hypot(e.clientX - drag.x, e.clientY - drag.y) < 5) return;
      if (!moved) { try { handle.setPointerCapture(e.pointerId); } catch {} } // capturé une fois glissé : le clic sur le titre arrive au titre
      moved = true;
      docked = false;
      card.style.maxHeight = '';
      card.style.overflowY = '';
      place(e.clientX - drag.dx, e.clientY - drag.dy);
    });
    handle.addEventListener('pointerup', () => {
      if (drag && moved) {
        try { localStorage.setItem(POS, JSON.stringify({ x: parseFloat(root.style.left), y: parseFloat(root.style.top) })); } catch {}
      }
      drag = null;
    });
    if (onClick) handle.addEventListener('click', () => { if (moved) { moved = false; return; } onClick(); });
  }
  draggable(head, null);
  // Clic sur le titre (sans glisser) : la card se réduit en bouton.
  let titleDown = null;
  titleEl.style.cursor = 'pointer';
  titleEl.addEventListener('pointerdown', (e) => { titleDown = { x: e.clientX, y: e.clientY }; });
  titleEl.addEventListener('click', (e) => { if (!titleDown || Math.hypot(e.clientX - titleDown.x, e.clientY - titleDown.y) <= 5) setCollapsed(true); });
  draggable(pill, () => setCollapsed(false));
  window.addEventListener('resize', () => {
    if (docked) dock();
    else if (root.style.left) place(parseFloat(root.style.left) || 0, parseFloat(root.style.top) || 0);
    if (!collapsed) keepInView();
  });

  // --- Bail auprès du noyau : suivi tant que la page est visible, mesures si la card est ouverte ------

  let visible = !document.hidden;
  let leaseTimer = null;
  function lease() {
    clearTimeout(leaseTimer);
    if (!bridge || !bridge.call) return;
    if (!visible) { bridge.call('audioWatch', false, false).catch(() => {}); return; }
    bridge.call('audioWatch', true, !collapsed || Boolean(J.musicMeter)).then(render).catch(() => {});
    leaseTimer = setTimeout(lease, 10000);
  }
  document.addEventListener('visibilitychange', () => { visible = !document.hidden; lease(); });
  if (bridge && bridge.onVisibility) bridge.onVisibility((v) => { if (v !== visible) { visible = v; lease(); } });

  if (bridge && bridge.onTaskEvent) {
    bridge.onTaskEvent((ev) => {
      if (!ev) return;
      switch (ev.type) {
        case 'audio_mixer': render(ev); break;
        case 'audio_peak': if (!collapsed) { out.peak(ev.out); inp.peak(ev.in); } break;
        case 'jarvis_volume': jarvisValue = ev.value; jslide.set(ev.value); break;
        case 'status':
          if (ev.contextMute) { silence = { ...silence, ...ev.contextMute }; paintJarvis(); }
          if (ev.theme) applyTheme(ev.theme);
          break;
        case 'context_mute': silence = { ...silence, ...ev }; paintJarvis(); break;
        case 'voice_state':
          if (typeof ev.muted === 'boolean' && ev.connected !== false) { micMuted = ev.muted; paintJarvis(); }
          break;
        case 'theme_config': applyTheme(ev.theme); break;
        default: break;
      }
    });
  }
  if (bridge && bridge.voiceState) {
    bridge.voiceState().then((s) => { if (s && typeof s.muted === 'boolean') { micMuted = s.muted; paintJarvis(); } }).catch(() => {});
  }

  restore();
  sync();
  // Bouton « ranger » (cards-dock.js) : pose la pastille à un endroit choisi (position mémorisée, plus amarrée).
  function moveTo(x, y) {
    docked = false;
    card.style.maxHeight = '';
    card.style.overflowY = '';
    place(x, y);
    try { localStorage.setItem(POS, JSON.stringify({ x: parseFloat(root.style.left), y: parseFloat(root.style.top) })); } catch {}
  }
  // Emplacement par défaut (bouton 📍) et retour à celui-ci (bouton « PLACER », cards-dock.js).
  const DKEY = () => (window.JARVIS_POS_PREFIX || 'jarvis.') + 'defaultPos.audio';
  const savedDefault = () => { try { const p = JSON.parse(localStorage.getItem(DKEY()) || 'null'); return p && Number.isFinite(p.x) && Number.isFinite(p.y) ? p : null; } catch { return null; } };
  const paintFix = () => fix.classList.toggle('on', Boolean(savedDefault()));
  fix.onclick = (e) => {
    e.stopPropagation();
    const r = root.getBoundingClientRect();
    const p = savedDefault();
    try {
      if (e.shiftKey) localStorage.removeItem(DKEY());
      else if (!p) localStorage.setItem(DKEY(), JSON.stringify({ x: Math.round(r.left), y: Math.round(r.top) })); // pas fixé : on fixe ici
      else if (Math.abs(r.left - p.x) <= 12 && Math.abs(r.top - p.y) <= 12) localStorage.removeItem(DKEY()); // déjà là : on désactive
      else moveTo(p.x, p.y); // ailleurs : on la replace
    } catch {}
    paintFix();
  };
  paintFix();
  function restoreDefault() {
    const p = savedDefault();
    if (!p) return 0;
    moveTo(p.x, p.y);
    return 1;
  }
  // { total, there } : 1 si la card a un emplacement par défaut, et si elle y est déjà (2e clic sur « PLACER »).
  function defaultState() {
    const p = savedDefault();
    if (!p) return { total: 0, there: 0 };
    return { total: 1, there: Math.abs((parseFloat(root.style.left) || 0) - p.x) <= 12 && Math.abs((parseFloat(root.style.top) || 0) - p.y) <= 12 ? 1 : 0 };
  }
  function openDefault() {
    if (!savedDefault() || !collapsed) return 0;
    setCollapsed(false);
    return 1;
  }
  // Vue figée (cards-dock.js) : emplacement et état.
  function layoutSnapshot() {
    const r = root.getBoundingClientRect();
    return { x: Math.round(r.left), y: Math.round(r.top), collapsed };
  }
  function layoutApply(s) {
    if (!s || !Number.isFinite(s.x) || !Number.isFinite(s.y)) return 0;
    moveTo(s.x, s.y);
    setCollapsed(Boolean(s.collapsed));
    return 1;
  }
  J.audioPanel = { render, setCollapsed, moveTo, restoreDefault, defaultState, openDefault, layoutSnapshot, layoutApply, element: root };
})();
