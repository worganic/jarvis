// Module Create module : créer, modifier et supprimer ses propres modules Jarvis. L'utilisateur décrit la card voulue,
// une IA (sans outil) pose des questions de précision, puis le noyau écrit le module dans data/user-modules/<id>/
// (orch : crates/jarvis-core/src/module_studio.rs, méthode « moduleStudio »). La card s'ouvre depuis la card Actions.
(function () {
  const J = (window.JARVIS = window.JARVIS || {});
  const bridge = window.jarvisBridge;
  const ID = 'creer-module';
  const info = (J.modules && J.modules[ID]) || {};
  if (!bridge || !bridge.call || !J.cardHeader) return;

  const h = (tag, props, ...kids) => {
    const e = document.createElement(tag);
    for (const [k, v] of Object.entries(props || {})) {
      if (k === 'class') e.className = v;
      else if (k.startsWith('on')) e.addEventListener(k.slice(2), v);
      else if (v !== false && v != null) e.setAttribute(k, v);
    }
    e.append(...kids.flat().filter((x) => x != null && x !== false));
    return e;
  };
  const button = (label, title, fn, cls) => h('button', { class: `cm-btn${cls ? ` ${cls}` : ''}`, title: title || '', onclick: fn }, label);

  async function call(op, args) {
    try { return (await bridge.call('moduleStudio', op, args || {})) || { ok: false, error: 'Pas de réponse.' }; } catch (e) { return { ok: false, error: String((e && e.message) || e) }; }
  }

  const card = h('div', { class: 'widget w-active mod-card', id: `mod-${ID}`, hidden: '' });
  card.style.display = 'none';
  const hdr = J.cardHeader({ key: ID, no: J.CARD_NO[ID], title: (info.card && info.card.titre) || 'CREATE MODULE', onCollapse: () => card.classList.toggle('mod-collapsed') });
  const closeBtn = h('button', { class: 'cm-close', title: 'Fermer', onclick: () => { card.style.display = 'none'; } }, '✕');
  hdr.head.append(closeBtn);
  const body = h('div', { class: 'mod-body cm-body' });
  card.append(hdr.head, body);
  document.body.appendChild(card);

  // Déplacement à la souris par l'en-tête.
  let drag = null;
  hdr.head.addEventListener('pointerdown', (e) => {
    if (e.target.closest('button')) return;
    const r = card.getBoundingClientRect();
    drag = { dx: e.clientX - r.left, dy: e.clientY - r.top };
    hdr.head.setPointerCapture(e.pointerId);
  });
  hdr.head.addEventListener('pointermove', (e) => {
    if (!drag) return;
    card.style.left = `${e.clientX - drag.dx}px`;
    card.style.top = `${e.clientY - drag.dy}px`;
  });
  hdr.head.addEventListener('pointerup', () => { drag = null; });

  // --- État -------------------------------------------------------------------------------------------------
  let view = 'list'; // list | new | chat
  let modules = [];
  let message = { text: '', error: false };
  let chat = null; // { session, target, lines: [{ who, text }], propose, busy }
  let armed = ''; // suppression armée (double clic)

  const say = (text, error) => { message = { text: text || '', error: Boolean(error) }; };

  async function refresh() {
    const r = await call('lister');
    if (r.ok) modules = r.modules || [];
    else say(r.error || 'Liste indisponible.', true);
    render();
  }

  // --- Vues -------------------------------------------------------------------------------------------------
  function listView() {
    const rows = modules.map((m) => h('div', { class: 'cm-row' },
      h('label', { class: 'cm-switch', title: m.actif ? 'Actif : clique pour désactiver' : 'Désactivé : clique pour activer' },
        h('input', { type: 'checkbox', ...(m.actif ? { checked: '' } : {}), onchange: async (e) => {
          const r = await call('activer', { module: m.id, actif: e.target.checked });
          say(r.ok ? '' : r.error, !r.ok);
          refresh();
        } })),
      h('div', { class: 'cm-row-text' },
        h('b', {}, m.nom), h('span', { class: 'cm-tag' }, `card ${m.numero}`),
        h('div', { class: 'cm-sub' }, m.description || '')),
      button('✎', 'Modifier ce module avec l’IA', () => startEdit(m)),
      (() => {
        const del = button('🗑', 'Supprimer (double clic) — le dossier va dans la corbeille', async () => {
          if (armed !== m.id) { armed = m.id; del.textContent = '⚠ confirmer'; setTimeout(() => { if (armed === m.id) { armed = ''; render(); } }, 4000); return; }
          armed = '';
          const r = await call('supprimer', { module: m.id, confirmer: m.id });
          say(r.ok ? 'Module supprimé (dans la corbeille de data/user-modules).' : r.error, !r.ok);
          refresh();
        }, 'danger');
        if (armed === m.id) del.textContent = '⚠ confirmer';
        return del;
      })()));
    return [
      h('div', { class: 'cm-top' }, h('span', { class: 'cm-title' }, 'Mes modules'), button('✚ Nouveau module', 'Décrire un module à créer', () => { view = 'new'; say(''); render(); }, 'primary')),
      rows.length ? h('div', { class: 'cm-list' }, ...rows) : h('div', { class: 'cm-empty' }, 'Aucun module créé pour l’instant. Clique sur « Nouveau module », décris la card que tu veux, et Jarvis la construit avec toi.')
    ];
  }

  function newView() {
    const ta = h('textarea', { class: 'cm-in', rows: '6', maxlength: '4000', placeholder: 'Explique le module que tu veux : à quoi il sert, ce que la card doit afficher, les boutons ou les données dont tu as besoin…\nEx. : « Un compteur d’eau bue : un bouton + ajoute un verre, la card affiche le total du jour et remet à zéro à minuit. »' });
    return [
      h('div', { class: 'cm-title' }, 'Nouveau module'),
      ta,
      h('div', { class: 'cm-actions' },
        button('Envoyer à l’IA', 'Jarvis peut te poser des questions pour préciser', () => { if (ta.value.trim()) send('demarrer', { texte: ta.value.trim() }, ta.value.trim()); }, 'primary'),
        button('Annuler', '', () => { view = 'list'; say(''); render(); }))
    ];
  }

  function startEdit(m) {
    view = 'edit';
    chat = { module: m, lines: [] };
    say('');
    render();
  }

  function editView() {
    const ta = h('textarea', { class: 'cm-in', rows: '4', maxlength: '4000', placeholder: 'Qu’est-ce que tu veux changer dans ce module ?' });
    return [
      h('div', { class: 'cm-title' }, `Modifier « ${chat.module.nom} »`),
      ta,
      h('div', { class: 'cm-actions' },
        button('Envoyer à l’IA', '', () => { if (ta.value.trim()) send('modifier', { module: chat.module.id, texte: ta.value.trim() }, ta.value.trim(), chat.module.id); }, 'primary'),
        button('Annuler', '', () => { view = 'list'; chat = null; say(''); render(); }))
    ];
  }

  function chatView() {
    const log = h('div', { class: 'cm-chat' }, ...chat.lines.map((l) => h('div', { class: `cm-msg ${l.who}` }, l.text)));
    if (chat.busy) log.append(h('div', { class: 'cm-msg ai cm-think' }, 'Jarvis réfléchit… (jusqu’à une minute ou deux)'));
    const ta = h('textarea', { class: 'cm-in', rows: '2', maxlength: '4000', placeholder: chat.propose ? 'Un changement à demander ? Écris-le ici…' : 'Ta réponse…' });
    const reply = () => { if (ta.value.trim() && !chat.busy) send('repondre', { session: chat.session, texte: ta.value.trim() }, ta.value.trim()); };
    ta.addEventListener('keydown', (e) => { if (e.key === 'Enter' && !e.shiftKey) { e.preventDefault(); reply(); } });
    const out = [h('div', { class: 'cm-title' }, chat.target ? `Modifier « ${chat.target} »` : 'Nouveau module'), log];
    if (chat.propose && !chat.busy) {
      out.push(h('div', { class: 'cm-propose' },
        h('b', {}, chat.propose.nom), h('div', { class: 'cm-sub' }, chat.propose.description),
        chat.propose.resume ? h('div', { class: 'cm-sub' }, chat.propose.resume) : null,
        h('div', { class: 'cm-actions' }, button(chat.propose.modification ? '✔ Enregistrer et activer' : '✔ Créer et activer', 'Écrit le module dans data/user-modules/ et l’affiche', create, 'primary'))));
    }
    out.push(ta, h('div', { class: 'cm-actions' }, button('Envoyer', '', reply, chat.propose ? '' : 'primary'), button('Annuler', 'Abandonner cette création', cancel)));
    queueMicrotask(() => { log.scrollTop = log.scrollHeight; });
    return out;
  }

  // --- Actions ----------------------------------------------------------------------------------------------
  async function send(op, args, userText, target) {
    if (!chat || !chat.session) chat = { session: '', target: target || (chat && chat.module && chat.module.nom) || '', lines: [], propose: null, busy: false };
    view = 'chat';
    chat.lines.push({ who: 'me', text: userText });
    chat.propose = null;
    chat.busy = true;
    say('');
    render();
    const r = await call(op, args);
    chat.busy = false;
    if (!r.ok) {
      chat.lines.push({ who: 'err', text: r.error || 'Échec.' });
    } else {
      chat.session = r.session || chat.session;
      if (r.question) chat.lines.push({ who: 'ai', text: r.question });
      if (r.propose) {
        chat.propose = r.propose;
        chat.lines.push({ who: 'ai', text: `J’ai préparé le module « ${r.propose.nom} ». Vérifie le résumé ci-dessous, puis crée-le — ou dis-moi ce qu’il faut changer.` });
      }
    }
    render();
  }

  async function create() {
    chat.busy = true;
    render();
    const r = await call('creer', { session: chat.session });
    chat.busy = false;
    if (r.ok) {
      view = 'list';
      chat = null;
      say(r.say || 'Module créé.');
      await refresh();
    } else {
      chat.lines.push({ who: 'err', text: r.error || 'Échec.' });
      render();
    }
  }

  function cancel() {
    if (chat && chat.session) call('annuler', { session: chat.session });
    view = 'list';
    chat = null;
    say('');
    render();
  }

  function render() {
    const parts = view === 'new' ? newView() : view === 'edit' ? editView() : view === 'chat' && chat ? chatView() : listView();
    if (message.text) parts.push(h('div', { class: `cm-msg-line${message.error ? ' err' : ''}` }, message.text));
    // Une saisie en cours n'est pas perdue quand la vue se redessine pour un changement d'état.
    const keep = body.querySelector('textarea');
    const draft = keep ? keep.value : '';
    body.replaceChildren(...parts);
    const again = body.querySelector('textarea');
    if (again && draft && !again.value) again.value = draft;
  }

  function open() {
    card.style.display = '';
    card.removeAttribute('hidden');
    card.classList.remove('mod-collapsed');
    refresh();
  }

  bridge.onTaskEvent((ev) => {
    if (!ev) return;
    if (ev.type === 'creer_module_open') open();
    else if (ev.type === 'modules_changed' && card.style.display !== 'none' && view === 'list') refresh();
  });
  render();
})();
