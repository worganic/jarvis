// Section « Nomade » de l'administration : relier Jarvis au site distant (web-nomade/, Hostinger),
// choisir les données partagées, changer l'identifiant et le mot de passe du site, suivre la synchro.
(function () {
  const A = window.JarvisAdmin;
  if (!A) return;
  const { h, panel, field, api, save, sections, render, toast, toggle, confirmDialog } = A;

  const state = { data: null, loading: false, busy: false, error: '', sessions: null, guests: null, guestsLoading: false, editing: null, lastInvite: null };

  async function call(method, ...args) {
    state.busy = true;
    render(true);
    const res = await api(method, ...args);
    state.busy = false;
    if (res && res.ok && res.sources) state.data = res;
    state.error = res && res.ok === false && !res.cancelled ? res.error || 'erreur' : '';
    render(true);
    return res;
  }

  async function load() {
    if (state.loading) return;
    state.loading = true;
    const res = await api('nomadeState');
    state.loading = false;
    if (res && res.ok) state.data = res;
    render(true);
  }

  // Les codes de validation passent par `save` (fenêtre de code commune) ; l'état est relu ensuite.
  async function validated(method, ...args) {
    const res = await save(method, ...args);
    if (res && res.ok && res.sources) state.data = res;
    else await load();
    render(true);
    return res;
  }

  const ago = (ms) => {
    if (!ms) return 'jamais';
    const s = (Date.now() - ms) / 1000;
    if (s < 90) return "à l'instant";
    if (s < 3600) return `il y a ${Math.round(s / 60)} min`;
    return new Date(ms).toLocaleString('fr-FR', { day: '2-digit', month: '2-digit', hour: '2-digit', minute: '2-digit' });
  };

  function connectionPanel(d) {
    const url = h('input', { type: 'url', value: d.url || d.urlHint || '', placeholder: 'https://jarvis.mon-domaine.fr', autocomplete: 'off', spellcheck: 'false' });
    const login = h('input', { type: 'text', value: d.login || '', placeholder: 'identifiant', autocomplete: 'off', spellcheck: 'false' });
    const pass = h('input', { type: 'password', placeholder: d.hasPassword ? 'inchangé (laisser vide)' : '10 caractères au moins', autocomplete: 'new-password' });
    const revoke = h('input', { type: 'checkbox' });
    const status = !d.configured ? 'non relié' : d.lastError ? 'erreur' : 'relié';

    const pair = async () => {
      const res = await call('nomadePair', url.value, login.value, pass.value);
      if (res && res.ok) toast(res.created ? '✓ Compte du site créé, Jarvis est relié' : '✓ Jarvis est relié au site');
    };
    const change = async () => {
      const res = await validated('nomadeAccountRequest', login.value, pass.value, revoke.checked);
      if (res && res.ok) toast('✓ Accès au site modifié');
    };

    return panel('Connexion au site', status,
      h('div', { class: 'grid' },
        field('Adresse du site', url, d.configured ? 'Changer d’adresse relie Jarvis à un autre site (nouvel envoi complet).' : d.urlHint ? 'Adresse laissée par le déploiement (deploy.ps1).' : 'Adresse où le dossier web-nomade a été déployé.'),
        field('Identifiant du site', login, 'Celui que tu tapes sur le téléphone. Lettres, chiffres, . _ @ -'),
        field('Mot de passe du site', pass, d.encrypted ? 'Gardé chiffré par Windows sur ce PC (pour se relier à nouveau).' : 'Non enregistré : chiffrement Windows indisponible.')),
      !d.configured && d.installAvailable ? h('div', { class: 'help', style: 'margin-top:8px' }, 'Site neuf : le premier « Relier » crée le compte avec cet identifiant et ce mot de passe (code d’installation fourni par le déploiement).') : null,
      d.configured ? h('label', { class: 'help', style: 'display:flex; gap:8px; align-items:center; margin-top:8px' }, revoke, 'Déconnecter aussi les téléphones et navigateurs déjà connectés') : null,
      h('div', { class: 'row', style: 'margin-top:10px' },
        d.configured
          ? h('button', { class: 'btn primary', disabled: state.busy, onclick: change }, 'Changer l’identifiant / le mot de passe')
          : h('button', { class: 'btn primary', disabled: state.busy, onclick: pair }, state.busy ? '… connexion' : 'Relier au site'),
        d.configured ? h('button', { class: 'btn ghost', disabled: state.busy, onclick: pair, title: 'Nouvelle clé d’appareil (autre adresse, clé perdue)' }, 'Relier à nouveau') : null,
        d.configured ? h('button', { class: 'btn ghost', disabled: state.busy, onclick: async () => {
          if (await confirmDialog('Oublier le site Nomade', 'Jarvis oublie l’adresse, l’identifiant et sa clé. Les données déjà en ligne ne sont pas effacées (décoche les sources avant pour les effacer).', 'Oublier', true)) call('nomadeForget');
        } }, 'Oublier') : null),
      state.error ? h('div', { class: 'help', style: 'color: var(--err); margin-top:10px' }, state.error) : null);
  }

  function syncPanel(d) {
    const s = d.summary || {};
    return panel('Synchronisation', d.busy ? 'en cours' : d.lastError ? 'erreur' : ago(d.lastSync),
      h('div', { class: 'help' }, d.lastSync
        ? `Dernier échange ${ago(d.lastSync)} : ${s.envoyes || 0} envoyé(s), ${s.supprimes || 0} retiré(s), ${s.recus || 0} modification(s) reprise(s) du site.`
        : 'Aucun échange pour l’instant.'),
      d.lastError ? h('div', { class: 'help', style: 'color: var(--warn); margin-top:6px' }, `Dernière erreur : ${d.lastError}`) : null,
      h('div', { class: 'grid', style: 'margin-top:10px' },
        toggle('Synchroniser automatiquement', d.enabled, (v) => call('nomadeOptions', { enabled: v })),
        field('Intervalle (secondes)', h('input', { type: 'number', min: 15, max: 3600, value: d.intervalS, onchange: (e) => call('nomadeOptions', { intervalS: Number(e.target.value) }) }), 'Lecture du site et envoi des changements. Une modification d’une liste à fiches part aussitôt.')),
      h('div', { class: 'row', style: 'margin-top:10px' },
        h('button', { class: 'btn primary', disabled: state.busy, onclick: async () => { const r = await call('nomadeSync'); if (r && r.ok) toast('✓ Synchronisé'); } }, state.busy ? '… échange' : 'Synchroniser maintenant')));
  }

  // Le site en ligne correspond-il au dossier web-nomade de ce Jarvis ? (empreintes écrites par deploy.ps1)
  function versionPanel(d) {
    if (!d.configured) return null;
    const v = d.site;
    const list = (title, files) => (files && files.length ? h('div', { class: 'help', style: 'margin-top:6px' }, `${title} : ${files.slice(0, 12).join(', ')}${files.length > 12 ? ` … (+${files.length - 12})` : ''}`) : null);
    const label = !v ? 'non vérifié' : v.ok === false ? 'injoignable' : !v.connu ? 'inconnue' : v.aJour ? 'à jour' : 'à déployer';
    const color = !v || v.ok === false || !v.connu ? 'var(--warn)' : v.aJour ? 'var(--ok)' : 'var(--err)';
    return panel('Version du site', label,
      !v ? h('div', { class: 'help' }, 'Pas encore comparé.')
        : v.ok === false ? h('div', { class: 'help', style: `color:${color}` }, `Vérification impossible : ${v.error}`)
          : h('div', {},
            h('div', { style: `color:${color}; font-weight:600` }, !v.connu
              ? 'Le site ne dit pas quelle version il porte : redéploie-le une fois (pwsh web-nomade/deploy.ps1).'
              : v.aJour ? `✓ Le site en ligne correspond exactement à Jarvis ${v.jarvisVersion} (${v.fichiers} fichiers, base à jour).`
                : '✕ Le site en ligne ne correspond pas à ce Jarvis : à déployer (pwsh web-nomade/deploy.ps1).'),
            v.connu ? h('div', { class: 'help', style: 'margin-top:6px' },
              `Déployé ${v.deployedAt ? new Date(v.deployedAt).toLocaleString('fr-FR') : '?'} depuis la version ${v.siteVersion || '?'}${v.commit ? ` (commit ${v.commit}${v.modifie ? ', modifications non commitées' : ''})` : ''} · Jarvis ${v.jarvisVersion}${v.php ? ` · PHP ${v.php}` : ''}.`) : null,
            list('Fichiers différents en ligne', v.differents),
            list('Absents du site', v.manquants),
            list('Migrations de base non appliquées', v.migrationsManquantes),
            list('En ligne mais plus dans Jarvis', v.enTrop),
            h('div', { class: 'help', style: 'margin-top:6px' }, `Vérifié ${ago(v.checkedAt)} (puis toutes les 10 minutes).`)),
      h('div', { class: 'row', style: 'margin-top:10px' },
        h('button', { class: 'btn ghost', disabled: state.busy, onclick: async () => { const r = await call('nomadeCheckSite'); if (r && r.ok) toast(r.site && r.site.aJour ? '✓ Site à jour' : '⚠ Site à déployer', !(r.site && r.site.aJour)); } }, 'Vérifier maintenant')));
  }

  function sourcesPanel(d) {
    const groups = {};
    for (const s of d.sources) (groups[s.group] = groups[s.group] || []).push(s);
    const shared = d.sources.filter((s) => s.shared).length;
    return panel('Données partagées', `${shared} / ${d.sources.length}`,
      h('div', { class: 'help', style: 'margin-bottom:8px' }, 'Coche ce qui doit être consultable sur le site. Décocher efface ces données du site (code de validation). « Modifiable » : le site peut y ajouter ou changer des éléments, repris par Jarvis.'),
      ...Object.entries(groups).map(([g, list]) => h('div', { style: 'margin-top:12px' },
        h('div', { class: 'help', style: 'text-transform:uppercase; letter-spacing:.08em; color: var(--accent)' }, g),
        h('div', { class: 'grid' }, ...list.map((s) => h('div', {},
          toggle(`${s.label} (${s.count})`, s.shared, async (v) => {
            if (v) {
              if (s.sensitive && !(await confirmDialog('Donnée sensible', `« ${s.label} » contient des informations personnelles. Les mettre en ligne sur le site ?`, 'Partager'))) return render(true);
              const r = await call('nomadeShare', s.id);
              state.guests = null; // les partages ne proposent que les sources partagées
              if (r && r.ok) toast(`✓ ${s.label} partagé`);
            } else {
              const r = await validated('nomadeDropRequest', s.id);
              state.guests = null;
              if (r && r.ok) toast(`✓ ${s.label} retiré du site`);
            }
          }),
          h('div', { class: 'help', style: 'margin:-2px 0 0 46px' },
            [s.writable ? 'modifiable depuis le site' : 'lecture seule', s.sensitive ? 'sensible' : '', s.shared ? `${s.sent} en ligne` : ''].filter(Boolean).join(' · ')))))) ),
      d.missing && d.missing.length ? h('div', { class: 'help', style: 'color: var(--warn); margin-top:10px' }, `Sources disparues (effacées du site au prochain échange) : ${d.missing.join(', ')}`) : null);
  }

  function depotsPanel(d) {
    if (!d.depots || !d.depots.length) return null;
    return panel('Déposé depuis le site', String(d.depots.length),
      ...d.depots.slice().reverse().map((x) => x.kind === 'nouvelle_liste' ? h('div', { class: 'row', style: 'align-items:flex-start; gap:8px; margin-top:8px; flex-wrap:wrap' },
        h('div', { style: 'flex:1; min-width:200px' },
          h('div', {}, `✚ ${(x.demande || {}).nom || 'Nouvelle liste'}${x.requester && x.requester.role !== 'owner' ? ` · demandée par ${x.requester.nom || x.requester.login} (invité)` : ''}`),
          h('div', { class: 'help' }, (x.demande || {}).type || ''),
          (x.demande || {}).champs && x.demande.champs.length ? h('div', { class: 'help' }, `Champs : ${x.demande.champs.join(', ')}`) : null,
          h('div', { class: 'help' }, `nouvelle liste · ${ago(x.createdAt)}`)),
        h('button', { class: 'btn', onclick: () => call('nomadeDepot', x.id, 'creer_liste') }, 'Créer la liste'),
        h('button', { class: 'btn ghost', onclick: () => call('nomadeDepot', x.id, 'refuser') }, 'Refuser'))
      : h('div', { class: 'row', style: 'align-items:flex-start; gap:8px; margin-top:8px; flex-wrap:wrap' },
        h('div', { style: 'flex:1; min-width:200px' }, h('div', {}, x.text), h('div', { class: 'help' }, `${x.kind} · ${ago(x.createdAt)}`)),
        h('button', { class: 'btn ghost', onclick: () => call('nomadeDepot', x.id, 'note') }, 'En note'),
        h('button', { class: 'btn ghost', onclick: () => call('nomadeDepot', x.id, x.kind === 'rappel' ? 'rappel' : 'demande') }, x.kind === 'rappel' ? 'Créer le rappel' : 'Demander à Jarvis'),
        h('button', { class: 'btn ghost', onclick: () => call('nomadeDepot', x.id, 'ignorer') }, 'Ignorer'))));
  }

  function conflictsPanel(d) {
    if (!d.conflicts || !d.conflicts.length) return null;
    return panel('À vérifier', String(d.conflicts.length),
      h('div', { class: 'help' }, 'Modifications du site appliquées alors que Jarvis avait aussi changé l’élément, ou refusées. « Réessayer » rejoue une modification refusée ; « Ignorer » donne raison à Jarvis (la fiche refusée est retirée du site).'),
      ...d.conflicts.map((c) => h('div', { class: 'row', style: 'align-items:flex-start; gap:8px; margin-top:8px; flex-wrap:wrap' },
        h('div', { style: 'flex:1; min-width:200px' },
          h('div', {}, `${c.titre || c.id}${c.fois > 1 ? ` ×${c.fois}` : ''}`),
          h('div', { class: 'help' }, `${c.sourceLabel || c.source} · ${ago(c.at)} · `,
            c.erreur ? c.erreur : (c.champs || []).map((f) => `${f.champ} « ${f.jarvis} » → « ${f.site} »`).join(', '))),
        c.retry ? h('button', { class: 'btn', onclick: () => conflict('reessayer', c) }, 'Réessayer') : null,
        h('button', { class: 'btn ghost', onclick: () => conflict('ignorer', c) }, c.retry ? 'Ignorer' : 'Vu'))),
      h('div', { class: 'row', style: 'margin-top:10px' },
        h('button', { class: 'btn ghost', onclick: () => conflict('effacer') }, 'Tout effacer')));
  }

  async function conflict(action, c = {}) {
    const r = await call('nomadeConflict', action, c.at || 0, c.id || '');
    if (r && r.ok && r.say) toast(`✓ ${r.say}`);
  }

  function sessionsPanel(d) {
    if (!d.configured) return null;
    const s = state.sessions;
    return panel('Appareils connectés au site', s ? String((s.sessions || []).length) : null,
      !s ? h('button', { class: 'btn ghost', onclick: async () => { state.sessions = await api('nomadeSessions'); render(true); } }, 'Afficher')
        : s.ok === false ? h('div', { class: 'help', style: 'color: var(--err)' }, s.error)
          : h('div', {},
            ...(s.sessions || []).map((x) => h('div', { class: 'row', style: 'gap:8px; margin-top:6px' },
              h('div', { style: 'flex:1' }, h('div', {}, (x.label || 'Navigateur').slice(0, 90)), h('div', { class: 'help' }, `${x.login ? x.login + ' · ' : ''}vu ${ago(+x.last_seen)} · ${x.ip}`)),
              h('button', { class: 'btn ghost', onclick: async () => { state.sessions = await api('nomadeRevoke', +x.id); render(true); } }, 'Révoquer'))),
            (s.sessions || []).length ? null : h('div', { class: 'help' }, 'Aucun navigateur connecté.'),
            h('div', { class: 'help', style: 'margin-top:8px' }, `Jarvis : ${(s.devices || []).map((x) => `${x.name} (vu ${ago(+x.last_seen)})`).join(', ')}`)));
  }

  // --- Partages : comptes invités du site ------------------------------------------------------

  async function loadGuests() {
    if (state.guestsLoading) return;
    state.guestsLoading = true;
    state.guests = await api('nomadeGuests');
    state.guestsLoading = false;
    render(true);
  }

  function inviteResult(res) {
    if (!res || !res.link) return;
    state.lastInvite = { login: res.guest && res.guest.login, link: res.link, mailed: res.mailed, error: res.mailError };
    toast(res.mailed ? '✓ Invitation envoyée par mail' : '⚠ Mail non envoyé : copie le lien', !res.mailed);
  }

  function guestForm(g, sources) {
    const e = state.editing;
    const label = h('input', { type: 'text', value: e.label || '', placeholder: 'Marie' });
    const email = h('input', { type: 'email', value: e.email || '', placeholder: 'marie@exemple.fr', autocomplete: 'off' });
    const login = h('input', { type: 'text', value: e.login || '', placeholder: 'marie', autocomplete: 'off', spellcheck: 'false' });
    const suggest = () => {
      if (e.id || login.dataset.touched) return;
      const base = (email.value.split('@')[0] || label.value).normalize('NFD').replace(/[̀-ͯ]/g, '').toLowerCase().replace(/[^a-z0-9._-]+/g, '.').replace(/^\.+|\.+$/g, '');
      login.value = base.slice(0, 40);
    };
    label.addEventListener('input', suggest);
    email.addEventListener('input', suggest);
    login.addEventListener('input', () => { login.dataset.touched = '1'; });
    const mode = h('select', {}, h('option', { value: 'lecture', selected: e.mode !== 'ecriture' }, 'Lecture seule'), h('option', { value: 'ecriture', selected: e.mode === 'ecriture' }, 'Lecture et écriture'));
    const relation = h('select', {}, ...Object.entries(RELATIONS).map(([v, l]) => h('option', { value: v, selected: (e.relation || 'ami') === v }, l)));
    const picked = new Set(e.sources || []);
    const boxes = sources.map((s) => toggle(s.label + (s.writable ? '' : ' (lecture seule)'), picked.has(s.id), (v) => { if (v) picked.add(s.id); else picked.delete(s.id); }));
    return panel(e.id ? `Modifier le partage — ${e.login}` : 'Nouveau partage', null,
      h('div', { class: 'grid' },
        field('Nom affiché', label, 'Utilisé dans le mail (« Bonjour Marie »).'),
        field('Adresse mail', email, 'L’invitation y est envoyée par le Gmail de Jarvis.'),
        field('Identifiant', login, 'Pré-rempli sur le site ; la personne choisit son mot de passe à sa première visite.'),
        field('Droits', mode, 'Écriture : ajouter et modifier dans les sources modifiables (jamais la boîte de dépôt).'),
        field('Lien avec cette personne', relation, 'Ses propres listes entrent dans la mémoire de Jarvis dans cette catégorie.')),
      h('div', { class: 'help', style: 'margin-top:10px' }, sources.length ? 'Données visibles par cette personne (parmi celles que Jarvis partage) :' : 'Jarvis ne partage encore aucune donnée : coche-les dans « Données partagées ».'),
      h('div', { class: 'grid' }, ...boxes),
      h('div', { class: 'row', style: 'margin-top:10px' },
        h('button', { class: 'btn primary', disabled: state.busy, onclick: async () => {
          const res = await validated('nomadeGuestRequest', { id: e.id || 0, label: label.value, email: email.value, login: login.value, mode: mode.value, relation: relation.value, sources: [...picked] });
          if (res && res.ok) {
            state.editing = null;
            inviteResult(res);
            if (!res.link) toast('✓ Partage modifié');
            loadGuests();
          }
        } }, e.id ? 'Enregistrer' : 'Créer et envoyer l’invitation'),
        h('button', { class: 'btn ghost', onclick: () => { state.editing = null; render(true); } }, 'Annuler')));
  }

  const RELATIONS = { famille: 'Famille', ami: 'Ami·e', collegue: 'Collègue', autre: 'Autre' };

  // Listes tenues par les invités sur le site : reprises dans la mémoire de Jarvis.
  function friendsPanel(d) {
    const f = (d.friends || []).filter((x) => x.lists && x.lists.length);
    if (!d.configured || !f.length) return null;
    return panel('Listes des invités', String(f.reduce((n, x) => n + x.lists.length, 0)),
      h('div', { class: 'help' }, 'Listes créées par les personnes invitées. Jarvis les relit à chaque changement et les range dans sa mémoire (Mémoire (RAG), catégories famille, amis…) : « qu’est-ce que Marie veut pour Noël ? ».'),
      ...f.map((x) => h('div', { class: 'help', style: 'margin-top:8px' },
        h('strong', {}, `${x.name} (${(RELATIONS[x.relation] || 'Ami·e').toLowerCase()}) : `),
        x.lists.map((l) => `${l.title} (${l.items})`).join(', '))));
  }

  function guestsPanel(d) {
    if (!d.configured) return [];
    const g = state.guests;
    if (!g && !state.guestsLoading) loadGuests();
    const sources = (g && g.sources) || [];
    const names = Object.fromEntries(sources.map((s) => [s.id, s.label]));
    const inv = state.lastInvite;
    return [
      panel('Partages', g && g.guests ? String(g.guests.length) : null,
        h('div', { class: 'help' }, 'Donne accès à une partie des données à une autre personne : elle reçoit un mail avec l’adresse du site, son identifiant et un lien pour choisir son mot de passe (enregistré chiffré). Chaque partage a ses propres données et ses droits ; le supprimer coupe l’accès immédiatement.'),
        inv ? h('div', { class: 'help', style: `margin-top:10px; color: var(${inv.mailed ? '--ok' : '--warn'})` },
          inv.mailed ? `Invitation envoyée à ${inv.login}. Lien (valable 7 jours) : ` : `Mail non envoyé (${inv.error || 'erreur'}). Transmets ce lien à ${inv.login} (valable 7 jours) : `,
          h('input', { type: 'text', readonly: true, value: inv.link, style: 'width:100%; margin-top:4px', onclick: (ev) => ev.target.select() })) : null,
        !g ? h('div', { class: 'help' }, 'Chargement…')
          : g.ok === false ? h('div', { class: 'help', style: 'color: var(--err)' }, g.error)
            : h('div', {},
              ...(g.guests || []).map((x) => h('div', { class: 'row', style: 'gap:8px; margin-top:10px; align-items:flex-start; flex-wrap:wrap' },
                h('div', { style: 'flex:1; min-width:220px' },
                  h('div', {}, `${x.label || x.login} · ${x.login}`, x.email ? h('span', { class: 'help' }, ` · ${x.email}`) : null),
                  h('div', { class: 'help' }, [RELATIONS[x.relation] || 'Ami·e', x.mode === 'ecriture' ? 'lecture et écriture' : 'lecture seule', x.lists ? `${x.lists} liste(s) à elle` : '',
                    x.active ? (x.lastSeen ? `vu ${ago(x.lastSeen)}` : 'accès créé') : (x.invitePending && x.inviteExpires > Date.now() ? 'invitation en attente' : 'invitation expirée')].filter(Boolean).join(' · ')),
                  h('div', { class: 'help' }, x.sources.length ? x.sources.map((id) => names[id] || id).join(', ') : 'aucune donnée')),
                h('button', { class: 'btn ghost', onclick: () => { state.editing = { ...x }; render(true); } }, 'Modifier'),
                h('button', { class: 'btn ghost', disabled: state.busy, onclick: async () => { const r = await call('nomadeGuestInvite', x.id); inviteResult(r); loadGuests(); } }, x.active ? 'Nouveau lien (mot de passe oublié)' : 'Renvoyer l’invitation'),
                h('button', { class: 'btn ghost', onclick: async () => {
                  if (await confirmDialog('Supprimer le partage', `${x.label || x.login} n’aura plus accès au site (connexions coupées tout de suite).`, 'Supprimer', true)) {
                    const r = await call('nomadeGuestDelete', x.id);
                    if (r && r.ok) { toast('✓ Partage supprimé'); loadGuests(); }
                  }
                } }, 'Supprimer'))),
              (g.guests || []).length ? null : h('div', { class: 'help', style: 'margin-top:8px' }, 'Aucun partage.'),
              state.editing ? null : h('div', { class: 'row', style: 'margin-top:12px' },
                h('button', { class: 'btn primary', onclick: () => { state.editing = { mode: 'lecture', sources: [] }; render(true); } }, '+ Nouveau partage')))),
      state.editing && g && g.ok !== false ? guestForm(g, sources) : null,
    ];
  }

  sections.nomade = () => {
    if (!state.data && !state.loading) load();
    const d = state.data;
    const intro = [h('h1', {}, 'Nomade'),
      h('p', { class: 'lead' }, 'Tes listes, notes et rappels consultables depuis le téléphone ou un autre PC, même quand celui-ci est éteint. Le site est hébergé chez Hostinger (dossier web-nomade) ; Jarvis lui envoie ce que tu coches ici et reprend ce que tu y ajoutes. Aucun port n’est ouvert sur ce PC.')];
    if (!d) return [...intro, h('div', { class: 'help' }, 'Chargement…')];
    return [...intro, connectionPanel(d), d.configured ? syncPanel(d) : null, versionPanel(d), depotsPanel(d), sourcesPanel(d), ...guestsPanel(d), friendsPanel(d), conflictsPanel(d), sessionsPanel(d)];
  };
})();
