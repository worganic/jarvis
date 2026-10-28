// Section « Projets » de l'administration : le dossier où vivent les projets de l'utilisateur
// (chaque sous-dossier est un projet) et ce que Jarvis en sait. Jarvis s'en sert pour répondre
// (« où en est stats-ia ? ») et pour lancer Claude Code dans un projet.
(function () {
  const A = window.JarvisAdmin;
  if (!A) return;
  const { h, panel, field, api, sections, render, toast, textInput, fmtDate } = A;

  const state = { data: null, loading: false, busy: false, error: '', filter: '' };

  async function load(method = 'projectsState', ...args) {
    if (state.loading) return;
    state.loading = true;
    render(true);
    const res = await api(method, ...args);
    state.loading = false;
    if (res && res.ok) {
      state.data = res;
      state.error = '';
    } else {
      state.error = (res && res.error) || 'erreur';
    }
    render(true);
    return res;
  }

  function ago(iso) {
    if (!iso) return '—';
    const days = Math.floor((Date.now() - Date.parse(iso)) / 86400000);
    if (days <= 0) return "aujourd'hui";
    if (days === 1) return 'hier';
    if (days < 30) return `il y a ${days} j`;
    if (days < 365) return `il y a ${Math.round(days / 30)} mois`;
    return 'il y a plus d’un an';
  }

  async function claude(p) {
    const res = await api('projectClaude', p.path);
    if (res && res.ok) toast(res.where === 'tmux' ? `✓ Claude Code lancé dans ${p.name} (psmux)` : `✓ Claude Code ouvert dans ${p.name} (terminal)`);
    else toast(`✕ ${(res && res.error) || 'lancement impossible'}`, true);
  }

  function projectRow(p) {
    const g = p.git;
    return h('tr', {},
      h('td', {},
        h('div', { class: 'proj-name' }, p.parent ? h('span', { class: 'help' }, `${p.parent} › `) : null, h('strong', {}, p.name)),
        p.description ? h('div', { class: 'help proj-desc' }, p.description) : null,
        p.subprojects && p.subprojects.length ? h('div', { class: 'help' }, `contient : ${p.subprojects.join(', ')}`) : null),
      h('td', {}, ...(p.tech.length ? p.tech.map((t) => h('span', { class: 'chip passive' }, t)) : [h('span', { class: 'help' }, '—')])),
      h('td', { style: 'white-space:nowrap' },
        g ? h('div', {}, h('span', { class: `badge ${g.changes ? 'warn' : 'ok'}` }, g.changes ? `${g.changes} modif.` : 'à jour'), ` ${g.branch || ''}`) : h('span', { class: 'help' }, 'pas de git'),
        g && g.lastCommit ? h('div', { class: 'help', title: g.lastCommit }, g.lastCommit.length > 46 ? `${g.lastCommit.slice(0, 45)}…` : g.lastCommit) : null),
      h('td', { class: 'help', style: 'white-space:nowrap', title: p.lastActivity ? fmtDate(p.lastActivity) : '' }, ago(p.lastActivity)),
      h('td', { class: 'actions', style: 'white-space:nowrap' },
        h('button', { class: 'btn ghost', title: 'Ouvrir le dossier dans l’explorateur', onclick: () => api('projectOpen', p.path) }, '📂'),
        h('button', { class: 'btn primary', title: 'Lancer Claude Code dans ce projet (psmux, sinon une fenêtre de terminal)', onclick: () => claude(p) }, '▶ Claude Code')));
  }

  sections.projects = () => {
    if (!state.data && !state.loading && !state.error) load();
    const d = state.data;
    const root = textInput(d ? d.root : '', { placeholder: 'E:\\www\\A-En cours', autocomplete: 'off', spellcheck: 'false' });
    const save = async () => {
      state.busy = true;
      const res = await load('projectsSetRoot', root.value);
      state.busy = false;
      if (res && res.ok) toast(`✓ dossier des projets : ${res.root} (${res.projects.filter((p) => !p.parent).length} projets)`);
      else render(true);
    };
    root.addEventListener('keydown', (e) => { if (e.key === 'Enter') save(); });

    const all = d ? d.projects : [];
    const f = state.filter.trim().toLowerCase();
    const shown = f ? all.filter((p) => `${p.name} ${p.parent || ''} ${p.description} ${p.tech.join(' ')}`.toLowerCase().includes(f)) : all;
    const tops = all.filter((p) => !p.parent).length;

    return [
      h('h1', {}, 'Projets'),
      h('p', { class: 'lead' }, 'Chaque dossier du dossier des projets est un projet (et un sous-dossier qui a son propre package.json, .git ou CLAUDE.md en est un aussi). Jarvis s’en sert pour répondre à tes questions et pour lancer Claude Code dans un projet. Lecture seule : Jarvis n’écrit jamais dans tes projets de lui-même.'),
      panel('Dossier des projets', d ? (d.exists ? `${tops} projets` : 'introuvable') : '',
        h('div', { class: 'grid' },
          field('Chemin', root, 'Chemin absolu. Tous ses sous-dossiers deviennent des projets.')),
        h('div', { class: 'row', style: 'margin-top:10px' },
          h('button', { class: 'btn primary', disabled: state.busy, onclick: save }, state.busy ? '… vérification' : 'Enregistrer'),
          h('button', { class: 'btn ghost', disabled: state.loading, onclick: () => load('projectsRefresh') }, '⟳ Relire les projets')),
        state.error ? h('div', { class: 'help', style: 'color: var(--err); margin-top:10px' }, state.error) : null,
        d && d.exists && !d.zone ? h('div', { class: 'help', style: 'color: var(--warn); margin-top:10px' },
          'Ce dossier n’est dans aucune zone d’accès : Claude Code demandera un code pour y lire. Ajoute-le dans « Zones & vocabulaire » si tu veux lui laisser l’accès libre.') : null,
        d && d.zone ? h('div', { class: 'help', style: 'margin-top:10px' },
          `Zone d’accès : ${d.zone.name} — lecture ${d.zone.read === 'free' ? 'libre' : d.zone.read === 'code' ? 'avec code' : 'interdite'}, écriture ${d.zone.write === 'free' ? 'libre' : d.zone.write === 'code' ? 'avec code' : 'interdite'}.`) : null),
      panel('À dire à Jarvis', null,
        h('div', { class: 'chips' },
          ...['Quels sont mes projets ?', 'Sur quoi j’ai travaillé récemment ?', 'Où en est le projet stats-ia ?', 'Parle-moi du projet jarvis v2',
            'Lance Claude Code dans le projet lego roller coaster', 'Ouvre le projet portail frankenstein dans VS Code'].map((s) => h('span', { class: 'chip passive' }, `« ${s} »`))),
        h('div', { class: 'help', style: 'margin-top:8px' }, 'Pour une question plus libre (« compare mes projets Angular », « lequel n’a pas été commité ? »), Claude Code interroge la même liste avec l’outil projets.')),
      panel('Mes projets', d ? `${shown.length}${f ? ` / ${all.length}` : ''}` : '',
        h('div', { class: 'row', style: 'margin-bottom:10px' },
          h('input', { type: 'search', placeholder: 'Filtrer (nom, techno, description)…', value: state.filter,
            oninput: (e) => { state.filter = e.target.value; render(true); setTimeout(() => { const s = document.querySelector('#section-projects input[type=search]'); if (s) { s.focus(); s.setSelectionRange(s.value.length, s.value.length); } }, 0); } })),
        !d ? h('div', { class: 'empty' }, state.loading ? 'Lecture des projets…' : '—')
          : !d.exists ? h('div', { class: 'empty' }, `Dossier introuvable : ${d.root}`)
            : shown.length ? h('table', {},
              h('thead', {}, h('tr', {}, h('th', {}, 'Projet'), h('th', {}, 'Technologies'), h('th', {}, 'Git'), h('th', {}, 'Activité'), h('th', {}, ''))),
              h('tbody', {}, ...shown.map(projectRow)))
              : h('div', { class: 'empty' }, 'Aucun projet ne correspond.'))
    ];
  };
})();
