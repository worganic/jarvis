// Section « Connecteurs MCP » de l'administration (chantier v3, étape 6).
// Ajouter, régler, tester et activer des serveurs MCP externes. Les outils découverts au test
// apparaissent dans la section Sécurité, avec un niveau par défaut prudent.
(function () {
  const A = window.JarvisAdmin;
  if (!A) return;
  const { h, panel, field, api, sections, render, toast, confirmDialog, textInput, selectSection } = A;

  const state = { data: null, loading: false, adding: null, editing: null, busy: new Set(), openTools: new Set(), lastAuth: {} };
  const WIDGETS = { '': 'Aucun', music: 'Musique', workspace: 'Mail / Agenda / Drive', browser: 'Navigateur' };

  async function load() {
    if (state.loading) return;
    state.loading = true;
    const res = await api('mcpState');
    state.loading = false;
    if (res && res.ok) state.data = res;
    render(true);
  }

  const previous = A.onJournal;
  A.onJournal = (ev) => {
    if (previous) previous(ev);
    if (ev && ev.type === 'mcp' && document.getElementById('section-mcp').classList.contains('active')) load();
  };

  // Découpe une ligne d'arguments en respectant les guillemets.
  function parseArgs(line) {
    const out = [];
    const re = /"([^"]*)"|(\S+)/g;
    let m;
    while ((m = re.exec(String(line || '')))) out.push(m[1] !== undefined ? m[1] : m[2]);
    return out;
  }

  async function run(id, method, ...args) {
    state.busy.add(id);
    render(true);
    const res = await api(method, ...args);
    state.busy.delete(id);
    if (res && res.ok) state.data = res;
    render(true);
    return res;
  }

  function envFields(preset, current = {}) {
    const inputs = {};
    const defs = (preset && preset.env) || [];
    const rows = defs.map((def) => {
      const cur = current[def.key];
      const input = h('input', {
        type: def.secret ? 'password' : 'text',
        value: cur ? cur.value : (def.default || ''),
        placeholder: def.secret && cur && cur.value ? 'inchangé' : def.key,
        autocomplete: 'off'
      });
      inputs[def.key] = input;
      return field(`${def.label}${def.required ? ' *' : ''}`, input, def.key);
    });
    return { rows, read: () => Object.fromEntries(Object.entries(inputs).map(([k, el]) => [k, el.value.trim()])) };
  }

  // Éditeur libre de variables (connecteur personnalisé) : une variable par ligne, CLÉ=valeur.
  function envEditor(current = {}) {
    const text = Object.entries(current).map(([k, v]) => `${k}=${v.secret ? '' : v.value}`).join('\n');
    const area = h('textarea', { placeholder: 'CLE=valeur (une par ligne ; laisser vide une valeur secrète pour la garder)', style: 'min-height:70px' }, text);
    return {
      rows: [field('Variables d’environnement', area, 'Les clés contenant secret, token, key ou password sont chiffrées.', true)],
      read: () => {
        const env = {};
        for (const line of area.value.split('\n')) {
          const m = /^\s*([A-Za-z_][A-Za-z0-9_]*)\s*=\s*(.*)$/.exec(line);
          if (!m) continue;
          const cur = current[m[1]];
          env[m[1]] = m[2] === '' && cur && cur.secret ? '••••••••' : m[2];
        }
        return env;
      }
    };
  }

  function addForm(preset) {
    const name = textInput(preset.name, { placeholder: 'nom affiché' });
    const env = preset.id === 'custom' ? envEditor({}) : envFields(preset);
    let transport = 'stdio';
    const command = textInput('', { placeholder: 'ex. npx, uvx, node, C:\\outils\\serveur.exe' });
    const args = textInput('', { placeholder: 'ex. -y mon-serveur-mcp@latest' });
    const url = textInput('', { placeholder: 'https://exemple.fr/mcp' });
    let widget = '';
    const params = {};
    const paramRows = (preset.params || []).map((p) => {
      params[p.key] = textInput('', { placeholder: p.label });
      return field(`${p.label}${p.required ? ' *' : ''}`, params[p.key]);
    });
    const custom = preset.id === 'custom' ? [
      field('Transport', A.select({ stdio: 'Commande locale (stdio)', http: 'Adresse HTTP' }, transport, (v) => { transport = v; })),
      field('Commande', command), field('Arguments', args), field('Adresse (HTTP)', url),
      field('Widget', A.select(WIDGETS, widget, (v) => { widget = v; }))
    ] : [];
    return panel(`Nouveau connecteur : ${preset.name}`, null,
      preset.setup && preset.setup.length ? h('ol', { class: 'setup' }, ...preset.setup.map((s) => h('li', {}, s))) : null,
      preset.link ? h('div', { class: 'help', style: 'margin-bottom:12px' }, 'Adresse : ', h('span', { class: 'mono' }, preset.link)) : null,
      h('div', { class: 'grid' }, field('Nom', name), ...paramRows, ...custom, ...env.rows),
      h('div', { class: 'row end', style: 'margin-top:14px' },
        h('button', { class: 'btn ghost', onclick: () => { state.adding = null; render(true); } }, 'Annuler'),
        h('button', {
          class: 'btn primary',
          onclick: async () => {
            const def = {
              preset: preset.id,
              name: name.value.trim(),
              env: env.read(),
              params: Object.fromEntries(Object.entries(params).map(([k, el]) => [k, el.value.trim()]))
            };
            if (preset.id === 'custom') {
              Object.assign(def, { transport, command: command.value.trim(), args: parseArgs(args.value), url: url.value.trim(), widget: widget || null });
            }
            const res = await api('mcpAdd', def);
            if (res && res.ok) {
              state.data = res;
              state.adding = null;
              toast('✓ connecteur ajouté — clique sur « Tester » pour découvrir ses outils');
              render(true);
            }
          }
        }, 'Ajouter')));
  }

  function serverCard(s) {
    const d = state.data;
    const preset = d.presets.find((p) => p.id === s.preset) || d.presets.find((p) => p.id === 'custom');
    const busy = state.busy.has(s.id);
    const statusBadge = s.missing && s.missing.length
      ? h('span', { class: 'badge warn' }, `à compléter : ${s.missing.join(', ')}`)
      : s.lastError ? h('span', { class: 'badge err' }, `erreur : ${s.lastError}`)
        : s.tools.length ? h('span', { class: 'badge ok' }, `${s.tools.length} outils`)
          : h('span', { class: 'badge off' }, 'jamais testé');
    const editing = state.editing === s.id;
    let editor = null;
    if (editing) {
      const env = s.preset === 'custom' ? envEditor(s.env) : envFields(preset, s.env);
      const name = textInput(s.name);
      let widget = s.widget || '';
      editor = h('div', { class: 'editor' },
        h('div', { class: 'grid' }, field('Nom', name), field('Widget', A.select(WIDGETS, widget, (v) => { widget = v; })), ...env.rows),
        h('div', { class: 'row end', style: 'margin-top:12px' },
          h('button', { class: 'btn ghost', onclick: () => { state.editing = null; render(true); } }, 'Annuler'),
          h('button', {
            class: 'btn primary',
            onclick: async () => {
              const res = await api('mcpUpdate', s.id, { name: name.value.trim(), widget: widget || null, env: env.read() });
              if (res && res.ok) { state.data = res; state.editing = null; toast('✓ enregistré'); render(true); }
            }
          }, 'Enregistrer')));
    }
    const auth = state.lastAuth[s.id];
    return h('div', { class: `panel mcp-card${s.enabled ? '' : ' disabled'}` },
      h('div', { class: 'row' },
        h('div', {},
          h('div', { class: 'mcp-name' }, s.name, h('span', { class: 'cat', style: 'margin-left:10px' }, preset ? preset.name : s.preset)),
          h('div', { class: 'help mono' }, s.transport === 'http' ? s.url : `${s.command} ${s.args.join(' ')}`)),
        h('div', { class: 'spacer' }),
        A.toggle(s.enabled ? 'Activé' : 'Désactivé', s.enabled, (v) => run(s.id, 'mcpToggle', s.id, v))),
      h('div', { class: 'row', style: 'margin-top:10px' }, statusBadge,
        s.testedAt ? h('span', { class: 'help' }, `testé ${A.fmtDate(s.testedAt)}`) : null,
        s.widget ? h('span', { class: 'help' }, `widget : ${WIDGETS[s.widget] || s.widget}`) : null,
        s.agy === false ? h('span', { class: 'badge warn', title: s.agyNote || '' }, 'Claude Code seulement') : null,
        h('div', { class: 'spacer' }),
        h('button', { class: 'btn', disabled: busy, onclick: () => run(s.id, 'mcpTest', s.id).then((r) => { if (r && r.test) toast(r.test.ok ? `✓ ${r.test.tools} outils découverts en ${(r.test.ms / 1000).toFixed(1)} s` : `✕ ${r.test.error}`, !r.test.ok); }) }, busy ? '… en cours' : '▶ Tester'),
        s.hasAuth ? h('button', {
          class: 'btn', disabled: busy,
          onclick: async () => {
            toast('Le navigateur va s’ouvrir : termine la connexion puis reviens ici.');
            const r = await run(s.id, 'mcpAuth', s.id);
            if (r && r.auth) { state.lastAuth[s.id] = r.auth; toast(r.auth.ok ? '✓ connecté' : `✕ ${r.auth.error || 'échec'}`, !r.auth.ok); render(true); }
          }
        }, s.authLabel || 'Se connecter') : null,
        h('button', { class: 'btn ghost', onclick: () => { state.editing = editing ? null : s.id; render(true); } }, 'Modifier'),
        h('button', { class: 'btn ghost', onclick: async () => { if (await confirmDialog('RETIRER LE CONNECTEUR', `Retirer « ${s.name} » ? Jarvis n’aura plus accès à ses outils.`, 'Retirer', true)) run(s.id, 'mcpRemove', s.id); } }, 'Retirer')),
      editor,
      auth && auth.output ? h('div', { class: 'mono-block', style: 'margin-top:10px;max-height:160px;overflow:auto' }, auth.output) : null,
      s.tools.length ? h('div', { style: 'margin-top:10px' },
        h('button', { class: 'btn ghost', onclick: () => { if (state.openTools.has(s.id)) state.openTools.delete(s.id); else state.openTools.add(s.id); render(true); } },
          state.openTools.has(s.id) ? '▾ Masquer les outils' : `▸ Voir les ${s.tools.length} outils`),
        state.openTools.has(s.id) ? h('div', {},
          h('div', { class: 'chips', style: 'margin:8px 0' }, ...s.tools.map((t) => h('span', { class: 'chip passive', title: t.description }, t.readOnly ? '◇ ' : '◆ ', t.name))),
          h('div', { class: 'help' }, '◇ lecture seule : accord direct par défaut · ◆ action : code à 4 chiffres par défaut. ',
            h('a', { href: '#', onclick: (e) => { e.preventDefault(); selectSection('security'); } }, 'Régler dans Sécurité →'))) : null) : null);
  }

  // Gmail intégré : adresse + mot de passe d'application (IMAP/SMTP), vérifié avant d'être gardé.
  // Lien d'aide ouvert dans le navigateur (liste fermée côté noyau : admin.js › openHelpLink).
  const helpLink = (key, label) => h('a', { href: '#', onclick: (e) => { e.preventDefault(); api('openHelpLink', key); } }, label);

  function gmailPanel(d) {
    const g = d.gmail || {};
    const email = textInput(g.email || '', { placeholder: 'ton.adresse@gmail.com', autocomplete: 'off' });
    // autocomplete « new-password » : la fenêtre ne propose ni ne remplit un mot de passe enregistré
    // (le mot de passe Google habituel, refusé par Gmail).
    const pass = h('input', { type: 'password', name: 'jarvis-gmail-app-password', placeholder: g.configured ? 'inchangé (laisser vide)' : 'abcd efgh ijkl mnop', autocomplete: 'new-password', spellcheck: 'false' });
    // Compteur : 16 lettres attendues (espaces ignorés), vérifié avant tout envoi à Google.
    const passInfo = h('div', { class: 'help' }, g.configured ? 'Laisse vide pour garder celui qui est enregistré.' : '16 lettres attendues (les espaces ne comptent pas).');
    pass.addEventListener('input', () => {
      const clean = pass.value.replace(/[\s\u00a0\u200b-\u200d\u2060\ufeff-]+/g, '');
      if (!clean) { passInfo.textContent = g.configured ? 'Laisse vide pour garder celui qui est enregistré.' : '16 lettres attendues (les espaces ne comptent pas).'; passInfo.style.color = ''; return; }
      const good = /^[a-z]{16}$/i.test(clean);
      passInfo.textContent = good ? '✓ 16 lettres : format d’un mot de passe d’application.'
        : `${clean.length} caractère${clean.length > 1 ? 's' : ''}${/[^a-z]/i.test(clean) ? ', dont des chiffres ou symboles' : ''} : ce n’est pas un mot de passe d’application (16 lettres).`;
      passInfo.style.color = good ? 'var(--ok)' : 'var(--warn)';
    });
    const result = state.gmailResult;
    // Jarvis a ouvert cette page parce qu'il lui manquait des informations (demande « lis mes
    // mails ») : le panneau est mis en avant, et le premier champ à remplir prend le focus.
    const wanted = A.focus && A.focus.target === 'gmail';
    const refused = result && !result.ok && result.auth;
    if (wanted && state.focusDone !== A.focus.at) {
      state.focusDone = A.focus.at;
      setTimeout(() => {
        const box = document.getElementById('panel-gmail');
        if (box) box.scrollIntoView({ block: 'center', behavior: 'smooth' });
        const target = document.querySelector(g.email ? '#panel-gmail input[type=password]' : '#panel-gmail input');
        if (target) target.focus();
      }, 60);
    }
    const node = panel('Gmail intégré', g.configured ? `connecté : ${g.masked}` : 'non configuré',
      wanted ? h('div', { class: 'setup-callout' },
        h('strong', {}, '◈ Jarvis a besoin de ces informations'),
        h('div', {}, g.configured
          ? 'Gmail a refusé le mot de passe enregistré : colle un nouveau mot de passe d’application, puis « Vérifier et enregistrer ».'
          : `Pour lire tes mails, il manque ${g.email ? 'ton mot de passe d’application Google' : 'ton adresse Gmail et ton mot de passe d’application Google'}.`)) : null,
      h('p', { class: 'hint' }, 'Le plus simple pour que Jarvis lise, cherche et envoie tes emails. Google exige un « mot de passe d’application » (pas ton mot de passe habituel).'),
      g.configured && !wanted && !refused ? null : h('ol', { class: 'setup' },
        h('li', {}, 'Active la validation en deux étapes de ton compte Google : ', helpLink('google_two_step', 'ouvrir la page ↗')),
        h('li', {}, 'Crée un mot de passe d’application (nom : Jarvis) : ', helpLink('google_app_passwords', 'ouvrir la page ↗')),
        h('li', {}, 'Colle ici ton adresse et les 16 lettres affichées par Google (sans les espaces, ou avec : peu importe), puis « Vérifier et enregistrer ».')),
      h('div', { class: 'grid' },
        field('Adresse Gmail', email),
        field('Mot de passe d’application', h('div', {}, pass, passInfo), g.encrypted ? 'Chiffré par Windows, jamais transmis aux modèles.' : 'Gardé dans le dossier de données protégé.')),
      h('div', { class: 'row', style: 'margin-top:12px' },
        h('button', {
          class: 'btn primary', disabled: state.busy.has('gmail'),
          onclick: async () => {
            state.busy.add('gmail');
            render(true);
            const res = await api('gmailConfigure', email.value.trim(), pass.value);
            state.busy.delete('gmail');
            state.gmailResult = res;
            if (res && res.ok) {
              toast(`✓ Gmail connecté : ${res.count} email(s) lus. Tu peux redemander à Jarvis de lire tes mails.`);
              if (A.focus && A.focus.target === 'gmail') A.focus = null; // plus rien ne manque
              await load();
            }
            render(true);
          }
        }, state.busy.has('gmail') ? '… vérification' : 'Vérifier et enregistrer'),
        g.configured ? h('button', {
          class: 'btn', onclick: async () => {
            const res = await api('gmailTest');
            state.gmailResult = res;
            if (res && res.ok) toast(`✓ ${res.count} email(s) lus`);
            render(true);
          }
        }, '▶ Tester') : null,
        g.configured ? h('button', {
          class: 'btn ghost', onclick: async () => {
            if (await confirmDialog('RETIRER L’ACCÈS GMAIL', 'Jarvis ne pourra plus lire ni envoyer d’emails.', 'Retirer', true)) {
              await api('gmailForget');
              state.gmailResult = null;
              load();
            }
          }
        }, 'Retirer l’accès') : null),
      result && result.ok && result.sample && result.sample.length
        ? h('div', { style: 'margin-top:12px' }, h('div', { class: 'help' }, 'Derniers emails reçus :'),
          ...result.sample.map((m) => h('div', { class: 'w-line' }, `• ${m.subject || '(sans objet)'} — ${m.from || ''}`)))
        : null,
      result && !result.ok ? h('div', { class: 'help', style: 'color: var(--err); margin-top:10px' }, result.error) : null);
    node.id = 'panel-gmail';
    if (wanted) node.classList.add('needs-setup');
    return node;
  }

  // Google Docs : comptes rendus de réunion (orchestrator/google-docs.js). ID client OAuth
  // « application de bureau », puis consentement dans le navigateur ; accès limité aux fichiers
  // créés par Jarvis (drive.file).
  function gdocsPanel(d) {
    const g = d.gdocs || {};
    const id = textInput(g.clientId || '', { placeholder: '…apps.googleusercontent.com (ou colle le JSON téléchargé)', autocomplete: 'off' });
    const secret = h('input', { type: 'password', placeholder: g.hasClient ? 'inchangé (laisser vide)' : 'GOCSPX-…', autocomplete: 'new-password', spellcheck: 'false' });
    const result = state.gdocsResult;
    const wanted = A.focus && A.focus.target === 'gdocs';
    if (wanted && state.focusDone !== A.focus.at) {
      state.focusDone = A.focus.at;
      setTimeout(() => {
        const box = document.getElementById('panel-gdocs');
        if (box) box.scrollIntoView({ block: 'center', behavior: 'smooth' });
      }, 60);
    }
    const busy = state.busy.has('gdocs');
    const connect = async () => {
      state.busy.add('gdocs');
      render(true);
      if (id.value.trim() && (id.value.trim() !== g.clientId || secret.value)) {
        const saved = await api('gdocsConfigure', id.value.trim(), secret.value);
        if (!saved || !saved.ok) { state.busy.delete('gdocs'); state.gdocsResult = saved; render(true); return; }
      }
      toast('Google s’ouvre dans ton navigateur : choisis ton compte et accepte, puis reviens ici.');
      const res = await api('gdocsConnect');
      state.busy.delete('gdocs');
      state.gdocsResult = res;
      if (res && res.ok) {
        toast(`✓ Google Docs relié${res.email ? ` (${res.email})` : ''}`);
        if (A.focus && A.focus.target === 'gdocs') A.focus = null;
        await load();
      }
      render(true);
    };
    const node = panel('Google Docs (comptes rendus de réunion)', g.connected ? `relié${g.email ? ` : ${g.email}` : ''}` : 'non relié',
      wanted ? h('div', { class: 'setup-callout' },
        h('strong', {}, '◈ Jarvis a besoin de cet accès'),
        h('div', {}, 'Pour mettre une réunion dans Google Docs, relie Jarvis à ton compte Google (une seule fois).')) : null,
      h('p', { class: 'hint' }, 'Dis « Jarvis, mets la réunion dans Google Docs » ou clique sur « Google Doc » dans la card de réunion : résumé et transcription arrivent dans un vrai Google Doc, rangé dans le dossier « Réunions Jarvis ». Jarvis ne voit que les fichiers qu’il crée, jamais le reste de ton Drive.'),
      g.connected && !wanted ? null : h('ol', { class: 'setup' },
        h('li', {}, 'Dans Google Cloud Console, crée un projet (nom : Jarvis) et active l’API Google Drive : ', helpLink('google_drive_api', 'ouvrir la page ↗')),
        h('li', {}, 'Écran de consentement OAuth : type « Externe », ton adresse comme utilisateur test, puis « Publier l’application » (sinon l’accès expire au bout de 7 jours) : ', helpLink('google_consent', 'ouvrir la page ↗')),
        h('li', {}, 'Identifiants → « Créer des identifiants » → « ID client OAuth » → type « Application de bureau » : ', helpLink('google_credentials', 'ouvrir la page ↗')),
        h('li', {}, 'Colle l’ID client et le code secret (ou tout le fichier JSON téléchargé), puis « Enregistrer et relier » : Google s’ouvre, accepte l’accès.')),
      h('div', { class: 'grid' },
        field('ID client OAuth', id),
        field('Code secret du client', secret, g.encrypted ? 'Chiffré par Windows (DPAPI), jamais transmis aux modèles.' : 'Gardé dans le dossier de données protégé.')),
      h('div', { class: 'row', style: 'margin-top:12px' },
        h('button', { class: 'btn primary', disabled: busy, onclick: connect }, busy ? '… en attente de Google' : g.connected ? 'Relier de nouveau' : 'Enregistrer et relier'),
        g.hasClient || g.connected ? h('button', {
          class: 'btn ghost', onclick: async () => {
            if (await confirmDialog('RETIRER L’ACCÈS GOOGLE DOCS', 'Jarvis ne pourra plus créer de Google Docs. Les documents déjà créés restent dans ton Drive.', 'Retirer', true)) {
              await api('gdocsForget');
              state.gdocsResult = null;
              load();
            }
          }
        }, 'Retirer l’accès') : null),
      result && !result.ok ? h('div', { class: 'help', style: 'color: var(--err); margin-top:10px' }, result.error) : null);
    node.id = 'panel-gdocs';
    if (wanted) node.classList.add('needs-setup');
    return node;
  }

  // Spotify intégré : identifiants d'application (recherche dans le catalogue), vérifiés avant
  // d'être gardés. La lecture, elle, passe par l'application Spotify du PC : sur un compte gratuit,
  // Spotify réserve le pilotage à distance aux abonnés Premium.
  function spotifyPanel(d) {
    const sp = d.spotify || {};
    const id = textInput(sp.configured ? sp.clientId : '', { placeholder: '32 caractères, visibles sur le tableau de bord', autocomplete: 'off' });
    const secret = h('input', { type: 'password', placeholder: sp.configured ? 'inchangé (laisser vide)' : 'client secret', autocomplete: 'off' });
    const result = state.spotifyResult;
    const isDirectActive = Boolean(sp.directModeActive || sp.mode === 'direct');
    const tag = isDirectActive
      ? 'Solution A active (contrôle direct)'
      : (sp.configured ? 'API Web active' : 'facultatif');

    return panel('Spotify intégré', tag,
      h('p', { class: 'hint' }, 'Dis « mets du Daft Punk sur Spotify » ou « lance la playlist bonne nuit sur Spotify » : Jarvis interagit avec Spotify pour lancer la lecture ou ouvrir la recherche.'),

      // Bannière Solution A active
      isDirectActive ? h('div', {
        class: 'help',
        style: 'color: var(--ok); background: rgba(30, 215, 96, 0.1); border-left: 3px solid #1db954; padding: 8px 12px; margin-bottom: 12px; border-radius: 4px; line-height: 1.5'
      },
        h('strong', {}, '✓ Solution A active (Contrôle direct Windows) : '),
        'Jarvis pilote directement l’application Spotify locale de votre PC. Vous pouvez demander n’importe quel artiste, titre ou playlist par la voix (« mets du Daft Punk sur Spotify », « mets la musique », « pause », « suivant »). Aucune clé API ni carte bancaire n’est nécessaire !'
      ) : null,

      // Explication claire des règles 2026 et du statut
      h('div', { class: 'help', style: 'color: var(--warn); margin-bottom: 12px; line-height: 1.5' },
        '⚠ Information API Spotify (2026) :', h('br', {}),
        '• Pour utiliser l’API officielle : un compte ', h('strong', {}, 'Spotify Premium'), ' est obligatoire et votre email doit être dans ', h('strong', {}, '« User Management »'), ' du dashboard Spotify (sinon erreur 403 Forbidden).', h('br', {}),
        '• ', h('strong', {}, 'Solution A automatique : '), 'Si vous saisissez des clés qui renvoient une erreur 403, Jarvis active automatiquement la Solution A pour que tout fonctionne immédiatement sans blocage.'),

      sp.configured ? null : h('ol', { class: 'setup' },
        h('li', {}, 'Ouvre developer.spotify.com/dashboard (compte Spotify Premium requis pour l’accès API Web).'),
        h('li', {}, '« Create app » : donne un nom (Jarvis), coche la case « Web API », et indique http://127.0.0.1:8888/callback comme Redirect URI.'),
        h('li', {}, 'Dans Settings → « User Management », ajoute impérativement le nom et l’adresse email de ton compte Spotify.'),
        h('li', {}, 'Dans Settings, copie le Client ID, puis « View client secret » pour le secret. Colle les deux ci-dessous.')),

      h('div', { class: 'grid' },
        field('Client ID', id),
        field('Client secret', secret, sp.encrypted ? 'Chiffré par Windows (DPAPI), jamais transmis aux modèles.' : 'Gardé dans le dossier de données protégé.')),

      h('div', { class: 'row', style: 'margin-top:12px; flex-wrap: wrap; gap: 8px' },
        h('button', {
          class: 'btn primary', disabled: state.busy.has('spotify'),
          onclick: async () => {
            state.busy.add('spotify');
            render(true);
            const res = await api('spotifyConfigure', id.value.trim(), secret.value);
            state.busy.delete('spotify');
            state.spotifyResult = res;
            if (res && res.ok) {
              if (res.autoFallback) {
                toast('✓ Erreur 403 évitée : Solution A activée automatiquement !');
              } else {
                toast('✓ Spotify connecté');
              }
              await load();
            }
            render(true);
          }
        }, state.busy.has('spotify') ? '… vérification' : 'Vérifier et enregistrer'),

        h('button', {
          class: 'btn', onclick: async () => {
            const res = await api('spotifyActivateDirectMode');
            state.spotifyResult = res;
            toast('✓ Solution A activée (contrôle direct)');
            await load();
            render(true);
          }
        }, '⚡ Activer la Solution A (Direct sans clé)'),

        sp.configured && sp.mode === 'api' ? h('button', {
          class: 'btn', onclick: async () => {
            const res = await api('spotifyTest');
            state.spotifyResult = res;
            if (res && res.ok) toast('✓ Spotify répond');
            render(true);
          }
        }, '▶ Tester l’API') : null,

        h('button', {
          class: 'btn ghost', onclick: async () => {
            const res = await api('spotifyTestLocal');
            state.spotifyResult = res;
            if (res && res.ok) toast('✓ Application locale prête');
            render(true);
          }
        }, '🔍 Détecter Spotify sur ce PC'),

        sp.configured ? h('button', {
          class: 'btn ghost', onclick: async () => {
            if (await confirmDialog('RETIRER L’ACCÈS SPOTIFY', 'Jarvis ne pourra plus chercher via l’API ; le pilotage direct de l’application locale restera actif.', 'Retirer', true)) {
              await api('spotifyForget');
              state.spotifyResult = null;
              load();
            }
          }
        }, 'Retirer les clés') : null),

      // Erreur ou résultat avec option de forçage en cas de 403
      result && !result.ok ? h('div', { class: 'help', style: 'color: var(--err); margin-top:10px; line-height: 1.4' },
        h('strong', {}, 'Échec de vérification : '), result.error || result.message,
        result.canForce ? h('div', { style: 'margin-top: 8px' },
          h('button', {
            class: 'btn small',
            onclick: async () => {
              state.busy.add('spotify');
              render(true);
              const res = await api('spotifyConfigure', id.value.trim(), secret.value, { force: true });
              state.busy.delete('spotify');
              state.spotifyResult = res;
              if (res && res.ok) {
                toast('Clés Spotify enregistrées (Solution A activée)');
                await load();
              }
              render(true);
            }
          }, 'Enregistrer quand même ces clés (avec Solution A)')) : null) : null,

      result && result.ok && result.message ? h('div', { class: 'help', style: 'color: var(--ok); margin-top:10px' }, result.message) : null,
      result && result.warning ? h('div', { class: 'help', style: 'color: var(--warn); margin-top:10px' }, `Enregistré : ${result.warning}`) : null,

      sp.installed
        ? h('div', { class: 'help', style: 'color: var(--ok); margin-top:10px' }, '✓ Application Spotify détectée sur ce PC : prête pour la lecture et le contrôle vocal.')
        : h('div', { class: 'help', style: 'color: var(--warn); margin-top:10px' }, 'L’application Spotify n’a pas été trouvée sur ce PC. Pour écouter la musique, installe l’application Spotify Windows.')
    );
  }

  sections.mcp = () => {
    if (!state.data && !state.loading) load();
    const d = state.data;
    if (!d) return [h('h1', {}, 'Connecteurs MCP'), h('div', { class: 'empty' }, 'Chargement…')];
    const adding = state.adding ? d.presets.find((p) => p.id === state.adding) : null;
    return [
      h('h1', {}, 'Connecteurs MCP'),
      h('p', { class: 'lead' }, 'Relie Jarvis à des outils externes : Spotify, Gmail, Agenda, Drive, Chrome… Claude Code utilise leurs outils ; chaque appel passe par la section Sécurité (code à 4 chiffres par défaut pour toute action qui modifie quelque chose).'),
      gmailPanel(d),
      gdocsPanel(d),
      spotifyPanel(d),
      d.encryption ? null : h('div', { class: 'panel warn-panel' }, h('h2', {}, '⚠ Chiffrement indisponible'),
        h('div', {}, 'Les secrets sont gardés dans le dossier de données protégé, mais en clair.')),
      adding ? addForm(adding) : panel('Ajouter un connecteur', null,
        h('div', { class: 'preset-grid' }, ...d.presets.map((p) => h('div', { class: 'preset', onclick: () => { state.adding = p.id; render(true); } },
          h('div', { class: 'preset-name' }, p.name),
          h('div', { class: 'help' }, p.description),
          p.agy === false ? h('div', { class: 'help', style: 'color: var(--warn); margin-top:6px' }, p.agyNote) : null)))),
      d.servers.length ? h('div', {}, ...d.servers.map(serverCard))
        : panel('Connecteurs branchés', '0', h('div', { class: 'empty' }, 'Aucun connecteur pour l’instant.'))
    ];
  };
})();
