// État de la voix dans la fenêtre (v2).
//
// En v1, ce fichier était un client du service vocal (ws://127.0.0.1:8787) : il recevait les
// phrases, les faisait prononcer et relayait tout à l'orchestrateur. En v2, le noyau fait ce
// travail (core/voice-link.js), même fenêtre masquée ; ce module n'est plus qu'un REFLET : il
// garde l'objet J.voice attendu par app.js, alimenté par les événements voice_* du noyau.
(function () {
  const J = (window.JARVIS = window.JARVIS || {});
  const bridge = window.jarvisBridge;

  const voice = {
    disabled: false,
    connected: false,
    capturing: false,
    speaking: false,
    muted: false,
    ready: false,
    model: '',
    name: 'Denise',
    rate: '+0%',
    partial: '',
    lastFinal: '',
    // Empreinte vocale : qui parle pendant la phrase (partielles) et verdict de la dernière phrase
    // ({ state: 'known' | 'doubt' | 'unknown', name, similarity }). Lus par app.js (couleur du texte).
    partialWho: null,
    finalWho: null,
    speaker: null,
    backend: 'engine',
    // « Jarvis » reconnu : actif pendant la phrase en cours, puis WAKE_HOLD_MS après (le temps de
    // dire la suite après une pause). Lu par app.js pour teinter le contour du cœur en rouge.
    wakeActive: false,
    wakeReleaseAt: 0,
    // Le noyau prononce lui-même les réponses de l'orchestrateur : rien à faire ici.
    say() {},
    sayEnd() {},
    reopenAudio() {},
    tune() {},
    setVoice() {},
    setVocabulary() {},
    command() {},
    startEnrollSpeaker() {},
    cancelEnrollSpeaker() {},
    deleteSpeaker() {},
    renameSpeaker() {},
    setPrimarySpeaker() {},
    setSpeakerEnabled() {},
    setSpeakerThreshold() {},
    resetSpeaker() {},
    refreshSpeakerStatus() {},
    // Une nouvelle demande tapée coupe la parole en cours ; F3 = sourdine.
    stop() { if (bridge && bridge.voiceStop) bridge.voiceStop(); },
    toggleMute() { if (bridge && bridge.voiceMute) bridge.voiceMute(); }
  };
  J.voice = voice;

  // Verdict de l'empreinte d'une phrase finale, au format des partielles.
  function whoOf(ev) {
    const c = ev.speakerCheck;
    if (c && c.confirmed) return { state: 'known', name: ev.speaker && ev.speaker.name, similarity: c.similarity, confirmed: true };
    if (c && c.decision === 'accepted') return { state: 'known', name: ev.speaker ? ev.speaker.name : c.candidate && c.candidate.name, similarity: c.similarity };
    if (c && c.decision === 'doubt') return { state: 'doubt', name: c.candidate && c.candidate.name, similarity: c.similarity };
    if (c && (c.decision === 'rejected' || c.decision === 'unknown')) return { state: 'unknown', name: c.candidate && c.candidate.name, similarity: c.similarity };
    if (ev.speaker && ev.speaker.name) return { state: 'known', name: ev.speaker.name, similarity: ev.speaker.similarity };
    return null;
  }

  const WAKE_HOLD_MS = 2500;
  const WAKE_TEXT = /\b(?:jarvis|jervis|jarvice|charvis)\b/i;
  function markWake() {
    voice.wakeActive = true;
    voice.wakeReleaseAt = performance.now() + WAKE_HOLD_MS;
  }

  function applyState(msg) {
    const wasSpeaking = voice.speaking;
    const wasCapturing = voice.capturing;
    voice.connected = msg.connected !== false;
    voice.ready = !!msg.ready;
    voice.capturing = !!msg.capturing;
    voice.speaking = !!msg.speaking;
    voice.muted = !!msg.muted;
    voice.error = msg.error || null;
    if (msg.model) voice.model = msg.model;
    if (msg.voice_name) voice.name = msg.voice_name;
    if (msg.rate) voice.rate = msg.rate;
    if (wasSpeaking !== voice.speaking && J.onSpeakingChange) J.onSpeakingChange(voice.speaking);
    if (wasCapturing && !voice.capturing && voice.wakeActive) voice.wakeReleaseAt = performance.now() + WAKE_HOLD_MS;
    if (voice.speaking) voice.wakeActive = false; // Jarvis répond : la demande est close
    if (!voice.capturing && voice.partial) {
      const stamp = voice.partialAt;
      setTimeout(() => { if (!voice.capturing && voice.partialAt === stamp) voice.partial = ''; }, 2000);
    }
    if (J.onVoiceState) J.onVoiceState({ ...msg, backend: 'engine' });
    if (msg.speaker) {
      voice.speaker = msg.speaker;
      if (J.onSpeakerStatus) J.onSpeakerStatus(msg.speaker);
    }
  }

  if (bridge) {
    bridge.onTaskEvent((ev) => {
      switch (ev.type) {
        case 'voice_state':
          if (ev.capturing && !voice.capturing) voice.partialWho = null; // nouvelle phrase : verdict à refaire
          applyState(ev);
          break;
        case 'voice_wake':
          markWake();
          break;
        case 'voice_partial':
          if (WAKE_TEXT.test(ev.text || '')) markWake();
          voice.partial = ev.text;
          voice.partialAt = performance.now();
          break;
        case 'voice_who':
          if (voice.capturing || voice.partial) voice.partialWho = ev.who || null;
          break;
        case 'voice_waiting':
          // Jarvis attend la fin de la phrase : la transcription reste affichée, suivie de « … ».
          voice.partial = `${String(ev.text || '').replace(/[\s.…]+$/, '')} …`;
          voice.partialAt = performance.now();
          break;
        case 'voice_final':
          voice.partial = '';
          if (ev.echo) break; // Jarvis s'est entendu : rien à afficher
          if (ev.accepted && WAKE_TEXT.test(ev.text || '')) markWake();
          voice.lastFinal = ev.text;
          voice.finalAt = performance.now();
          voice.finalWho = whoOf(ev);
          voice.partialWho = null;
          // Voix non reconnue : la phrase attend qu'on dise qui parlait (speaker-ask.js).
          if (ev.pendingId && J.speakerAsk) J.speakerAsk.show(ev);
          if (J.onUtterance) J.onUtterance(ev.text, { speaker: ev.speaker || null, accepted: ev.accepted, who: voice.finalWho, ask: ev.pendingId ? ev : null });
          break;
        case 'voice_backend':
          if (ev.state === 'paused') voice.disabled = true;
          else if (ev.state === 'starting' || ev.state === 'ready') voice.disabled = false;
          break;
        case 'speaker_enroll_progress':
          if (J.onSpeakerEnrollProgress) J.onSpeakerEnrollProgress(ev);
          break;
        case 'speaker_rejected':
          if (J.onSpeakerRejected) J.onSpeakerRejected(ev);
          break;
        case 'speaker_status':
        case 'speaker_deleted':
        case 'speaker_renamed':
        case 'speaker_updated':
          voice.speaker = ev.status || ev;
          if (J.onSpeakerStatus) J.onSpeakerStatus(voice.speaker);
          break;
        default:
          break;
      }
    });
    bridge.voiceState().then((st) => st && applyState(st)).catch(() => {});
  }

  // Optimisation › écoute orale : c'est le noyau qui arrête le moteur ; ici, l'affichage suit.
  J.voiceEnable = function (on) {
    voice.disabled = !on;
    if (!on) {
      voice.ready = false;
      voice.capturing = false;
      voice.speaking = false;
      voice.partial = '';
    }
  };
})();
