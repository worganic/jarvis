// Listes à fiches (vinyles, Lego, livres…) dans le HUD : la liste en mini-cards, la fiche complète, la création
// guidée et la vue « Mes listes ». Le noyau envoie des cards « tool » avec `ev.collection` = view | item | new | all
// (orchestrator/collections/cards.rs) ; module Listes : widgets.js les lui confie (registerTool('collection')).
// Tout ce qui modifie une liste passe par bridge.collections(op, args) : le même service que la voix et la page
// d'aide. La fenêtre n'envoie que des identifiants et des valeurs de champs, jamais d'adresse ni de chemin.
(function () {
  const J = (window.JARVIS = window.JARVIS || {});
  const bridge = window.jarvisBridge;

  const PREF_KEY = (window.JARVIS_POS_PREFIX || 'jarvis.') + 'collections';
  const keyKinds = new Map(); // clé de card -> view | item | new | all (position par défaut)
  const states = new Map(); // id de liste -> { q, adding, editing, scroll }
  let prefs = {};
  try { prefs = JSON.parse(localStorage.getItem(PREF_KEY) || '{}') || {}; } catch { prefs = {}; }

  const fold = (s) => String(s == null ? '' : s).normalize('NFD').replace(/[̀-ͯ]/g, '').toLowerCase();
  const pref = (id) => Object.assign({ mode: 'rail', filter: 'all' }, prefs[id] || {});
  function setPref(id, patch) {
    prefs[id] = Object.assign(pref(id), patch);
    try { localStorage.setItem(PREF_KEY, JSON.stringify(prefs)); } catch { /* préférence seulement */ }
  }
  const stateOf = (id) => { if (!states.has(id)) states.set(id, { q: '', adding: false, addingText: false, editing: false, selecting: false, bulkEdit: false, showStats: false, stats: null, sel: new Set(), scroll: 0 }); return states.get(id); };

  async function call(op, args) {
    if (!bridge || !bridge.collections) return { ok: false, erreur: 'Le noyau est injoignable.' };
    try { return (await bridge.collections(op, args || {})) || { ok: false, erreur: 'Pas de réponse.' }; } catch (e) { return { ok: false, erreur: String((e && e.message) || e) }; }
  }

  const kit = () => J.widgetKit;
  const h = (...a) => kit().h(...a);

  function btn(label, tip, onclick, cls) {
    return h('button', { class: `w-note-btn${cls ? ` ${cls}` : ''}`, title: tip || null, onclick }, label);
  }

  // Ligne de message (erreur ou confirmation) sous un formulaire.
  function messageLine() {
    const el = h('div', { class: 'col-msg' });
    el.show = (text, bad) => { el.textContent = text || ''; el.classList.toggle('bad', Boolean(bad)); };
    return el;
  }

  // --- Champs de saisie, générés d'après le schéma -----------------------------------------------------------
  function fieldInput(f, value) {
    const v = value === undefined || value === null ? '' : value;
    let el;
    let read = () => el.value.trim();
    switch (f.type) {
      case 'texte_long': el = h('textarea', { class: 'col-in', rows: 3, maxlength: 2000 }); el.value = String(v); read = () => el.value; break;
      case 'date': el = h('input', { class: 'col-in', type: 'date' }); el.value = String(v); break;
      case 'prix': el = h('input', { class: 'col-in', type: 'text', inputmode: 'decimal', placeholder: '0,00 €' }); el.value = v === '' ? '' : String(v).replace('.', ','); break;
      case 'nombre': el = h('input', { class: 'col-in', type: 'text', inputmode: 'decimal' }); el.value = String(v); break;
      case 'choix':
        el = h('select', { class: 'col-in' }, h('option', { value: '' }, '—'), ...(f.choix || []).map((c) => h('option', { value: c }, c)));
        el.value = String(v);
        break;
      case 'oui_non':
        el = h('select', { class: 'col-in' }, h('option', { value: '' }, '—'), h('option', { value: 'oui' }, 'Oui'), h('option', { value: 'non' }, 'Non'));
        el.value = v === true ? 'oui' : v === false ? 'non' : '';
        break;
      case 'lien':
      case 'image': el = h('input', { class: 'col-in', type: 'url', placeholder: 'https://…' }); el.value = String(v); break;
      default: el = h('input', { class: 'col-in', type: 'text', maxlength: 200 }); el.value = String(v);
    }
    el.setAttribute('aria-label', f.libelle || f.cle);
    return { el, get: read };
  }

  // Formulaire d'un élément (ajout ou modification) : un champ par champ de la liste.
  function itemForm(liste, element, done, cancel) {
    const msg = messageLine();
    const inputs = [];
    const grid = h('div', { class: 'col-form-grid' });
    for (const f of liste.champs) {
      const inp = fieldInput(f, element ? (element.valeurs || {})[f.cle] : '');
      inputs.push([f, inp]);
      const required = f.obligatoire || f.cle === liste.titre;
      grid.append(h('label', { class: `col-form-row${f.type === 'texte_long' ? ' wide' : ''}`, title: required ? 'Champ obligatoire' : null }, h('span', {}, required ? `${f.libelle} *` : f.libelle), inp.el));
    }
    const form = h('form', { class: 'col-form' }, grid, msg);
    form.addEventListener('submit', async (e) => {
      e.preventDefault();
      const valeurs = {};
      for (const [f, inp] of inputs) {
        const raw = inp.get();
        if (element || raw !== '') valeurs[f.cle] = raw;
      }
      msg.show('Enregistrement…');
      const r = element
        ? await call('modifier_element', { liste: liste.id, element: element.id, valeurs })
        : await call('ajouter_element', { liste: liste.id, valeurs });
      if (r.ok) { msg.show(''); done(r); return; }
      if (r.doublon) {
        msg.show(`${r.erreur} Ajouter quand même ?`, true);
        const again = btn('Ajouter quand même', '', async () => {
          const r2 = await call('ajouter_element', { liste: liste.id, valeurs, forcer: true });
          if (r2.ok) done(r2); else msg.show(r2.erreur || 'Échec.', true);
        });
        form.append(again);
        return;
      }
      msg.show(r.erreur || 'Échec.', true);
    });
    form.append(h('div', { class: 'w-note-actions' }, h('button', { class: 'w-note-btn primary', type: 'submit' }, element ? 'Enregistrer' : 'Ajouter'), btn('Annuler', '', cancel)));
    return form;
  }

  // Formulaire d'ajout en texte libre : le texte est envoyé à l'IA pour mise en forme selon la liste (ex. liste de numéros de sets Lego).
  function bulkForm(liste, done, cancel) {
    const msg = messageLine();
    const voc = vocab(liste);
    const hasId = (liste.identifiant || []).length > 0;
    const placeholder = hasId
      ? `Colle ou écris ta liste de ${liste.noms.pluriel} (ex. numéros de sets séparés par des espaces, virgules ou retours à la ligne : 75192, 75436, 10307…)`
      : `Colle ou écris plusieurs ${liste.noms.pluriel} (un par ligne ou séparés par des virgules)…`;
    const ta = h('textarea', { class: 'col-in', rows: 4, maxlength: 20000, placeholder, 'aria-label': `Texte contenant les ${liste.noms.pluriel} à ajouter` });
    const statutSel = liste.etat
      ? h('select', { class: 'col-in', 'aria-label': 'Statut par défaut' },
          h('option', { value: '' }, 'Statut automatique'),
          h('option', { value: 'possede' }, voc.fait),
          h('option', { value: 'souhaite' }, voc.voulu))
      : null;
    const submitBtn = h('button', { class: 'w-note-btn primary', type: 'submit' }, 'Ajouter tout');
    const grid = h('div', { class: 'col-form-grid' },
      h('label', { class: 'col-form-row wide' }, h('span', {}, `Texte à mettre en forme par l'IA (${liste.noms.pluriel})`), ta),
      statutSel ? h('label', { class: 'col-form-row' }, h('span', {}, 'Statut par défaut'), statutSel) : null);
    const form = h('form', { class: 'col-form' }, grid, msg,
      h('div', { class: 'w-note-actions' }, submitBtn, btn('Annuler', '', cancel)));
    form.addEventListener('submit', async (e) => {
      e.preventDefault();
      const texte = ta.value.trim();
      if (!texte) { msg.show('Écris ou colle au moins un élément.', true); return; }
      submitBtn.disabled = true;
      msg.show('Mise en forme par l\'IA et enregistrement…');
      const r = await call('ajouter_texte', { liste: liste.id, texte, statut: statutSel ? statutSel.value : '' });
      submitBtn.disabled = false;
      if (r.ok) {
        if (r.ajoutes === 0) { msg.show(r.resume || 'Déjà dans la liste.'); return; }
        msg.show('');
        done(r);
        return;
      }
      msg.show(r.erreur || 'Échec.', true);
    });
    return form;
  }

  // --- La liste : mini-cards ----------------------------------------------------------------------------
  function cover(item, liste, cls) {
    const box = h('div', { class: `col-cover ${cls || ''}` });
    const fallback = () => { box.classList.add('col-noimg'); box.replaceChildren(h('span', {}, liste.icone || '📋')); };
    if (typeof item.image === 'string' && (/^https:\/\//i.test(item.image) || /^http:\/\/127\.0\.0\.1:\d+\/api\/listes\/image\//.test(item.image))) {
      const img = h('img', { src: item.image, alt: '', loading: 'lazy', referrerpolicy: 'no-referrer', draggable: 'false' });
      img.addEventListener('error', fallback);
      box.append(img);
    } else fallback();
    return box;
  }

  // Mots du statut de la liste : « Possédé / Souhaité » par défaut, « Vu / À voir » pour un film ou un concert.
  const vocab = (liste) => Object.assign({ fait: 'Possédé', voulu: 'Souhaité', faitPluriel: 'Possédés', vouluPluriel: 'Souhaités' }, (liste && liste.vocabulaire) || {});

  function badge(item, liste) {
    if (item.brouillon) return h('span', { class: 'col-badge draft', title: 'Fiche à compléter' }, '…');
    if (item.statut === 'possede') return h('span', { class: 'col-badge own', title: vocab(liste).fait }, '✓');
    if (item.statut === 'souhaite') return h('span', { class: 'col-badge wish', title: `${vocab(liste).voulu} (pas encore fait)` }, '♡');
    return null;
  }

  function openItem(liste, item) { call('ouvrir', { liste: liste.id, element: item.id }); }

  // En mode sélection, un clic coche la fiche au lieu de l'ouvrir.
  function pick(liste, item, st, repaint) {
    return () => {
      if (!st || !st.selecting) { openItem(liste, item); return; }
      if (st.sel.has(item.id)) st.sel.delete(item.id); else st.sel.add(item.id);
      repaint();
    };
  }
  const picked = (st, item) => (st && st.selecting && st.sel.has(item.id) ? ' sel' : '');

  function miniCard(liste, item, st, repaint) {
    return h('button', { class: `col-mini ${item.statut || ''}${item.brouillon ? ' draft' : ''}${picked(st, item)}`, title: `${item.titre}${item.sousTitre ? ` — ${item.sousTitre}` : ''}\n${st && st.selecting ? 'Clic : cocher / décocher' : 'Clic : ouvrir la fiche'}`, onclick: pick(liste, item, st, repaint) },
      cover(item, liste), badge(item, liste),
      h('span', { class: 'col-mini-title' }, item.titre),
      h('span', { class: 'col-mini-sub' }, item.sousTitre || ' '),
      item.vignette ? h('span', { class: 'col-mini-tag' }, item.vignette) : null);
  }

  function listRow(liste, item, st, repaint) {
    return h('button', { class: `col-row ${item.statut || ''}${picked(st, item)}`, onclick: pick(liste, item, st, repaint), title: st && st.selecting ? 'Clic : cocher / décocher' : 'Clic : ouvrir la fiche' },
      cover(item, liste, 'small'),
      h('span', { class: 'col-row-text' }, h('span', { class: 'col-row-title' }, item.titre), h('span', { class: 'col-row-sub' }, item.sousTitre || '')),
      item.vignette ? h('span', { class: 'col-mini-tag' }, item.vignette) : null, badge(item, liste));
  }

  // Premier champ de type date de la liste (date du concert, d'achat…) : sert aux tris par date.
  const dateField = (liste) => (liste.champs || []).find((f) => f.type === 'date');
  // Ordres d'affichage proposés (le noyau accepte les mêmes : collections/schema.rs › SORTS).
  function sortChoices(liste) {
    const out = [['recent', 'Ajoutés récemment'], ['az', 'A → Z'], ['statut', `${vocab(liste).faitPluriel} d’abord`]];
    const d = dateField(liste);
    if (d) out.push(['date_desc', `${d.libelle} : plus récent d’abord`], ['date_asc', `${d.libelle} : plus ancien d’abord`]);
    return out;
  }

  function visibleItems(liste, p, st) {
    const q = fold(st.q).trim();
    let items = liste.elements.filter((it) => {
      if (p.filter === 'possede' && it.statut !== 'possede') return false;
      if (p.filter === 'souhaite' && it.statut !== 'souhaite') return false;
      if (!q) return true;
      return fold([it.titre, it.sousTitre, ...Object.values(it.affichage || {})].join(' ')).includes(q);
    });
    if (p.sort === 'az') items = items.slice().sort((a, b) => fold(a.titre).localeCompare(fold(b.titre)));
    else if (p.sort === 'statut') items = items.slice().sort((a, b) => (a.statut === b.statut ? 0 : a.statut === 'possede' ? -1 : 1));
    else if ((p.sort === 'date_desc' || p.sort === 'date_asc') && dateField(liste)) {
      // Dates AAAA-MM-JJ : l'ordre alphabétique est l'ordre chronologique ; une fiche sans date va à la fin.
      const k = dateField(liste).cle;
      const dir = p.sort === 'date_desc' ? -1 : 1;
      const dateOf = (it) => String((it.valeurs || {})[k] || '');
      items = items.slice().sort((a, b) => { const x = dateOf(a); const y = dateOf(b); if (!x || !y) return x ? -1 : y ? 1 : 0; return dir * x.localeCompare(y); });
    }
    else items = items.slice().sort((a, b) => String(b.creeLe).localeCompare(String(a.creeLe)));
    return items;
  }

  // --- Modification de la liste elle-même (champs, nom, suppression) ----------------------------------------
  function schemaEditor(liste, close) {
    const msg = messageLine();
    const run = async (op, args, ok) => {
      msg.show('…');
      const r = await call(op, Object.assign({ liste: liste.id }, args));
      if (r.ok) { msg.show(ok || 'Enregistré.'); } else msg.show(r.erreur || 'Échec.', true);
      return r;
    };
    const name = h('input', { class: 'col-in', type: 'text', maxlength: 60 });
    name.value = liste.nom;
    const rows = liste.champs.map((f) => {
      const label = h('input', { class: 'col-in', type: 'text', maxlength: 40 });
      label.value = f.libelle;
      label.addEventListener('change', () => { if (label.value.trim() && label.value.trim() !== f.libelle) run('modifier_schema', { renommer: { [f.cle]: label.value.trim() } }, 'Champ renommé.'); });
      const del = f.cle === liste.titre
        ? h('span', { class: 'col-tag', title: 'Ce champ sert de nom : il ne peut pas être retiré' }, 'nom')
        : btn('✕', 'Retirer ce champ (ses valeurs disparaissent ; « annule » les remet)', () => { if (confirmTwice(del)) run('modifier_schema', { retirer: [f.cle] }, 'Champ retiré.'); }, 'danger');
      // Obligatoire : une fiche ne s'enregistre pas sans ce champ (le nom l'est toujours).
      const req = h('input', { type: 'checkbox', checked: Boolean(f.obligatoire) || f.cle === liste.titre });
      req.disabled = f.cle === liste.titre;
      req.addEventListener('change', () => run('modifier_schema', { obligatoire: { [f.cle]: req.checked } }, req.checked ? 'Champ obligatoire.' : 'Champ facultatif.'));
      const reqBox = h('label', { class: 'col-tag', title: 'Obligatoire : une fiche ne peut pas être enregistrée sans ce champ' }, req, ' obligatoire');
      return h('div', { class: 'col-schema-row' }, label, h('span', { class: 'col-tag' }, f.type.replace('_', ' ')), reqBox, del);
    });
    const newLabel = h('input', { class: 'col-in', type: 'text', maxlength: 40, placeholder: 'Nouveau champ (ex. Pays)' });
    const newType = h('select', { class: 'col-in' }, ...['texte', 'nombre', 'prix', 'date', 'texte_long', 'lien', 'image', 'oui_non'].map((t) => h('option', { value: t }, t.replace('_', ' '))));
    const addField = btn('+ Champ', 'Ajouter ce champ à la liste', () => { if (newLabel.value.trim()) run('modifier_schema', { ajouter: [{ libelle: newLabel.value.trim(), type: newType.value }] }, 'Champ ajouté.'); });
    const defSort = h('select', { class: 'col-in', 'aria-label': 'Ordre d’affichage par défaut' }, ...sortChoices(liste).map(([v, l]) => h('option', { value: v }, l)));
    defSort.value = liste.tri || 'recent';
    const saveSort = btn('Enregistrer', 'Ordre utilisé à l’ouverture de la liste', async () => {
      const r = await run('modifier_schema', { tri: defSort.value }, 'Ordre par défaut enregistré.');
      if (r.ok) { setPref(liste.id, { sort: '' }); }
    });
    const confirmName = h('input', { class: 'col-in', type: 'text', placeholder: `Retape « ${liste.nom} » pour supprimer la liste` });
    return h('div', { class: 'col-panel' },
      h('div', { class: 'col-panel-title' }, 'Modifier la liste'),
      h('label', { class: 'col-form-row' }, h('span', {}, 'Nom'), name),
      btn('Renommer', '', () => run('modifier_schema', { nom: name.value.trim() }, 'Liste renommée.')),
      h('div', { class: 'col-panel-title' }, 'Ordre d’affichage par défaut'),
      h('div', { class: 'col-schema-row' }, defSort, saveSort),
      h('div', { class: 'col-panel-title' }, 'Champs'),
      ...rows,
      h('div', { class: 'col-schema-row' }, newLabel, newType, addField),
      h('div', { class: 'col-panel-title' }, 'Zone de danger'),
      h('div', { class: 'col-schema-row' }, confirmName, btn('Supprimer la liste', 'Supprime la liste et toutes ses fiches (« annule » la rétablit)', () => run('supprimer_liste', { confirmer: confirmName.value.trim() }, 'Liste supprimée.'), 'danger')),
      msg,
      h('div', { class: 'w-note-actions' }, btn('Fermer', '', close)));
  }

  // Bouton à double clic : le premier arme, le second confirme (4 s).
  function confirmTwice(button) {
    if (button.dataset.armed === '1') { button.dataset.armed = ''; return true; }
    button.dataset.armed = '1';
    const old = button.textContent;
    button.textContent = 'Confirmer ?';
    setTimeout(() => { button.dataset.armed = ''; if (button.isConnected) button.textContent = old; }, 4000);
    return false;
  }

  // Statistiques de la liste : totaux, dépenses, répartitions par champ (artiste, année, lieu…).
  function statsPanel(liste, st, repaint) {
    const box = h('div', { class: 'col-stats' });
    const fill = (s) => {
      const money = (n) => `${Number(n).toFixed(2).replace('.', ',')} €`;
      const kpis = [[s.total, liste.noms.pluriel]];
      if (liste.etat) kpis.push([s.possedes, vocab(liste).faitPluriel.toLowerCase()], [s.souhaites, vocab(liste).vouluPluriel.toLowerCase()]);
      if (s.avecPrix) kpis.push([money(s.depense), `dépensés (${s.avecPrix} prix notés)`]);
      box.replaceChildren(
        h('div', { class: 'col-kpis' }, ...kpis.map(([v, l]) => h('div', { class: 'col-kpi' }, h('b', {}, String(v)), h('span', {}, l)))),
        ...(s.repartitions || []).map((g) => {
          const max = Math.max(...g.lignes.map((l) => l.nombre));
          return h('div', { class: 'col-dist' }, h('div', { class: 'col-dist-title' }, g.libelle),
            ...g.lignes.map((l) => h('div', { class: 'col-dist-row' }, h('span', { class: 'col-dist-name', title: l.valeur }, l.valeur),
              h('span', { class: 'col-dist-bar' }, h('i', { style: `width:${Math.round((l.nombre / max) * 100)}%` })), h('span', { class: 'col-dist-n' }, String(l.nombre)))));
        }));
    };
    if (st.stats) fill(st.stats);
    else {
      box.textContent = 'Calcul…';
      call('statistiques', { liste: liste.id }).then((r) => { if (r.ok) { st.stats = r.stats; fill(r.stats); } else box.textContent = r.erreur || 'Échec.'; });
    }
    return box;
  }

  function statsButton(liste, st, repaint) {
    return btn('📊 Stats', 'Statistiques de la liste : totaux, dépenses, répartitions', () => { st.showStats = !st.showStats; st.stats = null; repaint(); });
  }

  // Import d'un fichier CSV (export Excel, tableur) : colonnes retrouvées par le nom des champs.
  function importButton(liste) {
    const file = h('input', { type: 'file', accept: '.csv,.txt,text/csv', style: 'display:none' });
    const b = btn('⇪ CSV', "Importer un fichier CSV (Excel : Enregistrer sous → CSV). La 1re ligne donne les colonnes, qui doivent porter le nom des champs de la liste.", () => file.click());
    file.addEventListener('change', async () => {
      const f = file.files && file.files[0];
      file.value = '';
      if (!f) return;
      const old = b.textContent;
      b.textContent = '…';
      const r = await call('importer_csv', { liste: liste.id, csv: await f.text() });
      b.textContent = r.ok ? `✓ ${r.resume}` : '⚠ ' + (r.erreur || 'échec');
      setTimeout(() => { if (b.isConnected) b.textContent = old; }, 6000);
    });
    return h('span', {}, b, file);
  }

  function imagesButton(liste) {
    return btn('🖼 Images', `Chercher sur le web l'image de chaque ${liste.noms.singulier} qui n'en a pas (artiste, pochette…) : une recherche IA par fiche`, async (e) => {
      const b = e.target;
      const old = b.textContent;
      b.textContent = '…';
      const r = await call('enrichir_liste', { liste: liste.id });
      b.textContent = r.ok ? (r.total ? `✓ ${r.total} en cours` : '✓ À jour') : '⚠ ' + (r.erreur || 'échec');
      setTimeout(() => { if (b.isConnected) b.textContent = old; }, 4000);
    });
  }

  function csvButton(liste) {
    return btn('⧉ CSV', 'Copier la liste en CSV (presse-papiers)', async (e) => {
      const r = await call('exporter_csv', { liste: liste.id });
      if (r.ok && navigator.clipboard) { try { await navigator.clipboard.writeText(r.csv); e.target.textContent = '✓ Copié'; setTimeout(() => { e.target.textContent = '⧉ CSV'; }, 1500); } catch { /* presse-papiers refusé */ } }
    });
  }

  // Barre de sélection : suppression ou modification de plusieurs fiches à la fois.
  function selectionBar(liste, st, items, repaint) {
    const msg = messageLine();
    const n = st.sel.size;
    const all = btn('Tout', 'Cocher toutes les fiches affichées', () => { items.forEach((i) => st.sel.add(i.id)); repaint(); });
    const none = btn('Aucun', 'Tout décocher', () => { st.sel.clear(); repaint(); });
    const edit = btn('✎ Modifier', 'Donner la même valeur à un champ des fiches cochées', () => { st.bulkEdit = !st.bulkEdit; repaint(); });
    const del = btn('🗑 Supprimer', 'Retirer les fiches cochées (« annule » les remet toutes)', async () => {
      if (!confirmTwice(del)) return;
      const r = await call('supprimer_elements', { liste: liste.id, elements: [...st.sel] });
      if (r.ok) { st.sel.clear(); st.bulkEdit = false; } else msg.show(r.erreur || 'Échec.', true);
    }, 'danger');
    if (!n) { edit.disabled = true; del.disabled = true; }
    const bar = h('div', { class: 'col-selbar' },
      h('div', { class: 'col-bar' }, h('span', { class: 'col-countline' }, `${n} sélectionné${n > 1 ? 's' : ''}`), all, none, edit, del,
        btn('✕ Terminer', 'Quitter le mode sélection', () => { st.selecting = false; st.bulkEdit = false; st.sel.clear(); repaint(); })), msg);
    if (st.bulkEdit && n) {
      const fields = liste.champs.filter((f) => f.cle !== liste.titre && f.type !== 'image');
      const sel = h('select', { class: 'col-in', 'aria-label': 'Champ à modifier' }, ...fields.map((f) => h('option', { value: f.cle }, f.libelle)));
      const holder = h('span', { class: 'col-form-row' });
      let inp;
      const mount = () => { inp = fieldInput(fields.find((f) => f.cle === sel.value), ''); holder.replaceChildren(inp.el); };
      sel.addEventListener('change', mount);
      mount();
      const apply = h('button', { class: 'w-note-btn primary', type: 'button' }, `Appliquer à ${n} fiche${n > 1 ? 's' : ''}`);
      apply.addEventListener('click', async () => {
        apply.disabled = true;
        msg.show('Enregistrement…');
        const r = await call('modifier_elements', { liste: liste.id, elements: [...st.sel], valeurs: { [sel.value]: inp.get() } });
        apply.disabled = false;
        if (r.ok) { st.bulkEdit = false; msg.show(''); } else msg.show(r.erreur || 'Échec.', true);
      });
      bar.append(h('div', { class: 'col-form' }, h('div', { class: 'col-form-grid' }, h('label', { class: 'col-form-row' }, h('span', {}, 'Champ'), sel), h('label', { class: 'col-form-row' }, h('span', {}, 'Nouvelle valeur'), holder)),
        h('div', { class: 'w-note-actions' }, apply, btn('Annuler', '', () => { st.bulkEdit = false; repaint(); }))));
    }
    return bar;
  }

  function renderView(ev) {
    const liste = ev.liste;
    const key = ev.cardKey || `tool:${ev.title}`;
    keyKinds.set(key, 'view');
    if (ev.filtre) setPref(liste.id, { filter: ev.filtre }); // « affiche mes souhaits Lego »
    const st = stateOf(liste.id);
    const body = h('div', { class: 'col-body' });
    const countLine = h('span', { class: 'col-countline' });
    const search = h('input', { class: 'col-in col-search', type: 'search', placeholder: 'Chercher…', 'aria-label': 'Chercher dans la liste' });
    search.value = st.q;
    const chips = h('div', { class: 'col-chips' });
    const modes = h('div', { class: 'col-chips' });
    const sort = h('select', { class: 'col-in col-sort', title: 'Tri' }, ...sortChoices(liste).map(([v, l]) => h('option', { value: v }, l)));

    function paint() {
      const p = pref(liste.id);
      p.sort = p.sort || liste.tri || 'recent'; // choix local, sinon ordre par défaut de la liste
      sort.value = p.sort;
      const own = liste.elements.filter((i) => i.statut === 'possede').length;
      const wish = liste.elements.filter((i) => i.statut === 'souhaite').length;
      chips.replaceChildren(...[['all', `Tous ${liste.elements.length}`], ['possede', `${vocab(liste).faitPluriel} ${own}`], ['souhaite', `${vocab(liste).vouluPluriel} ${wish}`]].filter(([f]) => f === 'all' || liste.etat).map(([f, label]) =>
        h('button', { class: `col-chip${p.filter === f ? ' on' : ''}`, onclick: () => { setPref(liste.id, { filter: f }); paint(); } }, label)));
      modes.replaceChildren(...[['rail', '▭▭', 'Mini-cards en rang horizontal'], ['grid', '▦', 'Grille'], ['list', '☰', 'Liste']].map(([m, ico, tip]) =>
        h('button', { class: `col-chip${p.mode === m ? ' on' : ''}`, title: tip, onclick: () => { setPref(liste.id, { mode: m }); paint(); } }, ico)));
      const items = visibleItems(liste, p, st);
      countLine.textContent = items.length === liste.elements.length ? '' : `${items.length} sur ${liste.elements.length}`;
      st.sel = new Set([...st.sel].filter((id) => liste.elements.some((i) => i.id === id)));
      const parts = [];
      if (st.selecting) parts.push(selectionBar(liste, st, items, paint));
      if (st.showStats) parts.push(statsPanel(liste, st, paint));
      if (st.adding) parts.push(itemForm(liste, null, () => { st.adding = false; }, () => { st.adding = false; paint(); }));
      if (st.addingText) parts.push(bulkForm(liste, () => { st.addingText = false; }, () => { st.addingText = false; paint(); }));
      if (!liste.elements.length && !st.adding && !st.addingText) {
        parts.push(h('div', { class: 'w-empty' }, `Aucun ${liste.noms.singulier} pour l'instant. Dis par exemple : « ajoute à ma liste ${(liste.alias[0] || liste.nom)} … », clique sur + Ajouter ou sur + Ajouter en texte.`));
      } else if (!items.length) {
        parts.push(h('div', { class: 'w-empty' }, 'Aucun résultat.'));
      } else if (p.mode === 'list') {
        parts.push(h('div', { class: 'col-list' }, ...items.map((it) => listRow(liste, it, st, paint))));
      } else {
        const wrap = h('div', { class: p.mode === 'grid' ? 'col-grid' : 'col-rail' }, ...items.map((it) => miniCard(liste, it, st, paint)));
        if (p.mode === 'rail') {
          wrap.addEventListener('wheel', (e) => { if (wrap.scrollWidth > wrap.clientWidth && Math.abs(e.deltaY) > Math.abs(e.deltaX)) { wrap.scrollLeft += e.deltaY; e.preventDefault(); } }, { passive: false });
          wrap.addEventListener('scroll', () => { st.scroll = wrap.scrollLeft; });
          requestAnimationFrame(() => { wrap.scrollLeft = st.scroll || 0; });
        }
        parts.push(wrap);
      }
      if (st.editing) parts.unshift(schemaEditor(liste, () => { st.editing = false; paint(); }));
      body.replaceChildren(...parts);
    }

    search.addEventListener('input', () => { st.q = search.value; paint(); });
    sort.addEventListener('change', () => { setPref(liste.id, { sort: sort.value }); paint(); });
    const K = kit();
    const el = h('div', { class: 'widget w-tool w-card-list w-col' },
      h('div', { class: 'w-head' },
        K.gripHandle(), K.numBadge(ev), h('span', { class: 'w-icon' }, liste.icone || '📋'),
        h('span', { class: 'w-title', title: liste.nom }, liste.nom),
        h('span', { class: 'w-count', title: `${liste.total} ${liste.total > 1 ? liste.noms.pluriel : liste.noms.singulier}` }, String(liste.total)),
        K.closeButton(ev, key)),
      liste.origine && liste.origine.nom ? h('div', { class: 'col-origin' }, `${liste.origine.type === 'invite' ? '👤 Liste invité' : 'Créée'} par ${liste.origine.nom}${liste.origine.relation ? ` (${liste.origine.relation})` : ''}`) : null,
      h('div', { class: 'col-bar' }, search, chips, sort, modes),
      h('div', { class: 'col-bar' },
        btn('+ Ajouter', `Ajouter un ${liste.noms.singulier} avec un formulaire (ou dis « ajoute à ma liste … »)`, () => { st.adding = !st.adding; if (st.adding) st.addingText = false; paint(); }, 'primary'),
        btn('+ Ajouter en texte', `Coller ou écrire une liste de ${liste.noms.pluriel} au format texte (ex. numéros de sets) mis en forme par l'IA`, () => { st.addingText = !st.addingText; if (st.addingText) st.adding = false; paint(); }),
        btn('☑ Sélection', 'Cocher plusieurs fiches pour les modifier ou les supprimer ensemble', () => { st.selecting = !st.selecting; if (!st.selecting) { st.sel.clear(); st.bulkEdit = false; } paint(); }),
        btn('✎ Liste', 'Modifier les champs, renommer ou supprimer la liste', () => { st.editing = !st.editing; paint(); }),
        imagesButton(liste), statsButton(liste, st, paint), importButton(liste), csvButton(liste), countLine),
      body,
      h('div', { class: 'w-rows-foot' }, `« ajoute à ma liste ${liste.alias[0] || liste.nom} … » · « affiche ma liste ${liste.alias[0] || liste.nom} » · clic sur une mini-card : la fiche`));
    paint();
    K.place(key, el, 'tool', ev.ttl || 900000);
    if (J.cardFront && J.cardFront.front) J.cardFront.front(el);
  }

  // --- La fiche -------------------------------------------------------------------------------------------
  function renderItem(ev) {
    const liste = ev.liste;
    const it = ev.element;
    const key = ev.cardKey || `tool:${ev.title}`;
    keyKinds.set(key, 'item');
    const K = kit();
    const msg = messageLine();
    let editing = false;
    const holder = h('div', { class: 'col-item-main' });

    function statutButton() {
      if (!liste.etat) return null;
      const own = it.statut === 'possede';
      return h('button', {
        class: `col-status ${own ? 'own' : 'wish'}`,
        title: own ? `${vocab(liste).fait} — clic : repasser en « ${vocab(liste).voulu} »` : `${vocab(liste).voulu} — clic : passer en « ${vocab(liste).fait} » (la date du jour est notée)`,
        onclick: async () => { const r = await call('changer_statut', { liste: liste.id, element: it.id, statut: own ? 'souhaite' : 'possede' }); if (!r.ok) msg.show(r.erreur, true); }
      }, own ? `✓ ${vocab(liste).fait}` : `♡ ${vocab(liste).voulu} · passer en « ${vocab(liste).fait} »`);
    }

    function fieldValue(f) {
      const shown = (it.affichage || {})[f.cle];
      const cell = h('div', { class: 'col-field-val', title: 'Clic : modifier' }, shown || '—');
      if (!shown) cell.classList.add('empty');
      cell.addEventListener('click', () => {
        if (cell.querySelector('input,select,textarea')) return;
        const inp = fieldInput(f, (it.valeurs || {})[f.cle]);
        cell.replaceChildren(inp.el);
        inp.el.focus();
        let done = false;
        const save = async () => {
          if (done) return; done = true;
          const r = await call('modifier_element', { liste: liste.id, element: it.id, valeurs: { [f.cle]: inp.get() } });
          if (!r.ok) { msg.show(r.erreur || 'Échec.', true); cell.textContent = shown || '—'; }
        };
        inp.el.addEventListener('keydown', (e) => {
          if (e.key === 'Enter' && f.type !== 'texte_long') { e.preventDefault(); save(); }
          if (e.key === 'Escape') { done = true; cell.textContent = shown || '—'; }
        });
        inp.el.addEventListener('change', () => { if (f.type === 'choix' || f.type === 'oui_non' || f.type === 'date') save(); });
        inp.el.addEventListener('blur', () => setTimeout(save, 120));
      });
      return cell;
    }

    function paint() {
      if (editing) {
        holder.replaceChildren(itemForm(liste, it, () => { editing = false; }, () => { editing = false; paint(); }));
        return;
      }
      const sub = it.sousTitre ? h('div', { class: 'col-item-sub' }, it.sousTitre) : null;
      const fields = liste.champs.filter((f) => f.cle !== liste.titre && !(liste.sousTitre || []).includes(f.cle) && f.cle !== liste.etat && f.type !== 'image');
      const del = btn('🗑 Supprimer', 'Retirer cette fiche (« annule » la remet)', async () => {
        if (!confirmTwice(del)) return;
        const r = await call('supprimer_element', { liste: liste.id, element: it.id });
        if (!r.ok) msg.show(r.erreur || 'Échec.', true);
      }, 'danger');
      const provider = liste.enrichissement && liste.enrichissement.fournisseur !== 'aucun';
      const verify = provider && !it.brouillon && it.origine && typeof it.confiance === 'number' && it.confiance < 0.85;
      const doAction = async (a) => {
        msg.show(`${a.libelle}…`);
        const r = await call('action', { liste: liste.id, element: it.id, action: a.id });
        msg.show(r.ok ? '' : (r.say || r.erreur || 'Échec.'), !r.ok);
      };
      const special = (liste.actions || []).filter((a) => a.type !== 'enrichir').map((a) => btn(`${a.icone || '↗'} ${a.libelle}`, a.libelle, () => doAction(a)));
      // Prix relevés : un prix avec lien ouvre la page du produit chez le marchand (le noyau ouvre l'adresse de la fiche).
      const hist = Array.isArray(it.historiquePrix) ? it.historiquePrix : [];
      const openPrice = async (index) => {
        const r = await call('ouvrir_lien_prix', { liste: liste.id, element: it.id, index });
        if (!r.ok) msg.show(r.say || r.erreur || 'Échec.', true);
      };
      const history = hist.length
        ? h('div', { class: 'col-hist', title: 'Prix relevés' }, 'Prix relevés : ', ...hist.slice(-5).flatMap((p, k, shown) => {
          const index = hist.length - shown.length + k;
          const label = `${String(p.date).split('-').reverse().slice(0, 2).join('/')} ${p.prix} €${p.boutique ? ` (${p.boutique})` : ''}`;
          const part = p.url ? h('button', { class: 'col-hist-link', title: `Ouvrir la page du produit${p.boutique ? ` chez ${p.boutique}` : ''}`, onclick: () => openPrice(index) }, label + ' ↗') : label;
          return k ? [' → ', part] : [part];
        }))
        : null;
      const lastLink = [...hist.keys()].reverse().find((i) => hist[i].url);
      if (lastLink !== undefined) special.push(btn(`↗ Voir chez ${hist[lastLink].boutique || 'la boutique'}`, 'Page du produit chez le marchand du dernier prix relevé', () => openPrice(lastLink)));
      const fillable = provider || Boolean(liste.origine);
      holder.replaceChildren(...[
        h('div', { class: 'col-item-top' },
          cover(it, liste, 'big'),
          h('div', { class: 'col-item-info' }, h('div', { class: 'col-item-title' }, it.titre), sub, statutButton(), it.brouillon ? h('div', { class: 'col-tag' }, 'recherche des infos…') : null,
            it.origine && !it.brouillon ? h('div', { class: `col-tag${verify ? ' warn' : ''}`, title: verify ? 'Résultat peu sûr : vérifie, ou prends un autre résultat' : 'Infos retrouvées automatiquement' }, `${verify ? 'à vérifier · ' : ''}source : ${it.origine}`) : null)),
        h('div', { class: 'col-fields' }, ...fields.map((f) => h('div', { class: `col-field${f.type === 'texte_long' ? ' wide' : ''}` }, h('div', { class: 'col-field-label' }, f.libelle), fieldValue(f)))),
        history, msg,
        special.length || fillable ? h('div', { class: 'w-note-actions' }, ...special,
          fillable && !provider ? btn('✨ Compléter (IA)', "Demander à l'IA de remplir les champs vides de cette fiche (ce que tu as saisi n'est jamais écrasé)", async () => { msg.show('Recherche des informations…'); const r = await call('enrichir', { liste: liste.id, element: it.id }); msg.show(r.ok ? (r.say || '') : (r.erreur || 'Échec.'), !r.ok); }) : null,
          provider ? btn('↻ Réenrichir', "Chercher de nouveau les infos et l'image (ce que tu as saisi n'est jamais écrasé)", async () => { msg.show('Recherche…'); const r = await call('enrichir', { liste: liste.id, element: it.id, ecraser: true }); msg.show(r.ok ? (r.say || '') : (r.erreur || 'Échec.'), !r.ok); }) : null,
          provider && it.candidats > 1 ? btn('⇄ Autre résultat', 'Prendre le résultat suivant de la recherche', async () => { msg.show('Recherche…'); const r = await call('enrichir', { liste: liste.id, element: it.id, suivant: true, ecraser: true }); msg.show(r.ok ? '' : (r.erreur || 'Échec.'), !r.ok); }) : null) : null,
        h('div', { class: 'w-note-actions' },
          btn('◀', 'Fiche précédente', () => ev.precedent && call('ouvrir', { liste: liste.id, element: ev.precedent }), ev.precedent ? '' : 'off'),
          btn('▶', 'Fiche suivante', () => ev.suivant && call('ouvrir', { liste: liste.id, element: ev.suivant }), ev.suivant ? '' : 'off'),
          btn('▭ La liste', 'Afficher toute la liste', () => call('ouvrir', { liste: liste.id })),
          h('span', { class: 'w-note-spacer' }),
          btn('✎ Modifier', 'Modifier tous les champs', () => { editing = true; paint(); }), del)].filter(Boolean));
    }
    paint();
    const el = h('div', { class: 'widget w-tool w-card-list w-col w-col-item' },
      h('div', { class: 'w-head' },
        K.gripHandle(), K.numBadge(ev), h('span', { class: 'w-icon' }, liste.icone || '📋'),
        h('span', { class: 'w-title', title: it.titre }, it.titre),
        h('span', { class: 'w-count' }, `${ev.position}/${ev.total}`), K.closeButton(ev, key)),
      holder);
    K.place(key, el, 'tool', ev.ttl || 900000);
    if (J.cardFront && J.cardFront.front) J.cardFront.front(el);
  }

  // --- Création guidée -----------------------------------------------------------------------------------------
  function renderNew(ev) {
    const prop = ev.proposition;
    const key = ev.cardKey || `tool:${ev.title}`;
    keyKinds.set(key, 'new');
    const K = kit();
    const msg = messageLine();
    const name = h('input', { class: 'col-in', type: 'text', maxlength: 60 });
    name.value = prop.nom;
    const checks = [];
    const list = h('div', { class: 'col-fields-pick' });
    const addRow = (f, extra) => {
      const box = h('input', { type: 'checkbox' });
      box.checked = Boolean(f.retenu);
      box.disabled = Boolean(f.fixe);
      checks.push({ f, box, extra });
      list.append(h('label', { class: 'col-pick', title: f.fixe ? 'Ce champ sert de nom' : '' }, box, h('span', { class: 'col-pick-label' }, f.libelle), h('span', { class: 'col-tag' }, String(f.type).replace('_', ' '))));
    };
    prop.champs.forEach((f) => addRow(f, false));
    const newLabel = h('input', { class: 'col-in', type: 'text', maxlength: 40, placeholder: 'Ajouter un champ (ex. Pays)' });
    const newType = h('select', { class: 'col-in' }, ...['texte', 'nombre', 'prix', 'date', 'texte_long', 'lien', 'image'].map((t) => h('option', { value: t }, t.replace('_', ' '))));
    const addBtn = btn('+', 'Ajouter ce champ', () => {
      const label = newLabel.value.trim();
      if (!label) return;
      addRow({ cle: `ajout_${checks.length}`, libelle: label, type: newType.value, retenu: true, fixe: false }, true);
      newLabel.value = '';
    });
    const create = btn('Créer la liste', 'Crée la liste avec les champs cochés (ou dis « c\'est bon »)', async () => {
      msg.show('Création…');
      const retenus = checks.filter((c) => !c.extra && c.box.checked).map((c) => c.f.cle);
      const ajouts = checks.filter((c) => c.extra && c.box.checked).map((c) => ({ libelle: c.f.libelle, type: c.f.type }));
      const r = await call('creer_depuis_proposition', { modele: prop.modele, nom: name.value.trim(), retenus, ajouts });
      if (!r.ok) msg.show(r.erreur || 'Échec.', true);
    }, 'primary');
    const cancel = btn('Annuler', '', async () => { await call('annuler_creation', {}); K.close(key); });
    const el = h('div', { class: 'widget w-tool w-card-list w-col w-col-new' },
      h('div', { class: 'w-head' }, K.gripHandle(), K.numBadge(ev), h('span', { class: 'w-icon' }, prop.icone || '📋'), h('span', { class: 'w-title' }, 'Nouvelle liste'), K.closeButton(ev, key)),
      h('label', { class: 'col-form-row' }, h('span', {}, 'Nom de la liste'), name),
      h('div', { class: 'col-panel-title' }, 'Champs de chaque fiche — coche ce qui te suffit'),
      list,
      h('div', { class: 'col-schema-row' }, newLabel, newType, addBtn),
      msg,
      h('div', { class: 'w-note-actions' }, create, cancel),
      h('div', { class: 'w-rows-foot' }, 'Tu peux aussi le dire : « c\'est bon », « enlève le prix », « ajoute un champ pays », « appelle-la Mes disques ».'));
    K.place(key, el, 'tool', ev.ttl || 900000);
    if (J.cardFront && J.cardFront.front) J.cardFront.front(el);
  }

  // --- Mes listes -----------------------------------------------------------------------------------------------
  function renderAll(ev) {
    const key = ev.cardKey || `tool:${ev.title}`;
    keyKinds.set(key, 'all');
    const K = kit();
    const msg = messageLine();
    const fiches = ev.fiches || [];
    const cocher = ev.cocher || [];
    const when = (iso) => { const d = new Date(iso); return Number.isNaN(d.getTime()) ? '' : d.toLocaleDateString('fr-FR'); };
    // Liste demandée par un invité du site Nomade (origine posée à sa création) : badge et groupe à part.
    const byGuest = (l) => Boolean(l.origine && l.origine.type === 'invite');
    const tile = (l) => h('button', { class: `col-tile${byGuest(l) ? ' guest' : ''}`, title: byGuest(l) ? `Liste invité : demandée par ${l.origine.nom || l.origine.login}` : 'Clic : ouvrir la liste', onclick: () => call('ouvrir', { liste: l.id }) },
      h('span', { class: 'col-tile-ico' }, l.icone || '📋'),
      h('span', { class: 'col-tile-name' }, l.nom),
      byGuest(l) ? h('span', { class: 'col-tile-guest' }, `👤 ${l.origine.nom || l.origine.login} · invité`) : null,
      h('span', { class: 'col-tile-sub' }, `${l.total} ${l.total > 1 ? l.noms.pluriel : l.noms.singulier}${l.etat ? ` · ${l.possedes} ✓ · ${l.souhaites} ♡` : ''}`),
      h('span', { class: 'col-tile-sub dim' }, when(l.majLe)));
    const own = fiches.filter((l) => !byGuest(l));
    const guest = fiches.filter(byGuest);
    const tiles = own.map(tile);
    const guestTiles = guest.map(tile);
    const checks = cocher.map((l) => h('button', { class: 'col-tile check', title: 'Liste à cocher : clic pour l\'ouvrir', onclick: () => bridge.listAction && bridge.listAction('smartlist', l.id, 'show') },
      h('span', { class: 'col-tile-ico' }, '☑'),
      h('span', { class: 'col-tile-name' }, l.nom),
      h('span', { class: 'col-tile-sub' }, `${l.coches}/${l.total} cochés`),
      h('span', { class: 'col-tile-sub dim' }, when(l.majLe))));
    const subject = h('input', { class: 'col-in', type: 'text', placeholder: 'Nouvelle liste sur… (ex. livres, jeux vidéo)', maxlength: 80 });
    const go = async () => {
      if (!subject.value.trim()) return;
      msg.show('…');
      const r = await call('proposer', { sujet: subject.value.trim() });
      msg.show(r.ok ? '' : r.erreur || 'Échec.', !r.ok);
    };
    subject.addEventListener('keydown', (e) => { if (e.key === 'Enter') go(); });
    const el = h('div', { class: 'widget w-tool w-card-list w-col w-col-all' },
      h('div', { class: 'w-head' }, K.gripHandle(), K.numBadge(ev), h('span', { class: 'w-icon' }, '▤'), h('span', { class: 'w-title' }, 'Mes listes'), h('span', { class: 'w-count' }, String(fiches.length + cocher.length)), K.closeButton(ev, key)),
      own.length ? h('div', { class: 'col-panel-title' }, 'Listes à fiches') : null,
      own.length ? h('div', { class: 'col-tiles' }, ...tiles) : null,
      guest.length ? h('div', { class: 'col-panel-title' }, 'Listes des invités') : null,
      guest.length ? h('div', { class: 'col-tiles' }, ...guestTiles) : null,
      cocher.length ? h('div', { class: 'col-panel-title' }, 'Listes à cocher') : null,
      cocher.length ? h('div', { class: 'col-tiles' }, ...checks) : null,
      !fiches.length && !cocher.length ? h('div', { class: 'w-empty' }, 'Aucune liste. Dis par exemple : « crée-moi une liste de disques vinyle ».') : null,
      h('div', { class: 'col-panel-title' }, 'Nouvelle liste'),
      h('div', { class: 'col-chips' }, ...(ev.modeles || []).map((m) => h('button', { class: 'col-chip', onclick: async () => { const r = await call('proposer', { modele: m.id }); if (!r.ok) msg.show(r.erreur, true); } }, `${m.icone} ${m.nom}`))),
      h('div', { class: 'col-schema-row' }, subject, btn('Créer', 'L\'IA propose les champs, ou dis « crée-moi une liste de … »', go, 'primary')),
      msg,
      h('div', { class: 'w-rows-foot' }, '« affiche ma liste de disques » · « crée-moi une liste de livres » · « ajoute à ma liste vinyle … »'));
    K.place(key, el, 'tool', ev.ttl || 900000);
    if (J.cardFront && J.cardFront.front) J.cardFront.front(el);
  }

  function render(ev) {
    switch (ev.collection) {
      case 'view': return renderView(ev);
      case 'item': return renderItem(ev);
      case 'new': return renderNew(ev);
      case 'all': return renderAll(ev);
      default: return undefined;
    }
  }

  // Position par défaut : à droite, fiche à gauche de la liste (l'utilisateur peut tout déplacer).
  function pos(key, S, W) {
    const kind = keyKinds.get(key);
    if (!kind) return null;
    const colW = 620 * S;
    const x = Math.max(20, W - colW - 36 * S);
    if (kind === 'item') return { x: Math.max(20, x - 500 * S), y: 110 * S };
    if (kind === 'new' || kind === 'all') return { x: Math.max(20, x - 20 * S), y: 100 * S };
    return { x, y: 96 * S };
  }

  J.collectionsUI = { render, pos };
  // Module Listes : les cards « tool » marquées collection lui sont confiées (widgets.js › registerTool).
  if (J.widgetKit && J.widgetKit.registerTool) J.widgetKit.registerTool('collection', render);
})();
