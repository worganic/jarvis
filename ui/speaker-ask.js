// « Qui parle ? » : phrase d'une voix non reconnue (empreinte vocale), juste au-dessus de la barre de
// commande. Jarvis ne l'a pas exécutée ; un bouton par personne enregistrée (« Johann ») l'attribue :
// Jarvis apprend cette phrase pour mieux reconnaître la voix et, si elle est récente, l'exécute.
// « Nouvelle personne » crée un profil (prénom + lien : fille, femme, ami…) à partir de cette phrase.
// Orange : doute (voix proche d'un profil) ; rouge : voix non reconnue.
// Événements : voice_final (pendingId, people, speakerCheck), speaker_learned, speaker_pending_done.
(function () {
  const J = (window.JARVIS = window.JARVIS || {});
  const bridge = window.jarvisBridge;

  const TYPES = [
    ['moi', 'Moi'], ['femme', 'Femme / compagne'], ['mari', 'Mari / compagnon'], ['fille', 'Fille'], ['fils', 'Fils'],
    ['famille', 'Famille'], ['ami', 'Ami(e)'], ['invite', 'Invité(e)'], ['autre', 'Autre']
  ];
  const TYPE_LABEL = Object.fromEntries(TYPES);
  const LIFETIME = { doubt: 60000, unknown: 30000 };

  function el(tag, cls, text) {
    const e = document.createElement(tag);
    if (cls) e.className = cls;
    if (text !== undefined) e.textContent = text;
    return e;
  }

  const box = el('div', 'sa-box');
  box.id = 'speaker-ask';
  box.hidden = true;
  document.body.appendChild(box);
  // Les touches tapées ici (prénom) ne déclenchent pas les raccourcis du HUD (Échap = STOP, F5…).
  box.addEventListener('keydown', (e) => e.stopPropagation());

  let current = null; // { ev, state, timer, sample, paused }

  function place() {
    const bar = document.getElementById('console');
    const r = bar ? bar.getBoundingClientRect() : null;
    const bottom = r && r.height ? window.innerHeight - r.top + 10 : 96;
    box.style.bottom = `${Math.max(8, bottom)}px`;
    box.style.left = r && r.width ? `${r.left + r.width / 2}px` : '50%';
  }
  window.addEventListener('resize', () => { if (!box.hidden) place(); });

  function hide() {
    if (current) clearTimeout(current.timer);
    current = null;
    box.hidden = true;
    box.textContent = '';
  }

  function arm(ms) {
    if (!current) return;
    clearTimeout(current.timer);
    current.timer = setTimeout(() => {
      if (!current) return;
      if (current.paused) { arm(3000); return; }
      hide();
    }, ms);
  }

  function header(state, check) {
    const pct = check && Number.isFinite(check.similarity) ? ` · ${Math.round(check.similarity * 100)} %` : '';
    const cand = check && check.candidate ? String(check.candidate.name || '').toUpperCase() : '';
    const head = el('div', 'sa-head');
    head.append(
      el('span', 'sa-dot', '●'),
      el('span', 'sa-state', state === 'doubt' ? `${cand ? `${cand} ?` : 'VOIX DOUTEUSE'}${pct}` : `VOIX NON RECONNUE${pct}`),
      el('span', 'sa-hint', state === 'doubt' ? 'pas sûr que ce soit toi — confirme pour exécuter' : 'demande ignorée — qui parle ?'));
    const close = el('button', 'sa-close', '✕');
    close.type = 'button';
    close.title = 'Ignorer (ce n’est personne d’enregistré : télévision, visiteur…)';
    close.onclick = () => {
      if (current && bridge && bridge.speakerDismiss) bridge.speakerDismiss(current.ev.pendingId).catch(() => {});
      hide();
    };
    head.appendChild(close);
    return head;
  }

  function done(text, sample) {
    if (!current) return;
    box.classList.add('sa-ok');
    const row = el('div', 'sa-done');
    row.append(el('span', 'sa-done-t', text));
    if (sample && bridge && bridge.speakerUndo) {
      const undo = el('button', 'sa-btn sa-undo', 'Annuler');
      undo.type = 'button';
      undo.title = 'Mauvais bouton : Jarvis oublie cette phrase';
      undo.onclick = () => {
        bridge.speakerUndo(sample).catch(() => {});
        row.textContent = 'Apprentissage annulé.';
        arm(2500);
      };
      row.appendChild(undo);
    }
    for (const old of box.querySelectorAll('.sa-actions, .sa-form, .sa-done, .sa-err')) old.remove();
    box.appendChild(row);
    current.paused = false;
    current.done = true;
    arm(6000);
  }

  function fail(message) {
    const err = box.querySelector('.sa-err') || el('div', 'sa-err');
    err.textContent = message || 'Impossible pour le moment.';
    if (!err.isConnected) box.appendChild(err);
  }

  async function confirm(person) {
    if (!current || !bridge || !bridge.speakerConfirm) return;
    const ev = current.ev;
    box.classList.add('sa-busy');
    current.busy = true;
    const r = await bridge.speakerConfirm(ev.pendingId, person.id).catch((e) => ({ ok: false, error: e.message }));
    box.classList.remove('sa-busy');
    if (!current || current.ev !== ev) return;
    current.busy = false;
    if (!r || !r.ok) { fail(r && r.error); return; }
    done(`✓ Voix de ${person.name} mémorisée${r.executed ? ' — demande exécutée' : ''}`, current.sample);
  }

  function newPersonForm() {
    if (!current) return;
    current.paused = true;
    const form = el('form', 'sa-form');
    const name = el('input', 'sa-input');
    name.placeholder = 'Prénom';
    name.maxLength = 40;
    name.required = true;
    name.autocomplete = 'off';
    name.spellcheck = false;
    const type = el('select', 'sa-select');
    type.required = true;
    const ph = el('option', null, 'Lien…');
    ph.value = '';
    type.appendChild(ph);
    for (const [value, label] of TYPES) {
      const o = el('option', null, label);
      o.value = value;
      type.appendChild(o);
    }
    const save = el('button', 'sa-btn sa-primary', 'Enregistrer');
    save.type = 'submit';
    const cancel = el('button', 'sa-btn', 'Annuler');
    cancel.type = 'button';
    cancel.onclick = () => { form.remove(); if (current) { current.paused = false; box.querySelector('.sa-actions').hidden = false; } };
    form.append(name, type, save, cancel);
    form.onsubmit = async (e) => {
      e.preventDefault();
      if (!current || !bridge || !bridge.speakerCreate) return;
      const n = name.value.replace(/\s+/g, ' ').trim();
      if (!n || !type.value) return;
      save.disabled = true;
      const ev = current.ev;
      current.busy = true;
      const r = await bridge.speakerCreate(ev.pendingId, n, type.value).catch((err) => ({ ok: false, error: err.message }));
      save.disabled = false;
      if (!current || current.ev !== ev) return;
      current.busy = false;
      if (!r || !r.ok) { fail(r && r.error); return; }
      done(`✓ ${n} (${TYPE_LABEL[type.value] || type.value}) ajouté${type.value === 'fille' || type.value === 'femme' ? 'e' : ''}${r.executed ? ' — demande exécutée' : ''}. Jarvis affinera sa voix à chaque confirmation.`, current.sample);
    };
    const actions = box.querySelector('.sa-actions');
    if (actions) actions.hidden = true;
    box.appendChild(form);
    name.focus();
  }

  function show(ev) {
    const check = ev.speakerCheck || {};
    const state = check.decision === 'doubt' ? 'doubt' : 'unknown';
    if (current) clearTimeout(current.timer);
    current = { ev, state, sample: check.sample || null, paused: false, timer: null };
    box.textContent = '';
    box.className = `sa-box sa-${state}`;
    box.appendChild(header(state, check));
    box.appendChild(el('div', 'sa-text', `« ${String(ev.text || '').trim()} »`));
    const actions = el('div', 'sa-actions');
    actions.appendChild(el('span', 'sa-q', 'Qui parle ?'));
    for (const p of ev.people || []) {
      const b = el('button', 'sa-btn sa-person', p.name);
      b.type = 'button';
      b.title = `C'est ${p.name}${TYPE_LABEL[p.type] ? ` (${TYPE_LABEL[p.type].toLowerCase()})` : ''} : Jarvis apprend cette phrase pour mieux reconnaître sa voix`;
      b.onclick = () => confirm(p);
      actions.appendChild(b);
    }
    const add = el('button', 'sa-btn sa-new', '＋ Nouvelle personne');
    add.type = 'button';
    add.title = 'Enregistrer une nouvelle voix (fille, femme, ami…) à partir de cette phrase';
    add.onclick = newPersonForm;
    actions.appendChild(add);

    const parasite = el('button', 'sa-btn sa-parasite', '🚫 Parole parasite');
    parasite.type = 'button';
    parasite.title = 'Parole provenant d’une source extérieure (télévision, conversation tierce...) : à ne pas prendre en compte ni mémoriser';
    parasite.onclick = async () => {
      if (!current || current.busy || current.done) return;
      const pid = current.ev ? current.ev.pendingId : null;
      done('Parole parasite ignorée (non prise en compte).', null);
      arm(2000);
      if (pid && bridge && bridge.speakerDismiss) {
        await bridge.speakerDismiss(pid).catch(() => {});
      }
    };
    actions.appendChild(parasite);

    box.appendChild(actions);
    box.hidden = false;
    place();
    arm(LIFETIME[state]);
  }

  box.addEventListener('pointerenter', () => { if (current) current.paused = true; });
  box.addEventListener('pointerleave', () => { if (current && !box.querySelector('.sa-form')) current.paused = false; });

  if (bridge && bridge.onTaskEvent) {
    bridge.onTaskEvent((ev) => {
      if (!ev || !current) return;
      if (ev.type === 'speaker_learned' && current.done) {
        const t = box.querySelector('.sa-done-t');
        if (!t) return;
        if (!ev.ok) t.textContent = `✗ ${ev.message || 'Apprentissage impossible'}`;
        else if (ev.learned) t.textContent += ` (${ev.learned} phrase${ev.learned > 1 ? 's' : ''} apprise${ev.learned > 1 ? 's' : ''})`;
      } else if (ev.type === 'speaker_pending_done' && ev.pendingId === current.ev.pendingId && !current.done && !current.busy) {
        hide(); // réglée depuis l'autre page HUD (Bureau JARVIS / fenêtre classique)
      }
    });
  }

  J.speakerAsk = {
    show,
    hide,
    // Haut de la boîte (px CSS), ou null : app.js place la parole en cours au-dessus.
    top: () => (box.hidden ? null : box.getBoundingClientRect().top)
  };
})();
