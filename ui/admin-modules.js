// Section « Modules » de l'administration : les modules trouvés dans modules/ (une card ou un service chacun), avec
// leur version, leur card et un interrupteur. Interface : effet aussitôt (le HUD se recharge) ; partie noyau :
// au prochain démarrage de Jarvis.
(function () {
  const A = window.JarvisAdmin;
  const direct = window.jarvisAdmin && window.jarvisAdmin.direct;
  if (!A || !direct) return;
  const { h, panel, sections, render, toast, toggle } = A;

  const state = { data: null, loading: false, relance: false };

  async function load() {
    if (state.loading) return;
    state.loading = true;
    state.data = await direct('modulesList').catch((e) => ({ ok: false, error: String(e.message || e) }));
    state.loading = false;
    render(true);
  }

  async function set(m, on) {
    const res = await direct('modulesSet', m.id, on).catch((e) => ({ ok: false, error: String(e.message || e) }));
    if (res && res.ok) {
      state.data = res.liste;
      if (res.relance) state.relance = true;
      toast(`${m.nom} ${on ? 'activé' : 'désactivé'}`);
    } else toast((res && res.error) || 'Échec', true);
    render(true);
  }

  // Type de la card : 1 = toujours active, 2 = temporaire ; un module sans numéro fixe mais qui affiche des cards
  // (fiche › widgets) ouvre des cards temporaires, numérotées à l'ouverture par le noyau.
  const typeOf = (m) => (m.card ? m.card.type : (Array.isArray(m.widgets) && m.widgets.length ? 2 : 0));
  const typeLabel = (t) => (t === 1 ? h('span', { title: 'Card toujours active (1 à 99)' }, h('strong', {}, '1'), h('div', { class: 'dim' }, 'toujours active'))
    : t === 2 ? h('span', { title: 'Card temporaire (100 à 199)' }, h('strong', {}, '2'), h('div', { class: 'dim' }, 'temporaire'))
      : h('span', { class: 'dim' }, '—'));
  const cardLabel = (m) => (m.card ? `card ${m.card.numero}` : (typeOf(m) === 2 ? 'numéro donné à l’ouverture' : 'sans card'));

  sections.modules = () => {
    if (!state.data && !state.loading) load();
    const d = state.data;
    if (!d) return [h('h1', {}, 'Modules'), h('div', { class: 'empty' }, 'Chargement…')];
    if (!d.ok) return [h('h1', {}, 'Modules'), h('div', { class: 'empty' }, d.error || 'Liste indisponible.')];
    const mods = d.modules || [];
    return [
      h('h1', {}, 'Modules'),
      h('p', { class: 'lead' }, "Jarvis est le système principal ; chaque card est un module indépendant rangé dans le dossier modules/ (un dossier par module : sa fiche module.json, son interface ui/ et, s'il en a une, sa partie noyau core/). Pour enlever un module : désactive-le ici ou supprime son dossier. Pour en créer un : copie modules/_modele (voir modules/README.md)."),
      state.relance ? h('div', { class: 'panel' }, h('strong', {}, '⟳ Relance nécessaire : '), 'la partie noyau d’un module ne démarre ou ne s’arrête qu’au prochain démarrage de Jarvis.') : null,
      panel('Modules installés', `${mods.filter((m) => m.actif).length} / ${mods.length} actifs`,
        mods.length
          ? h('table', {},
            h('thead', {}, h('tr', {}, h('th', {}, 'Type'), h('th', {}, 'Module'), h('th', {}, 'Card'), h('th', {}, 'Partie noyau'), h('th', {}, 'Actif'))),
            // Rangés par type (1, puis 2, puis sans card), puis par nom.
            h('tbody', {}, ...[...mods].sort((a, b) => ((typeOf(a) || 9) - (typeOf(b) || 9)) || String(a.nom).localeCompare(String(b.nom), 'fr')).map((m) => h('tr', {},
              h('td', {}, typeLabel(typeOf(m))),
              h('td', {}, h('strong', {}, m.nom), ` v${m.version || '?'}`, h('div', { class: 'dim' }, m.description || ''), h('div', { class: 'dim' }, `modules/${m.id}/`)),
              h('td', {}, cardLabel(m)),
              h('td', {}, !m.noyau ? 'non (interface seule)' : (m.lie ? 'oui' : '⚠ non compilée')),
              h('td', {}, toggle('', Boolean(m.actif), (on) => set(m, on)))))))
          : h('div', { class: 'empty' }, 'Aucun module trouvé dans modules/.')),
      (d.erreurs || []).length
        ? panel('Fiches refusées', String(d.erreurs.length), h('ul', {}, ...d.erreurs.map((e) => h('li', { class: 'dim' }, e))))
        : null
    ].filter(Boolean);
  };
})();
