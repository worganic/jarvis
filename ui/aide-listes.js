// Page d'aide › « RAG — Listes à fiches » : toutes les listes et leur contenu, avec des formulaires pour
// ajouter, modifier et supprimer. Même service que la voix et la card du HUD (orch/collections/service.rs),
// appelé par l'API HTTP /api/listes/<opération> avec le jeton de l'aide reçu dans l'adresse (#j=…) quand la
// page est ouverte depuis le bouton « ? AIDE » de Jarvis. Sans jeton : message, rien n'est lu ni modifié.
// Tout le contenu est inséré en texte (jamais en HTML) : un titre venu du web ne peut rien exécuter.
(() => {
  const TOKEN_KEY = 'jarvis.aide.jeton';
  let token = '';
  try {
    const m = /[#&]j=([0-9a-f]{40})/.exec(location.hash);
    if (m) { sessionStorage.setItem(TOKEN_KEY, m[1]); history.replaceState(null, '', location.pathname + location.search); }
    token = sessionStorage.getItem(TOKEN_KEY) || '';
  } catch { token = ''; }

  const fold = (s) => String(s == null ? '' : s).normalize('NFD').replace(/[̀-ͯ]/g, '').toLowerCase();

  function h(tag, attrs, ...kids) {
    const e = document.createElement(tag);
    for (const [k, v] of Object.entries(attrs || {})) {
      if (v === undefined || v === null || v === false) continue;
      if (k === 'class') e.className = v;
      else if (k.startsWith('on') && typeof v === 'function') e.addEventListener(k.slice(2), v);
      else e.setAttribute(k, v === true ? '' : v);
    }
    for (const c of kids.flat()) { if (c === null || c === undefined || c === false) continue; e.append(c instanceof Node ? c : document.createTextNode(String(c))); }
    return e;
  }

  async function api(op, args) {
    if (!token) return { ok: false, erreur: 'Pas de jeton : ouvre cette page depuis le bouton « ? AIDE » de Jarvis.' };
    try {
      const r = await fetch(`/api/listes/${op}`, { method: 'POST', headers: { 'Content-Type': 'application/json', 'X-Jarvis-Aide': token }, body: JSON.stringify(args || {}) });
      if (r.status === 401 || r.status === 403) return { ok: false, erreur: 'Accès refusé : le jeton n\'est plus valable (Jarvis a redémarré ?). Rouvre l\'aide depuis le HUD.' };
      return await r.json();
    } catch (e) { return { ok: false, erreur: 'Jarvis ne répond pas.' }; }
  }

  const STYLE = `
  #rag-listes .bar { display: flex; flex-wrap: wrap; gap: 8px; align-items: center; margin: 8px 0; }
  #rag-listes input, #rag-listes select, #rag-listes textarea { padding: 5px 8px; border: 1px solid var(--line); border-radius: 5px; background: var(--bg); color: var(--fg); font: inherit; font-size: 13.5px; min-width: 0; }
  #rag-listes textarea { width: 100%; }
  #rag-listes button { padding: 5px 11px; border: 1px solid var(--accent); border-radius: 5px; background: transparent; color: var(--accent); font: inherit; font-size: 13px; cursor: pointer; }
  #rag-listes button:hover { background: var(--panel2); }
  #rag-listes button.danger { border-color: var(--bad); color: var(--bad); }
  #rag-listes button.plain { border-color: var(--line); color: var(--dim); }
  #rag-listes details { margin: 10px 0; border: 1px solid var(--line); border-radius: 8px; background: var(--panel2); }
  #rag-listes details > summary { cursor: pointer; padding: 10px 14px; font-weight: 600; }
  #rag-listes details > .body { padding: 4px 14px 14px; }
  #rag-listes .tw { overflow-x: auto; }
  #rag-listes table { font-size: 13px; }
  #rag-listes td, #rag-listes th { white-space: nowrap; max-width: 260px; overflow: hidden; text-overflow: ellipsis; }
  #rag-listes td:first-child { font-weight: 400; }
  #rag-listes th.s { cursor: pointer; }
  #rag-listes img.th { width: 34px; height: 34px; object-fit: cover; border-radius: 4px; vertical-align: middle; border: 1px solid var(--line); }
  #rag-listes .formgrid { display: grid; grid-template-columns: repeat(auto-fill, minmax(220px, 1fr)); gap: 8px 12px; margin: 8px 0; }
  #rag-listes .formgrid label { display: flex; flex-direction: column; gap: 2px; font-size: 11.5px; color: var(--dim); text-transform: uppercase; letter-spacing: .6px; }
  #rag-listes .formgrid label.wide { grid-column: 1 / -1; }
  #rag-listes .msg { min-height: 20px; font-size: 13px; color: var(--ok); } #rag-listes .msg.bad { color: var(--bad); }
  #rag-listes .own { color: var(--ok); } #rag-listes .wish { color: #ff7aa8; }
  #rag-listes tr.editrow td { white-space: normal; max-width: none; background: var(--panel); }
  #rag-listes .frow { display: flex; gap: 8px; align-items: center; margin: 5px 0; flex-wrap: wrap; }`;

  function fieldInput(f, value) {
    const v = value === undefined || value === null ? '' : value;
    let e;
    switch (f.type) {
      case 'texte_long': e = h('textarea', { rows: 3, maxlength: 2000 }); e.value = String(v); break;
      case 'date': e = h('input', { type: 'date' }); e.value = String(v); break;
      case 'prix': e = h('input', { type: 'text', inputmode: 'decimal', placeholder: '0,00 €' }); e.value = v === '' ? '' : String(v).replace('.', ','); break;
      case 'choix': e = h('select', {}, h('option', { value: '' }, '—'), ...(f.choix || []).map((c) => h('option', { value: c }, c))); e.value = String(v); break;
      case 'oui_non': e = h('select', {}, h('option', { value: '' }, '—'), h('option', { value: 'oui' }, 'Oui'), h('option', { value: 'non' }, 'Non')); e.value = v === true ? 'oui' : v === false ? 'non' : ''; break;
      case 'lien': case 'image': e = h('input', { type: 'url', placeholder: 'https://…' }); e.value = String(v); break;
      default: e = h('input', { type: 'text', maxlength: 200 }); e.value = String(v);
    }
    return e;
  }

  function build() {
    const sec = h('section', { id: 'rag-listes' });
    sec.append(h('h2', {}, 'RAG — Listes à fiches', h('span', { class: 'tag' }, 'EN DIRECT')));
    const body = h('div');
    sec.append(body);
    const toast = h('div', { class: 'msg' });
    const say = (t, bad) => { toast.textContent = t || ''; toast.classList.toggle('bad', Boolean(bad)); };
    let busy = 0; // formulaires ouverts : on ne rafraîchit pas par-dessus
    const view = new Map(); // id de liste -> { q, filter, sort, dir, open }
    const vs = (id) => { if (!view.has(id)) view.set(id, { q: '', filter: 'all', sort: '', dir: 1, open: false }); return view.get(id); };
    let lists = [];
    let stats = null;
    let models = [];

    async function load() {
      if (!token) { render(); return; }
      const [l, s, m] = await Promise.all([api('lister'), api('stats'), api('modeles')]);
      if (!l.ok) { body.replaceChildren(h('p', { class: 'note' }, l.erreur || 'Indisponible.')); return; }
      const full = await Promise.all(l.listes.map((x) => api('lire', { liste: x.id })));
      lists = full.filter((r) => r.ok).map((r) => r.liste);
      stats = s.ok ? s : null;
      models = m.ok ? m.modeles : [];
      render();
    }

    async function run(op, args, okText) {
      say('…');
      const r = await api(op, args);
      if (r.ok) say(okText || 'Enregistré.'); else say(r.erreur || 'Échec.', true);
      if (r.ok) await load();
      return r;
    }

    function statsBar() {
      const s = stats || {};
      const stat = (v, l) => h('div', { class: 'stat' }, h('b', {}, String(v ?? '—')), h('span', {}, l));
      return h('div', { class: 'stats' }, stat(s.listes, 'listes à fiches'), stat(s.elements, 'fiches'), stat(s.passages, 'passages indexés'), stat(s.vecteurs, 'vecteurs calculés'),
        stat(s.modeleInstalle ? (s.modeleCharge ? 'chargé' : 'installé') : 'absent', 'modèle d\'embeddings'));
    }

    function ragTest() {
      const q = h('input', { type: 'search', placeholder: 'Poser une question (ex. est-ce que j\'ai le Lego 75436 ?)', style: 'flex:1;min-width:240px' });
      const out = h('div');
      const go = async () => {
        if (!q.value.trim()) return;
        out.replaceChildren(h('p', { class: 'note' }, 'Recherche…'));
        const r = await api('rechercher', { requete: q.value.trim(), limite: 8 });
        if (!r.ok) { out.replaceChildren(h('p', { class: 'bad' }, r.erreur || 'Échec.')); return; }
        if (!r.resultats.length) { out.replaceChildren(h('p', { class: 'note' }, 'Rien de pertinent : Jarvis ne trouverait aucune fiche.')); return; }
        out.replaceChildren(h('table', {}, h('thead', {}, h('tr', {}, ...['Liste', 'Fiche', 'Statut', 'Score', 'Retrouvée'].map((t) => h('th', {}, t)))),
          h('tbody', {}, ...r.resultats.map((x) => h('tr', {}, h('td', {}, x.listeNom), h('td', {}, x.element.titre + (x.element.sousTitre ? ` — ${x.element.sousTitre}` : '')), h('td', {}, statutText(x.element.statut, x.vocabulaire)),
            h('td', {}, String(x.score)), h('td', {}, x.exact ? 'identifiant exact' : 'par le sens / les mots'))))));
      };
      q.addEventListener('keydown', (e) => { if (e.key === 'Enter') go(); });
      return h('div', {}, h('h3', {}, 'Tester le RAG'), h('div', { class: 'bar' }, q, h('button', { onclick: go }, 'Chercher')), out,
        h('p', { class: 'note' }, 'Montre ce que Jarvis retrouverait pour cette question : fiche, score et façon dont elle a été retrouvée.'));
    }

    const statutText = (s, voc) => { const v = Object.assign({ fait: 'Possédé', voulu: 'Souhaité' }, voc || {}); return s === 'possede' ? v.fait : s === 'souhaite' ? v.voulu : '—'; };

    function itemForm(liste, element, close) {
      const inputs = liste.champs.map((f) => [f, fieldInput(f, element ? (element.valeurs || {})[f.cle] : '')]);
      const msg = h('div', { class: 'msg' });
      const form = h('form', {}, h('div', { class: 'formgrid' }, ...inputs.map(([f, i]) => h('label', { class: f.type === 'texte_long' ? 'wide' : '' }, f.libelle, i))), msg,
        h('div', { class: 'bar' }, h('button', { type: 'submit' }, element ? 'Enregistrer' : 'Ajouter'), h('button', { type: 'button', class: 'plain', onclick: () => { busy--; close(); } }, 'Annuler')));
      form.addEventListener('submit', async (e) => {
        e.preventDefault();
        const valeurs = {};
        for (const [f, i] of inputs) { const v = i.tagName === 'TEXTAREA' ? i.value : i.value.trim(); if (element || v !== '') valeurs[f.cle] = v; }
        msg.textContent = '…';
        let r = element ? await api('modifier_element', { liste: liste.id, element: element.id, valeurs }) : await api('ajouter_element', { liste: liste.id, valeurs });
        if (r.doublon && window.confirm(`${r.erreur} Ajouter quand même ?`)) r = await api('ajouter_element', { liste: liste.id, valeurs, forcer: true });
        if (r.ok) { busy--; say(element ? 'Fiche modifiée.' : 'Fiche ajoutée.'); await load(); } else { msg.textContent = r.erreur || 'Échec.'; msg.classList.add('bad'); }
      });
      busy++;
      return form;
    }

    function bulkForm(liste, close) {
      const voc = Object.assign({ fait: 'Possédé', voulu: 'Souhaité' }, liste.vocabulaire || {});
      const hasId = (liste.identifiant || []).length > 0;
      const placeholder = hasId
        ? `Colle ou écris ta liste de ${liste.noms.pluriel} (ex. numéros de sets séparés par des espaces, virgules ou retours à la ligne : 75192, 75436, 10307…)`
        : `Colle ou écris plusieurs ${liste.noms.pluriel} (un par ligne ou séparés par des virgules)…`;
      const ta = h('textarea', { rows: 4, maxlength: 20000, placeholder });
      const statutSel = liste.etat
        ? h('select', {}, h('option', { value: '' }, 'Statut automatique'), h('option', { value: 'possede' }, voc.fait), h('option', { value: 'souhaite' }, voc.voulu))
        : null;
      const msg = h('div', { class: 'msg' });
      const submitBtn = h('button', { type: 'submit' }, 'Ajouter tout');
      const form = h('form', {},
        h('div', { class: 'formgrid' },
          h('label', { class: 'wide' }, `Texte à mettre en forme par l'IA (${liste.noms.pluriel})`, ta),
          statutSel ? h('label', {}, 'Statut par défaut', statutSel) : null),
        msg,
        h('div', { class: 'bar' }, submitBtn, h('button', { type: 'button', class: 'plain', onclick: () => { busy--; close(); } }, 'Annuler')));
      form.addEventListener('submit', async (e) => {
        e.preventDefault();
        const texte = ta.value.trim();
        if (!texte) { msg.textContent = 'Écris ou colle au moins un élément.'; msg.classList.add('bad'); return; }
        submitBtn.disabled = true;
        msg.classList.remove('bad');
        msg.textContent = 'Mise en forme par l\'IA et enregistrement…';
        const r = await api('ajouter_texte', { liste: liste.id, texte, statut: statutSel ? statutSel.value : '' });
        submitBtn.disabled = false;
        if (r.ok) { busy--; say(r.resume || 'Éléments ajoutés.'); await load(); } else { msg.textContent = r.erreur || 'Échec.'; msg.classList.add('bad'); }
      });
      busy++;
      return form;
    }

    // Fiches cochées : suppression ou modification groupées (un seul « annule » défait tout).
    function bulkBar(liste, st) {
      const n = st.sel.size;
      if (!n) return h('span', {});
      const ids = [...st.sel];
      const fields = liste.champs.filter((f) => f.cle !== liste.titre && f.type !== 'image');
      const sel = h('select', { 'aria-label': 'Champ à modifier' }, ...fields.map((f) => h('option', { value: f.cle }, f.libelle)));
      const val = h('input', { type: 'text', placeholder: 'Nouvelle valeur', maxlength: 200 });
      const del = h('button', { class: 'danger', onclick: async () => {
        if (del.dataset.armed !== '1') { del.dataset.armed = '1'; del.textContent = 'Confirmer ?'; setTimeout(() => { del.dataset.armed = ''; del.textContent = `Supprimer ${n} fiche${n > 1 ? 's' : ''}`; }, 4000); return; }
        st.sel.clear();
        await run('supprimer_elements', { liste: liste.id, elements: ids }, 'Fiches supprimées (« annule » à la voix les remet toutes).');
      } }, `Supprimer ${n} fiche${n > 1 ? 's' : ''}`);
      return h('div', { class: 'frow' }, `${n} cochée${n > 1 ? 's' : ''} :`, sel, val,
        h('button', { onclick: () => run('modifier_elements', { liste: liste.id, elements: ids, valeurs: { [sel.value]: val.value.trim() } }, 'Fiches modifiées.') }, 'Appliquer à la sélection'), del);
    }

    function itemsTable(liste, st, renderAgain) {
      const q = fold(st.q);
      let rows = liste.elements.filter((it) => (st.filter === 'all' || it.statut === st.filter) && (!q || fold([it.titre, it.sousTitre, ...Object.values(it.affichage || {})].join(' ')).includes(q)));
      if (st.sort) rows = rows.slice().sort((a, b) => st.dir * fold((a.affichage || {})[st.sort] || '').localeCompare(fold((b.affichage || {})[st.sort] || ''), 'fr', { numeric: true }));
      const cols = liste.champs.filter((f) => f.type !== 'image');
      if (!st.sel) st.sel = new Set();
      st.sel = new Set([...st.sel].filter((id) => liste.elements.some((i) => i.id === id)));
      const allBox = h('input', { type: 'checkbox', title: 'Tout cocher / décocher', 'aria-label': 'Tout sélectionner' });
      allBox.checked = rows.length > 0 && rows.every((it) => st.sel.has(it.id));
      allBox.addEventListener('change', () => { rows.forEach((it) => { if (allBox.checked) st.sel.add(it.id); else st.sel.delete(it.id); }); renderAgain(); });
      const head = h('tr', {}, h('th', {}, allBox), ...cols.map((f) => h('th', { class: 's', title: 'Trier', onclick: () => { st.dir = st.sort === f.cle ? -st.dir : 1; st.sort = f.cle; renderAgain(); } }, f.libelle + (st.sort === f.cle ? (st.dir > 0 ? ' ▲' : ' ▼') : ''))), h('th', {}, ''));
      const tbody = h('tbody');
      for (const it of rows) {
        const tr = h('tr', {},
          h('td', {}, (() => { const cb = h('input', { type: 'checkbox', 'aria-label': 'Sélectionner' }); cb.checked = st.sel.has(it.id); cb.addEventListener('change', () => { if (cb.checked) st.sel.add(it.id); else st.sel.delete(it.id); renderAgain(); }); return cb; })(), it.image && /^https?:\/\//i.test(it.image) ? h('img', { class: 'th', src: it.image, alt: '', referrerpolicy: 'no-referrer', loading: 'lazy' }) : ''),
          ...cols.map((f) => h('td', { title: (it.affichage || {})[f.cle] || '', class: f.cle === liste.etat ? (it.statut === 'possede' ? 'own' : it.statut === 'souhaite' ? 'wish' : '') : '' }, (it.affichage || {})[f.cle] || '')),
          h('td', {}, h('button', { class: 'plain', onclick: () => { const edit = h('tr', { class: 'editrow' }, h('td', { colspan: cols.length + 2 }, itemForm(liste, it, () => edit.remove()))); tr.after(edit); } }, 'Modifier'), ' ',
            (() => { const b = h('button', { class: 'danger', onclick: async () => { if (b.dataset.armed !== '1') { b.dataset.armed = '1'; b.textContent = 'Confirmer ?'; setTimeout(() => { b.dataset.armed = ''; b.textContent = 'Supprimer'; }, 4000); return; } await run('supprimer_element', { liste: liste.id, element: it.id }, 'Fiche supprimée (« annule » à la voix la remet).'); } }, 'Supprimer'); return b; })()));
        tbody.append(tr);
      }
      if (!rows.length) tbody.append(h('tr', {}, h('td', { colspan: cols.length + 2 }, liste.elements.length ? 'Aucun résultat.' : 'Aucune fiche pour l\'instant.')));
      return h('div', {}, bulkBar(liste, st), h('div', { class: 'tw' }, h('table', {}, h('thead', {}, head), tbody)));
    }

    function schemaPanel(liste) {
      const rows = liste.champs.map((f) => {
        const label = h('input', { type: 'text', value: f.libelle, maxlength: 40 });
        label.addEventListener('change', () => { if (label.value.trim() && label.value.trim() !== f.libelle) run('modifier_schema', { liste: liste.id, renommer: { [f.cle]: label.value.trim() } }, 'Champ renommé.'); });
        const del = f.cle === liste.titre ? h('span', { class: 'note' }, 'sert de nom')
          : h('button', { class: 'danger', onclick: () => { if (window.confirm(`Retirer le champ « ${f.libelle} » ? Ses valeurs disparaissent.`)) run('modifier_schema', { liste: liste.id, retirer: [f.cle] }, 'Champ retiré.'); } }, 'Retirer');
        return h('div', { class: 'frow' }, label, h('code', {}, f.type), del);
      });
      const nl = h('input', { type: 'text', placeholder: 'Nouveau champ (ex. Pays)', maxlength: 40 });
      const nt = h('select', {}, ...['texte', 'nombre', 'prix', 'date', 'texte_long', 'lien', 'image', 'oui_non', 'choix'].map((t) => h('option', { value: t }, t)));
      const nc = h('input', { type: 'text', placeholder: 'Choix séparés par des virgules (type choix)', style: 'min-width:240px' });
      const name = h('input', { type: 'text', value: liste.nom, maxlength: 60 });
      const confirmName = h('input', { type: 'text', placeholder: `Retape « ${liste.nom} » pour supprimer` });
      return h('details', {}, h('summary', {}, 'Champs et réglages de la liste'), h('div', { class: 'body' },
        h('div', { class: 'frow' }, 'Nom :', name, h('button', { onclick: () => run('modifier_schema', { liste: liste.id, nom: name.value.trim() }, 'Liste renommée.') }, 'Renommer')),
        h('h3', {}, 'Champs'), ...rows,
        h('div', { class: 'frow' }, nl, nt, nc, h('button', { onclick: () => {
          if (!nl.value.trim()) return;
          const f = { libelle: nl.value.trim(), type: nt.value };
          if (nt.value === 'choix') f.choix = nc.value.split(',').map((x) => x.trim()).filter(Boolean);
          run('modifier_schema', { liste: liste.id, ajouter: [f] }, 'Champ ajouté.');
        } }, '+ Champ')),
        h('h3', {}, 'Zone de danger'),
        h('div', { class: 'frow' }, confirmName, h('button', { class: 'danger', onclick: () => run('supprimer_liste', { liste: liste.id, confirmer: confirmName.value.trim() }, 'Liste supprimée.') }, 'Supprimer la liste et ses fiches'))));
    }

    function listBlock(liste) {
      const st = vs(liste.id);
      const holder = h('div');
      const slot = h('div');
      const renderAgain = () => holder.replaceChildren(itemsTable(liste, st, renderAgain));
      const search = h('input', { type: 'search', placeholder: 'Chercher dans la liste…', value: st.q });
      search.addEventListener('input', () => { st.q = search.value; renderAgain(); });
      const filter = h('select', {}, h('option', { value: 'all' }, 'Tous'), h('option', { value: 'possede' }, (liste.vocabulaire || {}).faitPluriel || 'Possédés'), h('option', { value: 'souhaite' }, (liste.vocabulaire || {}).vouluPluriel || 'Souhaités'));
      filter.value = st.filter;
      filter.addEventListener('change', () => { st.filter = filter.value; renderAgain(); });
      const add = h('button', { onclick: () => { if (!slot.firstChild) slot.append(itemForm(liste, null, () => slot.replaceChildren())); } }, '+ Ajouter une fiche');
      const addText = h('button', { onclick: () => { if (!slot.firstChild) slot.append(bulkForm(liste, () => slot.replaceChildren())); } }, '+ Ajouter en texte');
      const images = h('button', { class: 'plain', title: "Cherche sur le web l'image des fiches qui n'en ont pas (une recherche IA par fiche)", onclick: async () => {
        const r = await api('enrichir_liste', { liste: liste.id });
        say(r.ok ? (r.say || 'Recherche lancée.') : r.erreur, !r.ok);
      } }, '🖼 Images');
      const stats = h('button', { class: 'plain', title: 'Totaux, dépenses et répartitions par champ', onclick: async () => {
        const r = await api('statistiques', { liste: liste.id });
        if (!r.ok) { say(r.erreur, true); return; }
        const s = r.stats;
        const lines = [`${s.total} ${liste.noms.pluriel}` + (liste.etat ? ` (${s.possedes} / ${s.souhaites})` : '') + (s.avecPrix ? ` · ${String(s.depense).replace('.', ',')} € dépensés sur ${s.avecPrix} prix notés` : '')];
        for (const g of s.repartitions || []) lines.push(`${g.libelle} : ` + g.lignes.map((l) => `${l.valeur} (${l.nombre})`).join(', '));
        slot.replaceChildren(h('div', { class: 'note' }, ...lines.flatMap((l) => [l, h('br')])), h('button', { class: 'plain', onclick: () => slot.replaceChildren() }, 'Fermer'));
      } }, '📊 Statistiques');
      const file = h('input', { type: 'file', accept: '.csv,.txt,text/csv', style: 'display:none' });
      const importCsv = h('button', { class: 'plain', title: 'Importer un CSV (Excel : Enregistrer sous → CSV) : la 1re ligne porte le nom des champs', onclick: () => file.click() }, '⇪ Importer un CSV');
      file.addEventListener('change', async () => {
        const f = file.files && file.files[0];
        file.value = '';
        if (!f) return;
        const r = await run('importer_csv', { liste: liste.id, csv: await f.text() }, 'Import terminé.');
        if (r && r.ok) say(`${r.resume}${(r.colonnesIgnorees || []).length ? ` Colonnes ignorées : ${r.colonnesIgnorees.join(', ')}.` : ''}`);
      });
      const csv = h('button', { class: 'plain', onclick: async () => {
        const r = await api('exporter_csv', { liste: liste.id });
        if (!r.ok) { say(r.erreur, true); return; }
        const a = h('a', { href: URL.createObjectURL(new Blob(['﻿' + r.csv], { type: 'text/csv;charset=utf-8' })), download: `${liste.id}.csv` });
        document.body.append(a); a.click(); a.remove();
      } }, 'Exporter en CSV');
      renderAgain();
      const det = h('details', { open: st.open ? true : null }, h('summary', {}, `${liste.icone || '📋'} ${liste.nom} — ${liste.total} ${liste.total > 1 ? liste.noms.pluriel : liste.noms.singulier}${liste.etat ? ` (${liste.possedes} ${((liste.vocabulaire || {}).faitPluriel || 'Possédés').toLowerCase()}, ${liste.souhaites} ${((liste.vocabulaire || {}).vouluPluriel || 'Souhaités').toLowerCase()})` : ''}`),
        h('div', { class: 'body' }, h('div', { class: 'bar' }, search, liste.etat ? filter : null, add, addText, images, stats, importCsv, file, csv), slot, holder, schemaPanel(liste),
          h('p', { class: 'note' }, `Alias reconnus à la voix : ${(liste.alias || []).join(', ') || '—'}. Enrichissement : ${liste.enrichissement && liste.enrichissement.fournisseur !== 'aucun' ? liste.enrichissement.fournisseur : 'aucun'}.`)));
      det.addEventListener('toggle', () => { st.open = det.open; });
      return det;
    }

    function newList() {
      const sel = h('select', {}, ...models.map((m) => h('option', { value: m.id }, `${m.icone} ${m.nom}`)));
      const name = h('input', { type: 'text', placeholder: 'Nom de la liste (facultatif)', maxlength: 60 });
      const fieldsBox = h('div', { class: 'formgrid' });
      const checks = [];
      const showFields = () => {
        const m = models.find((x) => x.id === sel.value);
        checks.length = 0;
        fieldsBox.replaceChildren(...((m && m.champs) || []).map((f) => { const c = h('input', { type: 'checkbox' }); c.checked = Boolean(f.retenu); checks.push([f, c]); return h('label', { style: 'flex-direction:row;align-items:center;gap:6px;text-transform:none;letter-spacing:0' }, c, f.libelle, h('code', {}, f.type)); }));
      };
      sel.addEventListener('change', showFields);
      showFields();
      const go = h('button', { onclick: async () => {
        const m = models.find((x) => x.id === sel.value);
        if (!m) return;
        const champs = checks.filter(([, c]) => c.checked).map(([f]) => f);
        if (!champs.length) { say('Coche au moins un champ.', true); return; }
        const preset = Object.assign({}, m, { champs, nom: name.value.trim() || m.nom });
        const retenusTitre = champs.some((f) => f.cle === m.titre);
        if (!retenusTitre) delete preset.titre;
        await run('creer_liste', preset, 'Liste créée.');
      } }, 'Créer la liste');
      return h('details', {}, h('summary', {}, '+ Nouvelle liste'), h('div', { class: 'body' },
        h('p', { class: 'note' }, 'Choisis un modèle puis les champs à garder. Pour un sujet libre, dis « crée-moi une liste de … » : l\'IA propose les champs, ou ajoute-les ici une fois la liste « Ma liste » créée.'),
        h('div', { class: 'bar' }, sel, name), fieldsBox, h('div', { class: 'bar' }, go)));
    }

    function render() {
      if (!token) {
        body.replaceChildren(h('p', { class: 'note' }, 'Pour voir et modifier tes listes ici, ouvre cette page depuis le bouton « ? AIDE » du HUD de Jarvis (le jeton d\'accès est dans l\'adresse). Ouverte à la main, elle reste en lecture seule et ne montre rien de tes listes.'));
        return;
      }
      body.replaceChildren(
        h('p', { class: 'note' }, 'Toutes tes listes à fiches et leur contenu. Les modifications faites ici sont les mêmes que celles faites à la voix ou dans la card du HUD : mêmes règles de validation, même index RAG, mêmes retours en arrière (« annule » à la voix).'),
        statsBar(),
        h('div', { class: 'bar' }, h('button', { onclick: async () => { say('Réindexation…'); const r = await api('reindexer'); say(r.ok ? `Index recalculé (${r.calcules} passages).` : r.erreur, !r.ok); await load(); } }, 'Réindexer'), toast),
        ragTest(),
        h('h3', {}, 'Listes'),
        ...(lists.length ? lists.map(listBlock) : [h('p', { class: 'note' }, 'Aucune liste à fiches. Dis « crée-moi une liste de disques vinyle » ou crée-en une ci-dessous.')]),
        newList());
    }

    load();
    setInterval(() => { if (!document.hidden && !busy && token) load(); }, 10000);
    document.addEventListener('visibilitychange', () => { if (!document.hidden && !busy && token) load(); });
    return sec;
  }

  document.head.append(h('style', {}, STYLE));
  window.AideListes = { build, aToken: () => Boolean(token) };
})();
