// Module Mails : la card de lecture d'un mail (expéditeur, destinataires, date, pièces jointes, texte et actions).
// La liste des mails est une card de liste du système (widgets.js › renderRows) ; le noyau (orch/mails.rs) envoie
// l'événement « widget » kind mail_view et reçoit les actions (mailAction).
(function () {
  const J = (window.JARVIS = window.JARVIS || {});
  const bridge = window.jarvisBridge;
  const K = J.widgetKit;
  if (!K) return;
  const { h, place, gripHandle, numBadge, closeButton } = K;

  // Clic sur un mail d'une card, ou « ouvre le mail 3 » : expéditeur, destinataires, date, pièces
  // jointes, texte (sélectionnable, jamais de lien actif : la fenêtre du HUD ne doit pas naviguer),
  // et les actions sur ce mail.
  function fmtSize(bytes) {
    if (!Number.isFinite(bytes)) return '';
    if (bytes < 1024) return `${bytes} o`;
    if (bytes < 1024 * 1024) return `${Math.round(bytes / 1024)} Ko`;
    return `${(bytes / 1024 / 1024).toFixed(1)} Mo`;
  }

  function renderMailView(ev) {
    const key = 'mail_view';
    const act = (label, tip, action, cls = '') => h('button', {
      class: `w-note-btn${cls ? ` ${cls}` : ''}`, title: tip,
      onclick: (e) => {
        const b = e.currentTarget;
        b.disabled = true;
        setTimeout(() => { b.disabled = false; }, 1500);
        if (bridge && bridge.mailAction) Promise.resolve(bridge.mailAction(ev.uid, action)).catch(() => {});
      }
    }, label);
    const line = (label, value) => (value ? h('div', { class: 'w-mail-line' }, h('span', { class: 'w-mail-label' }, label), h('span', { class: 'w-mail-value' }, value)) : null);
    const from = ev.fromAddress ? `${ev.fromName || ''} <${ev.fromAddress}>`.trim() : (ev.fromName || ev.from || '');
    const subject = ev.title || ev.subject || '(sans objet)';
    const dateLabel = ev.dateLabel || ev.date || '';
    const body = ev.text || ev.body || '';
    const el = h('div', { class: 'widget w-tool w-card-list w-wide w-mailview' },
      h('div', { class: 'w-head' },
        gripHandle(),
        numBadge(ev),
        h('span', { class: 'w-icon' }, '✉'),
        h('span', { class: 'w-title w-mail-subject', title: subject }, subject),
        ev.mailNo ? h('span', { class: 'w-count', title: `« lis le mail ${ev.mailNo} », « réponds au mail ${ev.mailNo} »` }, `mail ${ev.mailNo}`) : null,
        closeButton(ev, key)),
      h('div', { class: 'w-mail-meta' },
        line('De', from),
        line('À', ev.to),
        line('Cc', ev.cc),
        line('Date', dateLabel)),
      ev.attachments && ev.attachments.length
        ? h('div', { class: 'w-mail-attach' }, ...ev.attachments.map((a) => h('span', { class: 'w-mail-file', title: a.type || '' }, `📎 ${a.name}${a.size ? ` · ${fmtSize(a.size)}` : ''}`)))
        : null,
      h('div', { class: 'w-mail-body' }, body),
      ev.cut ? h('div', { class: 'w-rows-foot' }, 'Mail long : seul le début est affiché — « Ouvrir dans Chrome » pour le lire en entier.') : null,
      h('div', { class: 'w-note-actions' },
        act('🔊 Lire', 'Lire ce mail à voix haute', 'lire'),
        act('✦ Résumer', 'Résumé par l’IA, à voix haute', 'resumer'),
        act('↩ Répondre', 'L’IA propose une réponse (rien n’est envoyé sans ton accord et ton code)', 'repondre'),
        act('🚫 Exclure', 'Ne plus m’annoncer les mails de cet expéditeur', 'exclure'),
        h('span', { class: 'w-note-spacer' }),
        act('🌐 Ouvrir dans Chrome', 'Ouvrir ce mail directement dans Google Chrome', 'gmail', 'primary')));
    place(key, el, 'mail_view', 0);
  }

  K.register('mail_view', renderMailView);
  K.vanish('mail_view');
})();
