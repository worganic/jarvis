// Section « Sécurité » de l'administration (chantier v3, étape 5).
// Chaque type d'action soumis à accord, avec son niveau. Resserrer est immédiat ; abaisser
// (vers l'accord direct notamment) exige le code à 4 chiffres affiché puis retapé.
(function () {
  const A = window.JarvisAdmin;
  if (!A) return;
  const { h, panel, api, sections, render, challengeFlow, toast } = A;

  const state = { data: null, loading: false };
  const LEVEL_ORDER = ['allow', 'voice', 'code', 'deny'];
  const RISK_CLASS = { faible: 'ok', moyen: 'warn', 'élevé': 'err', inconnu: 'warn' };

  async function load() {
    if (state.loading) return;
    state.loading = true;
    const res = await api('securityList');
    state.loading = false;
    if (res && res.ok) state.data = res;
    render(true);
  }

  // Événement « security » (validation ouverte ou close, réglage modifié) : relecture.
  const previous = A.onJournal;
  A.onJournal = (ev) => {
    if (previous) previous(ev);
    if (document.getElementById('section-security').classList.contains('active')) load();
  };

  async function change(key, level) {
    const res = await api('securitySet', key, level);
    if (res && res.challenge) {
      const done = await challengeFlow(res.challenge, 'securitySetConfirm');
      if (done && done.ok && done.list) state.data = done.list;
    } else if (res && res.ok) {
      if (res.list) state.data = res.list;
      if (!res.unchanged) toast('✓ sécurité resserrée');
    }
    load();
  }

  function levelSelect(entry, levels) {
    const el = h('select', {
      class: `level-select level-${entry.level}`,
      onchange: (e) => change(entry.key, e.target.value)
    }, ...LEVEL_ORDER.map((l) => h('option', { value: l, selected: l === entry.level }, levels[l])));
    el.value = entry.level;
    return el;
  }

  sections.security = () => {
    if (!state.data && !state.loading) load();
    const d = state.data;
    if (!d) return [h('h1', {}, 'Sécurité'), h('div', { class: 'empty' }, 'Chargement…')];

    const groups = new Map();
    for (const e of d.entries) {
      if (!groups.has(e.group)) groups.set(e.group, []);
      groups.get(e.group).push(e);
    }
    const custom = d.entries.filter((e) => e.custom).length;
    const direct = d.entries.filter((e) => e.level === 'allow' && e.default !== 'allow');

    return [
      h('h1', {}, 'Sécurité'),
      h('p', { class: 'lead' }, 'Toutes les actions que Jarvis ne fait pas sans ton accord, et le niveau exigé pour chacune. Passer une action en « accord direct » exige de recopier un code ; resserrer est immédiat. Les fichiers internes de Jarvis (mémoire, journal, réglages) restent inaccessibles quoi qu’il arrive, et un refus de zone reste un refus.'),
      direct.length ? h('div', { class: 'panel warn-panel' },
        h('h2', {}, '⚠ Accords directs ajoutés'),
        h('div', {}, `${direct.length} action(s) normalement protégée(s) s’exécutent sans ton accord : `, direct.map((e) => e.label).join(', '), '.')) : null,
      d.pending.length ? panel('Validations en attente', `${d.pending.length}`,
        h('p', { class: 'hint' }, 'Le code à saisir s’affiche sur la carte de l’action dans le HUD. Tu peux refuser d’ici.'),
        h('table', {}, h('tbody', {}, ...d.pending.map((p) => h('tr', {},
          h('td', {}, h('span', { class: `badge ${p.type === 'voice' ? 'warn' : 'err'}` }, p.type === 'voice' ? 'accord oral' : 'code')),
          h('td', { style: 'overflow-wrap:anywhere' }, p.fullDetail || p.detail),
          h('td', { class: 'actions' }, h('button', { class: 'btn danger', onclick: async () => { await api('pendingDeny', p.taskId); load(); } }, 'Refuser'))))))) : null,
      ...[...groups.entries()].map(([group, entries]) => panel(group, `${entries.length} type(s)`,
        h('table', { class: 'security-table' },
          h('thead', {}, h('tr', {}, h('th', {}, 'Action'), h('th', {}, 'Risque'), h('th', {}, 'Niveau exigé'), h('th', {}, 'Par défaut'))),
          h('tbody', {}, ...entries.map((e) => h('tr', {},
            h('td', {}, h('div', {}, e.label, e.custom ? h('span', { class: 'pin' }, '◆ modifié') : null), e.description ? h('div', { class: 'help' }, e.description) : null),
            h('td', {}, e.risk ? h('span', { class: `badge ${RISK_CLASS[e.risk] || 'off'}` }, e.risk) : null),
            h('td', { style: 'width:230px' }, levelSelect(e, d.levels)),
            h('td', { class: 'help', style: 'white-space:nowrap' }, d.levels[e.default]))))))),
      panel('Historique récent', `${custom} réglage(s) personnalisé(s)`,
        d.recent.length ? h('table', {}, h('tbody', {}, ...d.recent.map((r) => h('tr', {},
          h('td', { class: 'help', style: 'white-space:nowrap' }, new Date(r.at).toLocaleString('fr-FR', { day: '2-digit', month: '2-digit', hour: '2-digit', minute: '2-digit' })),
          h('td', {}, h('span', { class: `badge ${/refus|expir/.test(r.type) ? 'err' : /accord|modifi/.test(r.type) ? 'ok' : 'warn'}` }, r.label)),
          h('td', {}, r.summary)))))
          : h('div', { class: 'empty' }, 'Aucune validation pour l’instant.'))
    ];
  };
})();
