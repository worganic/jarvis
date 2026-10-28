(function () {
  const J = (window.JARVIS = window.JARVIS || {});
  const TAU = Math.PI * 2;
  const rad = (d) => (d * Math.PI) / 180;

  // --- 7 Palettes d'humeur déduites de signaux réels ---------------------------
  // Transition douce sur ~600 ms pour éviter tout clignotement intempestif.
  const MOOD_PALETTES = {
    // 1. Repos : aucune tâche active, calme majestueux
    idle: {
      primary: [70, 130, 255],
      secondary: [40, 95, 210],
      glow: [50, 110, 240],
      body: [6, 18, 48],
      core0: [0, 4, 16],
      core1: [4, 16, 44],
      core2: [14, 42, 115],
      core3: [24, 72, 175],
      core4: [48, 112, 235],
      core5: [90, 155, 255],
      core6: [50, 120, 230]
    },
    // 2. À l'écoute : micro actif ou parole en cours
    listening: {
      primary: [56, 225, 255],
      secondary: [126, 242, 226],
      glow: [56, 225, 255],
      body: [8, 48, 60],
      core0: [0, 14, 18],
      core1: [4, 45, 58],
      core2: [20, 130, 160],
      core3: [40, 185, 220],
      core4: [90, 230, 255],
      core5: [160, 245, 255],
      core6: [56, 225, 255]
    },
    // 3. Au travail : une ou plusieurs tâches en cours
    working: {
      primary: [50, 225, 195],
      secondary: [120, 250, 215],
      glow: [45, 215, 175],
      body: [10, 52, 60],
      core0: [0, 14, 14],
      core1: [6, 52, 48],
      core2: [30, 150, 122],
      core3: [58, 210, 168],
      core4: [126, 242, 226],
      core5: [84, 224, 255],
      core6: [56, 225, 255]
    },
    // 4. Satisfait : tâche réussie récemment (~4 s)
    satisfied: {
      primary: [53, 255, 158],
      secondary: [120, 255, 190],
      glow: [53, 255, 158],
      body: [8, 55, 30],
      core0: [0, 16, 8],
      core1: [6, 52, 28],
      core2: [20, 140, 75],
      core3: [40, 205, 115],
      core4: [80, 245, 150],
      core5: [150, 255, 200],
      core6: [53, 255, 158]
    },
    // 5. En attente : validation par code requise (ambre)
    awaiting: {
      primary: [255, 176, 32],
      secondary: [255, 210, 100],
      glow: [255, 160, 20],
      body: [60, 36, 6],
      core0: [18, 10, 0],
      core1: [55, 32, 4],
      core2: [150, 95, 15],
      core3: [215, 140, 25],
      core4: [255, 185, 55],
      core5: [255, 225, 130],
      core6: [255, 170, 30]
    },
    // 5 bis. IA externe sollicitée (Claude Code / AGY) : orange vif, distinct de l'ambre de
    // validation. Tant que cette couleur est affichée, des tokens sont consommés (v5).
    external: {
      primary: [255, 122, 24],
      secondary: [255, 160, 70],
      glow: [255, 105, 10],
      body: [62, 24, 4],
      core0: [20, 6, 0],
      core1: [60, 20, 2],
      core2: [165, 60, 8],
      core3: [230, 95, 15],
      core4: [255, 130, 35],
      core5: [255, 190, 120],
      core6: [255, 110, 20]
    },
    // 6. Contrarié : échec, refus ou accès interdit récent (~5 s) - Rouge vif vermillon
    error: {
      primary: [255, 60, 75],
      secondary: [255, 120, 130],
      glow: [255, 45, 65],
      body: [65, 12, 18],
      core0: [20, 2, 4],
      core1: [60, 10, 15],
      core2: [160, 25, 35],
      core3: [225, 45, 60],
      core4: [255, 80, 95],
      core5: [255, 160, 170],
      core6: [255, 50, 65]
    },
    // 7. Sous tension : quota 5h >= 90% - Rouge sombre grenat pulsé (distinct de error)
    overload: {
      primary: [140, 20, 35],
      secondary: [105, 15, 28],
      glow: [120, 15, 30],
      body: [35, 5, 10],
      core0: [12, 0, 2],
      core1: [38, 4, 8],
      core2: [85, 12, 20],
      core3: [125, 18, 30],
      core4: [150, 25, 40],
      core5: [185, 45, 65],
      core6: [120, 18, 30]
    }
  };

  // Couleurs de conversation (demande de l'utilisateur) : elles priment sur les humeurs.
  //   user  : l'utilisateur parle            → VERT
  //   wake  : Jarvis a entendu son nom       → BLEU
  //   think : Jarvis réfléchit               → JAUNE
  //   reply : Jarvis répond                  → ROUGE
  const PHASE_COLORS = {
    user: [40, 235, 110],
    wake: [60, 135, 255],
    think: [255, 205, 30],
    reply: [255, 45, 55]
  };
  function phasePalette(c) {
    const k = (f) => c.map((v) => Math.round(v * f));
    const light = c.map((v) => Math.round(v + (255 - v) * 0.45));
    return {
      primary: [...c], secondary: k(0.8), glow: k(0.9), body: k(0.13),
      core0: k(0.03), core1: k(0.1), core2: k(0.35), core3: k(0.62), core4: k(0.9), core5: light, core6: k(0.88)
    };
  }
  for (const [phase, rgb] of Object.entries(PHASE_COLORS)) MOOD_PALETTES[`talk_${phase}`] = phasePalette(rgb);
  // Repos : cœur neutre gris argenté, quel que soit le thème (un cœur vert ou bleu au repos se
  // confondrait avec « tu parles » ou « nom entendu »). Le thème colore le reste de l'interface.
  MOOD_PALETTES.talk_rest = phasePalette([190, 200, 215]);
  // Couleur du contour réactif (traits, enveloppe, anneau pulsé) selon la phase.
  function phaseRgb(phase) {
    const c = PHASE_COLORS[phase] || PHASE_COLORS.user;
    return { hi: c.join(', '), light: c.map((v) => Math.round(v + (255 - v) * 0.5)).join(', ') };
  }

  // Clone une palette pour initialiser l'état interpolé
  function clonePalette(pal) {
    const out = {};
    for (const k of Object.keys(pal)) {
      out[k] = [...pal[k]];
    }
    return out;
  }

  const THEME_IDLE_PALETTES = {
    cyan: {
      primary: [70, 130, 255],
      secondary: [40, 95, 210],
      glow: [50, 110, 240],
      body: [6, 18, 48],
      core0: [0, 4, 16],
      core1: [4, 16, 44],
      core2: [14, 42, 115],
      core3: [24, 72, 175],
      core4: [48, 112, 235],
      core5: [90, 155, 255],
      core6: [50, 120, 230]
    },
    gold: {
      primary: [255, 180, 32],
      secondary: [230, 140, 20],
      glow: [255, 190, 40],
      body: [48, 24, 6],
      core0: [16, 8, 0],
      core1: [44, 20, 4],
      core2: [115, 60, 10],
      core3: [175, 105, 18],
      core4: [235, 155, 28],
      core5: [255, 210, 80],
      core6: [230, 160, 30]
    },
    matrix: {
      primary: [0, 255, 110],
      secondary: [0, 200, 85],
      glow: [20, 255, 130],
      body: [6, 44, 18],
      core0: [0, 14, 4],
      core1: [4, 40, 14],
      core2: [12, 105, 42],
      core3: [20, 165, 70],
      core4: [40, 225, 100],
      core5: [120, 255, 170],
      core6: [30, 210, 90]
    },
    crimson: {
      primary: [255, 48, 80],
      secondary: [200, 25, 55],
      glow: [255, 60, 95],
      body: [48, 8, 16],
      core0: [16, 0, 4],
      core1: [44, 4, 14],
      core2: [115, 14, 38],
      core3: [175, 24, 60],
      core4: [235, 45, 85],
      core5: [255, 120, 150],
      core6: [220, 35, 70]
    },
    violet: {
      primary: [190, 85, 240],
      secondary: [150, 50, 200],
      glow: [200, 95, 255],
      body: [40, 8, 48],
      core0: [12, 0, 16],
      core1: [36, 4, 44],
      core2: [95, 16, 115],
      core3: [145, 30, 175],
      core4: [195, 65, 235],
      core5: [230, 140, 255],
      core6: [175, 60, 215]
    }
  };

  J.setOrbTheme = function (themeId) {
    const pal = THEME_IDLE_PALETTES[themeId] || THEME_IDLE_PALETTES.cyan;
    MOOD_PALETTES.idle = clonePalette(pal);
  };

  const currentPal = clonePalette(MOOD_PALETTES.idle);

  function updatePalette(targetMood, dt, tau = 0.6) {
    const target = MOOD_PALETTES[targetMood] || MOOD_PALETTES.idle;
    // Constante de temps : ~600 ms pour les humeurs, ~180 ms pour les couleurs de conversation
    const blend = Math.min(1, dt / tau);
    for (const key of Object.keys(target)) {
      const cur = currentPal[key];
      const tgt = target[key];
      cur[0] += (tgt[0] - cur[0]) * blend;
      cur[1] += (tgt[1] - cur[1]) * blend;
      cur[2] += (tgt[2] - cur[2]) * blend;
    }
  }

  function col(key, alpha) {
    const c = currentPal[key];
    return `rgba(${Math.round(c[0])}, ${Math.round(c[1])}, ${Math.round(c[2])}, ${alpha})`;
  }

  // Couleur courante du cœur (« r, g, b ») : les liaisons cœur → cards la reprennent (branches.js).
  J.heartLine = function () {
    return colStr('primary');
  };

  function colStr(key) {
    const c = currentPal[key];
    return `${Math.round(c[0])}, ${Math.round(c[1])}, ${Math.round(c[2])}`;
  }

  // --- Configuration des anneaux interactifs : Zone Bleue (interne) & Zone Verte (externe) ---
  const RINGS = [
    // Zone externe : VERT — s'accélère lors d'une action externe (navigateur Chrome, Spotify, Google, web)
    { r: 0.97, baseSpeed: 0.05, curSpeed: 0.05, angle: 0, w: 1.3, a: 0.70, zone: 'green', segs: [[0, 55], [72, 112], [140, 192], [250, 312]] },
    { r: 0.90, baseSpeed: -0.08, curSpeed: -0.08, angle: 0, w: 1.2, a: 0.65, zone: 'green', segs: [[8, 120], [168, 250], [282, 342]] },
    // Zone intermédiaire / interne : BLEU — s'accélère lors d'une action interne / locale
    { r: 0.80, baseSpeed: 0.13, curSpeed: 0.13, angle: 0, w: 1.9, a: 0.80, zone: 'blue', segs: [[0, 34], [56, 142], [198, 262], [300, 352]] },
    { r: 0.70, baseSpeed: -0.04, curSpeed: -0.04, angle: 0, w: 1.3, a: 0.70, zone: 'blue', segs: [[20, 92], [120, 240], [268, 330]] }
  ];

  // Séquence d'allumage (boot.js) : part de l'anneau déjà tracée, 0 → 1. Dans une zone, l'anneau
  // intérieur part le premier ; le tracé avance comme un stylo dans le sens horaire.
  function ringTrace(ring, rv) {
    if (!rv) return 1;
    const zone = RINGS.filter((r) => r.zone === ring.zone);
    const idx = zone.length - 1 - zone.indexOf(ring);
    const p = ring.zone === 'blue' ? rv.blue : rv.green;
    return Math.max(0, Math.min(1, p * 1.5 - idx * 0.5));
  }

  function rings(g, cx, cy, R, dt, state) {
    g.lineCap = 'butt';
    const scale = Math.max(0.65, Math.min(1.0, R / 140));
    const hasLocal = !!(state && state.hasLocalAction);
    const hasExternal = !!(state && state.hasExternalAction);
    const rv = state && state.reveal;

    for (const ring of RINGS) {
      const trace = ringTrace(ring, rv);
      if (trace <= 0) continue;
      // Vitesse adaptative : accélérée selon la nature de l'action
      const isBlue = ring.zone === 'blue';
      const isAccelerated = isBlue ? hasLocal : hasExternal;
      const targetSpeed = ring.baseSpeed * (isAccelerated ? 4.5 : 1.0);
      
      ring.curSpeed += (targetSpeed - ring.curSpeed) * Math.min(1, dt * 6.0);
      ring.angle = (ring.angle + ring.curSpeed * dt) % TAU;
      const rot = ring.angle;

      g.lineWidth = ring.w * scale * (isAccelerated ? 1.25 : 1.0);

      // Couleurs distinctes demandées par l'utilisateur
      if (isBlue) {
        // Cercle / partie en BLEU (action interne)
        g.strokeStyle = isAccelerated ? `rgba(64, 185, 255, ${ring.a})` : `rgba(32, 144, 255, ${ring.a * 0.85})`;
      } else {
        // Cercle / partie en VERT (action externe)
        g.strokeStyle = isAccelerated ? `rgba(28, 255, 142, ${ring.a})` : `rgba(18, 220, 118, ${ring.a * 0.85})`;
      }

      const limit = trace * 360;
      for (const [a0, a1] of ring.segs) {
        if (a0 >= limit) break;
        g.beginPath();
        g.arc(cx, cy, R * ring.r, rad(a0) + rot, rad(Math.min(a1, limit)) + rot);
        g.stroke();
      }

      // Pointe lumineuse du tracé pendant l'allumage
      if (trace < 1) {
        const a = rad(limit) + rot;
        g.save();
        g.globalCompositeOperation = 'lighter';
        g.fillStyle = isBlue ? 'rgba(160, 220, 255, 0.95)' : 'rgba(160, 255, 200, 0.95)';
        g.beginPath();
        g.arc(cx + Math.cos(a) * R * ring.r, cy + Math.sin(a) * R * ring.r, 2.2 * scale, 0, TAU);
        g.fill();
        g.restore();
      }

      // Halo additif d'énergie cinétique lorsque l'action est active (ou flash de l'étape validée)
      const flash = rv ? (isBlue ? rv.flashBlue : rv.flashGreen) || 0 : 0;
      if (isAccelerated || flash > 0) {
        g.save();
        g.globalCompositeOperation = 'lighter';
        g.lineWidth = (ring.w + 3.0) * scale;
        const haloA = isAccelerated ? 0.35 : 0.6 * flash;
        g.strokeStyle = isBlue ? `rgba(0, 180, 255, ${haloA})` : `rgba(0, 255, 140, ${haloA})`;
        for (const [a0, a1] of ring.segs) {
          if (a0 >= limit) break;
          g.beginPath();
          g.arc(cx, cy, R * ring.r, rad(a0) + rot, rad(Math.min(a1, limit)) + rot);
          g.stroke();
        }
        g.restore();
      }
    }
  }

  // `shown` (0 → 1, allumage) : seules les premières graduations sont dessinées.
  function ticks(g, cx, cy, r0, r1, a0, a1, count, alpha, rot, shown = 1) {
    const scale = Math.max(0.65, Math.min(1.0, (r1 - r0) / 10));
    g.lineWidth = Math.max(0.8, 1.6 * scale);
    g.strokeStyle = col('primary', alpha);
    const visible = Math.round(count * shown);
    for (let i = 0; i < visible; i++) {
      const a = rad(a0 + ((a1 - a0) * i) / (count - 1)) + rot;
      const c = Math.cos(a);
      const s = Math.sin(a);
      g.beginPath();
      g.moveTo(cx + c * r0, cy + s * r0);
      g.lineTo(cx + c * r1, cy + s * r1);
      g.stroke();
    }
  }

  // Anneau déformé par sommes de sinus : surface qui respire
  function livingRing(g, cx, cy, r, t, amp, phase, width, alpha) {
    g.beginPath();
    for (let i = 0; i <= 96; i++) {
      const a = (i / 96) * TAU;
      const d =
        r +
        amp * Math.sin(3 * a + t * 0.8 + phase) +
        amp * 0.6 * Math.sin(5 * a - t * 1.3 + phase) +
        amp * 0.4 * Math.sin(8 * a + t * 0.5);
      const x = cx + Math.cos(a) * d;
      const y = cy + Math.sin(a) * d;
      if (i === 0) g.moveTo(x, y);
      else g.lineTo(x, y);
    }
    g.closePath();
    g.lineWidth = width;
    g.strokeStyle = col('secondary', alpha);
    g.stroke();
  }

  function blob(g, cx, cy, r, t, speed, phase, alpha) {
    const d = r * 0.4;
    const x = cx + Math.cos(t * speed + phase) * d;
    const y = cy + Math.sin(t * speed * 0.7 + phase) * d;
    const grd = g.createRadialGradient(x, y, 0, x, y, r * 0.8);
    grd.addColorStop(0, col('secondary', alpha));
    grd.addColorStop(1, col('secondary', 0));
    g.fillStyle = grd;
    g.beginPath();
    g.arc(x, y, r * 0.8, 0, TAU);
    g.fill();
  }

  let lastUserSpeechAt = 0;

  // Tampons réutilisés d'une image à l'autre par la couronne vocale (avant : une Map, des tableaux
  // et 120 objets créés à chaque image). 6 paliers d'opacité : round(alpha × 5) va de 0 à 5.
  const waveBuckets = [[], [], [], [], [], []];
  const waveTips = []; // x, y, x, y… des pointes (utilisé seulement quand l'utilisateur parle)

  // Couronne réactive extérieure : traits vers l'extérieur réagissant quand l'utilisateur parle
  function waveform(g, cx, cy, R, state, level, t) {
    const history = state.levels;
    if (!history || !history.length) return;
    const n = history.length;
    const inner = R * 1.06;

    // L'utilisateur parle et le système le prend en compte (fourni par state.userSpeaking)
    // Dès que l'utilisateur s'arrête de parler, isUserSpeaking repasse immédiatement à false
    const isUserSpeaking = !!(state && state.userSpeaking && !state.speaking);
    // Même couleur que le cœur : VERT quand l'utilisateur parle, BLEU une fois « Jarvis » entendu.
    const { hi, light: hiLight } = phaseRgb(state && state.voicePhase === 'wake' ? 'wake' : 'user');
    const userVoice = Math.max(state && state.userVoiceLevel ? state.userVoiceLevel : 0, level || 0);
    const hot = isUserSpeaking || (state && state.speaking);

    waveTips.length = 0;
    for (const bucket of waveBuckets) bucket.length = 0;
    // Allumage : la couronne se déploie en éventail depuis le haut, dans le sens horaire.
    const fan = state.reveal ? Math.round(n * state.reveal.wave) : n;

    g.lineCap = 'round';
    const lineWidth = isUserSpeaking ? 2.8 : (hot ? 2.4 : 1.8);
    g.lineWidth = lineWidth;

    // Une seule couleur par image (selon le mode) : seuls les paliers d'opacité diffèrent, ce qui
    // réduit les 120 appels stroke() à 1 à 6 appels par frame.
    let r, gr, b;
    if (isUserSpeaking) {
      [r, gr, b] = hi.split(',').map(Number);
    } else if (state && state.speaking) {
      const sec = currentPal.secondary;
      r = Math.round(sec[0]); gr = Math.round(sec[1]); b = Math.round(sec[2]);
    } else {
      r = 255; gr = 255; b = 255;
    }

    for (let i = 0; i < fan; i++) {
      const a = (i / n) * TAU - Math.PI / 2;
      const v = history[(state.head + i) % n] || 0;

      let len;
      let alpha;

      if (isUserSpeaking) {
        // Dynamique réactive amplifiée quand l'utilisateur parle (vert, ou bleu si « Jarvis » est entendu)
        const waveMod = Math.sin(i * 0.45 + t * 14) * 0.35 + 0.65;
        const voiceBoost = Math.max(v, userVoice) * waveMod;
        len = R * 0.06 + voiceBoost * R * 0.42;
        alpha = Math.min(1.0, 0.55 + voiceBoost * 0.75);
      } else if (state && state.speaking) {
        // Quand Jarvis parle en retour
        len = R * 0.06 + v * R * 0.22;
        alpha = 0.25 + v * 0.55;
      } else {
        // Quand le système ne prend pas en compte ce qu'il entend : ZONE BLANCHE
        len = R * 0.05 + v * R * 0.08 + Math.sin(i * 0.28 + t * 2.5) * R * 0.01;
        alpha = Math.min(1.0, 0.22 + v * 0.55);
      }

      const c = Math.cos(a);
      const s = Math.sin(a);
      const x1 = cx + c * inner;
      const y1 = cy + s * inner;
      const tipX = cx + c * (inner + len);
      const tipY = cy + s * (inner + len);

      if (isUserSpeaking) waveTips.push(tipX, tipY);

      waveBuckets[Math.min(5, Math.max(0, Math.round(alpha * 5)))].push(x1, y1, tipX, tipY);
    }

    for (let k = 0; k < waveBuckets.length; k++) {
      const coords = waveBuckets[k];
      if (!coords.length) continue;
      g.strokeStyle = `rgba(${r},${gr},${b},${k / 5})`;
      g.beginPath();
      for (let j = 0; j < coords.length; j += 4) {
        g.moveTo(coords[j], coords[j + 1]);
        g.lineTo(coords[j + 2], coords[j + 3]);
      }
      g.stroke();
    }

    // Ligne enveloppe ondulée entourant les traits (couleur de la phase de conversation)
    if (isUserSpeaking && waveTips.length > 4) {
      g.save();
      g.beginPath();
      g.moveTo(waveTips[0], waveTips[1]);
      for (let i = 2; i < waveTips.length; i += 2) {
        const prevX = waveTips[i - 2];
        const prevY = waveTips[i - 1];
        g.quadraticCurveTo(prevX, prevY, (prevX + waveTips[i]) / 2, (prevY + waveTips[i + 1]) / 2);
      }
      g.closePath();
      // Halo par traits additifs plutôt que shadowBlur (l'une des opérations canvas les plus coûteuses).
      g.globalCompositeOperation = 'lighter';
      g.strokeStyle = `rgba(${hi}, 0.10)`;
      g.lineWidth = 9;
      g.stroke();
      g.strokeStyle = `rgba(${hi}, 0.22)`;
      g.lineWidth = 5;
      g.stroke();
      g.globalCompositeOperation = 'source-over';
      g.lineWidth = 2.4;
      g.strokeStyle = `rgba(${hi}, 0.88)`;
      g.stroke();
      g.restore();
    }

    // Anneau de capture holographique pulsé, uniquement quand l'utilisateur parle
    if (isUserSpeaking) {
      const pulse = 0.45 + 0.45 * Math.abs(Math.sin(t * 4.5));
      const capR = inner + R * 0.5 + level * R * 0.15;
      g.save();
      g.globalCompositeOperation = 'lighter';
      g.lineWidth = 4.5;
      g.strokeStyle = `rgba(${hi}, ${pulse * 0.35})`;
      g.beginPath();
      g.arc(cx, cy, capR, 0, TAU);
      g.stroke();
      g.lineWidth = 2.0;
      g.strokeStyle = `rgba(${hiLight}, ${pulse * 0.95})`;
      g.beginPath();
      g.arc(cx, cy, capR, 0, TAU);
      g.stroke();
      g.restore();
    }
  }

  // --- Morphing holographique de la zone centrale du cœur selon la demande ---
  // Glyphe passager (une demande : heure, animal, météo…) prioritaire ; sinon glyphe maintenu
  // (musique en cours). Priorité : 2 = demande précise (orb-morph.js), 1 = mot-clé générique.
  const currentMorph = {
    type: null,
    startedAt: 0,
    duration: 2400,
    active: false,
    data: null,
    priority: 1
  };
  const heldMorph = { type: null, shown: null, alpha: 0 };

  J.triggerOrbMorph = function (type, duration = 2400, data = null, priority = 1) {
    if (!type) return;
    if (!data && heldMorph.type === type) return; // déjà affiché en continu (note pendant la musique)
    const now = performance.now();
    if (currentMorph.active && currentMorph.priority > priority && now - currentMorph.startedAt < currentMorph.duration - 600) return;
    currentMorph.type = type;
    currentMorph.startedAt = now;
    currentMorph.duration = duration;
    currentMorph.active = true;
    currentMorph.data = data;
    currentMorph.priority = priority;
  };

  // Complète le glyphe affiché (météo : la température arrive avec la réponse).
  J.updateOrbMorph = function (type, data) {
    if (currentMorph.active && currentMorph.type === type) currentMorph.data = { ...(currentMorph.data || {}), ...data };
  };

  // Glyphe maintenu tant que « on » (note de musique pendant la lecture).
  J.setOrbMorphHold = function (type, on) {
    if (on) heldMorph.type = type;
    else if (heldMorph.type === type) heldMorph.type = null;
  };

  J.getOrbMorph = function () {
    return currentMorph;
  };

  // Niveau du son joué par le PC (0..1), fourni par orb-morph.js ; à défaut, pulsation à 120 bpm.
  function musicLevel(t) {
    const m = J.musicLevel;
    if (m && performance.now() - m.at < 1200) return m.v;
    return 0.3 + 0.35 * Math.pow(Math.max(0, Math.sin(t * Math.PI * 2)), 4);
  }

  // Petites notes qui s'envolent sur les temps forts de la musique.
  const floaters = [];
  let lastBeatLevel = 0;

  // Silhouettes holographiques tirées des emojis de Windows (Segoe UI Emoji) : la forme est gardée,
  // les couleurs remplacées par une teinte unique qui garde un peu du relief, rayée de lignes de
  // balayage. Mises en cache (forme + halo flouté) : le coût n'est payé qu'à la première image.
  const emojiCache = new Map();
  function emojiHologram(emoji, px, rgb) {
    const key = `${emoji}|${px}|${rgb}`;
    const hit = emojiCache.get(key);
    if (hit) return hit;
    const pad = Math.ceil(px * 0.3);
    const size = px + pad * 2;
    const shape = document.createElement('canvas');
    shape.width = shape.height = size;
    const x = shape.getContext('2d', { willReadFrequently: true });
    x.font = `${px}px "Segoe UI Emoji", "Apple Color Emoji", "Noto Color Emoji", sans-serif`;
    x.textAlign = 'center';
    x.textBaseline = 'middle';
    x.fillText(emoji, size / 2, size / 2 + px * 0.04);
    const img = x.getImageData(0, 0, size, size);
    const d = img.data;
    const [r, gg, b] = rgb.split(',').map(Number);
    for (let i = 0; i < d.length; i += 4) {
      const a = d[i + 3];
      if (!a) continue;
      const lum = (0.3 * d[i] + 0.59 * d[i + 1] + 0.11 * d[i + 2]) / 255;
      d[i] = r; d[i + 1] = gg; d[i + 2] = b;
      d[i + 3] = a * (0.55 + 0.45 * lum);
    }
    x.putImageData(img, 0, 0);
    x.globalCompositeOperation = 'destination-out';
    x.fillStyle = 'rgba(0, 0, 0, 0.45)';
    for (let y = 0; y < size; y += 4) x.fillRect(0, y, size, 1.4);
    const glow = document.createElement('canvas');
    glow.width = glow.height = size;
    const gx = glow.getContext('2d');
    gx.filter = `blur(${Math.max(2, Math.round(px / 22))}px)`;
    gx.drawImage(shape, 0, 0);
    gx.drawImage(shape, 0, 0);
    const entry = { shape, glow, size };
    if (emojiCache.size > 40) emojiCache.clear();
    emojiCache.set(key, entry);
    return entry;
  }

  const DAYS = ['DIM.', 'LUN.', 'MAR.', 'MER.', 'JEU.', 'VEN.', 'SAM.'];
  const MONTHS = ['JANV.', 'FÉVR.', 'MARS', 'AVR.', 'MAI', 'JUIN', 'JUIL.', 'AOÛT', 'SEPT.', 'OCT.', 'NOV.', 'DÉC.'];

  function glowText(g, text, x, y, px, alpha, weight = 'bold') {
    g.font = `${weight} ${Math.round(px)}px Consolas, "Courier New", monospace`;
    g.textAlign = 'center';
    g.textBaseline = 'middle';
    g.lineJoin = 'round';
    g.lineWidth = Math.max(2, px * 0.12);
    g.strokeStyle = `rgba(56, 225, 255, ${0.35 * alpha})`;
    g.strokeText(text, x, y);
    g.fillStyle = `rgba(230, 250, 255, ${0.95 * alpha})`;
    g.fillText(text, x, y);
  }

  // Tracé des glyphes holographiques haute fidélité
  function drawHolographicGlyph(g, cx, cy, Rc, type, alpha, scale, t, data) {
    if (alpha <= 0.01) return;
    const size = Rc * 0.52 * scale;
    g.save();
    // Écran sombre derrière le glyphe : au repos le cœur est gris argenté très clair, un glyphe
    // clair s'y noierait.
    g.globalCompositeOperation = 'source-over'; // (la vie interne se dessine en « lighter » : ici on assombrit)
    const screen = g.createRadialGradient(cx, cy, 0, cx, cy, Rc * 0.97);
    screen.addColorStop(0, `rgba(0, 8, 18, ${0.88 * alpha})`);
    screen.addColorStop(0.8, `rgba(0, 12, 24, ${0.8 * alpha})`);
    screen.addColorStop(1, 'rgba(0, 12, 24, 0)');
    g.fillStyle = screen;
    g.beginPath();
    g.arc(cx, cy, Rc * 0.97, 0, TAU);
    g.fill();
    g.globalCompositeOperation = 'lighter';

    if (type === 'clock') {
      // 22h23 : heure réelle, rafraîchie à chaque image ; anneau des secondes autour.
      const d = new Date();
      if (data && data.mode === 'date') {
        glowText(g, DAYS[d.getDay()], cx, cy - size * 0.78, size * 0.36, alpha * 0.85);
        glowText(g, String(d.getDate()), cx, cy + size * 0.02, size * 1.15, alpha);
        glowText(g, MONTHS[d.getMonth()], cx, cy + size * 0.8, size * 0.36, alpha * 0.85);
      } else {
        const text = `${String(d.getHours()).padStart(2, '0')}h${String(d.getMinutes()).padStart(2, '0')}`;
        glowText(g, text, cx, cy - size * 0.08, size * 0.95, alpha);
        glowText(g, `${DAYS[d.getDay()]} ${d.getDate()} ${MONTHS[d.getMonth()]}`, cx, cy + size * 0.62, size * 0.27, alpha * 0.75, 'normal');
      }
      const sec = d.getSeconds() + d.getMilliseconds() / 1000;
      g.lineWidth = 2;
      g.strokeStyle = `rgba(160, 245, 255, ${0.55 * alpha})`;
      g.beginPath();
      g.arc(cx, cy, Rc * 0.9, -Math.PI / 2, -Math.PI / 2 + (sec / 60) * TAU);
      g.stroke();
    } else if (type === 'emoji' && data && data.emoji) {
      // Silhouette holographique (animal, météo, objet) ; légende facultative dessous (« 23° »).
      const px = Math.max(24, Math.round(size * (data.caption ? 1.7 : 2.2)));
      const holo = emojiHologram(data.emoji, px, data.rgb || '205, 245, 255');
      const lift = data.caption ? -size * 0.22 : 0;
      const bob = Math.sin(t * 1.6) * size * 0.03;
      const s = holo.size;
      g.globalAlpha = alpha * (0.7 + 0.1 * Math.sin(t * 5));
      g.drawImage(holo.glow, cx - s / 2, cy - s / 2 + lift + bob, s, s);
      g.globalAlpha = alpha * (0.92 + 0.08 * Math.sin(t * 9));
      g.drawImage(holo.shape, cx - s / 2, cy - s / 2 + lift + bob, s, s);
      g.globalAlpha = 1;
      if (data.caption) glowText(g, data.caption, cx, cy + size * 0.78, size * 0.45, alpha);
    } else if (type === 'music') {
      // ♫ NOTE DE MUSIQUE HOLOGRAPHIQUE (double croche stylisée Stark), qui danse sur le son du PC
      const level = musicLevel(t);
      if (level > 0.62 && lastBeatLevel <= 0.62 && floaters.length < 8) {
        floaters.push({ born: t, x: (Math.random() - 0.5) * 1.3, drift: (Math.random() - 0.5) * 0.4, s: 0.35 + Math.random() * 0.25 });
      }
      lastBeatLevel = level;
      for (let i = floaters.length - 1; i >= 0; i--) {
        const f = floaters[i];
        const age = (t - f.born) / 1.6;
        if (age >= 1) {
          floaters.splice(i, 1);
          continue;
        }
        const fx = cx + (f.x + f.drift * age) * size;
        const fy = cy + size * 0.3 - age * size * 1.1;
        const fa = alpha * (1 - age) * 0.8;
        g.fillStyle = `rgba(200, 245, 255, ${fa})`;
        g.strokeStyle = `rgba(200, 245, 255, ${fa})`;
        g.lineWidth = 1.6;
        g.beginPath();
        g.ellipse(fx, fy, size * 0.09 * f.s * 2, size * 0.065 * f.s * 2, -0.35, 0, TAU);
        g.fill();
        g.beginPath();
        g.moveTo(fx + size * 0.16 * f.s, fy);
        g.lineTo(fx + size * 0.16 * f.s, fy - size * 0.5 * f.s);
        g.stroke();
      }
      // Anneau qui bat la mesure
      g.lineWidth = 1.5 + level * 2.5;
      g.strokeStyle = `rgba(56, 225, 255, ${0.35 * alpha * level})`;
      g.beginPath();
      g.arc(cx, cy, size * (0.82 + 0.22 * level), 0, TAU);
      g.stroke();
      g.translate(cx, cy - size * 0.1 * level);
      g.rotate(Math.sin(t * 2.6) * (0.04 + 0.1 * level));
      g.scale(1.25 * (1 + 0.2 * level), 1.25 * (1 + 0.2 * level));
      g.translate(-cx, -cy);

      const glow = `rgba(56, 225, 255, ${0.45 * alpha})`;
      const stroke = `rgba(230, 250, 255, ${0.95 * alpha})`;
      const rNoteX = size * 0.22;
      const rNoteY = size * 0.15;
      const n1x = cx - size * 0.38;
      const n1y = cy + size * 0.28;
      const n2x = cx + size * 0.26;
      const n2y = cy + size * 0.12;

      for (let pass = 0; pass < 2; pass++) {
        g.lineWidth = pass === 0 ? 5.0 : 2.2;
        g.strokeStyle = pass === 0 ? glow : stroke;
        g.fillStyle = pass === 0 ? glow : stroke;

        // Tête note 1
        g.beginPath();
        g.ellipse(n1x, n1y, rNoteX, rNoteY, -0.35, 0, TAU);
        g.fill();

        // Tête note 2
        g.beginPath();
        g.ellipse(n2x, n2y, rNoteX, rNoteY, -0.35, 0, TAU);
        g.fill();

        // Hampe note 1
        g.beginPath();
        g.moveTo(n1x + rNoteX * 0.8, n1y);
        g.lineTo(n1x + rNoteX * 0.8, cy - size * 0.44);
        g.stroke();

        // Hampe note 2
        g.beginPath();
        g.moveTo(n2x + rNoteX * 0.8, n2y);
        g.lineTo(n2x + rNoteX * 0.8, cy - size * 0.60);
        g.stroke();

        // Barre 1 supérieure reliant les deux hampes
        g.beginPath();
        g.moveTo(n1x + rNoteX * 0.8, cy - size * 0.44);
        g.lineTo(n2x + rNoteX * 0.8, cy - size * 0.60);
        g.stroke();

        // Barre 2 intermédiaire
        g.beginPath();
        g.moveTo(n1x + rNoteX * 0.8, cy - size * 0.28);
        g.lineTo(n2x + rNoteX * 0.8, cy - size * 0.44);
        g.stroke();
      }
    } else if (type === 'mail') {
      // ✉ ENVELOPPE HOLOGRAPHIQUE (courrier / message / Gmail)
      const w = size * 1.25;
      const h = size * 0.82;
      const x = cx - w / 2;
      const y = cy - h / 2;
      const glow = `rgba(53, 255, 158, ${0.45 * alpha})`;
      const stroke = `rgba(225, 255, 240, ${0.95 * alpha})`;

      for (let pass = 0; pass < 2; pass++) {
        g.lineWidth = pass === 0 ? 5.0 : 2.2;
        g.strokeStyle = pass === 0 ? glow : stroke;

        // Rectangle extérieur
        g.beginPath();
        g.rect(x, y, w, h);
        g.stroke();

        // Rabat supérieur en V
        g.beginPath();
        g.moveTo(x, y);
        g.lineTo(cx, cy + size * 0.12);
        g.lineTo(x + w, y);
        g.stroke();

        // Plis intérieurs diagonaux
        g.beginPath();
        g.moveTo(x, y + h);
        g.lineTo(cx - size * 0.18, cy + size * 0.04);
        g.moveTo(x + w, y + h);
        g.lineTo(cx + size * 0.18, cy + size * 0.04);
        g.stroke();
      }
    } else if (type === 'browser' || type === 'web') {
      // 🌐 GLOBE TERRESTRE HOLOGRAPHIQUE (navigateur Chrome, recherche web)
      const r = size * 0.64;
      const glow = `rgba(56, 225, 255, ${0.45 * alpha})`;
      const stroke = `rgba(215, 250, 255, ${0.95 * alpha})`;

      for (let pass = 0; pass < 2; pass++) {
        g.lineWidth = pass === 0 ? 5.0 : 2.0;
        g.strokeStyle = pass === 0 ? glow : stroke;

        // Sphère
        g.beginPath();
        g.arc(cx, cy, r, 0, TAU);
        g.stroke();

        // Équateur
        g.beginPath();
        g.ellipse(cx, cy, r, r * 0.35, 0, 0, TAU);
        g.stroke();

        // Méridiens verticaux
        g.beginPath();
        g.ellipse(cx, cy, r * 0.48, r, 0, 0, TAU);
        g.stroke();

        // Axe
        g.beginPath();
        g.moveTo(cx, cy - r);
        g.lineTo(cx, cy + r);
        g.stroke();
      }
    } else if (type === 'code' || type === 'terminal') {
      // >_ INVITE TERMINAL HOLOGRAPHIQUE (fichiers, scripts, terminal)
      const glow = `rgba(56, 225, 255, ${0.45 * alpha})`;
      const stroke = `rgba(220, 250, 255, ${0.95 * alpha})`;
      const blink = Math.sin(t * 8) > 0 ? 1 : 0.25;

      for (let pass = 0; pass < 2; pass++) {
        g.lineWidth = pass === 0 ? 5.0 : 2.5;
        g.strokeStyle = pass === 0 ? glow : stroke;

        // Chevron >
        g.beginPath();
        g.moveTo(cx - size * 0.48, cy - size * 0.38);
        g.lineTo(cx - size * 0.06, cy);
        g.lineTo(cx - size * 0.48, cy + size * 0.38);
        g.stroke();

        // Curseur _ clignotant
        g.strokeStyle = pass === 0 ? glow : `rgba(220, 250, 255, ${0.95 * alpha * blink})`;
        g.beginPath();
        g.moveTo(cx + size * 0.08, cy + size * 0.38);
        g.lineTo(cx + size * 0.48, cy + size * 0.38);
        g.stroke();
      }
    } else if (type === 'calendar' || type === 'agenda') {
      // ▦ AGENDA / CALENDRIER HOLOGRAPHIQUE
      const w = size * 1.05;
      const h = size * 0.95;
      const x = cx - w / 2;
      const y = cy - h / 2 + size * 0.06;
      const glow = `rgba(255, 185, 55, ${0.45 * alpha})`;
      const stroke = `rgba(255, 240, 195, ${0.95 * alpha})`;

      for (let pass = 0; pass < 2; pass++) {
        g.lineWidth = pass === 0 ? 5.0 : 2.0;
        g.strokeStyle = pass === 0 ? glow : stroke;

        g.beginPath();
        g.rect(x, y, w, h);
        g.stroke();

        g.beginPath();
        g.moveTo(x, y + h * 0.3);
        g.lineTo(x + w, y + h * 0.3);
        g.stroke();

        g.beginPath();
        g.moveTo(cx - w * 0.25, y - size * 0.1);
        g.lineTo(cx - w * 0.25, y + size * 0.05);
        g.moveTo(cx + w * 0.25, y - size * 0.1);
        g.lineTo(cx + w * 0.25, y + size * 0.05);
        g.stroke();

        g.fillStyle = pass === 0 ? glow : stroke;
        for (let r = 0; r < 2; r++) {
          for (let c = 0; c < 3; c++) {
            g.fillRect(x + w * 0.2 + c * w * 0.28, y + h * 0.45 + r * h * 0.24, 4, 3);
          }
        }
      }
    } else if (type === 'security') {
      // 🛡️ BOUCLIER DE VALIDATION DE SÉCURITÉ
      const glow = `rgba(255, 77, 94, ${0.45 * alpha})`;
      const stroke = `rgba(255, 200, 205, ${0.95 * alpha})`;

      for (let pass = 0; pass < 2; pass++) {
        g.lineWidth = pass === 0 ? 5.0 : 2.2;
        g.strokeStyle = pass === 0 ? glow : stroke;

        g.beginPath();
        g.moveTo(cx, cy - size * 0.55);
        g.lineTo(cx + size * 0.48, cy - size * 0.34);
        g.lineTo(cx + size * 0.40, cy + size * 0.15);
        g.lineTo(cx, cy + size * 0.58);
        g.lineTo(cx - size * 0.40, cy + size * 0.15);
        g.lineTo(cx - size * 0.48, cy - size * 0.34);
        g.closePath();
        g.stroke();

        g.beginPath();
        g.arc(cx, cy - size * 0.06, size * 0.11, 0, TAU);
        g.moveTo(cx, cy);
        g.lineTo(cx, cy + size * 0.22);
        g.stroke();
      }
    } else if (type === 'emotion' && data) {
      // 🌟/💔 GLYPHE HOLOGRAPHIQUE DES 10 NIVEAUX D'ÉMOTIONS (RAG Émotions)
      const EMOTION_TABLE = {
        10: { emoji: '✨', rgb: '255, 220, 90', name: 'Extase', sub: 'Euphorie' },
        9:  { emoji: '😄', rgb: '110, 255, 170', name: 'Joie', sub: 'Chaleur' },
        8:  { emoji: '😌', rgb: '80, 240, 230', name: 'Contentement', sub: 'Bien-être' },
        7:  { emoji: '🧘', rgb: '130, 215, 255', name: 'Sérénité', sub: 'Calme' },
        6:  { emoji: '🥀', rgb: '190, 180, 255', name: 'Mélancolie', sub: 'Nostalgie' },
        5:  { emoji: '😢', rgb: '100, 175, 250', name: 'Chagrin', sub: 'Déception' },
        4:  { emoji: '😞', rgb: '235, 185, 120', name: 'Abattement', sub: 'Découragé' },
        3:  { emoji: '⚡', rgb: '255, 145, 60', name: 'Détresse', sub: 'Angoisse' },
        2:  { emoji: '😫', rgb: '255, 75, 95', name: 'Accablement', sub: 'Épuisement' },
        1:  { emoji: '💔', rgb: '235, 45, 115', name: 'Désespoir', sub: 'Rupture' }
      };
      const score = Math.max(1, Math.min(10, Math.round(Number(data.score) || 7)));
      const info = EMOTION_TABLE[score] || EMOTION_TABLE[7];
      const rgb = info.rgb;
      const glow = `rgba(${rgb}, ${0.5 * alpha})`;
      const stroke = `rgba(${rgb}, ${0.95 * alpha})`;
      const label = data.label ? String(data.label).replace(/^L['’]|^La |^Le /i, '') : info.name;

      // Abaissement vertical proportionnel : plus le score est bas, plus le cœur et l'émotion s'affaissent vers le bas
      const dropY = (7 - score) * size * 0.055;

      // Bobbing ou vibration selon la polarité émotionnelle
      let bob = 0;
      if (score >= 8) {
        bob = Math.sin(t * 5.0) * size * 0.05; // pulsation vive / rebond joyeux
      } else if (score <= 3) {
        bob = (Math.sin(t * 18.0) * 0.4 + Math.cos(t * 26.0) * 0.6) * size * 0.03; // tremblement / stress
      } else {
        bob = Math.sin(t * 2.0) * size * 0.02; // balancement serein ou mélancolique
      }

      // 1. Emoji holographique au centre supérieur
      const px = Math.max(26, Math.round(size * 1.55));
      const holo = emojiHologram(info.emoji, px, rgb);
      const s = holo.size;
      const holoY = cy - size * 0.22 + bob + dropY;
      g.globalAlpha = alpha * (0.7 + 0.15 * Math.sin(t * 6));
      g.drawImage(holo.glow, cx - s / 2, holoY - s / 2, s, s);
      g.globalAlpha = alpha * (0.92 + 0.08 * Math.sin(t * 10));
      g.drawImage(holo.shape, cx - s / 2, holoY - s / 2, s, s);
      g.globalAlpha = 1;

      // 2. Jauge segmentée 1..10 (arc sous l'emoji)
      const gaugeR = size * 0.68;
      const startAngle = Math.PI * 0.75;
      const totalSweep = Math.PI * 1.5;
      const segCount = 10;
      const segGap = 0.04;
      const segAngle = (totalSweep - (segCount - 1) * segGap) / segCount;

      for (let i = 0; i < segCount; i++) {
        const segIdx = i + 1; // 1 à 10
        const a1 = startAngle + i * (segAngle + segGap);
        const a2 = a1 + segAngle;
        const active = segIdx <= score;

        g.beginPath();
        g.arc(cx, cy - size * 0.05 + dropY, gaugeR, a1, a2);
        if (active) {
          g.lineWidth = 3.6;
          g.strokeStyle = stroke;
          g.stroke();
          g.lineWidth = 7.0;
          g.strokeStyle = glow;
          g.stroke();
        } else {
          g.lineWidth = 1.8;
          g.strokeStyle = `rgba(120, 160, 180, ${0.2 * alpha})`;
          g.stroke();
        }
      }

      // 3. Libellé et score pill
      glowText(g, `${score}/10 · ${label.toUpperCase()}`, cx, cy + size * 0.72 + dropY, size * 0.32, alpha);
    }

    // Anneau d'encadrement holographique rotatif
    const ringR = size * 0.90;
    g.lineWidth = 1.2;
    g.strokeStyle = `rgba(160, 245, 255, ${0.35 * alpha})`;
    g.beginPath();
    g.arc(cx, cy, ringR, t * 1.5, t * 1.5 + 1.2);
    g.arc(cx, cy, ringR, t * 1.5 + Math.PI, t * 1.5 + Math.PI + 1.2);
    g.stroke();

    g.restore();
  }

  // Calcul du rayon animé respirant (partagé avec branchGeometry)
  J.computeOrbRadius = function (baseR, t, level, state) {
    const hot = state && (state.capturing || state.speaking);
    const mood = (state && state.mood) || 'idle';
    const load = (state && state.loadFactor) || 0;
    const isThinking = !!(state && state.isThinking);

    // Respiration : lente, majestueuse et très apaisée (uniquement en taille)
    const breathSpeed = isThinking ? 1.4 : (mood === 'overload' ? 1.6 : (0.5 + 0.4 * load));
    const overloadPulse = mood === 'overload' ? 0.015 * Math.sin(t * 2.0) : 0;
    const thinkingPulse = isThinking ? 0.012 * Math.sin(t * 3.0) : 0;
    const breathAmp = 0.008 + 0.012 * load;
    // L'audio ne secoue pas violemment la taille du cœur (la couronne waveform s'en charge visuellement)
    const audioPulse = (hot ? 0.025 : 0.01) * Math.min(1.0, level || 0);
    const breathe = 1 + breathAmp * Math.sin(t * breathSpeed) + overloadPulse + thinkingPulse + audioPulse;
    return baseR * breathe;
  };

  J.drawOrb = function (g, cx, cy, R, t, level, state) {
    const dt = (state && state.dt) || 0.016;
    const mood = (state && state.mood) || 'idle';
    const isThinking = !!(state && state.isThinking);
    // Couleur de conversation prioritaire ; la validation par code (ambre) reste visible pendant
    // que Jarvis « réfléchit » en attendant le code.
    const phase = state && state.voicePhase;
    const usePhase = phase && !(phase === 'think' && (mood === 'awaiting' || mood === 'error' || mood === 'overload'));
    updatePalette(usePhase ? `talk_${phase}` : (mood === 'idle' ? 'talk_rest' : mood), dt, usePhase ? 0.18 : 0.6);

    const Rc = R * 0.52;
    // Petit cœur (widget de 80 px) : blobs et anneaux ondulants font moins de 1 px et coûtent
    // 2 dégradés + 3 tracés de 96 points par image. On les saute.
    const lite = !!(state && state.lite) || R < 60;
    const isOverload = mood === 'overload';
    // Présentation d'un dossier Data en cours (ui/presenter.js) : son panneau et le personnage remplacent le cœur.
    if (!lite && J.heartOverride && J.heartOverride.active()) {
      J.heartOverride.draw(g, cx, cy, R, t, level, state, currentPal.primary);
      return R;
    }
    const pulseFactor = isOverload ? (0.75 + 0.25 * Math.sin(t * 3.5)) : (isThinking ? (0.85 + 0.15 * Math.sin(t * 7.0)) : 1.0);

    // Séquence d'allumage (boot.js) : chaque couche a sa propre progression ; absent = tout visible.
    const rv = (state && state.reveal) || null;

    g.save();
    if (rv) g.globalAlpha *= rv.heart;

    // Corps sphérique sombre + halo
    const body = g.createRadialGradient(cx, cy, R * 0.15, cx, cy, R * 1.12);
    body.addColorStop(0, col('body', 0.9 * pulseFactor));
    body.addColorStop(0.62, col('body', 0.75 * pulseFactor));
    body.addColorStop(0.9, col('body', 0.35 * pulseFactor));
    body.addColorStop(1, col('body', 0));
    g.fillStyle = body;
    g.beginPath();
    g.arc(cx, cy, R * 1.12, 0, TAU);
    g.fill();

    // Veille (app.js) : longtemps sans sollicitation, les traits autour du cœur s'effacent
    // lentement (0 → 1) ; ils reviennent aussitôt qu'on le sollicite.
    const awake = 1 - Math.min(1, Math.max(0, (state && state.sleep) || 0));

    // 1. Zone extérieure avec traits vers l'extérieur : réagit quand l'utilisateur parle
    if (state && !state.noWaveform && awake > 0.02 && (!rv || rv.wave > 0)) { // désactivable (Optimisation)
      g.save();
      g.globalAlpha *= awake;
      waveform(g, cx, cy, R, state, level, t);
      g.restore();
    }

    // 2. Anneaux interactifs : Zone BLEUE (actions internes) et Zone VERTE (actions externes)
    g.save();
    g.globalAlpha *= 0.15 + 0.85 * awake;
    rings(g, cx, cy, R, dt, state);
    g.restore();

    const tickShown = rv ? rv.ticks : 1;
    const tickAlpha = 0.5 * awake + (rv ? 0.5 * rv.flashTicks : 0);
    ticks(g, cx, cy, Rc * 1.12, Rc * 1.32, -125, -55, 11, tickAlpha, 0, tickShown);
    ticks(g, cx, cy, Rc * 1.12, Rc * 1.32, 55, 125, 11, tickAlpha, 0, tickShown);

    // 3. Zone centrale : personnage 3D (réglage « Apparence du cœur », ui/character.js) ou cœur
    // classique. Le personnage remplace le disque, sa vie interne et ses glyphes ; anneaux, couronne
    // vocale et liseré coloré restent.
    const character = state && state.heartStyle && state.heartStyle !== 'classic' &&
      J.character && J.character.ready(state.heartStyle) ? state.heartStyle : null;
    if (character) {
      characterWindow(g, cx, cy, Rc);
    } else {
      // Cœur classique : disque et pupille
      const core = g.createRadialGradient(cx, cy, 0, cx, cy, Rc);
      core.addColorStop(0, col('core0', 1));
      core.addColorStop(0.1, col('core1', 1));
      core.addColorStop(0.34, col('core2', 0.98));
      core.addColorStop(0.58, col('core3', 0.98));
      core.addColorStop(0.76, col('core4', 1));
      core.addColorStop(0.9, col('core5', 1));
      core.addColorStop(1, col('core6', 0.35));
      g.fillStyle = core;
      g.beginPath();
      g.arc(cx, cy, Rc, 0, TAU);
      g.fill();

      // 4. Vie interne et réaction de réflexion de l'IA (entre envoi et réception)
      g.save();
      g.beginPath();
      g.arc(cx, cy, Rc * 0.995, 0, TAU);
      g.clip();
      g.globalCompositeOperation = 'lighter';

      if (!lite) {
        // Un glyphe est affiché (heure, silhouette…) : la vie interne s'efface pour le laisser lisible.
        const glyphOn = currentMorph.active || heldMorph.alpha > 0.05;
        const inner = pulseFactor * (glyphOn ? 0.35 : 1);
        const blobSpeed = isThinking ? 1.2 : 0.35;
        const ringSpeedMult = isThinking ? 2.5 : 1.0;
        blob(g, cx, cy, Rc, t, blobSpeed, 0, (0.1 + level * 0.12) * inner);
        blob(g, cx, cy, Rc, t, -blobSpeed * 0.7, 2.1, (0.08 + level * 0.1) * inner);
        livingRing(g, cx, cy, Rc * 0.5, t * ringSpeedMult, Rc * (isThinking ? 0.08 : 0.05), 0, 2, 0.16 * inner);
        livingRing(g, cx, cy, Rc * 0.68, t * ringSpeedMult, Rc * (isThinking ? 0.07 : 0.04), 1.4, 3, 0.14 * inner);
        livingRing(g, cx, cy, Rc * 0.86, t * ringSpeedMult, Rc * (isThinking ? 0.05 : 0.03), 2.8, 2, 0.12 * inner);
      }

      // Ondes neuronales concentriques quand l'IA réfléchit
      if (isThinking) {
        for (let w = 0; w < 3; w++) {
          const ripplePhase = (t * 2.2 + w * 0.33) % 1.0;
          const rippleR = Rc * ripplePhase;
          const rippleAlpha = (1 - ripplePhase) * 0.70;
          g.lineWidth = 1.8;
          g.strokeStyle = col('core5', rippleAlpha); // teinte de l'humeur (orange pendant un appel IA)
          g.beginPath();
          g.arc(cx, cy, Math.max(1, rippleR), 0, TAU);
          g.stroke();
        }
      }

      // 5. Morphing holographique au cœur selon la demande (heure, animal, note de musique…) :
      // le glyphe passager prime ; le glyphe maintenu (musique) revient en fondu ensuite.
      if (currentMorph.active) {
        const nowMs = performance.now();
        const elapsed = nowMs - currentMorph.startedAt;
        if (elapsed >= currentMorph.duration) {
          currentMorph.active = false;
        } else {
          let alpha = 1.0;
          let scale = 1.0;
          if (elapsed < 350) {
            // Apparition douce
            const p = elapsed / 350;
            alpha = p;
            scale = 0.65 + 0.35 * p;
          } else if (elapsed > currentMorph.duration - 600) {
            // Disparition douce vers le cœur initial
            const p = (elapsed - (currentMorph.duration - 600)) / 600;
            alpha = Math.max(0, 1 - p);
            scale = 1.0 - 0.12 * p;
          } else {
            // Maintien avec respiration holographique (plus discrète pour un texte, qui doit rester lisible)
            alpha = 1.0;
            scale = 1.0 + (currentMorph.type === 'clock' ? 0.008 : 0.03) * Math.sin(t * 7.5);
          }
          drawHolographicGlyph(g, cx, cy, Rc, currentMorph.type, alpha, scale, t, currentMorph.data);
        }
      }
      const heldTarget = heldMorph.type && !currentMorph.active ? 1 : 0;
      heldMorph.alpha += (heldTarget - heldMorph.alpha) * Math.min(1, dt * 3.5);
      if (heldMorph.type) heldMorph.shown = heldMorph.type;
      if (heldMorph.alpha > 0.01 && heldMorph.shown) {
        drawHolographicGlyph(g, cx, cy, Rc, heldMorph.shown, heldMorph.alpha, 1.0, t, null);
      }

      g.restore();

      // Balayage rotatif dans la couronne
      g.save();
      if (rv) g.globalAlpha *= rv.glow;
      g.beginPath();
      g.arc(cx, cy, Rc, 0, TAU);
      g.clip();
      g.globalCompositeOperation = 'lighter';
      g.rotate(0);
      g.strokeStyle = col('primary', 0.25 * pulseFactor);
      g.lineWidth = Rc * 0.06;
      g.beginPath();
      g.arc(cx, cy, Rc * 0.9, t * (isThinking ? 1.6 : 0.6), t * (isThinking ? 1.6 : 0.6) + 1.1);
      g.stroke();
      g.restore();
    }

    // Lueur de bord holographique
    const glowAlpha = (0.5 + level * 0.4) * (isOverload ? pulseFactor : 1.0);
    const glowScale = Math.max(0.65, Math.min(1.0, R / 140));
    g.save();
    if (rv) g.globalAlpha *= Math.min(1, rv.glow + rv.flashGlow);
    g.globalCompositeOperation = 'lighter';
    g.lineWidth = 6.0 * glowScale;
    g.strokeStyle = col('glow', 0.18 * glowAlpha);
    g.beginPath();
    g.arc(cx, cy, Rc * 1.008, 0, TAU);
    g.stroke();

    g.lineWidth = 3.0 * glowScale;
    g.strokeStyle = col('glow', 0.35 * glowAlpha);
    g.beginPath();
    g.arc(cx, cy, Rc * 1.005, 0, TAU);
    g.stroke();

    g.lineWidth = Math.max(0.9, 1.4 * glowScale);
    g.strokeStyle = col('primary', 0.85 * (isOverload ? pulseFactor : 1.0));
    g.beginPath();
    g.arc(cx, cy, Rc * 1.002, 0, TAU);
    g.stroke();
    g.restore();

    // Le personnage passe devant le liseré (sa tête dépasse du hublot). Ses yeux prennent la couleur
    // de la conversation ; au repos, celle du thème plutôt que le gris argenté du cœur (ou la sienne :
    // Marvin garde ses yeux verts).
    if (character) {
      const rest = !usePhase && mood === 'idle';
      J.character.draw(g, cx, cy, Rc, t, level, state, rest ? MOOD_PALETTES.idle.primary : currentPal.primary, rest);
    }

    g.restore();
    return R;
  };

  // Hublot du personnage : disque sombre teinté par l'état de Jarvis, plus clair vers le bord.
  function characterWindow(g, cx, cy, Rc) {
    const grd = g.createRadialGradient(cx, cy - Rc * 0.25, Rc * 0.05, cx, cy, Rc);
    grd.addColorStop(0, col('core2', 0.45));
    grd.addColorStop(0.55, col('core1', 0.75));
    grd.addColorStop(0.86, col('core0', 0.9));
    grd.addColorStop(1, col('core4', 0.55));
    g.fillStyle = grd;
    g.beginPath();
    g.arc(cx, cy, Rc, 0, TAU);
    g.fill();
  }
})();
