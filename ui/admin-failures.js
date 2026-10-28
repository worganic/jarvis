// Section « Échecs » de l'administration : les demandes qui n'ont pas abouti (tâche IA en échec,
// action locale en erreur ou « je n'ai pas pu »). Un échec peut être confié à Claude Code, qui
// cherche comment le faire réussir la prochaine fois — Jarvis apprend ainsi, fonction par fonction.
(function () {
  const A = window.JarvisAdmin;
  if (!A) return;
  const { h, panel, api, sections, render, toast, confirmDialog, fmtDate } = A;

  const state = { data: null, loading: false, filter: 'a_traiter' };
  const KIND = { ia: ['IA', 'warn'], local: ['sans IA', 'off'], action: ['action Windows', 'off'] };
  const STATUS = { nouveau: ['nouveau', 'err'], en_cours: ['confié à Claude', 'warn'], resolu: ['résolu', 'ok'], ignore: ['ignoré', 'off'] };
  const FILTERS = { a_traiter: 'À traiter', en_cours: 'Confiés à Claude', resolu: 'Résolus', ignore: 'Ignorés', tous: 'Tous' };

  async function load() {
    if (state.loading) return;
    state.loading = true;
    const res = await api('failuresList');
    state.loading = false;
    if (res && res.ok) state.data = res;
    render(true);
  }

  const previous = A.onJournal;
  A.onJournal = (ev) => {
    if (previous) previous(ev);
    if (ev && ev.type === 'failures' && document.getElementById('section-failures').classList.contains('active')) load();
  };

  async function act(method, ...args) {
    const res = await api(method, ...args);
    if (res && res.ok) {
      state.data = res;
      render(true);
    }
    return res;
  }

  async function learn(item) {
    const ok = await confirmDialog('APPRENDRE À JARVIS',
      `Claude Code va étudier l’échec « ${item.demande || item.erreur} » et modifier le code de Jarvis pour qu’il réussisse la prochaine fois (sans IA si possible). Il s’ouvre dans une fenêtre psmux où tu suis son travail ; Jarvis proposera ensuite de se relancer.`,
      'Confier à Claude Code');
    if (!ok) return;
    const res = await act('failureLearn', item.id);
    if (res && res.ok) toast(`✓ confié à Claude Code · ${res.where}`);
  }

  function visible(items) {
    if (state.filter === 'tous') return items;
    if (state.filter === 'a_traiter') return items.filter((i) => i.statut === 'nouveau');
    return items.filter((i) => i.statut === state.filter);
  }

  function row(item) {
    const [kindLabel, kindClass] = KIND[item.kind] || [item.kind, 'off'];
    const [stLabel, stClass] = STATUS[item.statut] || [item.statut, 'off'];
    const learning = item.apprentissage;
    return h('tr', {},
      h('td', { class: 'help', style: 'white-space:nowrap' }, fmtDate(item.lastAt || item.at),
        item.occurrences > 1 ? h('div', {}, h('span', { class: 'badge warn' }, `× ${item.occurrences}`)) : null),
      h('td', {},
        h('div', {}, item.demande ? `« ${item.demande} »` : '(demande inconnue)'),
        h('div', { class: 'help' }, item.erreur || ''),
        learning ? h('div', { class: 'help' }, `Claude Code : ${learning.ou || ''}${learning.fini ? ` · terminé (${learning.code === 0 ? 'ok' : `code ${learning.code}`})` : ''}`) : null,
        learning && learning.compteRendu ? h('details', {}, h('summary', {}, 'Compte rendu'), h('pre', { class: 'help', style: 'white-space:pre-wrap' }, learning.compteRendu)) : null),
      h('td', {}, h('span', { class: `badge ${kindClass}` }, kindLabel), item.moteur ? h('div', { class: 'help' }, item.moteur) : null,
        item.intention ? h('div', { class: 'help' }, item.intention) : null),
      h('td', {}, h('span', { class: `badge ${stClass}` }, stLabel)),
      h('td', { class: 'actions', style: 'white-space:nowrap' },
        item.statut !== 'resolu' ? h('button', { class: 'btn primary', title: 'Claude Code cherche comment faire réussir cette demande', onclick: () => learn(item) }, '🧠 Apprendre') : null,
        item.statut !== 'resolu' ? h('button', { class: 'btn ghost', onclick: () => act('failureSetStatus', item.id, 'resolu') }, 'Résolu') : null,
        item.statut === 'nouveau' ? h('button', { class: 'btn ghost', onclick: () => act('failureSetStatus', item.id, 'ignore') }, 'Ignorer') : null,
        h('button', { class: 'btn danger', title: 'Supprimer', onclick: () => act('failureDelete', item.id) }, '✕')));
  }

  sections.failures = () => {
    if (!state.data && !state.loading) load();
    const d = state.data;
    if (!d) return [h('h1', {}, 'Échecs & apprentissage'), h('div', { class: 'empty' }, 'Chargement…')];
    const items = visible(d.items);
    const c = d.counts;
    return [
      h('h1', {}, 'Échecs & apprentissage'),
      h('p', { class: 'lead' }, 'Toutes les demandes qui n’ont pas abouti : tâche confiée à une IA qui a échoué, action locale en erreur, ou réponse « je n’ai pas pu ». « Apprendre » confie l’échec à Claude Code, qui modifie le code de Jarvis pour que la demande réussisse la prochaine fois — de préférence sans IA. Les refus de sécurité ne sont pas des échecs.'),
      panel('Filtre', `${c.nouveau} à traiter · ${c.en_cours} confiés · ${c.resolu} résolus · ${c.total} au total`,
        h('div', { class: 'row' }, ...Object.entries(FILTERS).map(([k, label]) =>
          h('button', { class: `btn ${state.filter === k ? 'primary' : 'ghost'}`, onclick: () => { state.filter = k; render(true); } }, label)),
          h('button', { class: 'btn ghost', onclick: load }, '⟳ Actualiser'))),
      panel(FILTERS[state.filter], `${items.length}`,
        items.length
          ? h('table', {},
            h('thead', {}, h('tr', {}, h('th', {}, 'Date'), h('th', {}, 'Demande et erreur'), h('th', {}, 'Type'), h('th', {}, 'Statut'), h('th', {}, ''))),
            h('tbody', {}, ...items.map(row)))
          : h('div', { class: 'empty' }, state.filter === 'a_traiter' ? 'Aucun échec à traiter. 👍' : 'Rien ici.'))
    ];
  };
})();
