// Module Nomade : la card du site Nomade (orch/nomade). Le noyau envoie l'événement « widget » kind nomade_view
// (« montre Nomade », dépôt reçu) ; les boutons passent par la méthode « nomade » (etat, synchroniser, depot, ecart).
(function () {
  const J = (window.JARVIS = window.JARVIS || {});
  const bridge = window.jarvisBridge;
  const K = J.widgetKit;
  if (!K) return;
  const { h, place, gripHandle, numBadge, closeButton } = K;
  const KEY = 'nomade_view';
  let last = null;
  let notice = '';
  const say = (text) => { if (bridge && bridge.call) bridge.call('utterance', text).catch(() => {}); };

  const ago = (ms) => {
    if (!ms) return 'jamais';
    const s = (Date.now() - ms) / 1000;
    if (s < 90) return "à l'instant";
    if (s < 3600) return `il y a ${Math.round(s / 60)} min`;
    return new Date(ms).toLocaleTimeString('fr-FR', { hour: '2-digit', minute: '2-digit' });
  };

  async function call(op, ...args) {
    if (!bridge || !bridge.call) return null;
    try {
      const st = await bridge.call('nomade', op, ...args);
      notice = op === 'ecart' && st ? (st.ok ? st.say : `⚠ ${st.error}`) || '' : '';
      if (st && st.sources && last) render({ ...last, ...st, shared: st.sources.filter((s) => s.shared).length });
      return st;
    } catch { return null; }
  }

  // Demande de nouvelle liste (propriétaire ou invité) : Jarvis prépare le modèle, le propriétaire valide.
  function requestRow(d, btn) {
    const q = d.demande || {};
    const who = d.requester || {};
    const guest = who.role !== 'owner';
    return h('div', { class: 'w-nm-depot' },
      h('div', { class: 'w-nm-text' }, `✚ ${q.nom || 'Nouvelle liste'}${guest ? ` · demandée par ${who.nom || who.login} (invité)` : ''}`),
      h('div', { class: 'w-nm-sub' }, q.type || ''),
      (q.champs || []).length ? h('div', { class: 'w-nm-sub' }, `Champs : ${q.champs.join(', ')}`) : null,
      h('div', { class: 'w-nm-sub' }, `nouvelle liste · déposée ${ago(d.createdAt)}`),
      h('div', { class: 'w-note-actions' },
        btn('✔ Créer la liste', 'Jarvis prépare le modèle (IA sans outil), crée la liste et la partage sur le site', () => call('depot', d.id, 'creer_liste'), 'primary'),
        h('span', { class: 'w-note-spacer' }),
        btn('✕ Refuser', 'Refuser cette demande', () => call('depot', d.id, 'refuser'))));
  }

  // Modification du site refusée par Jarvis, ou appliquée alors que Jarvis avait aussi changé la fiche.
  function conflictRow(c, btn) {
    const detail = c.erreur || (c.champs || []).map((f) => `${f.champ} « ${f.jarvis} » → « ${f.site} »`).join(', ');
    return h('div', { class: 'w-nm-depot' },
      h('div', { class: 'w-nm-text' }, `⚠ ${c.titre || c.id}${c.fois > 1 ? ` ×${c.fois}` : ''}`),
      h('div', { class: 'w-nm-sub' }, `${c.sourceLabel || c.source} · ${ago(c.at)} · ${detail}`),
      h('div', { class: 'w-note-actions' },
        c.retry ? btn('↻ Réessayer', 'Rejouer la modification du site', () => call('ecart', 'reessayer', c.at, c.id), 'primary') : null,
        h('span', { class: 'w-note-spacer' }),
        c.retry ? btn('✕ Ignorer', 'Garder la version de Jarvis : la fiche refusée est retirée du site', () => call('ecart', 'ignorer', c.at, c.id))
          : btn('✓ Vu', 'Retirer de la liste à vérifier', () => call('ecart', 'ignorer', c.at, c.id))));
  }

  function render(ev) {
    last = ev;
    const depots = Array.isArray(ev.depots) ? ev.depots.slice().reverse() : [];
    const conflicts = Array.isArray(ev.conflicts) ? ev.conflicts : [];
    const toCheck = Array.isArray(ev.conflicts) ? conflicts.length : ev.conflicts || 0; // noyau d'avant 4.88 : un nombre
    const btn = (label, tip, fn, cls = '') => h('button', {
      class: `w-note-btn${cls ? ` ${cls}` : ''}`, title: tip,
      onclick: (e) => { e.currentTarget.disabled = true; fn(); }
    }, label);
    const state = !ev.configured ? 'Site non relié : Administration › Nomade.'
      : ev.lastError ? `Erreur : ${ev.lastError}`
        : `${ev.shared || 0} source(s) partagée(s) · dernier échange ${ago(ev.lastSync)}${toCheck ? ` · ${toCheck} à vérifier` : ''}`;
    const el = h('div', { class: 'widget w-tool w-card-list w-wide w-nomade' },
      h('div', { class: 'w-head' },
        gripHandle(), numBadge(ev),
        h('span', { class: 'w-icon' }, '⇅'),
        h('span', { class: 'w-title' }, 'NOMADE'),
        depots.length ? h('span', { class: 'w-count' }, String(depots.length)) : null,
        closeButton(ev, KEY)),
      h('div', { class: `w-nm-state${ev.lastError ? ' w-nm-err' : ''}` }, state),
      ...depots.map((d) => d.kind === 'nouvelle_liste' ? requestRow(d, btn) : h('div', { class: 'w-nm-depot' },
        h('div', { class: 'w-nm-text' }, d.text),
        h('div', { class: 'w-nm-sub' }, `${d.kind} · déposé ${ago(d.createdAt)}`),
        h('div', { class: 'w-note-actions' },
          btn('✎ En note', 'Enregistrer comme note', () => call('depot', d.id, 'note')),
          d.kind === 'rappel'
            ? btn('⏰ Rappel', 'Créer le rappel (Jarvis lit la date dans le texte)', () => call('depot', d.id, 'rappel'), 'primary')
            : btn('▶ Demander', 'Envoyer à Jarvis comme une demande tapée', () => call('depot', d.id, 'demande'), d.kind === 'demande' ? 'primary' : ''),
          h('span', { class: 'w-note-spacer' }),
          btn('✕ Ignorer', 'Retirer ce dépôt', () => call('depot', d.id, 'ignorer'))))),
      depots.length ? null : h('div', { class: 'w-rows-foot' }, 'Aucun dépôt en attente.'),
      conflicts.length ? h('div', { class: 'w-nm-group' }, `À vérifier (${conflicts.length})`) : null,
      ...conflicts.slice(0, 5).map((c) => conflictRow(c, btn)),
      conflicts.length > 5 ? h('div', { class: 'w-rows-foot' }, `… et ${conflicts.length - 5} autre(s) : Administration › Nomade.`) : null,
      notice ? h('div', { class: 'w-nm-sub' }, notice) : null,
      h('div', { class: 'w-note-actions' },
        conflicts.length ? btn('✕ Tout effacer', 'Vider la liste à vérifier (les fiches refusées sont retirées du site)', () => call('ecart', 'effacer')) : null,
        h('span', { class: 'w-note-spacer' }),
        ev.url ? h('button', { class: 'w-note-btn', title: `Ouvrir ${ev.url} dans le navigateur`, onclick: () => say(`Ouvre ${ev.url}`) }, '↗ Ouvrir le site') : null,
        ev.configured ? btn('⇅ Synchroniser', 'Échanger maintenant avec le site', () => call('synchroniser')) : null));
    place(KEY, el, KEY, 0);
  }

  K.register(KEY, render);
  K.vanish(KEY);
})();
