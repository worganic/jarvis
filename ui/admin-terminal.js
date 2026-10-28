// Section « Terminal » de l'administration : les actions affichées en boutons dans la card d'un
// terminal (cmd, PowerShell, tmux, Claude Code) et reconnues à la voix quand il est au premier plan.
// Catalogue : data/terminal-actions.json (orchestrator/terminal-actions.js).
(function () {
  const A = window.JarvisAdmin;
  if (!A) return;
  const { h, panel, field, api, sections, render, toast, toggle, confirmDialog } = A;

  const state = { draft: null, loading: false, dirty: false, busy: false, error: '' };

  async function load() {
    if (state.loading) return;
    state.loading = true;
    const res = await api('terminalActionsState');
    state.loading = false;
    if (res && res.ok) {
      state.draft = res.actions.map((a) => ({ ...a, phrases: [...a.phrases] }));
      state.dirty = false;
    }
    render(true);
  }

  function touch() {
    if (state.dirty) return;
    state.dirty = true;
    render(true);
  }

  async function save() {
    state.busy = true;
    state.error = '';
    render(true);
    const res = await api('terminalActionsSave', state.draft);
    state.busy = false;
    if (res && res.ok) {
      state.draft = res.actions.map((a) => ({ ...a, phrases: [...a.phrases] }));
      state.dirty = false;
      toast('✓ Actions enregistrées');
    } else {
      state.error = (res && res.error) || 'Enregistrement impossible.';
    }
    render(true);
  }

  function move(i, d) {
    const j = i + d;
    if (j < 0 || j >= state.draft.length) return;
    [state.draft[i], state.draft[j]] = [state.draft[j], state.draft[i]];
    state.dirty = true;
    render(true);
  }

  function newAction() {
    state.draft.push({ id: '', label: 'Nouvelle action', icon: '⚡', phrases: [], script: '/help\n{Entrée}', confirm: false, clear: false, card: true, enabled: true });
    state.dirty = true;
    render(true);
  }

  // Saisie : le brouillon est modifié sur place, sans redessiner (le champ garde le focus).
  function edit(a, prop, control, transform = (v) => v) {
    control.addEventListener('input', (e) => { a[prop] = transform(e.target.value); if (!state.dirty) { state.dirty = true; markDirty(); } });
    return control;
  }
  function markDirty() {
    const b = document.getElementById('terminal-save');
    if (b) b.disabled = false;
    const t = document.getElementById('terminal-dirty');
    if (t) t.textContent = 'modifications non enregistrées';
  }

  function actionPanel(a, i) {
    const phrases = h('textarea', { rows: 3, spellcheck: 'false', placeholder: 'une phrase par ligne, dite telle quelle' }, a.phrases.join('\n'));
    const script = h('textarea', { rows: 3, spellcheck: 'false', style: 'font-family: var(--mono, monospace)' }, a.script);
    return h('div', { class: 'panel' },
      h('h2', {}, `${a.icon || '⚡'} ${a.label || 'Sans nom'}`,
        h('span', { class: 'tag' }, a.enabled ? (a.confirm ? 'avec confirmation' : 'active') : 'désactivée')),
      h('div', { class: 'grid' },
        field('Nom (affiché sur le bouton)', edit(a, 'label', h('input', { type: 'text', value: a.label, maxlength: '40' }))),
        field('Icône', edit(a, 'icon', h('input', { type: 'text', value: a.icon, maxlength: '4', style: 'width:5em' }))),
        field('Phrases à dire', edit(a, 'phrases', phrases, (v) => v.split('\n')), 'Phrase exacte (ponctuation et accents ignorés) : une phrase ordinaire ne déclenche jamais l’action par hasard.', true),
        field('Script', edit(a, 'script', script), 'Une étape par ligne : du texte (tapé tel quel), une touche entre accolades — {Entrée} {Échap} {Ctrl+C} {Ctrl+D} {Ctrl+U} {Tab} {Maj+Tab} {Haut} {Bas} {Gauche} {Droite} —, {attendre 500}, ou {Mode plan} / {Mode bypass} / {Mode normal} / {Mode édition} (Maj+Tab jusqu’au mode voulu de Claude Code, lu à l’écran).', true)),
      h('div', { class: 'grid', style: 'margin-top:8px' },
        toggle('Active', a.enabled, (v) => { a.enabled = v; touch(); }),
        toggle('Bouton dans la card', a.card, (v) => { a.card = v; touch(); }),
        toggle('Demander confirmation avant de lancer', a.confirm, (v) => { a.confirm = v; touch(); }),
        toggle('Effacer d’abord le texte dicté en attente', a.clear, (v) => { a.clear = v; touch(); })),
      h('div', { class: 'row', style: 'margin-top:10px' },
        h('button', { class: 'btn ghost', disabled: i === 0, onclick: () => move(i, -1) }, '↑'),
        h('button', { class: 'btn ghost', disabled: i === state.draft.length - 1, onclick: () => move(i, 1) }, '↓'),
        h('button', { class: 'btn ghost', onclick: async () => {
          if (!(await confirmDialog('Supprimer l’action', `« ${a.label} » sera retirée du catalogue (après enregistrement).`, 'Supprimer', true))) return;
          state.draft.splice(i, 1);
          state.dirty = true;
          render(true);
        } }, 'Supprimer')));
  }

  sections.terminal = () => {
    if (!state.draft && !state.loading) load();
    const d = state.draft;
    return [
      h('h1', {}, 'Terminal'),
      h('p', { class: 'lead' }, 'Quand un terminal (cmd, PowerShell, tmux, Claude Code) est ouvert, sa card affiche ces actions en boutons, et Jarvis les reconnaît à la voix tant que le terminal est au premier plan. Chaque frappe passe par la sécurité habituelle et est journalisée. La card d’un terminal ne se ferme pas tant que la fenêtre est ouverte.'),
      panel('Catalogue', d ? `${d.length} action${d.length > 1 ? 's' : ''}` : 'chargement…',
        h('div', { class: 'row' },
          h('button', { class: 'btn primary', id: 'terminal-save', disabled: !state.dirty || state.busy, onclick: save }, state.busy ? '… enregistrement' : 'Enregistrer'),
          h('button', { class: 'btn ghost', disabled: !d || state.busy, onclick: newAction }, '＋ Nouvelle action'),
          h('button', { class: 'btn ghost', disabled: state.busy, onclick: async () => {
            if (!(await confirmDialog('Rétablir les actions d’origine', 'Le catalogue actuel (y compris tes actions) sera remplacé par celui d’origine.', 'Rétablir', true))) return;
            const res = await api('terminalActionsReset');
            if (res && res.ok) { state.draft = res.actions.map((a) => ({ ...a, phrases: [...a.phrases] })); state.dirty = false; toast('✓ Actions d’origine rétablies'); render(true); }
          } }, 'Rétablir les actions d’origine'),
          h('span', { id: 'terminal-dirty', class: 'help', style: 'margin-left:10px' }, state.dirty ? 'modifications non enregistrées' : '')),
        state.error ? h('div', { class: 'help', style: 'color: var(--err); margin-top:10px' }, state.error) : null),
      ...(d ? d.map(actionPanel) : [])
    ];
  };
})();
