// Section « Journal & dialogues » de l'administration (chantier v3, étape 4).
// Lecture seule : le journal scellé se consulte et se vérifie, il ne se modifie jamais.
(function () {
  const A = window.JarvisAdmin;
  if (!A) return;
  const { h, panel, api, sections, render, confirmDialog, toast } = A;

  const state = {
    view: localStorage.getItem('admin.journal.view') || 'dialogues',
    text: '',
    type: '',
    onlyFails: false,
    entries: [],
    more: false,
    loading: false,
    loaded: false,
    integrity: null,
    stats: null,
    open: new Set(),
    commenting: null, // n° de l'entrée dont le formulaire de commentaire est ouvert
    drafts: new Map(), // n° → texte en cours de saisie
    sending: false
  };

  const JARVIS_TYPES = new Set(['parole', 'tache_fin', 'intention_locale', 'clarification_demandee']);
  const SYSTEM_TYPES = new Set(['validation_demandee', 'validation_accordee', 'validation_refusee', 'tache_annulee']);

  function time(iso) {
    return new Date(iso).toLocaleTimeString('fr-FR', { hour: '2-digit', minute: '2-digit', second: '2-digit' });
  }

  function day(iso) {
    return new Date(iso).toLocaleDateString('fr-FR', { weekday: 'long', day: 'numeric', month: 'long', year: 'numeric' });
  }

  async function load(reset) {
    if (state.loading) return;
    state.loading = true;
    const before = !reset && state.entries.length ? state.entries[state.entries.length - 1].seq : null;
    const res = await api('journalQuery', {
      view: state.view,
      text: state.text,
      types: state.view === 'actions' ? (state.onlyFails ? ['demande'] : (state.type ? [state.type] : null)) : null,
      before,
      limit: 120
    });
    state.loading = false;
    state.loaded = true;
    if (res && res.ok) {
      state.entries = reset ? res.entries : state.entries.concat(res.entries);
      state.more = res.more;
    }
    render(true);
  }

  async function loadStats() {
    const res = await api('journalStats');
    if (res && res.ok) state.stats = res.types;
  }

  // Nouvelle entrée au journal pendant la consultation : la première page est rechargée.
  let liveTimer = null;
  A.onJournal = () => {
    if (!document.getElementById('section-journal').classList.contains('active')) return;
    if (state.text || !state.loaded) return;
    if (state.commenting !== null) return; // commentaire en cours de saisie : pas de rechargement
    clearTimeout(liveTimer);
    liveTimer = setTimeout(() => load(true), 400);
  };

  // Issue d'une demande : vert = exécutée, rouge = non exécutée, jaune = on ne sait pas.
  const OUTCOME = { ok: ['o-ok', '● exécutée'], fail: ['o-fail', '● non exécutée'], unknown: ['o-unknown', '● inconnue'] };

  async function learn(e) {
    const ok = await confirmDialog('TROUVER UNE SOLUTION',
      `Une fenêtre cmd va s’ouvrir dans le dossier de Jarvis avec Claude Code en mode sans restriction (--dangerously-skip-permissions : il agit sans demander). Il étudiera pourquoi « ${e.summary} » n’a pas été exécutée et modifiera le code pour que ce type de demande fonctionne à l’avenir via Jarvis, sans IA si possible.`,
      'Lancer Claude Code');
    if (!ok) return;
    const res = await api('journalLearn', e.seq);
    if (res && res.ok) {
      toast('✓ Claude Code lancé dans une fenêtre cmd');
      e.learning = true;
      render(true);
    }
  }

  // Pastille d'issue (+ motif, + bouton pour une demande non exécutée) ; null hors demande.
  function outcomeBlock(e) {
    if (!e.outcome) return null;
    const [cls, label] = OUTCOME[e.outcome.state] || OUTCOME.unknown;
    return h('div', { class: 'outcome' },
      h('span', { class: `badge ${cls}`, title: e.outcome.reason }, label),
      h('span', { class: 'help' }, ` ${e.outcome.reason}`),
      e.outcome.state === 'fail'
        ? h('button', {
          class: `btn ${e.learning ? 'ghost' : 'primary'} small`,
          title: 'Claude Code (fenêtre cmd, mode sans restriction) cherche une alternative pour ce type de demande',
          onclick: (ev) => { ev.stopPropagation(); learn(e); }
        }, e.learning ? '🔧 Relancer la recherche' : '🔧 Trouver une solution')
        : null);
  }

  // Commentaire sur une entrée (même notée « exécutée ») : envoyé tel quel à Claude Code, avec la
  // demande d'origine et la suite de l'action dans le journal (voir admin.js › journalComment).
  function openComment(e) {
    state.commenting = e.seq;
    render(true);
    setTimeout(() => {
      const box = document.getElementById(`comment-${e.seq}`);
      if (box) box.focus();
    }, 0);
  }

  async function sendComment(e) {
    const text = (state.drafts.get(e.seq) || '').trim();
    if (!text || state.sending) return;
    state.sending = true;
    render(true);
    const res = await api('journalComment', e.seq, text);
    state.sending = false;
    if (res && res.ok) {
      toast('✓ commentaire envoyé à Claude Code (fenêtre cmd)');
      e.comments = (e.comments || []).concat({ at: new Date().toISOString(), text });
      state.drafts.delete(e.seq);
      state.commenting = null;
    } else {
      toast(`✕ ${(res && res.error) || 'envoi impossible'}`, true);
    }
    render(true);
  }

  function commentBlock(e) {
    const past = (e.comments || []).map((c) => h('div', { class: 'comment-sent' },
      h('span', { class: 'help' }, `💬 envoyé à Claude Code · ${day(c.at).split(' ').slice(1, 3).join(' ')} ${time(c.at)}`),
      h('div', {}, c.text)));
    const stop = (ev) => ev.stopPropagation();
    if (state.commenting !== e.seq) {
      return h('div', { class: 'comment-zone', onclick: stop }, ...past,
        h('button', {
          class: 'btn ghost small',
          title: 'Signaler à Claude Code que cette action n’a pas fait ce que tu attendais',
          onclick: () => openComment(e)
        }, past.length ? '💬 Ajouter un commentaire' : '💬 Commenter'));
    }
    const draft = state.drafts.get(e.seq) || '';
    return h('div', { class: 'comment-zone open', onclick: stop }, ...past,
      h('textarea', {
        id: `comment-${e.seq}`,
        class: 'comment-input',
        rows: 3,
        placeholder: 'Ce qui ne va pas, et ce que tu attendais. Ex. : j’ai demandé ma playlist sur Spotify, Jarvis l’a lancée sur JARVIS AUDIO.',
        value: draft,
        oninput: (ev) => state.drafts.set(e.seq, ev.target.value),
        onkeydown: (ev) => {
          if (ev.key === 'Enter' && (ev.ctrlKey || ev.metaKey)) sendComment(e);
          if (ev.key === 'Escape') { state.commenting = null; render(true); }
        }
      }),
      h('div', { class: 'row' },
        h('button', { class: 'btn primary small', disabled: state.sending, onclick: () => sendComment(e) },
          state.sending ? 'Envoi…' : '➤ Envoyer à Claude Code'),
        h('button', { class: 'btn ghost small', onclick: () => { state.commenting = null; render(true); } }, 'Annuler'),
        h('span', { class: 'help' }, 'Ctrl+Entrée · s’ouvre dans une fenêtre cmd, Claude Code en mode sans restriction')));
  }

  function dialogueText(e) {
    if (e.type === 'intention_locale') return (e.data && e.data.reponse) || e.summary;
    if (e.type === 'tache_fin' && e.data && e.data.statut !== 'terminee') return `(la tâche a échoué${e.data.erreur ? ` : ${e.data.erreur}` : ''})`;
    return e.summary;
  }

  function renderDialogues() {
    if (!state.entries.length) return h('div', { class: 'empty' }, state.loaded ? 'Aucun échange.' : 'Chargement…');
    const out = [];
    let lastDay = null;
    for (const e of state.entries) {
      const d = day(e.at);
      if (d !== lastDay) {
        out.push(h('div', { class: 'day-sep' }, d));
        lastDay = d;
      }
      if (e.type === 'demande') {
        const src = e.data && e.data.source;
        const source = src === 'clavier' ? ' · tapé' : src === 'voix' ? ' · dit' : '';
        out.push(h('div', { class: `bubble user${e.outcome ? ` outcome-${e.outcome.state}` : ''}` },
          h('div', { class: 'meta' }, `Toi${source} · ${time(e.at)}`), h('div', {}, e.summary), outcomeBlock(e), commentBlock(e)));
      } else if (JARVIS_TYPES.has(e.type)) {
        out.push(h('div', { class: `bubble jarvis${e.type === 'clarification_demandee' ? ' ask' : ''}` },
          h('div', { class: 'meta' }, `Jarvis${e.type === 'parole' ? '' : ` · ${e.label.toLowerCase()}`} · ${time(e.at)}`), h('div', {}, dialogueText(e))));
      } else if (SYSTEM_TYPES.has(e.type)) {
        out.push(h('div', { class: 'sys-line' }, `${time(e.at)} · ${e.label} — ${e.summary}`));
      }
    }
    return h('div', { class: 'chat' }, ...out);
  }

  function badgeClass(type) {
    if (/refus|erreur|annul|expir|repli|acces_refuse|echec/.test(type)) return 'err';
    if (/validation|securite|zone|autorisation|purge|reinitialisation|oubli/.test(type)) return 'warn';
    return 'ok';
  }

  function renderActions() {
    const entries = state.onlyFails ? state.entries.filter((e) => e.outcome && e.outcome.state === 'fail') : state.entries;
    if (!entries.length) {
      return h('div', { class: 'empty' }, !state.loaded ? 'Chargement…' : state.onlyFails ? 'Aucune demande non exécutée dans ces entrées. 👍' : 'Aucune entrée.');
    }
    return h('table', { class: 'journal-table' },
      h('thead', {}, h('tr', {}, h('th', {}, '#'), h('th', {}, 'Heure'), h('th', {}, 'Type'), h('th', {}, 'Détail'))),
      h('tbody', {}, ...entries.flatMap((e) => {
        const open = state.open.has(e.seq);
        const row = h('tr', { class: `clickable${e.outcome ? ` outcome-${e.outcome.state}` : ''}`, onclick: () => { if (open) state.open.delete(e.seq); else state.open.add(e.seq); render(true); } },
          h('td', { class: 'help' }, String(e.seq)),
          h('td', { style: 'white-space:nowrap' }, `${new Date(e.at).toLocaleDateString('fr-FR', { day: '2-digit', month: '2-digit' })} ${time(e.at)}`),
          h('td', {}, h('span', { class: `badge ${badgeClass(e.type)}` }, e.label)),
          h('td', { class: 'summary' }, e.summary, outcomeBlock(e), commentBlock(e)));
        if (!open) return [row];
        return [row, h('tr', {}, h('td', { colspan: 4 },
          h('div', { class: 'mono-block' }, JSON.stringify(e.data, null, 2)),
          h('div', { class: 'help' }, `empreinte ${e.hash}…`)))];
      })));
  }

  sections.journal = () => {
    if (!state.loaded && !state.loading) {
      load(true);
      loadStats().then(() => render(true));
    }
    const integ = state.integrity || (A.getSnapshot() || {}).journal || {};
    const search = h('input', {
      type: 'search', placeholder: 'Rechercher dans le journal…', value: state.text,
      onkeydown: (e) => { if (e.key === 'Enter') { state.text = e.target.value; load(true); } },
      onsearch: (e) => { state.text = e.target.value; load(true); }
    });
    const typeSelect = state.view === 'actions' && !state.onlyFails ? h('select', {
      onchange: (e) => { state.type = e.target.value; load(true); }
    }, h('option', { value: '' }, 'Tous les types'),
      ...(state.stats || []).map((t) => h('option', { value: t.type, selected: state.type === t.type }, `${t.label} (${t.count})`))) : null;
    const failsToggle = state.view === 'actions' ? h('label', { class: 'switch' },
      h('input', { type: 'checkbox', checked: state.onlyFails, onchange: (e) => { state.onlyFails = e.target.checked; load(true); } }),
      h('span', { class: 'track' }), h('span', { class: 'label' }, 'Seulement les demandes non exécutées')) : null;
    const shown = state.entries.filter((e) => e.outcome);
    const tally = (s) => shown.filter((e) => e.outcome.state === s).length;
    const legend = shown.length ? h('div', { class: 'row outcome-legend' },
      h('span', { class: 'badge o-ok' }, `● exécutées ${tally('ok')}`),
      h('span', { class: 'badge o-fail' }, `● non exécutées ${tally('fail')}`),
      h('span', { class: 'badge o-unknown' }, `● inconnues ${tally('unknown')}`),
      h('span', { class: 'help' }, 'sur les demandes affichées')) : null;
    return [
      h('h1', {}, 'Journal & dialogues'),
      h('p', { class: 'lead' }, 'Tout ce que Jarvis fait et dit, inscrit dans le journal scellé : chaque entrée est chaînée à la précédente par une empreinte SHA-256, et le fichier n’accepte que des ajouts. Rien ici ne peut être modifié ni effacé.'),
      panel('Intégrité', null,
        h('div', { class: 'row' },
          h('span', { class: `badge ${integ.ok === false ? 'err' : 'ok'}` }, integ.ok === false ? `ALTÉRÉ — ${integ.reason || ''}` : 'Intact'),
          h('span', {}, `${integ.count || 0} entrées scellées`),
          h('div', { class: 'spacer' }),
          h('button', {
            class: 'btn', onclick: async () => {
              const res = await api('journalVerify');
              if (res && res.ok) {
                state.integrity = res.integrity;
                A.toast(res.integrity.ok ? `✓ chaîne recalculée : ${res.integrity.count} entrées intactes` : '✕ altération détectée', !res.integrity.ok);
                render(true);
              }
            }
          }, '⛨ Vérifier toute la chaîne'))),
      panel(state.view === 'dialogues' ? 'Conversation' : 'Toutes les actions', `${state.entries.length}${state.more ? '+' : ''} entrées affichées`,
        h('div', { class: 'row', style: 'margin-bottom:14px' },
          h('div', { class: 'seg' },
            h('button', { class: state.view === 'dialogues' ? 'on' : '', onclick: () => { state.view = 'dialogues'; localStorage.setItem('admin.journal.view', 'dialogues'); load(true); } }, 'Dialogues'),
            h('button', { class: state.view === 'actions' ? 'on' : '', onclick: () => { state.view = 'actions'; localStorage.setItem('admin.journal.view', 'actions'); load(true); } }, 'Actions')),
          search, typeSelect, failsToggle),
        legend,
        state.view === 'dialogues' ? renderDialogues() : renderActions(),
        state.more ? h('div', { class: 'row', style: 'justify-content:center;margin-top:14px' },
          h('button', { class: 'btn', disabled: state.loading, onclick: () => load(false) }, state.loading ? 'Chargement…' : 'Plus ancien…')) : null)
    ];
  };
})();
