// Module Data (« Savoir ») : la liste des dossiers de recherche et la fiche d'un dossier (résumé, fiche technique,
// images, vidéos, sources, historique des recherches, profil du dossier et ses liens). Le noyau (orch/savoir/)
// envoie l'événement « widget » kind savoir ; tout passe par la méthode savoir (liste, ouvrir, detail, image, creer,
// approfondir, lien, relancer, explorer, medias, ecouter, supprimer) ; « savoir_changed » rafraîchit la card ouverte.
// Les images arrivent en adresse data: (jamais le disque).
(function () {
  const J = (window.JARVIS = window.JARVIS || {});
  const bridge = window.jarvisBridge;
  const K = J.widgetKit;
  if (!K) return;
  const { h, place, cards, gripHandle, numBadge, closeButton } = K;
  const KEY = 'savoir';

  const ETATS = { en_cours: ['EN COURS', 'busy'], interrompu: ['INTERROMPU', 'warn'], echec: ['ÉCHEC', 'warn'], pret: ['', ''] };
  const call = (op, args = {}) => (bridge && bridge.call ? Promise.resolve(bridge.call('savoir', op, args)).catch((err) => { console.error('data :', err); return null; }) : Promise.resolve(null));
  const say = (text) => { if (bridge && bridge.call) bridge.call('utterance', text).catch(() => {}); };

  let last = null; // dernier événement affiché (numéro de card compris)
  const thumbs = new Map(); // « slug/fichier » → adresse data:

  // Frappe dans un champ : les raccourcis du HUD ne s'appliquent pas.
  function fieldKeys(e, submit) {
    if (/^F\d{1,2}$/.test(e.key)) return;
    e.stopPropagation();
    if (e.key === 'Escape') e.target.blur();
    if (e.key === 'Enter') { e.preventDefault(); submit(); }
  }

  function lazyImage(slug, file, cls, alt) {
    const box = h('div', { class: cls });
    if (!file) { box.classList.add('sv-noimg'); box.textContent = '◈'; return box; }
    const key = `${slug}/${file}`;
    const show = (src) => {
      if (!src) { box.classList.add('sv-noimg'); box.textContent = '◈'; return; }
      box.append(h('img', { src, alt: alt || '', draggable: 'false' }));
    };
    if (thumbs.has(key)) show(thumbs.get(key));
    else call('image', { slug, fichier: file }).then((r) => { const src = r && r.src; if (src) thumbs.set(key, src); show(src); });
    return box;
  }

  function stateChip(etat) {
    const [label, tone] = ETATS[etat] || ['', ''];
    return label ? h('span', { class: `sv-chip ${tone}` }, label) : null;
  }

  const plural = (n, word) => `${n} ${word}${n > 1 ? 's' : ''}`;

  function head(ev, icon, title, extra) {
    return h('div', { class: 'w-head' },
      gripHandle(),
      numBadge(ev),
      h('span', { class: 'w-icon' }, icon),
      h('span', { class: 'w-title sv-title', title }, title),
      extra || null,
      h('span', { class: 'w-source' }, 'DATA'),
      closeButton(ev, KEY));
  }

  // --- Liste des dossiers -------------------------------------------------------------------------------
  function renderList(ev) {
    const list = Array.isArray(ev.dossiers) ? ev.dossiers : [];
    const input = h('input', { class: 'sv-input', type: 'text', placeholder: 'Nouveau sujet : hélicoptère H160, batteries au sodium…', spellcheck: 'true', lang: 'fr' });
    const minutes = h('select', { class: 'sv-select', title: 'Temps de recherche' },
      ...[2, 5, 10, 15, 30].map((m) => h('option', { value: String(m), selected: m === 5 ? 'selected' : null }, `${m} min`)));
    const launch = () => {
      const sujet = input.value.trim();
      if (sujet.length < 2) { input.focus(); return; }
      input.value = '';
      call('creer', { sujet, minutes: Number(minutes.value) || 5 });
    };
    input.addEventListener('keydown', (e) => fieldKeys(e, launch));

    const rows = list.map((d) => {
      const s = d.stats || {};
      return h('button', { class: 'w-row clickable sv-row', title: 'Clic : ouvrir le dossier', onclick: () => call('ouvrir', { slug: d.slug }) },
        lazyImage(d.slug, d.vignette, 'sv-thumb'),
        h('span', { class: 'w-row-text' },
          h('span', { class: 'w-row-title' }, d.sujet || d.slug),
          h('span', { class: 'w-row-sub' }, [d.categorie, plural(s.sources || 0, 'source'), plural(s.images || 0, 'image'), s.questions ? plural(s.questions, 'question') : ''].filter(Boolean).join(' · '))),
        h('span', { class: 'w-row-meta' }, stateChip(d.etat) || (d.maj ? String(d.maj).slice(0, 10) : '')));
    });

    const el = h('div', { class: 'widget w-tool w-card-list w-wide sv-card', id: 'mod-savoir' },
      head(ev, '◈', `DATA · ${plural(list.length, 'dossier')}`),
      list.length
        ? h('div', { class: 'w-rows sv-list' }, ...rows)
        : h('div', { class: 'sv-empty' }, 'Aucun dossier. Donne un sujet ci-dessous, ou dis « fais un dossier sur… ».'),
      h('div', { class: 'w-note-actions sv-new' }, input, minutes, h('button', { class: 'w-note-btn primary', title: 'Lancer la recherche en fond', onclick: launch }, '✦ Rechercher')));
    place(KEY, el, KEY, 0);
  }

  // --- Fiche d'un dossier -------------------------------------------------------------------------------
  function section(title, ...children) {
    const items = children.filter(Boolean);
    return items.length ? h('div', { class: 'sv-sec' }, h('div', { class: 'sv-sec-title' }, title), ...items) : null;
  }

  function renderDossier(ev) {
    const d = ev.dossier || {};
    const s = d.stats || {};
    const slug = d.slug;
    const running = d.etat === 'en_cours';
    const btn = (label, tip, fn, cls = '') => h('button', { class: `w-note-btn${cls ? ` ${cls}` : ''}`, title: tip, onclick: fn }, label);
    // Profil du dossier (produit, musique, film, lieu… ou créé par l'IA) : ses liens utiles.
    const profil = d.profil || {};
    const profilLinks = (profil.boutons || []).map((b) => btn(`${b.icone || '↗'} ${b.libelle}`, b.url ? `Ouvrir ${b.url}` : `Spotify : ${b.requete}`,
      () => (b.type === 'spotify' ? call('ecouter', { requete: b.requete }) : say(`Ouvre ${b.url}`))));

    const axe = h('input', { class: 'sv-input', type: 'text', placeholder: 'Approfondir : versions militaires, motorisation, coûts…', lang: 'fr' });
    const deepen = () => { call('approfondir', { slug, axe: axe.value.trim() }); axe.value = ''; };
    axe.addEventListener('keydown', (e) => fieldKeys(e, deepen));
    const url = h('input', { class: 'sv-input', type: 'url', placeholder: 'Lien d’une page ou d’une vidéo à ajouter (https://…)' });
    const addLink = () => { const u = url.value.trim(); if (!/^https?:\/\//i.test(u)) { url.focus(); return; } call('lien', { slug, url: u }); url.value = ''; };
    url.addEventListener('keydown', (e) => fieldKeys(e, addLink));

    let armed = null;
    const del = btn('🗑 Supprimer', 'Supprimer le dossier, ses images et son index (deux clics)', (e) => {
      const b = e.currentTarget;
      if (!armed) {
        b.textContent = '🗑 Confirmer ?';
        b.classList.add('attention');
        armed = setTimeout(() => { armed = null; b.textContent = '🗑 Supprimer'; b.classList.remove('attention'); }, 4000);
        return;
      }
      clearTimeout(armed);
      call('supprimer', { slug });
    });

    const fiche = Array.isArray(d.fiche) ? d.fiche : [];
    const ficheBox = h('div', { class: 'sv-specs' }, ...fiche.slice(0, 14).map((f) => h('div', { class: 'sv-spec', title: f.source || '' }, h('span', {}, f.cle), h('b', {}, [f.valeur, f.unite].filter(Boolean).join(' ')))));
    const moreFiche = fiche.length > 14 ? btn(`+ ${fiche.length - 14} caractéristiques`, 'Tout afficher', (e) => {
      ficheBox.append(...fiche.slice(14).map((f) => h('div', { class: 'sv-spec', title: f.source || '' }, h('span', {}, f.cle), h('b', {}, [f.valeur, f.unite].filter(Boolean).join(' ')))));
      e.currentTarget.remove();
    }, 'sv-more') : null;

    const images = Array.isArray(d.images) ? d.images : [];
    const gallery = images.length ? h('div', { class: 'sv-gallery' }, ...images.slice(0, 24).map((i) => {
      const box = lazyImage(slug, i.fichier, 'sv-shot', i.legende);
      box.title = [i.legende, i.vue, i.licence].filter(Boolean).join(' · ');
      return box;
    })) : null;

    const link = (icon, title, sub, href) => h('button', { class: 'w-row clickable', title: `Clic : ouvrir ${href}`, onclick: () => say(`Ouvre ${href}`) },
      h('span', { class: 'w-row-ico' }, icon),
      h('span', { class: 'w-row-text' }, h('span', { class: 'w-row-title' }, title), h('span', { class: 'w-row-sub' }, sub)));
    const host = (u) => String(u || '').replace(/^https?:\/\/(?:www\.)?/, '').split('/')[0];
    const videos = (Array.isArray(d.videos) ? d.videos : []).map((v) => link('▶', v.titre || host(v.url), [host(v.url), v.duree].filter(Boolean).join(' · '), v.url));
    const sources = (Array.isArray(d.sources) ? d.sources : []).slice(0, 30).map((x) => link('↗', x.titre || host(x.url), host(x.url), x.url));
    const history = (Array.isArray(d.recherches) ? d.recherches : []).slice().reverse().map((r) => h('div', { class: 'sv-hist' },
      h('span', {}, String(r.date || '').slice(0, 16).replace('T', ' ')),
      h('span', {}, r.axe ? (/^https?:/i.test(r.axe) ? `lien ${host(r.axe)}` : r.axe) : 'création'),
      h('span', {}, `${r.minutes || '?'} min`),
      h('b', { class: r.statut === 'ok' ? 'ok' : 'warn' }, r.statut === 'ok' ? `+${r.ajouts || 0}` : r.statut === 'temps_ecoule' ? `+${r.ajouts || 0} (temps écoulé)` : 'échec')));

    const el = h('div', { class: 'widget w-tool w-card-list w-wide sv-card sv-detail', id: 'mod-savoir' },
      head(ev, '◈', d.sujet || slug, h('span', { class: 'sv-head-extra' }, stateChip(d.etat), h('button', { class: 'sv-back', title: 'Tous les dossiers', onclick: () => call('ouvrir', {}) }, '☰'))),
      h('div', { class: 'sv-top' },
        lazyImage(slug, d.vignette, 'sv-cover'),
        h('div', { class: 'sv-info' },
          d.categorie || profil.nom ? h('div', { class: 'sv-cat' }, [profil.nom ? `${profil.icone || '◈'} ${profil.nom}` : '', d.categorie].filter(Boolean).join(' · ')) : null,
          h('div', { class: 'sv-resume' }, d.resume || (running ? 'Recherche en cours…' : 'Pas encore de résumé.')))),
      h('div', { class: 'sv-stats' },
        ...[[s.sources, 'sources'], [s.sections, 'sections'], [s.fiche, 'caractéristiques'], [s.images, 'images'], [d.passages, 'passages'], [d.vecteurs, 'vecteurs'], [s.questions, 'questions'], [s.pieces, 'pièces 3D']]
          .map(([n, label]) => h('span', { class: 'sv-stat' }, h('b', {}, String(n || 0)), label)),
        h('span', { class: 'sv-stat' }, h('b', {}, `${((d.taille || 0) / 1048576).toFixed(1)}`), 'Mo')),
      h('div', { class: 'sv-scroll' },
        fiche.length ? section('Fiche technique', ficheBox, moreFiche) : null,
        gallery ? section(`Images (${images.length})`, gallery) : null,
        Array.isArray(d.sections) && d.sections.length ? section('Sections', h('div', { class: 'sv-tags' }, ...d.sections.map((t) => h('span', { class: 'sv-tag' }, t)))) : null,
        Array.isArray(d.pieces) && d.pieces.length ? section('Pièces à montrer', h('div', { class: 'sv-tags' }, ...d.pieces.map((p) => h('span', { class: 'sv-tag' }, p.nom)))) : null,
        videos.length ? section(`Vidéos (${videos.length})`, h('div', { class: 'w-rows' }, ...videos)) : null,
        sources.length ? section(`Sources (${(d.sources || []).length})`, h('div', { class: 'w-rows sv-sources' }, ...sources)) : null,
        history.length ? section('Recherches', ...history) : null),
      profilLinks.length ? h('div', { class: 'w-note-actions sv-links' }, ...profilLinks) : null,
      h('div', { class: 'w-note-actions sv-actions' }, axe, btn('✦ Approfondir', 'Relancer une recherche sur cet axe (5 min)', deepen, 'primary')),
      h('div', { class: 'w-note-actions sv-actions' }, url, btn('🔗 Ajouter', 'L’IA lit la page ou la vidéo et l’ajoute au dossier', addLink)),
      h('div', { class: 'w-note-actions' },
        btn('▶ Présenter', 'Présentation animée du sujet dans le cœur, par le personnage (questions possibles pendant la présentation)', () => call('presenter', { slug }), 'primary'),
        profil.troisD ? btn('◆ 3D', 'Explorer le produit en 3D (modèle réaliste trouvé sur Sketchfab, sinon schéma) : tourner autour, cliquer les pièces, le faire fonctionner, liens', () => call('explorer', { slug })) : null,
        s.medias ? btn('♪ Écoute & vidéos', 'Écran médias : écouter sur Spotify, regarder sur YouTube, avis, prix, liens', () => call('medias', { slug })) : null,
        d.etat === 'interrompu' || d.etat === 'echec' ? btn('↻ Relancer', 'Relancer la recherche interrompue', () => call('relancer', { slug })) : null,
        h('span', { class: 'w-note-spacer' }),
        del));
    if (running) el.querySelectorAll('.sv-actions button').forEach((b) => { b.disabled = true; });
    place(KEY, el, KEY, 0);
  }

  function render(ev) {
    last = ev;
    if (ev.view === 'dossier') renderDossier(ev);
    else renderList(ev);
  }

  // Un dossier a changé (recherche lancée ou terminée, suppression) : la card ouverte se met à jour.
  async function refresh() {
    const card = cards.get(KEY);
    if (!last || !card || card.state !== 'active') return;
    if (last.view === 'dossier') {
      const r = await call('detail', { slug: last.dossier && last.dossier.slug });
      if (r && r.ok) render({ ...last, dossier: r.dossier });
      else { const l = await call('liste'); if (l) render({ ...last, view: 'liste', dossiers: l.dossiers || [] }); }
    } else {
      const r = await call('liste');
      if (r) render({ ...last, dossiers: r.dossiers || [] });
    }
  }

  K.register(KEY, render);
  K.on('savoir_changed', refresh);
  K.vanish(KEY);
})();
