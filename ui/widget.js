// Widget flottant (chantier v3, étape 8) : une orbe discrète de 80 × 80 px (réglable), toujours
// au premier plan, sur fond translucide. Les clics traversent le fond ; seul le cœur de l'orbe
// réagit : glisser pour déplacer, double-clic pour revenir à l'interface complète, clic droit
// pour le menu. En mode fantôme, tout traverse (Ctrl+Maj+G pour en sortir).
//
// Il ne traite aucune phrase (la fenêtre principale, masquée, continue de le faire) : il lit
// seulement le niveau du micro et l'état de la voix pour s'animer, et reçoit l'état des tâches.
(function () {
  const J = window.JARVIS;
  const bridge = window.jarvisBridge;
  const canvas = document.getElementById('stage');
  const g = canvas.getContext('2d');
  const MONO = 'Consolas, "Courier New", monospace';
  const CORE_RATIO = 0.34; // rayon de la zone cliquable, en fraction de la taille réglée
  // Bureau JARVIS (shell/src/desk.rs) : le widget remplace le cœur du HUD dès qu'une application le
  // recouvre, et la coquille le fait glisser vers un endroit libre. Discret au repos, il grossit
  // jusqu'à GROW quand Jarvis écoute, parle ou travaille (même facteur que WIDGET_GROW de main.rs,
  // qui dimensionne la fenêtre en conséquence).
  const GROW = 1.6;

  let W = 0; // côté du carré de l'orbe (taille réglée × croissance)
  let H = 0;
  let win = 0; // côté de la fenêtre (canvas)
  let winW = 0; // taille réelle du canvas (px CSS) : win × win, sauf pendant un trajet du cœur (fenêtre agrandie)
  let winH = 0;
  let fly = null; // trajet du cœur en cours (voir « Cœur unique » plus bas)
  let grow = 1;
  let dpr = 1;
  let level = 0;
  let levelTarget = 0;
  const levels = new Array(120).fill(0);
  let head = 0;
  const voice = { connected: false, capturing: false, speaking: false, muted: false };
  const tasks = new Map(); // id -> statut
  let lastErrorAt = -1e9;
  let config = { size: 80, opacity: 0.9, clickThrough: true, ghost: false };
  let hovering = false;
  let drag = null;
  let presence = 0.55; // opacité de l'orbe : discrète au repos, pleine quand il se passe quelque chose
  let voiceEnabled = true; // écoute orale coupée dans Administration › Optimisation : plus de reconnexion
  let socket = null;
  let wakeUntil = 0; // « Jarvis » entendu : cœur bleu pendant la phrase (et 2,5 s après)

  let theme = localStorage.getItem('jarvis.theme') || 'cyan';
  if (J.setOrbTheme) J.setOrbTheme(theme);

  // v2 : la fenêtre est un carré d'au moins 140 px (largeur minimale imposée par Windows),
  // transparent et traversant ; l'orbe occupe au centre un carré de la taille réglée (× croissance).
  function winSide() {
    return Math.max(24, Math.min(window.innerWidth, window.innerHeight));
  }
  function baseSide() {
    return Math.max(24, Math.min(winSide(), Number(config.size) || 80));
  }
  function resize() {
    dpr = window.devicePixelRatio || 1;
    win = winSide();
    winW = win;
    winH = win;
    canvas.style.left = '';
    canvas.style.top = '';
    canvas.style.transform = '';
    canvas.style.width = `${win}px`;
    canvas.style.height = `${win}px`;
    canvas.width = Math.floor(win * dpr);
    canvas.height = Math.floor(win * dpr);
  }
  function setOrbSide(s) {
    if (Math.abs(s - W) < 0.5) return;
    W = s;
    H = s;
    document.documentElement.style.setProperty('--s', `${Math.round(s)}px`);
  }

  // v2 : niveau du micro et état de la voix relayés par le noyau (plus de lien direct au service
  // vocal, qui n'accepte d'ailleurs plus que le noyau). Jamais les phrases.
  function connect() {}
  if (bridge) {
    bridge.onTaskEvent((ev) => {
      if (ev.type === 'level') levelTarget = Math.max(0, Math.min(1, ev.value));
      else if (ev.type === 'voice_wake' || (ev.type === 'voice_partial' && /\bjarvis\b/i.test(ev.text || ''))) wakeUntil = performance.now() + 2500;
      else if (ev.type === 'voice_state') {
        voice.connected = ev.connected !== false && !!ev.ready;
        voice.capturing = !!ev.capturing;
        voice.speaking = !!ev.speaking;
        voice.muted = !!ev.muted;
      }
    });
  }

  if (bridge) {
    bridge.onTaskEvent((ev) => {
      if (ev.type === 'create') tasks.set(ev.id, ev.status || 'running');
      else if (ev.type === 'update' && ev.status && tasks.has(ev.id)) tasks.set(ev.id, ev.status);
      else if (ev.type === 'finish') {
        tasks.delete(ev.id);
        if (ev.status === 'error') lastErrorAt = performance.now();
      } else if ((ev.type === 'theme_config' || ev.type === 'status') && ev.theme && ev.theme !== theme) {
        theme = ev.theme;
        if (J.setOrbTheme) J.setOrbTheme(theme);
      } else if (ev.type === 'perf_config') {
        const wanted = ev.voiceListening !== false;
        if (wanted !== voiceEnabled) {
          voiceEnabled = wanted;
          if (!wanted) { if (socket) { try { socket.close(); } catch {} } } else connect();
        }
      } else if (ev.type === 'widget_config') {
        config = { ...config, ...ev };
        resize();
        // v2 : Tauri n'a pas d'opacité de fenêtre ; c'est la page transparente qui s'estompe.
        document.body.style.opacity = String(config.opacity || 0.9);
        document.body.classList.toggle('ghost', Boolean(config.ghost));
        if (config.ghost) setHover(false);
      }
    });
  }

  // --- Compagnon de bureau (option de l'administration) -----------------------------------------
  // Actif : le personnage choisi (ou le cœur holographique) vit dans le widget selon son stade et sa
  // branche ; Jarvis garde la main (ses gestes passent devant, ui/character-director.js). Inactif : rien ne change.
  let heartStyle = localStorage.getItem('jarvis.heartStyle') || 'classic';
  let companion = null;
  let note = null; // { text, until } : annonce du compagnon (éclosion, évolution…)
  let dodging = false; // Ctrl maintenu : le widget s'efface et laisse passer les clics
  const companionOn = () => Boolean(companion && companion.enabled);
  if (bridge) {
    bridge.onTaskEvent((ev) => {
      if (ev.type === 'companion') companion = ev;
      else if (ev.type === 'companion_note' && ev.text) note = { text: ev.text, until: performance.now() + 7000 };
      else if (ev.type === 'heart_style_config' && ev.style) heartStyle = ev.style;
      else if (ev.type === 'status' && ev.heartStyle) heartStyle = ev.heartStyle;
    });
    if (bridge.call) bridge.call('compagnon', 'etat').then((v) => { if (v && v.stage) companion = v; }).catch(() => {});
    // Fichier déposé sur le widget (seul le cœur reçoit la souris) : bridge.js n'en garde que l'extension.
    if (bridge.fileDropZone) bridge.fileDropZone(() => companionOn());
    if (bridge.onShell) bridge.onShell('widget-dodge', (on) => { dodging = Boolean(on); });
  }

  // Pancarte du moment (jamais à la voix).
  function companionSign() {
    if (note && performance.now() < note.until) return note.text;
    const fx = new Set(companion.effects || []);
    if (fx.has('meeting')) return 'Chut, en réunion';
    if (fx.has('night')) return 'Va dormir !';
    if (fx.has('stealth') || fx.has('asleep')) return '';
    if (fx.has('offline')) return 'Plus de réseau…';
    if (fx.has('lowbattery')) return 'Batterie faible';
    if (fx.has('hungry')) return 'J’ai faim';
    return '';
  }

  // Caresse : le curseur fait le tour du cœur en moins de 3 s.
  const caress = { angle: null, turned: 0, since: 0 };
  function trackCaress(x, y) {
    if (!companionOn() || drag || !bridge) return;
    const a = Math.atan2(y - window.innerHeight / 2, x - window.innerWidth / 2);
    const now = performance.now();
    if (caress.angle === null || now - caress.since > 3000) {
      Object.assign(caress, { angle: a, turned: 0, since: now });
      return;
    }
    let d = a - caress.angle;
    if (d > Math.PI) d -= Math.PI * 2;
    if (d < -Math.PI) d += Math.PI * 2;
    caress.angle = a;
    caress.turned += Math.abs(d);
    if (caress.turned > Math.PI * 2) {
      Object.assign(caress, { turned: 0, since: now });
      bridge.call('compagnon', 'caresse').catch(() => {});
    }
  }

  // Double-clic du bouton du milieu : « ne pas déranger » (il dort jusqu'à nouvel ordre), ou réveil.
  let middleAt = 0;
  canvas.addEventListener('auxclick', (e) => {
    if (e.button !== 1 || !companionOn() || !inCore(e.clientX, e.clientY) || !bridge) return;
    const now = performance.now();
    if (now - middleAt < 450) {
      middleAt = 0;
      bridge.call('compagnon', 'veille', { on: !companion.asleep }).catch(() => {});
    } else middleAt = now;
  });

  // Promenade : au repos depuis 2 minutes, le compagnon longe le bas de l'écran ; dès que Jarvis s'active
  // (ou qu'on approche la souris), il revient à sa place. Pas en Bureau JARVIS (desk.rs range déjà le widget).
  const WALK_AFTER_MS = 120000;
  const walk = { home: null, x: 0, y: 0, dir: 1, idleSince: performance.now(), pauseUntil: 0, starting: false };
  const CALM_BLOCKERS = ['stealth', 'asleep', 'afk', 'meeting'];
  function walkHome() {
    if (walk.home && bridge && bridge.widgetWalk) bridge.widgetWalk(walk.home[0], walk.home[1]);
    walk.home = null;
  }
  setInterval(async () => {
    const now = performance.now();
    const allowed = companionOn() && companion.config && companion.config.walk !== false && bridge && bridge.widgetWalk && bridge.widgetPosition
      && !(bridge.deskEnabled && bridge.deskEnabled()) && !fly && !drag && !hovering && windowVisible;
    const calm = allowed && companion.stage !== 'oeuf' && tasks.size === 0 && !voice.capturing && !voice.speaking
      && currentMood(now) === 'idle' && !(companion.effects || []).some((e) => CALM_BLOCKERS.includes(e));
    if (!calm) {
      walk.idleSince = now;
      if (walk.home) walkHome();
      return;
    }
    if (!walk.home) {
      if (now - walk.idleSince < WALK_AFTER_MS || walk.starting) return;
      walk.starting = true;
      const pos = await bridge.widgetPosition().catch(() => null);
      walk.starting = false;
      if (!pos) return;
      walk.home = pos;
      [walk.x, walk.y] = pos;
    }
    if (now < walk.pauseUntil) return;
    const sw = window.screen;
    const left = sw.availLeft || 0;
    const right = left + (sw.availWidth || 1280) - win;
    const floor = (sw.availTop || 0) + (sw.availHeight || 720) - win + Math.round(win * 0.18);
    // Il descend d'abord jusqu'à la barre des tâches, puis marche ; une petite pause de temps en temps.
    walk.y += (floor - walk.y) * 0.12;
    if (Math.abs(floor - walk.y) < 2) {
      walk.x += walk.dir * 2;
      if (walk.x < left || walk.x > right) {
        walk.dir = -walk.dir;
        walk.x = Math.max(left, Math.min(right, walk.x));
      }
      if (Math.random() < 0.004) walk.pauseUntil = now + 2000 + Math.random() * 4000;
      if (Math.random() < 0.002) walk.dir = -walk.dir;
    }
    bridge.widgetWalk(walk.x, walk.y);
  }, 60);

  // Cœur holographique en compagnon : coque d'œuf dessinée en 2D, puis le signe de sa branche.
  const BRANCH_GLYPH = { gamer: '⚔', dev: '☕', artiste: '✎', chill: '♫' };
  function drawClassicCompanion(cx, cy, R, t) {
    const c = companion;
    if (c.stage === 'oeuf') {
      const warm = Math.min(1, (c.warmth || 0) / 100);
      g.save();
      g.translate(cx, cy + R * 0.1);
      g.rotate(Math.sin(t * (1.5 + warm * 4)) * (0.03 + warm * 0.12));
      const grd = g.createRadialGradient(-R * 0.2, -R * 0.35, R * 0.1, 0, 0, R * 1.1);
      grd.addColorStop(0, 'rgba(240,250,255,0.95)');
      grd.addColorStop(1, 'rgba(120,200,235,0.85)');
      g.fillStyle = grd;
      g.beginPath();
      g.ellipse(0, 0, R * 0.62, R * 0.82, 0, 0, Math.PI * 2);
      g.fill();
      if (warm > 0.4) {
        g.strokeStyle = `rgba(20,30,45,${(warm - 0.4) / 0.6})`;
        g.lineWidth = Math.max(1, R * 0.04);
        g.beginPath();
        g.moveTo(-R * 0.4, -R * 0.1);
        for (let i = 0; i < 5; i++) g.lineTo(-R * 0.4 + (i + 1) * R * 0.16, -R * 0.1 + (i % 2 ? R * 0.12 : -R * 0.02));
        g.stroke();
      }
      g.restore();
      return;
    }
    const glyph = BRANCH_GLYPH[c.branch || ''];
    if (glyph && c.stage !== 'bebe') {
      g.font = `${Math.round(R * 0.5)}px Segoe UI Symbol, sans-serif`;
      g.textAlign = 'center';
      g.fillStyle = 'rgba(230, 250, 255, 0.9)';
      g.fillText(glyph, cx + R * 0.72, cy + R * 0.85);
    }
  }

  // --- Souris : seul le cœur réagit ---------------------------------------------------------------

  // Zone cliquable à la taille réglée (celle que suit la coquille), même quand l'orbe a grossi.
  function inCore(x, y) {
    const r = baseSide() * CORE_RATIO;
    return Math.hypot(x - window.innerWidth / 2, y - window.innerHeight / 2) <= r;
  }

  function setHover(on) {
    if (on === hovering) return;
    hovering = on;
    document.body.classList.toggle('hover', on);
    // Le processus principal cesse d'ignorer la souris tant que le cœur est survolé.
    if (bridge && bridge.widgetHover) bridge.widgetHover(on);
  }

  window.addEventListener('mousemove', async (e) => {
    if (drag) {
      const dx = e.screenX - drag.sx;
      const dy = e.screenY - drag.sy;
      if (!drag.moved && Math.hypot(dx, dy) > 3) drag.moved = true;
      if (drag.moved && drag.origin && bridge && bridge.widgetMove) bridge.widgetMove(drag.origin[0] + dx, drag.origin[1] + dy);
      return;
    }
    if (!config.ghost) setHover(inCore(e.clientX, e.clientY));
    if (hovering) trackCaress(e.clientX, e.clientY);
  });

  document.addEventListener('mouseleave', () => {
    if (!drag) setHover(false);
  });

  canvas.addEventListener('mousedown', async (e) => {
    if (e.button !== 0 || !inCore(e.clientX, e.clientY)) return;
    drag = { sx: e.screenX, sy: e.screenY, origin: null, moved: false };
    document.body.classList.add('dragging');
    if (bridge && bridge.widgetPosition) drag.origin = await bridge.widgetPosition();
  });

  window.addEventListener('mouseup', () => {
    if (!drag) return;
    if (drag.moved && bridge && bridge.widgetMoved) bridge.widgetMoved();
    // Bureau JARVIS actif : un clic (sans glisser) sur le cœur du widget ramène au bureau JARVIS.
    if (!drag.moved && bridge && bridge.deskEnabled && bridge.deskEnabled() && bridge.deskGoto) bridge.deskGoto();
    drag = null;
    document.body.classList.remove('dragging');
  });

  canvas.addEventListener('dblclick', (e) => {
    if (!inCore(e.clientX, e.clientY)) return;
    // Bureau JARVIS : réduit les fenêtres qui cachent le HUD ; sinon, retour à l'interface complète.
    if (bridge && bridge.deskEnabled && bridge.deskEnabled() && bridge.deskReveal) bridge.deskReveal();
    else if (bridge && bridge.setWidget) bridge.setWidget(false);
  });

  canvas.addEventListener('contextmenu', (e) => {
    e.preventDefault();
    if (inCore(e.clientX, e.clientY) && bridge && bridge.widgetMenu) bridge.widgetMenu();
  });

  // --- Rendu ---------------------------------------------------------------------------------------

  // Une seule passe sur les tâches par image (avant : deux copies de la Map par image).
  let hasAwaiting = false;
  let hasWaitingReply = false;
  function scanTasks() {
    hasAwaiting = false;
    hasWaitingReply = false;
    for (const status of tasks.values()) {
      if (status === 'awaiting') hasAwaiting = true;
      else if (status === 'waiting_reply') hasWaitingReply = true;
    }
  }

  function currentMood(now) {
    if (now - lastErrorAt < 5000) return 'error';
    if (hasAwaiting) return 'awaiting';
    if (voice.capturing || voice.speaking || hasWaitingReply) return 'listening';
    if (tasks.size) return 'working';
    return 'idle';
  }

  // Grand widget (≥ 110 px) : une étiquette d'état sous l'orbe. À 80 px, l'orbe seule suffit :
  // sa couleur et son mouvement disent l'état (écoute, travail, validation, erreur).
  function label() {
    const count = tasks.size;
    if (!voiceEnabled) return ['MODE TEXTE', 'rgba(255, 196, 64, 0.8)'];
    if (!voice.connected) return ['VOIX HORS LIGNE', 'rgba(255, 77, 94, 0.8)'];
    if (hasAwaiting) return ['VALIDATION', 'rgba(255, 176, 32, 0.95)'];
    if (voice.muted) return ['MICRO COUPÉ', 'rgba(255, 176, 32, 0.8)'];
    if (voice.capturing) return ['ÉCOUTE', 'rgba(255, 190, 90, 0.95)'];
    if (voice.speaking) return ['PARLE', 'rgba(126, 242, 226, 0.95)'];
    if (count) return [`${count} TÂCHE${count > 1 ? 'S' : ''}`, 'rgba(160, 245, 255, 0.85)'];
    return ['JARVIS', 'rgba(160, 245, 255, 0.6)'];
  }

  // Mêmes couleurs de conversation que l'interface : rouge > bleu > vert > jaune.
  function conversation(now) {
    const mood = currentMood(now);
    if (voice.capturing && now < wakeUntil) wakeUntil = now + 2500; // la phrase continue
    const running = [...tasks.values()].some((s) => s === 'running');
    const talking = voice.capturing && level > 0.03;
    const voicePhase = voice.speaking ? 'reply'
      : (now < wakeUntil && (talking || !running)) ? 'wake'
        : talking ? 'user'
          : running ? 'think' : null;
    return { mood, running, talking, voicePhase };
  }

  // Halo sombre : laisse voir (et cliquer) ce qu'il y a derrière. Rayon extérieur 0,472 × côté de l'orbe,
  // comme l'ancien calque CSS ; `a` : opacité (0 à 1).
  const HALO_R = 0.472;
  function drawHalo(cx, cy, r, a) {
    if (a <= 0.01 || r < 1) return;
    const grad = g.createRadialGradient(cx, cy, 0, cx, cy, r);
    grad.addColorStop(0, `rgba(0, 12, 22, ${0.55 * a})`);
    grad.addColorStop(0.733, `rgba(0, 12, 22, ${0.32 * a})`);
    grad.addColorStop(1, 'rgba(0, 12, 22, 0)');
    g.save();
    g.fillStyle = grad;
    g.beginPath();
    g.arc(cx, cy, r, 0, Math.PI * 2);
    g.fill();
    g.restore();
  }

  // --- Cœur unique (shell/src/desk.rs) ---------------------------------------------------------------
  // Une application recouvre le cœur du HUD : CE cœur-là rétrécit et glisse jusqu'au widget (dir « out »),
  // puis grossit et revient quand la place se libère (dir « in »). La coquille agrandit la fenêtre le temps
  // du trajet (rectangle englobant départ et arrivée) ; tout est dessiné ici, avec une seule horloge.
  // Passage de relais : 1re image peinte à la taille exacte du cœur du HUD → heartReady (le HUD fond le sien) ;
  // arrivée → canvas effacé, heartLanded ; la coquille rend alors la fenêtre à sa taille (heart-settled).
  // Trace du dernier trajet (position, rayon, avancement), lue par tools/test-shell.js et le débogage.
  const flightTrace = (window.__flightTrace = []);
  const easeInOut = (p) => (p < 0.5 ? 4 * p * p * p : 1 - Math.pow(-2 * p + 2, 3) / 2);
  const lerp = (a, b, k) => a + (b - a) * k;

  // Position et rayon du petit cœur à l'arrêt (mise en page normale), dans le repère de la fenêtre agrandie.
  function restGeometry(cxWin, cyWin, small) {
    const side = Math.min(win || winSide(), baseSide() * grow);
    return { x: cxWin, y: cyWin + (small ? 0 : side * (0.44 - 0.5)), r: side * (small ? 0.3 : 0.26) };
  }

  function startFly(p) {
    const k = 1 / (window.devicePixelRatio || 1); // la coquille parle en pixels physiques
    const small = baseSide() < 110;
    const inward = p.dir === 'in';
    dpr = window.devicePixelRatio || 1;
    winW = Math.round(p.box.w * k);
    winH = Math.round(p.box.h * k);
    canvas.style.left = '0';
    canvas.style.top = '0';
    canvas.style.transform = 'none';
    canvas.style.width = `${winW}px`;
    canvas.style.height = `${winH}px`;
    canvas.width = Math.floor(winW * dpr); // efface aussi toute image rémanente
    canvas.height = Math.floor(winH * dpr);
    const fromRest = restGeometry(p.from.x * k, p.from.y * k, small);
    const toRest = restGeometry(p.to.x * k, p.to.y * k, small);
    fly = {
      inward,
      t0: performance.now(),
      dur: Math.max(300, Number(p.durMs) || 900),
      from: inward ? fromRest : { x: p.from.x * k, y: p.from.y * k, r: p.from.r * k },
      to: inward ? { x: p.to.x * k, y: p.to.y * k, r: p.to.r * k } : toRest,
      ready: false,
      landing: false
    };
    if (hovering) setHover(false);
    flightTrace.length = 0;
    flightTrace.push(['départ', p.dir, fly.from, fly.to, fly.dur]);
    // 1re image tout de suite (la fenêtre vient d'être montrée) : le HUD ne cède son cœur qu'ensuite.
    const now = performance.now();
    scanTasks();
    const c = conversation(now);
    g.setTransform(dpr, 0, 0, dpr, 0, 0);
    drawFlight(now, 0.016, c.mood, c.voicePhase, c.talking, small);
    if (bridge && bridge.heartReady) bridge.heartReady();
    if (!loopRunning) {
      loopRunning = true;
      last = performance.now();
      requestAnimationFrame(frame);
    }
  }

  function drawFlight(now, dt, mood, voicePhase, talking, small) {
    const f = fly;
    if (!f || f.landing) return;
    const p = Math.min(1, (now - f.t0) / f.dur);
    const e = easeInOut(p);
    const s = f.inward ? 1 - e : e; // 0 : grand cœur du HUD, 1 : petit cœur du widget
    const x = lerp(f.from.x, f.to.x, e);
    const y = lerp(f.from.y, f.to.y, e);
    const r = lerp(f.from.r, f.to.r, e);
    const t = now / 1000;
    const ratio = small ? 0.3 : 0.26;
    if (flightTrace.length < 400) flightTrace.push([Math.round(now - f.t0), Math.round(x), Math.round(y), Math.round(r), Number(s.toFixed(2)), winW, winH, dpr]);
    const state = {
      dt,
      mood,
      voicePhase,
      userSpeaking: talking,
      loadFactor: Math.min(1, tasks.size * 0.35),
      levels,
      head,
      capturing: voice.capturing,
      speaking: voice.speaking,
      lite: r / ratio < 110
    };
    g.setTransform(dpr, 0, 0, dpr, 0, 0);
    g.clearRect(0, 0, winW, winH);
    drawHalo(x, y, r * (HALO_R / ratio), s); // le halo n'existe pas dans le HUD : il apparaît en rétrécissant
    g.save();
    g.globalAlpha = lerp(1, presence, s); // pleine opacité comme le HUD, puis celle du widget à l'arrivée
    const R = J.computeOrbRadius ? J.computeOrbRadius(r, t, level, state) : r;
    J.drawOrb(g, x, y, R, t, level, state);
    g.restore();
    if (p >= 1) {
      f.landing = true;
      if (!f.inward) g.clearRect(0, 0, winW, winH); // la fenêtre va changer de taille : pas d'image décalée
      if (bridge && bridge.heartLanded) bridge.heartLanded();
    }
  }

  // Arrivée confirmée par la coquille : la fenêtre a repris sa taille (départ) ou va être masquée (retour).
  function endFly(p) {
    const inward = p && p.dir === 'in';
    g.setTransform(dpr, 0, 0, dpr, 0, 0);
    g.clearRect(0, 0, winW, winH);
    fly = null;
    resize();
    if (inward) {
      loopRunning = false; // fenêtre sur le point d'être masquée : rien à redessiner
      return;
    }
    if (!loopRunning) {
      loopRunning = true;
      last = performance.now();
      requestAnimationFrame(frame);
    }
  }

  window.__heartFly = startFly; // crochet de test (tools/test-shell.js) : joue un trajet sans la coquille
  if (bridge && bridge.onHeartFly) bridge.onHeartFly(startFly);
  if (bridge && bridge.onHeartSettled) bridge.onHeartSettled(endFly);

  let last = performance.now();
  // Au repos, réveil par minuteur (~12 images/s) plutôt qu'au rythme de l'écran : requestAnimationFrame
  // se déclenche 60 à 144 fois par seconde même quand on n'a rien à dessiner.
  const IDLE_FRAME_MS = 70;

  // Widget masqué (retour à l'interface complète) : plus aucun dessin. La fenêtre garde
  // backgroundThrottling: false, donc l'arrêt doit venir d'ici (jarvis:visibility, voir main.js).
  let windowVisible = true;
  let loopRunning = true;
  if (bridge && bridge.onVisibility) {
    bridge.onVisibility((visible) => {
      windowVisible = visible;
      // Masquée : canvas vidé, pour qu'aucune dernière image ne reparaisse au prochain affichage.
      if (!visible && !fly) { g.setTransform(1, 0, 0, 1, 0, 0); g.clearRect(0, 0, canvas.width, canvas.height); }
      if (visible && !loopRunning) {
        loopRunning = true;
        last = performance.now();
        requestAnimationFrame(frame);
      }
    });
  }

  function frame(now) {
    if (!windowVisible) {
      loopRunning = false;
      return;
    }
    // Cœur posé, en attente de l'arrivée (fenêtre en train de reprendre sa taille) : rien n'est dessiné.
    if (fly && fly.landing) {
      loopRunning = false;
      return;
    }
    if (!fly && win !== winSide()) resize();

    scanTasks();
    const { mood, running, talking, voicePhase } = conversation(now);
    // Petit au repos ; il grossit quand Jarvis intervient (écoute, parle, travaille, attend une validation).
    const growTarget = voicePhase || hasAwaiting ? GROW : 1;
    const growing = Math.abs(growTarget - grow) > 0.005;
    const busy = tasks.size > 0 || voice.capturing || voice.speaking || level > 0.04;
    // Au repos, ~12 images par seconde suffisent (le widget reste à l'écran pendant les jeux) :
    // l'orbe de 80 px ne bouge presque pas, et le fondu de présence reste fluide.
    // Compagnon en mode furtif (jeu, plein écran) : 5 images par seconde.
    const stealth = companionOn() && (companion.effects || []).includes('stealth');
    if (!busy && !hovering && !growing && !fly) setTimeout(() => requestAnimationFrame(frame), stealth ? 200 : IDLE_FRAME_MS);
    else requestAnimationFrame(frame);

    const dt = Math.min(0.05, (now - last) / 1000);
    last = now;
    level += (levelTarget - level) * Math.min(1, dt * 14);
    levels[head] = level;
    head = (head + 1) % levels.length;
    grow += (growTarget - grow) * Math.min(1, dt * 3);
    const base = baseSide();
    setOrbSide(Math.min(win, base * grow));

    // Discrète au repos ; pleine quand il se passe quelque chose ou que le cœur est survolé.
    let wanted = hovering || busy || mood !== 'idle' ? 1 : 0.55;
    if (companionOn() && !busy && mood === 'idle') wanted = stealth ? 0.25 : Math.max(wanted, 0.85);
    if (dodging) wanted = 0.2;
    presence += (wanted - presence) * Math.min(1, dt * 4);

    const small = base < 110; // d'après la taille réglée : pas de changement de mise en page en grossissant
    g.setTransform(dpr, 0, 0, dpr, 0, 0);
    g.clearRect(0, 0, winW, winH);
    if (fly) {
      drawFlight(now, dt, mood, voicePhase, talking, small);
      return;
    }
    // L'orbe est dessinée dans son carré (W × H), centré dans la fenêtre.
    const off = (win - W) / 2;
    g.translate(off, off);
    const cx = W / 2;
    const cy = small ? H / 2 : H * 0.44;
    const baseR = Math.min(W, H) * (small ? 0.3 : 0.26);
    const t = now / 1000;
    // Halo sombre sous l'orbe (dessiné ici, avec elle : pas de calque CSS qui resterait seul à l'écran).
    drawHalo(W / 2, H / 2, W * HALO_R, config.ghost ? 0.45 : 1);
    const state = {
      dt,
      mood,
      voicePhase,
      userSpeaking: talking,
      loadFactor: Math.min(1, tasks.size * 0.35),
      levels,
      head,
      capturing: voice.capturing,
      speaking: voice.speaking,
      lite: W < 110 // à 80 px, la vie interne du cœur (blobs, anneaux ondulants) est invisible
    };
    if (companionOn()) {
      state.heartStyle = heartStyle;
      state.companion = companion;
      state.musicPlaying = (companion.effects || []).includes('music');
    }
    g.save();
    g.globalAlpha = presence;
    const R = J.computeOrbRadius ? J.computeOrbRadius(baseR, t, level, state) : baseR;
    J.drawOrb(g, cx, cy, R, t, level, state);
    const drawn3d = companionOn() && heartStyle !== 'classic' && J.character && J.character.ready(heartStyle);
    if (companionOn() && !drawn3d) drawClassicCompanion(cx, cy, R, t);
    g.restore();
    if (companionOn()) {
      const text = companionSign();
      if (text) {
        g.font = `600 ${small ? 9 : 10}px ${MONO}`;
        g.letterSpacing = '0px';
        g.textAlign = 'center';
        const w = g.measureText(text).width + 10;
        g.fillStyle = 'rgba(8, 16, 26, 0.82)';
        g.beginPath();
        g.roundRect(cx - w / 2, 2, w, small ? 14 : 16, 7);
        g.fill();
        g.fillStyle = 'rgba(220, 245, 255, 0.95)';
        g.fillText(text, cx, small ? 12 : 13);
      }
    }

    // Anneau de survol : indique que le cœur est cliquable.
    if (hovering) {
      g.beginPath();
      g.arc(cx, cy, base * CORE_RATIO, 0, Math.PI * 2);
      g.strokeStyle = 'rgba(160, 245, 255, 0.35)';
      g.lineWidth = 1;
      g.stroke();
    }

    if (!small) {
      const [text, color] = label();
      g.font = `600 10px ${MONO}`;
      g.letterSpacing = '2px';
      g.textAlign = 'center';
      g.fillStyle = color;
      g.fillText(text, cx, H - 12);
    } else if ((voiceEnabled && !voice.connected) || mood === 'awaiting') {
      // À 80 px : un point d'alerte discret (voix hors ligne en rouge, validation en orange).
      g.beginPath();
      g.arc(W - 10, 10, 3.5, 0, Math.PI * 2);
      g.fillStyle = mood === 'awaiting' ? 'rgba(255, 176, 32, 0.95)' : 'rgba(255, 77, 94, 0.9)';
      g.fill();
    }
  }

  resize();
  requestAnimationFrame(frame);
})();
