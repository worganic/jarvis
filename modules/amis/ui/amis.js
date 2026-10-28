// Module Amis : la card de Jarvis Réseau (crates/jarvis-core/src/orch/reseau.rs). Le noyau envoie l'événement « widget »
// kind amis_view (« montre mes amis », action de la card 5) ; tout passe par la méthode « reseau » (etat, inscription,
// verification, demander, repondre, envoyer, partager…). Vues : compte à relier, liste des amis, conversation avec un ami.
(function () {
  const J = (window.JARVIS = window.JARVIS || {});
  const bridge = window.jarvisBridge;
  const K = J.widgetKit;
  if (!K) return;
  const { h, place, gripHandle, numBadge, closeButton } = K;
  const KEY = 'amis_view';
  let st = null; // dernier état (view du noyau)
  let open = false; // card affichée
  let chat = null; // { ami, messages } : conversation ouverte
  let share = null; // id de l'ami dont on règle les partages
  let myLists = [];
  let notice = '';
  const form = { serveur: '', email: '', nom: '', code: '', ajout: '', texte: '', dossier: '' };
  let fastTimer = null;

  const ETATS = { en_ligne: ['●', 'en ligne'], absent: ['◐', 'absent'], ne_pas_deranger: ['⊘', 'ne pas déranger'], hors_ligne: ['○', 'hors ligne'] };
  const when = (iso) => {
    if (!iso) return '';
    const d = new Date(iso);
    return d.toDateString() === new Date().toDateString() ? d.toLocaleTimeString('fr-FR', { hour: '2-digit', minute: '2-digit' }) : d.toLocaleDateString('fr-FR', { day: '2-digit', month: '2-digit' });
  };

  async function call(op, args) {
    if (!bridge || !bridge.call) return null;
    try {
      const r = await bridge.call('reseau', op, args || {});
      if (r && r.ok === false) notice = `⚠ ${r.error || 'échec'}`;
      else notice = '';
      if (r && r.etat) st = r.etat;
      else if (r && r.amis) st = { ...st, ...r };
      return r;
    } catch (e) { notice = `⚠ ${e.message || e}`; return null; }
  }

  // Tant que la card est ouverte : relève rapide (le noyau la garde deux minutes, renouvelée ici).
  function keepFast() {
    clearInterval(fastTimer);
    fastTimer = setInterval(() => {
      if (!document.querySelector('.w-amis')) open = false; // card fermée
      if (open) call('rapide').then(() => draw()); else clearInterval(fastTimer);
    }, 60000);
  }

  async function openChat(f) {
    const r = await call('messages', { ami: f.ami.id });
    chat = { ami: f.ami, messages: (r && r.messages) || [] };
    share = null;
    draw();
    call('lu', { ami: f.ami.id });
    setTimeout(() => { const m = document.querySelector('.w-amis .w-am-msgs'); if (m) m.scrollTop = m.scrollHeight; }, 30);
  }

  async function sendMessage() {
    const texte = form.texte.trim();
    if (!texte || !chat) return;
    form.texte = '';
    const r = await call('envoyer', { ami: chat.ami.id, texte });
    if (r && r.messages) chat.messages = r.messages;
    draw();
    setTimeout(() => { const m = document.querySelector('.w-amis .w-am-msgs'); if (m) m.scrollTop = m.scrollHeight; }, 30);
  }

  const btn = (label, tip, fn, cls = '') => h('button', {
    class: `w-note-btn${cls ? ` ${cls}` : ''}`, title: tip,
    onclick: async (e) => { e.currentTarget.disabled = true; await fn(); draw(); }
  }, label);
  const input = (key, attrs) => h('input', { class: 'w-am-input', value: form[key], oninput: (e) => { form[key] = e.target.value; }, ...attrs });

  // --- Compte à relier ------------------------------------------------------------------------------------------------

  function setupView() {
    if (!form.serveur) form.serveur = st.serveurPropose || st.serveur || '';
    if (st.codeEnAttente) {
      return [
        h('div', { class: 'w-am-help' }, `Un code à 6 chiffres vient de partir vers ${form.email}. Recopie-le ici (valable 15 minutes).`),
        input('code', { placeholder: 'Code reçu par mail', inputmode: 'numeric', maxlength: '8' }),
        h('div', { class: 'w-note-actions' },
          btn('✔ Relier mon compte', 'Vérifier le code', () => call('verification', { code: form.code }), 'primary'),
          h('span', { class: 'w-note-spacer' }),
          btn('↺ Autre mail', 'Recommencer', async () => { st.codeEnAttente = false; }))];
    }
    return [
      h('div', { class: 'w-am-help' }, 'Relie ton mail pour échanger avec d’autres Jarvis : messages, présence, listes et dossiers, toujours chiffrés de bout en bout et partagés ami par ami.'),
      input('serveur', { placeholder: 'Adresse du service (https://…/reseau)' }),
      input('nom', { placeholder: 'Ton prénom (vu par tes amis)', maxlength: '80' }),
      input('email', { placeholder: 'Ton mail', type: 'email' }),
      h('div', { class: 'w-note-actions' },
        btn('✉ Recevoir un code', 'Un code de vérification part vers ce mail', () => call('inscription', { serveur: form.serveur, email: form.email, nom: form.nom }), 'primary'))];
  }

  // --- Amis -----------------------------------------------------------------------------------------------------------

  function friendRow(f) {
    const [ico, label] = ETATS[(f.presence && f.presence.etat) || ''] || ['', ''];
    const name = f.ami.nom || f.ami.email;
    if (f.statut === 'pending' && !f.envoyee) {
      return h('div', { class: 'w-am-row' },
        h('div', { class: 'w-am-name' }, `✉ ${name}`, h('span', { class: 'w-am-sub' }, ` ${f.ami.email} veut devenir ton ami`)),
        h('div', { class: 'w-note-actions' },
          btn('✔ Accepter', 'Devenir amis (rien n’est partagé tant que tu ne le choisis pas)', () => call('repondre', { id: f.id, reponse: 'accepter' }), 'primary'),
          btn('✕ Refuser', 'Refuser la demande', () => call('repondre', { id: f.id, reponse: 'refuser' })),
          h('span', { class: 'w-note-spacer' }),
          btn('⊘ Bloquer', 'Ne plus recevoir de demande de ce compte', () => call('repondre', { id: f.id, reponse: 'bloquer' }))));
    }
    if (f.statut === 'pending') {
      return h('div', { class: 'w-am-row w-am-dim' }, h('div', { class: 'w-am-name' }, `⌛ ${name}`, h('span', { class: 'w-am-sub' }, ' demande envoyée')),
        h('div', { class: 'w-note-actions' }, h('span', { class: 'w-note-spacer' }), btn('Annuler', 'Retirer la demande', () => call('retirer', { id: f.id }))));
    }
    if (f.statut === 'blocked') {
      return h('div', { class: 'w-am-row w-am-dim' }, h('div', { class: 'w-am-name' }, `⊘ ${name}`, h('span', { class: 'w-am-sub' }, ' bloqué')),
        h('div', { class: 'w-note-actions' }, h('span', { class: 'w-note-spacer' }), btn('Débloquer', 'Débloquer (il pourra de nouveau te demander)', () => call('repondre', { id: f.id, reponse: 'debloquer' }))));
    }
    const last = f.dernier ? `${f.dernier.moi ? 'toi : ' : ''}${f.dernier.texte}` : 'aucun message';
    return h('div', { class: 'w-am-row w-am-click', onclick: () => openChat(f), title: `Empreinte de sa clé : ${f.empreinte}` },
      h('div', { class: 'w-am-name' },
        f.presence ? h('span', { class: `w-am-etat w-am-${f.presence.etat}`, title: label }, ico) : null,
        ` ${name}`,
        f.nonLus ? h('span', { class: 'w-count' }, String(f.nonLus)) : null,
        f.presence && f.presence.avatar ? h('span', { class: 'w-am-sub', title: 'Son compagnon (stade, branche)' }, ` · ${f.presence.avatar.replace(':', ' ')}`) : null),
      h('div', { class: 'w-am-sub w-am-last' }, last));
  }

  function listView() {
    const amis = st.amis || [];
    const waiting = amis.filter((f) => f.statut === 'pending' && !f.envoyee);
    const others = amis.filter((f) => !(f.statut === 'pending' && !f.envoyee));
    const recues = st.listesRecues || [];
    return [
      h('div', { class: 'w-am-help' }, `${st.compte && st.compte.nom ? st.compte.nom : ''} · ${st.email || ''} · ${(ETATS[st.presence] || ['', ''])[1]}`),
      ...waiting.map(friendRow),
      ...others.map(friendRow),
      amis.length ? null : h('div', { class: 'w-rows-foot' }, 'Pas encore d’ami : ajoute son mail ci-dessous.'),
      recues.length ? h('div', { class: 'w-nm-group' }, `Listes partagées avec toi (${recues.length})`) : null,
      ...recues.map((l) => h('div', { class: 'w-am-row' },
        h('div', { class: 'w-am-name' }, `▤ ${(l.liste && l.liste.nom) || 'Liste'}`, h('span', { class: 'w-am-sub' }, ` de ${l.nom} · ${(l.liste && (l.liste.elements || []).length) || 0} fiche(s) · ${l.droits === 'ecriture' ? 'modifiable' : 'lecture seule'} · ${when(l.le)}`)))),
      h('div', { class: 'w-am-add' },
        input('ajout', { placeholder: 'Mail d’un ami à inviter', type: 'email' }),
        btn('＋ Demander', 'Demande d’ami (une invitation par mail s’il n’a pas encore Jarvis)', async () => {
          const r = await call('demander', { email: form.ajout });
          if (r && r.ok) { notice = r.statut === 'invite' ? 'Invitation envoyée par mail : la demande l’attendra à son inscription.' : r.statut === 'acceptee' ? 'Vous êtes amis.' : 'Demande envoyée.'; form.ajout = ''; }
        }, 'primary')),
      h('div', { class: 'w-note-actions' },
        h('label', { class: 'w-am-check', title: 'Tes amis te voient « ne pas déranger » ; les messages ne sont pas lus à voix haute' },
          h('input', { type: 'checkbox', checked: Boolean(st.nePasDeranger), onchange: (e) => call('reglage', { cle: 'nePasDeranger', valeur: e.target.checked }).then(draw) }), ' Ne pas déranger'),
        h('span', { class: 'w-note-spacer' }),
        st.empreinte ? h('span', { class: 'w-am-sub', title: 'À comparer de vive voix avec tes amis pour être sûr que c’est bien toi' }, `ma clé : ${st.empreinte}`) : null)];
  }

  // --- Conversation et partages -------------------------------------------------------------------------------------------

  function shareView(f) {
    const given = f.partagesDonnes || [];
    const has = (type, ref = '') => given.find((s) => s.type === type && s.ref === ref);
    const toggleShare = (type, ref, label, on, droits) => call('partager', { ami: f.ami.id, type, ref, label, actif: on, droits }).then(draw);
    if (!myLists.length) call('mesListes').then((r) => { myLists = (r && r.listes) || []; draw(); });
    return h('div', { class: 'w-am-share' },
      h('div', { class: 'w-nm-group' }, `Ce que tu partages avec ${f.ami.nom}`),
      h('label', { class: 'w-am-check' }, h('input', { type: 'checkbox', checked: Boolean(has('presence')), onchange: (e) => toggleShare('presence', '', 'Présence', e.target.checked) }),
        ' Ma présence (en ligne, absent, ne pas déranger) et mon compagnon'),
      ...myLists.map((l) => {
        const s = has('liste', l.id);
        return h('div', { class: 'w-am-check' },
          h('input', { type: 'checkbox', checked: Boolean(s), onchange: (e) => toggleShare('liste', l.id, l.nom, e.target.checked, 'lecture') }), ` ▤ ${l.nom} `,
          s ? h('select', { class: 'w-am-select', onchange: (e) => toggleShare('liste', l.id, l.nom, true, e.target.value) },
            h('option', { value: 'lecture', selected: s.rights !== 'ecriture' }, 'lecture'),
            h('option', { value: 'ecriture', selected: s.rights === 'ecriture' }, 'il peut modifier')) : null);
      }),
      h('div', { class: 'w-am-add' },
        input('dossier', { placeholder: 'Dossier à lui envoyer (copie, 25 Mo au plus)' }),
        btn('⇪ Envoyer', 'Envoie une copie du dossier ; il la reçoit en lecture seule', async () => {
          const r = await call('envoyerDossier', { ami: f.ami.id, chemin: form.dossier });
          if (r && r.ok) { notice = `${r.fichiers} fichier(s) envoyé(s).`; form.dossier = ''; }
        })),
      h('div', { class: 'w-note-actions' },
        h('label', { class: 'w-am-check' }, h('input', { type: 'checkbox', checked: Boolean(f.lireMessages), onchange: (e) => call('reglage', { cle: 'lireMessages', ami: String(f.ami.id), valeur: e.target.checked }).then(draw) }), ' Lire ses messages à voix haute'),
        h('span', { class: 'w-note-spacer' }),
        btn('📂 Reçus', 'Ouvrir le dossier de ce qu’il t’a envoyé', () => call('ouvrirRecus', { ami: f.ami.id })),
        btn('✕ Retirer cet ami', 'Ne plus être amis : tous les partages s’arrêtent', async () => { await call('retirer', { id: f.id }); chat = null; share = null; }, 'danger')));
  }

  function chatView() {
    const f = (st.amis || []).find((x) => x.ami.id === chat.ami.id) || { ami: chat.ami, partagesDonnes: [] };
    const [ico, label] = ETATS[(f.presence && f.presence.etat) || ''] || ['', ''];
    return [
      h('div', { class: 'w-note-actions' },
        btn('‹ Amis', 'Revenir à la liste', async () => { chat = null; share = null; }),
        h('span', { class: 'w-am-name' }, ` ${chat.ami.nom} `, f.presence ? h('span', { class: `w-am-etat w-am-${f.presence.etat}` }, `${ico} ${label}`) : null),
        h('span', { class: 'w-note-spacer' }),
        btn(share ? '✉ Messages' : '⇄ Partages', share ? 'Revenir à la conversation' : 'Choisir ce que tu partages avec lui', async () => { share = share ? null : f.id; })),
      share ? shareView(f) : h('div', { class: 'w-am-msgs' },
        ...(chat.messages.length ? chat.messages.map((m) => h('div', { class: `w-am-msg${m.moi ? ' w-am-mine' : ''}` },
          h('div', { class: 'w-am-text' }, m.texte),
          h('div', { class: 'w-am-sub' }, when(m.le), m.moi && m.luParAmi ? ' · lu' : ''))) : [h('div', { class: 'w-rows-foot' }, 'Aucun message. Écris le premier !')])),
      share ? null : h('div', { class: 'w-am-add' },
        h('input', { class: 'w-am-input', value: form.texte, placeholder: `Message à ${chat.ami.nom}`, oninput: (e) => { form.texte = e.target.value; }, onkeydown: (e) => { if (e.key === 'Enter') sendMessage(); } }),
        h('button', { class: 'w-note-btn primary', title: 'Envoyer (chiffré de bout en bout)', onclick: sendMessage }, '➤'))];
  }

  function draw() {
    if (!st || !open) return;
    const unread = (st.amis || []).reduce((n, f) => n + (f.nonLus || 0), 0);
    const body = !st.relie ? setupView() : chat ? chatView() : listView();
    const el = h('div', { class: 'widget w-tool w-card-list w-wide w-amis' },
      h('div', { class: 'w-head' },
        gripHandle(), numBadge(st),
        h('span', { class: 'w-icon' }, '♥'),
        h('span', { class: 'w-title' }, 'AMIS'),
        unread ? h('span', { class: 'w-count' }, String(unread)) : null,
        closeButton(st, KEY)),
      st.erreur ? h('div', { class: 'w-nm-state w-nm-err' }, st.erreur) : null,
      ...body,
      notice ? h('div', { class: 'w-am-sub w-am-notice' }, notice) : null);
    place(KEY, el, KEY, 0);
  }

  function render(ev) {
    st = { ...(st || {}), ...ev };
    open = true;
    keepFast();
    draw();
  }

  // Le noyau signale chaque changement (message, demande, partage) : la card ouverte se redessine.
  if (bridge && bridge.onTaskEvent) {
    bridge.onTaskEvent((ev) => {
      if (!ev || !open) return;
      if (!document.querySelector('.w-amis')) { open = false; return; }
      if (ev.type === 'reseau' && ev.etat) {
        st = { ...st, ...ev.etat };
        if (chat) call('messages', { ami: chat.ami.id }).then((r) => { if (r && r.messages) { chat.messages = r.messages; call('lu', { ami: chat.ami.id }); } draw(); });
        else draw();
      }
    });
  }

  K.register(KEY, render);
  K.vanish(KEY);
})();
