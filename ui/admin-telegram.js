// Section « Telegram » de l'administration : relier un bot privé à Jarvis pour lui écrire depuis
// le téléphone et recevoir ses notifications (rappels, nouveaux mails, Claude Code…).
(function () {
  const A = window.JarvisAdmin;
  if (!A) return;
  const { h, panel, field, api, sections, render, toast, toggle, confirmDialog } = A;

  const state = { data: null, loading: false, busy: false, error: '' };
  let poll = null;

  async function call(method, ...args) {
    state.busy = true;
    render(true);
    const res = await api(method, ...args);
    state.busy = false;
    if (res && res.ok) {
      if (method !== 'telegramTest') state.data = res;
      state.error = '';
    } else {
      state.error = (res && res.error) || 'erreur';
    }
    render(true);
    return res;
  }

  async function load() {
    if (state.loading) return;
    state.loading = true;
    const res = await api('telegramState');
    state.loading = false;
    if (res && res.ok) state.data = res;
    render(true);
  }

  // En attente d'appairage : l'état est relu toutes les 3 s pour voir le téléphone arriver.
  function watchPairing(d) {
    const waiting = d && d.configured && !d.paired && document.getElementById('section-telegram')?.classList.contains('active');
    if (waiting && !poll) poll = setInterval(async () => {
      const res = await api('telegramState');
      if (!res || !res.ok) return;
      const was = state.data && state.data.paired;
      state.data = res;
      if (res.paired || !document.getElementById('section-telegram')?.classList.contains('active')) { clearInterval(poll); poll = null; }
      if (res.paired && !was) toast(`✓ Telegram relié à ${res.chatName}`);
      render(true);
    }, 3000);
  }

  function steps() {
    return h('ol', { class: 'help', style: 'margin:0 0 10px 18px; line-height:1.7' },
      h('li', {}, 'Sur Telegram, ouvre une conversation avec ', h('strong', {}, '@BotFather'), ' et envoie ', h('code', {}, '/newbot'), '.'),
      h('li', {}, 'Choisis un nom (ex. « Jarvis ») puis un identifiant finissant par « bot ».'),
      h('li', {}, 'BotFather te donne un jeton (', h('code', {}, '123456789:AA…'), ') : colle-le ci-dessous.'),
      h('li', {}, 'Envoie ensuite à ton bot le code à 6 chiffres qui s’affichera ici.'));
  }

  sections.telegram = () => {
    if (!state.data && !state.loading) load();
    const d = state.data;
    watchPairing(d);
    const token = h('input', { type: 'password', placeholder: d && d.configured ? 'inchangé (laisser vide)' : '123456789:AA…', autocomplete: 'off', spellcheck: 'false' });
    const saveToken = async () => {
      if (!token.value.trim()) return;
      const res = await call('telegramConfigure', token.value);
      if (res && res.ok) toast(`✓ Bot @${res.bot} enregistré`);
    };
    token.addEventListener('keydown', (e) => { if (e.key === 'Enter') saveToken(); });

    const status = !d ? '' : !d.configured ? 'non configuré' : d.paired ? (d.running ? 'relié' : 'en pause') : 'à appairer';

    return [
      h('h1', {}, 'Telegram'),
      h('p', { class: 'lead' }, 'Écris à Jarvis depuis ton téléphone comme tu lui parlerais, et reçois ses notifications quand tu n’es pas devant le PC. Le bot est privé : seule la conversation appairée est écoutée. Une action critique demande toujours le code affiché sur l’écran du PC — il n’est jamais envoyé sur Telegram.'),
      panel('Bot', status,
        d && !d.configured ? steps() : null,
        h('div', { class: 'grid' },
          field('Jeton du bot', token, d && d.bot ? `Bot actuel : @${d.bot}${d.encrypted ? ' — jeton chiffré par Windows' : ''}` : 'Donné par @BotFather.')),
        h('div', { class: 'row', style: 'margin-top:10px' },
          h('button', { class: 'btn primary', disabled: state.busy, onclick: saveToken }, state.busy ? '… vérification' : 'Enregistrer'),
          d && d.configured ? h('button', { class: 'btn ghost', disabled: state.busy, onclick: async () => {
            if (await confirmDialog('Oublier Telegram', 'Le jeton et l’appairage seront effacés. Jarvis n’écoutera plus ce bot.', 'Oublier', true)) call('telegramForget');
          } }, 'Oublier') : null),
        state.error ? h('div', { class: 'help', style: 'color: var(--err); margin-top:10px' }, state.error) : null,
        d && d.lastError && d.configured ? h('div', { class: 'help', style: 'color: var(--warn); margin-top:10px' }, `Dernière erreur Telegram : ${d.lastError}`) : null),
      d && d.configured ? panel('Appairage', d.paired ? d.chatName : 'en attente',
        d.paired
          ? h('div', {},
            h('div', { class: 'help' }, `Conversation reliée : ${d.chatName}. Les messages des autres conversations sont refusés.`),
            h('div', { class: 'row', style: 'margin-top:10px' },
              h('button', { class: 'btn primary', disabled: state.busy, onclick: async () => { const r = await call('telegramTest'); if (r && r.ok) toast('✓ Message de test envoyé'); } }, 'Envoyer un test'),
              h('button', { class: 'btn ghost', disabled: state.busy, onclick: () => call('telegramUnpair') }, 'Changer de conversation')))
          : h('div', {},
            h('div', { class: 'help' }, `Ouvre la conversation avec @${d.bot} sur Telegram (t.me/${d.bot}) et envoie ce code :`),
            h('div', { style: 'font: 600 34px/1.2 var(--mono, monospace); letter-spacing: .3em; margin: 12px 0; color: var(--accent)' }, d.pairingCode || '——————'),
            h('div', { class: 'help' }, d.pairingCode ? 'Valable 15 minutes. Cette page se met à jour dès que le téléphone est relié.' : 'Code expiré.'),
            h('div', { class: 'row', style: 'margin-top:10px' },
              h('button', { class: 'btn ghost', disabled: state.busy, onclick: () => call('telegramUnpair') }, 'Nouveau code')))) : null,
      d && d.configured ? panel('Réglages', null,
        h('div', { class: 'grid' },
          toggle('Écouter le bot', d.enabled, (v) => call('telegramSet', { enabled: v })),
          toggle('Envoyer les notifications (rappels, nouveaux mails, Claude Code, briefing)', d.notifications, (v) => call('telegramSet', { notifications: v }))),
        h('div', { class: 'help', style: 'margin-top:8px' }, 'Pendant 10 minutes après un message Telegram, les réponses de Jarvis et ses listes y sont aussi envoyées.')) : null,
      panel('À écrire au bot', null,
        h('div', { class: 'chips' },
          ...['Quels sont mes rappels ?', 'Rappelle-moi dans 20 minutes d’appeler Paul', 'Fais-moi le briefing', 'Où en est le projet stats-ia ?', 'Éteins la lumière', 'stop'].map((s) => h('span', { class: 'chip passive' }, `« ${s} »`))))
    ];
  };
})();
