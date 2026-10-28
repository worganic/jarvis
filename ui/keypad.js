// Clavier numérique du code de sécurité (actions critiques), au centre du HUD.
//
// Il apparaît dès que Jarvis demande un code (événement « update », statut « awaiting », type
// « code ») et affiche le code à saisir. Les chiffres viennent d'un clic sur les touches, du clavier
// (chiffres, pavé numérique, Retour arrière) ou de la voix : tous passent par le noyau
// (orchestrator/code-entry.js), qui vérifie chaque chiffre et renvoie « code_entry ». La réponse
// graphique est donc la même quelle que soit la source : case verte si le chiffre est bon, touche et
// case rouges sinon (la case reste vide). Au 4e bon chiffre, l'action est validée et le clavier
// disparaît.
(function () {
  const bridge = window.jarvisBridge;
  if (!bridge) return;

  const el = (tag, cls, text) => {
    const n = document.createElement(tag);
    if (cls) n.className = cls;
    if (text !== undefined) n.textContent = text;
    return n;
  };

  const root = el('div', 'kp-overlay');
  root.setAttribute('role', 'dialog');
  root.setAttribute('aria-label', 'Code de sécurité');
  const box = el('div', 'kp-box');
  const head = el('div', 'kp-head', 'CODE DE SÉCURITÉ');
  const detail = el('div', 'kp-detail');
  const codeLine = el('div', 'kp-code');
  const slotsEl = el('div', 'kp-slots');
  const slots = [0, 1, 2, 3].map(() => slotsEl.appendChild(el('div', 'kp-slot')));
  const grid = el('div', 'kp-grid');
  const keys = {};
  for (const k of ['1', '2', '3', '4', '5', '6', '7', '8', '9', 'back', '0', 'cancel']) {
    const b = el('button', `kp-key${k === 'back' || k === 'cancel' ? ' kp-fn' : ''}${k === 'cancel' ? ' kp-cancel' : ''}`, k === 'back' ? '⌫' : k === 'cancel' ? '✕' : k);
    b.type = 'button';
    b.title = k === 'back' ? 'Effacer le dernier chiffre' : k === 'cancel' ? 'Refuser l’action' : `Chiffre ${k}`;
    b.addEventListener('click', () => press(k));
    grid.appendChild(b);
    keys[k] = b;
  }
  const hint = el('div', 'kp-hint', 'Clique, tape ou dis les 4 chiffres pour autoriser · ✕ pour refuser');
  box.append(head, detail, codeLine, slotsEl, grid, hint);
  root.appendChild(box);
  document.body.appendChild(root);

  let current = null; // { id, code, filled }
  let closeTimer = null;

  function paint(filled) {
    slots.forEach((s, i) => {
      s.textContent = i < filled ? current.code[i] : '';
      s.classList.toggle('ok', i < filled);
      s.classList.toggle('next', i === filled);
    });
    // Code à saisir : chiffres déjà bons atténués, le suivant souligné.
    codeLine.replaceChildren(...current.code.split('').map((d, i) => el('span', i < filled ? 'done' : i === filled ? 'next' : '', d)));
  }

  function renderInfo(text, info) {
    detail.replaceChildren();
    if (!info || typeof info !== 'object') {
      detail.textContent = text || '';
      return;
    }
    const addRow = (label, value, cls) => {
      if (!value) return;
      const row = el('div', `kp-info-row${cls ? ` ${cls}` : ''}`);
      row.append(el('span', 'kp-info-lbl', `${label} : `), el('span', 'kp-info-val', value));
      detail.appendChild(row);
    };
    const taskWithEngine = info.task ? (info.engine ? `${info.task} · ${info.engine}` : info.task) : '';
    addRow('Tâche', taskWithEngine);
    addRow('Action', info.action || text);
    addRow('Cible', info.target || text, 'kp-info-target');
    if (info.zone) addRow('Zone', info.zone);
    if (info.reason) {
      const r = info.risk ? `${info.reason} (risque ${info.risk})` : info.reason;
      addRow('Motif', r, 'kp-info-reason');
    }
    if (!detail.childElementCount) {
      detail.textContent = text || '';
    }
  }

  function open(id, code, text, info) {
    clearTimeout(closeTimer);
    if (current && current.id === id) return;
    current = { id, code: String(code), filled: 0 };
    renderInfo(text, info);
    root.classList.remove('kp-done', 'kp-leaving');
    root.classList.add('kp-open');
    paint(0);
  }

  function close(success) {
    if (!current || root.classList.contains('kp-done')) return; // déjà en train de se fermer
    const leaving = () => {
      root.classList.add('kp-leaving');
      closeTimer = setTimeout(() => { root.classList.remove('kp-open', 'kp-leaving', 'kp-done'); }, 260);
      current = null;
    };
    if (success) {
      root.classList.add('kp-done');
      closeTimer = setTimeout(leaving, 650);
    } else leaving();
  }

  function flash(node) {
    if (!node) return;
    node.classList.remove('bad');
    void node.offsetWidth; // relance l'animation
    node.classList.add('bad');
    setTimeout(() => node.classList.remove('bad'), 550);
  }

  function press(k) {
    if (!current) return;
    const id = current.id;
    if (k === 'cancel') { bridge.deny(id); close(false); return; }
    if (k === 'back') { bridge.codeBack(id).catch(() => {}); return; }
    keys[k].classList.add('pressed');
    setTimeout(() => keys[k].classList.remove('pressed'), 120);
    bridge.codeDigits(id, k).catch(() => {});
  }

  // Touches du clavier : prises par le clavier numérique tant qu'il est affiché (même si le champ de
  // commande a le focus), pour que la saisie réponde exactement comme un clic.
  window.addEventListener('keydown', (e) => {
    if (!current || e.ctrlKey || e.altKey || e.metaKey) return;
    const k = /^[0-9]$/.test(e.key) ? e.key : e.key === 'Backspace' ? 'back' : null;
    if (!k) return;
    e.preventDefault();
    e.stopPropagation();
    press(k);
  }, true);

  bridge.onTaskEvent((ev) => {
    if (!ev) return;
    if (ev.type === 'update' && ev.status === 'awaiting' && ev.code && ev.validationType !== 'voice') {
      // Raison complète de la demande (commande entière, chemin complet, tâche et motif), pas la version raccourcie.
      open(ev.id, ev.code, ev.fullDetail || ev.detail, ev.info);
    } else if (ev.type === 'code_entry' && current && ev.id === current.id) {
      current.filled = ev.filled;
      paint(ev.filled);
      if (ev.ok === false) {
        flash(keys[ev.digit]);
        flash(slots[ev.index]);
      }
      if (ev.filled === current.code.length) close(true);
    } else if (ev.type === 'update' && current && ev.id === current.id && ev.status && ev.status !== 'awaiting') {
      // Une simple mise à jour du détail (outil suivant de l'IA, sans statut) ne referme pas le clavier.
      close(current.filled === current.code.length);
    } else if ((ev.type === 'finish' || (ev.type === 'update' && ev.cancelled)) && current && ev.id === current.id) {
      close(false); // tâche terminée ou annulée pendant l'attente du code
    } else if (ev.type === 'stopped' && current) {
      close(false);
    }
  });
})();
