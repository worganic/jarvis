// En-tête commun des cards du HUD, identique partout : poignée ⠿, numéro, titre, puis 📍 (emplacement par défaut),
// 📌 (figer) et – (réduire en bouton). Les cards de widgets.js (Bureau, Terminal, mails…) ont le même dessin ; ce
// constructeur sert aux cards construites hors de widgets.js (Audio, Matériel, Actions).
// Numéros : le noyau numérote les cards qu'il ouvre (1, 2, 3… « ferme la card 2 ») ; les cards propres au HUD
// reçoivent un numéro fixe (J.CARD_NO) : type 1 de 1 à 99, type 2 de 100 à 199.
(function () {
  const J = (window.JARVIS = window.JARVIS || {});

  // Couleurs des cards (réglées dans l'administration, diffusées par le noyau) : type 1 = cards toujours actives,
  // type 2 = cards temporaires ; n = ouverte, r = réduite en bouton. t1n vide : couleur du thème.
  const COLOR_DEFAULTS = { t1n: '', t1r: '#ffd640', t2n: '#be8cff', t2r: '#be8cff' };
  const rgbOf = (hex) => (/^#[0-9a-f]{6}$/i.test(hex || '') ? [1, 3, 5].map((i) => parseInt(hex.slice(i, i + 2), 16)).join(', ') : '');
  J.cardColors = { ...COLOR_DEFAULTS };
  J.cardColorsRgb = () => ({ t1n: rgbOf(J.cardColors.t1n), t1r: rgbOf(J.cardColors.t1r) || '255, 214, 64', t2n: rgbOf(J.cardColors.t2n), t2r: rgbOf(J.cardColors.t2r) || '190, 140, 255' });
  J.applyCardColors = function (colors) {
    const c = {};
    for (const k of Object.keys(COLOR_DEFAULTS)) c[k] = colors && typeof colors[k] === 'string' ? colors[k] : COLOR_DEFAULTS[k];
    J.cardColors = c;
    const rgb = J.cardColorsRgb();
    const root = document.documentElement;
    root.style.setProperty('--t1-rgb', rgb.t1n || '56, 225, 255');
    root.style.setProperty('--t1r-rgb', rgb.t1r);
    root.style.setProperty('--t2-rgb', rgb.t2n || '190, 140, 255');
    root.style.setProperty('--t2r-rgb', rgb.t2r);
    root.classList.toggle('c-t1n', Boolean(rgb.t1n)); // type 1 ouvert : couleur du thème tant qu'aucune n'est choisie
    root.classList.toggle('c-t2n', Boolean(rgb.t2n));
    try { localStorage.setItem('jarvis.cardColors', JSON.stringify(c)); } catch {}
  };
  try { J.applyCardColors(JSON.parse(localStorage.getItem('jarvis.cardColors') || 'null') || {}); } catch { J.applyCardColors({}); }
  if (window.jarvisBridge && window.jarvisBridge.onTaskEvent) {
    window.jarvisBridge.onTaskEvent((ev) => {
      if (!ev) return;
      if (ev.type === 'card_colors_config') J.applyCardColors(ev.colors || {});
      else if (ev.type === 'status' && ev.cardColors) J.applyCardColors(ev.cardColors);
    });
  }

  // Cards de type 1 (toujours actives) : 1 à 99, fixes. Cards de type 2 (temporaires) : 100 à 199 ; le noyau numérote celles
  // qu'il ouvre à partir de 101, la musique (HUD) a le 100.
  const NO_DEFAULTS = { bureau: 1, audio: 2, hw: 3, conversation: 4, actions: 5, music: 100 };
  J.CARD_NO = { ...NO_DEFAULTS };
  // Numéros choisis dans l'administration (clé de card → numéro) : les cards du HUD les prennent aussitôt ; celles
  // du noyau reçoivent le leur avec leur prochain affichage.
  // Cards temporaires connues : numéro par défaut de leur emplacement (le même que dans le noyau, cards.rs).
  const SLOT_DEFAULTS = { 'tool:Terminal': 101, mail: 102, agenda: 103, note: 104, meeting: 105, 'tool:Mes listes': 106, 'tool:notes': 107, 'tool:reunions': 108, 'tool:projets': 109 };
  let custom = {};
  J.cardSlotNo = (slot) => (Number.isFinite(custom[slot]) ? custom[slot] : (SLOT_DEFAULTS[slot] ?? NO_DEFAULTS[slot]));
  J.applyCardNumbers = function (map) {
    const m = map && typeof map === 'object' ? map : {};
    custom = m;
    for (const k of Object.keys(NO_DEFAULTS)) J.CARD_NO[k] = Number.isFinite(m[k]) ? m[k] : NO_DEFAULTS[k];
    for (const badge of document.querySelectorAll('[data-cardno]')) {
      const n = J.CARD_NO[badge.dataset.cardno];
      if (Number.isFinite(n)) badge.textContent = String(n);
    }
    try { localStorage.setItem('jarvis.cardNumbers', JSON.stringify(m)); } catch {}
    document.dispatchEvent(new CustomEvent('jarvis:cardnumbers'));
  };
  try { J.applyCardNumbers(JSON.parse(localStorage.getItem('jarvis.cardNumbers') || 'null')); } catch {}
  // Numéro par défaut donné par la fiche d'un module (modules/<id>/module.json › card.numero), avant le chargement de
  // son interface ; un numéro choisi dans l'administration reste prioritaire.
  J.setCardDefault = function (key, n) {
    if (!key || !Number.isFinite(n)) return;
    SLOT_DEFAULTS[key] = n; // card ouverte par le noyau (emplacement « note », « tool:Terminal »…) ou propre au HUD
    NO_DEFAULTS[key] = n;
    if (!Number.isFinite(custom[key])) J.CARD_NO[key] = n;
    document.dispatchEvent(new CustomEvent('jarvis:cardnumbers'));
  };
  if (window.jarvisBridge && window.jarvisBridge.onTaskEvent) {
    window.jarvisBridge.onTaskEvent((ev) => {
      if (!ev) return;
      if (ev.type === 'card_numbers_config') J.applyCardNumbers(ev.numbers || {});
      else if (ev.type === 'status' && ev.cardNumbers) J.applyCardNumbers(ev.cardNumbers);
    });
  }

  function el(tag, cls, text) {
    const e = document.createElement(tag);
    if (cls) e.className = cls;
    if (text !== undefined) e.textContent = text;
    return e;
  }

  function grip() {
    const g = el('div', 'w-grip');
    for (let r = 0; r < 2; r++) {
      const row = el('div', 'w-grip-row');
      row.append(el('div', 'w-grip-dot'), el('div', 'w-grip-dot'));
      g.appendChild(row);
    }
    return g;
  }

  // opts : { no, key, title, onCollapse, onClose } ; rend { head, title, fix, pin, collapse, close }.
  J.cardHeader = function (opts) {
    const head = el('div', 'w-head hw-hdr');
    const title = el('span', 'w-title', opts.title);
    const fix = el('button', 'w-fix', '📍');
    fix.title = 'Fixer : garde l’emplacement de la card. Quand il est actif : un clic la replace à cet emplacement, un second clic (elle y est déjà) désactive le fixage, puis on recommence (Maj + clic : l’oublier) · « PLACER » y remet toutes les cards';
    const pin = el('button', 'w-pin', '📌');
    pin.title = 'Figer la card (elle ne se déplace plus)';
    const collapse = el('button', 'w-collapse', '–');
    collapse.title = 'Réduire en un bouton';
    if (opts.onCollapse) collapse.onclick = (e) => { e.stopPropagation(); opts.onCollapse(); };
    const num = el('span', 'w-num', String(opts.no));
    if (opts.key) num.dataset.cardno = opts.key; // mis à jour quand le numéro change dans l'administration
    head.append(grip(), num, title, fix, pin, collapse);
    let close = null;
    if (opts.onClose) {
      close = el('button', 'w-close', '×');
      close.title = 'Fermer';
      close.onclick = (e) => { e.stopPropagation(); opts.onClose(); };
      head.append(close);
    }
    return { head, title, fix, pin, collapse, close };
  };
})();
