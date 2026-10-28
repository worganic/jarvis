// Jarvis Nomade — application du site : connexion mémorisée, sources partagées par Jarvis, boîte de dépôt.
(() => {
  'use strict';

  const $ = (s) => document.querySelector(s);
  // section : rubrique du menu ; tab : source ouverte (liste, notes…) ; day : jour choisi dans l'agenda.
  const state = { me: null, section: 'accueil', tab: null, records: {}, search: '', filter: '', offline: false, day: null, month: null };
  const REFRESH_MS = 45000;

  // Rubriques du menu. Une source appartient à une rubrique selon son type (kind) ; le reste va dans « Autres ».
  const SECTIONS = [
    { id: 'accueil', icon: '◎', label: 'Accueil' },
    { id: 'listes', icon: '☰', label: 'Listes', kinds: ['collection', 'liste'] },
    { id: 'data', icon: '◈', label: 'Data', kinds: ['dossiers'] },
    { id: 'agenda', icon: '▦', label: 'Agenda', kinds: ['agenda'] },
    { id: 'agents', icon: '✦', label: 'Agents', kinds: ['agents'] },
    { id: 'projets', icon: '◧', label: 'Projets', kinds: ['projets'] },
    { id: 'autres', icon: '⋯', label: 'Autres' },
    { id: 'depot', icon: '✎', label: 'Déposer', owner: true },
  ];
  const KNOWN_KINDS = SECTIONS.flatMap((s) => s.kinds || []);

  // --- Outils -----------------------------------------------------------------------------------

  function h(tag, attrs, ...kids) {
    const el = document.createElement(tag);
    for (const [k, v] of Object.entries(attrs || {})) {
      if (v == null || v === false) continue;
      if (k === 'class') el.className = v;
      else if (k.startsWith('on')) el.addEventListener(k.slice(2), v);
      else el.setAttribute(k, v === true ? '' : v);
    }
    for (const kid of kids.flat()) if (kid != null && kid !== false) el.append(kid.nodeType ? kid : String(kid));
    return el;
  }

  function store(key, value) {
    try {
      if (value === undefined) return JSON.parse(localStorage.getItem('nomade:' + key) || 'null');
      localStorage.setItem('nomade:' + key, JSON.stringify(value));
    } catch { return null; }
  }

  function toast(text) {
    const t = h('div', { class: 'toast', role: 'status' }, text);
    document.body.append(t);
    setTimeout(() => t.remove(), 2600);
  }

  async function api(route, opts = {}) {
    const init = { method: opts.body ? 'POST' : 'GET', credentials: 'same-origin', headers: { 'X-Nomade': '1' } };
    if (opts.body) {
      init.headers['Content-Type'] = 'application/json';
      init.body = JSON.stringify(opts.body);
    }
    const q = opts.query ? '&' + new URLSearchParams(opts.query) : '';
    const res = await fetch('api/?r=' + route + q, init);
    const data = await res.json().catch(() => ({ ok: false, error: 'Réponse illisible du serveur.' }));
    if (res.status === 401 && !['login', 'activate', 'invite-info', 'password'].includes(route)) {
      showLogin();
      throw new Error(data.error || 'Connexion requise.');
    }
    if (!data.ok) throw new Error(data.error || 'Erreur ' + res.status);
    return data;
  }

  function ago(ms) {
    if (!ms) return 'jamais';
    const s = Math.max(0, (Date.now() - ms) / 1000);
    if (s < 90) return "à l'instant";
    if (s < 3600) return `il y a ${Math.round(s / 60)} min`;
    if (s < 86400) return `il y a ${Math.round(s / 3600)} h`;
    return `le ${new Date(ms).toLocaleDateString('fr-FR')}`;
  }

  // Image d'un enregistrement : copie envoyée par Jarvis (servie par le site) ou adresse web d'origine.
  function imageSrc(d) {
    if (typeof d._img === 'string' && /^[a-f0-9]{64}\.(jpg|png|gif|webp)$/.test(d._img)) return 'api/?r=file&n=' + d._img;
    if (typeof d._imgUrl === 'string' && d._imgUrl.startsWith('https://')) return d._imgUrl;
    return null;
  }

  const safeUrl = (u) => (typeof u === 'string' && /^https?:\/\//i.test(u) ? u : null);
  const link = (label, url) => h('a', { href: url, target: '_blank', rel: 'noopener noreferrer' }, label);

  const fold = (s) => String(s ?? '').normalize('NFD').replace(/[̀-ͯ]/g, '').toLowerCase();

  // --- Connexion --------------------------------------------------------------------------------

  function showLogin() {
    $('#main').hidden = true;
    $('#invite').hidden = true;
    $('#login').hidden = false;
  }

  // --- Invitation : lien reçu par mail (#invite=<jeton>&login=<identifiant>) -----------------------

  const isOwner = () => state.me?.role !== 'guest';

  async function showInvite(token, login) {
    history.replaceState(null, '', location.pathname + location.search); // le jeton ne reste pas dans l'adresse
    $('#main').hidden = true;
    $('#login').hidden = true;
    $('#invite').hidden = false;
    const f = $('#invite-form');
    f.elements.login.value = login;
    try {
      const d = await api('invite-info', { body: { login, token } });
      $('#invite-hello').textContent = d.name ? `Bonjour ${d.name} : des informations sont partagées avec toi.` : 'Des informations sont partagées avec toi.';
    } catch (err) {
      $('#invite-error').textContent = err.message;
      f.querySelector('button').disabled = true;
      return;
    }
    f.onsubmit = async (e) => {
      e.preventDefault();
      $('#invite-error').textContent = '';
      if (f.elements.password.value !== f.elements.confirm.value) {
        $('#invite-error').textContent = 'Les deux mots de passe ne sont pas identiques.';
        return;
      }
      try {
        await api('activate', { body: { login, token, password: f.elements.password.value, label: navigator.userAgent.slice(0, 150) } });
        f.reset();
        $('#invite').hidden = true;
        await boot();
      } catch (err) {
        $('#invite-error').textContent = err.message;
      }
    };
  }

  $('#login-form').addEventListener('submit', async (e) => {
    e.preventDefault();
    const f = new FormData(e.target);
    $('#login-error').textContent = '';
    try {
      await api('login', { body: { login: f.get('login'), password: f.get('password'), label: navigator.userAgent.slice(0, 150) } });
      e.target.reset();
      await boot();
    } catch (err) {
      $('#login-error').textContent = err.message;
    }
  });

  // --- Chargement -------------------------------------------------------------------------------

  async function loadMe() {
    try {
      state.me = await api('me');
      state.offline = false;
      store('me', state.me);
    } catch (err) {
      if ($('#login').hidden === false) throw err;
      state.me = store('me');
      state.offline = true;
      if (!state.me) throw err;
    }
  }

  async function loadRecords(source) {
    try {
      const d = await api('records', { query: { source } });
      state.records[source] = d.records;
      store('rec:' + source, d.records);
    } catch {
      state.records[source] = store('rec:' + source) || [];
      state.offline = true;
    }
  }

  async function boot() {
    try {
      await loadMe();
    } catch {
      showLogin();
      return;
    }
    $('#login').hidden = true;
    $('#main').hidden = false;
    const saved = store('nav') || {};
    if (!state.tab && saved.section) {
      state.section = saved.section;
      state.tab = saved.tab || null;
    }
    if (!visibleSections().some((s) => s.id === state.section)) state.section = 'accueil';
    if (state.tab && !state.me.sources.some((s) => s.id === state.tab)) state.tab = null;
    await go(state.section, state.tab);
  }

  async function refresh() {
    if (document.hidden || $('#main').hidden) return;
    await loadMe().catch(() => {});
    await Promise.all(neededSources().map(loadRecords));
    render();
  }

  // --- Rubriques --------------------------------------------------------------------------------

  const sectionOf = (src) => SECTIONS.find((s) => (s.kinds || []).includes(src.kind))?.id || 'autres';
  const sourcesOf = (section) => (state.me?.sources || []).filter((s) => sectionOf(s) === section);
  const sourceOfKind = (kind) => (state.me?.sources || []).find((s) => s.kind === kind);

  /** Rubriques du menu : celles qui ont des données (Listes et Accueil toujours, Déposer pour le propriétaire). */
  function visibleSections() {
    return SECTIONS.filter((s) => {
      if (s.owner) return isOwner();
      if (s.id === 'accueil' || s.id === 'listes') return true;
      return sourcesOf(s.id).length > 0;
    });
  }

  /** Liste à moi ou partagée avec moi : créée par moi (ou par Jarvis pour le propriétaire), sinon partagée. */
  const isPersonal = (src) => src.iCreated || (isOwner() && src.creatorUser == null);

  /** Sources dont la rubrique ouverte a besoin. */
  function neededSources() {
    if (state.tab) return [state.tab];
    if (state.section === 'accueil') return ['agenda', 'agents', 'dossiers', 'projets'].map(sourceOfKind).filter(Boolean).map((s) => s.id);
    if (['data', 'agenda', 'agents', 'projets'].includes(state.section)) return sourcesOf(state.section).map((s) => s.id);
    return [];
  }

  async function go(section, tab = null) {
    state.section = section;
    state.tab = tab;
    state.search = '';
    state.filter = '';
    store('nav', { section, tab });
    await Promise.all(neededSources().map(loadRecords));
    render();
    window.scrollTo(0, 0);
  }
  const openTab = (id) => go(sectionOf(state.me.sources.find((s) => s.id === id) || {}), id);

  // --- Rendu ------------------------------------------------------------------------------------

  function render() {
    const me = state.me;
    const seen = me.jarvis?.lastSeen || 0;
    const online = Date.now() - seen < 3 * 60000;
    $('#jarvis-dot').classList.toggle('on', online);
    $('#jarvis-state').textContent = online ? 'Jarvis en ligne' : `Jarvis vu ${ago(seen)}`;
    $('#offline').hidden = !state.offline;
    $('#tabs').replaceChildren(...visibleSections().map((s) => {
      const n = s.id === 'depot' ? me.inboxWaiting : s.id === 'accueil' ? 0 : s.id === 'listes' ? sourcesOf('listes').length : sourcesOf(s.id).reduce((a, x) => a + (x.count || 0), 0);
      return h('button', { class: s.id === state.section ? 'active' : '', onclick: () => go(s.id), 'aria-current': s.id === state.section ? 'page' : null },
        h('span', { class: 'ico', 'aria-hidden': 'true' }, s.icon), h('span', { class: 'lbl' }, s.label), n ? h('span', { class: 'count' }, n) : null);
    }));
    const view = $('#view');
    if (state.section === 'depot') return view.replaceChildren(head('Déposer pour Jarvis', 'Une note, un rappel, une liste ou une demande : Jarvis la récupère dès qu’il est allumé.'), ...renderDeposit());
    if (state.tab) {
      const src = me.sources.find((s) => s.id === state.tab);
      if (src) return view.replaceChildren(back(), head(src.label, src.by ? `Partagée par ${src.by}` : src.group), ...renderSource(src, state.records[src.id] || []));
    }
    const draw = { accueil: renderHome, listes: renderLists, data: renderData, agenda: renderAgenda, agents: renderAgents, projets: renderProjects, autres: renderOthers }[state.section] || renderHome;
    view.replaceChildren(...draw());
  }

  function head(title, sub) {
    return h('div', { class: 'head' }, h('h1', {}, title), sub ? h('p', { class: 'sub' }, sub) : null);
  }

  function back() {
    const s = SECTIONS.find((x) => x.id === state.section) || SECTIONS[0];
    return h('button', { class: 'back', onclick: () => go(state.section) }, '← ' + s.label);
  }

  const recordsOf = (kind) => { const s = sourceOfKind(kind); return s ? state.records[s.id] || [] : []; };
  const empty = (text) => h('p', { class: 'empty' }, text);
  const notShared = (what) => empty(isOwner() ? `${what} : rien n’est partagé. Coche-le dans l’administration de Jarvis › Nomade.` : 'Rien ne t’est partagé ici pour le moment.');
  const clip = (s, n) => { s = String(s ?? ''); return s.length > n ? s.slice(0, n - 1) + '…' : s; };
  const dateOf = (v) => { const ms = typeof v === 'number' ? v : Date.parse(v); return Number.isFinite(ms) && ms > 0 ? new Date(ms) : null; };
  const dayKey = (d) => `${d.getFullYear()}-${String(d.getMonth() + 1).padStart(2, '0')}-${String(d.getDate()).padStart(2, '0')}`;
  const hm = (d) => d.toLocaleTimeString('fr-FR', { hour: '2-digit', minute: '2-digit' });
  const longDay = (d) => d.toLocaleDateString('fr-FR', { weekday: 'long', day: 'numeric', month: 'long' });

  // --- Accueil : vue d'ensemble ----------------------------------------------------------------

  function renderHome() {
    const me = state.me;
    const tiles = visibleSections().filter((s) => !['accueil', 'depot'].includes(s.id)).map((s) => {
      const srcs = s.id === 'listes' ? sourcesOf('listes') : sourcesOf(s.id);
      const n = s.id === 'listes' ? srcs.length : srcs.reduce((a, x) => a + (x.count || 0), 0);
      const unit = { listes: 'liste', data: 'dossier', agenda: 'rendez-vous', agents: 'agent', projets: 'projet', autres: 'élément' }[s.id];
      return h('button', { class: 'tile', onclick: () => go(s.id) }, h('span', { class: 'tile-ico' }, s.icon), h('span', { class: 'tile-n' }, n),
        h('span', { class: 'tile-l' }, `${s.label}`), h('span', { class: 'tile-s' }, n > 1 && unit !== 'rendez-vous' ? unit + 's' : unit));
    });
    const out = [head(me.name ? `Bonjour ${me.name}` : 'Vue d’ensemble', 'Toutes tes informations partagées par Jarvis.'), h('div', { class: 'tiles' }, ...tiles)];
    const next = recordsOf('agenda').filter((r) => (dateOf(r.data.debut) || 0) >= new Date(new Date().toDateString())).sort((a, b) => dateOf(a.data.debut) - dateOf(b.data.debut)).slice(0, 5);
    if (sourceOfKind('agenda')) out.push(panel('Prochains rendez-vous', () => go('agenda'), next.length ? next.map(eventRow) : [empty('Aucun rendez-vous à venir.')]));
    const agents = recordsOf('agents');
    if (agents.length) out.push(panel('Agents', () => go('agents'), agents.slice(0, 4).map(agentRow)));
    const dossiers = recordsOf('dossiers').slice().sort((a, b) => String(b.data.maj || '').localeCompare(String(a.data.maj || ''))).slice(0, 4);
    if (dossiers.length) out.push(panel('Derniers dossiers', () => go('data'), [h('div', { class: 'cards' }, ...dossiers.map(dossierCard))]));
    const projets = recordsOf('projets').slice().sort((a, b) => (b.data.lastActivity || 0) - (a.data.lastActivity || 0)).slice(0, 4);
    if (projets.length) out.push(panel('Projets récents', () => go('projets'), [h('div', { class: 'cards' }, ...projets.map(projectCard))]));
    const lists = sourcesOf('listes').slice().sort((a, b) => (b.updatedAt || 0) - (a.updatedAt || 0)).slice(0, 6);
    out.push(panel('Listes', () => go('listes'), lists.length ? [h('div', { class: 'cards lists' }, ...lists.map(listTile))] : [empty('Aucune liste pour le moment.')]));
    return out;
  }

  function panel(title, more, kids) {
    return h('section', { class: 'panel' }, h('div', { class: 'panel-head' }, h('h2', {}, title), more ? h('button', { class: 'link', onclick: more }, 'Tout voir →') : null), ...kids);
  }

  // --- Listes : à moi / partagées avec moi -------------------------------------------------------

  function listTile(src) {
    const icon = src.meta?.icone || (src.kind === 'liste' ? '☑' : '▤');
    const label = String(src.label || '').replace(/^\p{Extended_Pictographic}\s*/u, '');
    return h('button', { class: 'card list-tile', onclick: () => openTab(src.id) },
      h('span', { class: 'list-ico' }, icon),
      h('span', { class: 'card-body' }, h('span', { class: 'card-t' }, label || '—'),
        h('span', { class: 'card-s' }, `${src.count} élément${src.count > 1 ? 's' : ''}${src.kind === 'collection' ? ' · fiches' : ''}${src.by ? ' · par ' + src.by : ''}`)));
  }

  function renderLists() {
    const all = sourcesOf('listes');
    const grid = h('div', {});
    const draw = () => {
      const q = fold(state.search);
      const match = (s) => !q || fold(s.label).includes(q);
      const m = all.filter(isPersonal).filter(match);
      const sh = all.filter((s) => !isPersonal(s)).filter(match);
      grid.replaceChildren(
        h('h2', { class: 'group' }, `Mes listes · ${m.length}`),
        m.length ? h('div', { class: 'cards lists' }, ...m.map(listTile)) : empty(isOwner() ? 'Aucune liste partagée par Jarvis.' : 'Tu n’as pas encore créé de liste.'),
        h('h2', { class: 'group' }, `Partagées avec moi · ${sh.length}`),
        sh.length ? h('div', { class: 'cards lists' }, ...sh.map(listTile)) : empty('Aucune liste partagée avec toi.'));
    };
    draw();
    const toolbar = h('div', { class: 'toolbar' },
      h('input', { type: 'search', placeholder: 'Rechercher une liste…', value: state.search, oninput: (e) => { state.search = e.target.value; draw(); } }),
      isOwner() ? null : h('button', { onclick: () => listForm(null) }, '+ Liste'),
      h('button', { title: 'Décris un type de liste : Jarvis prépare un modèle sur mesure', onclick: () => requestForm() }, '✚ Demander une liste'));
    return [head('Listes', 'Tes listes et celles qu’on partage avec toi.'), toolbar, grid];
  }

  function renderOthers() {
    const srcs = sourcesOf('autres');
    if (!srcs.length) return [head('Autres', ''), notShared('Notes, rappels, suivis')];
    const groups = [...new Set(srcs.map((s) => s.group))];
    return [head('Autres', 'Notes, rappels, suivis et mémoire de Jarvis.'),
      ...groups.flatMap((g) => [h('h2', { class: 'group' }, g), h('div', { class: 'cards lists' }, ...srcs.filter((s) => s.group === g).map(listTile))])];
  }

  // --- Data : dossiers de recherche ------------------------------------------------------------

  function dossierCard(r) {
    const d = r.data || {};
    const img = imageSrc(d);
    return h('button', { class: 'card dossier', onclick: () => openDossier(r) },
      img ? h('img', { class: 'cover', src: img, alt: '', loading: 'lazy', referrerpolicy: 'no-referrer', onerror: (e) => e.target.replaceWith(h('div', { class: 'cover none' }, d.icone || '◈')) }) : h('div', { class: 'cover none' }, d.icone || '◈'),
      h('span', { class: 'card-body' }, h('span', { class: 'card-t' }, d._t || '—'), d._s ? h('span', { class: 'card-s' }, d._s) : null,
        d.resume ? h('span', { class: 'card-p' }, clip(d.resume, 150)) : null));
  }

  function renderData() {
    const records = recordsOf('dossiers');
    if (!sourceOfKind('dossiers')) return [head('Data', ''), notShared('Dossiers Data')];
    const grid = h('div', { class: 'cards' });
    const draw = () => {
      const q = fold(state.search);
      const list = records.filter((r) => !q || fold(JSON.stringify(r.data)).includes(q)).sort((a, b) => String(b.data.maj || '').localeCompare(String(a.data.maj || '')));
      grid.replaceChildren(...(list.length ? list.map(dossierCard) : [empty(records.length ? 'Aucun résultat.' : 'Aucun dossier pour le moment.')]));
    };
    draw();
    return [head('Data', `${records.length} dossier${records.length > 1 ? 's' : ''} de recherche`),
      h('div', { class: 'toolbar' }, h('input', { type: 'search', placeholder: 'Rechercher dans les dossiers…', value: state.search, oninput: (e) => { state.search = e.target.value; draw(); } })), grid];
  }

  function openDossier(r) {
    const d = r.data || {};
    const kids = [h('h2', {}, `${d.icone ? d.icone + ' ' : ''}${d._t || '—'}`), d._s ? h('p', { class: 'sub' }, d._s) : null];
    const img = imageSrc(d);
    if (img) kids.push(h('img', { class: 'photo', src: img, alt: d._t || '', referrerpolicy: 'no-referrer', onerror: (e) => e.target.remove() }));
    if (d.resume) kids.push(h('div', { class: 'group' }, 'Résumé'), h('p', { class: 'para' }, d.resume));
    if (Array.isArray(d.fiche) && d.fiche.length) kids.push(h('div', { class: 'group' }, 'Fiche technique'), h('dl', {}, ...d.fiche.flatMap(([k, v]) => [h('dt', {}, k), h('dd', {}, v)])));
    if (Array.isArray(d.sections) && d.sections.length) {
      kids.push(h('div', { class: 'group' }, 'Sections'), ...d.sections.map((s) => h('details', { class: 'section' }, h('summary', {}, s.titre || '—'), h('p', { class: 'para' }, s.texte || ''))));
    }
    if (Array.isArray(d.chronologie) && d.chronologie.length) {
      kids.push(h('div', { class: 'group' }, 'Chronologie'), h('ol', { class: 'timeline' }, ...d.chronologie.map((c) => h('li', {}, h('b', {}, c.date), ' ', c.fait))));
    }
    const videos = (d.videos || []).filter((v) => safeUrl(v.url));
    if (videos.length) kids.push(h('div', { class: 'group' }, 'Vidéos'), h('div', { class: 'links' }, ...videos.map((v) => link(`▶ ${clip(v.titre || 'Vidéo', 60)}${v.duree ? ' · ' + v.duree : ''}`, v.url))));
    const links = (Array.isArray(d._liens) ? d._liens : []).filter((l) => Array.isArray(l) && safeUrl(l[1]));
    if (links.length) kids.push(h('div', { class: 'group' }, 'Sources'), h('div', { class: 'links' }, ...links.map(([label, url]) => link('↗ ' + clip(label || url, 60), url))));
    if (d.maj) kids.push(h('p', { class: 'sub version' }, `Mis à jour ${ago(Date.parse(d.maj))}.`));
    kids.push(h('div', { class: 'actions' }, h('span', { class: 'spacer' }), h('button', { onclick: closeSheet }, 'Fermer')));
    $('#sheet').classList.add('wide');
    sheet(...kids);
  }

  // --- Agenda -----------------------------------------------------------------------------------

  function eventRow(r) {
    const d = r.data || {};
    const start = dateOf(d.debut);
    return h('button', { class: 'row event', onclick: () => openEvent(r) },
      h('span', { class: 'when' }, start ? h('b', {}, start.toLocaleDateString('fr-FR', { day: 'numeric', month: 'short' })) : null, h('span', {}, d.journee || !start ? 'journée' : hm(start))),
      h('div', { class: 'txt' }, h('div', { class: 't' }, d._t || '(sans titre)'), d.lieu ? h('div', { class: 's' }, '⌖ ' + d.lieu) : null));
  }

  function openEvent(r) {
    const d = r.data || {};
    const start = dateOf(d.debut);
    const end = dateOf(d.fin);
    const when = !start ? d.debut : d.journee ? longDay(start) + ' · toute la journée' : `${longDay(start)} · ${hm(start)}${end ? ' – ' + hm(end) : ''}`;
    sheet(h('h2', {}, d._t || '(sans titre)'), h('p', { class: 'sub' }, when),
      h('dl', {}, d.lieu ? [h('dt', {}, 'Lieu'), h('dd', {}, d.lieu)] : null, d.source ? [h('dt', {}, 'Agenda'), h('dd', {}, d.source)] : null),
      d.description ? h('p', { class: 'para' }, d.description) : null,
      h('div', { class: 'actions' }, h('span', { class: 'spacer' }), h('button', { onclick: closeSheet }, 'Fermer')));
  }

  function renderAgenda() {
    if (!sourceOfKind('agenda')) return [head('Agenda', ''), notShared('Agenda')];
    const events = recordsOf('agenda').filter((r) => dateOf(r.data.debut)).sort((a, b) => dateOf(a.data.debut) - dateOf(b.data.debut));
    const byDay = new Map();
    for (const r of events) {
      const k = dayKey(dateOf(r.data.debut));
      if (!byDay.has(k)) byDay.set(k, []);
      byDay.get(k).push(r);
    }
    const today = new Date();
    const month = state.month || new Date(today.getFullYear(), today.getMonth(), 1);
    const cal = h('div', { class: 'calendar' });
    const list = h('div', { class: 'rows' });

    function drawList() {
      const todayKey = dayKey(today);
      const keys = state.day ? [state.day] : [...byDay.keys()].filter((k) => k >= todayKey).slice(0, 30);
      if (!keys.length || (state.day && !byDay.has(state.day))) {
        list.replaceChildren(empty(state.day ? 'Rien de prévu ce jour-là.' : 'Aucun rendez-vous à venir.'));
        return;
      }
      list.replaceChildren(...keys.flatMap((k) => [h('div', { class: 'group' }, longDay(new Date(k + 'T12:00:00'))), ...(byDay.get(k) || []).map(eventRow)]));
    }

    function drawCal() {
      const first = new Date(month.getFullYear(), month.getMonth(), 1);
      const offset = (first.getDay() + 6) % 7; // semaine qui commence le lundi
      const days = new Date(month.getFullYear(), month.getMonth() + 1, 0).getDate();
      const cells = [];
      for (let i = 0; i < offset; i++) cells.push(h('span', { class: 'cell out' }));
      for (let n = 1; n <= days; n++) {
        const d = new Date(month.getFullYear(), month.getMonth(), n);
        const k = dayKey(d);
        const count = (byDay.get(k) || []).length;
        cells.push(h('button', { class: `cell${k === dayKey(today) ? ' today' : ''}${k === state.day ? ' sel' : ''}${count ? ' busy' : ''}`, title: count ? `${count} rendez-vous` : null,
          onclick: () => { state.day = state.day === k ? null : k; drawCal(); drawList(); } }, h('span', {}, n), count ? h('i', {}, count > 3 ? '•••' : '•'.repeat(count)) : null));
      }
      const shift = (m) => { state.month = new Date(month.getFullYear(), month.getMonth() + m, 1); state.day = null; render(); };
      cal.replaceChildren(
        h('div', { class: 'cal-head' }, h('button', { class: 'icon', 'aria-label': 'Mois précédent', onclick: () => shift(-1) }, '‹'),
          h('b', {}, month.toLocaleDateString('fr-FR', { month: 'long', year: 'numeric' })),
          h('button', { class: 'icon', 'aria-label': 'Mois suivant', onclick: () => shift(1) }, '›')),
        h('div', { class: 'cal-grid' }, ...['L', 'M', 'M', 'J', 'V', 'S', 'D'].map((x) => h('span', { class: 'dow' }, x)), ...cells),
        state.day ? h('button', { class: 'link', onclick: () => { state.day = null; drawCal(); drawList(); } }, 'Voir tous les prochains rendez-vous') : null);
    }
    drawCal();
    drawList();
    const upcoming = events.filter((r) => dateOf(r.data.debut) >= new Date(today.toDateString())).length;
    return [head('Agenda', `${upcoming} rendez-vous à venir (90 jours)`), h('div', { class: 'agenda' }, cal, h('div', { class: 'agenda-list' }, list))];
  }

  // --- Agents -----------------------------------------------------------------------------------

  const AGENT_STATE = { en_cours: ['En cours', 'run'], actif: ['Actif', 'own'], pause: ['En pause', 'wish'] };

  function agentRow(r) {
    const d = r.data || {};
    const [label, cls] = AGENT_STATE[d.etat] || ['—', ''];
    const last = dateOf(d.derniereExecution);
    return h('button', { class: 'card agent', onclick: () => openAgent(r) },
      h('span', { class: 'card-body' },
        h('span', { class: 'card-top' }, h('span', { class: 'card-t' }, d._t || '—'), h('span', { class: 'badge ' + cls }, label)),
        d._s ? h('span', { class: 'card-s' }, d._s) : null,
        (d.declencheurs || []).length ? h('span', { class: 'chips' }, ...d.declencheurs.map((t) => h('span', { class: 'chip' }, t))) : null,
        h('span', { class: 'card-s' }, last ? `Dernière exécution ${ago(last.getTime())}${d.nbExecutions ? ` · ${d.nbExecutions} au total` : ''}` : 'Jamais exécuté'),
        d.dernierResultat ? h('span', { class: 'card-p' }, clip(d.dernierResultat, 180)) : null));
  }

  function openAgent(r) {
    const d = r.data || {};
    const [label] = AGENT_STATE[d.etat] || ['—'];
    const kids = [h('h2', {}, d._t || '—'), h('p', { class: 'sub' }, [label, d.modele, d.sortie ? 'sortie : ' + d.sortie : null].filter(Boolean).join(' · '))];
    if (d.description) kids.push(h('p', { class: 'para' }, d.description));
    if ((d.declencheurs || []).length) kids.push(h('div', { class: 'group' }, 'Déclencheurs'), h('div', { class: 'chips' }, ...d.declencheurs.map((t) => h('span', { class: 'chip' }, t))));
    if (d.consigne) kids.push(h('div', { class: 'group' }, 'Consigne'), h('div', { class: 'body' }, d.consigne));
    if (d.dernierResultat) kids.push(h('div', { class: 'group' }, 'Dernier résultat'), h('div', { class: 'body' }, d.dernierResultat));
    const hist = (d.historique || []).filter((x) => x.resultat || x.at);
    if (hist.length) kids.push(h('div', { class: 'group' }, 'Historique'), ...hist.map((x) => h('details', { class: 'section' },
      h('summary', {}, `${dateOf(x.at) ? dateOf(x.at).toLocaleString('fr-FR', { dateStyle: 'short', timeStyle: 'short' }) : ''}${x.declencheur ? ' · ' + x.declencheur : ''}${x.statut ? ' · ' + x.statut : ''}`),
      h('p', { class: 'para' }, x.resultat || ''))));
    kids.push(h('div', { class: 'actions' }, h('span', { class: 'spacer' }), h('button', { onclick: closeSheet }, 'Fermer')));
    $('#sheet').classList.add('wide');
    sheet(...kids);
  }

  function renderAgents() {
    if (!sourceOfKind('agents')) return [head('Agents', ''), notShared('Agents')];
    const records = recordsOf('agents').slice().sort((a, b) => (a.data.etat === 'pause') - (b.data.etat === 'pause'));
    const active = records.filter((r) => r.data.etat !== 'pause').length;
    return [head('Agents', `${records.length} agent${records.length > 1 ? 's' : ''} · ${active} actif${active > 1 ? 's' : ''}`),
      records.length ? h('div', { class: 'cards' }, ...records.map(agentRow)) : empty('Aucun agent pour le moment.')];
  }

  // --- Projets ----------------------------------------------------------------------------------

  function projectCard(r) {
    const d = r.data || {};
    const g = d.git;
    return h('button', { class: 'card project', onclick: () => openProject(r) },
      h('span', { class: 'card-body' },
        h('span', { class: 'card-top' }, h('span', { class: 'card-t' }, (d.favorite ? '★ ' : '') + (d._t || '—')), g && g.changes ? h('span', { class: 'badge wish', title: 'Fichiers modifiés non commités' }, `${g.changes} modif.`) : null),
        d._s ? h('span', { class: 'card-p' }, d._s) : null,
        (d.tech || []).length ? h('span', { class: 'chips' }, ...d.tech.slice(0, 5).map((t) => h('span', { class: 'chip' }, t))) : null,
        h('span', { class: 'card-s' }, [g && g.branch ? '⎇ ' + g.branch : null, d.lastActivity ? 'actif ' + ago(d.lastActivity) : null].filter(Boolean).join(' · '))));
  }

  function openProject(r) {
    const d = r.data || {};
    const g = d.git;
    const kids = [h('h2', {}, (d.favorite ? '★ ' : '') + (d._t || '—')), h('p', { class: 'sub' }, d.lastActivity ? `Dernière activité ${ago(d.lastActivity)}` : '')];
    if (d.description) kids.push(h('p', { class: 'para' }, d.description));
    const rows = [];
    if ((d.tech || []).length) rows.push(['Technologies', d.tech.join(', ')]);
    if (g) {
      rows.push(['Branche', g.branch || '—']);
      if (g.lastCommit) rows.push(['Dernier commit', `${g.lastCommit}${g.lastCommitAt ? ' (' + ago(g.lastCommitAt) + ')' : ''}`]);
      rows.push(['Fichiers modifiés', String(g.changes ?? 0)]);
    }
    if ((d.subprojects || []).length) rows.push(['Sous-projets', d.subprojects.join(', ')]);
    if (rows.length) kids.push(h('dl', {}, ...rows.flatMap(([k, v]) => [h('dt', {}, k), h('dd', {}, v)])));
    kids.push(h('div', { class: 'actions' }, h('span', { class: 'spacer' }), h('button', { onclick: closeSheet }, 'Fermer')));
    sheet(...kids);
  }

  function renderProjects() {
    if (!sourceOfKind('projets')) return [head('Projets', ''), notShared('Projets')];
    const records = recordsOf('projets').filter((r) => !r.data.hidden);
    const grid = h('div', { class: 'cards' });
    const draw = () => {
      const q = fold(state.search);
      const list = records.filter((r) => !q || fold(JSON.stringify(r.data)).includes(q))
        .sort((a, b) => (b.data.favorite - a.data.favorite) || ((b.data.lastActivity || 0) - (a.data.lastActivity || 0)));
      grid.replaceChildren(...(list.length ? list.map(projectCard) : [empty(records.length ? 'Aucun résultat.' : 'Aucun projet pour le moment.')]));
    };
    draw();
    return [head('Projets', `${records.length} projet${records.length > 1 ? 's' : ''}, du plus récemment actif au plus ancien`),
      h('div', { class: 'toolbar' }, h('input', { type: 'search', placeholder: 'Rechercher un projet, une techno…', value: state.search, oninput: (e) => { state.search = e.target.value; draw(); } })), grid];
  }

  function statusBadge(src, d) {
    const lib = src.meta?.libelles || {};
    if (d._statut === 'possede') return h('span', { class: 'badge own' }, lib.possede || 'Possédé');
    if (d._statut === 'souhaite') return h('span', { class: 'badge wish' }, lib.souhaite || 'Souhaité');
    return null;
  }

  function renderSource(src, records) {
    const can = (op) => src.ops.includes(op);
    const toolbar = h('div', { class: 'toolbar' },
      h('input', { type: 'search', placeholder: 'Rechercher…', value: state.search, oninput: (e) => { state.search = e.target.value; drawRows(); } }));
    if (src.kind === 'collection' && src.meta?.etat) {
      const lib = src.meta.libelles || {};
      toolbar.append(h('select', { onchange: (e) => { state.filter = e.target.value; drawRows(); } },
        h('option', { value: '' }, 'Tout'), h('option', { value: 'possede' }, lib.possede || 'Possédé'), h('option', { value: 'souhaite' }, lib.souhaite || 'Souhaité')));
    }
    const rows = h('div', { class: 'rows' + (src.kind === 'liste' ? '' : ' grid') });

    function drawRows() {
      const q = fold(state.search);
      let list = records.filter((r) => !q || fold(JSON.stringify(r.data)).includes(q));
      if (state.filter) list = list.filter((r) => r.data._statut === state.filter);
      if (src.kind === 'liste') list.sort((a, b) => (a.data.checked === b.data.checked ? 0 : a.data.checked ? 1 : -1));
      if (!list.length) return rows.replaceChildren(h('p', { class: 'empty' }, records.length ? 'Aucun résultat.' : 'Vide pour le moment.'));
      let group = null;
      const out = [];
      for (const r of list) {
        const d = r.data || {};
        if (src.kind === 'liste' && d.category && d.category !== group) {
          group = d.category;
          out.push(h('div', { class: 'group' }, group));
        }
        const wait = r.origin === 'web' ? h('span', { class: 'badge wait', title: 'En attente de Jarvis' }, '⟳') : null;
        if (src.kind === 'liste') {
          out.push(h('div', { class: 'row' + (d.checked ? ' done' : '') },
            h('input', { type: 'checkbox', checked: !!d.checked, disabled: !can('check'), 'aria-label': d._t || d.text,
              onchange: (e) => edit(src, 'check', r.id, { checked: e.target.checked }) }),
            h('div', { class: 'txt', onclick: src.ownerUser && can('update') ? () => openForm(src, r) : null },
              h('div', { class: 't' }, d._t || d.text || '—'), d.note ? h('div', { class: 's' }, d.note) : null),
            src.ownerUser ? null : wait,
            can('delete') ? h('button', { class: 'icon', title: 'Supprimer', 'aria-label': 'Supprimer', onclick: () => edit(src, 'delete', r.id, {}) }, '✕') : null));
        } else {
          const img = imageSrc(d);
          out.push(h('button', { class: 'row', onclick: () => openRecord(src, r) },
            img ? h('img', { class: 'thumb', src: img, alt: '', loading: 'lazy', referrerpolicy: 'no-referrer', onerror: (e) => e.target.remove() }) : null,
            h('div', { class: 'txt' }, h('div', { class: 't' }, d._t || d.title || d.text || '—'), d._s ? h('div', { class: 's' }, d._s) : null),
            statusBadge(src, d), wait));
        }
      }
      rows.replaceChildren(...out);
    }
    drawRows();
    if (src.kind === 'collection' && can('add') && src.meta?.scan) {
      toolbar.append(h('button', { class: 'primary', title: 'Scanner le code-barres d’un article', onclick: () => openScanner(src) }, '📷 Scanner'));
    }
    // Liste créée par un invité : son créateur règle qui d'autre la voit (aucun accès, lecture, écriture).
    if (src.iCreated) toolbar.append(h('button', { title: 'Choisir qui voit et modifie cette liste', onclick: () => shareForm(src) }, '👥 Partager'));
    // Liste créée par un invité : la renommer ou la supprimer (son auteur, ou le propriétaire du site).
    if (src.mine || (isOwner() && src.ownerUser)) {
      toolbar.append(
        h('button', { onclick: () => listForm(src) }, 'Renommer'),
        h('button', { class: 'danger', onclick: async () => {
          if (!confirm(`Supprimer la liste « ${src.label} » et tout son contenu ?`)) return;
          try {
            await api('list-delete', { body: { id: src.id } });
            state.tab = null;
            await boot();
          } catch (err) { toast(err.message); }
        } }, 'Supprimer la liste'));
    }
    const out = [toolbar, rows];
    if (can('add')) out.push(h('button', { class: 'fab', title: 'Ajouter', 'aria-label': 'Ajouter', onclick: () => openForm(src, null) }, '+'));
    return out;
  }

  // --- Fiches -----------------------------------------------------------------------------------

  function sheet(...kids) {
    const dlg = $('#sheet');
    $('#sheet-body').replaceChildren(...kids.filter(Boolean));
    if (!dlg.open) dlg.showModal();
  }
  const closeSheet = () => $('#sheet').close();
  $('#sheet').addEventListener('close', () => $('#sheet').classList.remove('wide'));
  $('#sheet').addEventListener('click', (e) => { if (e.target === e.currentTarget) closeSheet(); });

  function fieldsOf(src) {
    return (src.meta?.champs || []).filter((c) => c.cle !== src.meta?.etat && !(c.type === 'image' && c.cle === src.meta?.image));
  }

  function openRecord(src, r) {
    const d = r.data || {};
    const can = (op) => src.ops.includes(op);
    const kids = [h('h2', {}, d._t || d.title || '—')];
    if (d._s) kids.push(h('p', { class: 'sub' }, d._s));
    const img = imageSrc(d);
    if (img) kids.push(h('img', { class: 'photo', src: img, alt: d._t || '', referrerpolicy: 'no-referrer', onerror: (e) => e.target.remove() }));
    if (src.kind === 'collection') {
      const dl = h('dl');
      for (const c of fieldsOf(src)) {
        const v = d[c.cle];
        if (v == null || v === '' || /^\s*(null|undefined|n\/a)\s*$/i.test(String(v)) || c.cle === src.meta.titre) continue;
        const url = c.type === 'lien' || c.type === 'image' ? safeUrl(v) : null;
        dl.append(h('dt', {}, c.libelle || c.cle), h('dd', {}, url ? link(url.replace(/^https?:\/\/(www\.)?/, '').slice(0, 60), url) : shown(c, v)));
      }
      kids.push(dl);
    } else if (src.kind === 'notes') {
      kids.push(h('div', { class: 'body' }, d.body || ''));
    } else if (Array.isArray(d._d)) {
      kids.push(h('dl', {}, ...d._d.flatMap(([k, v]) => [h('dt', {}, k), h('dd', {}, String(v ?? ''))])));
    }
    const links = (Array.isArray(d._liens) ? d._liens : []).filter((l) => Array.isArray(l) && safeUrl(l[1]));
    if (links.length) kids.push(h('div', { class: 'links' }, ...links.map(([label, url]) => link('↗ ' + label, url))));
    const actions = h('div', { class: 'actions' });
    if (src.kind === 'collection' && can('status') && src.meta?.etat) {
      const lib = src.meta.libelles || {};
      if (d._statut !== 'possede') actions.append(h('button', { onclick: () => edit(src, 'status', r.id, { _statut: 'possede' }).then(closeSheet) }, '✓ ' + (lib.possede || 'Possédé')));
      if (d._statut !== 'souhaite') actions.append(h('button', { onclick: () => edit(src, 'status', r.id, { _statut: 'souhaite' }).then(closeSheet) }, '☆ ' + (lib.souhaite || 'Souhaité')));
    }
    if (can('update')) actions.append(h('button', { onclick: () => openForm(src, r) }, 'Modifier'));
    actions.append(h('span', { class: 'spacer' }));
    if (can('delete')) actions.append(h('button', { class: 'danger', onclick: () => { if (confirm('Supprimer cet élément ?')) edit(src, 'delete', r.id, {}).then(closeSheet); } }, 'Supprimer'));
    actions.append(h('button', { onclick: closeSheet }, 'Fermer'));
    kids.push(actions);
    sheet(...kids);
  }

  function shown(c, v) {
    if (Array.isArray(v)) return v.join(', ');
    if (typeof v === 'boolean') return v ? 'Oui' : 'Non';
    if (c.type === 'date' && /^\d{4}-\d{2}-\d{2}$/.test(v)) return v.split('-').reverse().join('/');
    if (c.type === 'prix' && typeof v === 'number') return v.toLocaleString('fr-FR', { style: 'currency', currency: 'EUR' });
    return String(v);
  }

  function input(c, value) {
    if (c.type === 'oui_non') {
      return h('select', { name: c.cle }, h('option', { value: '' }, '—'), h('option', { value: 'oui', selected: value === true }, 'Oui'), h('option', { value: 'non', selected: value === false }, 'Non'));
    }
    if (Array.isArray(c.choix) && c.choix.length) {
      return h('select', { name: c.cle }, h('option', { value: '' }, '—'), ...c.choix.map((x) => h('option', { value: x, selected: String(value ?? '') === x }, x)));
    }
    if (c.type === 'texte_long' || c.type === 'notes') return h('textarea', { name: c.cle }, value ?? '');
    const type = c.type === 'nombre' || c.type === 'prix' ? 'number' : c.type === 'date' ? 'date' : 'text';
    return h('input', { name: c.cle, type, step: type === 'number' ? 'any' : null, value: Array.isArray(value) ? value.join(', ') : value ?? '' });
  }

  function openForm(src, r) {
    const d = r?.data || {};
    const form = h('form', {});
    let fields = [];
    if (src.kind === 'collection') fields = fieldsOf(src).map((c) => ({ ...c, value: d[c.cle] }));
    else if (src.kind === 'liste' && src.ownerUser) fields = [{ cle: 'text', libelle: 'Élément', value: d.text }, { cle: 'note', libelle: 'Note (taille, couleur, lien…)', value: d.note }, { cle: 'category', libelle: 'Catégorie', value: d.category }];
    else if (src.kind === 'liste') fields = [{ cle: 'text', libelle: 'Élément', value: d.text }, { cle: 'category', libelle: 'Rayon / catégorie', value: d.category }];
    else if (src.kind === 'notes') fields = [{ cle: 'title', libelle: 'Titre', value: d.title }, { cle: 'body', libelle: 'Texte', type: 'texte_long', value: d.body }];
    else if (src.kind === 'rappels') fields = [{ cle: 'what', libelle: 'Rappel', value: d.what }, { cle: 'at', libelle: 'Quand', type: 'datetime' }];
    for (const c of fields) {
      const el = c.type === 'datetime' ? h('input', { name: c.cle, type: 'datetime-local', required: true }) : input(c, c.value);
      form.append(h('label', {}, c.libelle || c.cle, el));
    }
    if (src.kind === 'collection' && !r && src.meta?.etat) {
      const lib = src.meta.libelles || {};
      form.append(h('label', {}, 'Statut', h('select', { name: '_statut' }, h('option', { value: '' }, '—'), h('option', { value: 'possede' }, lib.possede || 'Possédé'), h('option', { value: 'souhaite' }, lib.souhaite || 'Souhaité'))));
    }
    form.append(h('div', { class: 'actions' }, h('span', { class: 'spacer' }), h('button', { type: 'button', onclick: closeSheet }, 'Annuler'), h('button', { type: 'submit' }, r ? 'Enregistrer' : 'Ajouter')));
    form.addEventListener('submit', async (e) => {
      e.preventDefault();
      const data = {};
      for (const [k, v] of new FormData(form)) {
        let before = d[k];
        if (typeof before === 'boolean') before = before ? 'oui' : 'non';
        if (r && String(Array.isArray(before) ? before.join(', ') : before ?? '') === v) continue;
        data[k] = v;
      }
      if (src.kind === 'rappels' && data.at) data.at = new Date(data.at).getTime();
      if (!r) data._t = data[src.meta?.titre] || data.text || data.title || data.what || '';
      if (r && !Object.keys(data).length) return closeSheet();
      await edit(src, r ? 'update' : 'add', r?.id, data);
      closeSheet();
    });
    sheet(h('h2', {}, r ? 'Modifier' : 'Ajouter — ' + src.label), h('p', { class: 'sub' }, 'Jarvis reprendra la modification dès qu’il sera allumé.'), form);
  }

  // --- Scan d'un code-barres -------------------------------------------------------------------
  // Caméra arrière ; lecture native (BarcodeDetector : Chrome, Android) sinon ZXing (vendor/, Safari).
  // La fiche part avec son code ; Jarvis retrouve le vrai nom, les infos et la photo à sa prochaine lecture.

  const SCAN_FORMATS = ['ean_13', 'ean_8', 'upc_a', 'upc_e'];

  /** Clé de contrôle GTIN (EAN-8, UPC-A, EAN-13, GTIN-14) : écarte les lectures erronées. */
  function validGtin(code) {
    if (!/^\d{8}$|^\d{12,14}$/.test(code)) return false;
    const d = code.split('').map(Number);
    const check = d.pop();
    const sum = d.reverse().reduce((acc, n, i) => acc + n * (i % 2 === 0 ? 3 : 1), 0);
    return (10 - (sum % 10)) % 10 === check;
  }

  let zxingLoading = null;
  function loadZxing() {
    if (window.ZXing) return Promise.resolve(window.ZXing);
    zxingLoading = zxingLoading || new Promise((resolve, reject) => {
      const sc = document.createElement('script');
      sc.src = 'vendor/zxing.min.js';
      sc.onload = () => resolve(window.ZXing);
      sc.onerror = () => { zxingLoading = null; reject(new Error('Lecteur de codes-barres indisponible.')); };
      document.head.append(sc);
    });
    return zxingLoading;
  }

  async function nativeDetector() {
    if (!('BarcodeDetector' in window)) return null;
    try {
      const ok = (await window.BarcodeDetector.getSupportedFormats()).filter((f) => SCAN_FORMATS.includes(f));
      return ok.length ? new window.BarcodeDetector({ formats: ok }) : null;
    } catch { return null; }
  }

  // Un scan = une fiche : la caméra s'arrête dès qu'un code est lu, la fiche de validation s'affiche ;
  // rien n'est enregistré sans « Enregistrer ». Un nouveau scan repart du bouton 📷 de la liste.
  function openScanner(src) {
    let stream = null, reader = null, timer = null, read = false;

    const stop = () => {
      clearInterval(timer);
      timer = null;
      try { reader && reader.reset(); } catch {}
      reader = null;
      if (stream) stream.getTracks().forEach((t) => t.stop());
      stream = null;
    };
    $('#sheet').addEventListener('close', stop, { once: true });

    function found(raw) {
      const code = String(raw || '').replace(/\D/g, '');
      if (read || !validGtin(code)) return;
      read = true; // les lectures suivantes de la même image sont ignorées
      stop();
      if (navigator.vibrate) navigator.vibrate(80);
      confirmScan(code);
    }

    // Étape 1 : la caméra (ou le code tapé à la main).
    function scanView() {
      read = false;
      const video = h('video', { class: 'scan-video', playsinline: true, muted: true, autoplay: true });
      const status = h('p', { class: 'sub' }, 'Vise le code-barres de la boîte ou de la pochette.');
      const manual = h('input', { inputmode: 'numeric', placeholder: 'ou tape le code (8 à 14 chiffres)', maxlength: 18 });
      const typed = () => {
        const code = manual.value.replace(/\D/g, '');
        if (!validGtin(code)) { status.textContent = 'Ce code n’est pas un code-barres valide (vérifie les chiffres).'; return; }
        found(code);
      };
      manual.addEventListener('keydown', (e) => { if (e.key === 'Enter') typed(); });
      sheet(h('h2', {}, 'Scanner — ' + src.label), status, video,
        h('div', { class: 'line scan-line' }, manual, h('button', { onclick: typed }, 'Valider le code')),
        h('div', { class: 'actions' }, h('span', { class: 'spacer' }), h('button', { onclick: closeSheet }, 'Annuler')));
      start(video, status);
    }

    async function start(video, status) {
      try {
        const detector = await nativeDetector();
        if (detector) {
          stream = await navigator.mediaDevices.getUserMedia({ video: { facingMode: { ideal: 'environment' } }, audio: false });
          video.srcObject = stream;
          await video.play();
          timer = setInterval(async () => {
            if (read || video.readyState < 2) return;
            try { (await detector.detect(video)).forEach((b) => found(b.rawValue)); } catch {}
          }, 250);
        } else {
          const ZX = await loadZxing();
          const hints = new Map([[ZX.DecodeHintType.POSSIBLE_FORMATS, [ZX.BarcodeFormat.EAN_13, ZX.BarcodeFormat.EAN_8, ZX.BarcodeFormat.UPC_A, ZX.BarcodeFormat.UPC_E]]]);
          reader = new ZX.BrowserMultiFormatReader(hints, 250);
          await reader.decodeFromConstraints({ video: { facingMode: { ideal: 'environment' } }, audio: false }, video, (result) => { if (result) found(result.getText()); });
        }
      } catch (err) {
        status.textContent = (err && err.name === 'NotAllowedError')
          ? 'Accès à la caméra refusé : autorise-le dans le navigateur, ou tape le code.'
          : `Caméra indisponible (${(err && err.message) || err}) : tape le code.`;
      }
    }

    // Étape 2 : la fiche à valider.
    function confirmScan(code) {
      const lib = src.meta?.libelles || {};
      const statut = src.meta?.etat ? h('select', {}, h('option', { value: 'possede' }, lib.possede || 'Possédé'), h('option', { value: 'souhaite' }, lib.souhaite || 'Souhaité')) : null;
      const titleKey = src.meta?.titre;
      const name = h('input', { placeholder: 'facultatif : Jarvis le retrouve tout seul', maxlength: 200 });
      const dup = (state.records[src.id] || []).find((r) => r.data && r.data._barcode === code);
      const msg = h('p', { class: 'error' });
      const save = h('button', { class: 'primary', onclick: async () => {
        save.disabled = true;
        const data = { _barcode: code, _t: name.value.trim() || `Code-barres ${code}`, ...(statut ? { _statut: statut.value } : {}) };
        if (titleKey && name.value.trim()) data[titleKey] = name.value.trim();
        try {
          await edit(src, 'add', null, data);
          closeSheet();
          toast('Enregistré : Jarvis va retrouver le nom, les infos et la photo.');
        } catch (err) {
          msg.textContent = err.message;
          save.disabled = false;
        }
      } }, 'Enregistrer');
      sheet(h('h2', {}, 'Nouvel article — ' + src.label),
        h('p', { class: 'sub' }, 'Vérifie avant d’enregistrer. Le nom, les infos et la photo seront complétés par Jarvis.'),
        h('dl', {}, h('dt', {}, 'Code-barres'), h('dd', { class: 'code' }, code)),
        dup ? h('p', { class: 'warn' }, `Déjà dans la liste : « ${dup.data._t || code} ».`) : null,
        h('label', {}, 'Nom', name),
        statut ? h('label', {}, 'Statut', statut) : null,
        msg,
        h('div', { class: 'actions' },
          h('button', { onclick: scanView }, '📷 Scanner à nouveau'),
          h('span', { class: 'spacer' }),
          h('button', { onclick: closeSheet }, 'Annuler'),
          save));
    }

    scanView();
  }

  // Nouvelle liste (invité) ou nouveau nom.
  function listForm(src) {
    const form = h('form', {},
      h('label', {}, 'Nom de la liste', h('input', { name: 'title', value: src ? src.label : '', placeholder: 'Idées cadeaux, à voir, courses…', required: true, maxlength: 120 })),
      h('div', { class: 'actions' }, h('span', { class: 'spacer' }), h('button', { type: 'button', onclick: closeSheet }, 'Annuler'), h('button', { type: 'submit' }, src ? 'Renommer' : 'Créer')));
    form.addEventListener('submit', async (e) => {
      e.preventDefault();
      try {
        const title = form.elements.title.value;
        if (src) await api('list-rename', { body: { id: src.id, title } });
        else state.tab = (await api('list-create', { body: { title } })).id;
        closeSheet();
        await boot();
      } catch (err) { toast(err.message); }
    });
    sheet(h('h2', {}, src ? 'Renommer la liste' : 'Nouvelle liste'),
      h('p', { class: 'sub' }, src ? '' : 'Une liste à toi, que tu remplis comme tu veux ; elle est visible par la personne qui t’a invité et par son assistant Jarvis.'), form);
    setTimeout(() => form.elements.title.focus(), 50);
  }

  // Partage d'une liste créée par un invité : pour chaque autre invité, aucun accès / lecture / lecture et écriture.
  async function shareForm(src) {
    const body = h('div', { class: 'rows' }, h('p', { class: 'empty' }, 'Chargement…'));
    const selects = [];
    sheet(h('h2', {}, `Partager « ${src.label} »`), body);
    try {
      const d = await api('list-shares?id=' + encodeURIComponent(src.id));
      body.replaceChildren(
        h('p', { class: 'sub' }, `${d.owner ? d.owner + ' (propriétaire du site)' : 'Le propriétaire du site'} voit toujours cette liste. Choisis qui d’autre la voit.`),
        ...(d.guests.length ? d.guests.map((g) => {
          const sel = h('select', {}, ...[['none', 'Aucun accès'], ['lecture', 'Lecture seule'], ['ecriture', 'Lecture et écriture']].map(([v, t]) => h('option', { value: v, selected: g.mode === v }, t)));
          selects.push([g.user, sel]);
          return h('label', {}, g.name, sel);
        }) : [h('p', { class: 'empty' }, 'Aucun autre invité pour le moment.')]),
        h('div', { class: 'actions' }, h('span', { class: 'spacer' }), h('button', { type: 'button', onclick: closeSheet }, 'Annuler'),
          h('button', { type: 'button', class: 'primary', onclick: async () => {
            try {
              await api('list-share', { body: { id: src.id, shares: selects.map(([user, sel]) => ({ user, mode: sel.value })) } });
              toast('Partage enregistré.');
              closeSheet();
            } catch (err) { toast(err.message); }
          } }, 'Enregistrer')));
    } catch (err) { body.replaceChildren(h('p', { class: 'empty' }, err.message)); }
  }

  // Demande d'une liste d'un type nouveau (films, vêtements, animaux…) : Jarvis prépare un modèle, le propriétaire valide.
  function requestForm() {
    const STATE = { nouveau: 'envoyée', pris: 'reçue par Jarvis, en attente de validation', traite: 'créée', refuse: 'refusée' };
    const mine = h('div', { class: 'rows' });
    $('#sheet').classList.add('wide');
    const form = h('form', { class: 'req' },
      h('label', {}, 'Nom de la liste', h('input', { name: 'nom', placeholder: 'Mes films, Ma garde-robe, Mes animaux…', required: true, maxlength: 60 })),
      h('label', {}, 'Type de liste', h('textarea', { name: 'type', rows: 4, placeholder: 'Qu’est-ce qu’on y range ? Ex. : des films à voir ou déjà vus, avec réalisateur et année.', required: true, maxlength: 300 })),
      h('label', {}, 'Informations à noter pour chaque fiche (facultatif, une par ligne)', h('textarea', { name: 'champs', rows: 4, placeholder: ['Titre', 'Réalisateur', 'Année', 'Note sur 10', 'Vu ou à voir'].join(String.fromCharCode(10)) })),
      h('label', {}, 'Exemples (facultatif)', h('input', { name: 'exemples', placeholder: 'Matrix, Alien, Amélie Poulain', maxlength: 500 })),
      h('div', { class: 'actions' }, h('span', { class: 'spacer' }), h('button', { type: 'button', onclick: closeSheet }, 'Annuler'), h('button', { type: 'submit' }, 'Envoyer la demande')));
    async function loadMine() {
      try {
        const d = await api('list-requests');
        mine.replaceChildren(...(d.items.length ? d.items.slice(0, 3).map((i) => h('div', { class: 'row' },
          h('div', { class: 'txt' }, h('div', { class: 't' }, '✚ ' + (i.nom || 'Liste')), h('div', { class: 's' }, `${i.type.slice(0, 80)} · ${ago(i.createdAt)}`)),
          h('span', { class: 'badge' + (i.state === 'traite' ? ' own' : ' wait') }, STATE[i.state] || i.state))) : []));
      } catch { mine.replaceChildren(); }
    }
    form.addEventListener('submit', async (e) => {
      e.preventDefault();
      try {
        const f = form.elements;
        await api('list-request', { body: { nom: f.nom.value, type: f.type.value, champs: f.champs.value, exemples: f.exemples.value } });
        toast('Demande envoyée : Jarvis la récupère dès qu’il est allumé, puis le propriétaire valide.');
        form.reset();
        loadMine();
      } catch (err) { toast(err.message); }
    });
    sheet(h('h2', {}, 'Demander une liste'),
      h('p', { class: 'sub' }, 'Décris le type de liste voulu. Jarvis prépare un modèle adapté (champs, statuts), le propriétaire valide, puis la liste apparaît ici.'),
      form, h('div', { class: 'group' }, 'Mes demandes'), mine);
    loadMine();
    setTimeout(() => form.elements.nom.focus(), 50);
  }

  async function edit(src, op, id, data) {
    try {
      await api('edit', { body: { source: src.id, op, id, data } });
      await loadRecords(src.id);
      await loadMe().catch(() => {});
      render();
    } catch (err) {
      toast(err.message);
      throw err;
    }
  }

  // --- Boîte de dépôt ---------------------------------------------------------------------------

  function renderDeposit() {
    const NOTE_HINT = 'Une idée, une note, une chose à rappeler… Jarvis la récupère dès qu’il est allumé.';
    const text = h('textarea', { placeholder: NOTE_HINT });
    const title = h('input', { placeholder: 'Nom de la liste (courses, valise, à faire…)', maxlength: 80, hidden: true });
    const kind = h('select', {}, h('option', { value: 'note' }, 'Note'), h('option', { value: 'rappel' }, 'Rappel'), h('option', { value: 'liste' }, 'Liste à cocher'), h('option', { value: 'demande' }, 'Demande à Jarvis'));
    // Liste : un nom et un élément par ligne ; Jarvis la crée dès réception et la partage ici.
    kind.addEventListener('change', () => {
      const list = kind.value === 'liste';
      title.hidden = !list;
      text.placeholder = list ? 'Un élément par ligne :\nlait\nœufs\npain' : NOTE_HINT;
      if (list) title.focus();
    });
    const list = h('div', { class: 'rows' }, h('p', { class: 'empty' }, 'Chargement…'));
    const Speech = window.SpeechRecognition || window.webkitSpeechRecognition;
    let rec = null;
    const mic = Speech ? h('button', { type: 'button', class: 'mic', title: 'Dicter', 'aria-label': 'Dicter', onclick: () => {
      if (rec) { rec.stop(); return; }
      rec = new Speech();
      rec.lang = 'fr-FR';
      rec.interimResults = false;
      rec.onresult = (e) => { text.value = (text.value ? text.value + ' ' : '') + e.results[0][0].transcript; };
      rec.onend = () => { rec = null; mic.classList.remove('on'); };
      mic.classList.add('on');
      rec.start();
    } }, '🎤') : null;
    const send = h('button', { class: 'primary', onclick: async () => {
      const isList = kind.value === 'liste';
      if (isList && !title.value.trim()) { toast('Donne un nom à la liste.'); title.focus(); return; }
      if (!isList && !text.value.trim()) return;
      try {
        await api('inbox', { body: { text: isList ? title.value.trim() + '\n' + text.value : text.value, kind: kind.value } });
        text.value = '';
        title.value = '';
        toast(isList ? 'Liste déposée : Jarvis la crée dès qu’il est allumé, puis elle apparaît ici.' : 'Déposé pour Jarvis.');
        loadInbox();
      } catch (err) { toast(err.message); }
    } }, 'Déposer');
    async function loadInbox() {
      try {
        const d = await api('inbox');
        const label = { nouveau: 'en attente', pris: 'reçu par Jarvis', traite: 'fait', refuse: 'refusé' };
        const listName = (i) => { try { return JSON.parse(i.text).nom || 'Liste'; } catch { return 'Liste'; } };
        list.replaceChildren(...(d.items.length ? d.items.map((i) => h('div', { class: 'row' },
          h('div', { class: 'txt' }, h('div', { class: 't' }, i.kind === 'nouvelle_liste' ? '✚ ' + listName(i) : i.kind === 'liste' ? '☑ ' + i.text.split('\n')[0] : i.text), h('div', { class: 's' }, `${i.kind === 'nouvelle_liste' ? 'nouvelle liste' : i.kind === 'liste' ? `liste de ${Math.max(0, i.text.split('\n').filter((l) => l.trim()).length - 1)} élément(s)` : i.kind} · ${ago(i.createdAt)}`)),
          h('span', { class: 'badge' + (i.state === 'nouveau' ? ' wait' : ' own') }, label[i.state] || i.state))) : [h('p', { class: 'empty' }, 'Rien de déposé.')]));
      } catch { list.replaceChildren(h('p', { class: 'empty' }, 'Hors ligne.')); }
    }
    loadInbox();
    return [h('div', { class: 'deposit' }, title, text, h('div', { class: 'line' }, kind, mic, send)), h('div', { class: 'group' }, 'Derniers dépôts'), list];
  }

  // --- Compte -----------------------------------------------------------------------------------

  $('#btn-account').addEventListener('click', async () => {
    const list = h('div', { class: 'rows' }, 'Chargement…');
    const me = state.me || {};
    const ver = document.createTextNode('');
    api('version').then((v) => {
      ver.textContent = v.version ? `Site version ${v.version}${v.commit ? ` (${v.commit})` : ''}, déployé le ${new Date(v.deployedAt).toLocaleDateString('fr-FR')}.` : '';
    }).catch(() => {});
    const pwd = isOwner() ? null : h('form', { class: 'deposit' },
      h('label', {}, 'Mot de passe actuel', h('input', { name: 'old', type: 'password', autocomplete: 'current-password', required: true })),
      h('label', {}, 'Nouveau mot de passe (10 caractères au moins)', h('input', { name: 'new', type: 'password', autocomplete: 'new-password', minlength: 10, required: true })),
      h('div', { class: 'actions' }, h('span', { class: 'spacer' }), h('button', { type: 'submit' }, 'Changer le mot de passe')));
    if (pwd) pwd.addEventListener('submit', async (e) => {
      e.preventDefault();
      try {
        await api('password', { body: { old: pwd.elements.old.value, new: pwd.elements.new.value } });
        pwd.reset();
        toast('Mot de passe changé ; les autres appareils sont déconnectés.');
      } catch (err) { toast(err.message); }
    });
    sheet(h('h2', {}, 'Compte'),
      h('p', { class: 'sub' }, isOwner()
        ? `Connecté : ${me.login || ''}. L’identifiant et le mot de passe se changent dans l’administration de Jarvis.`
        : `Connecté : ${me.login || ''} · accès ${me.mode === 'ecriture' ? 'en lecture et écriture' : 'en lecture seule'}.`),
      pwd ? h('div', { class: 'group' }, 'Mot de passe') : null, pwd,
      h('div', { class: 'group' }, 'Appareils connectés'), list,
      h('p', { class: 'sub version' }, ver),
      h('div', { class: 'actions' }, h('button', { class: 'danger', onclick: async () => { await api('logout', { body: {} }).catch(() => {}); closeSheet(); showLogin(); } }, 'Se déconnecter'), h('span', { class: 'spacer' }), h('button', { onclick: closeSheet }, 'Fermer')));
    try {
      const d = await api('sessions');
      const draw = (rows) => list.replaceChildren(...rows.map((s) => h('div', { class: 'row' },
        h('div', { class: 'txt' }, h('div', { class: 't' }, (s.label || 'Appareil').slice(0, 80)), h('div', { class: 's' }, `${s.id === d.current ? 'cet appareil · ' : ''}${isOwner() && s.login !== me.login ? s.login + ' · ' : ''}vu ${ago(+s.last_seen)}`)),
        s.id === d.current ? null : h('button', { class: 'danger', onclick: async () => { await api('revoke', { body: { id: s.id } }); draw(rows.filter((x) => x.id !== s.id)); } }, 'Révoquer'))));
      draw(d.sessions.map((s) => ({ ...s, id: +s.id })));
    } catch (err) { list.replaceChildren(err.message); }
  });

  // --- Démarrage --------------------------------------------------------------------------------

  if ('serviceWorker' in navigator) navigator.serviceWorker.register('sw.js').catch(() => {});
  document.addEventListener('visibilitychange', () => { if (!document.hidden) refresh(); });
  setInterval(refresh, REFRESH_MS);
  const invite = new URLSearchParams(location.hash.slice(1));
  if (invite.get('invite') && invite.get('login')) showInvite(invite.get('invite'), invite.get('login'));
  else boot();
})();
