// Module Checkup : la card « Checkup » (fonctions vitales de Jarvis, orch/checkup.rs) avec le cœur passé au scanner
// holographique (ui/checkup-3d.js du module, J.checkupScan). Le noyau envoie une card « tool » marquée checkup.
(function () {
  const J = (window.JARVIS = window.JARVIS || {});
  const bridge = window.jarvisBridge;
  const K = J.widgetKit;
  if (!K) return;
  const { h, place, cards, keepInView, gripHandle, numBadge, pinButton, closeButton } = K;

  // --- Card « Checkup » : le cœur passé au scanner (ui/checkup.js), fonctions vitales de Jarvis ----------
  // Hologramme à gauche, fonctions vitales à droite : chaque ligne apparaît quand le faisceau passe sur son
  // organe, reliée à lui par un trait. Rafraîchie toutes les 15 s par le noyau (ev.refresh : le scan ne se
  // rejoue pas) ; « checkup » redit pendant qu'elle est ouverte : nouveau scan.
  const CHECKUP_ORDER = ['memory', 'brain', 'eyes', 'ears', 'mouth', 'heart', 'spine', 'armR', 'armL', 'legs', 'nerves'];
  const CHECKUP_ICON = { memory: '💾', brain: '🧠', eyes: '👁', ears: '👂', mouth: '🗣', heart: '❤', spine: '🦴', armR: '🔌', armL: '🛠', legs: '⚙', nerves: '⚡' };
  const CHECKUP_STATE = { ok: 'OK', warn: 'À SURVEILLER', error: 'EN PANNE' };
  function renderCheckup(ev) {
    const key = `tool:${ev.title || 'Checkup'}`;
    const prev = cards.get(key);
    if (prev && prev.state === 'active' && prev.el && prev.el.isConnected && prev.el._checkup) {
      prev.el._checkup.update(ev);
      if (!ev.refresh) prev.el._checkup.rescan();
      return;
    }
    const sorted = (list) => [...(list || [])].sort((a, b) => CHECKUP_ORDER.indexOf(a.id) - CHECKUP_ORDER.indexOf(b.id));
    const canvas = h('canvas', { class: 'w-ck-canvas' });
    const phaseEl = h('div', { class: 'w-ck-phase' }, 'INITIALISATION DU SCANNER');
    const scoreEl = h('span', { class: 'w-ck-score' }, '…');
    const svg = document.createElementNS('http://www.w3.org/2000/svg', 'svg');
    svg.setAttribute('class', 'w-ck-links');
    const stage = h('div', { class: 'w-ck-stage' }, canvas, h('div', { class: 'w-ck-grid' }), phaseEl,
      h('span', { class: 'w-ck-corner tl' }), h('span', { class: 'w-ck-corner tr' }), h('span', { class: 'w-ck-corner bl' }), h('span', { class: 'w-ck-corner br' }));
    const listEl = h('div', { class: 'w-ck-list' });
    const body = h('div', { class: 'w-ck-body' }, stage, listEl, svg);
    const footEl = h('div', { class: 'w-rows-foot' }, 'Chaque partie du corps est une fonction de Jarvis · ↻ relancer le scan · « Jarvis, checkup »');
    const rowsById = {};
    let data = ev;
    let revealed = new Set();
    let scan = null;

    function fillRows() {
      listEl.textContent = '';
      for (const v of sorted(data.vitals)) {
        const row = h('div', { class: `w-ck-vital state-${v.state}${revealed.has(v.id) ? '' : ' pending'}`, title: v.detail || '' },
          h('span', { class: 'w-ck-ico' }, CHECKUP_ICON[v.id] || '•'),
          h('span', { class: 'w-ck-text' },
            h('span', { class: 'w-ck-name' }, h('strong', {}, v.label || ''), ` · ${v.organ || ''}`),
            h('span', { class: 'w-ck-detail' }, v.detail || '')),
          h('span', { class: 'w-ck-side' },
            h('span', { class: 'w-ck-badge' }, CHECKUP_STATE[v.state] || ''),
            h('span', { class: 'w-ck-value' }, v.value || '')));
        rowsById[v.id] = row;
        listEl.appendChild(row);
      }
    }
    function showScore() {
      const all = (data.vitals || []).every((v) => revealed.has(v.id));
      scoreEl.className = `w-ck-score state-${all ? data.state || 'ok' : 'scan'}`;
      scoreEl.textContent = all ? `${data.score} %` : '…';
      if (all) phaseEl.textContent = `BILAN · ${data.score} % · ${data.state === 'ok' ? 'TOUS SYSTÈMES NOMINAUX' : data.state === 'warn' ? 'FONCTIONS À SURVEILLER' : 'DÉFAILLANCE DÉTECTÉE'}`;
    }
    function revealAll() {
      for (const v of data.vitals || []) revealed.add(v.id);
      fillRows();
      showScore();
    }
    // Traits : de l'organe (projeté à l'écran par le scanner) au début de sa ligne.
    function drawLinks(info) {
      const bodyRect = body.getBoundingClientRect();
      if (!bodyRect.width) return;
      const k = body.offsetWidth / bodyRect.width;
      const cr = canvas.getBoundingClientRect();
      const ox = (cr.left - bodyRect.left) * k;
      const oy = (cr.top - bodyRect.top) * k;
      const parts = [];
      for (const [id, p] of Object.entries(info.points || {})) {
        const row = rowsById[id];
        if (!row || !revealed.has(id)) continue;
        const rr = row.getBoundingClientRect();
        const x1 = ox + p[0];
        const y1 = oy + p[1];
        const x2 = (rr.left - bodyRect.left) * k;
        const y2 = (rr.top - bodyRect.top) * k + row.offsetHeight / 2;
        const mid = Math.max(x1 + 14, x2 - 22);
        const st = ((data.vitals || []).find((v) => v.id === id) || {}).state || 'ok';
        parts.push(`<polyline class="state-${st}" points="${x1.toFixed(1)},${y1.toFixed(1)} ${mid.toFixed(1)},${y1.toFixed(1)} ${(mid + 10).toFixed(1)},${y2.toFixed(1)} ${x2.toFixed(1)},${y2.toFixed(1)}"/><circle class="state-${st}" cx="${x1.toFixed(1)}" cy="${y1.toFixed(1)}" r="2.6"/>`);
      }
      svg.setAttribute('viewBox', `0 0 ${body.offsetWidth} ${body.offsetHeight}`);
      svg.innerHTML = parts.join('');
    }

    const rescanBtn = h('button', { class: 'w-head-btn', title: 'Relancer le scan', onclick: (e) => { e.stopPropagation(); if (scan) scan.rescan(); } }, '↻');
    // Réduite : seulement le personnage sur le fond de Jarvis (card à 40 %), toujours reliée au cœur.
    let mini = false;
    let home = null; // place de la card avant d'être rangée près des cards matériel
    try { mini = localStorage.getItem('jarvis.checkupMini') === '1'; } catch {}
    const el = h('div', { class: `widget w-tool w-card-list w-checkup${mini ? ' w-ck-mini' : ''}` });
    const miniBtn = h('button', {
      class: 'w-ck-minibtn', title: mini ? 'Afficher le bilan complet' : 'Réduire au personnage',
      onclick: (e) => {
        e.stopPropagation();
        mini = !el.classList.contains('w-ck-mini');
        if (mini) home = { left: el.style.left, top: el.style.top };
        el.classList.toggle('w-ck-mini', mini);
        armDim();
        if (mini) dock();
        else if (home) {
          el.style.left = home.left;
          el.style.top = home.top;
          keepInView(el);
        }
        miniBtn.title = mini ? 'Afficher le bilan complet' : 'Réduire au personnage';
        try { localStorage.setItem('jarvis.checkupMini', mini ? '1' : '0'); } catch {}
        if (J.invalidateWidgetBoxes) J.invalidateWidgetBoxes(); // la liaison au cœur suit la nouvelle taille
      }
    });
    el.append(
      h('div', { class: 'w-head' },
        gripHandle(),
        numBadge(ev),
        h('span', { class: 'w-title' }, ev.title || 'Checkup'),
        scoreEl,
        rescanBtn,
        miniBtn,
        pinButton(el, key),
        // persist ne sert qu'à éviter l'effacement pour inactivité : le × ferme vraiment la card (pas de réduction).
        closeButton({ ...ev, persist: false }, key)),
      body, footEl);
    fillRows();
    el._checkup = {
      update(next) {
        data = next;
        fillRows();
        showScore();
        if (scan) scan.update(next.vitals);
      },
      rescan() { if (scan) scan.rescan(); }
    };
    place(key, el, ev.kind, 0);
    if (mini) home = { left: el.style.left, top: el.style.top };

    // Réduite : elle se range d'elle-même à gauche de la colonne des cards matériel (hw-panels.js), à
    // hauteur de sa première card ouverte, et la suit quand celle-ci bouge ou change de taille.
    function dock() {
      if (!el.isConnected || !el.classList.contains('w-ck-mini') || el.classList.contains('is-dragging')) return;
      const col = document.getElementById('hw-panels');
      if (!col || col.style.display === 'none' || !col.offsetWidth) return;
      const S = parseFloat(document.documentElement.style.getPropertyValue('--ui')) || 1;
      const open = [...col.querySelectorAll('.hw-card')].find((c) => c.offsetParent && !c.classList.contains('collapsed'));
      const r = col.getBoundingClientRect();
      const x = Math.max(6, Math.round(r.left - el.offsetWidth - 10 * S));
      const y = Math.max(6, Math.round((open || col).getBoundingClientRect().top));
      if (Math.abs((parseFloat(el.style.left) || 0) - x) < 1 && Math.abs((parseFloat(el.style.top) || 0) - y) < 1) return;
      el.style.left = `${x}px`;
      el.style.top = `${y}px`;
      const rec = cards.get(key);
      if (rec) rec._cachedBox = null; // la liaison au cœur suit
    }
    const dockTimer = setInterval(() => {
      if (!el.isConnected) clearInterval(dockTimer);
      else dock();
    }, 600);
    if (mini) requestAnimationFrame(dock);
    // Réduite depuis 30 s : la card s'efface à 25 % (40 % au survol).
    let dimTimer = null;
    function armDim() {
      clearTimeout(dimTimer);
      el.classList.remove('w-ck-dim');
      if (el.classList.contains('w-ck-mini')) dimTimer = setTimeout(() => { if (el.isConnected) el.classList.add('w-ck-dim'); }, 30000);
    }
    armDim();

    const onPhase = (kind, id) => {
      if (kind === 'spin') {
        revealed = new Set();
        fillRows();
        svg.innerHTML = '';
        phaseEl.textContent = 'ROTATION 360° · ACQUISITION DE L’HOLOGRAMME';
        showScore();
      } else if (kind === 'analyse') {
        phaseEl.textContent = 'ANALYSE DES FONCTIONS VITALES';
      } else if (kind === 'reveal' && id) {
        revealed.add(id);
        const row = rowsById[id];
        if (row) {
          row.classList.remove('pending');
          row.classList.add('w-ck-in');
        }
        showScore();
      }
    };
    let tries = 0;
    const mount = () => {
      if (!el.isConnected) return;
      if (!J.checkupScan) {
        if (++tries < 20) setTimeout(mount, 150);
        else { phaseEl.textContent = 'RENDU 3D INDISPONIBLE'; revealAll(); }
        return;
      }
      try {
        scan = J.checkupScan.mount(canvas, { heartStyle: ev.heartStyle, vitals: ev.vitals, onPhase, onFrame: drawLinks });
      } catch (err) {
        console.error('Checkup : rendu 3D impossible', err);
        phaseEl.textContent = 'RENDU 3D INDISPONIBLE';
        revealAll();
      }
    };
    requestAnimationFrame(mount);
  }

  K.registerTool('checkup', renderCheckup);
})();
