// Module Data : la card de suivi d'une recherche (« fais un dossier sur… »). Elle s'ouvre au lancement (événement
// « widget » kind savoir_suivi) et montre en direct l'étape en cours (attente, recherche, vérification, images,
// indexation, prêt), le temps écoulé sur le temps donné, les chiffres du dossier, chaque élément recueilli par l'IA
// (« savoir_progress » : le noyau relit le fichier qu'elle écrit), son activité (recherches et pages lues : événements
// « update » de la tâche) puis ce qui est enregistré dans le RAG du dossier.
(function () {
  const J = (window.JARVIS = window.JARVIS || {});
  const bridge = window.jarvisBridge;
  const K = J.widgetKit;
  if (!K) return;
  const { h, place, cards, gripHandle, numBadge, closeButton } = K;
  const KEY = 'savoir_suivi';
  const MAX_LINES = 250;

  const STEPS = [
    ['attente', 'File'], ['recherche', 'Recherche web'], ['verification', 'Vérification'],
    ['images', 'Images'], ['indexation', 'Indexation RAG'], ['presentation', 'Présentation'], ['pret', 'Prêt']
  ];
  const COUNTS = [['sources', 'sources'], ['fiche', 'caractéristiques'], ['sections', 'sections'], ['images', 'images'],
    ['videos', 'vidéos'], ['pieces', 'pièces'], ['chronologie', 'dates'], ['comparaisons', 'comparaisons']];
  const ICON = { 'résumé': '≡', fiche: '▦', section: '¶', chronologie: '◷', comparaison: '⇄', 'pièce': '◎', image: '▣', 'vidéo': '▶', 'modèle 3D': '◆', 'maquette 3D': '◇', source: '↗' };

  let s = null; // { slug, sujet, minutes, debut, phase, taskId, ui: {…}, counts, done }
  const call = (op, args) => (bridge && bridge.call ? Promise.resolve(bridge.call('savoir', op, args)).catch(() => null) : Promise.resolve(null));
  const clock = (ms) => { const t = Math.max(0, Math.floor(ms / 1000)); return `${Math.floor(t / 60)}:${String(t % 60).padStart(2, '0')}`; };
  const hhmmss = () => new Date().toLocaleTimeString('fr-FR', { hour: '2-digit', minute: '2-digit', second: '2-digit' });

  function line(list, cls, ...children) {
    const el = h('div', { class: `ss-line ${cls || ''}` }, h('span', { class: 'ss-time' }, hhmmss()), ...children);
    list.prepend(el);
    while (list.childElementCount > MAX_LINES) list.lastElementChild.remove();
    const empty = list.parentElement && list.parentElement.querySelector('.ss-empty');
    if (empty) empty.remove();
  }

  function log(text, cls) {
    if (!s) return;
    line(s.ui.activity, cls, h('span', { class: 'ss-text' }, text));
  }

  function setPhase(phase) {
    if (!s) return;
    s.phase = phase;
    const idx = STEPS.findIndex(([id]) => id === phase);
    s.ui.steps.forEach((el, i) => {
      const failed = phase === 'echec' && i === s.ui.steps.length - 1;
      el.className = `ss-step${failed ? ' fail' : i < idx || (phase === 'pret' && i === idx) ? ' done' : i === idx ? ' now' : ''}`;
      if (failed) el.textContent = 'Échec';
    });
  }

  function setCounts(c) {
    if (!s || !c) return;
    for (const [k] of COUNTS) {
      const el = s.ui.counts[k];
      if (!el || c[k] === undefined) continue;
      const v = String(c[k]);
      if (el.textContent !== v) { el.textContent = v; el.parentElement.classList.remove('bump'); void el.offsetWidth; el.parentElement.classList.add('bump'); }
    }
  }

  function render(ev) {
    const steps = STEPS.map(([, label]) => h('span', { class: 'ss-step' }, label));
    const countEls = {};
    const countBox = h('div', { class: 'ss-counts' }, ...COUNTS.map(([k, label]) => { countEls[k] = h('b', {}, '0'); return h('span', { class: 'ss-count' }, countEls[k], label); }));
    const timer = h('span', { class: 'ss-timer' }, '0:00');
    const chip = h('span', { class: 'ss-chip' }, 'EN FILE');
    const found = h('div', { class: 'ss-list' });
    const activity = h('div', { class: 'ss-list ss-activity' });
    const rag = h('div', { class: 'ss-rag' });
    const actions = h('div', { class: 'w-note-actions ss-actions' });
    const el = h('div', { class: 'widget w-tool w-card-list w-wide ss-card', id: 'mod-savoir-suivi' },
      h('div', { class: 'w-head' },
        gripHandle(), numBadge(ev),
        h('span', { class: 'w-icon' }, '◈'),
        h('span', { class: 'w-title ss-title', title: ev.sujet || '' }, `SUIVI · ${ev.sujet || ev.slug}`),
        chip, timer,
        closeButton(ev, KEY)),
      ev.axe ? h('div', { class: 'ss-axe' }, /^https?:/i.test(ev.axe) ? `Lien à lire : ${ev.axe}` : `Approfondissement : ${ev.axe}`) : null,
      h('div', { class: 'ss-steps' }, ...steps),
      countBox,
      h('div', { class: 'ss-cols' },
        h('div', { class: 'ss-col' }, h('div', { class: 'ss-sec' }, 'RECUEILLI'), h('div', { class: 'ss-empty' }, 'En attente des premières informations…'), found),
        h('div', { class: 'ss-col' }, h('div', { class: 'ss-sec' }, 'ACTIVITÉ'), activity)),
      rag,
      actions);
    s = {
      slug: ev.slug, sujet: ev.sujet || ev.slug, minutes: ev.minutes || 5, debut: Date.now() - Math.max(0, (ev.at || Date.now()) - (ev.debut || Date.now())),
      phase: 'attente', taskId: null, done: false, endedAt: 0,
      ui: { el, steps, counts: countEls, timer, chip, found, activity, rag, actions }
    };
    place(KEY, el, KEY, 0);
    setPhase('attente');
    log(`Recherche lancée : ${s.minutes} min de recherche web, puis vérification, images et indexation dans le RAG.`, 'ss-info');
    if (ev.deja && ev.deja.sources) log(`Dossier existant : ${ev.deja.sources} sources, ${ev.deja.sections || 0} sections déjà enregistrées ; seules les nouveautés seront ajoutées.`, 'ss-info');
  }

  const PHASE_CHIP = { attente: 'EN FILE', recherche: 'RECHERCHE', verification: 'VÉRIFICATION', images: 'IMAGES', indexation: 'INDEXATION', presentation: 'PRÉSENTATION', pret: 'PRÊT', echec: 'ÉCHEC' };

  function onProgress(ev) {
    if (!s || ev.slug !== s.slug || !cards.get(KEY)) return;
    const u = s.ui;
    if (ev.taskId && ev.taskId !== s.taskId) { s.taskId = ev.taskId; log('L’IA a commencé la recherche.', 'ss-info'); }
    if (ev.phase && ev.phase !== s.phase) setPhase(ev.phase);
    u.chip.textContent = PHASE_CHIP[s.phase] || s.phase.toUpperCase();
    u.chip.className = `ss-chip ${s.phase}`;
    if (ev.compte) setCounts(ev.compte);
    for (const n of ev.nouveaux || []) {
      line(u.found, '', h('span', { class: 'ss-kind', title: n.type }, ICON[n.type] || '•'), h('span', { class: 'ss-text' }, h('i', {}, `${n.type} · `), n.texte));
    }
    if (ev.note) log(ev.note, 'ss-info');
    if (ev.erreur) log(`Erreur : ${ev.erreur}`, 'ss-bad');
    if (ev.phase === 'images' && ev.image) log(`${ev.image.ok ? '✓' : '✗'} image ${ev.fait}/${ev.sur}${ev.image.legende ? ` : ${ev.image.legende}` : ''}${ev.image.ok ? '' : ` (${ev.image.erreur || 'refusée'})`}`, ev.image.ok ? '' : 'ss-bad');
    else if (ev.phase === 'images' && ev.sur && !ev.fait) log(`Téléchargement de ${ev.sur} image(s) (https, 3 Mo au plus chacune)…`, 'ss-info');
    if (ev.ajouts !== undefined && ev.phase === 'verification' && ev.compte) log(`Fusion : ${ev.ajouts} élément(s) ajouté(s) au dossier.`, 'ss-ok');
    if (ev.fin) finish(ev);
  }

  function finish(ev) {
    s.done = true;
    s.endedAt = Date.now();
    if (ev.total) setCounts({ ...ev.total, images: ev.total.images });
    const r = ev.rag || {};
    s.ui.rag.replaceChildren(
      h('div', { class: 'ss-sec' }, ev.phase === 'pret' ? 'ENREGISTRÉ DANS LE RAG' : 'RÉSULTAT'),
      h('div', { class: 'ss-rag-line' },
        h('b', {}, String(r.passages || 0)), ' passages indexés · ', h('b', {}, String(r.vecteurs || 0)), ' vecteurs',
        r.calcules ? ` (${r.calcules} calculés)` : '', ' · ', r.semantique ? 'recherche par le sens' : 'recherche par mots-clés (modèle d’embeddings absent)',
        ' · ', h('b', {}, String(ev.ajouts || 0)), ' ajouts, ', h('b', {}, String(ev.images || 0)), ' images téléchargées',
        ev.tempsEcoule ? ' · temps écoulé' : ''),
      ...(ev.exemples || []).map((x) => h('div', { class: 'ss-passage', title: 'Passage indexé' }, x)));
    log(ev.phase === 'pret' ? `Dossier enregistré${ev.tempsEcoule ? ' (temps écoulé)' : ''}.` : `Échec${ev.erreur ? ` : ${ev.erreur}` : ''}.`, ev.phase === 'pret' ? 'ss-ok' : 'ss-bad');
    // Préparation automatique selon le profil du dossier : présentation écrite, modèle 3D réaliste ou écran médias.
    const prep = ev.preparation || {};
    if (prep.scenes) {
      const fin = prep.finale === 'exploration' ? `explorateur 3D${prep.modeles3d ? ` (${prep.modeles3d} modèle(s) réaliste(s))` : ''}` : prep.finale === 'media' ? 'écran écoute et vidéos' : 'liens';
      log(`Profil ${prep.profil || '—'} : présentation prête (${prep.scenes} scènes), fin par ${fin}.`, 'ss-ok');
    }
    const btn = (label, tip, fn, cls = '') => h('button', { class: `w-note-btn${cls ? ` ${cls}` : ''}`, title: tip, onclick: fn }, label);
    s.ui.actions.replaceChildren(
      btn('◈ Ouvrir le dossier', 'Fiche du dossier dans la card Data', () => call('ouvrir', { slug: s.slug })),
      ev.phase === 'pret' ? btn('▶ Présenter', 'Présentation animée dans le cœur', () => call('presenter', { slug: s.slug }), 'primary') : btn('↻ Relancer', 'Relancer la recherche', () => call('relancer', { slug: s.slug })));
  }

  // Activité de l'IA : chaque outil utilisé (« recherche web : … », « consultation web : … »).
  function onTaskUpdate(ev) {
    if (!s || !s.taskId || ev.id !== s.taskId || !ev.detail) return;
    if (ev.detail === s.lastDetail) return;
    s.lastDetail = ev.detail;
    const cls = ev.status === 'awaiting' ? 'ss-bad' : '';
    log(ev.status === 'awaiting' ? `Accord demandé : ${ev.detail}` : ev.detail, cls);
  }

  setInterval(() => {
    if (!s || !cards.get(KEY)) return;
    const spent = (s.done ? s.endedAt : Date.now()) - s.debut;
    s.ui.timer.textContent = `${clock(spent)} / ${s.minutes}:00`;
    s.ui.timer.classList.toggle('over', spent > s.minutes * 60000);
  }, 1000);

  K.register(KEY, render);
  K.on('savoir_progress', onProgress);
  K.on('update', onTaskUpdate);
  K.vanish(KEY);
})();
