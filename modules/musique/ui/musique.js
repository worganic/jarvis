// Module Musique : la card du lecteur (pochette, titre, progression, boutons, niveaux et volume de l'application qui joue).
// Sources : le lecteur interne (ui/music.js du module, événement de page jarvis:media-state) et les applications de
// Windows (Spotify…, événement « media » du noyau, orch/media.rs). La card reste visible tant que la musique joue ;
// une minute sans lecture, elle se réduit en bouton. Niveaux et volume : module Audio (audioApp, audioAppVolume).
(function () {
  const J = (window.JARVIS = window.JARVIS || {});
  const bridge = window.jarvisBridge;
  const K = J.widgetKit;
  if (!K) return;
  const { h, place, cards, close, gripHandle, appName, updateSourceBadge, ICONS, fmtTime, setFolded } = K;

  let media = { active: false };
  let musicOpen = false;
  let musicHideTimer = null;


  const DEFAULT_MUSIC_COVER = 'data:image/svg+xml;utf8,<svg xmlns="http://www.w3.org/2000/svg" width="120" height="120" viewBox="0 0 120 120"><defs><linearGradient id="g" x1="0%" y1="0%" x2="100%" y2="100%"><stop offset="0%" stop-color="%23042436"/><stop offset="100%" stop-color="%23010b14"/></linearGradient><linearGradient id="c" x1="0%" y1="0%" x2="100%" y2="100%"><stop offset="0%" stop-color="%2338e1ff"/><stop offset="100%" stop-color="%237ef2e2"/></linearGradient></defs><rect width="120" height="120" rx="10" fill="url(%23g)" stroke="%2338e1ff" stroke-opacity="0.4" stroke-width="2"/><circle cx="60" cy="60" r="34" fill="none" stroke="%2338e1ff" stroke-opacity="0.3" stroke-width="2"/><circle cx="60" cy="60" r="22" fill="none" stroke="%2338e1ff" stroke-opacity="0.5" stroke-dasharray="4,4"/><circle cx="60" cy="60" r="10" fill="url(%23c)"/><path d="M54 48 L72 38 L72 58 A8 8 0 1 1 64 52 L64 45 L56 49 L56 64 A8 8 0 1 1 48 58 Z" fill="url(%23c)" opacity="0.9"/></svg>';

  function mediaButton(label, cmd, styleKind, enabled, tip) {
    const btnClass = `w-btn${styleKind === true || styleKind === 'primary' ? ' primary' : (styleKind === 'stop' ? ' stop' : '')}`;
    const defaultTips = { previous: 'Avant (Précédent)', toggle: 'Play (Lecture / Pause)', stop: 'Stop (Arrêt)', next: 'Après (Suivant)' };
    return h('button', {
      class: btnClass,
      title: tip || defaultTips[cmd] || cmd,
      disabled: enabled === false,
      onclick: () => {
        // Le widget affiche une application externe (Spotify…) : c'est elle que les boutons
        // pilotent, sans réveiller le lecteur interne (qui lancerait une webradio par-dessus).
        const external = media && media.app && !isInternalApp(media.app);
        if (external) {
          if (bridge && bridge.mediaCommand) bridge.mediaCommand(cmd, /spotify/i.test(media.app) ? 'spotify' : media.app);
          return;
        }
        if (J.musicPlayer && J.musicPlayer.command) {
          J.musicPlayer.command(cmd);
        }
        if (bridge && bridge.mediaCommand) {
          bridge.mediaCommand(cmd);
        }
      }
    }, label);
  }

  function renderMusic() {
    const m = media || {};
    const playing = m.status === 'playing';

    // Si le widget existe déjà, mettre à jour in-place
    const existingCard = cards.get('music');
    if (existingCard && existingCard.el && existingCard.el.isConnected) {
      const el = existingCard.el;
      const playlistEl = el.querySelector('.w-playlist');
      const songEl = el.querySelector('.w-song');
      const artistEl = el.querySelector('.w-artist');
      const coverEl = el.querySelector('.w-cover');
      const labelEl = el.querySelector('.w-label');
      const sourceEl = el.querySelector('.w-source');
      const playBtn = el.querySelector('.w-btn.primary');
      const stopBtn = el.querySelector('.w-btn.stop');
      const trackEl = el.querySelector('.w-track');
      const controlsEl = el.querySelector('.w-controls');

      // Si le DOM est incomplet (ex: initialisation fantôme), reconstruire immédiatement la structure complète
      if (!playlistEl || !songEl || !coverEl || !trackEl || !controlsEl) {
        rebuildMusicWidget(m, playing);
        return;
      }

      const playlistName = m.playlist || m.album || (appName(m.app) ? appName(m.app) : 'Radio & Flux Audio');
      const songTitle = m.title || (m.active ? 'Lecture en cours' : 'Aucune musique en cours');
      const artistName = m.artist || (m.active ? 'Jarvis Audio' : 'Dis « mets la musique »');
      const coverSrc = m.thumbnail || DEFAULT_MUSIC_COVER;

      // Rien en cours : le fantôme de la card ne garde pas « Aucune musique en cours » (style.css › w-idle).
      el.classList.toggle('w-idle', !m.active);
      if (labelEl) labelEl.textContent = m.active ? (playing ? 'EN ÉCOUTE' : 'EN PAUSE') : 'MUSIQUE';
      if (sourceEl) updateSourceBadge(sourceEl, m.app);
      if (playlistEl && playlistEl.textContent !== playlistName) {
        playlistEl.textContent = playlistName;
        playlistEl.title = playlistName;
      }
      if (songEl && songEl.textContent !== songTitle) {
        songEl.textContent = songTitle;
        songEl.title = songTitle;
      }
      if (artistEl && artistEl.textContent !== artistName) {
        artistEl.textContent = artistName;
        artistEl.title = artistName;
      }
      if (coverEl && coverEl.src !== coverSrc) {
        coverEl.src = coverSrc;
      }
      if (playBtn) {
        playBtn.textContent = playing ? '⏸' : '▶';
        playBtn.disabled = !m.active;
      }
      if (stopBtn) {
        stopBtn.disabled = !m.active;
      }

      // Règle absolue : tant que la musique est active (en lecture), le widget DOIT RESTER À 100% VISIBLE (active, 0% transparence)
      if (m.active && playing) {
        clearTimeout(existingCard.fadeTimer);
        clearTimeout(existingCard.timer);
        existingCard.state = 'active';
        el.classList.remove('w-ghost', 'w-fading');
        el.classList.add('w-active');
      } else if (existingCard.state !== 'active' && musicOpen) {
        clearTimeout(existingCard.fadeTimer);
        existingCard.state = 'active';
        el.classList.remove('w-ghost', 'w-fading');
        el.classList.add('w-active');
      }

      tickProgress();
      return;
    }

    rebuildMusicWidget(m, playing);
  }

  function rebuildMusicWidget(m, playing) {
    const playlistName = m.playlist || m.album || (appName(m.app) ? appName(m.app) : 'Radio & Flux Audio');
    const songTitle = m.title || (m.active ? 'Lecture en cours' : 'Aucune musique en cours');
    const artistName = m.artist || (m.active ? 'Jarvis Audio' : 'Dis « mets la musique »');
    const coverSrc = m.thumbnail || DEFAULT_MUSIC_COVER;

    const playlistEl = h('div', { class: 'w-playlist', title: playlistName }, playlistName);
    const songEl = h('div', { class: 'w-song', title: songTitle }, songTitle);
    const artistEl = h('div', { class: 'w-artist', title: artistName }, artistName);
    const coverEl = h('img', {
      class: 'w-cover',
      src: coverSrc,
      alt: 'Pochette',
      onerror: (e) => { e.target.src = DEFAULT_MUSIC_COVER; }
    });

    const sourceEl = h('span', { class: 'w-source' });
    updateSourceBadge(sourceEl, m.app);

    const el = h('div', { class: `widget w-music${m.active ? '' : ' w-idle'}` },
      h('div', { class: 'w-head' },
        gripHandle(),
        h('span', { class: 'w-icon' }, ICONS.music),
        h('span', { class: 'w-label' }, m.active ? (playing ? 'EN ÉCOUTE' : 'EN PAUSE') : 'MUSIQUE'),
        sourceEl,
        h('button', { class: 'w-close', title: 'Fermer', onclick: () => close('music') }, '×')),
      h('div', { class: 'w-track' },
        coverEl,
        h('div', { class: 'w-meta' }, playlistEl, songEl, artistEl),
        volumeBlock()),
      h('div', { class: 'w-progress' }, h('div', { class: 'w-bar' }), h('span', { class: 'w-time' })),
      h('div', { class: 'w-controls' },
        mediaButton('⏮', 'previous', false, m.canPrevious !== false, 'Avant (Précédent)'),
        mediaButton(playing ? '⏸' : '▶', 'toggle', true, m.active !== false, 'Play (Lecture / Pause)'),
        mediaButton('⏹', 'stop', 'stop', m.active !== false, 'Stop (Arrêt)'),
        mediaButton('⏭', 'next', false, m.canNext !== false, 'Après (Suivant)')));

    place('music', el, 'music', 0);
    tickProgress();
  }

  // Card musique : niveaux gauche / droite et volume de la seule application qui joue (Spotify, lecteur de Jarvis…),
  // jamais ceux de Windows entier ni d'une autre source (YouTube dans Chrome…). Sessions audio de Windows : module Audio
  // (méthodes audioApp, audioAppVolume) ; lecteur interne : son propre volume (music.js).
  const vu = { l: null, r: null, slider: null, holding: false, sent: 0, target: '' };
  const vuPct = (p) => Math.min(100, Math.round(Math.pow(Math.max(0, Number(p) || 0), 0.6) * 100));
  const musicInternal = () => !media || !media.app || isInternalApp(media.app);
  // Application suivie : « jarvis » pour le lecteur interne (WebView2 du HUD), sinon le nom donné par Windows.
  const musicTarget = () => (media && media.active ? (musicInternal() ? 'jarvis' : String(media.app)) : '');
  function paintSlider(pct) {
    if (!vu.slider || vu.holding || !Number.isFinite(pct)) return;
    vu.slider.value = String(Math.round(pct));
    vu.slider.disabled = false;
  }
  // Envoie au mélangeur l'application à suivre et relit son volume (le lecteur interne a le sien).
  function followMusicApp() {
    const target = musicTarget();
    vu.target = target;
    if (vu.slider) vu.slider.title = musicInternal() ? 'Volume du lecteur de Jarvis' : `Volume de ${appName(media.app) || 'l’application'} (mélangeur de volume de Windows)`;
    if (target === 'jarvis' && J.musicPlayer && J.musicPlayer.getVolume) paintSlider(J.musicPlayer.getVolume() * 100);
    if (!bridge || !bridge.call) return;
    bridge.call('audioApp', target).then((r) => {
      if (target !== 'jarvis' && r && r.ok) {
        if (Number.isFinite(r.volume)) paintSlider(r.volume);
        else if (vu.slider) vu.slider.disabled = true; // l'application n'a pas (encore) de son ouvert
      }
    }).catch(() => {});
  }
  function volumeBlock() {
    const bar = (title) => {
      const fill = h('div', { class: 'w-vu-fill' });
      return { fill, el: h('div', { class: 'w-vu', title }, fill) };
    };
    const l = bar('Niveau gauche de la musique');
    const r = bar('Niveau droit de la musique');
    const slider = h('input', { class: 'w-vol', type: 'range', min: '0', max: '100', step: '1', value: '0', title: 'Volume de la musique' });
    const send = () => {
      vu.sent = Date.now();
      const v = Number(slider.value);
      if (musicInternal()) { if (J.musicPlayer && J.musicPlayer.setVolume) J.musicPlayer.setVolume(v / 100); return; }
      if (bridge && bridge.call) bridge.call('audioAppVolume', v).catch(() => {});
    };
    slider.addEventListener('pointerdown', () => { vu.holding = true; });
    slider.addEventListener('input', () => { if (Date.now() - vu.sent > 90) send(); });
    slider.addEventListener('change', () => { vu.holding = false; send(); });
    slider.addEventListener('pointerup', () => { vu.holding = false; });
    slider.addEventListener('wheel', (e) => {
      e.preventDefault();
      slider.value = String(Math.max(0, Math.min(100, Number(slider.value) + (e.deltaY < 0 ? 2 : -2))));
      send();
    }, { passive: false });
    vu.l = l.fill; vu.r = r.fill; vu.slider = slider;
    J.musicMeter = true;
    if (bridge && bridge.audioWatch) bridge.audioWatch(true, true).then(followMusicApp).catch(() => {});
    return h('div', { class: 'w-volume' }, l.el, r.el, slider);
  }
  // Bail du compteur de niveaux : renouvelé tant que la card musique est ouverte (l'application suivie est renvoyée :
  // le mélangeur l'oublie s'il s'est arrêté entre-temps).
  setInterval(() => {
    const card = cards.get('music');
    const on = Boolean(card && card.el && card.el.isConnected && card.state === 'active' && !card.el.classList.contains('w-collapsed') && !document.hidden);
    J.musicMeter = on;
    if (on && bridge && bridge.audioWatch) bridge.audioWatch(true, true).then(followMusicApp).catch(() => {});
  }, 8000);

  // Progression interpolée entre deux relevés
  function tickProgress() {
    const card = cards.get('music');
    if (!card || card.state === 'ghost' || !card.el || typeof card.el.querySelector !== 'function') return;
    const bar = card.el.querySelector('.w-bar');
    const label = card.el.querySelector('.w-time');
    const m = media || {};
    if (!bar || !m.duration) {
      if (bar) bar.style.width = '0%';
      if (label) label.textContent = '';
      return;
    }
    let pos = m.position || 0;
    if (m.status === 'playing' && m.updatedAt) pos += Date.now() - m.updatedAt;
    pos = Math.min(pos, m.duration);
    bar.style.width = `${(100 * pos) / m.duration}%`;
    label.textContent = `${fmtTime(pos)} / ${fmtTime(m.duration)}`;
  }
  setInterval(tickProgress, 500);

  function openMusic() {
    musicOpen = true;
    if (bridge && bridge.widgetState) bridge.widgetState('music', true);
    renderMusic();
  }

  function isInternalApp(app) {
    return /^jarvis audio$/i.test(String(app || '').trim());
  }

  function onMedia(state) {
    const previous = media;
    // Le lecteur interne qui s'arrête (il se tait quand Spotify prend le relais) ne remplace pas
    // l'affichage d'une musique externe en cours de lecture.
    if (state && isInternalApp(state.app) && state.status !== 'playing'
      && previous && previous.active && previous.status === 'playing' && previous.app && !isInternalApp(previous.app)) {
      return;
    }
    media = state || { active: false };
    J.currentMedia = media;
    if (vu.slider && vu.slider.isConnected && musicTarget() !== vu.target) followMusicApp();
    if (media.status === 'playing' && !(previous && previous.status === 'playing') && K.isFolded('music')) setFolded('music', false);

    // Si une musique est active ou en cours de lecture, ouvrir et maintenir le widget 100% visible
    if (media.active || media.status === 'playing' || media.title) {
      musicOpen = true;
    }

    renderMusic();
    clearTimeout(musicHideTimer);

    // Ne lancer le timer de masquage doux que si la musique est réellement arrêtée/inactive
    if (!media.active || (media.status !== 'playing' && media.status !== 'paused')) {
      musicHideTimer = setTimeout(() => {
        if (!media.active || media.status !== 'playing') foldMusic();
      }, 60000);
    }
  }

  // Écoute de l'événement média interne (émis par music.js)
  if (typeof window.addEventListener === 'function') {
    window.addEventListener('jarvis:media-state', (e) => {
      if (e && e.detail) onMedia(e.detail);
    });
  }

  const musicPlaying = () => Boolean(media && media.active && media.status === 'playing');
  function foldMusic() {
    setFolded('music', true);
    musicOpen = false;
    if (bridge && bridge.widgetState) bridge.widgetState('music', false);
  }

  // Branchement sur le cadre commun des cards (ui/widgets.js › J.widgetKit).
  K.ownKey('music'); // numérotée par le HUD (100), pas par le noyau
  K.guardClose('music', musicPlaying); // fermée pendant la lecture : reste visible
  K.onClose('music', () => {
    if (musicPlaying()) return;
    musicOpen = false;
    if (bridge && bridge.widgetState) bridge.widgetState('music', false);
  });
  K.onIdle('music', { busy: musicPlaying, idle: foldMusic });
  K.register('music', openMusic);
  K.on('media', onMedia);
  K.on('audio_peak', (ev) => {
    if (!vu.l || !vu.l.isConnected) return;
    // Niveaux de l'application suivie seulement (al, ar) ; rien tant qu'elle n'est pas suivie ou n'a pas de son.
    const on = Boolean(vu.target) && ev.af === true;
    vu.l.style.height = `${on ? vuPct(ev.al) : 0}%`;
    vu.r.style.height = `${on ? vuPct(ev.ar) : 0}%`;
  });
  // État du lecteur interne (ui/music.js du module).
  window.addEventListener('jarvis:media-state', (e) => {
    if (e && e.detail) onMedia(e.detail);
  });

  // Au chargement : la card en fantôme, en attendant une musique.
  if (!cards.has('music')) {
    rebuildMusicWidget(J.currentMedia && J.currentMedia.active ? J.currentMedia : {
      active: false, status: 'stopped', title: 'Nightwave Plaza', artist: 'Synthwave & Cyberpunk Radio',
      playlist: 'Radio Cyberpunk & Synthwave', app: 'JARVIS AUDIO'
    }, false);
    const card = cards.get('music');
    if (card && card.el) {
      card.state = 'ghost';
      card.el.classList.remove('w-active', 'w-fading');
      card.el.classList.add('w-ghost');
    }
    if (J.currentMedia && J.currentMedia.active) onMedia(J.currentMedia);
  }
})();
