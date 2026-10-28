// Module Diction : corriger la prononciation de Jarvis. Le noyau (orch/pronunciation.rs) envoie l'événement
// « widget » kind diction_view (« corrige ta prononciation », card Actions) ; tout passe par la méthode « diction »
// (recentes, ecouter, essayer, enregistrer, oublier, pilote : pause, reprise et scènes d'une présentation Data).
(function () {
  const J = (window.JARVIS = window.JARVIS || {});
  const bridge = window.jarvisBridge;
  const K = J.widgetKit;
  if (!K) return;
  const { h, place, gripHandle, numBadge, closeButton } = K;
  const KEY = 'diction_view';
  const MAX_WORDS = 4;
  let last = null;
  // Mot ou expression choisi : phrase, premier et dernier mot.
  let sel = null;
  let message = '';

  async function call(op, args) {
    if (!bridge || !bridge.call) return null;
    try { return await bridge.call('diction', op, args || {}); } catch { return null; }
  }

  async function refresh(extra) {
    const v = await call('recentes');
    if (v && v.ok) render({ ...last, ...v, ...(extra || {}) });
  }

  // Présentation Data en cours : la barre suit les scènes. Un mot en cours de correction n'est pas dérangé
  // (les phrases ne sont pas relues, seul l'état de la présentation l'est).
  async function followPresentation() {
    if (!last) return;
    const v = await call('recentes');
    if (!v || !v.ok) return;
    if (sel) render({ ...last, presentation: v.presentation });
    else render({ ...last, ...v });
  }
  let followTimer = null;
  if (bridge && bridge.onTaskEvent) {
    bridge.onTaskEvent((e) => {
      if (!e || !String(e.type || '').startsWith('presentation') || !last) return;
      clearTimeout(followTimer);
      followTimer = setTimeout(followPresentation, 700); // laisse la phrase dite entrer dans la liste
    });
  }

  async function pilot(action) {
    const r = await call('pilote', { action });
    if (r && r.ok) { sel = null; message = ''; render({ ...last, ...r }); } else { message = (r && r.error) || 'Pilotage impossible.'; render(last); }
  }

  function presentationBar(pr) {
    const b = (label, tip, action, cls = '') => h('button', { class: `w-note-btn${cls ? ` ${cls}` : ''}`, title: tip, onclick: () => pilot(action) }, label);
    return h('div', { class: 'dc-pres' },
      h('div', { class: 'dc-line' }, h('span', { class: 'dc-label' }, 'Présentation'),
        h('b', {}, `${pr.sujet || 'Data'} · scène ${pr.n}/${pr.total}`),
        h('span', { class: `dc-tag${pr.pause ? '' : ' ok'}` }, pr.pause ? 'en pause' : 'en cours')),
      h('div', { class: 'w-note-actions' },
        b('⏮', 'Revenir au début', 'debut'),
        b('◀', 'Scène précédente', 'precedent'),
        pr.pause ? b('▶ Relancer', 'Redire la scène en cours (avec les corrections)', 'reprendre', 'primary') : b('⏸ Pause', 'Mettre la présentation en pause', 'pause'),
        b('▶▶', 'Scène suivante', 'suivant'),
        h('span', { class: 'w-note-spacer' }),
        b('■ Arrêter', 'Arrêter la présentation', 'stop')));
  }

  // « d'Airbus » → « Airbus » : la règle porte sur le mot, pas sur l'élision.
  const bare = (w) => w.replace(/^(?:[ldjmnstc]|qu|jusqu|lorsqu|puisqu)['’](?=\p{L})/iu, '');

  function selection(ev) {
    if (!sel) return null;
    const p = (ev.phrases || []).find((x) => x.id === sel.phrase);
    if (!p) return null;
    const words = p.mots.slice(sel.from, sel.to + 1);
    if (!words.length) return null;
    const text = words.map((w, i) => (i === 0 ? bare(w.mot) : w.mot)).join(' ');
    const rule = words.length === 1 ? words[0].regle : (words.every((w) => w.regle && w.regle.expression) ? words[0].regle : null);
    return { p, words, text, rule, said: rule ? rule.dire : words.map((w) => w.dit).join(' ') };
  }

  function pick(phrase, index) {
    if (sel && sel.phrase === phrase && (index === sel.to + 1 || index === sel.from - 1) && sel.to - sel.from + 1 < MAX_WORDS) {
      sel = { phrase, from: Math.min(sel.from, index), to: Math.max(sel.to, index) }; // expression : mot voisin ajouté
    } else if (sel && sel.phrase === phrase && sel.from === index && sel.to === index) {
      sel = null; // second clic : désélection
    } else {
      sel = { phrase, from: index, to: index };
    }
    message = '';
    render(last);
  }

  function editor(ev, s) {
    const input = h('input', { class: 'dc-input', type: 'text', value: s.rule ? s.rule.dire : '', placeholder: 'Comment le dire (ex. hélicoptère)', spellcheck: 'false' });
    const all = h('input', { type: 'radio', name: 'dc-voix', checked: true });
    const one = h('input', { type: 'radio', name: 'dc-voix' });
    const btn = (label, tip, fn, cls = '') => h('button', { class: `w-note-btn${cls ? ` ${cls}` : ''}`, title: tip, onclick: fn }, label);
    const save = async () => {
      const dire = input.value.trim();
      if (!dire) { message = 'Écris d’abord comment le dire.'; render(last); return; }
      const r = await call('enregistrer', { mot: s.text, dire, toutesVoix: all.checked });
      if (!r || r.ok === false) { message = (r && r.error) || 'Enregistrement impossible.'; render(last); return; }
      message = `✓ « ${s.text} » se dira « ${dire} »${all.checked ? '' : ` avec ${ev.voixNom || 'cette voix'}`}.`;
      sel = null;
      render({ ...last, ...r });
      call('ecouter', { texte: s.p.texte }); // la phrase entière, corrigée
    };
    input.addEventListener('keydown', (e) => { if (e.key === 'Enter') save(); });
    setTimeout(() => input.focus(), 30);
    return h('div', { class: 'dc-editor' },
      h('div', { class: 'dc-line' }, h('span', { class: 'dc-label' }, 'Écrit'), h('b', {}, s.text)),
      h('div', { class: 'dc-line' }, h('span', { class: 'dc-label' }, 'Lu par la voix'), h('span', {}, s.said),
        s.rule ? h('span', { class: 'dc-tag ok' }, 'corrigé') : null),
      h('div', { class: 'w-note-actions' },
        btn('▶ Écouter tel quel', 'Comment Jarvis le dit maintenant', () => call('ecouter', { texte: s.text })),
        s.words[0].raison ? h('span', { class: 'dc-why' }, s.words[0].raison) : null),
      h('div', { class: 'dc-line' }, h('span', { class: 'dc-label' }, 'Dire comme'), input),
      h('div', { class: 'dc-voices' },
        h('label', {}, all, ' Toutes les voix'),
        h('label', {}, one, ` Seulement ${ev.voixNom || 'cette voix'}`)),
      h('div', { class: 'w-note-actions' },
        btn('▶ Essayer', 'Écouter la proposition sans l’enregistrer', () => { if (input.value.trim()) call('essayer', { dire: input.value }); }),
        h('span', { class: 'w-note-spacer' }),
        s.rule ? btn('✕ Oublier', 'Retirer la correction de ce mot (toutes les voix)', async () => {
          const r = await call('oublier', { mot: s.rule.mot });
          message = `Correction de « ${s.rule.mot} » oubliée.`;
          sel = null;
          if (r && r.ok) render({ ...last, ...r });
        }) : null,
        btn('✓ Enregistrer', 'Jarvis le dira ainsi désormais', save, 'primary')));
  }

  function chip(p, w, i) {
    const on = sel && sel.phrase === p.id && i >= sel.from && i <= sel.to;
    const cls = ['dc-word', w.regle ? 'fixed' : '', !w.regle && w.suspect ? 'suspect' : '', on ? 'on' : ''].filter(Boolean).join(' ');
    const tip = w.regle ? `Corrigé : « ${w.regle.mot} » se dit « ${w.regle.dire} »`
      : w.suspect ? `${w.raison} — touche pour écouter et corriger`
        : w.auto ? `Lu « ${w.dit} »` : 'Touche pour écouter et corriger';
    return h('button', { class: cls, title: tip, onclick: () => pick(p.id, i) }, w.mot);
  }

  function render(ev) {
    last = ev;
    const phrases = ev.phrases || [];
    const s = selection(ev);
    const el = h('div', { class: 'widget w-tool w-card-list w-wide w-diction' },
      h('div', { class: 'w-head' },
        gripHandle(), numBadge(ev),
        h('span', { class: 'w-icon' }, '🗣'),
        h('span', { class: 'w-title' }, 'DICTION'),
        h('button', { class: 'w-note-btn dc-refresh', title: 'Relire les dernières phrases', onclick: () => { sel = null; message = ''; refresh(); } }, '↻'),
        closeButton(ev, KEY)),
      ev.presentation ? presentationBar(ev.presentation) : null,
      h('div', { class: 'dc-help' }, phrases.length
        ? 'Touche le mot mal dit (touche aussi le mot voisin pour une expression). En orange : mots probablement mal lus ; en vert : déjà corrigés.'
        : 'Jarvis n’a encore rien dit depuis son démarrage.'),
      ...phrases.map((p) => h('div', { class: 'dc-phrase' },
        h('div', { class: 'dc-words' }, ...p.mots.map((w, i) => chip(p, w, i))),
        h('button', { class: 'dc-replay', title: 'Réécouter la phrase', onclick: () => call('ecouter', { texte: p.texte }) }, '▶'))),
      s ? editor(ev, s) : null,
      message ? h('div', { class: 'dc-message' }, message) : null,
      h('div', { class: 'w-rows-foot' }, `Voix : ${ev.voixNom || ev.voix || '?'} · toutes les corrections : Administration › Voix › Prononciation`));
    place(KEY, el, KEY, 0);
  }

  K.register(KEY, (ev) => { sel = null; message = ''; render(ev); });
  K.vanish(KEY);
})();
