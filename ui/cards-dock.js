// Deux boutons du HUD, en haut à droite sous « AIDE » :
//   « RANGER »  : réduit les cards de type 1 en boutons jaunes et les aligne en colonne, collées à gauche, puis range dessous
//                 les cards de type 2 ouvertes (boutons violets) ;
//   « PLACER »  : les remet à leur emplacement par défaut, puis les ouvre ;
//   « ACTIONS » : ouvre / réduit la card Actions (module Actions, modules/actions/), qui liste les cards qu'on peut ouvrir.
// Les cards se réduisent chacune par leur propre module (widgets.js, modules/audio/ui/audio.js, hw-panels.js) : ici on ne fait
// que les appeler, puis les poser (positions mémorisées comme après un glisser-déposer).
(function () {
  const J = (window.JARVIS = window.JARVIS || {});
  const bridge = window.jarvisBridge;
  const root = document.getElementById('widgets');
  if (!root) return;

  const MARGIN = 12; // même marge que widgets.js › keepInView
  const TOP = 120; // ancrage de la card Bureau (widgets.js › renderRows)
  const GAP = 6;
  const ui = () => parseFloat(document.documentElement.style.getPropertyValue('--ui')) || 1;

  function el(tag, cls, text) {
    const e = document.createElement(tag);
    if (cls) e.className = cls;
    if (text !== undefined) e.textContent = text;
    return e;
  }

  // --- Ranger ----------------------------------------------------------------------------------------------

  function ranger() {
    const w = J.widgets;
    const folded = w && w.collapseAll ? w.collapseAll() : [];
    const type2 = w && w.collapseOpenType2 ? w.collapseOpenType2() : []; // cards temporaires ouvertes : rangées sous celles de type 1
    if (J.audioPanel) J.audioPanel.setCollapsed(true);
    if (J.hwPanels) J.hwPanels.setAllCollapsed(true);
    if (J.conversationCard) J.conversationCard.setCollapsed(true);
    if (J.actionsCard) J.actionsCard.setCollapsed(true);
    // Les pastilles prennent leur taille après le changement de classe : on mesure juste après.
    setTimeout(() => stack(folded, type2), 150);
  }

  // Colonne à gauche : Bureau d'abord (sa place habituelle), puis Audio, Matériel, Conversation et Actions, puis les cards de type 2 ouvertes.
  function stack(folded, type2 = []) {
    const S = ui();
    const x = MARGIN;
    let y = TOP * S;
    const items = [];
    for (const f of folded) items.push({ h: f.el.offsetHeight, move: (px, py) => J.widgets.moveCard(f.key, px, py), bureau: f.el.classList.contains('w-compact') });
    items.sort((a, b) => Number(b.bureau) - Number(a.bureau));
    if (J.audioPanel) items.splice(Math.min(items.length, items.filter((i) => i.bureau).length), 0, { h: J.audioPanel.element.offsetHeight, move: J.audioPanel.moveTo });
    if (J.hwPanels) items.push({ h: J.hwPanels.element.offsetHeight, move: J.hwPanels.moveTo });
    if (J.conversationCard) items.push({ h: 26 * S, move: J.conversationCard.moveTo });
    if (J.actionsCard) items.push({ h: 26 * S, move: J.actionsCard.moveTo });
    for (const f of type2) items.push({ h: f.el.offsetHeight, move: (px, py) => J.widgets.moveCard(f.key, px, py) });
    for (const it of items) {
      it.move(x, Math.round(y));
      y += (it.h || 26 * S) + GAP * S;
    }
  }

  // 1er clic : replace toutes les cards qui ont un emplacement par défaut. 2e clic (elles y sont déjà) : les ouvre toutes.
  function replace() {
    const mods = [J.widgets && J.widgets.defaultsState ? { s: J.widgets.defaultsState(), restore: J.widgets.restoreDefaults, open: J.widgets.openDefaults } : null]
      .concat([J.audioPanel, J.hwPanels, J.conversationCard, J.actionsCard].map((m) => (m && m.defaultState ? { s: m.defaultState(), restore: m.restoreDefault, open: m.openDefault } : null)))
      .filter(Boolean);
    const total = mods.reduce((n, m) => n + m.s.total, 0);
    const there = mods.reduce((n, m) => n + m.s.there, 0);
    let label;
    if (!total) label = '↺ AUCUN 📍';
    else if (there === total) { // déjà à leur place : on les ouvre
      const n = mods.reduce((k, m) => k + m.open(), 0);
      label = n ? `↺ ${n} OUVERTE${n > 1 ? 'S' : ''}` : '↺ DÉJÀ PLACÉES';
    } else {
      mods.forEach((m) => m.restore());
      label = `↺ ${total} PLACÉE${total > 1 ? 'S' : ''}`;
    }
    placeBtn.textContent = label;
    setTimeout(() => { placeBtn.textContent = '↺ PLACER'; }, 1800);
  }

  // « FIGER » : enregistre la vue (emplacement et état ouverte / réduite de toutes les cards) ; « RESTAURER » : la rétablit.
  const LAYOUT_KEY = () => (window.JARVIS_POS_PREFIX || 'jarvis.') + 'layout';
  const flash = (btn, text, label) => { btn.textContent = text; setTimeout(() => { btn.textContent = label; }, 1800); };
  function freezeLayout() {
    const snap = {
      widgets: J.widgets && J.widgets.layoutSnapshot ? J.widgets.layoutSnapshot() : {},
      audio: J.audioPanel ? J.audioPanel.layoutSnapshot() : null,
      hw: J.hwPanels ? J.hwPanels.layoutSnapshot() : null,
      conversation: J.conversationCard ? J.conversationCard.layoutSnapshot() : null,
      actions: J.actionsCard ? J.actionsCard.layoutSnapshot() : null
    };
    try { localStorage.setItem(LAYOUT_KEY(), JSON.stringify(snap)); } catch {}
    const n = Object.keys(snap.widgets).length + ['audio', 'hw', 'conversation', 'actions'].filter((k) => snap[k]).length;
    flash(freezeBtn, `❄ ${n} CARDS FIGÉES`, '❄ FIGER');
  }
  function restoreLayout() {
    let snap = null;
    try { snap = JSON.parse(localStorage.getItem(LAYOUT_KEY()) || 'null'); } catch {}
    if (!snap) { flash(restoreBtn, '⟲ RIEN DE FIGÉ', '⟲ RESTAURER'); return; }
    let n = 0;
    if (J.widgets && J.widgets.layoutApply) n += J.widgets.layoutApply(snap.widgets);
    if (J.audioPanel) n += J.audioPanel.layoutApply(snap.audio);
    if (J.hwPanels) n += J.hwPanels.layoutApply(snap.hw);
    if (J.conversationCard) n += J.conversationCard.layoutApply(snap.conversation);
    if (J.actionsCard) n += J.actionsCard.layoutApply(snap.actions);
    flash(restoreBtn, `⟲ ${n} RESTAURÉE${n > 1 ? 'S' : ''}`, '⟲ RESTAURER');
  }

  // --- Boutons, sous « AIDE » -------------------------------------------------------------------------------

  const bar = el('div');
  bar.id = 'dock-btns';
  const rangerBtn = el('button', 'dock-btn', '⇤ RANGER');
  rangerBtn.title = 'Réduire toutes les cards en boutons, rangés en colonne à gauche';
  rangerBtn.onclick = ranger;
  const actionsBtn = el('button', 'dock-btn', '☰ ACTIONS');
  actionsBtn.title = 'Liste des cards qu’on peut ouvrir';
  actionsBtn.onclick = () => { if (J.actionsCard) J.actionsCard.setCollapsed(!J.actionsCard.isCollapsed()); };
  // Card Actions (module Actions) ouverte ou réduite : le bouton s'allume.
  J.dockActionsOn = (on) => actionsBtn.classList.toggle('on', Boolean(on));
  const placeBtn = el('button', 'dock-btn', '↺ PLACER');
  placeBtn.title = 'Remettre à leur emplacement par défaut (bouton 📍 de chaque card) toutes les cards qui en ont un';
  placeBtn.onclick = replace;
  const freezeBtn = el('button', 'dock-btn', '❄ FIGER');
  freezeBtn.title = 'Figer la vue : enregistre l’emplacement et l’état (ouverte, réduite) de toutes les cards';
  freezeBtn.onclick = freezeLayout;
  const restoreBtn = el('button', 'dock-btn', '⟲ RESTAURER');
  restoreBtn.title = 'Replacer toutes les cards comme au dernier « FIGER » (emplacement et état)';
  restoreBtn.onclick = restoreLayout;
  // Bouton qui ouvre ou ferme la liste des boutons ci-dessus (état mémorisé).
  const toggleBtn = el('button', 'dock-btn dock-toggle', '▾ OUTILS');
  toggleBtn.title = 'Afficher ou masquer les boutons RANGER, PLACER, ACTIONS, FIGER, RESTAURER';
  let open = true;
  try { open = localStorage.getItem('jarvis.dockOpen') !== '0'; } catch {}
  const paintDock = () => {
    bar.classList.toggle('closed', !open);
    toggleBtn.textContent = open ? '▾ OUTILS' : '▸ OUTILS';
    toggleBtn.classList.toggle('on', open);
  };
  toggleBtn.onclick = () => {
    open = !open;
    try { localStorage.setItem('jarvis.dockOpen', open ? '1' : '0'); } catch {}
    paintDock();
  };
  bar.append(rangerBtn, placeBtn, actionsBtn, freezeBtn, restoreBtn, toggleBtn);
  paintDock();
  document.body.appendChild(bar);

  // « AIDE » est dessiné dans le canevas (app.js) : sa position suit l'échelle et la hauteur des boutons de fenêtre.
  function follow() {
    const S = ui();
    bar.style.top = `${Math.round((J.hudButtonsBottom || 72 * S) + 6 * S)}px`;
    bar.style.right = `${Math.round(34 * S)}px`;
  }
  follow();
  setInterval(follow, 500);
  window.addEventListener('resize', follow);

  J.cardsDock = { ranger, replace };
})();
