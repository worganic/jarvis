// Module Terminal : la card des terminaux et agents (Claude Code, tmux…) ouverts par Jarvis. C'est une card de liste du
// système (une ligne par agent, renderRows) ; le module y ajoute ses boutons d'en-tête — « 🎙 » (dicter dans le terminal
// qui est devant / rendre la main) et « ☰ » (afficher ou cacher la liste des actions) — et la garde affichée en fantôme
// quand elle est fermée. Noyau : orch/terminal_*.rs, orch/tmux*.rs.
(function () {
  const J = (window.JARVIS = window.JARVIS || {});
  const bridge = window.jarvisBridge;
  const K = J.widgetKit;
  if (!K) return;
  const { h } = K;

  let actionsHidden = false;
  try { actionsHidden = localStorage.getItem('jarvis.terminalActionsHidden') === '1'; } catch {}
  function dictateHeadButton(ev) {
    if (!ev.dictate || !ev.appId || !bridge || !bridge.cardAppAction) return null;
    const on = Boolean(ev.dictate.active);
    return h('button', {
      class: `w-head-btn w-dictate ${on ? 'on' : 'off'}`,
      title: on ? 'Dictée active : quand un terminal est devant, tout ce que tu dis y est écrit · clic : couper' : 'Dictée coupée · clic : écrire dans le terminal ce que tu dis (quand un terminal est devant)',
      onclick: (e) => { e.stopPropagation(); bridge.cardAppAction(ev.appId, 'dictee'); }
    }, '🎙');
  }
  function actionsHeadButton(el, ev) {
    if (!ev.rows.some((r) => r.actionList)) return null;
    const btn = h('button', { class: 'w-head-btn', title: 'Afficher / cacher la liste des actions' }, '☰');
    const apply = () => { el.classList.toggle('w-actions-hidden', actionsHidden); btn.classList.toggle('on', !actionsHidden); };
    btn.onclick = (e) => {
      e.stopPropagation();
      actionsHidden = !actionsHidden;
      try { localStorage.setItem('jarvis.terminalActionsHidden', actionsHidden ? '1' : '0'); } catch {}
      apply();
    };
    apply();
    return btn;
  }

  K.headButtons((ev, el) => (ev.tool === 'terminal' ? [dictateHeadButton(ev), actionsHeadButton(el, ev)] : null));
  K.noAppClick('terminal');
  K.ghostKey('tool:Terminal');
  K.addGhost({ key: 'tool:Terminal', kind: 'tool', title: 'Terminal', source: '' });
})();
