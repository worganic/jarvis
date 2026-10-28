// Sous-titres de Jarvis et boutons « micro » / « son », en haut au milieu du HUD.
//
// - Bouton « micro » : coupe ou rétablit l'écoute orale (le moteur vocal n'entend plus rien ;
//   on écrit ses demandes dans la console). Conservé si le moteur redémarre (voice-link.js).
// - Bouton « son » : coupe la voix de Jarvis ou la rétablit (orchestrator › toggleSilence :
//   sourdine complète, ou retour au mode automatique).
// - Tant que le moteur vocal n'est pas lancé (hors ligne, modèle en chargement, écoute orale
//   désactivée), les deux boutons sont grisés et inactifs.
// - Les réponses de Jarvis ne s'écrivent ici que lorsqu'on ne les entend pas : son coupé
//   (bouton, sourdine automatique) ou service vocal hors ligne. Quand il parle, rien ne s'affiche.
//   Réponse lue au fil de l'eau : complétée morceau par morceau, puis effacée après quelques secondes.
(function () {
  const bridge = window.jarvisBridge;
  const J = (window.JARVIS = window.JARVIS || {});
  const box = document.getElementById('jarvis-caption');
  if (!box) return;
  const soundBtn = box.querySelector('.cap-voice');
  const micBtn = box.querySelector('.cap-mic');
  const text = box.querySelector('.cap-text');

  let state = { mode: 'auto', isMuted: false, reason: '' };
  let micMuted = false;
  let current = { task: null, at: 0 };
  let hideTimer = null;

  function mutedNow() {
    return state.mode === 'on' || state.isMuted;
  }

  // Jarvis ne peut pas être entendu : son coupé, ou moteur vocal absent / désactivé.
  function unheard(ev) {
    const v = J.voice;
    return Boolean(ev && ev.muted) || mutedNow() || !v || v.disabled || !v.connected;
  }

  // Moteur vocal lancé : connecté, modèle chargé, écoute orale active.
  function voiceUp() {
    const v = J.voice;
    return Boolean(v && v.connected && v.ready && !v.disabled);
  }

  function paint(btn, icon, label, muted, title) {
    btn.querySelector('.cap-sound-icon').textContent = icon;
    btn.querySelector('.cap-sound-label').textContent = label;
    btn.classList.toggle('muted', muted);
    btn.title = title;
  }

  function paintButtons() {
    const forced = state.mode === 'on';
    const auto = !forced && state.isMuted;
    paint(soundBtn, mutedNow() ? '🔇' : '🔊', forced ? 'MUET' : auto ? 'MUET AUTO' : 'SON', mutedNow(), forced
      ? 'Voix de Jarvis coupée : il écrit ses réponses ici. Clic : rétablir le son.'
      : auto
        ? `Sourdine automatique (${state.reason || 'application en cours'}). Clic : couper le son en permanence.`
        : 'Couper la voix de Jarvis (il écrira ses réponses ici)');
    paint(micBtn, micMuted ? '🚫' : '🎙', micMuted ? 'MICRO COUPÉ' : 'MICRO', micMuted, micMuted
      ? 'Micro coupé : Jarvis n’écoute plus. Clic : rétablir l’écoute orale.'
      : 'Couper le micro (Jarvis n’écoute plus ; écris tes demandes dans la console)');
    const up = voiceUp();
    for (const btn of [soundBtn, micBtn]) {
      btn.disabled = !up;
      btn.classList.toggle('offline', !up);
      if (!up) btn.title = 'Moteur vocal non lancé : bouton indisponible.';
    }
    wasUp = up;
  }
  let wasUp = null;

  function scheduleHide(chars) {
    clearTimeout(hideTimer);
    const base = Math.min(25000, 4000 + chars * 60);
    hideTimer = setTimeout(() => {
      box.classList.remove('has-text');
    }, base * 2);
  }

  function show(ev) {
    const said = String(ev.text || '').trim();
    if (!said) return;
    if (!unheard(ev)) {
      // Jarvis le dit à voix haute : pas de sous-titre (et on retire l'éventuel précédent).
      clearTimeout(hideTimer);
      box.classList.remove('has-text');
      current = { task: null, at: 0 };
      return;
    }
    const now = Date.now();
    // Réponse lue au fil de l'eau : les morceaux d'une même tâche se suivent dans le même sous-titre.
    const continues = ev.stream && ev.task && current.task === ev.task && now - current.at < 30000;
    const next = continues ? `${text.textContent} ${said}` : said;
    // Au-delà de ~320 caractères, seule la fin reste (le début a été lu).
    text.textContent = next.length > 320 ? `…${next.slice(-318)}` : next;
    current = { task: ev.task || null, at: now };
    box.classList.add('has-text');
    box.classList.add('muted-text');
    scheduleHide(text.textContent.length);
  }

  async function press(btn, action) {
    if (!bridge) return;
    btn.disabled = true;
    try { await action(); } catch {}
    paintButtons();
  }

  soundBtn.addEventListener('click', () => press(soundBtn, async () => {
    if (!bridge.voiceSilence) return;
    const r = await bridge.voiceSilence();
    if (r) state = { ...state, mode: r.mode, isMuted: Boolean(r.muted), reason: r.reason || '' };
  }));

  micBtn.addEventListener('click', () => press(micBtn, async () => {
    if (!bridge.voiceMute) return;
    const r = await bridge.voiceMute(!micMuted);
    if (r && typeof r.muted === 'boolean') micMuted = r.muted;
  }));

  paintButtons();
  // Le moteur vocal démarre, s'arrête ou finit de charger son modèle : les boutons suivent.
  setInterval(() => { if (voiceUp() !== wasUp) paintButtons(); }, 400);
  if (!bridge) return;
  // État initial du micro (Jarvis déjà lancé, fenêtre rechargée).
  if (bridge.voiceState) {
    bridge.voiceState().then((s) => {
      if (s && typeof s.muted === 'boolean') { micMuted = s.muted; paintButtons(); }
    }).catch(() => {});
  }
  bridge.onTaskEvent((ev) => {
    if (!ev) return;
    if (ev.type === 'say' && !ev.ack) show(ev);
    else if (ev.type === 'status' && ev.contextMute) {
      state = { ...state, ...ev.contextMute };
      paintButtons();
    } else if (ev.type === 'voice_state' && typeof ev.muted === 'boolean' && ev.connected !== false) {
      micMuted = ev.muted;
      paintButtons();
    } else if (ev.type === 'voice_stop' || ev.type === 'stopped') {
      scheduleHide(0);
    }
  });
})();
