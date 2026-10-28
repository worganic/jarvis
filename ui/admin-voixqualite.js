// Section « Qualité vocale » de l'administration : combien de phrases Jarvis a comprises, écartées ou acceptées de
// justesse (24 h et 7 jours, d'après le journal scellé), et les dernières phrases écartées avec leur similarité.
(function () {
  const A = window.JarvisAdmin;
  if (!A) return;
  const { h, panel, api, sections, render, fmtDate } = A;

  const state = { data: null, loading: false };
  const LABELS = [
    ['comprises', 'Comprises', 'Demandes acceptées'],
    ['prochesAcceptees', 'Voix proche acceptée', 'Similarité sous le seuil mais dans la tolérance, avec « Jarvis »'],
    ['voixRejetees', 'Voix rejetées', "L'empreinte vocale n'a pas reconnu la voix"],
    ['ignorees', 'Ignorées', 'Phrase entendue mais sans demande (écoute libre, voix douteuse…)'],
    ['parasites', 'Parole parasite', 'Bruit, télévision ou boucle écartés']
  ];
  const KIND = { voix_non_reconnue: 'voix rejetée', voix_proche_acceptee: 'proche acceptée', demande_ignoree: 'ignorée', parole_parasite_ignoree: 'parasite' };

  async function load() {
    if (state.loading) return;
    state.loading = true;
    const res = await api('voiceQuality');
    state.loading = false;
    if (res && res.ok) state.data = res;
    render(true);
  }

  const pct = (v) => `${Math.round((Number(v) || 0) * 100)} %`;

  sections.voixqualite = () => {
    if (!state.data && !state.loading) load();
    const d = state.data;
    if (!d) return [h('h1', {}, 'Qualité vocale'), h('div', { class: 'empty' }, 'Chargement…')];
    const r = d.reglages || {};
    return [
      h('h1', {}, 'Qualité vocale'),
      h('p', { class: 'lead' }, "Ce que Jarvis a fait des phrases qu'il a entendues, d'après le journal scellé. Un taux d'écartées élevé veut dire que des demandes valables sont perdues : ajuste la tolérance de l'empreinte vocale (section Voix) ou apprends-lui de nouvelles phrases."),
      panel('Synthèse 24 h', d.significatif ? `${pct(d.tauxRejet24h)} écartées sur ${d.echantillon24h} phrases` : `${d.echantillon24h} phrases : trop peu pour un taux fiable`,
        h('div', { class: 'row' }, h('button', { class: 'btn ghost', onclick: load }, '⟳ Actualiser')),
        h('table', {},
          h('thead', {}, h('tr', {}, h('th', {}, 'Catégorie'), h('th', {}, '24 h'), h('th', {}, '7 jours'), h('th', {}, 'Sens'))),
          h('tbody', {}, ...LABELS.map(([k, label, tip]) => h('tr', {}, h('td', {}, label), h('td', {}, String(d.h24[k] || 0)), h('td', {}, String(d.j7[k] || 0)), h('td', { class: 'dim' }, tip)))))),
      panel('Réglages actuels', `${r.voixConnues || 0} voix connue(s)`,
        h('div', { class: 'dim' }, `Seuil de similarité ${r.seuil ?? '—'} · tolérance « presque reconnue » ${r.tolerance !== undefined ? Math.round(r.tolerance * 100) + ' points' : '—'} · marge de doute ${r.margeDoute ?? '—'} · mode ${r.mode === 'signal' ? 'signalement' : 'rejet'}`)),
      panel('Dernières phrases écartées', `${(d.recentes || []).length}`,
        (d.recentes || []).length
          ? h('table', {},
            h('thead', {}, h('tr', {}, h('th', {}, 'Date'), h('th', {}, 'Cas'), h('th', {}, 'Phrase'), h('th', {}, 'Similarité'))),
            h('tbody', {}, ...d.recentes.map((e) => h('tr', {}, h('td', {}, fmtDate(e.at)), h('td', {}, KIND[e.type] || e.type), h('td', {}, e.texte || e.motif || '—'),
              h('td', {}, e.similarite !== undefined && e.similarite !== null ? `${Math.round(Number(e.similarite) * 100)} %${e.seuil ? ` (seuil ${Math.round(Number(e.seuil) * 100)} %)` : ''}` : '—')))))
          : h('div', { class: 'empty' }, 'Aucune phrase écartée sur la période. 👍'))
    ];
  };
})();
