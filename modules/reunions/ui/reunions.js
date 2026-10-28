// Module Réunions : la card d'une réunion enregistrée (personnes présentes, voix à nommer, texte en direct, résumé).
// Le noyau (orch/meetings.rs) enregistre et transcrit ; la card agit par meetingAction ; événements : « widget »
// (kind meeting), meeting_line, meeting_update, meeting_relabel.
(function () {
  const J = (window.JARVIS = window.JARVIS || {});
  const bridge = window.jarvisBridge;
  const K = J.widgetKit;
  if (!K) return;
  const { h, place, cards, gripHandle, numBadge, closeButton } = K;

  const CONFIG_LABELS = { casque: 'casque', 'haut-parleurs': 'haut-parleurs' };

  function fmtDuration(ms) {
    const s = Math.max(0, Math.floor(ms / 1000));
    const hh = Math.floor(s / 3600);
    const mm = Math.floor((s % 3600) / 60);
    return `${hh ? `${hh}:${String(mm).padStart(2, '0')}` : mm}:${String(s % 60).padStart(2, '0')}`;
  }

  // Titre (repris de Google Meet, modifiable), durée, personnes présentes, voix inconnues à nommer
  // (suggestions : personnes enregistrées et participants de Meet), transcription en direct.
  let meeting = null; // { id, el, state, title, status, meta, people, voices, lines, summary, buttons, clock, voicesDirty }

  const clockHM = (ms) => { const d = new Date(ms); return `${String(d.getHours()).padStart(2, '0')}:${String(d.getMinutes()).padStart(2, '0')}`; };

  // Couleur stable par personne ; toi : la couleur du thème ; voix inconnue : orange.
  function speakerColor(who) {
    if (!who) return 'rgba(200, 235, 245, 0.7)';
    if (who.self) return 'rgba(var(--w-rgb), 1)';
    if (!who.known) return '#ffb35c';
    let hash = 0;
    for (const c of String(who.name || '')) hash = (hash * 31 + c.charCodeAt(0)) % 360;
    return `hsl(${hash}, 75%, 72%)`;
  }

  function meetingCall(action, payload = {}) {
    if (!meeting || !bridge || !bridge.meetingAction) return Promise.resolve(null);
    return Promise.resolve(bridge.meetingAction(meeting.id, action, payload)).catch((err) => { console.error('réunion :', err); return null; });
  }

  function meetingLine(line) {
    const who = line.who || null;
    return h('div', { class: `w-mt-line${who && !who.known ? ' unknown' : ''}`, 'data-at': String(line.at), 'data-cluster': (who && who.cluster) || '' },
      h('span', { class: 'w-mt-time' }, clockHM(line.at)),
      h('span', { class: 'w-mt-who', style: `color:${speakerColor(who)}` }, who ? who.name : 'Participants'),
      h('span', { class: 'w-mt-text' }, line.text));
  }

  function meetingAddLine(line) {
    const box = meeting.lines;
    const atEnd = box.scrollTop + box.clientHeight >= box.scrollHeight - 30;
    const el = meetingLine(line);
    // Les deux pistes (micro, PC) ne sont pas transcrites dans l'ordre : rangée à son heure.
    let before = null;
    for (let n = box.lastElementChild; n && n.classList.contains('w-mt-line'); n = n.previousElementSibling) {
      if (Number(n.dataset.at) <= line.at) break;
      before = n;
    }
    const empty = box.querySelector('.w-mt-empty');
    if (empty) empty.remove();
    box.insertBefore(el, before);
    meeting.state.total = (meeting.state.total || 0) + 1;
    if (atEnd) box.scrollTop = box.scrollHeight;
  }

  function meetingPeople() {
    const st = meeting.state;
    meeting.people.textContent = '';
    for (const p of st.people || []) {
      const cls = p.kind === 'self' ? 'self' : p.kind === 'unknown' ? 'unknown' : p.kind === 'meet' ? 'meet' : 'known';
      const tip = p.kind === 'self' ? 'Toi' : p.kind === 'unknown' ? 'Voix non reconnue : nomme-la ci-dessous'
        : p.kind === 'meet' ? 'Dans la liste des participants de Meet (voix pas encore entendue)' : `Voix reconnue${p.meet ? ' · présent dans Meet' : ''}`;
      meeting.people.append(h('span', { class: `w-mt-person ${cls}`, title: tip, style: p.kind === 'voice' ? `border-color:${speakerColor({ name: p.name, known: true })}` : null },
        p.kind === 'unknown' ? '? ' : p.kind === 'voice' ? '🎙 ' : p.kind === 'meet' ? '◍ ' : '', p.name));
    }
  }

  function meetingVoices(force = false) {
    const box = meeting.voices;
    // Prénom en cours de saisie : pas de reconstruction (reprise à la sortie du champ).
    const typing = [...box.querySelectorAll('input')].some((i) => i === document.activeElement || i.value.trim());
    if (typing && !force) { meeting.voicesDirty = true; return; }
    meeting.voicesDirty = false;
    box.textContent = '';
    const todo = (meeting.state.voices || []).filter((v) => !v.person);
    box.style.display = todo.length ? '' : 'none';
    for (const v of todo) {
      const input = h('input', { class: 'w-mt-name', type: 'text', placeholder: 'À qui est cette voix ?', spellcheck: 'false', list: `w-mt-dl-${v.id}` });
      const submit = (name) => {
        const n = String(name || input.value).trim();
        if (!n) return;
        v.pending = n;
        meetingCall('name_voice', { voice: v.id, name: n });
        input.value = '';
        meetingVoices(true);
      };
      input.addEventListener('keydown', (e) => {
        if (/^F\d{1,2}$/.test(e.key)) return;
        e.stopPropagation();
        if (e.key === 'Enter') submit();
        if (e.key === 'Escape') { input.value = ''; input.blur(); }
      });
      input.addEventListener('blur', () => { setTimeout(() => { if (meeting && meeting.voicesDirty && !input.value.trim()) meetingVoices(); }, 150); });
      box.append(h('div', { class: 'w-mt-voice' },
        h('div', { class: 'w-mt-voice-head' },
          h('span', { class: 'w-mt-voice-label' }, `? ${v.label}`),
          h('span', { class: 'w-mt-voice-count' }, v.pending ? `→ ${v.pending}… (apprentissage de la voix)` : `${v.count || 0} passage${(v.count || 0) > 1 ? 's' : ''}`)),
        v.pending ? null : h('div', { class: 'w-mt-voice-form' },
          input,
          h('datalist', { id: `w-mt-dl-${v.id}` }, ...(v.suggest || []).map((n) => h('option', { value: n }))),
          h('button', { class: 'w-note-btn primary', title: 'Attribuer cette voix (Jarvis l’apprend)', onclick: () => submit() }, 'OK')),
        v.pending || !(v.suggest || []).length ? null : h('div', { class: 'w-mt-suggest' },
          ...v.suggest.slice(0, 6).map((n) => h('button', { class: 'w-mt-chip', title: `C’est ${n}`, onclick: () => submit(n) }, n)))));
    }
  }

  function meetingRefresh() {
    if (!meeting) return;
    const st = meeting.state;
    const b = meeting.buttons;
    if (st.recording) {
      meeting.status.textContent = `● REC ${fmtDuration(Date.now() - st.created)}`;
      meeting.status.className = 'w-note-status rec';
    } else {
      if (!st.ended && (!st.lines || !st.lines.length)) {
        meeting.status.textContent = 'PRÊTE';
        meeting.status.className = 'w-note-status ready';
      } else {
        meeting.status.textContent = st.ended ? `TERMINÉE · ${fmtDuration(st.ended - st.created)}` : 'ARCHIVE';
        meeting.status.className = 'w-note-status saved';
      }
    }
    const e = st.engine;
    const src = e ? [`micro${e.micro ? ` « ${e.micro} »` : ''}`, e.pc ? 'son du PC' : null].filter(Boolean).join(' + ') : '';
    meeting.meta.textContent = [
      st.dateLabel,
      st.meet ? `Google Meet ${st.meet}` : '',
      st.recording && e ? `${CONFIG_LABELS[e.config] || e.config || ''}${src ? ` · ${src}` : ''}${e.pcError ? ' · son du PC indisponible' : ''}` : '',
      st.file ? `data/meetings/${st.file}` : ''
    ].filter(Boolean).join(' · ');
    const busy = new Set(st.busy || []);
    if (st.recording) {
      b.rec.textContent = '■ Arrêter l’enregistrement';
      b.rec.className = 'w-note-btn rec';
    } else {
      b.rec.textContent = (!st.ended && (!st.lines || !st.lines.length)) ? '◉ Lancer l’enregistrement' : '◉ Nouvelle réunion';
      b.rec.className = 'w-note-btn primary';
    }
    b.summary.disabled = busy.has('summary') || !st.total;
    b.summary.textContent = busy.has('summary') ? '✦ Résumé…' : '✦ Résumé IA';
    b.gdoc.disabled = busy.has('gdoc') || !st.total;
    b.gdoc.textContent = busy.has('gdoc') ? 'Google Doc…' : st.doc ? '↗ Ouvrir le Google Doc' : '📄 Google Doc';
    b.gdoc.classList.toggle('primary', Boolean(st.doc));
    meeting.summary.style.display = st.summary ? '' : 'none';
    if (st.summary && meeting.summaryText.textContent !== st.summary) meeting.summaryText.textContent = st.summary;
  }

  function renderMeeting(ev) {
    const state = {
      created: ev.created, dateLabel: ev.dateLabel || '', ended: ev.ended || null, recording: Boolean(ev.recording),
      meet: ev.meet || '', engine: ev.engine || null, people: ev.people || [], voices: ev.voices || [],
      summary: ev.summary || '', doc: ev.doc || null, file: ev.file || '', busy: ev.busy || [], total: ev.total || 0
    };
    if (meeting) clearInterval(meeting.clock);
    const title = h('input', { class: 'w-note-title', type: 'text', spellcheck: 'true', placeholder: 'Titre de la réunion (Google Meet)', title: 'Titre de la réunion (modifiable)' });
    title.value = ev.title || '';
    const sendTitle = () => { if (meeting && title.value.trim() && title.value.trim() !== meeting.sentTitle) { meeting.sentTitle = title.value.trim(); meetingCall('title', { title: meeting.sentTitle }); } };
    title.addEventListener('keydown', (e) => {
      if (/^F\d{1,2}$/.test(e.key)) return;
      e.stopPropagation();
      if (e.key === 'Enter' || e.key === 'Escape') title.blur();
    });
    title.addEventListener('blur', sendTitle);
    const status = h('span', { class: 'w-note-status' });
    const meta = h('div', { class: 'w-note-meta' });
    const people = h('div', { class: 'w-mt-people' });
    const voices = h('div', { class: 'w-mt-voices' });
    const lines = h('div', { class: 'w-mt-lines' });
    const summaryText = h('div', { class: 'w-mt-summary-text' });
    const summary = h('div', { class: 'w-mt-summary' }, h('div', { class: 'w-mt-summary-head' }, '✦ Résumé'), summaryText);
    const btn = (label, tip, onclick, cls = '') => h('button', { class: `w-note-btn${cls ? ` ${cls}` : ''}`, title: tip, onclick }, label);
    const buttons = {
      rec: btn(state.recording ? '■ Arrêter l’enregistrement' : '◉ Lancer l’enregistrement', 'Arrêter ou démarrer l’enregistrement', () => meetingCall(meeting && meeting.state.recording ? 'stop' : 'start')),
      summary: btn('✦ Résumé IA', 'Résumé de la réunion par l’IA (points clés, décisions, actions)', () => meetingCall('summary')),
      gdoc: btn('📄 Google Doc', 'Compte rendu (résumé + transcription) dans un Google Doc', () => meetingCall('gdoc')),
      list: btn('☰ Liste GMeet', 'Liste des réunions enregistrées (card « liste gmeet »)', () => meetingCall('list'))
    };
    const el = h('div', { class: 'widget w-tool w-card-list w-note w-meeting' },
      h('div', { class: 'w-head' },
        gripHandle(),
        numBadge(ev),
        h('span', { class: 'w-icon' }, '◉'),
        title,
        status,
        closeButton(ev, 'meeting')),
      meta,
      people,
      voices,
      lines,
      summary,
      h('div', { class: 'w-note-actions' }, buttons.rec, buttons.summary, buttons.gdoc, h('span', { class: 'w-note-spacer' }), buttons.list));
    meeting = { id: ev.meetingId, el, state, title, status, meta, people, voices, lines, summary, summaryText, buttons, sentTitle: ev.title || '', voicesDirty: false };
    const all = ev.lines || [];
    if (!all.length) lines.append(h('div', { class: 'w-mt-empty' }, state.recording ? 'J’écoute… les phrases apparaissent ici.' : 'Aucune phrase.'));
    if (state.total > all.length) lines.append(h('div', { class: 'w-mt-empty' }, `… ${state.total - all.length} phrases plus anciennes dans l’archive`));
    for (const l of all) lines.append(meetingLine(l));
    meeting.clock = setInterval(() => { if (meeting && meeting.state.recording) meetingRefresh(); }, 1000);
    place('meeting', el, 'meeting', 0);
    meetingPeople();
    meetingVoices(true);
    meetingRefresh();
    lines.scrollTop = lines.scrollHeight;
  }

  function onMeetingEvent(ev) {
    if (!meeting || ev.meetingId !== meeting.id || !meeting.el.isConnected) return;
    const st = meeting.state;
    if (ev.type === 'meeting_line') {
      meetingAddLine(ev.line);
      meetingRefresh();
    } else if (ev.type === 'meeting_relabel') {
      for (const n of meeting.lines.querySelectorAll('.w-mt-line')) {
        if (!ev.clusters.includes(n.dataset.cluster)) continue;
        n.classList.remove('unknown');
        const who = n.querySelector('.w-mt-who');
        who.textContent = ev.who.name;
        who.style.color = speakerColor(ev.who);
      }
    } else if (ev.type === 'meeting_update') {
      for (const k of ['recording', 'ended', 'engine', 'summary', 'doc', 'file', 'busy']) if (k in ev) st[k] = ev[k];
      if ('title' in ev && document.activeElement !== meeting.title) { meeting.title.value = ev.title; meeting.sentTitle = ev.title; }
      if ('people' in ev) { st.people = ev.people; meetingPeople(); }
      if ('voices' in ev) { st.voices = ev.voices; meetingVoices(); }
      meetingRefresh();
    }
  }

  K.register('meeting', renderMeeting);
  for (const t of ['meeting_line', 'meeting_update', 'meeting_relabel']) K.on(t, onMeetingEvent);
  K.onClose('meeting', () => { if (meeting) clearInterval(meeting.clock); });
  K.vanish('meeting');
})();
