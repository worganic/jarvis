// Style de la musique jouée sur le PC : de quoi faire danser les personnages du cœur différemment (character-director.js,
// characters/rig.js). Pas de WebGL ici, rien que de la logique.
//
// Deux sources, la première qui parle gagne :
//   1. le texte : titre, artiste et playlist du lecteur (événement « media ») comparés à une liste de mots-clés ;
//   2. le son : le niveau sonore du PC (événement « pc_level », 50 fois par seconde) donne le tempo (autocorrélation des
//      attaques), l'énergie moyenne et la dynamique, d'où une classe : calme, pop, électro, rock, rap ou latino.
// Le style est lissé : il ne change qu'après quatre analyses (une par seconde) qui disent la même chose.
// Réglage « Danse selon le style de la musique » (administration) : « auto » ou un style imposé.
(function () {
  const J = (window.JARVIS = window.JARVIS || {});

  const STYLES = ['calme', 'pop', 'electro', 'rock', 'rap', 'latino'];
  const STYLE_LABELS = { calme: 'Calme', pop: 'Pop', electro: 'Électro', rock: 'Rock', rap: 'Rap', latino: 'Latino' };
  const RATE = 50; // Hz : un échantillon de niveau toutes les 20 ms
  const WINDOW_S = 8;
  const STABLE_NEEDED = 4;

  const fold = (s) => String(s || '').toLowerCase().normalize('NFD').replace(/[̀-ͯ]/g, '')
    .replace(/[^\p{L}\p{N}\s&]/gu, ' ').replace(/\s+/g, ' ').trim();

  // Mots-clés (sans accents) → style. Le premier style de la liste qui correspond gagne.
  const KEYWORDS = [
    ['calme', /\b(classique|classical|piano|symphon|sonate|mozart|beethoven|chopin|debussy|bach|vivaldi|ambient|relax|lofi|lo fi|chill|jazz|bossa|blues|acoustique|acoustic|ballade|berceuse|lullaby|sleep|meditation|zen|opera|instrumental)\b/],
    ['rock', /\b(rock|metal|punk|grunge|hard rock|heavy|acdc|ac dc|nirvana|metallica|led zeppelin|iron maiden|slipknot|foo fighters|rammstein|muse)\b/],
    ['rap', /\b(rap|hip hop|hiphop|trap|drill|freestyle|booba|jul|ninho|eminem|kendrick|snoop|drake)\b/],
    ['electro', /\b(electro|techno|house|edm|trance|dubstep|drum and bass|dnb|club|rave|daft punk|david guetta|deadmau5|avicii|swedish house|tomorrowland)\b/],
    ['latino', /\b(reggaeton|salsa|bachata|latino|latin|merengue|samba|cumbia|reggae|zumba|shakira|bad bunny|despacito|mambo|tropical|afrobeat|kompa|zouk)\b/],
    ['pop', /\b(pop|disco|funk|variete|hits|top 50|top hits|dance pop)\b/]
  ];

  const samples = []; // { t, v } : niveau du PC (0..1) horodaté en secondes
  let meta = '';
  let mediaPlaying = false;
  let mode = 'auto'; // « auto » ou un style imposé
  let bpm = 0;
  let energy = 0;
  let current = { style: 'pop', source: 'défaut', since: 0 };
  let candidate = null;
  let candidateCount = 0;

  const nowS = () => performance.now() / 1000;

  function fromText() {
    if (!meta) return null;
    for (const [style, re] of KEYWORDS) if (re.test(meta)) return style;
    return null;
  }

  // Niveau rééchantillonné à RATE Hz sur les WINDOW_S dernières secondes (maintien du dernier échantillon connu).
  function series() {
    const end = nowS();
    const n = WINDOW_S * RATE;
    const out = new Float32Array(n);
    let j = 0;
    let last = 0;
    for (let i = 0; i < n; i++) {
      const t = end - (n - 1 - i) / RATE;
      while (j < samples.length && samples[j].t <= t) last = samples[j++].v;
      out[i] = last;
    }
    return out;
  }

  // Tempo : attaques (hausses du niveau) puis autocorrélation sur 70-170 bpm, avec un peu de poids pour le double du pas.
  function tempoOf(v) {
    const n = v.length;
    const o = new Float32Array(n);
    let mean = 0;
    for (let i = 1; i < n; i++) {
      o[i] = Math.max(0, v[i] - v[i - 1]);
      mean += o[i];
    }
    mean /= n;
    for (let i = 0; i < n; i++) o[i] = Math.max(0, o[i] - mean * 0.5);
    const at = (x) => {
      const i = Math.floor(x);
      const f = x - i;
      return i < 0 || i + 1 >= n ? 0 : o[i] * (1 - f) + o[i + 1] * f;
    };
    const ac = (lag) => {
      let s = 0;
      for (let i = 0; i + lag < n - 1; i++) s += o[i] * at(i + lag);
      return s;
    };
    const zero = ac(0) || 1e-9;
    let best = 0;
    let bestScore = 0;
    for (let b = 70; b <= 170; b++) {
      const lag = (60 / b) * RATE;
      const score = ac(lag) + 0.5 * ac(lag * 2);
      if (score > bestScore) {
        bestScore = score;
        best = b;
      }
    }
    return { bpm: best, confidence: bestScore / zero };
  }

  // Style d'après le son seul : heuristique simple (tempo, énergie, dynamique).
  function fromSound(tempo, e, dyn) {
    if (e < 0.35 && tempo < 100) return 'calme';
    if (tempo >= 124 && e >= 0.45 && dyn < 0.12) return 'electro'; // régulier et fort : machine
    if (tempo >= 110 && dyn >= 0.12) return 'rock'; // attaques marquées
    if (tempo >= 82 && tempo <= 100 && e >= 0.4) return 'rap';
    if (tempo >= 98 && tempo <= 112 && dyn >= 0.06) return 'latino';
    return 'pop';
  }

  function analyze() {
    const cutoff = nowS() - WINDOW_S - 1;
    while (samples.length && samples[0].t < cutoff) samples.shift();
    const recent = samples.length && nowS() - samples[samples.length - 1].t < 2;
    if (!recent) return;
    const v = series();
    let sum = 0;
    for (const x of v) sum += x;
    energy = sum / v.length;
    if (energy < 0.05) return; // silence : on garde ce qu'on avait
    let varSum = 0;
    for (const x of v) varSum += (x - energy) * (x - energy);
    const dyn = Math.sqrt(varSum / v.length);
    const t = tempoOf(v);
    if (t.confidence > 0.12) {
      // Tempo lissé ; un écart de plus de 8 % n'est suivi qu'après deux mesures concordantes.
      if (!bpm) bpm = t.bpm;
      else if (Math.abs(t.bpm - bpm) / bpm < 0.08) bpm += (t.bpm - bpm) * 0.3;
      else if (analyze.lastOff && Math.abs(t.bpm - analyze.lastOff) / analyze.lastOff < 0.06) { bpm = t.bpm; analyze.lastOff = 0; }
      else analyze.lastOff = t.bpm;
    }
    let style;
    let source;
    if (mode !== 'auto') {
      style = mode;
      source = 'réglage';
    } else {
      const text = fromText();
      style = text || fromSound(bpm || 110, energy, dyn);
      source = text ? 'texte' : 'son';
    }
    if (style === current.style) {
      candidate = null;
      candidateCount = 0;
      current.source = source;
    } else if (style === candidate) {
      candidateCount++;
      if (candidateCount >= STABLE_NEEDED || mode !== 'auto') {
        current = { style, source, since: performance.now() };
        candidate = null;
        candidateCount = 0;
      }
    } else {
      candidate = style;
      candidateCount = 1;
    }
  }

  setInterval(() => { try { analyze(); } catch { /* analyse facultative */ } }, 1000);

  function setMode(m) {
    mode = m === 'auto' || STYLES.includes(m) ? m : 'auto';
    if (mode !== 'auto') current = { style: mode, source: 'réglage', since: performance.now() };
  }

  J.musicStyle = {
    STYLES,
    LABELS: STYLE_LABELS,
    // { style, bpm, energy, source, changedAt } ; bpm vaut 120 tant que le tempo n'est pas mesuré.
    current: () => ({ style: current.style, bpm: Math.round(bpm || 120), energy, source: current.source, changedAt: current.since }),
    setMode,
    // Pour les essais : injecte un niveau comme le ferait « pc_level ».
    setMeta: (text) => { meta = fold(text); },
    reset: () => { samples.length = 0; bpm = 0; candidate = null; candidateCount = 0; current = { style: 'pop', source: 'défaut', since: 0 }; },
    feed: (v, t = nowS()) => { samples.push({ t, v: Math.max(0, Math.min(1, Number(v) || 0)) }); },
    analyze
  };

  try { setMode(localStorage.getItem('jarvis.danceStyle') || 'auto'); } catch { /* réglage facultatif */ }

  const bridge = window.jarvisBridge;
  if (!bridge || !bridge.onTaskEvent) return;
  bridge.onTaskEvent((ev) => {
    if (!ev) return;
    if (ev.type === 'pc_level') {
      samples.push({ t: nowS(), v: Math.max(0, Math.min(1, Number(ev.v) || 0)) });
    } else if (ev.type === 'media') {
      mediaPlaying = Boolean(ev.active && ev.status === 'playing');
      meta = fold(`${ev.title || ''} ${ev.artist || ''} ${ev.playlist || ''}`);
    } else if (ev.type === 'dance_style_config') {
      setMode(ev.style);
      try { localStorage.setItem('jarvis.danceStyle', mode); } catch { /* mémorisation facultative */ }
    } else if (ev.type === 'status' && ev.danceStyle) {
      setMode(ev.danceStyle);
      try { localStorage.setItem('jarvis.danceStyle', mode); } catch { /* mémorisation facultative */ }
    }
  });
  J.musicStyle.isPlaying = () => mediaPlaying;
})();
