// Module « Compagnon » (card 123) : le compagnon de bureau (tamagotchi) — stade et jour du cycle, branche et XP
// des 4 branches, faim / humeur / énergie, effets du moment, historique et souvenirs des générations passées.
// Boutons : veille / réveil, nouvel œuf (deux clics), activer / désactiver (désactivé : le cœur classique revient). L'état vient du noyau
// (orch/companion, méthode « compagnon ») et de l'événement « companion ».
(function () {
  const J = (window.JARVIS = window.JARVIS || {});
  const bridge = window.jarvisBridge;
  const root = document.getElementById('widgets');
  if (!root || !J.cardHeader || !J.widgets) return;

  const KEY = 'compagnon';
  if (J.widgetKit) {
    J.widgetKit.ownKey(KEY);
    J.widgetKit.vanish(KEY);
  }
  const MARGIN = 12;
  const ui = () => parseFloat(document.documentElement.style.getPropertyValue('--ui')) || 1;
  const call = (op, p = {}) => (bridge && bridge.call ? bridge.call('compagnon', op, p) : Promise.resolve({ ok: false }));

  const BRANCHES = [['gamer', '⚔ Gamer'], ['dev', '☕ Dev'], ['artiste', '✎ Artiste'], ['chill', '♫ Chill']];
  const EFFECTS = {
    asleep: 'dort (ne pas déranger)', stealth: 'mode furtif', meeting: 'chut, réunion', hot: 'PC chaud', night: 'nuit blanche',
    afk: 'tu es absent', offline: 'sans réseau', lowbattery: 'batterie faible', music: 'musique', hungry: 'a faim', sulky: 'boude', tired: 'fatigué'
  };

  function h(tag, attrs = {}, ...children) {
    const e = document.createElement(tag);
    for (const [k, v] of Object.entries(attrs || {})) {
      if (v === undefined || v === null || v === false) continue;
      if (k === 'class') e.className = v;
      else if (k.startsWith('on') && typeof v === 'function') e.addEventListener(k.slice(2), v);
      else e.setAttribute(k, v === true ? '' : v);
    }
    for (const c of children.flat()) if (c !== null && c !== undefined && c !== false) e.append(c instanceof Node ? c : document.createTextNode(String(c)));
    return e;
  }

  let state = null;
  let confirmEgg = 0;

  const card = h('div', { class: 'widget w-tool w-card-list w-compagnon' });
  const head = h('div', { class: 'w-head' },
    J.widgetKit && J.widgetKit.gripHandle ? J.widgetKit.gripHandle() : null,
    h('span', { class: 'w-title' }, 'COMPAGNON'),
    h('button', { class: 'w-close', title: 'Fermer', onclick: (e) => { e.stopPropagation(); J.widgets.close(KEY); } }, '×'));
  const body = h('div', { class: 'cp-body' });
  card.append(head, body);
  if (J.widgetKit) {
    if (J.widgetKit.makeDraggable) J.widgetKit.makeDraggable(card, KEY);
    if (J.widgetKit.normalizeHead) J.widgetKit.normalizeHead(KEY, card);
  }

  function placeDefault() {
    const S = ui();
    let saved = null;
    try { saved = JSON.parse(localStorage.getItem((window.JARVIS_POS_PREFIX || 'jarvis.') + 'widgetPos.' + KEY) || 'null'); } catch { saved = null; }
    card.style.left = `${saved && Number.isFinite(saved.x) ? saved.x : Math.max(MARGIN, Math.round(window.innerWidth - 400 * S - 40))}px`;
    card.style.top = `${saved && Number.isFinite(saved.y) ? saved.y : Math.max(MARGIN, Math.round(90 * S))}px`;
  }

  const bar = (label, value, cls = '') => h('div', { class: 'cp-bar ' + cls },
    h('span', { class: 'cp-bar-l' }, label),
    h('span', { class: 'cp-bar-track' }, h('span', { class: 'cp-bar-fill', style: `width:${Math.max(0, Math.min(100, value))}%` })),
    h('span', { class: 'cp-bar-v' }, `${Math.round(value)}`));

  const ago = (ms) => {
    const s = Math.max(0, (Date.now() - ms) / 1000);
    if (s < 3600) return `il y a ${Math.max(1, Math.round(s / 60))} min`;
    if (s < 86400) return `il y a ${Math.round(s / 3600)} h`;
    return new Date(ms).toLocaleDateString('fr-FR', { day: 'numeric', month: 'short' });
  };

  function render() {
    body.replaceChildren();
    const s = state;
    if (!s) {
      body.append(h('div', { class: 'cp-empty' }, 'Chargement…'));
      return;
    }
    if (!s.enabled) {
      body.append(
        h('div', { class: 'cp-empty' }, 'Le compagnon de bureau est désactivé. Activé, le cœur choisi naît d’un œuf et évolue selon l’usage du PC (7 ou 30 jours), sans jamais passer devant Jarvis.'),
        h('div', { class: 'cp-actions' }, h('button', { class: 'cp-btn primary', onclick: async () => { await call('regler', { enabled: true }); refresh(); } }, '◉ Activer le compagnon')));
      return;
    }
    const xp = s.xp || {};
    const maxXp = Math.max(1, ...BRANCHES.map(([b]) => xp[b] || 0));
    body.append(
      h('div', { class: 'cp-top' },
        h('div', { class: 'cp-stage' }, s.stageLabel || s.stage, s.branchLabel ? h('small', {}, ' · ' + s.branchLabel) : null),
        h('div', { class: 'cp-day' }, `Jour ${s.day} / ${s.cycle} · génération ${s.generation}`)),
      h('div', { class: 'cp-desc' }, s.describe || ''),
      h('div', { class: 'cp-cycle' }, h('span', { style: `width:${Math.min(100, (s.day / s.cycle) * 100)}%` })),
      s.stage === 'oeuf' ? bar('Chaleur', s.warmth || 0, 'warm') : null,
      bar('Faim', s.hunger, s.hunger < 25 ? 'low' : ''),
      bar('Humeur', s.mood, s.mood < 25 ? 'low' : ''),
      bar('Énergie', s.energy, s.energy < 20 ? 'low' : ''),
      h('div', { class: 'cp-sec' }, 'Branches (minutes passées au premier plan)'),
      ...BRANCHES.map(([b, label]) => bar(label, ((xp[b] || 0) / maxXp) * 100, (s.branch || s.leading) === b ? 'lead' : '')),
      (s.effects || []).length ? h('div', { class: 'cp-chips' }, ...s.effects.map((e) => h('span', { class: 'cp-chip' }, EFFECTS[e] || e))) : null,
      h('div', { class: 'cp-actions' },
        h('button', { class: 'cp-btn', onclick: async () => { await call('veille', { on: !s.asleep }); refresh(); } }, s.asleep ? '☀ Réveiller' : '☾ Ne pas déranger'),
        h('button', { class: 'cp-btn danger', title: 'Il part en voyage (souvenir gardé) et laisse un nouvel œuf', onclick: async (e) => {
          if (Date.now() - confirmEgg > 4000) {
            confirmEgg = Date.now();
            e.target.textContent = 'Confirmer ?';
            return;
          }
          confirmEgg = 0;
          await call('oeuf');
          refresh();
        } }, '⟲ Nouvel œuf'),
        h('button', { class: 'cp-btn', title: 'Le cœur reprend son apparence et ses animations habituelles ; le compagnon est gardé tel quel pour la prochaine activation', onclick: async () => { await call('regler', { enabled: false }); refresh(); } }, '⏻ Désactiver')),
      h('div', { class: 'cp-hint' }, 'Glisse un fichier sur le cœur (widget ou Bureau JARVIS) pour le nourrir (seule l’extension compte, le fichier n’est ni lu ni modifié). Fais tourner la souris autour de lui pour le caresser ; double-clic du bouton du milieu : ne pas déranger.'));
    const hist = s.history || [];
    if (hist.length) body.append(h('div', { class: 'cp-sec' }, 'Historique'), h('div', { class: 'cp-list' }, ...hist.slice(0, 8).map((x) => h('div', { class: 'cp-row' }, h('span', {}, x.text), h('small', {}, ago(x.at))))));
    const souv = s.souvenirs || [];
    if (souv.length) {
      body.append(h('div', { class: 'cp-sec' }, 'Souvenirs'), h('div', { class: 'cp-list' }, ...souv.slice(0, 6).map((x) =>
        h('div', { class: 'cp-row' }, h('span', {}, `Génération ${x.generation} · ${(BRANCHES.find(([b]) => b === x.branch) || [, x.branch])[1]} · ${x.days} jours`), h('small', {}, x.retired_at ? ago(x.retired_at) : '')))));
    }
  }

  async function refresh() {
    const r = await call('etat');
    if (r && r.stage) {
      state = r;
      render();
    }
  }

  function open() {
    let rec = J.widgets.cards.get(KEY);
    if (!rec || !rec.el || !rec.el.isConnected) {
      root.appendChild(card);
      placeDefault();
      rec = { el: card, timer: null, kind: KEY, state: 'active', fadeTimer: null, persist: true, lastActive: performance.now() };
      J.widgets.cards.set(KEY, rec);
    }
    clearTimeout(rec.fadeTimer);
    if (J.widgetKit && J.widgetKit.setFolded) J.widgetKit.setFolded(KEY, false);
    card.classList.remove('w-ghost', 'w-fading', 'w-collapsed', 'w-leaving');
    card.classList.add('w-active');
    rec.state = 'active';
    rec.lastActive = performance.now();
    if (J.widgetKit && J.widgetKit.keepInView) J.widgetKit.keepInView(card);
    if (J.cardFront && J.cardFront.front) J.cardFront.front(card);
    render();
    refresh();
  }

  if (bridge && bridge.onTaskEvent) {
    bridge.onTaskEvent((ev) => {
      if (!ev) return;
      if (ev.type === 'companion_open') open();
      else if (ev.type === 'companion') {
        state = ev;
        if (card.isConnected && !card.classList.contains('w-ghost')) render();
      }
    });
  }

  J.compagnonCard = { open, refresh };
})();
