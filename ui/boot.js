// Séquence d'allumage du HUD : le cœur s'allume, puis ses anneaux et ses traits se dessinent au fil
// des VRAIES étapes du lancement (événements que le noyau envoie à la connexion, voir
// core/main.js › gateway.onConnect), enfin l'interface se déploie élément par élément.
//
//   ignite     → le point de lumière devient le cœur (onde de choc)
//   steps      → chaque étape validée trace sa couche du cœur ; la liste s'écrit sous le cœur
//   interface  → fond, en-tête, panneaux, cards, widgets, matériel, puis la zone de saisie
//   ready      → tout est visible, J.boot ne touche plus au rendu
//
// Hors coquille (page ouverte dans un navigateur), les étapes se valident sur un faux calendrier.
(function () {
  const J = (window.JARVIS = window.JARVIS || {});
  const bridge = window.jarvisBridge;
  const MONO = 'Consolas, "Courier New", monospace';

  const clamp01 = (x) => Math.max(0, Math.min(1, x));
  const ease = {
    outCubic: (x) => 1 - Math.pow(1 - clamp01(x), 3),
    outBack: (x) => {
      const p = clamp01(x) - 1;
      return 1 + 2.70158 * p * p * p + 1.70158 * p * p;
    },
    inOutCubic: (x) => {
      const p = clamp01(x);
      return p < 0.5 ? 4 * p * p * p : 1 - Math.pow(-2 * p + 2, 3) / 2;
    }
  };

  const IGNITE_MS = 650;
  const STEP_MIN_MS = 380; // une étape reste affichée au moins ce temps avant la suivante
  const DRAW_MS = 520; // tracé de la couche du cœur liée à une étape
  const GRACE_MS = 1500; // donnée secondaire (journal, périphérique audio) : on ne l'attend pas plus
  const GIVE_UP_MS = 15000; // au-delà, les étapes manquantes sont passées (jamais bloquer l'utilisateur)
  const UI_DELAY_MS = 250;

  // Cascade de l'interface : [début, durée] en ms après le début de l'acte 3.
  const UI_TIMELINE = {
    steps: [0, 380], // la liste des étapes est aspirée dans le cœur
    hud: [0, 700],
    header: [150, 450],
    clock: [250, 400],
    controls: [300, 500],
    'panel:transcript': [350, 420],
    'panel:memory': [460, 420],
    'panel:status': [570, 420],
    'panel:journal': [680, 420],
    cards: [500, 550],
    widgets: [650, 600],
    hardware: [800, 450],
    console: [900, 700]
  };
  const UI_TOTAL_MS = 1650;
  // Classes posées sur <body> au fil de la cascade (animations CSS des éléments HTML).
  const DOM_STEPS = [['controls', 'boot-controls'], ['widgets', 'boot-widgets'], ['console', 'boot-console']];

  const STEPS = [
    { id: 'core', pending: 'LIAISON AU NOYAU', label: 'NOYAU RELIÉ' },
    { id: 'orchestrator', pending: 'ORCHESTRATEUR', label: 'ORCHESTRATEUR EN LIGNE' },
    { id: 'memory', pending: 'MÉMOIRE & JOURNAL', label: 'MÉMOIRE & JOURNAL CHARGÉS' },
    { id: 'settings', pending: 'RÉGLAGES & AUDIO', label: 'RÉGLAGES & AUDIO APPLIQUÉS' },
    { id: 'voice', pending: 'MOTEUR VOCAL', label: 'MOTEUR VOCAL OPÉRATIONNEL' }
  ];
  for (const s of STEPS) {
    s.signaled = false;
    s.doneAt = 0;
    s.warn = false; // validée « par défaut » (ambre) : absente, désactivée ou délai dépassé
  }
  const stepById = (id) => STEPS.find((s) => s.id === id);

  const state = {
    phase: 'ignite',
    startedAt: 0, // première image dessinée (pas le chargement du script)
    cursor: 0, // prochaine étape à afficher validée
    lastDoneAt: 0,
    uiStart: 0,
    readyAt: 0,
    speed: 1, // > 1 : séquence raccourcie (mode éco, cœur simplifié)
    dom: new Set()
  };

  // --- Signaux réels ------------------------------------------------------------------------
  const seen = new Map(); // type d'événement → heure de première réception
  let voiceBackend = null;

  function signal(id, warnLabel) {
    const s = stepById(id);
    if (!s || s.signaled) return;
    s.signaled = true;
    if (warnLabel) {
      s.warn = true;
      s.label = warnLabel;
    }
  }

  function checkSignals(now) {
    if (seen.size) signal('core');
    if (seen.has('status')) signal('orchestrator');
    const graceOk = (main, extra) => seen.has(main) && (seen.has(extra) || now - seen.get(main) > GRACE_MS);
    if (graceOk('memory', 'journal')) signal('memory');
    if (graceOk('perf_config', 'audio_device')) signal('settings');

    const v = J.voice;
    if (v && v.ready && v.connected) signal('voice');
    else if (v && v.disabled) signal('voice', 'ÉCOUTE ORALE DÉSACTIVÉE');
    else if (voiceBackend === 'unavailable' || voiceBackend === 'failed') signal('voice', 'MOTEUR VOCAL ABSENT · MODE CLAVIER');

    if (state.startedAt && now - state.startedAt > GIVE_UP_MS) {
      for (const s of STEPS) {
        if (!s.signaled) signal(s.id, s.id === 'voice' ? 'MODE CLAVIER · VOIX EN ATTENTE' : `${s.pending} · NON CONFIRMÉ`);
      }
    }
  }

  if (bridge) {
    bridge.onTaskEvent((ev) => {
      if (!ev || !ev.type) return;
      if (!seen.has(ev.type)) seen.set(ev.type, performance.now());
      if (ev.type === 'voice_backend') voiceBackend = ev.state;
    });
    // Lancé masqué (mode widget) : pas de spectacle à rattraper au premier affichage.
    if (bridge.onVisibility) bridge.onVisibility((visible) => { if (!visible) finish(performance.now()); });
  } else {
    // Démonstration hors coquille : faux calendrier des étapes.
    const DEMO = { core: 250, orchestrator: 650, memory: 1000, settings: 1350, voice: 2300 };
    for (const [id, at] of Object.entries(DEMO)) setTimeout(() => signal(id), at + IGNITE_MS);
  }

  // --- Machine d'états --------------------------------------------------------------------------
  function startInterface(now) {
    state.phase = 'interface';
    state.uiStart = now + UI_DELAY_MS / state.speed;
    if (J.boot.onInterface) J.boot.onInterface();
  }

  function finish(now) {
    if (state.phase === 'ready') return;
    for (const s of STEPS) if (!s.doneAt) s.doneAt = now - DRAW_MS;
    state.phase = 'ready';
    state.readyAt = now;
    document.body.classList.remove('booting');
    // Les classes d'animation sont retirées une fois les animations finies : un élément réaffiché
    // plus tard ne doit pas rejouer son entrée.
    setTimeout(() => {
      for (const [, cls] of DOM_STEPS) document.body.classList.remove(cls);
    }, 800);
    if (J.boot.onReady) J.boot.onReady();
  }

  function update(now) {
    if (state.phase === 'ready') return;
    if (!state.startedAt) {
      state.startedAt = now;
      state.lastDoneAt = now;
    }
    checkSignals(now);
    const k = state.speed;

    if (state.phase === 'ignite') {
      if (now - state.startedAt >= IGNITE_MS / k) {
        state.phase = 'steps';
        state.lastDoneAt = now;
      }
      return;
    }

    if (state.phase === 'steps') {
      const next = STEPS[state.cursor];
      if (next && next.signaled && now - state.lastDoneAt >= STEP_MIN_MS / k) {
        next.doneAt = now;
        state.lastDoneAt = now;
        state.cursor++;
      }
      if (state.cursor >= STEPS.length && now - state.lastDoneAt >= DRAW_MS / k) startInterface(now);
      return;
    }

    // Acte 3 : classes CSS au bon moment, puis fin.
    for (const [name, cls] of DOM_STEPS) {
      if (!state.dom.has(cls) && ui(name) > 0) {
        state.dom.add(cls);
        document.body.classList.add(cls);
        if (name === 'console') typePlaceholder(UI_TIMELINE.console[1] / k);
      }
    }
    if (now - state.uiStart >= UI_TOTAL_MS / k) finish(now);
  }

  // Une touche ou un clic : l'acte en cours se termine tout de suite.
  function skip() {
    const now = performance.now();
    if (state.phase === 'ignite' || state.phase === 'steps') {
      if (!state.startedAt) state.startedAt = now;
      for (const s of STEPS) {
        if (!s.signaled) signal(s.id, s.id === 'voice' ? 'MOTEUR VOCAL · EN ARRIÈRE-PLAN' : `${s.pending} · EN ARRIÈRE-PLAN`);
        if (!s.doneAt) s.doneAt = now - DRAW_MS;
      }
      state.cursor = STEPS.length;
      startInterface(now - UI_DELAY_MS / state.speed);
    } else if (state.phase === 'interface') {
      for (const [, cls] of DOM_STEPS) document.body.classList.add(cls);
      finish(now);
    }
  }

  // --- Progressions lues par le rendu -------------------------------------------------------------
  function stepProgress(id, now) {
    const s = stepById(id);
    return s && s.doneAt ? ease.outCubic((now - s.doneAt) / (DRAW_MS / state.speed)) : 0;
  }
  function flash(id, now) {
    const s = stepById(id);
    if (!s || !s.doneAt) return 0;
    return Math.max(0, 1 - (now - s.doneAt) / 650);
  }

  // Progression 0 → 1 d'un élément de l'interface (acte 3).
  function ui(name) {
    if (state.phase === 'ready') return 1;
    if (state.phase !== 'interface') return 0;
    const slot = UI_TIMELINE[name];
    if (!slot) return 1;
    const t = (performance.now() - state.uiStart) * state.speed;
    return clamp01((t - slot[0]) / slot[1]);
  }

  // Couches du cœur (orb.js lit cet objet ; null = tout visible).
  const reveal = {};
  function orbReveal(now) {
    if (state.phase === 'ready') return null;
    const ign = state.startedAt ? (now - state.startedAt) / (IGNITE_MS / state.speed) : 0;
    reveal.heart = clamp01(ign);
    reveal.heartScale = ease.outBack(ign);
    reveal.glow = stepProgress('core', now);
    reveal.blue = stepProgress('orchestrator', now);
    reveal.green = stepProgress('memory', now);
    reveal.ticks = stepProgress('settings', now);
    reveal.wave = stepProgress('voice', now);
    reveal.flashGlow = flash('core', now);
    reveal.flashBlue = flash('orchestrator', now);
    reveal.flashGreen = flash('memory', now);
    reveal.flashTicks = flash('settings', now);
    return reveal;
  }

  // Texte « décodé » : les caractères se figent de gauche à droite, le reste défile au hasard.
  const GLYPHS = 'ABCDEFGHJKLMNPRSTUVXYZ0123456789#%&*+<>/=';
  function decode(text, p) {
    const s = String(text);
    if (p >= 1) return s;
    if (p <= 0) return '';
    const fixed = Math.floor(s.length * p);
    let out = s.slice(0, fixed);
    for (let i = fixed; i < s.length; i++) {
      out += s[i] === ' ' ? ' ' : GLYPHS[(Math.random() * GLYPHS.length) | 0];
    }
    return out;
  }

  // Le texte d'aide de la zone de saisie s'écrit pendant son ouverture.
  function typePlaceholder(durationMs) {
    const input = document.getElementById('input');
    if (!input) return;
    const full = input.getAttribute('data-placeholder') || input.placeholder;
    input.setAttribute('data-placeholder', full);
    input.placeholder = '';
    const t0 = performance.now() + durationMs * 0.45;
    const tick = () => {
      const p = clamp01((performance.now() - t0) / (durationMs * 0.55));
      input.placeholder = full.slice(0, Math.round(full.length * p));
      if (p < 1 && state.phase !== 'ready') requestAnimationFrame(tick);
      else input.placeholder = full;
    };
    requestAnimationFrame(tick);
  }

  // --- Dessins propres à la séquence (par-dessus le cœur) -----------------------------------------
  function rgbOf(theme) {
    return (J.THEMES && J.THEMES[theme] && J.THEMES[theme].primaryRgb) || '56, 225, 255';
  }

  function shockwave(g, cx, cy, R, p, rgb) {
    if (p <= 0 || p >= 1) return;
    const e = ease.outCubic(p);
    g.save();
    g.globalCompositeOperation = 'lighter';
    g.lineWidth = 3.5 * (1 - p) + 0.5;
    g.strokeStyle = `rgba(${rgb}, ${(1 - p) * 0.85})`;
    g.beginPath();
    g.arc(cx, cy, R * (0.5 + e * 2.2), 0, Math.PI * 2);
    g.stroke();
    g.lineWidth = 10 * (1 - p);
    g.strokeStyle = `rgba(${rgb}, ${(1 - p) * 0.18})`;
    g.stroke();
    g.restore();
  }

  function drawOverlay(g, cx, cy, R, now, theme) {
    if (state.phase === 'ready' || !state.startedAt) return;
    const rgb = rgbOf(theme);
    const k = state.speed;
    const ign = (now - state.startedAt) / (IGNITE_MS / k);

    // Acte 1 : point de lumière qui pulse puis s'ouvre en cœur.
    if (ign < 1) {
      const spark = 1 - clamp01((ign - 0.35) / 0.65);
      const pulse = 0.7 + 0.3 * Math.sin(now / 45);
      const r = 3 + 10 * clamp01(ign * 2.2);
      g.save();
      g.globalCompositeOperation = 'lighter';
      const grd = g.createRadialGradient(cx, cy, 0, cx, cy, r * 4);
      grd.addColorStop(0, `rgba(255, 255, 255, ${0.95 * spark * pulse})`);
      grd.addColorStop(0.25, `rgba(${rgb}, ${0.6 * spark})`);
      grd.addColorStop(1, `rgba(${rgb}, 0)`);
      g.fillStyle = grd;
      g.beginPath();
      g.arc(cx, cy, r * 4, 0, Math.PI * 2);
      g.fill();
      // Fine croix de visée
      g.strokeStyle = `rgba(${rgb}, ${0.5 * spark})`;
      g.lineWidth = 1;
      const len = 30 + 90 * ease.outCubic(ign);
      g.beginPath();
      g.moveTo(cx - len, cy); g.lineTo(cx - 8, cy);
      g.moveTo(cx + 8, cy); g.lineTo(cx + len, cy);
      g.moveTo(cx, cy - len); g.lineTo(cx, cy - 8);
      g.moveTo(cx, cy + 8); g.lineTo(cx, cy + len);
      g.stroke();
      g.restore();
    }
    shockwave(g, cx, cy, R, (ign - 0.35) / 1.1, rgb);

    // Attente du moteur vocal : arc en pointillés qui cherche autour du cœur.
    const voice = stepById('voice');
    if (state.phase === 'steps' && state.cursor === STEPS.length - 1 && !voice.doneAt) {
      g.save();
      g.setLineDash([6, 9]);
      g.lineDashOffset = -now / 40;
      g.lineWidth = 1.4;
      g.strokeStyle = `rgba(255, 215, 90, ${0.35 + 0.2 * Math.sin(now / 260)})`;
      const a0 = now / 700;
      g.beginPath();
      g.arc(cx, cy, R * 1.12, a0, a0 + Math.PI * 1.3);
      g.stroke();
      g.restore();
    }

    // Acte 3 : seconde onde de choc au départ de la cascade.
    if (state.phase === 'interface') {
      shockwave(g, cx, cy, R, (now - state.uiStart) * k / 800, rgb);
    }

    drawSteps(g, cx, cy, R, now, rgb);
  }

  function drawSteps(g, cx, cy, R, now, rgb) {
    const out = state.phase === 'interface' ? ease.inOutCubic(ui('steps')) : 0;
    if (out >= 1) return;
    const ign = clamp01((now - state.startedAt) / (IGNITE_MS / state.speed));
    const alpha = Math.min(ign, 1 - out);
    if (alpha <= 0) return;

    g.save();
    g.textAlign = 'center';
    g.textBaseline = 'middle';

    // Titre et chrono au-dessus du cœur
    const titleY = cy - R * 1.32 - 26;
    g.globalAlpha = alpha;
    g.font = `700 11px ${MONO}`;
    g.letterSpacing = '3px';
    g.fillStyle = `rgba(${rgb}, 0.9)`;
    const elapsed = ((state.phase === 'interface' ? state.uiStart : now) - state.startedAt) / 1000;
    const done = STEPS.filter((s) => s.doneAt).length;
    g.fillText(`SÉQUENCE D'ALLUMAGE · ${done}/${STEPS.length}`, cx, titleY);
    g.font = `10px ${MONO}`;
    g.letterSpacing = '2px';
    g.fillStyle = 'rgba(190, 240, 255, 0.55)';
    g.fillText(`T+${elapsed.toFixed(2)}s`, cx, titleY + 16);

    // Liste des étapes sous le cœur : validées + celle en cours ; aspirée vers le cœur à la fin.
    const baseY = cy + R * 1.32 + 30;
    g.font = `11px ${MONO}`;
    g.letterSpacing = '1px';
    const visible = STEPS.slice(0, Math.min(STEPS.length, state.cursor + (state.phase === 'steps' ? 1 : 0)));
    visible.forEach((s, i) => {
      let y = baseY + i * 18;
      y += (cy - y) * out;
      const rowAlpha = alpha * (1 - out * 0.5);
      if (s.doneAt) {
        const typed = clamp01((now - s.doneAt) / 260);
        const fl = Math.max(0, 1 - (now - s.doneAt) / 420);
        const text = `[✓] ${s.label}`;
        const color = s.warn ? '255, 196, 64' : '53, 255, 158';
        g.globalAlpha = rowAlpha;
        g.fillStyle = fl > 0 ? `rgba(${fl > 0.5 ? '255, 255, 255' : color}, 0.95)` : `rgba(${color}, 0.85)`;
        g.fillText(decode(text, 0.25 + 0.75 * typed), cx, y);
      } else {
        const blink = 0.45 + 0.4 * (0.5 + 0.5 * Math.sin(now / 140));
        const dots = '.'.repeat(1 + (Math.floor(now / 300) % 3));
        g.globalAlpha = rowAlpha * blink;
        g.fillStyle = 'rgba(255, 215, 30, 0.9)';
        g.fillText(`[●] ${s.pending}${dots.padEnd(3, ' ')}`, cx, y);
      }
    });
    g.restore();
  }

  J.boot = {
    ease,
    clamp01,
    update,
    skip,
    finish: () => finish(performance.now()),
    isReady: () => state.phase === 'ready',
    phase: () => state.phase,
    orbReveal,
    ui,
    decode,
    drawOverlay,
    // Séquence raccourcie (mode éco, cœur simplifié) : mêmes étapes, deux fois plus vite.
    setLite(on) { state.speed = on ? 2 : 1; },
    onInterface: null,
    onReady: null
  };
})();
