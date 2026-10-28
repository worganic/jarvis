// Pont des fenêtres de Jarvis v2 (remplace preload.js et preload-admin.js d'Electron).
//
// Deux canaux :
//   - le noyau (données, demandes, administration) : WebSocket 127.0.0.1, avec le port, le jeton
//     et le rôle que la coquille injecte dans SES fenêtres (window.__JARVIS__) ;
//   - la coquille Tauri (fenêtres, widget, démarrage automatique) : window.__TAURI__.core.invoke.
// Les pages gardent l'API de la v1 (window.jarvisBridge, window.jarvisAdmin) : app.js, widget.js,
// widgets.js, music.js et admin.js n'ont presque pas changé.

(function () {
  const cfg = window.__JARVIS__;
  const tauri = window.__TAURI__;
  // Bureau JARVIS (fenêtre « desk ») : ses positions de cards sont rangées à part de celles de la
  // fenêtre classique (écrans et tailles différents).
  window.JARVIS_POS_PREFIX = cfg && cfg.label === 'desk' ? 'jarvis.desk.' : 'jarvis.';
  if (!cfg || !cfg.port || !cfg.token) return; // page ouverte hors coquille : mode démonstration

  const invoke = (cmd, args) => (tauri && tauri.core ? tauri.core.invoke(cmd, args || {}) : Promise.resolve(null));
  const listeners = new Set();
  const pending = new Map();
  const outbox = [];
  let ws = null;
  let seq = 0;
  let deskEnabled = false; // configuration « desk » diffusée par le noyau à chaque connexion

  function connect() {
    ws = new WebSocket(`ws://127.0.0.1:${cfg.port}/?token=${encodeURIComponent(cfg.token)}&role=${encodeURIComponent(cfg.role || 'hud')}`);
    ws.onopen = () => {
      while (outbox.length) ws.send(outbox.shift());
    };
    ws.onmessage = (e) => {
      let msg;
      try { msg = JSON.parse(e.data); } catch { return; }
      if (msg.ev) {
        if (msg.ev.type === 'desk_config') deskEnabled = Boolean(msg.ev.enabled);
        for (const cb of listeners) {
          try { cb(msg.ev); } catch (err) { console.error('événement du noyau :', err); }
        }
      } else if (msg.id !== undefined && pending.has(msg.id)) {
        const { resolve, reject } = pending.get(msg.id);
        pending.delete(msg.id);
        if (msg.e) reject(new Error(msg.e));
        else resolve(msg.r);
      }
    };
    ws.onclose = () => {
      for (const { reject } of pending.values()) reject(new Error('noyau déconnecté'));
      pending.clear();
      setTimeout(connect, 1000); // le noyau redémarre avec la coquille : on se relie dès qu'il revient
    };
    ws.onerror = () => {};
  }

  function call(m, ...a) {
    return new Promise((resolve, reject) => {
      const id = ++seq;
      pending.set(id, { resolve, reject });
      const data = JSON.stringify({ id, m, a });
      if (ws && ws.readyState === 1) ws.send(data);
      else outbox.push(data);
    });
  }
  const fire = (m, ...a) => { call(m, ...a).catch(() => {}); };
  const onEvent = (cb) => listeners.add(cb);

  connect();

  // Interface modifiée sur le disque (ui/ est servie sans recompilation) : la page se recharge d'elle-même.
  let reloadTimer = null;
  listeners.add((ev) => {
    if (ev && ev.type === 'ui_changed') {
      clearTimeout(reloadTimer);
      reloadTimer = setTimeout(() => location.reload(), 300);
    }
  });

  // Fichiers glissés depuis l'Explorateur (compagnon de bureau) : la page ne retient que l'extension du nom,
  // jamais le chemin ni le contenu. Hors de la zone déclarée, le dépôt est refusé (sans quoi WebView2 ouvrirait
  // le fichier à la place de la page). La coquille laisse ces dépôts à la page (disable_drag_drop_handler).
  let dropZone = null; // (x, y) en px CSS => vrai au-dessus du cœur
  const dragFiles = (e) => Boolean(e.dataTransfer && Array.from(e.dataTransfer.types || []).includes('Files'));
  const overZone = (e) => dragFiles(e) && Boolean(dropZone && dropZone(e.clientX, e.clientY));
  const onDrag = (e) => {
    if (!dragFiles(e)) return;
    e.preventDefault();
    e.dataTransfer.dropEffect = overZone(e) ? 'copy' : 'none';
  };
  window.addEventListener('dragenter', onDrag);
  window.addEventListener('dragover', onDrag);
  window.addEventListener('drop', (e) => {
    if (!dragFiles(e)) return;
    e.preventDefault();
    if (!overZone(e)) return;
    for (const f of Array.from(e.dataTransfer.files || []).slice(0, 3)) {
      const m = /\.([^.\\/]+)$/.exec(f.name || '');
      const ext = m ? m[1].toLowerCase().replace(/[^a-z0-9]/g, '').slice(0, 8) : '';
      call('compagnon', 'nourrir', { ext }).catch(() => {});
    }
  });

  window.jarvisBridge = {
    // Zone où un fichier déposé nourrit le compagnon : fn(x, y) en px CSS, ou null.
    fileDropZone: (fn) => { dropZone = typeof fn === 'function' ? fn : null; },
    // Appel direct d'une méthode du noyau ou d'un module (modules/<id>/core) : les modules n'ajoutent rien ici.
    call: (method, ...args) => call(String(method), ...args),
    coreVoice: true, // la voix est traitée par le noyau (voir core/voice-link.js)
    label: cfg.label || cfg.role,
    // Page qui porte les effets uniques (musique, webcam) : le Bureau JARVIS quand il est actif,
    // sinon la fenêtre classique.
    isLeader: () => (cfg.label === 'desk' ? deskEnabled : !deskEnabled),
    deskHeart: (r) => invoke('desk_heart', { r: Number(r) || 0 }),
    deskCards: (rects) => invoke('desk_cards', { rects: Array.isArray(rects) ? rects : [] }),
    deskGoto: () => invoke('desk_goto'),
    deskEnabled: () => deskEnabled,
    deskLeave: () => invoke('desk_leave'),
    deskEnter: () => invoke('desk_enter'),
    deskReveal: () => invoke('desk_reveal'),
    deskState: () => invoke('desk_state'),
    onTaskEvent: onEvent,
    // La coquille diffuse la visibilité de chaque fenêtre ; chacune ne retient que la sienne.
    onVisibility: (cb) => {
      if (tauri && tauri.event) {
        tauri.event.listen('jarvis:visibility', (e) => {
          if (e.payload && e.payload.label === cfg.label) cb(Boolean(e.payload.visible));
        });
      }
    },
    // Cœur unique (shell/src/desk.rs) : le grand cœur du HUD rétrécit et glisse jusqu'au widget, et revient.
    // fly : trajet à dessiner (widget) ; settled : arrivée, la fenêtre reprend sa taille ; owner : qui dessine le cœur (HUD).
    heartReady: () => invoke('heart_ready'),
    heartLanded: () => invoke('heart_landed'),
    onHeartFly: (cb) => { if (tauri && tauri.event) tauri.event.listen('jarvis:heart-fly', (e) => { if (e.payload && e.payload.label === cfg.label) cb(e.payload); }); },
    onHeartSettled: (cb) => { if (tauri && tauri.event) tauri.event.listen('jarvis:heart-settled', (e) => { if (e.payload && e.payload.label === cfg.label) cb(e.payload); }); },
    onHeartSpot: (cb) => { if (tauri && tauri.event) tauri.event.listen('jarvis:heart-spot', (e) => { if (e.payload && e.payload.label === cfg.label) cb(e.payload); }); },
    onHeartOwner: (cb) => { if (tauri && tauri.event) tauri.event.listen('jarvis:heart-owner', (e) => { if (e.payload && e.payload.label === cfg.label) cb(String(e.payload.owner)); }); },
    // Événement de la coquille propre à cette fenêtre (compagnon : fichier déposé, Ctrl maintenu).
    onShell: (name, cb) => { if (tauri && tauri.event) tauri.event.listen('jarvis:' + name, (e) => cb(e.payload)); },
    utterance: (text, meta) => fire('utterance', text, meta),
    speculativeUtterance: (text, meta) => fire('speculativeUtterance', text, meta),
    approve: (id) => fire('approve', id),
    deny: (id) => fire('deny', id),
    codeDigits: (id, digits) => call('codeDigits', id, String(digits)),
    codeBack: (id) => call('codeBack', id),
    cancelTask: (id) => call('cancelTask', id),
    stopAll: (origin) => call('stopAll', origin || 'bouton'),
    cameraFrame: (id, dataUrl) => call('cameraFrame', id, dataUrl),
    cardClosed: (key) => fire('cardClosed', key),
    cardOpen: (n) => call('cardOpen', n),
    cardClose: (n) => call('cardClose', n),
    cardAppOpen: (id) => call('cardAppOpen', id),
    cardAppClose: (id) => call('cardAppClose', id),
    cardAppAction: (id, action) => call('cardAppAction', id, action),
    cameraError: (id, message) => call('cameraError', id, message),
    presenceFrame: (b64) => call('presenceFrame', b64),
    presenceConfig: () => call('presenceConfig'),
    desktopAction: (hwnd, action) => call('desktopAction', hwnd, action),
    devantAction: (action, appKey, type) => call('devantAction', action, appKey, type),
    listAction: (kind, id, action) => call('listAction', kind, id, action),
    // Listes à fiches : opération du service (lister, lire, ajouter_element, modifier_element, creer_liste…).
    collections: (op, args) => call('collections', op, args || {}),
    browserTab: (tabId, action) => call('browserTab', tabId, action),
    uiClick: (hwnd, index, name) => call('uiClick', hwnd, index, name),
    mailOpen: (uid) => call('mailOpen', uid),
    mailAction: (uid, action) => call('mailAction', uid, action),
    noteEdit: (id, patch) => fire('noteEdit', id, patch),
    noteAction: (id, action, payload) => call('noteAction', id, action, payload || {}),
    meetingAction: (id, action, payload) => call('meetingAction', id || '', action, payload || {}),
    restartVoice: () => fire('restartVoice'),
    restartJarvis: () => call('restartJarvis'),
    appVersion: () => call('appVersion'),
    openChangelog: () => call('openChangelog'),
    openHelp: () => call('openHelp'),
    screenClip: () => call('screenClip'),
    ptt: (active) => fire('ptt', Boolean(active)),
    voiceStop: () => fire('voiceStop'),
    voiceSilence: (on) => call('voiceSilence', typeof on === 'boolean' ? on : null),
    voiceMute: (on) => call('voiceMute', typeof on === 'boolean' ? on : null),
    voiceState: () => call('voiceState'),
    // LiveKit n'existe plus en v2 : ces appels restent inoffensifs pour l'ancien code.
    livekitJoin: () => Promise.resolve(null),
    livekitStatus: () => {},
    setAutostart: (enable) => invoke('set_autostart', { enable: Boolean(enable) }),
    getAutostart: () => invoke('get_autostart'),
    toggleWidget: () => invoke('toggle_widget'),
    toggleHalfScreen: () => invoke('toggle_half_screen'),
    setWindowMode: (mode) => invoke('set_window_mode', { mode }),
    getWindowState: () => invoke('get_window_state'),
    openAdmin: (section) => invoke('open_admin_cmd', { section: section || null }),
    mediaCommand: (cmd, app) => (app ? call('mediaCommand', cmd, app) : call('mediaCommand', cmd)),
    getLocalMusic: async () => {
      const tracks = await call('getLocalMusic');
      // Fichiers locaux lus par le protocole « asset » de la coquille (portée limitée aux dossiers de musique).
      const conv = tauri && tauri.core && tauri.core.convertFileSrc;
      return (tracks || []).map((t) => ({ ...t, src: conv ? conv(t.path) : '' }));
    },
    sendMusicState: (state) => fire('sendMusicState', state),
    widgetState: (kind, open) => fire('widgetState', kind, Boolean(open)),
    setWidget: (on) => invoke('set_widget', { on: Boolean(on) }),
    widgetHover: (on) => invoke('widget_hover', { on: Boolean(on) }),
    widgetMove: (x, y) => invoke('widget_move', { x: Math.round(x), y: Math.round(y) }),
    widgetMoved: () => invoke('widget_moved'),
    // Promenade du compagnon : déplace le widget sans le rendre cliquable ni enregistrer sa place.
    widgetWalk: (x, y) => invoke('widget_walk', { x: Math.round(x), y: Math.round(y) }),
    widgetPosition: () => invoke('widget_position'),
    widgetMenu: () => invoke('widget_menu'),
    getErrors: () => call('getErrors'),
    clearErrors: () => fire('clearErrors'),
    getAudioDevice: () => call('getAudioDevice'),
    toggleAudioDevice: () => call('toggleAudioDevice'),
    // Méthodes du module Audio (modules/audio/core), aussi utilisées par la card musique (niveaux, volume).
    audioWatch: (on, meter) => call('audioWatch', Boolean(on), Boolean(meter)),
    audioVolume: (kind, value) => call('audioVolume', kind, Number(value)),
    audioMute: (kind, on) => call('audioMute', kind, on === 'toggle' ? 'toggle' : Boolean(on)),
    audioDefault: (id) => call('audioDefault', String(id || '')),
    jarvisVolume: (value, test) => call('jarvisVolume', Number(value), Boolean(test)),
    clearMemory: () => call('clearMemory'),
    // Voix non reconnue (voice_final.pendingId) : « c'est Johann », « nouvelle personne », ignorer,
    // annuler un apprentissage (empreinte « sample » de la phrase).
    speakerConfirm: (pendingId, speakerId) => call('speakerConfirm', String(pendingId || ''), String(speakerId || '')),
    speakerCreate: (pendingId, name, type) => call('speakerCreate', String(pendingId || ''), String(name || ''), String(type || '')),
    speakerDismiss: (pendingId) => call('speakerDismiss', String(pendingId || '')),
    speakerUndo: (sample) => call('speakerUndo', String(sample || '')),
    speakerLearning: (on) => call('speakerLearning', Boolean(on)),
    // Empreinte vocale : le noyau reçoit directement ces événements du moteur.
    speakerEnrolled: () => {},
    speakerProgress: () => {},
    speakerRejected: () => {},
    quit: () => invoke('quit_app')
  };

  if (cfg.role === 'admin') {
    window.jarvisAdmin = {
      call: async (method, ...args) => call('admin', method, ...args),
      // Méthodes hors liste blanche ouvertes à la fenêtre « admin » (ex. modulesList, modulesSet).
      direct: async (method, ...args) => call(String(method), ...args),
      window: (action) => invoke('admin_window', { action }),
      onEvent
    };
  }
})();
