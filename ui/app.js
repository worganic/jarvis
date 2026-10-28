(function () {
  const J = (window.JARVIS = window.JARVIS || {});
  // Largeur de texte mémorisée (hud.js) ; repli sur measureText si hud.js n'est pas chargé.
  const tw = (g, text) => (J.textWidth ? J.textWidth(g, text) : g.measureText(text).width);
  const canvas = document.getElementById('stage');
  // Fond entièrement repeint (opaque) à chaque image : pas besoin de canal alpha, ce qui évite
  // au compositeur de mélanger le canvas avec la page. Sauf sur le Bureau JARVIS, dont le fond peut être
  // celui de Windows (wallpaper.js) : canvas transparent, réglage changeable sans recharger.
  let g = canvas.getContext('2d', { alpha: Boolean(window.__JARVIS__ && window.__JARVIS__.label === 'desk') }); // `let` : la card Conversation est parfois dessinée sur le canevas de devant
  const frontCanvas = document.getElementById('stage-front');
  const frontCtx = frontCanvas ? frontCanvas.getContext('2d') : null;
  let frontDirty = false;
  const input = document.getElementById('input');
  const consoleBar = document.getElementById('console');

  let W = 0;
  let H = 0;
  let dpr = 1;
  let hudLayer = null;

  // Bureau JARVIS : cette page est le fond vivant du bureau virtuel (shell/src/desk.rs). Pas de barre
  // de fenêtre ; la coquille reçoit le rayon du cœur pour détacher celui-ci à sa place, au premier
  // plan, quand une application le recouvre.
  const deskMode = Boolean(window.__JARVIS__ && window.__JARVIS__.label === 'desk');
  if (deskMode) document.body.classList.add('desk-mode');
  let deskHeartSent = -1; // -1 : rien rapporté encore (la coquille apprend ainsi que la page dessine, même avec r = 0)
  let deskHeartAt = 0;
  // r : rayon du cœur réellement dessiné (px CSS), 0 tant que le cœur grandit encore (animation d'allumage : le
  // cœur est encore minuscule et un trajet partirait de lui). Rapporté au plus 4 fois par seconde.
  function reportDeskHeart(r, now) {
    if (!deskMode || !window.jarvisBridge || !window.jarvisBridge.deskHeart) return;
    const reset = r === 0 && deskHeartSent !== 0; // retour à « inconnu » : tout de suite
    if (!reset && (now - deskHeartAt < 250 || Math.abs(r - deskHeartSent) < Math.max(1.5, deskHeartSent * 0.03))) return;
    deskHeartSent = r;
    deskHeartAt = now;
    window.jarvisBridge.deskHeart(r);
  }
  // Cards visibles (px CSS de la page) : le mini-cœur du Bureau JARVIS ne se pose jamais dessus (shell/src/desk.rs ›
  // card_covers). Cards d'action pas masquées (même en mode fantôme) et widgets ouverts ou en fondu. Envoyé au plus
  // 2 fois par seconde, et seulement quand la liste change (à 4 px près).
  let deskCardsSig = '';
  let deskCardsAt = 0;
  function reportDeskCards(now, widgetBoxes, scale) {
    if (!deskMode || !window.jarvisBridge || !window.jarvisBridge.deskCards || now - deskCardsAt < 500) return;
    deskCardsAt = now;
    const rects = [];
    for (const card of (J.getActionCards ? J.getActionCards() : [])) {
      const b = card.panelBox;
      if (b && (card.visibility === undefined || card.visibility > 0.2)) rects.push([b.x, b.y, b.w, b.h]);
    }
    for (const b of Object.values(widgetBoxes || {})) {
      if (b && (b.state === 'active' || b.state === 'fading')) rects.push([b.x, b.y, b.w, b.h]);
    }
    // Panneaux du HUD (conversation, mémoire, journal, aide, audio, matériel, actions, console) : le petit cœur ne s'y pose pas non plus.
    for (const p of (J.networkPanels || [])) {
      const b = p && p.box;
      if (b && b.w > 0 && b.h > 0) rects.push([b.x, b.y, b.w, b.h]);
    }
    const css = rects.map((r) => r.map((v) => Math.round(v * scale)));
    const sig = css.map((r) => r.map((v) => Math.round(v / 4)).join(',')).join(';');
    if (sig === deskCardsSig) return;
    deskCardsSig = sig;
    Promise.resolve(window.jarvisBridge.deskCards(css)).catch(() => {});
  }
  // Cœur unique (shell/src/desk.rs) : dès que le widget a pris le cœur (il peint déjà le même, au même endroit, à la
  // même taille), ce HUD cesse de dessiner le sien, en fondu de ~120 ms : jamais deux cœurs, aucune trace.
  // Il le reprend (fondu) quand le widget est revenu au centre.
  let heartOnWidget = false;
  let heartAlpha = 1;
  // Petit cœur du Bureau JARVIS recouvert : le HUD (sous toutes les fenêtres) le déplace lui-même vers un endroit libre
  // (jarvis:heart-spot, shell/src/desk.rs › drive_hud_heart) au lieu du widget toujours au-dessus. Coordonnées de la
  // coquille en px CSS ; ici en repère de mise en page (÷ S). { x, y, r } courants, cible, départ, heure et durée.
  let mini = null; // null : le cœur est au centre
  const easeMini = (p) => (p < 0.5 ? 4 * p * p * p : 1 - Math.pow(-2 * p + 2, 3) / 2);
  function miniNow(now, hud) {
    if (!mini) return null;
    const to = mini.toHud ? hud : mini.to;
    const p = Math.min(1, (now - mini.t0) / mini.dur);
    const e = easeMini(p);
    return { x: mini.from.x + (to.x - mini.from.x) * e, y: mini.from.y + (to.y - mini.from.y) * e, r: mini.from.r + (to.r - mini.from.r) * e, p };
  }
  function onHeartSpot(ev) {
    const now = performance.now();
    const hud = { x: W / 2, y: H / 2, r: lastHeartR };
    const from = miniNow(now, hud) || hud;
    const to = ev.mode === 'mini' ? { x: ev.x / S, y: ev.y / S, r: ev.r / S } : hud;
    mini = { from: { x: from.x, y: from.y, r: from.r }, to, toHud: ev.mode !== 'mini', t0: now, dur: Math.max(300, Number(ev.durMs) || 900) };
  }
  window.__miniHeart = () => (mini ? { toHud: mini.toHud, to: mini.to } : null); // contrôle de tools/test-shell.js
  let lastHeartR = 0;
  if (deskMode && window.jarvisBridge) {
    if (window.jarvisBridge.onHeartSpot) window.jarvisBridge.onHeartSpot(onHeartSpot);
    if (window.jarvisBridge.onHeartOwner) {
      window.jarvisBridge.onHeartOwner((owner) => {
        heartOnWidget = owner === 'widget';
        // HUD entièrement recouvert (plus de dessin) : pas de fondu à faire, et pas de grand cœur qui reparaîtrait
        // un instant quand la fenêtre se ferme, avant que le petit cœur ne revienne.
        if (!windowVisible) heartAlpha = heartOnWidget ? 0 : 1;
      });
    }
    window.__heartState = () => ({ onWidget: heartOnWidget, alpha: heartAlpha }); // contrôle de tools/test-shell.js
    // Page rechargée pendant que le widget porte le cœur : état relu auprès de la coquille.
    if (window.jarvisBridge.deskState) {
      Promise.resolve(window.jarvisBridge.deskState()).then((st) => {
        if (st && st.owner === 'widget') { heartOnWidget = true; heartAlpha = 0; }
        if (st && st.mini) { const m = { x: st.mini.x / S, y: st.mini.y / S, r: st.mini.r / S }; mini = { from: m, to: m, toHud: false, t0: performance.now(), dur: 300 }; heartAlpha = 0; }
      }).catch(() => {});
    }
  }
  let debug = false;
  let showTranscript = localStorage.getItem('jarvis.transcript') !== '0';
  let showMemory = localStorage.getItem('jarvis.memory') !== '0';
  let showStatus = localStorage.getItem('jarvis.status') !== '0';
  let showJournal = localStorage.getItem('jarvis.journal') === '1';
  let currentTheme = localStorage.getItem('jarvis.theme') || 'cyan';
  let currentDensity = localStorage.getItem('jarvis.density') || 'normal';
  // Apparence du cœur : 'classic' ou un personnage 3D (ui/character.js), réglée par le noyau.
  let heartStyle = localStorage.getItem('jarvis.heartStyle') || 'classic';
  let companionState = null; // compagnon de bureau activé : stade, branche, effets (orch/companion)
  if (window.jarvisBridge && window.jarvisBridge.call) {
    window.jarvisBridge.call('compagnon', 'etat').then((v) => { companionState = v && v.enabled ? v : null; }).catch(() => {});
  }
  let heartDropR = 0; // rayon du cœur dessiné (px CSS) : un fichier déposé dessus nourrit le compagnon
  if (window.jarvisBridge && window.jarvisBridge.fileDropZone) {
    window.jarvisBridge.fileDropZone((x, y) => {
      if (!companionState || heartDropR <= 0) return false;
      const rc = canvas.getBoundingClientRect();
      return Math.hypot(x - (rc.left + rc.width / 2), y - (rc.top + rc.height / 2)) <= Math.max(heartDropR, 40);
    });
  }
  let journalScrollOffset = 0;
  let transcriptScrollOffset = 0;
  let mousePos = { x: -1, y: -1 };
  let transcriptBox = null;
  let journalBox = null;
  let memoryBox = null;
  let draggedTask = null;
  let dragTaskOffset = { x: 0, y: 0 };
  let draggedActionCard = null;
  let dragCardOffset = { x: 0, y: 0 };
  let draggedPanel = null;
  let dragPanelOffset = { x: 0, y: 0 };
  let isDragging = false;
  let dragStartPos = { x: 0, y: 0 };
  let hasDragged = false;
  let transcriptCustomPos = null;
  // Zone conversation : réductible en un seul bouton déplaçable ; état et place mémorisés.
  let transcriptCollapsed = false;
  let transcriptToggleBtn = null;
  // Même en-tête que les autres cards : 📍 (emplacement par défaut) et 📌 (figer) à côté du « – ».
  let transcriptFixBtn = null;
  let transcriptPinBtn = null;
  let transcriptTitleBtn = null; // numéro + titre : un clic réduit la card, comme les autres cards
  let transcriptPinned = false;
  const TRANSCRIPT_DEFAULT_KEY = () => (window.JARVIS_POS_PREFIX || 'jarvis.') + 'defaultPos.conversation';
  const transcriptDefault = () => { try { const p = JSON.parse(localStorage.getItem(TRANSCRIPT_DEFAULT_KEY()) || 'null'); return p && Number.isFinite(p.x) && Number.isFinite(p.y) ? p : null; } catch { return null; } };
  // Bouton APPRENTISSAGE de la zone conversation, et lignes cliquables (voix non reconnue : « qui parle ? »).
  let transcriptLearnBtn = null;
  let transcriptDictionBtn = null;
  let transcriptRowHits = [];
  let learnOverride = null; // choix qui vient d'être fait, en attendant la confirmation du noyau
  try {
    transcriptCollapsed = localStorage.getItem('jarvis.transcriptCollapsed') === '1';
    transcriptPinned = localStorage.getItem('jarvis.transcriptPinned') === '1';
    const tp = JSON.parse(localStorage.getItem((window.JARVIS_POS_PREFIX || 'jarvis.') + 'transcriptPos') || 'null');
    if (tp && Number.isFinite(tp.x) && Number.isFinite(tp.y)) transcriptCustomPos = tp;
  } catch {}
  let memoryCustomPos = null;
  let journalCustomPos = null;
  let lastFullAnswer = null;
  let journalEntries = [];
  let memories = [];
  let journal = null;
  let status = null;
  let listeningMode = localStorage.getItem('jarvis.listeningMode') || 'wakeword'; // 'wakeword' | 'ptt' | 'free'
  let isPttActive = false;
  let lastPttAt = 0;
  let contextMute = { mode: 'auto', isMuted: false, reason: '' };
  let lastMutedSpeech = null; // { text, reason, at }
  let showHelp = false;
  let helpBtnCoords = null;
  const DOCK_BTNS_H = 26; // boutons RANGER / ACTIONS (ui/cards-dock.js), juste sous AIDE
  let helpBox = null; // panneau d’aide ouvert (F1) : obstacle pour le placement des cards
  let configBtnCoords = null;

  const tasks = new Map();

  // Une seule passe sur les tâches par image : avant, chaque image copiait et filtrait la Map
  // 5 à 6 fois (humeur, en-tête, matériel, boucle de rendu). `active` est réutilisé d'une image
  // à l'autre : ne le conserver nulle part au-delà de l'image en cours.
  const frameStats = { active: [], awaiting: 0, waitingReply: 0, running: 0 };
  function collectTaskStats() {
    const active = frameStats.active;
    active.length = 0;
    let awaiting = 0;
    let waitingReply = 0;
    let running = 0;
    for (const t of tasks.values()) {
      if (!t.endedAt) active.push(t);
      if (t.status === 'awaiting') awaiting++;
      else if (t.status === 'waiting_reply') waitingReply++;
      else if (t.status === 'running') running++;
    }
    frameStats.awaiting = awaiting;
    frameStats.waitingReply = waitingReply;
    frameStats.running = running;
  }

  // Action externe (navigateur, musique, mail…) ou locale : classement mémorisé par tâche,
  // recalculé seulement si son libellé ou son détail change.
  const EXTERNAL_TASK_RE = /chrome|navigateur|web|recherche|google|url|site|spotify|musique|mail|gmail|agenda|calendar|mcp/i;
  function isExternalTask(t) {
    if (t._extLabel !== t.label || t._extDetail !== t.detail) {
      t._extLabel = t.label;
      t._extDetail = t.detail;
      t._ext = EXTERNAL_TASK_RE.test(`${t.label || ''} ${t.detail || ''}`);
    }
    return t._ext;
  }

  let nextId = 1;
  let level = 0;
  let levelTarget = 0;
  const levels = new Array(120).fill(0);
  let levelHead = 0;
  let lastSuccessAt = 0;
  let lastErrorAt = 0;
  let activityEnergy = 0;
  let currentBaseR = 0;
  let smoothedLoad = 0;
  let telemetry = { cpu: 0, ramPct: 0, ramUsedGb: '0.0', ramTotalGb: '0.0', procMb: 0 };
  // Surveillance permanente des ressources système (CPU & RAM) & Mode Éco automatique
  const ECO_TRIGGER_THRESHOLD = 90; // Déclenchement de la pause si CPU ou RAM >= 90%
  const ECO_RECOVERY_THRESHOLD = 85; // Rétablissement automatique si CPU et RAM < 85%
  let isResourceEcoMode = false;

  // Réglages d'optimisation (Administration › Optimisation), reçus de l'orchestrateur (perf_config).
  // Tout est actif tant que rien n'a été reçu : comportement d'origine.
  let autoInfo = null; // état du mode automatique (niveaux de charge, options réduites) : bandeau du HUD
  const perf = {
    voiceListening: true,
    hardwareMonitoring: true,
    autoEco: true,
    orbDetail: true,
    voiceWaveform: true,
    cards: true,
    hudBackground: true,
    maxFps: 30 // 0 = libre ; sinon 60 | 30 | 20 images/s au plus
  };

  function applyPerf(ev) {
    autoInfo = ev.auto && ev.auto.enabled ? ev.auto : null;
    const hadHud = perf.hudBackground;
    for (const key of Object.keys(perf)) {
      if (key in ev) perf[key] = typeof perf[key] === 'number' ? Number(ev[key]) || 0 : Boolean(ev[key]);
    }
    if (J.voiceEnable) J.voiceEnable(perf.voiceListening);
    if (!perf.hardwareMonitoring) telemetry = { cpu: 0, ramPct: 0, ramUsedGb: '0.0', ramTotalGb: '0.0', procMb: 0 };
    if (!perf.autoEco || !perf.hardwareMonitoring) isResourceEcoMode = false;
    // Le calque du fond (canvas hors écran de la taille de la fenêtre) n'est gardé en mémoire que
    // s'il est affiché ; il est reconstruit au réaffichage.
    if (perf.hudBackground !== hadHud) {
      if (perf.hudBackground && W > 0 && H > 0) hudLayer = J.buildHudLayer(W, H, dpr * S, currentTheme);
      else hudLayer = null;
    }
  }

  function updateResourceEcoMode(cpu, ram) {
    if (!perf.autoEco || !perf.hardwareMonitoring) return false;
    const c = typeof cpu === 'number' ? cpu : (telemetry.cpu || 0);
    const r = typeof ram === 'number' ? ram : (telemetry.ramPct || 0);
    if (!isResourceEcoMode) {
      if (c >= ECO_TRIGGER_THRESHOLD || r >= ECO_TRIGGER_THRESHOLD) {
        isResourceEcoMode = true;
      }
    } else {
      if (c < ECO_RECOVERY_THRESHOLD && r < ECO_RECOVERY_THRESHOLD) {
        isResourceEcoMode = false;
      }
    }
    return isResourceEcoMode;
  }
  let audioDevice = { mode: 'casque', label: 'CASQUE', render: '', capture: '', headsetAvailable: true, headsetName: 'Casque', pcName: 'Haut-parleurs PC' };
  let audioBtnCoords = null;
  let clearMemoryBtnCoords = null;

  // Réflexion de l'IA (entre envoi de la demande et réception de la réponse) & Morphing holographique
  // v5 : piloté par les événements `ai_call` de l'orchestrateur (appel réel à Claude Code / AGY),
  // et non plus deviné : une réponse locale n'allume jamais le cœur.
  let isAiThinking = false;
  const aiCalls = new Map(); // id -> { engine, model, startedAt (horloge murale) }

  function triggerIntentMorph(text) {
    if (!text || !J.triggerOrbMorph) return;
    const s = String(text).toLowerCase().normalize('NFD').replace(/[\u0300-\u036f]/g, '');
    if (/(?:musique|chanson|spotify|titre|morceau|playlist|ecoute|joue|mets la musique|lance la musique|pause|reprends|suivant|suivante)/i.test(s)) {
      J.triggerOrbMorph('music');
    } else if (/(?:mail|gmail|courriel|message|inbox|email|boite de reception)/i.test(s)) {
      J.triggerOrbMorph('mail');
    } else if (/(?:chrome|web|recherche|google|site|internet|cherche|meteo|navigation)/i.test(s)) {
      J.triggerOrbMorph('browser');
    } else if (/(?:code|script|terminal|fichier|dossier|console|programme|powershell|python|node|git)/i.test(s)) {
      J.triggerOrbMorph('code');
    } else if (/(?:agenda|calendrier|reunion|rendez-vous|planning|rendez vous)/i.test(s)) {
      J.triggerOrbMorph('calendar');
    }
  }

  function applyTheme(themeId) {
    if (!J.THEMES || !J.THEMES[themeId]) themeId = 'cyan';
    currentTheme = themeId;
    localStorage.setItem('jarvis.theme', themeId);
    if (J.setOrbTheme) J.setOrbTheme(themeId);
    if (J.setThemeColors) J.setThemeColors(themeId);
    if (W > 0 && H > 0 && perf.hudBackground) {
      hudLayer = J.buildHudLayer(W, H, dpr * S, currentTheme);
    }
  }

  function applyDensity(densityId) {
    if (!J.DENSITIES || !J.DENSITIES[densityId]) densityId = 'normal';
    currentDensity = densityId;
    localStorage.setItem('jarvis.density', densityId);
  }

  function applyHeartStyle(styleId) {
    if (!J.HEART_STYLES || !J.HEART_STYLES[styleId]) styleId = 'classic';
    heartStyle = styleId;
    localStorage.setItem('jarvis.heartStyle', styleId);
  }

  // Initialisation immédiate des modules d'affichage selon les réglages mémorisés
  if (J.setOrbTheme) J.setOrbTheme(currentTheme);
  if (J.setThemeColors) J.setThemeColors(currentTheme);

  // Traces récentes de rémanence dessinées directement sur le canvas principal
  const remanenceTraces = [];
  const MAX_REMANENCE = 16;
  const REMANENCE_LIFETIME = 240000; // 4 minutes (entre 3 et 5 min)

  // Panneaux de l'interface branchés eux aussi au réseau du cœur (pistes du circuit, branches.js) : conversation,
  // mémoire, journal, aide et console de commande. Boîtes de l'image précédente, en unités virtuelles.
  // Boîte d'une card HTML en unités du canevas, relue 10 fois par seconde (pas de mise en page à chaque image).
  const domBoxCache = {};
  function domPanelBox(key, el) {
    if (!el || !el.isConnected) return null;
    const now = performance.now();
    let c = domBoxCache[key];
    if (!c || now - c.at > 100) {
      const r = el.getBoundingClientRect();
      c = domBoxCache[key] = { at: now, box: r.width > 0 && r.height > 0 ? { x: r.left / S, y: r.top / S, w: r.width / S, h: r.height / S } : null };
    }
    return c.box;
  }

  function networkPanels() {
    const out = [];
    const add = (key, b) => { if (b && b.width > 0 && b.height > 0) out.push({ key, box: { x: b.x, y: b.y, w: b.width, h: b.height } }); };
    if (showTranscript) add('p:transcript', transcriptBox);
    if (showMemory) add('p:memory', memoryBox);
    if (showJournal) add('p:journal', journalBox);
    if (showHelp) add('p:help', helpBox);
    // Cards HTML de type 1 (Audio, Matériel, Actions) : reliées au cœur comme les autres, ouvertes ou réduites en bouton.
    for (const [key, el] of [['p:audio', J.audioPanel && J.audioPanel.element], ['p:hw', J.hwPanels && J.hwPanels.element], ['p:actions', J.actionsCard && J.actionsCard.element]]) {
      const b = domPanelBox(key, el);
      if (b) out.push({ key, box: b });
    }
    const c = consoleBar ? getConsoleRect() : null;
    if (c && c.width > 0) out.push({ key: 'p:console', box: { x: c.left, y: c.top, w: c.width, h: c.height } });
    return out;
  }

  // Mise en cache de la géométrie de consoleBar pour éviter le layout thrashing (getBoundingClientRect)
  let cachedConsoleRect = null;
  function getConsoleRect() {
    if (!cachedConsoleRect) {
      if (!consoleBar) {
        cachedConsoleRect = { top: H - 78, right: W / 2 + 200, bottom: H, left: W / 2 - 200, width: 400, height: 40 };
      } else {
        const r = consoleBar.getBoundingClientRect();
        cachedConsoleRect = {
          top: r.top / S,
          right: r.right / S,
          bottom: r.bottom / S,
          left: r.left / S,
          width: r.width / S,
          height: r.height / S
        };
      }
    }
    return cachedConsoleRect;
  }

  // Échelle de l'interface : tout le HUD est dessiné dans un repère virtuel d'environ
  // 1400 × 820, agrandi ou réduit selon la fenêtre. Textes, marges et panneaux suivent
  // ainsi la taille de la fenêtre sans que chaque constante ait à être recalculée.
  const UI_REF_W = 1400;
  const UI_REF_H = 820;
  let realW = 0;
  let realH = 0;
  let S = 1;

  function uiScale(w, h) {
    return Math.max(0.75, Math.min(2, Math.min(w / UI_REF_W, h / UI_REF_H)));
  }

  function resize() {
    dpr = window.devicePixelRatio || 1;
    realW = window.innerWidth;
    realH = window.innerHeight;
    S = uiScale(realW, realH);
    W = realW / S;
    H = realH / S;
    const k = dpr * S;
    document.documentElement.style.setProperty('--ui', S.toFixed(3));
    cachedConsoleRect = null;

    canvas.width = Math.floor(realW * dpr);
    canvas.height = Math.floor(realH * dpr);
    canvas.style.width = `${realW}px`;
    canvas.style.height = `${realH}px`;
    g.setTransform(k, 0, 0, k, 0, 0);
    if (frontCanvas) {
      frontCanvas.width = canvas.width;
      frontCanvas.height = canvas.height;
      frontCanvas.style.width = canvas.style.width;
      frontCanvas.style.height = canvas.style.height;
      frontCtx.setTransform(k, 0, 0, k, 0, 0);
    }
    hudLayer = perf.hudBackground ? J.buildHudLayer(W, H, k, currentTheme) : null;
  }

  const api = {
    create({ id: givenId, label, detail, status, code, background }) {
      const id = givenId || `t${nextId++}`;
      const cx = W / 2;
      const cy = H / 2;
      const currentR_val = currentBaseR || (Math.min(W, H) * 0.2);
      const panelsState = { showTranscript, showMemory, showStatus, showJournal };
      const taskData = { label, detail, status, code, background: Boolean(background) };
      const slot = J.allocateSlot(cx, cy, currentR_val, W, H, [...tasks.values()], panelsState, taskData);

      const task = {
        id,
        label: label || 'TÂCHE',
        detail: detail || '',
        status: status || 'running',
        code: code || null,
        background: Boolean(background),
        slot,
        grow: 0,
        fadeAlpha: 1,
        createdAt: performance.now(),
        endedAt: null
      };
      tasks.set(id, task);
      activityEnergy = Math.min(1, activityEnergy + 0.25);
      // Tâche prise par une card d'action : la card elle-même s'estompe lentement (sa piste aussi) ; pas de
      // trace de rémanence en double, posée ailleurs et sans liaison.
      if (J.mapTaskToActionCard && J.mapTaskToActionCard(task)) task.onCard = true;
      return id;
    },
    update(id, patch) {
      const task = tasks.get(id);
      if (!task) return;
      Object.assign(task, patch);
      if (patch.status === 'done') {
        task.endedAt = performance.now();
        lastSuccessAt = performance.now();
        activityEnergy = Math.min(1, activityEnergy + 0.15);
        if (J.finishActionCard) J.finishActionCard(id, 'done');
      } else if (patch.status === 'error') {
        task.endedAt = performance.now();
        lastErrorAt = performance.now();
        activityEnergy = Math.min(1, activityEnergy + 0.25);
        if (J.finishActionCard) J.finishActionCard(id, 'error');
      }
    },
    finish(id, status) {
      api.update(id, { status: status || 'done', code: null });
      if (J.finishActionCard) J.finishActionCard(id, status || 'done');
    },
    get(id) {
      return tasks.get(id);
    },
    all() {
      return [...tasks.values()];
    }
  };

  J.tasks = api;
  J.setLevel = (v) => { levelTarget = Math.max(0, Math.min(1, v)); };

  function computeMood(now) {
    // 1. Quota >= 90% : sous tension (rouge sombre grenat pulsé)
    if (status && status.usage && status.usage.five && status.usage.five.pct >= 90) {
      return 'overload';
    }
    // 2. Erreur, refus ou rejet récent (< 5s) : contrarié (rouge vif vermillon)
    if (now - lastErrorAt < 5000) {
      return 'error';
    }
    // 3. Validation par code requise : en attente (ambre)
    if (frameStats.awaiting > 0) {
      return 'awaiting';
    }
    // 3a. IA externe sollicitée (v5) : cœur orange tant que Claude Code / AGY travaille.
    if (aiCalls.size > 0) {
      return 'external';
    }
    // 3b. Question posée à l'utilisateur : en attente de réponse (cyan vibrant / écoute)
    if (frameStats.waitingReply > 0) {
      return 'listening';
    }
    // 4. Détection micro / parole : à l'écoute (cyan)
    const v = J.voice;
    if (v && (v.capturing || v.speaking)) {
      return 'listening';
    }
    // 5. Tâche terminée avec succès récemment (< 4s) sans tâche restante en cours : satisfait (vert)
    const runningCount = frameStats.running;
    if (now - lastSuccessAt < 4000 && runningCount === 0) {
      return 'satisfied';
    }
    // 6. Une ou plusieurs tâches en cours : au travail (cyan-vert)
    if (runningCount > 0) {
      return 'working';
    }
    // 7. Repos : aucune tâche active, calme (bleu profond)
    return 'idle';
  }

  function step(dt, now) {
    level += (levelTarget - level) * Math.min(1, dt * 14);
    // Décroissance très lente, longue et paisible de l'énergie vers le calme au repos (~25 s)
    activityEnergy = Math.max(0, activityEnergy - dt / 25);
    if (J.updateActionCards) {
      J.updateActionCards(dt, now);
    }
    for (const task of [...tasks.values()]) {
      if (task.endedAt) {
        // Effacement progressif sur ~2 s (Étape 13)
        const elapsed = now - task.endedAt;
        task.fadeAlpha = Math.max(0, 1 - elapsed / 2000);
        if (elapsed >= 2000) {
          // Dépôt dans le calque de rémanence (canvas hors écran) sans le trait de liaison
          if (task.slot && task.slot.box && !task.onCard) {
            remanenceTraces.unshift({
              box: { ...task.slot.box }, // copie : la trace peut glisser seule (placement sans chevauchement)
              sx: task.slot.sx,
              geo: task.lastGeo || null,
              label: task.label,
              detail: task.detail,
              status: task.status,
              color: J.taskColor ? J.taskColor(task) : (J.COLORS[task.status] || J.COLORS.done),
              depositedAt: now,
              duration: REMANENCE_LIFETIME
            });
            if (remanenceTraces.length > MAX_REMANENCE) {
              remanenceTraces.pop();
            }
          }
          tasks.delete(task.id);
        }
      } else {
        // Apparition en fondu rapide (~300 ms)
        task.grow += (1 - task.grow) * Math.min(1, dt * 6.5);
      }
    }
  }

  // Moteur vocal lancé : connecté, modèle chargé, écoute orale active.
  function voiceUp() {
    const v = J.voice;
    return Boolean(v && v.connected && v.ready && !v.disabled);
  }

  // Croix rouge sur le cœur tant que le moteur vocal n'est pas lancé (lueur qui respire doucement).
  function drawHeartOffCross(g, cx, cy, R, t, alpha) {
    // Son pas encore activé : un cœur à l'envers (pointe en haut) à la place de l'ancienne croix.
    const h = R * 0.5;
    const pulse = 0.75 + 0.25 * Math.sin(t * 2.2);
    g.save();
    g.globalAlpha *= alpha;
    g.translate(cx, cy + h * 0.1);
    g.scale(1, -1);
    g.lineCap = 'round';
    g.lineJoin = 'round';
    g.shadowColor = `rgba(255, 50, 70, ${0.8 * pulse})`;
    g.shadowBlur = R * 0.12;
    g.beginPath();
    g.moveTo(0, h * 0.62);
    g.bezierCurveTo(-h * 1.15, -h * 0.05, -h * 0.62, -h * 0.85, 0, -h * 0.3);
    g.bezierCurveTo(h * 0.62, -h * 0.85, h * 1.15, -h * 0.05, 0, h * 0.62);
    g.closePath();
    g.fillStyle = `rgba(255, 77, 94, ${0.14 + 0.1 * pulse})`;
    g.fill();
    g.strokeStyle = `rgba(255, 77, 94, ${0.85 + 0.15 * pulse})`;
    g.lineWidth = Math.max(3, R * 0.075);
    g.stroke();
    g.restore();
  }

  function voiceStatus() {
    const v = J.voice;
    if (v && v.disabled) return { text: 'ÉCOUTE ORALE DÉSACTIVÉE · MODE TEXTE', color: 'rgba(255, 196, 64, 0.75)' };
    if (!v || !v.connected) return { text: 'SERVICE VOCAL HORS LIGNE', color: 'rgba(255, 77, 94, 0.75)' };
    if (v.error) return { text: v.error.toUpperCase(), color: 'rgba(255, 77, 94, 0.85)' };
    if (!v.ready) return { text: 'CHARGEMENT DU MODÈLE…', color: 'rgba(255, 176, 32, 0.8)' };
    if (v.muted) return { text: 'MICRO COUPÉ', color: 'rgba(255, 176, 32, 0.8)' };
    if (contextMute && contextMute.isMuted) {
      return {
        text: `SOURDINE CONTEXTUELLE : ${String(contextMute.reason || 'APPLICATION ACTIVE').toUpperCase()}`,
        color: 'rgba(255, 196, 64, 0.9)'
      };
    }
    if (v.speaking) return { text: 'JARVIS PARLE', color: 'rgba(126, 242, 226, 0.9)' };
    if (v.capturing) return { text: 'TRANSCRIPTION EN COURS', color: 'rgba(126, 242, 226, 0.9)' };

    // Modes de discipline d'écoute (Étape 5)
    if (listeningMode === 'ptt') {
      return isPttActive
        ? { text: 'PTT : ENREGISTREMENT EN COURS… (ESPACE)', color: 'rgba(56, 225, 255, 0.9)' }
        : { text: 'APPUI POUR PARLER : MAINTENIR ESPACE', color: 'rgba(255, 196, 64, 0.75)' };
    }
    if (listeningMode === 'wakeword') {
      if (inConversation()) {
        return { text: 'CONVERSATION OUVERTE : PARLE SANS DIRE « JARVIS »', color: 'rgba(56, 255, 178, 0.8)' };
      }
      return { text: 'MOT D’ÉVEIL : DIS « JARVIS »', color: 'rgba(56, 225, 255, 0.65)' };
    }
    return { text: 'ÉCOUTE CONTINUE LIBRE', color: 'rgba(56, 255, 178, 0.65)' };
  }

  // Deux lignes maximum, en gardant la fin du texte (dictée en direct).
  function wrapTail(text, maxWidth, lines) {
    const words = text.split(/\s+/);
    const out = [];
    let current = '';
    for (let i = words.length - 1; i >= 0; i--) {
      const candidate = current ? `${words[i]} ${current}` : words[i];
      if (g.measureText(candidate).width > maxWidth) {
        out.unshift(current);
        current = words[i];
        if (out.length >= lines - 1) break;
      } else {
        current = candidate;
      }
    }
    out.unshift(current);
    return out.slice(-lines);
  }

  const SPEAKER_TONES = { known: '53, 255, 158', doubt: '255, 196, 64', unknown: '255, 77, 94' };

  // Apprentissage des voix (voice/speaker_verifier.py › learning) : réglage du noyau, reçu avec l'état.
  function learningOn() {
    if (learnOverride && performance.now() - learnOverride.at < 4000) return learnOverride.on;
    const spk = status && status.speaker;
    return !(spk && spk.learning === false);
  }
  function toggleLearning() {
    const on = !learningOn();
    learnOverride = { on, at: performance.now() };
    if (!on && J.speakerAsk) J.speakerAsk.hide();
    if (window.jarvisBridge && window.jarvisBridge.speakerLearning) window.jarvisBridge.speakerLearning(on).catch(() => {});
  }
  // Survol : bouton APPRENTISSAGE, ou phrase à attribuer (curseur main).
  function overLearn(mx, my) {
    if (!showTranscript || transcriptCollapsed || !transcriptBox) return false;
    const lb = transcriptLearnBtn;
    if (lb && mx >= lb.x && mx <= lb.x + lb.w && my >= lb.y && my <= lb.y + lb.h) return true;
    const db = transcriptDictionBtn;
    if (db && mx >= db.x && mx <= db.x + db.w && my >= db.y && my <= db.y + db.h) return true;
    return mx >= transcriptBox.x && mx <= transcriptBox.x + transcriptBox.width && transcriptRowHits.some((r) => my >= r.y0 && my <= r.y1);
  }

  function transcript(cx, cy, R, now) {
    const v = J.voice;
    const hasMuted = lastMutedSpeech && now - lastMutedSpeech.at < 8000;
    const fresh = v && v.lastFinal && now - (v.finalAt || 0) < 5000;
    const text = (v && (v.partial || (fresh ? v.lastFinal : ''))) || '';

    if (!hasMuted && !text) return;

    // Positionnement au-dessus de la card des commandes / code de validation (#console)
    const consoleTop = consoleBar ? (consoleBar.getBoundingClientRect().top / S) : (H - 78);
    const maxWidth = Math.min(W - 120, 900);

    if (hasMuted) {
      const alpha = Math.max(0, 1 - (now - lastMutedSpeech.at) / 8000);
      g.font = '13px Consolas, "Courier New", monospace';
      g.letterSpacing = '0.5px';
      g.textAlign = 'center';
      g.fillStyle = `rgba(255, 196, 64, ${alpha * 0.9})`;

      g.font = '15px Consolas, "Courier New", monospace';
      const lines = wrapTail(lastMutedSpeech.text, maxWidth, 2);
      const totalH = 20 + lines.length * 22;
      const startY = Math.max(cy + 100, consoleTop - totalH - 12);

      g.font = '13px Consolas, "Courier New", monospace';
      g.fillText(`JARVIS (SOURDINE · ${String(lastMutedSpeech.reason).toUpperCase()})`, cx, startY);

      g.font = '15px Consolas, "Courier New", monospace';
      g.fillStyle = `rgba(255, 245, 215, ${alpha * 0.95})`;
      lines.forEach((line, i) => g.fillText(line, cx, startY + 20 + i * 22));
      g.textAlign = 'left';
      g.letterSpacing = '0px';
      return;
    }

    // Voix non reconnue en attente de confirmation : la phrase est dans la boîte de speaker-ask.js,
    // juste au-dessus de la barre de commande ; la parole en cours s'affiche au-dessus d'elle.
    const askTop = J.speakerAsk && J.speakerAsk.top ? J.speakerAsk.top() : null;
    if (askTop != null && !(v && v.partial)) return;
    const anchorTop = askTop != null ? Math.min(consoleTop, askTop / S) : consoleTop;

    // Empreinte vocale : vert = voix reconnue, orange = doute, rouge = voix non reconnue.
    const who = v && v.partial ? v.partialWho : v && v.finalWho;
    const tone = who ? SPEAKER_TONES[who.state] : null;
    g.font = '15px Consolas, "Courier New", monospace';
    g.letterSpacing = '0.5px';
    g.textAlign = 'center';
    const lines = wrapTail(text, maxWidth, 2);
    const alpha = (v && v.partial) ? 0.85 : Math.max(0, 1 - (now - v.finalAt) / 5000) * 0.95;
    if (tone) g.fillStyle = `rgba(${tone}, ${alpha})`;
    else g.fillStyle = (v && v.partial) ? `rgba(190, 240, 255, ${alpha})` : `rgba(126, 242, 226, ${alpha})`;

    // Positionnement juste au-dessus de la barre de commande / code de validation
    const totalH = lines.length * 22;
    const startY = Math.max(cy + 120, anchorTop - totalH - 14);

    lines.forEach((line, i) => g.fillText(line, cx, startY + i * 22));
    if (tone) {
      // Qui parle, en petit au-dessus du texte : « ● JOHANN · 72 % », « ● JOHANN ? · 44 % »…
      const pct = Number.isFinite(who.similarity) ? ` · ${Math.round(who.similarity * 100)} %` : '';
      const name = String(who.name || '').toUpperCase();
      const label = who.state === 'known' ? `● ${name || 'VOIX RECONNUE'}${pct}`
        : who.state === 'doubt' ? `● ${name ? `${name} ?` : 'VOIX DOUTEUSE'}${pct}`
          : `● VOIX NON RECONNUE${pct}`;
      g.font = '10px Consolas, "Courier New", monospace';
      g.letterSpacing = '2px';
      g.fillStyle = `rgba(${tone}, ${alpha * 0.9})`;
      g.fillText(label, cx, startY - 18);
    }
    g.textAlign = 'left';
    g.letterSpacing = '0px';
  }

  // Ce que Jarvis sait de l'utilisateur, alimenté par l'orchestrateur.
  function memoryPanel() {
    if (!showMemory) {
      clearMemoryBtnCoords = null;
      return;
    }

    const densityCfg = (J.DENSITIES && J.DENSITIES[currentDensity]) || J.DENSITIES.normal;
    const lines = densityCfg.panelLines;
    const width = Math.min(540, W * densityCfg.panelWidthRatio);
    const lineHeight = densityCfg.lineHeight;
    const count = memories ? memories.length : 0;
    const displayLines = count > 0 ? Math.min(lines, count) : 1;
    const height = displayLines * lineHeight + 46;
    const consoleTop = getConsoleRect().top;
    const defaultY = Math.min(H - 108, consoleTop - 14) - height;
    const x = memoryCustomPos ? Math.max(14, Math.min(W - width - 14, memoryCustomPos.x)) : (W - 34 - width);
    const y = memoryCustomPos ? Math.max(60, Math.min(H - height - 10, memoryCustomPos.y)) : defaultY;
    memoryBox = { x, y, width, height };

    g.fillStyle = 'rgba(2, 16, 22, 0.62)';
    g.fillRect(x, y, width, height);
    g.lineWidth = 1;
    g.strokeStyle = 'rgba(53, 255, 158, 0.28)';
    g.strokeRect(x, y, width, height);

    g.letterSpacing = '2px';
    g.font = `${densityCfg.fontTitle} Consolas, "Courier New", monospace`;
    g.fillStyle = 'rgba(53, 255, 158, 0.7)';
    g.fillText(`MÉMOIRE (${count})`, x + 12, y + 20);

    // Bouton de réinitialisation / effacement de la mémoire locale RAG
    const btnLabel = '[🗑️ EFFACER]';
    g.font = `600 9px ${MONO}`;
    g.letterSpacing = '0.5px';
    const btnW = tw(g, btnLabel) + 8;
    const btnH = 16;
    const btnX = x + width - btnW - 32;
    const btnY = y + 10;
    clearMemoryBtnCoords = { x: btnX, y: btnY, w: btnW, h: btnH };

    // Indicateur de préhension du panneau mémoire
    g.fillStyle = `rgba(53, 255, 158, ${draggedPanel === 'memory' ? 0.9 : 0.4})`;
    for (let r = 0; r < 2; r++) {
      for (let c = 0; c < 3; c++) {
        g.fillRect(x + width - 20 + c * 3, y + 14 + r * 3, 1.5, 1.5);
      }
    }

    const isHover = mousePos.x >= btnX && mousePos.x <= btnX + btnW && mousePos.y >= btnY && mousePos.y <= btnY + btnH;
    if (isHover) {
      g.fillStyle = 'rgba(255, 77, 94, 0.2)';
      g.fillRect(btnX, btnY, btnW, btnH);
      g.strokeStyle = 'rgba(255, 77, 94, 0.75)';
      g.lineWidth = 1;
      g.strokeRect(btnX, btnY, btnW, btnH);
    }
    g.fillStyle = isHover ? 'rgba(255, 120, 130, 0.98)' : (count > 0 ? 'rgba(255, 90, 100, 0.75)' : 'rgba(160, 160, 160, 0.35)');
    g.fillText(btnLabel, btnX + 4, btnY + 11.5);

    g.letterSpacing = '0px';
    g.font = `${densityCfg.fontText} Consolas, "Courier New", monospace`;
    if (count === 0) {
      g.fillStyle = 'rgba(190, 240, 255, 0.35)';
      g.fillText('Mémoire RAG vierge (aucun souvenir).', x + 12, y + 46);
    } else {
      memories.slice(-lines).forEach((entry, i) => {
        const at = y + 46 + i * lineHeight;
        g.fillStyle = 'rgba(126, 242, 226, 0.7)';
        const tag = `${(entry.category || 'divers').slice(0, 10)}`;
        g.fillText(tag, x + 12, at);
        g.fillStyle = 'rgba(214, 248, 255, 0.9)';
        g.fillText(clip(entry.text, width - 110), x + 100, at);
      });
    }
  }

  // Card Conversation vue du reste du HUD (cards-dock.js : RANGER, PLACER). Positions en pixels de la page.
  const saveTranscriptPos = () => { try { localStorage.setItem((window.JARVIS_POS_PREFIX || 'jarvis.') + 'transcriptPos', JSON.stringify(transcriptCustomPos)); } catch {} };
  J.conversationCard = {
    setCollapsed(on) {
      transcriptCollapsed = Boolean(on);
      try { localStorage.setItem('jarvis.transcriptCollapsed', transcriptCollapsed ? '1' : '0'); } catch {}
    },
    height: () => (transcriptBox ? transcriptBox.height * S : 26 * S),
    // Rectangle de la card en pixels de la page (null : masquée), pour card-front.js.
    box: () => (showTranscript && transcriptBox ? { x: transcriptBox.x * S, y: transcriptBox.y * S, w: transcriptBox.width * S, h: transcriptBox.height * S } : null),
    moveTo(px, py) {
      transcriptCustomPos = { x: px / S, y: py / S };
      saveTranscriptPos();
    },
    // Vue figée (cards-dock.js) : emplacement (unités du canevas) et état.
    layoutSnapshot() {
      return { x: transcriptBox ? Math.round(transcriptBox.x) : null, y: transcriptBox ? Math.round(transcriptBox.y) : null, collapsed: transcriptCollapsed };
    },
    layoutApply(s) {
      if (!s) return 0;
      if (Number.isFinite(s.x) && Number.isFinite(s.y)) {
        transcriptCustomPos = { x: s.x, y: s.y };
        saveTranscriptPos();
      }
      J.conversationCard.setCollapsed(Boolean(s.collapsed));
      return 1;
    },
    defaultState() {
      const p = transcriptDefault();
      if (!p) return { total: 0, there: 0 };
      const c = transcriptCustomPos;
      return { total: 1, there: c && Math.abs(c.x - p.x) <= 12 && Math.abs(c.y - p.y) <= 12 ? 1 : 0 };
    },
    openDefault() {
      if (!transcriptDefault() || !transcriptCollapsed) return 0;
      J.conversationCard.setCollapsed(false);
      return 1;
    },
    restoreDefault() {
      const p = transcriptDefault();
      if (!p) return 0;
      transcriptCustomPos = { x: p.x, y: p.y };
      saveTranscriptPos();
      return 1;
    }
  };

  // Zone de conversation : ce que Jarvis a compris ET ce qu'il a répondu, avec défilement.
  // Chaque entrée est découpée à la largeur du panneau (découpage mis en cache par entrée).
  let transcriptRowCount = 0;
  let cachedTranscriptRows = null;
  let cachedTranscriptKey = '';
  const JARVIS_MAX_ROWS = 5; // au-delà, la réponse complète reste consultable avec F9
  const USER_MAX_ROWS = 2;

  function transcriptPanel() {
    if (!showTranscript) {
      transcriptBox = null;
      return;
    }

    const theme = (J.THEMES && J.THEMES[currentTheme]) || J.THEMES.cyan;
    const cc = J.cardColorsRgb ? J.cardColorsRgb() : null; // couleurs des cards de type 1 (administration)
    const rgb = (cc && cc.t1n) || theme.primaryRgb;
    const densityCfg = (J.DENSITIES && J.DENSITIES[currentDensity]) || J.DENSITIES.normal;
    const lines = densityCfg.panelLines;
    const width = Math.min(580, W * densityCfg.panelWidthRatio);
    const lineHeight = densityCfg.lineHeight;
    const v = J.voice;
    // Une transcription partielle n'est montrée que pendant la capture (jamais figée à l'écran).
    const partial = v && v.partial && (v.capturing || performance.now() - (v.partialAt || 0) < 2000) ? v.partial : '';
    const bottom = H - 120;
    if (transcriptCollapsed) { // un seul bouton, à la place de la zone
      const bw = 170; // même taille que les autres boutons de cards réduites
      const bh = 26;
      const bx = transcriptCustomPos ? Math.max(14, Math.min(W - bw - 14, transcriptCustomPos.x)) : 34;
      const by = transcriptCustomPos ? Math.max(60, Math.min(H - bh - 10, transcriptCustomPos.y)) : (bottom - bh);
      transcriptBox = { x: bx, y: by, width: bw, height: bh };
      transcriptToggleBtn = null;
      const hover = mousePos.x >= bx && mousePos.x <= bx + bw && mousePos.y >= by && mousePos.y <= by + bh;
      // Bouton miniature : jaune, à 50 % d'opacité (pleine au survol).
      g.save();
      g.globalAlpha = hover ? 1 : 0.5;
      const yr = (cc && cc.t1r) || '255, 214, 64'; // couleur réduite des cards de type 1
      g.fillStyle = hover ? `rgba(${yr}, 0.22)` : 'rgba(2, 16, 22, 0.78)';
      g.fillRect(bx, by, bw, bh);
      g.lineWidth = 1;
      g.strokeStyle = `rgba(${yr}, 0.9)`;
      g.strokeRect(bx, by, bw, bh);
      g.letterSpacing = '2px';
      g.font = `${densityCfg.fontTitle} Consolas, "Courier New", monospace`;
      g.fillStyle = `rgba(${yr}, 1)`;
      g.fillText('▤ CONVERSATION', bx + 10, by + 17);
      g.letterSpacing = '0px';
      g.restore();
      return;
    }
    const height = lines * lineHeight + 46;
    const x = transcriptCustomPos ? Math.max(14, Math.min(W - width - 14, transcriptCustomPos.x)) : 34;
    const y = transcriptCustomPos ? Math.max(60, Math.min(H - height - 10, transcriptCustomPos.y)) : (bottom - height);
    transcriptBox = { x, y, width, height };
    // Bouton « – » : réduit la zone en un seul bouton.
    // Boutons à droite, dans l'ordre des autres cards : 📍 📌 –
    transcriptToggleBtn = { x: x + width - 14 - 18, y: y + 9, w: 18, h: 18 };
    transcriptPinBtn = { x: transcriptToggleBtn.x - 6 - 18, y: y + 9, w: 18, h: 18 };
    transcriptFixBtn = { x: transcriptPinBtn.x - 6 - 18, y: y + 9, w: 18, h: 18 };

    g.fillStyle = 'rgba(2, 16, 22, 0.68)';
    g.fillRect(x, y, width, height);
    g.lineWidth = 1;
    g.strokeStyle = theme.border || `rgba(${rgb}, 0.28)`;
    g.strokeRect(x, y, width, height);

    // En-tête identique aux autres cards : poignée, numéro, titre (puis 📍 📌 – à droite).
    g.fillStyle = `rgba(${rgb}, ${draggedPanel === 'transcript' ? 0.9 : 0.4})`;
    for (let r = 0; r < 2; r++) for (let c = 0; c < 2; c++) g.fillRect(x + 10 + c * 4, y + 15 + r * 4, 2, 2);
    g.beginPath();
    g.arc(x + 34, y + 18, 10, 0, Math.PI * 2);
    g.fillStyle = `rgba(${rgb}, 0.9)`;
    g.fill();
    g.save();
    g.textAlign = 'center';
    g.fillStyle = '#021018';
    g.font = `700 10.5px ${MONO}`;
    g.fillText(String(J.CARD_NO ? J.CARD_NO.conversation : 4), x + 34, y + 21.5);
    g.restore();
    for (const [btn, icon, on] of [[transcriptFixBtn, '📍', Boolean(transcriptDefault())], [transcriptPinBtn, '📌', transcriptPinned]]) {
      g.save();
      g.globalAlpha = on ? 1 : 0.4;
      g.font = '11px sans-serif';
      g.fillText(icon, btn.x + 3, btn.y + 13.5);
      g.restore();
    }

    {
      const tb = transcriptToggleBtn;
      const hover = mousePos.x >= tb.x && mousePos.x <= tb.x + tb.w && mousePos.y >= tb.y && mousePos.y <= tb.y + tb.h;
      g.strokeStyle = `rgba(${rgb}, ${hover ? 0.9 : 0.45})`;
      if (hover) { g.fillStyle = `rgba(${rgb}, 0.18)`; g.fillRect(tb.x, tb.y, tb.w, tb.h); }
      g.strokeRect(tb.x, tb.y, tb.w, tb.h);
      g.fillStyle = `rgba(${rgb}, 0.9)`;
      g.fillRect(tb.x + 4, tb.y + 9, tb.w - 8, 1.5);
    }
    {
      // Apprentissage des voix : « qui parle ? » pour les voix non reconnues, empreintes qui s'affinent.
      const on = learningOn();
      g.save();
      g.letterSpacing = '1.5px';
      g.font = `600 9px ${MONO}`;
      const label = `${on ? '●' : '○'} ${width < 440 ? 'APPR.' : 'APPRENTISSAGE'}`;
      const w = Math.ceil(tw(g, label)) + 14;
      const lb = { x: transcriptFixBtn.x - 8 - w, y: transcriptToggleBtn.y, w, h: transcriptToggleBtn.h };
      transcriptLearnBtn = lb;
      const hover = mousePos.x >= lb.x && mousePos.x <= lb.x + lb.w && mousePos.y >= lb.y && mousePos.y <= lb.y + lb.h;
      const tone = on ? '53, 255, 158' : rgb;
      g.fillStyle = `rgba(${tone}, ${on ? (hover ? 0.24 : 0.12) : (hover ? 0.16 : 0)})`;
      g.fillRect(lb.x, lb.y, lb.w, lb.h);
      g.strokeStyle = `rgba(${tone}, ${on ? 0.85 : (hover ? 0.8 : 0.4)})`;
      g.strokeRect(lb.x, lb.y, lb.w, lb.h);
      g.fillStyle = `rgba(${tone}, ${on ? 1 : (hover ? 0.9 : 0.55)})`;
      g.fillText(label, lb.x + 7, lb.y + 12.5);
      g.restore();

      // Diction : ouvre la card de correction de prononciation (comme « Jarvis, corrige ta prononciation »).
      g.save();
      g.letterSpacing = '1.5px';
      g.font = `600 9px ${MONO}`;
      const dLabel = width < 440 ? '🗣' : '🗣 DICTION';
      const dw = Math.ceil(tw(g, dLabel)) + 14;
      const db = { x: lb.x - 8 - dw, y: lb.y, w: dw, h: lb.h };
      transcriptDictionBtn = db;
      const dHover = mousePos.x >= db.x && mousePos.x <= db.x + db.w && mousePos.y >= db.y && mousePos.y <= db.y + db.h;
      g.fillStyle = `rgba(${rgb}, ${dHover ? 0.16 : 0})`;
      g.fillRect(db.x, db.y, db.w, db.h);
      g.strokeStyle = `rgba(${rgb}, ${dHover ? 0.8 : 0.4})`;
      g.strokeRect(db.x, db.y, db.w, db.h);
      g.fillStyle = `rgba(${rgb}, ${dHover ? 0.9 : 0.55})`;
      g.fillText(dLabel, db.x + 7, db.y + 12.5);
      g.restore();
    }
    g.letterSpacing = '3px';
    g.font = `700 11px ${MONO}`; // même titre que les autres cards
    g.fillStyle = `rgba(${rgb}, 0.95)`;
    const scrollTag = transcriptScrollOffset > 0 ? `  ·  ▲ DÉCALAGE +${transcriptScrollOffset}` : '';
    g.fillText(`CONVERSATION${scrollTag}`, x + 54, y + 22);
    transcriptTitleBtn = { x: x + 22, y: y + 6, w: 32 + tw(g, `CONVERSATION${scrollTag}`), h: 24 };

    g.letterSpacing = '0px';
    g.font = `${densityCfg.fontText} Consolas, "Courier New", monospace`;
    const textX = x + 84;
    const textW = width - 96;

    const lastEntry = heard.length ? heard[heard.length - 1] : null;
    const rowsKey = `${heard.length}|${lastEntry ? lastEntry.text : ''}|${Math.round(textW)}|${g.font}`;

    if (cachedTranscriptKey !== rowsKey || !cachedTranscriptRows) {
      cachedTranscriptRows = [];
      for (const entry of heard) {
        const key = `${entry.text.length}|${Math.round(textW)}|${g.font}`;
        if (entry._key !== key) {
          const max = entry.who === 'jarvis' ? JARVIS_MAX_ROWS : USER_MAX_ROWS;
          let wrapped = wrapLines(entry.text, textW).filter((l) => l.trim());
          if (wrapped.length > max) {
            wrapped = wrapped.slice(0, max);
            wrapped[max - 1] = clip(`${wrapped[max - 1]} …`, textW);
          }
          entry._rows = wrapped.length ? wrapped : [''];
          entry._key = key;
        }
        entry._rows.forEach((line, i) => cachedTranscriptRows.push({ entry, line, first: i === 0 }));
      }
      cachedTranscriptKey = rowsKey;
    }
    const rows = cachedTranscriptRows;
    transcriptRowCount = rows.length;

    // La dernière ligne est réservée à la transcription en cours.
    const capacity = lines - (partial ? 1 : 0);
    transcriptScrollOffset = Math.min(transcriptScrollOffset, Math.max(0, rows.length - capacity));
    const end = rows.length - transcriptScrollOffset;
    const visible = rows.slice(Math.max(0, end - capacity), end);
    transcriptRowHits = [];
    const asking = learningOn();

    if (!visible.length && !partial) {
      g.fillStyle = 'rgba(190, 240, 255, 0.3)';
      g.fillText('en attente de parole…', x + 12, y + 46);
    }
    visible.forEach((row, i) => {
      const age = (visible.length - 1 - i) / lines;
      const ry = y + 46 + i * lineHeight;
      const jarvis = row.entry.who === 'jarvis';
      // Voix entendue : couleur de l'empreinte (vert reconnue, orange doute, rouge non reconnue).
      const tone = !jarvis && row.entry.state ? SPEAKER_TONES[row.entry.state] : null;
      if (row.first) {
        g.fillStyle = jarvis ? `rgba(126, 242, 226, ${0.85 - age * 0.4})` : tone ? `rgba(${tone}, ${0.9 - age * 0.45})` : `rgba(${rgb}, ${0.9 - age * 0.45})`;
        g.fillText(jarvis ? 'JARVIS' : row.entry.at.toTimeString().slice(0, 8), x + 12, ry);
      }
      if (row.entry.muted) g.fillStyle = `rgba(255, 196, 64, ${0.9 - age * 0.4})`;
      else if (tone && row.entry.state !== 'known') g.fillStyle = `rgba(${tone}, ${0.95 - age * 0.45})`;
      else if (jarvis) g.fillStyle = `rgba(170, 248, 238, ${0.95 - age * 0.4})`;
      else g.fillStyle = `rgba(235, 250, 255, ${0.98 - age * 0.45})`;
      g.fillText(row.line, textX, ry);
      // Voix non reconnue, apprentissage actif : un clic rouvre « qui parle ? » pour cette phrase.
      if (asking && row.entry.ask) {
        transcriptRowHits.push({ y0: ry - lineHeight + 5, y1: ry + 5, entry: row.entry });
        if (row.first) {
          g.save();
          g.textAlign = 'right';
          g.font = `600 9px ${MONO}`;
          g.fillStyle = `rgba(${tone || '255, 196, 64'}, ${0.95 - age * 0.4})`;
          g.fillText('QUI PARLE ? ▸', x + width - 14, ry);
          g.restore();
        }
      }
    });

    if (partial) {
      g.fillStyle = `rgba(${rgb}, 0.95)`;
      g.fillText(`> ${clip(partial, width - 40, true)}`, x + 12, y + 46 + capacity * lineHeight);
    }
  }

  function clip(text, maxWidth, keepTail) {
    if (!text || maxWidth <= 10) return '';
    if (g.measureText(text).width <= maxWidth) return text;
    let out = String(text);
    if (keepTail) {
      while (out.length > 4 && g.measureText(`…${out}`).width > maxWidth) out = out.slice(1);
      return `…${out}`;
    }
    while (out.length > 4 && g.measureText(`${out}…`).width > maxWidth) out = out.slice(0, -1);
    return `${out}…`;
  }

  function countdown(epochSeconds) {
    const left = Math.max(0, epochSeconds * 1000 - Date.now());
    const hours = Math.floor(left / 3600000);
    const minutes = Math.floor((left % 3600000) / 60000);
    return hours ? `${hours}H ${String(minutes).padStart(2, '0')}M` : `${minutes}M`;
  }

  // 1234 → « 1,2K », 1 480 000 → « 1,5M » (compteur de tokens du jour).
  function compactCount(n) {
    const v = Number(n) || 0;
    if (v >= 1e6) return `${(v / 1e6).toFixed(1).replace('.', ',')}M`;
    if (v >= 1e3) return `${(v / 1e3).toFixed(v >= 1e5 ? 0 : 1).replace('.', ',')}K`;
    return String(v);
  }

  const STATUS_LINE = 18;

  // Hauteur occupée par le bloc d'état, pour que le panneau mémoire se pose dessus.
  // Le bloc d'état vit désormais en bas à droite, comme les infos matériel en bas à gauche :
  // il ne prend plus de place au-dessus de la barre de saisie (les panneaux gardent leur hauteur).
  function statusHeight() {
    return 0;
  }

  // Segments colorés alignés à droite sur la ligne de base y, avec « · » entre eux. Un segment
  // [texte, couleur, { bar: n }] est précédé d'une barre de 10 cases dont n sont pleines.
  const BAR_CELL = 5;
  function partWidth(part) {
    const text = tw(g, part[0]);
    return part[2] && part[2].bar !== undefined ? 10 * BAR_CELL + tw(g, ' ') + text : text;
  }

  function drawPart(part, x, y) {
    const [text, color, extra] = part;
    if (extra && extra.bar !== undefined) {
      for (let i = 0; i < 10; i++) {
        g.fillStyle = i < extra.bar ? color : 'rgba(56, 225, 255, 0.18)';
        g.fillRect(x + i * BAR_CELL, y - 6, BAR_CELL - 1.5, 6);
      }
      x += 10 * BAR_CELL + tw(g, ' ');
    }
    g.fillStyle = color;
    g.fillText(text, x, y);
  }

  function rightParts(parts, y, limitLeft) {
    const dim = 'rgba(56, 225, 255, 0.3)';
    const sep = ' · ';
    const sepW = tw(g, sep);
    const width = (list) => list.reduce((sum, part, i) => sum + partWidth(part) + (i ? sepW : 0), 0);
    // Pas de chevauchement avec la barre de saisie : on retire les segments les moins importants.
    let shown = parts.slice();
    while (shown.length > 1 && W - 34 - width(shown) < limitLeft) shown.pop();
    let x = W - 34 - width(shown);
    g.textAlign = 'left';
    shown.forEach((part, i) => {
      if (i) {
        g.fillStyle = dim;
        g.fillText(sep, x, y);
        x += sepW;
      }
      drawPart(part, x, y);
      x += partWidth(part);
    });
  }

  // Quota du compte Claude, modèle et effort : les mêmes chiffres que la statusline.
  function statusPanel() {
    if (!showStatus || (!status && !journal)) return;

    g.save();
    g.letterSpacing = '1px';
    g.font = `8.5px ${MONO}`;

    // La barre de saisie est centrée : ces lignes s'arrêtent avant elle.
    const box = getConsoleRect();
    const limitLeft = box.right + 24 / S;
    const dimText = 'rgba(160, 245, 255, 0.55)';

    // Ligne 0 (v5) : consommation du jour — appels IA, tokens entrants ↓ / sortants ↑, réponses locales.
    if (status && status.today) {
      const td = status.today;
      const approx = td.estimated ? '≈' : '';
      const parts = [
        [`IA AUJOURD'HUI ${td.calls} APPEL${td.calls > 1 ? 'S' : ''}`, 'rgba(255, 150, 60, 0.75)'],
        [`↓${approx}${compactCount(td.in)} ↑${approx}${compactCount(td.out)} TOKENS`, 'rgba(255, 170, 90, 0.7)'],
        [`${td.local + td.ignored} SANS IA`, 'rgba(53, 255, 158, 0.6)']
      ];
      rightParts(parts, H - 49, limitLeft);
    }

    // Ligne 1 : moteur, modèle, effort — remplacée, pendant un appel, par l'IA sollicitée (orange).
    if (status && aiCalls.size > 0) {
      const calls = [...aiCalls.values()];
      const first = calls[0];
      const seconds = Math.max(0, Math.round((Date.now() - first.startedAt) / 1000));
      const pulse = 0.65 + 0.35 * Math.sin(performance.now() / 180);
      const name = first.engine === 'agy' ? 'AGY' : 'CLAUDE CODE';
      const parts = [[`● ${name} SOLLICITÉ${calls.length > 1 ? ` (×${calls.length})` : ''}`, `rgba(255, 122, 24, ${pulse})`]];
      if (first.model) parts.push([String(first.model).replace('claude-', '').toUpperCase(), 'rgba(255, 160, 80, 0.85)']);
      parts.push([`${seconds} S`, 'rgba(255, 160, 80, 0.85)']);
      rightParts(parts, H - 36, limitLeft);
    } else if (status) {
      const eng = (status.engine || 'claude').toUpperCase();
      const model = (status.model || 'modèle par défaut').replace('claude-', '');
      const parts = [[`[${eng}] ${model}`.toUpperCase(), dimText]];
      if (status.effort) parts.push([`EFFORT ${status.effort}`.toUpperCase(), dimText]);
      if (status.economyMode || status.model === 'auto') parts.push(['ÉCO', 'rgba(53, 255, 158, 0.6)']);
      rightParts(parts, H - 36, limitLeft);
    }

    // Ligne 2 : quota sur 5 h (barre) et sur 7 jours.
    if (status) {
      const usage = status.usage;
      if (!usage) {
        rightParts([['QUOTA NON RELEVÉ', 'rgba(56, 225, 255, 0.35)']], H - 23, limitLeft);
      } else {
        const pct = usage.five.pct;
        const filled = Math.max(0, Math.min(10, Math.round(pct / 10)));
        const color = pct >= 90 ? 'rgba(255, 77, 94, 0.9)' : pct >= 70 ? 'rgba(255, 176, 32, 0.9)' : 'rgba(53, 255, 158, 0.75)';
        const parts = [[`${pct}% SUR 5H`, color, { bar: filled }]];
        if (usage.five.resetsAt) parts.push([`RESET ${countdown(usage.five.resetsAt)}`, 'rgba(56, 225, 255, 0.4)']);
        parts.push([`7 JOURS ${usage.week.pct}%${usage.overage ? ' · DÉPASSEMENT' : ''}`, 'rgba(56, 225, 255, 0.4)']);
        rightParts(parts, H - 23, limitLeft);
      }
    }

    // Ligne 3 : journal scellé.
    if (journal) {
      const intact = journal.ok !== false;
      rightParts([[
        intact
          ? `JOURNAL SCELLÉ · ${journal.count} ENTRÉES · ${journal.hash}`
          : `JOURNAL ALTÉRÉ · ${journal.reason || 'chaîne rompue'}`,
        intact ? 'rgba(53, 255, 158, 0.55)' : 'rgba(255, 77, 94, 0.9)'
      ]], H - 10, limitLeft);
    }

    g.restore();
  }

  function wrapLines(text, maxWidth) {
    if (!text) return [];
    const paragraphs = String(text).split('\n');
    const lines = [];
    for (const p of paragraphs) {
      if (!p.trim()) {
        lines.push('');
        continue;
      }
      const words = p.split(/\s+/);
      let current = '';
      for (const w of words) {
        const test = current ? `${current} ${w}` : w;
        if (g.measureText(test).width > maxWidth) {
          if (current) lines.push(current);
          current = w;
        } else {
          current = test;
        }
      }
      if (current) lines.push(current);
    }
    return lines;
  }

  let verifyBtnCoords = null;

  function journalPanel() {
    if (!showJournal) {
      verifyBtnCoords = null;
      journalBox = null;
      return;
    }

    const theme = (J.THEMES && J.THEMES[currentTheme]) || J.THEMES.cyan;
    const rgb = theme.primaryRgb;
    const densityCfg = (J.DENSITIES && J.DENSITIES[currentDensity]) || J.DENSITIES.normal;

    const width = Math.min(680, W * Math.max(0.44, densityCfg.panelWidthRatio));
    const height = Math.min(540, H - 200);
    const x = journalCustomPos ? Math.max(14, Math.min(W - width - 14, journalCustomPos.x)) : (W - 34 - width);
    const y = journalCustomPos ? Math.max(60, Math.min(H - height - 10, journalCustomPos.y)) : 110;
    journalBox = { x, y, width, height };

    g.save();
    // Fond biseauté sombre HUD
    g.fillStyle = 'rgba(2, 16, 26, 0.90)';
    g.fillRect(x, y, width, height);
    g.lineWidth = 1;
    g.strokeStyle = theme.border || `rgba(${rgb}, 0.35)`;
    g.strokeRect(x, y, width, height);

    // En-tête du panneau
    g.letterSpacing = '2px';
    g.font = `600 ${densityCfg.fontTitle} Consolas, "Courier New", monospace`;
    g.fillStyle = `rgba(${rgb}, 0.92)`;
    const scrollBadge = journalScrollOffset > 0 ? ` · ▲ RECUL +${journalScrollOffset} (MOLETTE/PGUP)` : '';
    g.fillText(`JOURNAL & RÉPONSE INTÉGRALE${scrollBadge}`, x + 14, y + 22);

    // Indicateur de préhension du panneau journal
    g.fillStyle = `rgba(${rgb}, ${draggedPanel === 'journal' ? 0.9 : 0.4})`;
    for (let r = 0; r < 2; r++) {
      for (let c = 0; c < 3; c++) {
        g.fillRect(x + width - 20 + c * 3, y + 14 + r * 3, 1.5, 1.5);
      }
    }

    // Statut d'intégrité scellée et bouton de vérification
    const intact = !journal || journal.ok !== false;
    const count = journal ? journal.count : 0;
    const hash = journal && journal.hash ? journal.hash : '--------';
    g.letterSpacing = '1px';
    g.font = `${densityCfg.fontTitle} Consolas, "Courier New", monospace`;
    g.fillStyle = intact ? 'rgba(53, 255, 158, 0.85)' : 'rgba(255, 77, 94, 0.95)';
    const statusText = intact
      ? `SCELLÉ SHA-256 [${hash}] · ${count} ENTRÉES INTACTES`
      : `ALTÉRATION DÉTECTÉE · ${journal && journal.reason ? journal.reason : 'CHAÎNE ROMPUE'}`;
    g.fillText(statusText, x + 14, y + 40);

    // Bouton de vérification d'intégrité
    const btnW = 160;
    const btnH = 20;
    const btnX = x + width - 14 - btnW;
    const btnY = y + 26;
    verifyBtnCoords = { x: btnX, y: btnY, w: btnW, h: btnH };
    g.fillStyle = `rgba(${rgb}, 0.15)`;
    g.fillRect(btnX, btnY, btnW, btnH);
    g.strokeStyle = `rgba(${rgb}, 0.5)`;
    g.strokeRect(btnX, btnY, btnW, btnH);
    g.fillStyle = 'rgba(190, 240, 255, 0.95)';
    g.textAlign = 'center';
    g.fillText('VÉRIFIER INTÉGRITÉ (V)', btnX + btnW / 2, btnY + 14);
    g.textAlign = 'left';

    // Ligne de séparation
    g.strokeStyle = `rgba(${rgb}, 0.2)`;
    g.beginPath();
    g.moveTo(x + 14, y + 54);
    g.lineTo(x + width - 14, y + 54);
    g.stroke();

    // Section 1 : Dernière réponse complète (sans troncature)
    g.letterSpacing = '1.5px';
    g.font = `600 ${densityCfg.fontTitle} Consolas, "Courier New", monospace`;
    g.fillStyle = `rgba(${rgb}, 0.85)`;
    g.fillText('DERNIÈRE RÉPONSE COMPLÈTE', x + 14, y + 72);

    g.letterSpacing = '0px';
    g.font = `${densityCfg.fontText} Consolas, "Courier New", monospace`;
    let curY = y + 90;

    const answerLinesLimit = currentDensity === 'compact' ? 6 : (currentDensity === 'large' ? 10 : 8);
    const lineStep = densityCfg.lineHeight - 2;

    if (lastFullAnswer && lastFullAnswer.text) {
      if (lastFullAnswer.query) {
        g.fillStyle = `rgba(${rgb}, 0.65)`;
        g.fillText(`> ${clip(lastFullAnswer.query, width - 40)}`, x + 14, curY);
        curY += lineStep + 1;
      }
      g.fillStyle = 'rgba(235, 250, 255, 0.96)';
      const answerW = Math.round(width - 30);
      if (!lastFullAnswer._cachedLines || lastFullAnswer._cachedW !== answerW) {
        lastFullAnswer._cachedLines = wrapLines(lastFullAnswer.text, width - 30);
        lastFullAnswer._cachedW = answerW;
      }
      const answerLines = lastFullAnswer._cachedLines.slice(0, answerLinesLimit);
      for (const line of answerLines) {
        g.fillText(line, x + 14, curY);
        curY += lineStep;
      }
      if (answerLines.length === answerLinesLimit) {
        g.fillStyle = 'rgba(190, 240, 255, 0.45)';
        g.fillText('… [texte complet archivé au journal]', x + 14, curY);
        curY += lineStep;
      }
    } else {
      g.fillStyle = 'rgba(190, 240, 255, 0.35)';
      g.fillText('Aucune réponse complète pour le moment.', x + 14, curY);
      curY += 20;
    }

    // Ligne de séparation
    curY = Math.max(curY + 6, y + (currentDensity === 'compact' ? 210 : 240));
    g.strokeStyle = `rgba(${rgb}, 0.2)`;
    g.beginPath();
    g.moveTo(x + 14, curY);
    g.lineTo(x + width - 14, curY);
    g.stroke();
    curY += 18;

    // Section 2 : Entrées du journal scellé (chronologie consultable avec défilement)
    g.letterSpacing = '1.5px';
    g.font = `600 ${densityCfg.fontTitle} Consolas, "Courier New", monospace`;
    g.fillStyle = 'rgba(53, 255, 158, 0.85)';
    const historyTitle = journalScrollOffset > 0
      ? `JOURNAL D’AUDIT SCELLÉ (DÉCALAGE +${journalScrollOffset} · FIN/END POUR DIRECT)`
      : 'JOURNAL D’AUDIT SCELLÉ (CHRONOLOGIE)';
    g.fillText(historyTitle, x + 14, curY);
    curY += 18;

    g.letterSpacing = '0px';
    g.font = `${densityCfg.fontText} Consolas, "Courier New", monospace`;

    const maxLogs = densityCfg.panelLines + (currentDensity === 'compact' ? 3 : 1);
    const allLogs = journalEntries || [];
    const recentLogs = allLogs.slice(journalScrollOffset, journalScrollOffset + maxLogs);

    if (!recentLogs.length) {
      g.fillStyle = 'rgba(190, 240, 255, 0.35)';
      g.fillText(allLogs.length ? 'Début de l’historique atteint.' : 'Chargement du journal…', x + 14, curY);
    } else {
      for (const item of recentLogs) {
        if (curY > y + height - 16) break;
        if (!item._cachedDesc) {
          let desc = '';
          if (item.data) {
            if (item.data.texte) desc = item.data.texte;
            else if (item.data.demande) desc = item.data.demande;
            else if (item.data.detail) desc = item.data.detail;
            else if (item.data.action) desc = item.data.action;
            else if (item.data.moteur) desc = `moteur ${item.data.moteur}`;
            else desc = typeof item.data === 'string' ? item.data : JSON.stringify(item.data);
          }
          item._cachedDesc = desc;
          item._cachedTime = (item.at || '').slice(11, 19);
          item._cachedType = clip((item.type || '').toUpperCase(), 110);
        }
        g.fillStyle = 'rgba(53, 255, 158, 0.7)';
        g.fillText(item._cachedTime, x + 14, curY);
        g.fillStyle = `rgba(${rgb}, 0.85)`;
        g.fillText(item._cachedType, x + 82, curY);
        g.fillStyle = 'rgba(235, 250, 255, 0.9)';
        g.fillText(clip(item._cachedDesc, width - 220), x + 195, curY);
        curY += densityCfg.lineHeight;
      }
    }

    g.restore();
  }

  const MONO = 'Consolas, "Courier New", monospace';
  const LISTEN_LABELS = { wakeword: 'mot d’éveil', ptt: 'appui pour parler', free: 'écoute libre' };

  // En-tête volontairement sobre : identité, état de l'écoute, alertes. Le moteur et le quota
  // sont dans le bloc d'état (bas droite), le matériel en bas à gauche, les touches dans l'aide.
  function header(now) {
    // Allumage : titre et état vocal se « décodent », l'horloge et les boutons suivent.
    const headP = boot.ui('header');
    const clockP = boot.ui('clock');
    if (headP <= 0 && clockP <= 0) return;
    g.textAlign = 'left';
    g.textBaseline = 'alphabetic';
    g.letterSpacing = '4px';
    g.font = `600 15px ${MONO}`;
    g.fillStyle = 'rgba(160, 245, 255, 0.85)';
    g.fillText(boot.decode('J.A.R.V.I.S.', headP), 34, 46);

    g.letterSpacing = '2px';
    g.font = `10px ${MONO}`;
    let y = 66;
    const vState = voiceStatus();
    g.fillStyle = vState.color;
    g.fillText(boot.decode(vState.text, headP * 1.4 - 0.4), 34, y);
    if (headP < 1) {
      g.letterSpacing = '0px';
      if (clockP <= 0) return;
    }

    const active = frameStats.active.length;
    const waiting = frameStats.awaiting;
    if (waiting) {
      y += 18;
      g.fillStyle = 'rgba(255, 176, 32, 0.85)';
      g.fillText(`VALIDATION EN ATTENTE ${waiting}`, 34, y);
    }
    if (active) {
      y += 18;
      g.fillStyle = 'rgba(56, 225, 255, 0.45)';
      g.fillText(`BRANCHES ACTIVES ${String(active).padStart(2, '0')}`, 34, y);
    }

    // Horloge juste sous les boutons de fenêtre (½ ÉCRAN, WIDGET : HTML, en haut à droite), puis
    // CONFIG et AIDE dessous. Le bas des boutons est mesuré dans la page : quelle que soit l'échelle,
    // l'heure ne passe plus derrière eux.
    const clockY = Math.max(46, windowControlsBottom() + 17);
    const d = new Date();
    const clock = [d.getHours(), d.getMinutes(), d.getSeconds()]
      .map((n) => String(n).padStart(2, '0'))
      .join(':');
    g.save();
    g.globalAlpha = boot.ease.outCubic(clockP);
    g.textAlign = 'right';
    g.letterSpacing = '3px';
    g.font = `600 14px ${MONO}`;
    g.fillStyle = 'rgba(160, 245, 255, 0.7)';
    g.fillText(boot.decode(clock, clockP * 1.5), W - 34, clockY);
    g.letterSpacing = '0px';
    g.textAlign = 'left';

    helpButton(clockY + 10);
    g.restore();
  }

  // Bas des boutons de fenêtre, en unités du canevas (pixels de la page / S). Relu deux fois par seconde.
  let controlsBottom = { at: -1e9, v: 0 };
  function windowControlsBottom() {
    const now = performance.now();
    if (now - controlsBottom.at > 500) {
      const group = document.querySelector('#window-controls .win-btn-group');
      const r = group ? group.getBoundingClientRect() : null;
      controlsBottom = { at: now, v: r && r.height ? r.bottom / S : 0 };
    }
    return controlsBottom.v;
  }

  // Petit bouton « ? AIDE » sous l'horloge : affiche la liste des touches (aussi avec F1).
  function helpButton(y = 56) {
    const theme = (J.THEMES && J.THEMES[currentTheme]) || J.THEMES.cyan;
    const rgb = theme.primaryRgb;
    g.font = `9px ${MONO}`;
    g.letterSpacing = '2px';
    const label = showHelp ? '× FERMER' : '? AIDE · F1';
    const w = tw(g, label) + 16;
    const h = 18;
    const x = W - 34 - w;
    helpBtnCoords = { x, y, w, h };
    J.hudButtonsBottom = (y + h) * S; // pixels de la page : cards-dock.js pose ses boutons dessous
    const hover = mousePos.x >= x && mousePos.x <= x + w && mousePos.y >= y && mousePos.y <= y + h;
    g.fillStyle = `rgba(${rgb}, ${showHelp || hover ? 0.16 : 0.06})`;
    g.fillRect(x, y, w, h);
    g.lineWidth = 1;
    g.strokeStyle = `rgba(${rgb}, ${showHelp || hover ? 0.6 : 0.3})`;
    g.strokeRect(x + 0.5, y + 0.5, w - 1, h - 1);
    g.fillStyle = `rgba(190, 240, 255, ${showHelp || hover ? 0.95 : 0.6})`;
    g.fillText(label, x + 8, y + 12.5);

    // Bouton « CONFIG » à gauche de l'aide : ouvre la page d'administration (aussi avec F10).
    const cLabel = '⚙ CONFIG · F10';
    const cw = tw(g, cLabel) + 16;
    const cx = x - 8 - cw;
    configBtnCoords = { x: cx, y, w: cw, h };
    const cHover = mousePos.x >= cx && mousePos.x <= cx + cw && mousePos.y >= y && mousePos.y <= y + h;
    g.fillStyle = `rgba(${rgb}, ${cHover ? 0.16 : 0.06})`;
    g.fillRect(cx, y, cw, h);
    g.strokeStyle = `rgba(${rgb}, ${cHover ? 0.6 : 0.3})`;
    g.strokeRect(cx + 0.5, y + 0.5, cw - 1, h - 1);
    g.fillStyle = `rgba(190, 240, 255, ${cHover ? 0.95 : 0.6})`;
    g.fillText(cLabel, cx + 8, y + 12.5);
    g.letterSpacing = '0px';
  }

  // Page d'aide complète (ui/aide.html) dans le navigateur ; sans passerelle, repli sur l'aide-mémoire du HUD.
  function openHelp() {
    const b = window.jarvisBridge;
    if (b && b.openHelp) b.openHelp().catch(() => { showHelp = !showHelp; });
    else showHelp = !showHelp;
  }

  function openAdmin() {
    if (window.jarvisBridge && window.jarvisBridge.openAdmin) window.jarvisBridge.openAdmin();
  }

  function helpPanel() {
    if (!showHelp) {
      helpBox = null;
      return;
    }
    const theme = (J.THEMES && J.THEMES[currentTheme]) || J.THEMES.cyan;
    const rgb = theme.primaryRgb;
    const themeName = ((J.THEMES && J.THEMES[currentTheme]) || { name: 'Cyan' }).name;
    const densName = ((J.DENSITIES && J.DENSITIES[currentDensity]) || { name: 'Normal' }).name;
    const rows = [
      ['F1', 'Ouvrir la page d’aide complète'],
      ['MAJ + F1', 'Afficher / masquer cet aide-mémoire'],
      ['« AIDE »', 'Dis « aide pour … » : la commande à dire'],
      ['F2', 'Mode débogage'],
      ['F3', 'Couper / rétablir le micro'],
      ['F4', `Thème suivant (${themeName})`],
      ['MAJ + F4', `Densité suivante (${densName})`],
      ['F5', 'Transcription'],
      ['MAJ + F5', 'Apprentissage des voix (« qui parle ? »)'],
      ['F6', 'Mémoire'],
      ['F7', 'État : moteur, quota, journal'],
      ['F8', `Mode d’écoute (${LISTEN_LABELS[listeningMode] || listeningMode})`],
      ['F9', 'Journal et réponse complète'],
      ['F10', 'Configuration (administration)'],
      ['V', 'Vérifier le journal (panneau ouvert)'],
      ['ESPACE', 'Parler (mode appui pour parler)'],
      ['PG↑ / PG↓', 'Faire défiler les panneaux'],
      ['CTRL+MAJ+J', 'Afficher / masquer Jarvis'],
      ['CTRL+MAJ+W', 'Mode widget flottant (80 × 80)'],
      ['CTRL+MAJ+G', 'Widget fantôme (tout traverse)'],
      ['CTRL maintenu', 'Widget effacé, les clics traversent'],
      ['CLIC MILIEU ×2', 'Compagnon : ne pas déranger / réveil'],
      ['ÉCHAP', 'Tout arrêter (explorateur 3D ouvert : le fermer)'],
      ['CTRL+Q', 'Quitter']
    ];
    const lineH = 20;
    const width = 400;
    const height = rows.length * lineH + 44;
    const x = W - 34 - width;
    const y = helpBtnCoords ? helpBtnCoords.y + helpBtnCoords.h + 10 + DOCK_BTNS_H : 84 + DOCK_BTNS_H; // sous les boutons AIDE puis RANGER / ACTIONS
    helpBox = { x, y, width, height };

    g.save();
    g.fillStyle = 'rgba(2, 14, 22, 0.94)';
    g.fillRect(x, y, width, height);
    g.lineWidth = 1;
    g.strokeStyle = theme.border || `rgba(${rgb}, 0.35)`;
    g.strokeRect(x + 0.5, y + 0.5, width - 1, height - 1);

    g.letterSpacing = '2px';
    g.font = `600 10px ${MONO}`;
    g.fillStyle = `rgba(${rgb}, 0.85)`;
    g.fillText('RACCOURCIS CLAVIER', x + 14, y + 22);

    g.letterSpacing = '0.5px';
    rows.forEach(([key, action], i) => {
      const ry = y + 44 + i * lineH;
      g.font = `600 11px ${MONO}`;
      g.fillStyle = `rgba(${rgb}, 0.9)`;
      g.fillText(key, x + 14, ry);
      g.font = `12px ${MONO}`;
      g.fillStyle = 'rgba(220, 248, 255, 0.88)';
      g.fillText(clip(action, width - 140), x + 124, ry);
    });
    g.restore();
  }

  // Informations matérielles et chaîne vocale : discrètes, en bas à gauche.
  function hardwareInfo() {
    const v = J.voice;
    const activeTasks = frameStats.active;
    const isVoiceActive = !!(v && (v.capturing || v.speaking || level > 0.04));
    const hasRecentInput = (performance.now() - lastUserInputAt) < 2500;
    const isBusy = activeTasks.length > 0 || activityEnergy > 0.05 || isVoiceActive || hasRecentInput;

    const cpuVal = telemetry.cpu || 0;
    const ramPctVal = telemetry.ramPct || 0;
    const cpuColor = cpuVal >= 80 ? 'rgba(255, 77, 94, 0.85)' : (cpuVal >= 50 ? 'rgba(255, 176, 32, 0.8)' : 'rgba(53, 255, 158, 0.55)');
    const ramColor = ramPctVal >= 85 ? 'rgba(255, 77, 94, 0.85)' : (ramPctVal >= 70 ? 'rgba(255, 176, 32, 0.7)' : 'rgba(56, 225, 255, 0.5)');
    const dim = 'rgba(56, 225, 255, 0.35)';

    const parts = perf.hardwareMonitoring
      ? [
        [`CPU ${cpuVal}%`, cpuColor],
        [`RAM ${telemetry.ramUsedGb || '0'}/${telemetry.ramTotalGb || '0'} GO (${ramPctVal}%)`, ramColor],
        [`${fps.toFixed(0)} FPS${isBusy ? '' : ' · ÉCO'}`, dim]
      ]
      : [
        ['SURVEILLANCE MATÉRIEL DÉSACTIVÉE', dim],
        [`${fps.toFixed(0)} FPS${isBusy ? '' : ' · ÉCO'}`, dim]
      ];
    if (isResourceEcoMode) parts.push(['[MODE ÉCO · CHARGE > 90%]', 'rgba(255, 77, 94, 0.95)']);
    // Mode automatique : discret tant que rien n'est réduit, signalé dès qu'il a réduit des options.
    if (autoInfo) {
      parts.push(autoInfo.labels && autoInfo.labels.length
        ? [`AUTO · ${autoInfo.labels.length} OPTION${autoInfo.labels.length > 1 ? 'S' : ''} RÉDUITE${autoInfo.labels.length > 1 ? 'S' : ''}`, 'rgba(255, 176, 32, 0.8)']
        : ['AUTO', 'rgba(53, 255, 158, 0.45)']);
    }
    if (debug) parts.push([`${tasks.size} TÂCHES`, dim]);

    g.textAlign = 'left';
    g.letterSpacing = '1px';
    g.font = `8.5px ${MONO}`;
    let x = 34;
    const y1 = H - 36;
    parts.forEach(([text, color], i) => {
      if (i) {
        g.fillStyle = dim;
        g.fillText(' · ', x, y1);
        x += tw(g, ' · ');
      }
      g.fillStyle = color;
      g.fillText(text, x, y1);
      x += tw(g, text);
    });

    // Ligne 2 : Périphérique audio actif & basculement interactif (Casque <-> Enceintes/Micro PC)
    const y2 = H - 23;
    const isCasque = audioDevice && audioDevice.mode === 'casque';
    const audioIcon = isCasque ? '🎧' : '🔊';
    const modeLabel = isCasque ? 'CASQUE' : 'ENCEINTES & MICRO PC';

    let devName = isCasque
      ? (audioDevice.headsetName || audioDevice.render || 'Casque')
      : (audioDevice.pcName || audioDevice.render || 'Enceintes');
    devName = devName.replace(/Haut-parleurs\s*\((.*?)\)/i, '$1').replace(/\(R\)/gi, '').replace(/High Definition Audio/i, 'HD Audio').trim();
    if (devName.length > 28) devName = devName.slice(0, 26) + '…';

    const audioText = `AUDIO : ${audioIcon} ${modeLabel} (${devName})`;
    const toggleTag = ' [⇄ BASCULER]';

    const wText = tw(g, audioText);
    const wTag = tw(g, toggleTag);
    const totalW = wText + wTag;

    const bx = 30;
    const by = y2 - 10;
    const bw = totalW + 8;
    const bh = 14;
    audioBtnCoords = { x: bx, y: by, w: bw, h: bh };

    const isHover = mousePos.x >= bx && mousePos.x <= bx + bw && mousePos.y >= by && mousePos.y <= by + bh;
    if (isHover) {
      g.fillStyle = 'rgba(56, 225, 255, 0.14)';
      g.fillRect(bx, by, bw, bh);
      g.strokeStyle = 'rgba(56, 225, 255, 0.6)';
      g.lineWidth = 1;
      g.strokeRect(bx, by, bw, bh);
    }

    g.fillStyle = isHover ? 'rgba(255, 255, 255, 0.95)' : (isCasque ? 'rgba(53, 255, 158, 0.75)' : 'rgba(255, 190, 40, 0.85)');
    g.fillText(audioText, 34, y2);

    g.fillStyle = isHover ? 'rgba(56, 225, 255, 1)' : 'rgba(56, 225, 255, 0.65)';
    g.fillText(toggleTag, 34 + wText, y2);

    const voiceRateTag = v && v.rate && v.rate !== '+0%' ? ` (${v.rate})` : '';
    g.fillStyle = dim;
    g.fillText(
      v && v.model ? `WHISPER ${v.model} · VOIX ${v.name || 'DENISE'}${voiceRateTag}`.toUpperCase() : (J.voice && J.voice.disabled ? 'ÉCOUTE ORALE DÉSACTIVÉE' : 'CHAÎNE VOCALE NON CONNECTÉE'),
      34,
      H - 10
    );
    g.letterSpacing = '0px';
  }

  let last = performance.now();
  let fps = 0;
  // Mesure (lue par CDP dans les tests) : images par seconde, durée moyenne du dessin d'une image (ms),
  // état du circuit (pistes, impulsions, routes en cache, durée du dernier routage).
  let frameMs = 0;
  let frameWorkStart = 0;
  window.__hudPerf = () => ({
    fps: Math.round(fps), frameMs: Math.round(frameMs * 100) / 100, eco: isResourceEcoMode,
    circuit: J.circuit ? J.circuit.stats() : null
  });

  // --- Dynamique Cinématique Apaisée (Transitions de taille calmes et lentes) ---
  function smoothTransitionOrb(current, target, dt) {
    if (!current) return target;
    const isGrowing = target > current;
    // Mouvements de taille beaucoup plus lents et majestueux (uniquement en taille)
    // Montée très progressive et descente très douce sans à-coups
    const rate = isGrowing ? 0.12 : 0.035;
    const blend = 1 - Math.exp(-rate * Math.max(0, dt));
    return current + (target - current) * blend;
  }

  // --- Rafraîchissement Adaptatif & Dynamique d'Interaction (Chantiers B2/B3/Étape 12) ---
  let lastUserInputAt = performance.now();
  let lastDirectInterventionAt = 0;
  // Veille du cœur (voir frame) : dernière sollicitation réelle, et niveau de veille lissé (0 → 1).
  const SLEEP_AFTER_MS = 60000;
  let lastSolicitedAt = performance.now();
  let sleepLevel = 0;
  J.markSolicited = () => { lastSolicitedAt = performance.now(); };
  let lastRenderTimestamp = 0;
  let lastRafAt = 0;
  let rafStep = 16.7; // intervalle moyen entre deux images d'écran (ms)
  window.addEventListener('pointermove', () => { lastUserInputAt = performance.now(); }, { passive: true });
  window.addEventListener('keydown', () => {
    lastUserInputAt = performance.now();
    lastDirectInterventionAt = performance.now();
  }, { passive: true });

  // --- Séquence d'allumage (boot.js) ------------------------------------------------------------
  // Le cœur s'allume, ses anneaux et ses traits se dessinent au fil des étapes réelles du lancement,
  // puis l'interface se déploie. Sans boot.js, tout est visible d'emblée.
  const boot = J.boot || {
    update() {}, skip() {}, isReady: () => true, orbReveal: () => null, ui: () => 1,
    decode: (s) => s, drawOverlay() {}, setLite() {},
    ease: { outCubic: (x) => x, outBack: (x) => x }
  };
  boot.onReady = () => {
    const primaryName = (J.voice && J.voice.speaker && J.voice.speaker.name) ? J.voice.speaker.name : 'Monsieur';
    speak(`Système initialisé. Tous les services sont opérationnels, ${primaryName}.`);
    input.focus();
  };

  // Pendant la séquence, clavier et souris ne font que la passer (acte par acte).
  function bootIntercept(e) {
    if (boot.isReady()) return;
    if (e.type === 'keydown' && (e.key === 'Shift' || e.key === 'Control' || e.key === 'Alt')) return;
    e.preventDefault();
    e.stopImmediatePropagation();
    if (e.type !== 'click') boot.skip();
  }
  window.addEventListener('keydown', bootIntercept, true);
  window.addEventListener('mousedown', bootIntercept, true);
  window.addEventListener('click', bootIntercept, true);

  // Panneau encadré : les 4 coins apparaissent, le cadre s'ouvre depuis sa ligne centrale, puis le
  // contenu arrive. La boîte est celle de l'image précédente (tant qu'elle est inconnue, le panneau
  // est dessiné invisible pour la calculer).
  // Card Conversation : sur le canevas du fond, ou, une fois cliquée (ui/card-front.js), sur celui de devant pour passer
  // au-dessus des autres cards. Sur celui de devant, un fond plein évite de voir les cards dessous à travers.
  function drawTranscript() {
    const front = Boolean(frontCtx && J.cardFront && J.cardFront.convRaised());
    if (!front) {
      if (frontDirty) {
        frontCtx.save();
        frontCtx.setTransform(1, 0, 0, 1, 0, 0);
        frontCtx.clearRect(0, 0, frontCanvas.width, frontCanvas.height);
        frontCtx.restore();
        frontDirty = false;
      }
      revealPanel('transcript', () => transcriptBox, transcriptPanel);
      return;
    }
    const back = g;
    frontCtx.save();
    frontCtx.setTransform(1, 0, 0, 1, 0, 0);
    frontCtx.clearRect(0, 0, frontCanvas.width, frontCanvas.height);
    frontCtx.restore();
    g = frontCtx;
    try {
      if (showTranscript && transcriptBox && !transcriptCollapsed) {
        g.fillStyle = 'rgba(2, 12, 20, 0.97)';
        g.fillRect(transcriptBox.x, transcriptBox.y, transcriptBox.width, transcriptBox.height);
      }
      revealPanel('transcript', () => transcriptBox, transcriptPanel);
    } finally {
      g = back;
    }
    frontDirty = true;
  }

  function revealPanel(name, getBox, draw) {
    const p = boot.ui(`panel:${name}`);
    if (p >= 1) {
      draw();
      return;
    }
    const box = getBox();
    g.save();
    if (p <= 0 || !box) {
      g.globalAlpha = 0;
      draw();
      g.restore();
      return;
    }
    const rgb = ((J.THEMES && J.THEMES[currentTheme]) || J.THEMES.cyan).primaryRgb;
    const open = boot.ease.outCubic((p - 0.15) / 0.5);
    const midY = box.y + box.height / 2;
    const h = Math.max(1, box.height * open);
    g.save();
    g.beginPath();
    g.rect(box.x - 2, midY - h / 2, box.width + 4, h);
    g.clip();
    g.globalAlpha = 0.25 + 0.75 * boot.ease.outCubic((p - 0.4) / 0.6);
    draw();
    g.restore();

    // Coins du cadre, puis lignes lumineuses sur les bords qui s'ouvrent
    const c = 12;
    const cornerA = Math.min(1, p * 4) * (1 - Math.max(0, (p - 0.75) / 0.25));
    const { x, y, width: w, height: bh } = box;
    g.lineWidth = 1.5;
    g.strokeStyle = `rgba(${rgb}, ${0.9 * cornerA})`;
    g.beginPath();
    g.moveTo(x, y + c); g.lineTo(x, y); g.lineTo(x + c, y);
    g.moveTo(x + w - c, y); g.lineTo(x + w, y); g.lineTo(x + w, y + c);
    g.moveTo(x, y + bh - c); g.lineTo(x, y + bh); g.lineTo(x + c, y + bh);
    g.moveTo(x + w - c, y + bh); g.lineTo(x + w, y + bh); g.lineTo(x + w, y + bh - c);
    g.stroke();
    if (open < 1) {
      const grow = (w / 2) * boot.ease.outCubic(p / 0.2);
      g.globalCompositeOperation = 'lighter';
      g.strokeStyle = `rgba(${rgb}, ${0.9 * (1 - open * 0.6)})`;
      g.lineWidth = 1.2;
      g.beginPath();
      g.moveTo(x + w / 2 - grow, midY - h / 2); g.lineTo(x + w / 2 + grow, midY - h / 2);
      g.moveTo(x + w / 2 - grow, midY + h / 2); g.lineTo(x + w / 2 + grow, midY + h / 2);
      g.stroke();
    }
    g.restore();
  }

  // Élément sans cadre (lignes d'état, en-tête) : fondu et légère montée.
  function revealFade(name, draw) {
    const p = boot.ui(name);
    if (p >= 1) {
      draw();
      return;
    }
    if (p <= 0) return;
    const e = boot.ease.outCubic(p);
    g.save();
    g.globalAlpha = e;
    g.translate(0, (1 - e) * 10);
    draw();
    g.restore();
  }

  // Affichage épuré et ultra-léger du cœur en cas de charge CPU ou RAM élevée (> 90%)
  function drawSimpleEcoOrb(g, cx, cy, R, t, cpu, ram) {
    g.save();
    const r = Math.max(34, R * 0.42);

    // Fond circulaire sobre
    g.fillStyle = 'rgba(4, 18, 28, 0.85)';
    g.beginPath();
    g.arc(cx, cy, r, 0, Math.PI * 2);
    g.fill();

    // Anneau ambre avec pulsation douce
    const pulse = 0.75 + 0.25 * Math.sin(t * 3.0);
    g.lineWidth = 1.6;
    g.strokeStyle = `rgba(255, 176, 32, ${pulse * 0.95})`;
    g.beginPath();
    g.arc(cx, cy, r, 0, Math.PI * 2);
    g.stroke();

    // Repères cardinaux fins style HUD
    g.strokeStyle = 'rgba(255, 176, 32, 0.45)';
    g.lineWidth = 1;
    g.beginPath();
    g.moveTo(cx - r - 8, cy); g.lineTo(cx - r + 4, cy);
    g.moveTo(cx + r - 4, cy); g.lineTo(cx + r + 8, cy);
    g.moveTo(cx, cy - r - 8); g.lineTo(cx, cy - r + 4);
    g.moveTo(cx, cy + r - 4); g.lineTo(cx, cy + r + 8);
    g.stroke();

    // Textes centraux ultra-légers
    g.textAlign = 'center';
    g.textBaseline = 'middle';

    g.font = '700 11px Consolas, "Courier New", monospace';
    g.letterSpacing = '1.2px';
    g.fillStyle = 'rgba(255, 196, 64, 0.98)';
    g.fillText('MODE ÉCO ACTIF', cx, cy - 14);

    g.font = '600 10px Consolas, "Courier New", monospace';
    g.letterSpacing = '0.5px';
    g.fillStyle = 'rgba(235, 248, 255, 0.9)';
    g.fillText(`CPU ${cpu || 0}% · RAM ${ram || 0}%`, cx, cy + 2);

    g.font = '9px Consolas, "Courier New", monospace';
    g.letterSpacing = '0.5px';
    g.fillStyle = 'rgba(255, 176, 32, 0.85)';
    g.fillText('CHARGE > 90%', cx, cy + 16);

    // Indication de reprise automatique
    g.font = '9.5px Consolas, "Courier New", monospace';
    g.letterSpacing = '0.5px';
    g.fillStyle = 'rgba(190, 240, 255, 0.7)';
    g.fillText('CŒUR EN PAUSE · REPRISE AUTO < 85%', cx, cy + r + 18);

    g.letterSpacing = '0px';
    g.restore();
  }
  J.drawSimpleEcoOrb = drawSimpleEcoOrb;

  // Fenêtre masquée ou réduite (mode widget, zone de notification) : on ne dessine plus rien.
  // La fenêtre garde backgroundThrottling: false pour que la voix continue, donc c'est ici que
  // le rendu s'arrête. La visibilité est poussée par main.js (jarvis:visibility).
  let windowVisible = true;
  let frameLoopRunning = true;
  if (window.jarvisBridge && window.jarvisBridge.onVisibility) {
    window.jarvisBridge.onVisibility((visible) => {
      windowVisible = visible;
      if (visible && !frameLoopRunning) {
        frameLoopRunning = true;
        last = performance.now();
        requestAnimationFrame(frame);
      }
    });
  }

  function frame(now) {
    try {
      if (!windowVisible) {
        frameLoopRunning = false;
        return;
      }
      if (realW !== window.innerWidth || realH !== window.innerHeight || dpr !== (window.devicePixelRatio || 1)) resize();

      // Rythme réel de l'écran (60, 100, 144 Hz…), estimé sur les images reçues : sert à plafonner
      // la cadence sans tomber sous la cible (à 100 Hz, viser 30 images/s donne 3 images sur 10).
      if (lastRafAt) {
        const step = now - lastRafAt;
        if (step > 2 && step < 50) rafStep += (step - rafStep) * 0.1;
      }
      lastRafAt = now;

      const v = J.voice;
      collectTaskStats();
      updateStopButton();
      const activeTasks = frameStats.active;
      const isVoiceActive = !!(v && (v.capturing || v.speaking || level > 0.04));
      const hasRecentInput = (now - lastUserInputAt) < 2000;
      const hasRecentIntervention = (now - lastDirectInterventionAt) < 3500;
      const isSystemBusy = !boot.isReady() || activeTasks.length > 0 || activityEnergy > 0.02 || isVoiceActive || hasRecentInput || hasRecentIntervention;

      // Surveillance permanente des ressources : mise à jour du mode éco (surcharge CPU/RAM)
      updateResourceEcoMode(telemetry.cpu, telemetry.ramPct);

      // Chantier B3 (Eco-VSync) : au repos complet sans sollicitation, bride à ~20 FPS (la respiration
      // du cœur est très lente, 30 FPS n'apportaient rien de visible) ; en mode Éco de surcharge
      // (CPU ou RAM >= 90%), bride à ~15 FPS pour soulager le système hôte.
      const isEcoThrottled = isResourceEcoMode;
      // Cadence maximale choisie dans Optimisation (0 = libre) : intervalle visé moins une demi-image d'écran.
      const capDelay = perf.maxFps > 0 ? 1000 / perf.maxFps - rafStep * 0.5 : 0;
      if (!isSystemBusy || isEcoThrottled || capDelay > 0) {
        const elapsedSinceRender = now - lastRenderTimestamp;
        const idleDelay = !isSystemBusy || isEcoThrottled ? (isEcoThrottled ? 64.0 : 46.0) : 0;
        if (elapsedSinceRender < Math.max(idleDelay, capDelay)) {
          return;
        }
      }
      lastRenderTimestamp = now;
      frameWorkStart = performance.now();

      const dt = Math.min(0.05, (now - last) / 1000);
      last = now;
      fps += (1 / Math.max(dt, 0.0001) - fps) * 0.08;
      const t = now / 1000;

      step(dt, now);
      boot.setLite(isResourceEcoMode || !perf.orbDetail);
      boot.update(now);

      g.clearRect(0, 0, W, H);
      const themeDef = (J.THEMES && J.THEMES[currentTheme]) || (J.THEMES && J.THEMES.cyan) || {};
      if (!J.wallpaperOn) { // Bureau JARVIS sur le fond d'écran de Windows (wallpaper.js) : canvas transparent
        g.fillStyle = themeDef.bgHex || '#00040a'; // fond teinté par le thème (ambre, émeraude…)
        g.fillRect(0, 0, W, H);
      }
      if (hudLayer && perf.hudBackground) {
        // Allumage : le fond se dévoile en cercle depuis le cœur, avec un liseré sur le front.
        const hudP = boot.ui('hud');
        if (hudP >= 1) {
          g.drawImage(hudLayer, 0, 0, W, H);
        } else if (hudP > 0) {
          const front = Math.hypot(W, H) / 2 * boot.ease.outCubic(hudP);
          g.save();
          g.beginPath();
          g.arc(W / 2, H / 2, front, 0, Math.PI * 2);
          g.save();
          g.clip();
          g.drawImage(hudLayer, 0, 0, W, H);
          g.restore();
          g.lineWidth = 1.5;
          g.strokeStyle = `rgba(${((J.THEMES && J.THEMES[currentTheme]) || J.THEMES.cyan).primaryRgb}, ${0.35 * (1 - hudP)})`;
          g.stroke();
          g.restore();
        }
      }

      // Rendu des traces de rémanence : mis en pause en mode éco pour soulager le processeur graphique
      // (et pendant l'allumage, elles arrivent avec les cards)
      const remanenceP = boot.ui('cards');
      if (!isResourceEcoMode && perf.hudBackground && remanenceP > 0) {
        g.save();
        g.globalAlpha = remanenceP;
        for (let i = remanenceTraces.length - 1; i >= 0; i--) {
          const trace = remanenceTraces[i];
          if (now - trace.depositedAt >= trace.duration || trace.gone) {
            remanenceTraces.splice(i, 1);
          } else {
            J.drawRemanence(g, trace, now);
          }
        }
        g.restore();
      }

      // Courant dans le circuit imprimé (circuit.js), sous les cards et le cœur : il sort du cœur quand Jarvis
      // réfléchit ou parle, y rentre quand il écoute, suit la musique, et s'arrête quand Jarvis s'assoupit.
      // Coupé avec le fond (Optimisation › fond), en mode éco et pendant l'allumage.
      if (J.circuit && hudLayer && perf.hudBackground) {
        const hudP = boot.ui('hud');
        const playing = !!(J.currentMedia && J.currentMedia.active && J.currentMedia.status === 'playing');
        const ml = J.musicLevel && now - J.musicLevel.at < 1500 ? J.musicLevel.v : 0.35;
        const mood = v && v.speaking ? 'speak' : isAiThinking ? 'think' : (v && v.capturing && level > 0.03) ? 'listen' : 'idle';
        J.circuit.update(dt, now, { enabled: !isResourceEcoMode && hudP >= 1, mood, level, music: playing ? Math.min(1, ml * 1.6) : 0, sleep: sleepLevel });
        J.circuit.draw(g, now, (J.heartLine && J.heartLine()) || themeDef.primaryRgb || '56, 225, 255', hudP);
      }

      const cx = W / 2;
      const cy = H / 2;
      const minDim = Math.min(W, H);

      // --- Dynamique d'incarnation : Cœur posé et majestueux, mouvements de taille lents ---
      // 1. Réactivité vocale sereine et posée
      let voiceIntensity = 0;
      if (v && v.capturing) {
        voiceIntensity = Math.max(voiceIntensity, 0.40);
      }
      if (v && v.speaking) {
        voiceIntensity = Math.max(voiceIntensity, 0.45);
      }
      if (level > 0.05) {
        voiceIntensity = Math.max(voiceIntensity, Math.min(0.35, (level - 0.05) * 1.0));
      }

      // 2. Charge des tâches et actions en cours
      const taskIntensity = Math.min(0.8, activeTasks.length * 0.25);

      // 3. Intervention directe récente de notre part (saisie, commande, clic)
      const timeSinceIntervention = now - lastDirectInterventionAt;
      const interventionIntensity = timeSinceIntervention < 7000
        ? Math.pow(Math.max(0, 1 - timeSinceIntervention / 7000), 1.8) * 0.35
        : 0;

      // 4. Synthèse globale d'interaction et de sollicitation avec lissage TRÈS DOUX et TRÈS LENT
      const rawActivity = Math.max(taskIntensity, voiceIntensity, interventionIntensity, activityEnergy);
      const isRising = rawActivity > smoothedLoad;
      const loadBlend = 1 - Math.exp(-(isRising ? 0.18 : 0.03) * dt);
      smoothedLoad += (rawActivity - smoothedLoad) * loadBlend;
      const loadFactor = Math.min(1.0, Math.max(0, smoothedLoad));

      // Échelle dimensionnelle du cœur (variations calmes et proportionnées) :
      // - Au repos : taille posée et majestueuse (~11% de minDim, ~110px sur 1080p, diamètre 220px)
      // - En pleine activité : grandit doucement jusqu'à ~21% de minDim (~210px sur 1080p, diamètre 420px)
      const MIN_IDLE_R_RATIO = 0.11;
      const MAX_ACTIVE_R_RATIO = 0.21;

      // Veille : une minute sans sollicitation (demande retenue, saisie, clic, tâche, Jarvis qui
      // parle, musique), le cœur rétrécit et ses traits s'effacent lentement (≈ 8 s) ; tout
      // revient en un tiers de seconde. Une voix dans la pièce non adressée à Jarvis ne le réveille pas.
      const musicOn = !!(J.currentMedia && J.currentMedia.active && J.currentMedia.status === 'playing');
      const solicited = !boot.isReady() || activeTasks.length > 0 || musicOn || hasRecentInput
        || !!(v && (v.speaking || v.wakeActive)) || inConversation() || (listeningMode !== 'wakeword' && !!(v && v.capturing));
      if (solicited) lastSolicitedAt = now;
      const sleepTarget = now - lastSolicitedAt > SLEEP_AFTER_MS ? 1 : 0;
      sleepLevel += (sleepTarget - sleepLevel) * Math.min(1, dt / (sleepTarget > sleepLevel ? 8 : 0.35));

      const targetBaseR = minDim * (MIN_IDLE_R_RATIO + (MAX_ACTIVE_R_RATIO - MIN_IDLE_R_RATIO) * loadFactor) * (1 - 0.3 * sleepLevel);

      if (!currentBaseR) currentBaseR = targetBaseR;
      currentBaseR = smoothTransitionOrb(currentBaseR, targetBaseR, dt);

      const mood = computeMood(now);

      // Détection des actions internes vs externes pour les anneaux Bleu et Vert
      let hasExternalTask = false;
      let hasLocalTask = false;
      for (const t of activeTasks) {
        if (isExternalTask(t)) hasExternalTask = true;
        else hasLocalTask = true; // active = pas terminée (endedAt vide)
      }
      const hasExternalAction = hasExternalTask || (typeof media !== 'undefined' && media && media.active) || false;
      const hasLocalAction = hasLocalTask || (activeTasks.length > 0 && !hasExternalAction);

      // Réflexion = au moins un appel IA externe en cours (événements ai_call).
      isAiThinking = aiCalls.size > 0;

      // « Jarvis » reconnu : pendant la phrase en cours et un court instant après (voir voice.js).
      if (v && v.wakeActive && !v.capturing && performance.now() > v.wakeReleaseAt) v.wakeActive = false;
      const wakeRecognized = !!(v && v.wakeActive);

      // Phase de conversation → couleur du cœur (rouge > bleu > vert > jaune, voir orb.js).
      const requestRunning = isAiThinking || activeTasks.some((t) => !t.background && t.status !== 'awaiting' && t.status !== 'waiting_reply');
      const userTalking = !!(v && v.capturing && level > 0.03);
      let voicePhase = null;
      if (v && v.speaking) voicePhase = 'reply';
      else if (wakeRecognized && (userTalking || !requestRunning)) voicePhase = 'wake';
      else if (userTalking) voicePhase = 'user';
      else if (requestRunning) voicePhase = 'think';

      // Détection précise si l'utilisateur parle ET si JARVIS le prend en compte
      const isVoiceAddressed = !!(
        v && !v.speaking && (
          (v.capturing && (level > 0.05 || (v.partial && v.partial.length > 0))) ||
          (listeningMode === 'ptt' && isPttActive && level > 0.05) ||
          (inConversation() && level > 0.09) ||
          (wakeRecognized && v.capturing && level > 0.05)
        )
      );

      const orbState = {
        heartOff: boot.isReady() && !voiceUp(), // moteur vocal pas encore activé : les personnages tournent le dos
        dt,
        mood,
        loadFactor,
        levels,
        head: levelHead,
        capturing: !!(v && v.capturing),
        speaking: !!(v && v.speaking),
        userSpeaking: isVoiceAddressed || userTalking,
        wakeRecognized,
        voicePhase,
        userVoiceLevel: level,
        isThinking: isAiThinking,
        hasLocalAction,
        hasExternalAction,
        lite: !perf.orbDetail,
        noWaveform: !perf.voiceWaveform,
        sleep: sleepLevel,
        heartStyle,
        musicPlaying: musicOn,
        companion: companionState
      };

      const R = J.computeOrbRadius ? J.computeOrbRadius(currentBaseR, t, level, orbState) : currentBaseR;
      orbState.reveal = boot.orbReveal(now);

      // Rendu des cards d'action permanentes et de leurs liaisons vers le cœur et vers les widgets.
      // Allumage : elles sortent du cœur (échelle 0,6 → 1 centrée sur lui).
      const cardsP = perf.cards ? boot.ui('cards') : 0;
      // Présentation d'un dossier en cours (presenter.js) : tout le reste passe à 10 % pour la mettre en avant.
      const presentDim = J.presentDim ? J.presentDim() : 1;
      if (cardsP > 0) {
        settleCards(dt, now, cx, cy, R);
        g.save();
        if (cardsP < 1) {
          const s = 0.6 + 0.4 * boot.ease.outBack(cardsP);
          g.globalAlpha = boot.ease.outCubic(cardsP);
          g.translate(cx, cy);
          g.scale(s, s);
          g.translate(-cx, -cy);
        }
        g.globalAlpha *= presentDim;
        if (J.drawAllActionCards) {
          J.networkPanels = networkPanels(); // panneaux reliés au réseau du cœur (branches.js)
          const widgetBoxes = J.getWidgetBoxes ? J.getWidgetBoxes() : {};
          J.drawAllActionCards(g, cx, cy, R, t, now, widgetBoxes, W, H);
          reportDeskCards(now, widgetBoxes, S);
        }
        for (const task of tasks.values()) {
          try {
            // Évite le dédoublement visuel : si la tâche est déjà prise en charge par une Action Card permanente
            if (J.getActionCards && J.getActionCards().some((c) => c.activeTask && c.activeTask.id === task.id)) {
              continue;
            }
            const geo = J.branchGeometry(cx, cy, R, task.slot, W, H);
            task.lastGeo = geo;
            g.save();
            J.drawBranch(g, task, geo, t, task.fadeAlpha);
            g.restore();
          } catch (branchErr) {
            console.error('Erreur drawBranch ignorée:', branchErr);
          }
        }
        g.restore();
      }

      levels[levelHead] = level;
      levelHead = (levelHead + 1) % levels.length;

      // Cœur (ou affichage épuré si charge CPU/RAM > 90 %) ; pendant l'allumage, il grandit depuis
      // un point et ses couches suivent les étapes validées (orbState.reveal).
      const rv = orbState.reveal;
      const heartR = rv ? R * rv.heartScale : R;
      heartDropR = heartR * S;
      reportDeskHeart(!rv || rv.heartScale >= 0.98 ? heartR * S : 0, now); // cœur fini de grandir (allumage terminé)
      lastHeartR = heartR;
      // Petit cœur rangé dans le HUD : le grand cœur s'efface, le petit prend sa place (même rendu, même position de départ).
      const miniPos = miniNow(now, { x: cx, y: cy, r: heartR });
      const arrived = Boolean(miniPos && mini.toHud && miniPos.p >= 1); // revenu au centre : le grand cœur reprend
      const bigHidden = heartOnWidget || (Boolean(mini) && !arrived);
      heartAlpha += ((bigHidden ? 0 : 1) - heartAlpha) * Math.min(1, dt / 0.045);
      if (Math.abs(heartAlpha - (bigHidden ? 0 : 1)) < 0.02) heartAlpha = bigHidden ? 0 : 1;
      if (miniPos && heartR > 1) {
        g.save();
        if (boot.isReady() && !voiceUp()) g.globalAlpha *= 0.4;
        J.drawOrb(g, miniPos.x, miniPos.y, miniPos.r, t, level, orbState);
        g.restore();
      }
      if (arrived && heartAlpha >= 0.95) mini = null;
      const heartOff = boot.isReady() && !voiceUp();
      if (heartR > 1 && heartAlpha > 0) {
        g.save();
        if (heartAlpha < 1) g.globalAlpha *= heartAlpha;
        if (isResourceEcoMode) {
          if (rv) g.globalAlpha *= rv.heart;
          drawSimpleEcoOrb(g, cx, cy, heartR, t, telemetry.cpu, telemetry.ramPct);
        } else {
          // Moteur vocal non lancé : cœur éteint (estompé) et barré d'une croix rouge.
          if (heartOff) g.globalAlpha *= 0.4;
          J.drawOrb(g, cx, cy, heartR, t, level, orbState);
        }
        g.restore();
        if (heartOff) drawHeartOffCross(g, cx, cy, heartR, t, heartAlpha);
      }
      boot.drawOverlay(g, cx, cy, R, now, currentTheme);
      // Card Conversation : module Conversation (modules/conversation/) ; désactivé, elle n'est plus dessinée.
      const conversationOn = !J.moduleActive || J.moduleActive('conversation');
      g.save();
      g.globalAlpha *= presentDim;
      if (boot.isReady() && conversationOn) transcript(cx, cy, R, now);

      if (conversationOn) drawTranscript();
      revealPanel('memory', () => memoryBox, memoryPanel);
      revealFade('panel:status', statusPanel);
      revealPanel('journal', () => journalBox, journalPanel);
      header(now);
      revealFade('hardware', hardwareInfo);
      if (boot.isReady()) helpPanel();
      g.restore();
    } catch (frameErr) {
      console.error('Erreur globale rendu frame:', frameErr);
    } finally {
      if (frameWorkStart) {
        frameMs += (performance.now() - frameWorkStart - frameMs) * 0.05;
        frameWorkStart = 0;
      }
      if (frameLoopRunning) requestAnimationFrame(frame);
    }
  }

  // --- Saisie : commande libre ou code de validation ---------------------------

  function reject() {
    lastErrorAt = performance.now();
    activityEnergy = Math.min(1, activityEnergy + 0.25);
    consoleBar.classList.add('reject');
    setTimeout(() => consoleBar.classList.remove('reject'), 400);
  }

  function speak(text, meta) {
    // La voix met les phrases en attente tant qu'elle n'est pas prête (voir voice.js).
    if (J.voice) J.voice.say(text, meta);
  }

  // Parole locale de l'interface (hors orchestrateur) : écrite et prononcée.
  function speakLocal(text) {
    pushJarvis({ text });
    rememberSpeech(text);
    speak(text);
  }

  // Politique provisoire : l'étape 3 déplacera cette décision côté orchestrateur,
  // où elle portera sur les outils réellement appelés par Claude Code.
  const WAKE_PREFIX = /^\s*(?:dis\s+jarvis|hey\s+jarvis|ok\s+jarvis|bonjour\s+jarvis|salut\s+jarvis|jarvis|jervis|jarvice)\b[\s,.!:]*/i;
  const CONTAINS_WAKE = /\b(?:dis\s+jarvis|hey\s+jarvis|ok\s+jarvis|bonjour\s+jarvis|salut\s+jarvis|jarvis|jervis|jarvice)\b/i;

  // v6 (P8) : variantes du mot d'éveil mal transcrites (« Jarvi », « Charvis », « J'arvis »,
  // « Jarvisse »…), reconnues uniquement en tête de phrase, par distance d'édition ≤ 1 avec
  // « jarvis » ou liste connue. Au milieu d'une phrase, seul « Jarvis » exact compte.
  const WAKE_VARIANTS = new Set(['jarvi', 'jarviss', 'jarvisse', 'jarvise', 'charvis', 'jarvice', 'jervis', 'garvis', 'djarvis', 'jarviz', 'jarvys']);
  function editDistance(a, b) {
    const row = Array.from({ length: b.length + 1 }, (_, i) => i);
    for (let i = 1; i <= a.length; i++) {
      let diag = row[0];
      row[0] = i;
      for (let j = 1; j <= b.length; j++) {
        const tmp = row[j];
        row[j] = Math.min(row[j] + 1, row[j - 1] + 1, diag + (a[i - 1] === b[j - 1] ? 0 : 1));
        diag = tmp;
      }
    }
    return row[b.length];
  }
  function normalizeWake(text) {
    const m = String(text || '').match(/^\s*((?:(?:dis|hey|ok|bonjour|salut)\s+)?)([a-zà-ÿ'’]+)(.*)$/i);
    if (!m) return text;
    const word = m[2].toLowerCase().replace(/['’]/g, '');
    if (m[2].toLowerCase() === 'jarvis') return text;
    const close = word === 'jarvis' || WAKE_VARIANTS.has(word) || (word.length >= 5 && word.length <= 8 && editDistance(word, 'jarvis') <= 1);
    return close ? `${m[1]}Jarvis${m[3]}` : text;
  }

  // --- Fenêtre de conversation (mode mot d'éveil) ------------------------------
  // Après « Jarvis », on peut enchaîner sans répéter son nom : la fenêtre reste ouverte pendant
  // que Jarvis réfléchit ou parle, puis CONVERSATION_MS après sa réponse. « Merci, c'est tout »
  // la referme.
  const CONVERSATION_MS = 12000;
  const END_CONVERSATION = /^(?:merci[\s,.!]*)?(?:c['’]est tout|ce sera tout|ça sera tout|ca sera tout|fin de (?:la )?conversation|au revoir|à plus tard|a plus tard|bonne nuit)\b/i;
  let conversationUntil = 0;
  let conversationClosedAt = -1e9;

  function openConversation(ms = CONVERSATION_MS) {
    if (listeningMode !== 'wakeword') return;
    conversationUntil = Math.max(conversationUntil, performance.now() + ms);
  }

  function inConversation() {
    if (listeningMode !== 'wakeword' || performance.now() - conversationClosedAt < 3000) return false;
    if (performance.now() < conversationUntil) return true;
    if (J.voice && J.voice.speaking) return performance.now() - conversationClosedAt > 8000;
    // Une demande est en cours de traitement : la réponse va arriver, la fenêtre reste ouverte.
    return [...tasks.values()].some((t) => !t.endedAt && !t.background && /^c\d+$/.test(t.id));
  }

  function closeConversation() {
    conversationUntil = 0;
    conversationClosedAt = performance.now();
  }

  // Fin de la parole de Jarvis : on laisse le temps de répondre sans dire « Jarvis ».
  J.onSpeakingChange = (speaking) => {
    if (!speaking && performance.now() - conversationClosedAt > 8000) openConversation();
  };

  // --- Filtre d'écho : avec haut-parleurs, le micro entend Jarvis ----------------
  const recentSpeech = []; // { words, at }
  const ECHO_WINDOW = 15000;

  function speechWords(text) {
    return String(text || '')
      .toLowerCase()
      .normalize('NFD')
      .replace(/[̀-ͯ]/g, '')
      .replace(/[^a-z0-9\s]/g, ' ')
      .split(/\s+/)
      .filter((w) => w.length > 1);
  }

  function rememberSpeech(text) {
    recentSpeech.push({ words: new Set(speechWords(text)), at: performance.now() });
    while (recentSpeech.length > 12) recentSpeech.shift();
  }

  // Une phrase « entendue » dont presque tous les mots viennent de ce que Jarvis vient de dire.
  function isEcho(text) {
    const words = speechWords(text);
    if (words.length < 2) return false;
    const now = performance.now();
    const spoken = new Set();
    for (const s of recentSpeech) if (now - s.at < ECHO_WINDOW) s.words.forEach((w) => spoken.add(w));
    if (!spoken.size) return false;
    return words.filter((w) => spoken.has(w)).length / words.length >= 0.7;
  }

  function runCommand(text, isFromVoice = false, extraMeta = {}) {
    let cleaned = isFromVoice ? normalizeWake(text.trim()) : text.trim();
    lastDirectInterventionAt = performance.now();
    activityEnergy = Math.min(1, activityEnergy + 0.35);
    const hasWake = CONTAINS_WAKE.test(cleaned);

    // En mode mot d'éveil : l'entrée vocale n'agit que si « Jarvis » est prononcé, SAUF pendant
    // une conversation ouverte, ou si Jarvis attend une confirmation ou la réponse à sa question.
    const isAwaitingOrWaitingReply = [...tasks.values()].some(
      (t) => t.status === 'awaiting' || t.status === 'waiting_reply'
    );
    const conversing = inConversation();
    if (isFromVoice && listeningMode === 'wakeword' && !hasWake && !isAwaitingOrWaitingReply && !conversing) {
      // Transcription passive affichée, mais aucune action déclenchée
      return;
    }
    if (isFromVoice && listeningMode === 'wakeword') {
      if (!hasWake && END_CONVERSATION.test(cleaned)) {
        closeConversation();
        speakLocal('Très bien.');
        return;
      }
      openConversation();
    }

    // En mode Push-To-Talk : n'agit que si la touche est ou vient d'être relâchée
    if (isFromVoice && listeningMode === 'ptt' && !isPttActive && (performance.now() - lastPttAt > 1500)) {
      return;
    }

    // Nettoyage du mot d'éveil pour envoyer la requête propre
    cleaned = cleaned.replace(WAKE_PREFIX, '').trim();
    if (!cleaned) {
      if (hasWake) {
        cleaned = "Es-tu là ?";
      } else {
        return;
      }
    }

    // Détection d'intention pour le morphing du cœur. L'état « IA sollicitée » (cœur orange)
    // n'est allumé que par l'orchestrateur, au lancement réel d'un appel externe (v5).
    // Demande tapée : forme précise d'abord (heure, animal…, orb-morph.js) ; les phrases dites
    // passent par l'événement voice_final, que orb-morph.js écoute lui-même.
    J.markSolicited();
    if (!isFromVoice && J.morphForText) J.morphForText(cleaned);
    triggerIntentMorph(cleaned);

    // En fonctionnement normal, c'est l'orchestrateur qui crée et pilote la branche.
    if (window.jarvisBridge) {
      // Une nouvelle demande remplace ce que Jarvis était en train de dire.
      if (J.voice) J.voice.stop();
      // L'origine permet à l'orchestrateur de refuser un code de validation dicté à la voix.
      window.jarvisBridge.utterance(cleaned, { source: isFromVoice ? 'voice' : 'typed', ...(extraMeta || {}) });
      return;
    }

    // Repli hors Electron (test navigateur) : simulation locale.
    if (SENSITIVE.test(cleaned)) {
      const code = String(1000 + Math.floor(Math.random() * 9000));
      const id = api.create({ label: 'Action sensible', detail: cleaned, status: 'awaiting', code });
      if (id) speakLocal("Action sensible détectée. Saisis le code affiché à l'écran pour valider.");
      return;
    }

    const id = api.create({ label: 'Demande', detail: cleaned });
    if (!id) return;
    setTimeout(() => api.finish(id, 'done'), 3000 + Math.random() * 2500);
  }

  // Fil de la conversation affiché dans la zone de transcription : ce que l'utilisateur a dit
  // (who: 'user') et ce que Jarvis a répondu (who: 'jarvis').
  const heard = [];

  function pushHeard(entry) {
    lastDirectInterventionAt = performance.now();
    heard.push({ who: 'user', at: new Date(), ...entry });
    if (heard.length > 200) heard.shift();
  }

  // --- Tout arrêter (bouton STOP, Échap) ---------------------------------------------------------
  // Tâches IA, validations et questions en attente, parole de Jarvis, relais tmux, dictée
  // (orchestrateur › stopAll). La musique continue. Hors coquille : les tâches de démo s'arrêtent.
  const stopBtn = document.getElementById('stop-btn');
  let stopShown = false;
  function stopEverything(origin) {
    if (window.jarvisBridge && window.jarvisBridge.stopAll) {
      window.jarvisBridge.stopAll(origin).catch(() => {});
    } else {
      for (const t of tasks.values()) if (!t.endedAt) api.finish(t.id, 'error');
    }
  }
  if (stopBtn) stopBtn.addEventListener('click', () => stopEverything('bouton'));

  // Bouton visible seulement quand il y a quelque chose à arrêter.
  function updateStopButton() {
    if (!stopBtn) return;
    const v = J.voice;
    const want = boot.isReady() && (frameStats.active.length > 0 || frameStats.awaiting > 0 || frameStats.waitingReply > 0 || !!(v && v.speaking));
    if (want === stopShown) return;
    stopShown = want;
    stopBtn.classList.toggle('visible', want);
  }

  // Réponse de Jarvis : les phrases diffusées au fil de l'eau d'une même tâche forment une
  // seule entrée, complétée à mesure qu'elles arrivent.
  function pushJarvis(ev) {
    if (ev.ack) return; // « Je regarde. » n'est pas une réponse
    activityEnergy = Math.min(1, activityEnergy + 0.45);
    const text = ev.muted ? `[sourdine] ${ev.text}` : ev.text;
    const last = heard[heard.length - 1];
    if (ev.stream && ev.task && last && last.who === 'jarvis' && last.task === ev.task) {
      last.text = `${last.text} ${text}`;
      return;
    }
    pushHeard({ who: 'jarvis', text, task: ev.task || null, muted: !!ev.muted });
  }

  // Routage spéculatif sur transcription partielle : pré-analyse dès la parole
  J.onPartialUtterance = (text, meta = {}) => {
    // v2 : le routage spéculatif est fait par le noyau (core/voice-link.js).
    if (window.jarvisBridge && window.jarvisBridge.coreVoice) return;
    const raw = String(text || '').trim();
    if (!raw || raw.length < 4 || isEcho(raw)) return;
    const cleaned = normalizeWake(raw).replace(WAKE_PREFIX, '').trim();
    if (cleaned.length >= 4 && window.jarvisBridge && window.jarvisBridge.speculativeUtterance) {
      window.jarvisBridge.speculativeUtterance(cleaned, { source: 'voice', ...(meta || {}) });
    }
  };

  J.onUtterance = (text, meta = {}) => {
    // Jarvis s'entend lui-même dans le micro : on ne le prend pas pour une demande.
    if (isEcho(text)) return;
    lastDirectInterventionAt = performance.now();
    activityEnergy = Math.min(1, activityEnergy + 0.40);
    const spkName = meta && meta.speaker && meta.speaker.name;
    pushHeard({ who: spkName || 'user', text, speaker: meta && meta.speaker, state: meta && meta.who ? meta.who.state : null, ask: (meta && meta.ask) || null });
    // v2 : le noyau a déjà appliqué mot d'éveil, conversation et écho, et transmis la demande ;
    // la fenêtre ne fait qu'afficher (et animer le cœur si la phrase a été retenue).
    if (window.jarvisBridge && window.jarvisBridge.coreVoice) {
      if (meta.accepted) {
        J.markSolicited();
        triggerIntentMorph(String(text).replace(WAKE_PREFIX, ''));
      }
      return;
    }
    runCommand(text, true, meta);
  };

  let lastSpeakerRejectAt = 0;
  J.onSpeakerRejected = (ev) => {
    if (window.jarvisBridge && window.jarvisBridge.speakerRejected) {
      window.jarvisBridge.speakerRejected(ev);
    }
    const now = performance.now();
    if (now - lastSpeakerRejectAt > 3000) {
      lastSpeakerRejectAt = now;
      pushHeard({ who: 'system', text: `[Voix externe ignorée · ${Math.round((ev.similarity || 0) * 100)}% de similarité]` });
    }
  };

  J.onSpeakerEnrollProgress = (ev) => {
    if (window.jarvisBridge && window.jarvisBridge.speakerProgress) {
      window.jarvisBridge.speakerProgress(ev);
    }
    if (ev.message) {
      pushHeard({ who: 'jarvis', text: ev.message });
      // Pas de synthèse vocale locale pendant les étapes intermédiaires :
      // le micro doit rester silencieux pour ne capter que la voix de l'utilisateur.
    }
  };

  J.onSpeakerEnrolled = (ev) => {
    const status = ev ? (ev.status || ev) : {};
    const spk = ev ? (ev.speaker || status.speaker) : null;
    if (window.jarvisBridge && window.jarvisBridge.speakerEnrolled) {
      window.jarvisBridge.speakerEnrolled(status);
    }
    const name = (spk && spk.name) ? spk.name : "votre voix";
    const msg = `Empreinte vocale de ${name} enregistrée avec succès !`;
    pushHeard({ who: 'jarvis', text: msg });
    speakLocal(msg);
  };

  J.onSpeakerReset = (ev) => {
    pushHeard({ who: 'jarvis', text: "Empreintes vocales réinitialisées." });
    speakLocal("Empreintes vocales réinitialisées.");
  };

  input.addEventListener('input', () => {
    lastUserInputAt = performance.now();
    lastDirectInterventionAt = performance.now();
  });

  input.addEventListener('keydown', (e) => {
    lastUserInputAt = performance.now();
    lastDirectInterventionAt = performance.now();
    // Échap : efface la saisie en cours, sinon arrête tout (quitter : Ctrl+Q).
    if (e.key === 'Escape') {
      e.preventDefault();
      e.stopPropagation(); // le gestionnaire de la fenêtre ne doit pas arrêter une seconde fois
      input.placeholder = 'commande ou code de validation';
      if (input.value) input.value = '';
      else stopEverything('echap');
      return;
    }
    if (e.key !== 'Enter') return;
    const value = input.value.trim();
    if (!value) return;

    const awaiting = [...tasks.values()].filter((t) => t.status === 'awaiting');

    // Refus explicite (oral ou écrit)
    if (awaiting.length && /^(?:non|refuse|annule|stop|pas question)$/i.test(value)) {
      awaiting.forEach((t) => {
        if (window.jarvisBridge) window.jarvisBridge.deny(t.id);
        else api.update(t.id, { status: 'error', code: null, detail: 'refusé' });
      });
      input.value = '';
      return;
    }

    // Accord oral ou saisie simple (« oui », « vas-y », etc.)
    if (awaiting.length && /^(?:oui|vas[\s-]y|d['’]accord|valide|confirme|ok)$/i.test(value)) {
      const voicePending = awaiting.find((t) => t.validationType === 'voice' || !t.code);
      if (voicePending) {
        if (window.jarvisBridge) window.jarvisBridge.approve(voicePending.id);
        else api.update(voicePending.id, { status: 'done', code: null, detail: 'validé par accord oral' });
        input.value = '';
        return;
      }
      // Si toutes les tâches sont 'code' (Critique), refuser la validation orale
      reject();
      speakLocal("Cette action est critique : saisis le code à 4 chiffres.");
      input.value = '';
      return;
    }

    // Validation par code écrit (Critique)
    const pending = awaiting.find((t) => t.code === value);
    if (pending) {
      if (window.jarvisBridge) window.jarvisBridge.approve(pending.id);
      else api.update(pending.id, { status: 'done', code: null, detail: 'validé par code' });
      input.value = '';
      return;
    }
    if (awaiting.length && /^\d{3,6}$/.test(value)) {
      reject();
      input.value = '';
      return;
    }

    // Affichage immédiat du texte saisi dans l'interface (comme pour la voix)
    pushHeard({ text: value });
    if (J.voice) {
      J.voice.lastFinal = value;
      J.voice.finalAt = performance.now();
    }

    runCommand(value);
    input.value = '';
  });

  window.addEventListener('keydown', (e) => {
    if (/^F(?:[1-9]|10)$/.test(e.key)) e.preventDefault();
    if (e.key === 'F10') openAdmin();
    if (e.key === 'F1') {
      e.preventDefault();
      if (e.shiftKey) showHelp = !showHelp; // Maj+F1 : aide-mémoire des touches dans le HUD
      else openHelp();
    }
    if (e.key === 'F2') debug = !debug;
    if (e.key === 'F5' && e.shiftKey) {
      toggleLearning();
    } else if (e.key === 'F5') {
      showTranscript = !showTranscript;
      localStorage.setItem('jarvis.transcript', showTranscript ? '1' : '0');
    }
    if (e.key === 'F6') {
      showMemory = !showMemory;
      localStorage.setItem('jarvis.memory', showMemory ? '1' : '0');
    }
    if (e.key === 'F7') {
      showStatus = !showStatus;
      localStorage.setItem('jarvis.status', showStatus ? '1' : '0');
    }
    if (e.key === 'F8') {
      const cycle = { wakeword: 'ptt', ptt: 'free', free: 'wakeword' };
      const next = cycle[listeningMode] || 'wakeword';
      runCommand(`/mode ${next}`);
    }
    if (e.key === 'F9') {
      showJournal = !showJournal;
      localStorage.setItem('jarvis.journal', showJournal ? '1' : '0');
    }
    if ((e.key === 'v' || e.key === 'V') && showJournal && document.activeElement !== input) {
      runCommand('/verifier');
    }
    if (e.key === 'F3' && J.voice) J.voice.toggleMute();
    if (e.key === 'F4') {
      if (e.shiftKey) {
        const densCycle = { compact: 'normal', normal: 'large', large: 'compact' };
        const nextDens = densCycle[currentDensity] || 'normal';
        runCommand(`/densite ${nextDens}`);
      } else {
        const themeCycle = { cyan: 'gold', gold: 'matrix', matrix: 'crimson', crimson: 'violet', violet: 'cyan' };
        const nextTheme = themeCycle[currentTheme] || 'cyan';
        runCommand(`/theme ${nextTheme}`);
      }
    }
    if (e.key === 'PageUp') {
      if (showJournal) {
        journalScrollOffset = Math.min(Math.max(0, (journalEntries.length || 0) - 3), journalScrollOffset + 4);
      } else if (showTranscript) {
        transcriptScrollOffset = Math.min(Math.max(0, transcriptRowCount - 4), transcriptScrollOffset + 4);
      }
    }
    if (e.key === 'PageDown') {
      if (showJournal) {
        journalScrollOffset = Math.max(0, journalScrollOffset - 4);
      } else if (showTranscript) {
        transcriptScrollOffset = Math.max(0, transcriptScrollOffset - 4);
      }
    }
    if ((e.key === 'Home' || e.key === 'End') && document.activeElement !== input) {
      if (e.key === 'Home') {
        if (showJournal) journalScrollOffset = Math.max(0, (journalEntries.length || 0) - 4);
        else if (showTranscript) transcriptScrollOffset = Math.max(0, transcriptRowCount - 4);
      } else if (e.key === 'End') {
        journalScrollOffset = 0;
        transcriptScrollOffset = 0;
      }
    }
    if (e.key === ' ' && listeningMode === 'ptt' && document.activeElement !== input && !e.repeat) {
      e.preventDefault();
      isPttActive = true;
      lastPttAt = performance.now();
      // v2 : c'est le noyau qui décide quelles phrases entendues sont des demandes.
      if (window.jarvisBridge && window.jarvisBridge.ptt) window.jarvisBridge.ptt(true);
    }
    // Échap arrête tout (sauf quand le menu des modes est ouvert : il se ferme seulement).
    if (e.key === 'Escape') {
      const menu = document.getElementById('mode-dropdown');
      if (!menu || menu.classList.contains('hidden')) {
        showHelp = false;
        stopEverything('echap');
      }
    }
    if ((e.key === 'q' || e.key === 'Q') && e.ctrlKey && !e.shiftKey && !e.altKey && window.jarvisBridge) {
      e.preventDefault();
      window.jarvisBridge.quit();
    }
  });

  // --- Placement sans chevauchement (v3) --------------------------------------------------------
  // Aucune card ne reste sous une autre : quand deux éléments se superposent, le MOINS VISIBLE glisse
  // doucement vers la place libre la plus proche (le vertical d'abord, sans sortir de l'écran).
  // Ordre de visibilité, du plus fort au plus faible :
  //   1. obstacles fixes : panneaux (conversation, mémoire, journal, aide), zone de saisie, widgets
  //      HTML, cœur, en-tête, et tout élément tenu à la souris ;
  //   2. cards actives et encarts de tâche (à égalité, le plus ancien garde sa place) ;
  //   3. cards d'action au repos (fantômes), selon leur transparence ;
  //   4. traces de rémanence, les plus pâles en dernier.
  // Chaque élément revient vers sa position d'origine dès qu'elle se libère. Une trace qui ne trouve
  // aucune place s'efface.
  const CARD_GAP = 10;
  const SETTLE_SPEED = 3.2; // constante de glissement (~0,3 s) : mouvement doux, pas de saut
  const LAYOUT_EVERY_MS = 180; // recherche des places (le glissement, lui, suit chaque image)
  const EDGE_X = 14;
  const EDGE_Y = 60;
  let lastLayoutAt = 0;

  // Déplacements candidats, du plus court au plus long (un pas horizontal coûte plus qu'un vertical).
  const LAYOUT_OFFSETS = (() => {
    const list = [];
    for (let dy = -480; dy <= 480; dy += 12) {
      for (let dx = -360; dx <= 360; dx += 24) {
        if (dx || dy) list.push({ dx, dy, cost: Math.abs(dy) + Math.abs(dx) * 1.6 });
      }
    }
    return list.sort((a, b) => a.cost - b.cost);
  })();

  function overlaps(a, b) {
    return a.x < b.x + b.w + CARD_GAP && b.x < a.x + a.w + CARD_GAP && a.y < b.y + b.h + CARD_GAP && b.y < a.y + a.h + CARD_GAP;
  }

  function fixedObstacles(cx, cy, R) {
    const obs = [];
    const add = (b) => {
      if (!b) return;
      const w = b.w !== undefined ? b.w : b.width;
      const h = b.h !== undefined ? b.h : b.height;
      if (w > 0 && h > 0) obs.push({ x: b.x, y: b.y, w, h });
    };
    if (showTranscript) add(transcriptBox);
    if (showMemory) add(memoryBox);
    if (showJournal) add(journalBox);
    if (showHelp) add(helpBox);
    const c = getConsoleRect();
    add({ x: c.left, y: c.top, w: c.width, h: c.height });
    // Cœur et ses anneaux
    const r = R * 1.2;
    add({ x: cx - r, y: cy - r, w: r * 2, h: r * 2 });
    // En-tête : titre et état à gauche ; horloge, CONFIG, AIDE et boutons de fenêtre à droite
    add({ x: 20, y: 24, w: 400, h: 70 });
    add({ x: W - 440, y: 0, w: 440, h: (helpBtnCoords ? helpBtnCoords.y + helpBtnCoords.h : 90) + 4 + DOCK_BTNS_H });
    return obs;
  }

  // Widgets HTML (widgets.js) : un widget actif ou tenu à la souris est un obstacle fixe ; en
  // fondu ou fantôme (transparence 0,10), il cède la place comme une card, par un décalage CSS
  // (`translate`) qui ne touche pas à sa position enregistrée.
  const WIDGET_RANK = { fading: 1.5, ghost: 1.099 };
  let widgetBoxesReadAt = 0;
  function widgetItems(items) {
    const recs = J.widgets && J.widgets.cards;
    if (!recs || !J.getWidgetBox) return;
    // Un widget ouvert qui change de taille est déjà relu (ResizeObserver de widgets.js) ; les autres sont
    // relus toutes les 2 s au lieu de 180 ms : beaucoup moins de lectures de mise en page.
    const nowMs = performance.now();
    if (nowMs - widgetBoxesReadAt > 2000) {
      widgetBoxesReadAt = nowMs;
      if (J.invalidateWidgetBoxes) J.invalidateWidgetBoxes();
    }
    for (const [key, rec] of recs) {
      const b = rec.el && rec.el.isConnected ? J.getWidgetBox(key) : null;
      if (!b || b.w <= 0 || b.h <= 0) continue;
      const fixed = !(rec.state in WIDGET_RANK) || rec.el.classList.contains('is-dragging');
      const box = { x: b.x, y: b.y, w: b.w, h: b.h };
      items.push({
        ref: rec, kind: 'widget', box, home: { ...box, x: box.x - (rec.layoutX || 0), y: box.y - (rec.layoutY || 0) },
        rank: fixed ? 4 : WIDGET_RANK[rec.state], since: 0, fixed
      });
    }
  }

  function layoutItems(now) {
    const items = [];
    const mapped = new Set();
    if (J.getActionCards) {
      for (const card of J.getActionCards()) {
        if (card.activeTask) mapped.add(card.activeTask.id);
        const vis = card.visibility === undefined ? 1 : card.visibility;
        if (!card.panelBox || vis < 0.05) continue;
        const active = card.state === 'active' || card.state === 'activating';
        const alpha = active ? 1 : (card.boxAlpha !== undefined ? card.boxAlpha : 0.14);
        items.push({
          ref: card, kind: 'card', box: card.panelBox, home: card.homeBox || card.panelBox,
          rank: (active ? 2 : 1) + alpha * vis * 0.99, since: card.activatedAt || 0, fixed: card.isDragging
        });
      }
    }
    for (const task of tasks.values()) {
      if (mapped.has(task.id) || !task.panelBox || task.grow < 0.5) continue;
      if (task.endedAt && now - task.endedAt > 1500) continue; // en train de s'effacer : ignorée
      const b = task.panelBox;
      items.push({
        ref: task, kind: 'task', box: b, home: { x: b.x - (task.layoutX || 0), y: b.y - (task.layoutY || 0), w: b.w, h: b.h },
        rank: 2.99, since: task.createdAt || 0, fixed: task.isDragging
      });
    }
    widgetItems(items);
    for (const trace of remanenceTraces) {
      if (trace.gone || !trace.box) continue;
      if (!trace.homeBox) trace.homeBox = { ...trace.box };
      const alpha = Math.max(0, 1 - (now - trace.depositedAt) / trace.duration);
      items.push({ ref: trace, kind: 'trace', box: trace.box, home: trace.homeBox, rank: alpha * 0.9, since: trace.depositedAt, fixed: false });
    }
    // Du plus visible au moins visible ; à égalité, le plus ancien garde sa place.
    return items.sort((a, b) => (b.rank - a.rank) || (a.since - b.since));
  }

  // Place de chaque élément : sa position d'origine si elle est libre, sinon sa place actuelle si
  // elle l'est encore (pas de va-et-vient), sinon la place libre la plus proche.
  function solveLayout(now, cx, cy, R) {
    const placed = fixedObstacles(cx, cy, R);
    const items = layoutItems(now);
    for (const it of items) if (it.fixed) placed.push(it.box);
    const inside = (b) => b.x >= EDGE_X && b.x + b.w <= W - EDGE_X && b.y >= EDGE_Y && b.y + b.h <= H - EDGE_Y;
    const free = (b) => inside(b) && !placed.some((o) => overlaps(b, o));
    // Surface recouverte (écran saturé : on choisit la place où l'élément gêne le moins).
    const covered = (b) => {
      let sum = 0;
      for (const o of placed) {
        const w = Math.min(b.x + b.w, o.x + o.w) - Math.max(b.x, o.x);
        const h = Math.min(b.y + b.h, o.y + o.h) - Math.max(b.y, o.y);
        if (w > 0 && h > 0) sum += w * h;
      }
      return sum;
    };

    for (const it of items) {
      if (it.fixed) continue;
      const ref = it.ref;
      const home = it.home;
      const at = (dx, dy) => ({ x: home.x + dx, y: home.y + dy, w: it.box.w, h: it.box.h });
      let target = null;
      if (free(at(0, 0))) target = { dx: 0, dy: 0 };
      else if ((ref.layoutTX || ref.layoutTY) && free(at(ref.layoutTX, ref.layoutTY))) target = { dx: ref.layoutTX, dy: ref.layoutTY };
      else {
        for (const o of LAYOUT_OFFSETS) {
          if (free(at(o.dx, o.dy))) {
            target = o;
            break;
          }
        }
      }
      // Aucune place libre : une trace s'efface ; une card ou un widget prend la place la moins recouverte.
      ref.layoutBlocked = !target;
      if (!target && it.kind !== 'trace') {
        let best = Infinity;
        for (const o of [{ dx: 0, dy: 0, cost: 0 }, ...LAYOUT_OFFSETS]) {
          const b = at(o.dx, o.dy);
          if (!inside(b)) continue;
          const c = covered(b) + o.cost; // à recouvrement égal, la plus proche
          if (c < best) {
            best = c;
            target = o;
          }
        }
      }
      if (target) {
        ref.layoutTX = target.dx;
        ref.layoutTY = target.dy;
        placed.push(at(target.dx, target.dy));
      } else {
        placed.push(it.box);
      }
    }
  }

  // Glissement doux de l'élément vers sa place ; renvoie le déplacement de l'image (ou null).
  function glide(ref, k) {
    const tx = ref.layoutTX || 0;
    const ty = ref.layoutTY || 0;
    const x0 = ref.layoutX || 0;
    const y0 = ref.layoutY || 0;
    if (x0 === tx && y0 === ty) return null;
    let x1 = x0 + (tx - x0) * k;
    let y1 = y0 + (ty - y0) * k;
    if (Math.abs(tx - x1) < 0.3) x1 = tx;
    if (Math.abs(ty - y1) < 0.3) y1 = ty;
    ref.layoutX = x1;
    ref.layoutY = y1;
    return { dx: x1 - x0, dy: y1 - y0 };
  }

  // Un élément pris à la souris repart de là où on le pose : on oublie son décalage automatique.
  function clearLayout(ref) {
    ref.layoutX = ref.layoutY = ref.layoutTX = ref.layoutTY = 0;
  }

  function settleCards(dt, now, cx, cy, R) {
    if (now - lastLayoutAt > LAYOUT_EVERY_MS) {
      lastLayoutAt = now;
      solveLayout(now, cx, cy, R);
    }
    const k = Math.min(1, dt * SETTLE_SPEED);
    // Card d'action : branches.js ajoute le décalage à sa position d'origine.
    if (J.getActionCards) for (const card of J.getActionCards()) if (!card.isDragging) glide(card, k);
    // Encart de tâche : sa position vient de son point d'ancrage, qui glisse avec lui.
    for (const task of tasks.values()) {
      if (task.isDragging) continue;
      const d = glide(task, k);
      if (d && task.slot) {
        task.slot.endX += d.dx;
        task.slot.endY += d.dy;
        if (task.slot.box) {
          task.slot.box.x += d.dx;
          task.slot.box.y += d.dy;
        }
      }
    }
    // Widget HTML : décalage CSS (réappliqué si widgets.js a recréé l'élément).
    if (J.widgets && J.widgets.cards) {
      for (const rec of J.widgets.cards.values()) {
        if (!rec.el || rec.el.classList.contains('is-dragging')) continue;
        // La liaison card → widget suit le glissement : la boîte en cache est déplacée du même pas (pas de
        // relecture de mise en page juste après avoir écrit le décalage CSS).
        const moved = glide(rec, k);
        if (moved && rec._cachedBox) rec._cachedBox = { ...rec._cachedBox, x: rec._cachedBox.x + moved.dx, y: rec._cachedBox.y + moved.dy };
        else if (moved) rec._cachedBox = null;
        const px = (v) => Math.round((v || 0) * S * 10) / 10;
        const want = rec.layoutX || rec.layoutY ? `${px(rec.layoutX)}px ${px(rec.layoutY)}px` : '';
        if (rec.el.style.translate !== want) rec.el.style.translate = want;
      }
    }
    // Trace de rémanence : la boîte et sa liaison au cœur suivent ; sans place, elle s'efface.
    for (const trace of remanenceTraces) {
      if (!trace.homeBox) continue;
      const fade = trace.fade === undefined ? 1 : trace.fade;
      trace.fade = trace.layoutBlocked ? Math.max(0, fade - dt / 0.8) : Math.min(1, fade + dt / 0.8);
      if (trace.fade <= 0) trace.gone = true;
      if (!glide(trace, k)) continue;
      const b = trace.homeBox;
      trace.box = { x: b.x + trace.layoutX, y: b.y + trace.layoutY, w: b.w, h: b.h };
      const slot = {
        endX: trace.sx > 0 ? trace.box.x - 10 : trace.box.x + trace.box.w + 10,
        endY: trace.box.y + trace.box.h / 2,
        sx: trace.sx
      };
      trace.geo = J.branchGeometry(cx, cy, R, slot, W, H);
    }
  }

  // --- Bouton × des cards (v2) : fermer une card arrête sa tâche -----------------------------
  const ACTIVE_STATUSES = ['running', 'awaiting', 'waiting_reply'];
  function inBox(b, mx, my) {
    return Boolean(b) && mx >= b.x && mx <= b.x + b.w && my >= b.y && my <= b.y + b.h;
  }
  // Tâche dont le bouton × est sous la souris (card permanente ou encart de tâche).
  function closeTargetAt(mx, my) {
    if (J.getActionCards) {
      for (const card of J.getActionCards()) {
        if (card.closeBox && card.activeTask && J.cardVisible(card) && inBox(card.closeBox, mx, my)) return { task: card.activeTask, owner: card };
      }
    }
    const now = performance.now();
    for (const task of [...tasks.values()].reverse()) {
      if (task.grow > 0.94 && task.closeBox && (!task.endedAt || now - task.endedAt < 2500) && inBox(task.closeBox, mx, my)) {
        return { task, owner: task };
      }
    }
    return null;
  }
  function dismissTask(task) {
    // Disparition immédiate (sans les 2 s de fondu) ; la rémanence garde une trace discrète.
    const past = performance.now() - 2000;
    task.endedAt = task.endedAt ? Math.min(task.endedAt, past) : past;
    if (J.finishActionCard) J.finishActionCard(task.id, task.status === 'done' ? 'done' : 'error');
  }
  function closeTask(task) {
    const active = !task.endedAt && ACTIVE_STATUSES.includes(task.status);
    if (!active || !window.jarvisBridge || !window.jarvisBridge.cancelTask) {
      dismissTask(task);
      return;
    }
    task.detail = 'arrêt demandé…';
    window.jarvisBridge.cancelTask(task.id).then((res) => {
      // Tâche inconnue de l'orchestrateur (action locale déjà finie) : on ferme simplement.
      if (!res || !res.ok) {
        Object.assign(task, { status: 'error', cancelled: true });
        dismissTask(task);
      }
    }).catch(() => dismissTask(task));
  }

  window.addEventListener('mousemove', (e) => {
    mousePos.x = e.clientX / S;
    mousePos.y = e.clientY / S;
    const mx = mousePos.x;
    const my = mousePos.y;

    if (isDragging) {
      lastDirectInterventionAt = performance.now();
      if (Math.hypot(mx - dragStartPos.x, my - dragStartPos.y) > 3) {
        hasDragged = true;
      }

      // Déplacement d'une Card d'action permanente
      if (draggedActionCard) {
        const b = draggedActionCard.panelBox || { w: 230, h: 48 };
        let newX = mx - dragCardOffset.x;
        let newY = my - dragCardOffset.y;
        newX = Math.max(14, Math.min(W - b.w - 14, newX));
        newY = Math.max(60, Math.min(H - b.h - 60, newY));

        const cx = W / 2;
        const cardCenterX = newX + b.w / 2;
        const sx = cardCenterX >= cx ? 1 : -1;
        const endX = sx > 0 ? newX - 10 : newX + b.w + 10;
        const endY = newY + b.h / 2;

        draggedActionCard.customPos = { x: newX, y: newY };
        clearLayout(draggedActionCard);
        draggedActionCard.panelBox.x = newX;
        draggedActionCard.panelBox.y = newY;
        if (!draggedActionCard.slot) draggedActionCard.slot = {};
        draggedActionCard.slot.endX = endX;
        draggedActionCard.slot.endY = endY;
        draggedActionCard.slot.sx = sx;
        draggedActionCard.slot.box = { x: newX, y: newY, w: b.w, h: b.h };
        draggedActionCard.isDragging = true;
        canvas.style.cursor = 'grabbing';
        return;
      }

      // Déplacement de l'encart d'action sélectionné
      if (draggedTask) {
        const b = draggedTask.panelBox || { w: 220, h: 46 };
        let newX = mx - dragTaskOffset.x;
        let newY = my - dragTaskOffset.y;
        newX = Math.max(14, Math.min(W - b.w - 14, newX));
        newY = Math.max(60, Math.min(H - b.h - 60, newY));

        const cx = W / 2;
        const taskCenterX = newX + b.w / 2;
        const sx = taskCenterX >= cx ? 1 : -1;
        const endX = sx > 0 ? newX - 10 : newX + b.w + 10;
        const endY = newY + b.h / 2;

        if (!draggedTask.slot) draggedTask.slot = {};
        draggedTask.slot.endX = endX;
        draggedTask.slot.endY = endY;
        draggedTask.slot.sx = sx;
        draggedTask.slot.box = { x: newX, y: newY, w: b.w, h: b.h };
        draggedTask.panelBox.x = newX;
        draggedTask.panelBox.y = newY;
        draggedTask.isDragging = true;
        draggedTask.isManuallyPositioned = true;
        clearLayout(draggedTask);
        canvas.style.cursor = 'grabbing';
        return;
      }

      // Déplacement du panneau d'encart sélectionné
      if (draggedPanel) {
        if (draggedPanel === 'journal' && journalBox) {
          const px = Math.max(14, Math.min(W - journalBox.width - 14, mx - dragPanelOffset.x));
          const py = Math.max(60, Math.min(H - journalBox.height - 60, my - dragPanelOffset.y));
          journalCustomPos = { x: px, y: py };
        } else if (draggedPanel === 'transcript' && transcriptBox) {
          const px = Math.max(14, Math.min(W - transcriptBox.width - 14, mx - dragPanelOffset.x));
          const py = Math.max(60, Math.min(H - transcriptBox.height - 60, my - dragPanelOffset.y));
          transcriptCustomPos = { x: px, y: py };
        } else if (draggedPanel === 'memory' && memoryBox) {
          const px = Math.max(14, Math.min(W - memoryBox.width - 14, mx - dragPanelOffset.x));
          const py = Math.max(60, Math.min(H - memoryBox.height - 60, my - dragPanelOffset.y));
          memoryCustomPos = { x: px, y: py };
        }
        canvas.style.cursor = 'grabbing';
        return;
      }
    }

    // Détection de survol (hover) des Action Cards permanentes (actives OU idle)
    let overActionCard = null;
    if (J.getActionCards) {
      for (const card of J.getActionCards()) {
        if (card.panelBox && J.cardVisible(card)) {
          const b = card.panelBox;
          if (mx >= b.x && mx <= b.x + b.w && my >= b.y && my <= b.y + b.h) {
            overActionCard = card;
            card.isHovered = true;
          } else {
            card.isHovered = false;
          }
        }
      }
    }

    // Détection de survol (hover) des encarts de tâches actives
    let overTask = null;
    const now = performance.now();
    for (const task of [...tasks.values()].reverse()) {
      if (task.grow > 0.94 && task.panelBox && (!task.endedAt || (now - task.endedAt < 2500))) {
        const b = task.panelBox;
        if (mx >= b.x && mx <= b.x + b.w && my >= b.y && my <= b.y + b.h) {
          overTask = task;
          break;
        }
      }
    }

    for (const task of tasks.values()) {
      task.isHovered = (task === overTask);
    }

    // Survol du bouton × (surbrillance et curseur main)
    const closeHit = closeTargetAt(mx, my);
    for (const task of tasks.values()) task.closeHover = Boolean(closeHit && closeHit.owner === task);
    if (J.getActionCards) for (const card of J.getActionCards()) card.closeHover = Boolean(closeHit && closeHit.owner === card);

    // Détection de survol des en-têtes de panneaux d'encart
    let overPanelHeader = false;
    if (showJournal && journalBox && mx >= journalBox.x && mx <= journalBox.x + journalBox.width && my >= journalBox.y && my <= journalBox.y + 36) {
      overPanelHeader = true;
    } else if (showTranscript && transcriptBox && mx >= transcriptBox.x && mx <= transcriptBox.x + transcriptBox.width && my >= transcriptBox.y && my <= transcriptBox.y + 36) {
      overPanelHeader = true;
    } else if (showMemory && memoryBox && mx >= memoryBox.x && mx <= memoryBox.x + memoryBox.width && my >= memoryBox.y && my <= memoryBox.y + 36) {
      overPanelHeader = true;
    }

    const overAudio = audioBtnCoords && mx >= audioBtnCoords.x && mx <= audioBtnCoords.x + audioBtnCoords.w && my >= audioBtnCoords.y && my <= audioBtnCoords.y + audioBtnCoords.h;
    const overHelp = (helpBtnCoords && mx >= helpBtnCoords.x && mx <= helpBtnCoords.x + helpBtnCoords.w && my >= helpBtnCoords.y && my <= helpBtnCoords.y + helpBtnCoords.h) ||
      (configBtnCoords && mx >= configBtnCoords.x && mx <= configBtnCoords.x + configBtnCoords.w && my >= configBtnCoords.y && my <= configBtnCoords.y + configBtnCoords.h);
    const overVerify = verifyBtnCoords && mx >= verifyBtnCoords.x && mx <= verifyBtnCoords.x + verifyBtnCoords.w && my >= verifyBtnCoords.y && my <= verifyBtnCoords.y + verifyBtnCoords.h;
    const overClearMemory = clearMemoryBtnCoords && mx >= clearMemoryBtnCoords.x && mx <= clearMemoryBtnCoords.x + clearMemoryBtnCoords.w && my >= clearMemoryBtnCoords.y && my <= clearMemoryBtnCoords.y + clearMemoryBtnCoords.h;

    if (closeHit) {
      canvas.style.cursor = 'pointer';
    } else if (overTask || overPanelHeader || overActionCard) {
      canvas.style.cursor = 'grab';
    } else if (overAudio || overHelp || overVerify || overClearMemory || overLearn(mx, my)) {
      canvas.style.cursor = 'pointer';
    } else {
      canvas.style.cursor = 'default';
    }
  });

  canvas.addEventListener('mousedown', (e) => {
    J.markSolicited();
    if (e.button !== 0) return;
    const rect = canvas.getBoundingClientRect();
    const mx = (e.clientX - rect.left) / S;
    const my = (e.clientY - rect.top) / S;
    dragStartPos = { x: mx, y: my };
    hasDragged = false;

    // 1. Boutons cliquables prioritaires (ne pas initier de drag sur un bouton)
    const cm = clearMemoryBtnCoords;
    if (cm && mx >= cm.x && mx <= cm.x + cm.w && my >= cm.y && my <= cm.y + cm.h) return;
    const ab = audioBtnCoords;
    if (ab && mx >= ab.x && mx <= ab.x + ab.w && my >= ab.y && my <= ab.y + ab.h) return;
    const cb = configBtnCoords;
    if (cb && mx >= cb.x && mx <= cb.x + cb.w && my >= cb.y && my <= cb.y + cb.h) return;
    const b = helpBtnCoords;
    if (b && mx >= b.x && mx <= b.x + b.w && my >= b.y && my <= b.y + b.h) return;
    const vb = verifyBtnCoords;
    if (vb && mx >= vb.x && mx <= vb.x + vb.w && my >= vb.y && my <= vb.y + vb.h) return;
    const lb = showTranscript && !transcriptCollapsed ? transcriptLearnBtn : null;
    if (lb && mx >= lb.x && mx <= lb.x + lb.w && my >= lb.y && my <= lb.y + lb.h) return;
    const dbt = showTranscript && !transcriptCollapsed ? transcriptDictionBtn : null;
    if (dbt && mx >= dbt.x && mx <= dbt.x + dbt.w && my >= dbt.y && my <= dbt.y + dbt.h) return;

    // 1 bis. Bouton × d'une card : arrête la tâche (avant toute prise pour déplacement)
    const closeHit = closeTargetAt(mx, my);
    if (closeHit) {
      closeTask(closeHit.task);
      e.preventDefault();
      return;
    }

    // 2. Détection de clic sur une Action Card permanente (active OU idle)
    if (J.getActionCards) {
      for (const card of J.getActionCards()) {
        if (card.panelBox && J.cardVisible(card)) {
          const box = card.panelBox;
          if (mx >= box.x && mx <= box.x + box.w && my >= box.y && my <= box.y + box.h) {
            draggedActionCard = card;
            dragCardOffset = { x: mx - box.x, y: my - box.y };
            draggedActionCard.isDragging = true;
            isDragging = true;
            canvas.style.cursor = 'grabbing';
            e.preventDefault();
            return;
          }
        }
      }
    }

    // 3. Détection de prise d'un encart de tâche actif
    const now = performance.now();
    for (const task of [...tasks.values()].reverse()) {
      if (task.grow > 0.94 && task.panelBox && (!task.endedAt || (now - task.endedAt < 2500))) {
        const box = task.panelBox;
        if (mx >= box.x && mx <= box.x + box.w && my >= box.y && my <= box.y + box.h) {
          draggedTask = task;
          dragTaskOffset = { x: mx - box.x, y: my - box.y };
          draggedTask.isDragging = true;
          isDragging = true;
          canvas.style.cursor = 'grabbing';
          e.preventDefault();
          return;
        }
      }
    }

    // 4. Détection de prise sur la barre de titre d'un panneau
    if (showJournal && journalBox && mx >= journalBox.x && mx <= journalBox.x + journalBox.width && my >= journalBox.y && my <= journalBox.y + 36) {
      draggedPanel = 'journal';
      dragPanelOffset = { x: mx - journalBox.x, y: my - journalBox.y };
      isDragging = true;
      canvas.style.cursor = 'grabbing';
      e.preventDefault();
      return;
    }
    if (showTranscript && transcriptBox && !transcriptPinned && mx >= transcriptBox.x && mx <= transcriptBox.x + transcriptBox.width && my >= transcriptBox.y && my <= transcriptBox.y + 36) {
      draggedPanel = 'transcript';
      dragPanelOffset = { x: mx - transcriptBox.x, y: my - transcriptBox.y };
      isDragging = true;
      canvas.style.cursor = 'grabbing';
      e.preventDefault();
      return;
    }
    if (showMemory && memoryBox && mx >= memoryBox.x && mx <= memoryBox.x + memoryBox.width && my >= memoryBox.y && my <= memoryBox.y + 36) {
      draggedPanel = 'memory';
      dragPanelOffset = { x: mx - memoryBox.x, y: my - memoryBox.y };
      isDragging = true;
      canvas.style.cursor = 'grabbing';
      e.preventDefault();
      return;
    }
  });

  window.addEventListener('mouseup', () => {
    if (isDragging) {
      if (draggedActionCard) {
        draggedActionCard.isDragging = false;
        if (draggedActionCard.customPos && typeof localStorage !== 'undefined') {
          try {
            localStorage.setItem((window.JARVIS_POS_PREFIX || 'jarvis.') + 'cardPos.' + draggedActionCard.type, JSON.stringify(draggedActionCard.customPos));
          } catch {}
        }
        draggedActionCard = null;
      }
      if (draggedTask) {
        draggedTask.isDragging = false;
        draggedTask = null;
      }
      if (draggedPanel === 'transcript' && transcriptCustomPos) {
        try { localStorage.setItem((window.JARVIS_POS_PREFIX || 'jarvis.') + 'transcriptPos', JSON.stringify(transcriptCustomPos)); } catch {}
      }
      draggedPanel = null;
      isDragging = false;
      canvas.style.cursor = 'default';
    }
  });

  window.addEventListener('blur', () => {
    if (isDragging) {
      if (draggedActionCard) {
        draggedActionCard.isDragging = false;
        draggedActionCard = null;
      }
      if (draggedTask) {
        draggedTask.isDragging = false;
        draggedTask = null;
      }
      draggedPanel = null;
      isDragging = false;
      canvas.style.cursor = 'default';
    }
  });

  window.addEventListener('wheel', (e) => {
    const mx = mousePos.x;
    const my = mousePos.y;

    // Défilement du journal scellé
    if (showJournal && journalBox && mx >= journalBox.x && mx <= journalBox.x + journalBox.width && my >= journalBox.y && my <= journalBox.y + journalBox.height) {
      e.preventDefault();
      if (e.deltaY < 0) {
        journalScrollOffset = Math.min(Math.max(0, (journalEntries.length || 0) - 3), journalScrollOffset + 1);
      } else if (e.deltaY > 0) {
        journalScrollOffset = Math.max(0, journalScrollOffset - 1);
      }
      return;
    }

    // Défilement de la transcription
    if (showTranscript && transcriptBox && mx >= transcriptBox.x && mx <= transcriptBox.x + transcriptBox.width && my >= transcriptBox.y && my <= transcriptBox.y + transcriptBox.height) {
      e.preventDefault();
      if (e.deltaY < 0) {
        transcriptScrollOffset = Math.min(Math.max(0, transcriptRowCount - 4), transcriptScrollOffset + 1);
      } else if (e.deltaY > 0) {
        transcriptScrollOffset = Math.max(0, transcriptScrollOffset - 1);
      }
    }
  }, { passive: false });

  canvas.addEventListener('click', (e) => {
    if (hasDragged) {
      hasDragged = false;
      return;
    }
    lastDirectInterventionAt = performance.now();
    const rect = canvas.getBoundingClientRect();
    const clickX = (e.clientX - rect.left) / S;
    const clickY = (e.clientY - rect.top) / S;

    // Zone conversation : bouton APPRENTISSAGE, puis une phrase de voix non reconnue (« qui parle ? »).
    if (showTranscript && transcriptBox && !transcriptCollapsed) {
      const lb = transcriptLearnBtn;
      if (lb && clickX >= lb.x && clickX <= lb.x + lb.w && clickY >= lb.y && clickY <= lb.y + lb.h) {
        toggleLearning();
        return;
      }
      const dbc = transcriptDictionBtn;
      if (dbc && clickX >= dbc.x && clickX <= dbc.x + dbc.w && clickY >= dbc.y && clickY <= dbc.y + dbc.h) {
        if (window.jarvisBridge && window.jarvisBridge.call) window.jarvisBridge.call('diction', 'ouvrir', {}).catch(() => {});
        return;
      }
      if (clickX >= transcriptBox.x && clickX <= transcriptBox.x + transcriptBox.width) {
        const hit = transcriptRowHits.find((r) => clickY >= r.y0 && clickY <= r.y1);
        if (hit && hit.entry.ask && J.speakerAsk) {
          J.speakerAsk.show(hit.entry.ask);
          return;
        }
      }
    }
    // Zone conversation : bouton « – » (réduire) ou bouton réduit (rouvrir).
    if (showTranscript && transcriptBox) {
      const tb = transcriptToggleBtn;
      const inBtn = (b) => b && clickX >= b.x && clickX <= b.x + b.w && clickY >= b.y && clickY <= b.y + b.h;
      if (!transcriptCollapsed && inBtn(transcriptFixBtn)) { // 📍 : emplacement actuel = emplacement par défaut (Maj : l'oublier)
        const p = transcriptDefault();
        try {
          if (e.shiftKey) localStorage.removeItem(TRANSCRIPT_DEFAULT_KEY());
          else if (!p) localStorage.setItem(TRANSCRIPT_DEFAULT_KEY(), JSON.stringify({ x: Math.round(transcriptBox.x), y: Math.round(transcriptBox.y) })); // pas fixé : on fixe ici
          else if (Math.abs(transcriptBox.x - p.x) <= 12 && Math.abs(transcriptBox.y - p.y) <= 12) localStorage.removeItem(TRANSCRIPT_DEFAULT_KEY()); // déjà là : on désactive
          else { transcriptCustomPos = { x: p.x, y: p.y }; saveTranscriptPos(); } // ailleurs : on la replace
        } catch {}
        return;
      }
      if (!transcriptCollapsed && inBtn(transcriptTitleBtn)) { // clic sur le numéro ou le titre : la card se réduit
        transcriptCollapsed = true;
        try { localStorage.setItem('jarvis.transcriptCollapsed', '1'); } catch {}
        return;
      }
      if (!transcriptCollapsed && inBtn(transcriptPinBtn)) { // 📌 : la card ne se déplace plus
        transcriptPinned = !transcriptPinned;
        try { localStorage.setItem('jarvis.transcriptPinned', transcriptPinned ? '1' : '0'); } catch {}
        return;
      }
      const onBtn = transcriptCollapsed
        ? clickX >= transcriptBox.x && clickX <= transcriptBox.x + transcriptBox.width && clickY >= transcriptBox.y && clickY <= transcriptBox.y + transcriptBox.height
        : tb && clickX >= tb.x && clickX <= tb.x + tb.w && clickY >= tb.y && clickY <= tb.y + tb.h;
      if (onBtn) {
        transcriptCollapsed = !transcriptCollapsed;
        try { localStorage.setItem('jarvis.transcriptCollapsed', transcriptCollapsed ? '1' : '0'); } catch {}
        return;
      }
    }

    const cm = clearMemoryBtnCoords;
    if (cm && clickX >= cm.x && clickX <= cm.x + cm.w && clickY >= cm.y && clickY <= cm.y + cm.h) {
      runCommand('/memoire reset');
      return;
    }

    const ab = audioBtnCoords;
    if (ab && clickX >= ab.x && clickX <= ab.x + ab.w && clickY >= ab.y && clickY <= ab.y + ab.h) {
      if (window.jarvisBridge && window.jarvisBridge.toggleAudioDevice) {
        window.jarvisBridge.toggleAudioDevice().then((res) => {
          if (res) audioDevice = res;
        }).catch((err) => console.error('Erreur basculement audio:', err));
      }
      return;
    }

    const cb = configBtnCoords;
    if (cb && clickX >= cb.x && clickX <= cb.x + cb.w && clickY >= cb.y && clickY <= cb.y + cb.h) {
      openAdmin();
      return;
    }

    const b = helpBtnCoords;
    if (b && clickX >= b.x && clickX <= b.x + b.w && clickY >= b.y && clickY <= b.y + b.h) {
      if (showHelp) showHelp = false; // aide-mémoire ouvert : le bouton le referme
      else openHelp();
      return;
    }
    if (!showJournal || !verifyBtnCoords) return;
    if (
      clickX >= verifyBtnCoords.x &&
      clickX <= verifyBtnCoords.x + verifyBtnCoords.w &&
      clickY >= verifyBtnCoords.y &&
      clickY <= verifyBtnCoords.y + verifyBtnCoords.h
    ) {
      runCommand('/verifier');
    }
  });

  window.addEventListener('keyup', (e) => {
    if (e.key === ' ' && isPttActive) {
      isPttActive = false;
      lastPttAt = performance.now();
      if (window.jarvisBridge && window.jarvisBridge.ptt) window.jarvisBridge.ptt(false);
    }
  });

  // --- Webcam (orchestrator/vision.js) ----------------------------------------
  // Sur demande seulement : aperçu, compte à rebours, UNE image, puis la caméra est coupée.
  let cameraBusy = false;
  async function cameraCapture(ev) {
    const bridge = window.jarvisBridge;
    if (cameraBusy || !bridge) return;
    cameraBusy = true;
    const box = document.createElement('div');
    box.className = 'cam-overlay';
    box.innerHTML = '<div class="cam-frame"><video autoplay muted playsinline></video><div class="cam-count"></div><div class="cam-label">◉ WEBCAM · une image</div><button type="button" class="cam-cancel">Annuler</button></div>';
    document.body.appendChild(box);
    const video = box.querySelector('video');
    const count = box.querySelector('.cam-count');
    let stream = null;
    let cancelled = false;
    const close = () => {
      if (stream) stream.getTracks().forEach((tr) => tr.stop());
      box.classList.remove('visible');
      setTimeout(() => box.remove(), 300);
      cameraBusy = false;
    };
    box.querySelector('.cam-cancel').addEventListener('click', () => {
      cancelled = true;
      bridge.cameraError(ev.id, 'annulé par l’utilisateur');
      close();
    });
    try {
      stream = await navigator.mediaDevices.getUserMedia({ video: { width: { ideal: 1280 }, height: { ideal: 720 } }, audio: false });
      video.srcObject = stream;
      await new Promise((res) => { if (video.readyState >= 2) res(); else video.onloadeddata = () => res(); });
      requestAnimationFrame(() => box.classList.add('visible'));
      for (let n = Math.max(1, ev.countdown || 2); n > 0 && !cancelled; n--) {
        count.textContent = n;
        count.classList.remove('tick');
        void count.offsetWidth;
        count.classList.add('tick');
        await new Promise((res) => setTimeout(res, 1000));
      }
      if (cancelled) return;
      count.textContent = '';
      const canvas = document.createElement('canvas');
      canvas.width = video.videoWidth || 1280;
      canvas.height = video.videoHeight || 720;
      canvas.getContext('2d').drawImage(video, 0, 0, canvas.width, canvas.height);
      box.classList.add('flash');
      await bridge.cameraFrame(ev.id, canvas.toDataURL('image/jpeg', 0.85));
      setTimeout(close, 450);
    } catch (err) {
      if (!cancelled) bridge.cameraError(ev.id, `${err && err.name ? err.name : 'Erreur'} ${err && err.message ? err.message : ''}`.trim());
      close();
    }
  }

  // --- Présence par webcam ------------------------------------------------------
  // Réglage presence.enabled : une image 320×240 par seconde, envoyée en pixels RVB au noyau qui compte les visages
  // (YuNet, sur le PC) et n'en garde rien. Une seule page HUD capture ; la photo « regarde ça » reste prioritaire.
  const pbridge = () => window.jarvisBridge;
  const presence = { config: null, stream: null, video: null, canvas: null, timer: null, sending: false };

  async function presenceStart() {
    if (presence.stream || !pbridge()) return;
    try {
      presence.stream = await navigator.mediaDevices.getUserMedia({ video: { width: { ideal: 640 }, height: { ideal: 480 }, frameRate: { ideal: 5 } }, audio: false });
    } catch (err) {
      console.warn('[présence] webcam indisponible', err);
      presence.stream = null;
      return;
    }
    presence.video = document.createElement('video');
    presence.video.muted = true;
    presence.video.playsInline = true;
    presence.video.srcObject = presence.stream;
    try { await presence.video.play(); } catch (_) { /* lecture sans affichage */ }
    presence.canvas = document.createElement('canvas');
    presence.canvas.width = presence.config.width || 320;
    presence.canvas.height = presence.config.height || 240;
    presence.timer = setInterval(presenceTick, presence.config.intervalMs || 1000);
  }

  function presenceStop() {
    clearInterval(presence.timer);
    presence.timer = null;
    if (presence.stream) presence.stream.getTracks().forEach((t) => t.stop());
    presence.stream = null;
    document.body.classList.remove('privacy');
  }

  async function presenceTick() {
    if (presence.sending || cameraBusy || !presence.video || presence.video.readyState < 2) return;
    const { width, height } = presence.canvas;
    const ctx = presence.canvas.getContext('2d', { willReadFrequently: true });
    ctx.drawImage(presence.video, 0, 0, width, height);
    const rgba = ctx.getImageData(0, 0, width, height).data;
    const rgb = new Uint8Array(width * height * 3);
    for (let i = 0, j = 0; i < rgba.length; i += 4, j += 3) {
      rgb[j] = rgba[i]; rgb[j + 1] = rgba[i + 1]; rgb[j + 2] = rgba[i + 2];
    }
    let bin = '';
    for (let i = 0; i < rgb.length; i += 0x8000) bin += String.fromCharCode.apply(null, rgb.subarray(i, i + 0x8000));
    presence.sending = true;
    try {
      const r = await pbridge().presenceFrame(btoa(bin));
      if (r && r.enabled === false) presenceStop();
    } catch (_) { /* noyau occupé ou relancé : image suivante */ }
    presence.sending = false;
  }

  function presenceApply(config) {
    presence.config = config || { enabled: false };
    const leader = !window.jarvisBridge || !window.jarvisBridge.isLeader || window.jarvisBridge.isLeader();
    if (presence.config.enabled && leader) presenceStart(); else presenceStop();
  }

  if (pbridge() && pbridge().presenceConfig) {
    setTimeout(() => pbridge().presenceConfig().then(presenceApply).catch(() => {}), 3000);
  }

  // --- Pont orchestrateur (étape 3) -------------------------------------------

  if (window.jarvisBridge) {
    window.jarvisBridge.onTaskEvent((ev) => {
      if (ev.type === 'create') {
        activityEnergy = Math.min(1, activityEnergy + 0.45);
        lastDirectInterventionAt = performance.now();
        triggerIntentMorph(`${ev.label || ''} ${ev.detail || ''}`);
        api.create(ev);
      }
      else if (ev.type === 'update') {
        activityEnergy = Math.min(1, activityEnergy + 0.15);
        if (ev.status === 'awaiting') {
          isAiThinking = false;
          if (J.triggerOrbMorph) J.triggerOrbMorph('security');
        } else if (ev.status === 'waiting_reply') {
          isAiThinking = false;
        }
        api.update(ev.id, ev);
      }
      else if (ev.type === 'finish') {
        activityEnergy = Math.min(1, activityEnergy + 0.10);
        aiCalls.delete(ev.id); // filet de sécurité si la fin d'appel n'est pas parvenue
        api.finish(ev.id, ev.status);
      }
      else if (ev.type === 'ai_call') {
        const aiTask = api.get(ev.id);
        if (aiTask) aiTask.aiActive = ev.state === 'start'; // contour rouge de la card (branches.js)
        if (ev.state === 'start') {
          aiCalls.set(ev.id, { engine: ev.engine, model: ev.model, startedAt: ev.startedAt || Date.now() });
          activityEnergy = Math.min(1, activityEnergy + 0.3);
        } else {
          aiCalls.delete(ev.id);
        }
      }
      else if (ev.type === 'say') {
        isAiThinking = false;
        activityEnergy = Math.min(1, activityEnergy + 0.50);
        // Toute parole de Jarvis est aussi écrite dans la zone de conversation.
        pushJarvis(ev);
        if (ev.muted) {
          lastMutedSpeech = { text: ev.text, reason: ev.muteReason || 'Application', at: performance.now() };
        } else {
          rememberSpeech(ev.text); // pour reconnaître son écho dans le micro
          speak(ev.text, { task: ev.task, stream: ev.stream });
        }
      }
      else if (ev.type === 'media') {
        J.currentMedia = ev;
        if (ev.active) {
          if (J.triggerOrbMorph) J.triggerOrbMorph('music');
        }
      }
      else if (ev.type === 'voice_tune') {
        if (J.voice && J.voice.tune) J.voice.tune(ev.options);
      }
      else if (ev.type === 'voice_command') {
        if (J.voice && J.voice.command) J.voice.command(ev.payload);
      }
      else if (ev.type === 'say_end') {
        isAiThinking = false;
        if (J.voice && J.voice.sayEnd) J.voice.sayEnd(ev.task);
      }
      else if (ev.type === 'stopped') {
        // Arrêt général : flash rouge et trace écrite (rien n'est prononcé).
        lastErrorAt = performance.now();
        consoleBar.classList.add('reject');
        setTimeout(() => consoleBar.classList.remove('reject'), 500);
        pushJarvis({ text: ev.count ? `■ Tout est arrêté (${ev.count} élément${ev.count > 1 ? 's' : ''}).` : '■ Rien à arrêter.' });
      }
      else if (ev.type === 'camera_capture') {
        // Deux pages HUD (fenêtre classique, Bureau JARVIS) : une seule prend la photo.
        if (!window.jarvisBridge || !window.jarvisBridge.isLeader || window.jarvisBridge.isLeader()) cameraCapture(ev);
      }
      else if (ev.type === 'context_mute') {
        contextMute = ev;
      }
      else if (ev.type === 'presence_config') {
        presenceApply(ev.config);
      }
      else if (ev.type === 'presence') {
        // Mode vie privée : d'autres personnes devant l'écran, les cards sont floutées.
        document.body.classList.toggle('privacy', !!ev.privacy && !(presence.config && presence.config.privacyBlur === false));
      }
      else if (ev.type === 'theme_config') {
        applyTheme(ev.theme);
      }
      else if (ev.type === 'perf_config') {
        applyPerf(ev);
      }
      else if (ev.type === 'density_config') {
        applyDensity(ev.density);
      }
      else if (ev.type === 'heart_style_config') {
        applyHeartStyle(ev.style);
      }
      // Compagnon de bureau (option) : le personnage du cœur grandit et s'équipe selon son état.
      else if (ev.type === 'companion') companionState = ev.enabled ? ev : null;
      else if (ev.type === 'voice_config') {
        if (J.voice && J.voice.setVoice) J.voice.setVoice(ev.voice, ev.rate, ev.sample);
      }
      else if (ev.type === 'memory') memories = ev.entries || [];
      else if (ev.type === 'journal') {
        journal = ev;
        if (ev.entries) journalEntries = ev.entries;
      }
      else if (ev.type === 'answer_full') {
        isAiThinking = false;
        lastFullAnswer = ev;
      }
      else if (ev.type === 'toggle_journal') {
        showJournal = !showJournal;
        localStorage.setItem('jarvis.journal', showJournal ? '1' : '0');
      }
      else if (ev.type === 'widget_mode') {
        // Commande vocale « mode widget » / « plein écran » (Chantier D2)
        if (window.jarvisBridge && window.jarvisBridge.setWidget) window.jarvisBridge.setWidget(ev.on);
      }
      else if (ev.type === 'show_answer') {
        lastFullAnswer = ev.answer;
        showJournal = true;
        localStorage.setItem('jarvis.journal', '1');
      }
      else if (ev.type === 'speaker_pending_done') {
        // Phrase attribuée (ici ou depuis l'autre page HUD) : plus rien à demander pour elle.
        for (const entry of heard) if (entry.ask && entry.ask.pendingId === ev.pendingId) entry.ask = null;
      }
      else if (ev.type === 'status') {
        if (ev.speaker && ev.speaker.learning === false && J.speakerAsk) J.speakerAsk.hide();
        status = ev;
        if (ev.theme && ev.theme !== currentTheme) {
          applyTheme(ev.theme);
        }
        if (ev.density && ev.density !== currentDensity) {
          applyDensity(ev.density);
        }
        if (ev.heartStyle && ev.heartStyle !== heartStyle) {
          applyHeartStyle(ev.heartStyle);
        }
        if (ev.listeningMode) {
          listeningMode = ev.listeningMode;
        }
        if (ev.contextMute) {
          contextMute = ev.contextMute;
        }
        if (ev.voice && J.voice && J.voice.setVoice) {
          J.voice.setVoice(ev.voice, ev.voiceRate, false);
        }
      }
      else if (ev.type === 'level') J.setLevel(ev.value);
      else if (ev.type === 'telemetry') {
        telemetry = ev;
        updateResourceEcoMode(telemetry.cpu, telemetry.ramPct);
      }
      else if (ev.type === 'audio_device') {
        const changed = audioDevice && audioDevice.mode && ev.mode && audioDevice.mode !== ev.mode;
        audioDevice = ev;
        if (changed && J.voice && J.voice.reopenAudio) J.voice.reopenAudio();
      }
      else if (ev.type === 'vocabulary') {
        if (J.voice && J.voice.setVocabulary) J.voice.setVocabulary(ev);
      }
      else if (ev.type === 'vocab_ask') {
        const inp = document.getElementById('input');
        if (inp) {
          inp.placeholder = `Mot non compris « ${ev.heard} » : écris le mot souhaité ici...`;
          inp.focus();
        }
        pushHeard({ who: 'system', text: `[Mot non compris : « ${ev.heard} » · Écris le mot correct dans la console]` });
      }
      else if (ev.type === 'vocab_learned') {
        const inp = document.getElementById('input');
        if (inp) inp.placeholder = 'commande ou code de validation';
        if (ev.entendu && ev.correct) {
          pushHeard({ who: 'system', text: `[Vocabulaire associé : « ${ev.entendu} » → « ${ev.correct} »]` });
        }
      }
      else if (ev.type === 'speaker_enroll_start') {
        const name = ev.name || 'Johann';
        if (J.voice && J.voice.startEnrollSpeaker) J.voice.startEnrollSpeaker(name, ev.count || 3);
        pushHeard({ who: 'system', text: `[Enrôlement vocal actif pour ${name} : prononcez 2 à 3 phrases à voix normale...]` });
      }
      else if (ev.type === 'speaker_enroll_cancel') {
        if (J.voice && J.voice.cancelEnrollSpeaker) J.voice.cancelEnrollSpeaker();
        pushHeard({ who: 'system', text: "[Enrôlement vocal annulé]" });
      }
      else if (ev.type === 'speaker_delete') {
        if (J.voice && J.voice.deleteSpeaker) J.voice.deleteSpeaker(ev.id);
        pushHeard({ who: 'system', text: `[Locuteur supprimé]` });
      }
      else if (ev.type === 'speaker_rename') {
        if (J.voice && J.voice.renameSpeaker) J.voice.renameSpeaker(ev.id, ev.name);
      }
      else if (ev.type === 'speaker_primary') {
        if (J.voice && J.voice.setPrimarySpeaker) J.voice.setPrimarySpeaker(ev.id);
      }
      else if (ev.type === 'speaker_set_enabled') {
        if (J.voice && J.voice.setSpeakerEnabled) J.voice.setSpeakerEnabled(ev.enabled);
      }
      else if (ev.type === 'speaker_set_threshold') {
        if (J.voice && J.voice.setSpeakerThreshold) J.voice.setSpeakerThreshold(ev.threshold);
      }
      else if (ev.type === 'speaker_reset') {
        if (J.voice && J.voice.resetSpeaker) J.voice.resetSpeaker();
      }
    });

    if (window.jarvisBridge.getAudioDevice) {
      window.jarvisBridge.getAudioDevice().then((res) => {
        if (res && res.ok) audioDevice = res;
      }).catch(() => {});
    }
  }

  // --- Mode démonstration (à retirer quand l'orchestrateur pilote l'UI) -------

  const DEMO = [
    { label: 'Recherche web', detail: 'météo Bordeaux demain' },
    { label: 'Claude Code', detail: 'refactor du module auth' },
    { label: 'Lecture fichiers', detail: 'src/*.js — 12 fichiers' },
    { label: 'Mémoire', detail: 'préférence : café sans sucre' },
    { label: 'Agenda', detail: 'réunion déplacée à 14h30' },
    { label: 'Musique', detail: 'playlist focus' },
    { label: 'Suppression fichiers', detail: '3 fichiers temporaires', sensitive: true },
    { label: 'Git push', detail: 'main → origin', sensitive: true }
  ];

  function spawnDemo() {
    if (window.jarvisBridge) return;
    if (J.voice && J.voice.connected) {
      setTimeout(spawnDemo, 4000);
      return;
    }
    const model = DEMO[Math.floor(Math.random() * DEMO.length)];
    const id = api.create({ label: model.label, detail: model.detail });
    if (id) {
      if (model.sensitive) {
        setTimeout(() => {
          const code = String(1000 + Math.floor(Math.random() * 9000));
          api.update(id, { status: 'awaiting', code });
          setTimeout(() => {
            const task = api.get(id);
            if (task && task.status === 'awaiting') api.update(task.id, { status: 'error', code: null, detail: 'validation expirée' });
          }, 20000);
        }, 1600);
      } else {
        setTimeout(() => api.finish(id, Math.random() < 0.12 ? 'error' : 'done'), 4000 + Math.random() * 6000);
      }
    }
    setTimeout(spawnDemo, 2200 + Math.random() * 2600);
  }

  function simulateVoice() {
    if (window.jarvisBridge || (J.voice && J.voice.connected)) {
      setTimeout(simulateVoice, 1000);
      return;
    }
    levelTarget = Math.random() < 0.35 ? 0.15 + Math.random() * 0.8 : 0.05;
    setTimeout(simulateVoice, 180 + Math.random() * 320);
  }

  function initWindowControls() {
    const btnAppMode = document.getElementById('btn-app-mode');
    if (btnAppMode) {
      btnAppMode.addEventListener('click', () => {
        if (window.jarvisBridge && window.jarvisBridge.deskLeave) window.jarvisBridge.deskLeave();
      });
    }
    const btnDeskMode = document.getElementById('btn-desk-mode');
    if (btnDeskMode) {
      btnDeskMode.addEventListener('click', () => {
        if (window.jarvisBridge && window.jarvisBridge.deskEnter) window.jarvisBridge.deskEnter();
      });
    }
    const btnHalf = document.getElementById('btn-half-screen');
    const btnWidget = document.getElementById('btn-widget-mode');
    const btnDropdown = document.getElementById('btn-mode-dropdown');
    const dropdown = document.getElementById('mode-dropdown');
    const bridge = window.jarvisBridge;

    if (!btnHalf || !btnWidget) return;

    // Bascule demi-écran
    btnHalf.addEventListener('click', async () => {
      if (dropdown) dropdown.classList.add('hidden');
      if (bridge && bridge.toggleHalfScreen) {
        const res = await bridge.toggleHalfScreen();
        const isHalf = res && res.isHalfScreen;
        btnHalf.classList.toggle('active', Boolean(isHalf));
        const txt = btnHalf.querySelector('.hud-win-text');
        if (txt) txt.textContent = isHalf ? 'NORMAL' : '½ ÉCRAN';
      }
    });

    // Bascule widget flottant
    btnWidget.addEventListener('click', () => {
      if (dropdown) dropdown.classList.add('hidden');
      if (bridge && bridge.toggleWidget) {
        bridge.toggleWidget();
      }
    });

    // Menu déroulant
    if (btnDropdown && dropdown) {
      btnDropdown.addEventListener('click', (e) => {
        e.stopPropagation();
        dropdown.classList.toggle('hidden');
      });

      const items = dropdown.querySelectorAll('.mode-item');
      items.forEach((item) => {
        item.addEventListener('click', async () => {
          dropdown.classList.add('hidden');
          const mode = item.dataset.mode;
          if (!bridge) return;
          if (mode === 'half') {
            await bridge.setWindowMode('half');
            if (btnHalf) {
              btnHalf.classList.add('active');
              const txt = btnHalf.querySelector('.hud-win-text');
              if (txt) txt.textContent = 'NORMAL';
            }
          } else if (mode === 'widget') {
            bridge.setWindowMode('widget');
          } else if (mode === 'normal') {
            await bridge.setWindowMode('normal');
            if (btnHalf) {
              btnHalf.classList.remove('active');
              const txt = btnHalf.querySelector('.hud-win-text');
              if (txt) txt.textContent = '½ ÉCRAN';
            }
          }
        });
      });

      // Fermer le dropdown en cliquant ailleurs ou touche Échap
      document.addEventListener('click', (e) => {
        if (!dropdown.contains(e.target) && e.target !== btnDropdown) {
          dropdown.classList.add('hidden');
        }
      });
      window.addEventListener('keydown', (e) => {
        if (e.key === 'Escape' && !dropdown.classList.contains('hidden')) {
          dropdown.classList.add('hidden');
          e.stopPropagation();
        }
      });
    }

    // Récupérer l'état initial
    if (bridge && bridge.getWindowState) {
      bridge.getWindowState().then((state) => {
        if (state && state.isHalfScreen && btnHalf) {
          btnHalf.classList.add('active');
          const txt = btnHalf.querySelector('.hud-win-text');
          if (txt) txt.textContent = 'NORMAL';
        }
      }).catch(() => {});
    }
  }

  J.isResourceEcoMode = () => isResourceEcoMode;
  J.updateResourceEcoMode = updateResourceEcoMode;
  J.setResourceEcoMode = (v) => { isResourceEcoMode = Boolean(v); };
  J.drawSimpleEcoOrb = drawSimpleEcoOrb;
  J.ECO_TRIGGER_THRESHOLD = ECO_TRIGGER_THRESHOLD;
  J.ECO_RECOVERY_THRESHOLD = ECO_RECOVERY_THRESHOLD;

  window.addEventListener('resize', resize);
  resize();
  requestAnimationFrame(frame);
  setTimeout(spawnDemo, 600);
  simulateVoice();
  initWindowControls();
  input.focus();
})();
