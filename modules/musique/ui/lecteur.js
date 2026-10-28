// Plugin Musique interne de JARVIS (Lecteur Audio Intégré)
// Permet à Jarvis de jouer directement de la musique dans l'application
// sans dépendre d'une application externe Windows.

(function () {
  const J = (window.JARVIS = window.JARVIS || {});
  const bridge = window.jarvisBridge;

  // Playlist intégrée par défaut : flux audio haute fidélité streaming et webradios immersives
  const DEFAULT_PLAYLIST = [
    {
      id: 'synthwave',
      title: 'Nightwave Plaza',
      artist: 'Synthwave & Cyberpunk Radio',
      album: 'Jarvis Audio Stream',
      playlist: 'Radio Cyberpunk & Synthwave',
      thumbnail: 'https://radio.plaza.one/static/img/meta.png',
      src: 'https://radio.plaza.one/mp3',
      isStream: true
    },
    {
      id: 'groovesalad',
      title: 'Groove Salad Chillout',
      artist: 'SomaFM Ambient Lounge',
      album: 'Jarvis Audio Stream',
      playlist: 'Ambiance Lounge & Chillout',
      thumbnail: 'https://somafm.com/img3/groovesalad-400.jpg',
      src: 'https://ice1.somafm.com/groovesalad-128-mp3',
      isStream: true
    },
    {
      id: 'defcon',
      title: 'DEF CON Tech Pulse',
      artist: 'Electro & Hacker Beats',
      album: 'Jarvis Audio Stream',
      playlist: 'Electro & Hacker Beats',
      thumbnail: 'https://somafm.com/img3/defcon-400.jpg',
      src: 'https://ice1.somafm.com/defcon-128-mp3',
      isStream: true
    },
    {
      id: 'dronezone',
      title: 'Drone Zone Deep Space',
      artist: 'Atmospheric Sci-Fi Soundscape',
      album: 'Jarvis Audio Stream',
      playlist: 'Atmosphères Spatiales Sci-Fi',
      thumbnail: 'https://somafm.com/img3/dronezone-400.jpg',
      src: 'https://ice1.somafm.com/dronezone-128-mp3',
      isStream: true
    },
    {
      id: 'beatblender',
      title: 'Beat Blender',
      artist: 'Electronic & Deep Beats',
      album: 'Jarvis Audio Stream',
      playlist: 'Electronic & Deep Beats',
      thumbnail: 'https://somafm.com/img3/beatblender-400.jpg',
      src: 'https://ice1.somafm.com/beatblender-128-mp3',
      isStream: true
    }
  ];

  let playlist = [...DEFAULT_PLAYLIST];
  let currentIndex = 0;
  let isPlaying = false;
  let baseVolume = 0.80;
  let currentVolume = baseVolume;
  let isDucked = false;

  // Création de l'élément Audio HTML5
  const audio = new Audio();
  audio.id = 'jarvis-internal-audio';
  audio.preload = 'none';
  audio.volume = baseVolume;
  audio.crossOrigin = 'anonymous';

  // Synthétiseur d'ambiance Web Audio en repli 100% hors-ligne
  let offlineAudioCtx = null;
  let offlineNodes = [];
  let isOfflineSynthPlaying = false;

  function startOfflineSynth() {
    try {
      if (isOfflineSynthPlaying) return;
      const AudioCtx = window.AudioContext || window.webkitAudioContext;
      if (!AudioCtx) return;
      offlineAudioCtx = new AudioCtx();
      const masterGain = offlineAudioCtx.createGain();
      masterGain.gain.setValueAtTime(0.12 * currentVolume, offlineAudioCtx.currentTime);
      masterGain.connect(offlineAudioCtx.destination);

      // Accord ambiant apaisant (accords cinématiques en La mineur 9ème)
      const freqs = [110, 164.81, 220, 261.63, 329.63, 493.88];
      offlineNodes = [];

      freqs.forEach((f, idx) => {
        const osc = offlineAudioCtx.createOscillator();
        const filter = offlineAudioCtx.createBiquadFilter();
        const gain = offlineAudioCtx.createGain();

        osc.type = idx % 2 === 0 ? 'sine' : 'triangle';
        osc.frequency.setValueAtTime(f, offlineAudioCtx.currentTime);

        filter.type = 'lowpass';
        filter.frequency.setValueAtTime(450 + idx * 80, offlineAudioCtx.currentTime);

        gain.gain.setValueAtTime(0.15 / freqs.length, offlineAudioCtx.currentTime);

        osc.connect(filter);
        filter.connect(gain);
        gain.connect(masterGain);
        osc.start();
        offlineNodes.push(osc);
      });

      isOfflineSynthPlaying = true;
    } catch (e) {
      console.warn('[musique] Synthétiseur hors-ligne non disponible:', e);
    }
  }

  function stopOfflineSynth() {
    if (!isOfflineSynthPlaying) return;
    try {
      offlineNodes.forEach((n) => {
        try { n.stop(); n.disconnect(); } catch {}
      });
      offlineNodes = [];
      if (offlineAudioCtx && offlineAudioCtx.state !== 'closed') {
        offlineAudioCtx.close().catch(() => {});
      }
      isOfflineSynthPlaying = false;
    } catch {}
  }

  // Chargement des pistes locales depuis data/music
  async function loadLocalTracks() {
    if (bridge && bridge.getLocalMusic) {
      try {
        const local = await bridge.getLocalMusic();
        if (Array.isArray(local) && local.length > 0) {
          playlist = [...local, ...DEFAULT_PLAYLIST];
        }
      } catch (err) {
        console.warn('[musique] Erreur chargement pistes locales:', err);
      }
    }
  }

  // Émission de l'état vers le widget musique et l'orchestrateur
  function emitState() {
    const track = playlist[currentIndex] || DEFAULT_PLAYLIST[0];
    const duration = audio.duration && Number.isFinite(audio.duration) ? Math.round(audio.duration * 1000) : 0;
    const position = audio.currentTime && Number.isFinite(audio.currentTime) ? Math.round(audio.currentTime * 1000) : 0;

    const state = {
      active: true,
      status: isPlaying ? 'playing' : 'paused',
      title: track.title,
      artist: track.artist || 'JARVIS Music',
      album: track.album || 'Lecteur interne',
      playlist: track.playlist || track.album || 'Radio & Flux Audio',
      thumbnail: track.thumbnail || '',
      duration,
      position,
      canNext: true,
      canPrevious: true,
      app: 'JARVIS AUDIO',
      source: track.isStream ? 'Radio Web' : 'Fichier Local',
      updatedAt: Date.now()
    };

    // Mise à jour directe du widget musique HUD
    if (window.JARVIS && window.JARVIS.widgets) {
      const w = window.JARVIS.widgets;
      if (isPlaying && w.openMusic) w.openMusic();
    }

    // Transmission au pont IPC pour les autres composants
    if (bridge && bridge.sendMusicState) {
      bridge.sendMusicState(state);
    }

    // Événement interne relayé à onMedia de widgets.js
    if (typeof window.dispatchEvent === 'function') {
      window.dispatchEvent(new CustomEvent('jarvis:media-state', { detail: state }));
    }
  }

  // Recherche de piste ou webradio par mot-clé ou titre/playlist
  function findTrackByQuery(query) {
    if (!query || typeof query !== 'string') return -1;
    const q = query.toLowerCase().normalize('NFD').replace(/[\u0300-\u036f]/g, '').trim();

    // Mots-clés sémantiques pour les ambiances courantes
    if (/\b(?:bonne nuit|sommeil|dodo|dormir|calme|detente|relax|nuit|soiree|chill|lounge|douce?)\b/i.test(q)) {
      const idx = playlist.findIndex((p) => p.id === 'groovesalad');
      if (idx !== -1) return idx;
    }
    if (/\b(?:espace|spatial|etoile|deep space|cosmos|ambian(?:ce|te)|sci-?fi|drone|zen)\b/i.test(q)) {
      const idx = playlist.findIndex((p) => p.id === 'dronezone');
      if (idx !== -1) return idx;
    }
    if (/\b(?:tech|pulse|hacker|code|cyber|dev|hack|defcon)\b/i.test(q)) {
      const idx = playlist.findIndex((p) => p.id === 'defcon');
      if (idx !== -1) return idx;
    }
    if (/\b(?:synth|wave|synthwave|retro|80s|cyberpunk|plaza|nightwave)\b/i.test(q)) {
      const idx = playlist.findIndex((p) => p.id === 'synthwave');
      if (idx !== -1) return idx;
    }
    if (/\b(?:electro|dance|club|party|fete|beat|blender)\b/i.test(q)) {
      const idx = playlist.findIndex((p) => p.id === 'beatblender');
      if (idx !== -1) return idx;
    }

    // Recherche textuelle dans le titre, artiste, album ou nom de playlist
    const found = playlist.findIndex((p) => {
      const hay = `${p.title || ''} ${p.artist || ''} ${p.album || ''} ${p.playlist || ''} ${p.id || ''}`
        .toLowerCase()
        .normalize('NFD')
        .replace(/[\u0300-\u036f]/g, '');
      return hay.includes(q);
    });
    return found;
  }

  // Contrôles de lecture
  async function play(indexOrQuery) {
    if (typeof indexOrQuery === 'number' && indexOrQuery >= 0 && indexOrQuery < playlist.length) {
      currentIndex = indexOrQuery;
    } else if (typeof indexOrQuery === 'string' && indexOrQuery.trim()) {
      const found = findTrackByQuery(indexOrQuery);
      if (found !== -1) {
        currentIndex = found;
      }
    }

    const track = playlist[currentIndex] || playlist[0];
    if (!track) return;

    if (audio.src !== track.src) {
      audio.src = track.src;
      audio.load();
    }

    try {
      await audio.play();
      isPlaying = true;
      stopOfflineSynth();
    } catch (err) {
      console.warn('[musique] Échec lecture audio stream, bascule sur synthétiseur d\'ambiance:', err);
      startOfflineSynth();
      isPlaying = true;
    }

    // Morphing visuel du cœur en note de musique
    if (J.triggerOrbMorph) {
      J.triggerOrbMorph('music', 2800);
    }

    emitState();
  }

  function pause() {
    audio.pause();
    stopOfflineSynth();
    isPlaying = false;
    emitState();
  }

  function stop() {
    audio.pause();
    try { audio.currentTime = 0; } catch {}
    stopOfflineSynth();
    isPlaying = false;
    emitState();
  }

  function toggle() {
    if (isPlaying) {
      pause();
    } else {
      play();
    }
  }

  function next() {
    currentIndex = (currentIndex + 1) % playlist.length;
    if (isPlaying) {
      play(currentIndex);
    } else {
      emitState();
    }
  }

  function previous() {
    currentIndex = (currentIndex - 1 + playlist.length) % playlist.length;
    if (isPlaying) {
      play(currentIndex);
    } else {
      emitState();
    }
  }

  function setVolume(v) {
    baseVolume = Math.max(0, Math.min(1, v));
    currentVolume = isDucked ? baseVolume * 0.20 : baseVolume;
    audio.volume = currentVolume;
  }

  // Ducking audio : baisse douce du volume quand Jarvis parle
  function updateDucking() {
    const v = J.voice;
    const isSpeaking = !!(v && v.speaking);

    if (isSpeaking && !isDucked) {
      isDucked = true;
      currentVolume = baseVolume * 0.20; // Réduction à 20%
      audio.volume = currentVolume;
    } else if (!isSpeaking && isDucked) {
      isDucked = false;
      // Remontée progressive du volume
      const step = 0.05;
      const interval = setInterval(() => {
        if (isDucked) { clearInterval(interval); return; }
        currentVolume = Math.min(baseVolume, currentVolume + step);
        audio.volume = currentVolume;
        if (currentVolume >= baseVolume) clearInterval(interval);
      }, 50);
    }
  }

  // Événements Audio
  audio.addEventListener('play', () => { isPlaying = true; emitState(); });
  audio.addEventListener('pause', () => { isPlaying = false; emitState(); });
  audio.addEventListener('ended', () => { next(); });
  audio.addEventListener('timeupdate', () => { emitState(); });
  audio.addEventListener('error', () => {
    console.warn('[musique] Erreur flux audio, passage au suivant...');
    if (isPlaying) {
      setTimeout(() => next(), 1200);
    }
  });

  // Surveillance régulière du ducking et de l'état
  setInterval(updateDucking, 200);

  // Commandes directes
  function handleCommand(cmd, target) {
    const c = String(cmd || '').toLowerCase().trim();
    if (c === 'play') play(target);
    else if (c === 'pause') pause();
    else if (c === 'stop') stop();
    else if (c === 'toggle') toggle();
    else if (c === 'next') next();
    else if (c === 'previous') previous();
    else if (c === 'status') emitState();
  }

  // Écoute des commandes de l'orchestrateur
  if (bridge && bridge.onTaskEvent) {
    bridge.onTaskEvent((ev) => {
      if (!ev) return;
      // Deux pages HUD (fenêtre classique, Bureau JARVIS) : une seule joue la musique.
      if (ev.type === 'music_control' && (!bridge.isLeader || bridge.isLeader())) {
        handleCommand(ev.action, ev.playlist || ev.query || ev.target);
      }
    });
  }

  // Chargement initial
  loadLocalTracks();

  // Export vers l'API JARVIS
  J.musicPlayer = {
    play,
    pause,
    stop,
    toggle,
    next,
    previous,
    setVolume,
    getVolume: () => baseVolume,
    command: handleCommand,
    getState: () => ({
      isPlaying,
      track: playlist[currentIndex] || DEFAULT_PLAYLIST[0],
      playlist
    })
  };
})();
