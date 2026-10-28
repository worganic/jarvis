// Personnage du cœur : quelle animation jouer, selon ce que fait Jarvis.
// Le dessin 3D est dans ui/character.js (scène, rendu) et ui/characters/<id>.js (modèle, gestes) ;
// ce fichier ne contient que la logique, sans WebGL (testé par tools/test-heart-style.js).
//
// Priorités (la première qui s'applique gagne) :
//   l'utilisateur parle                       → listen  (main à l'oreille)
//   geste en cours (demande, glyphe du cœur)  → mail, globe, code, calendar, clock, show, dance, wave…
//   échec récent                              → shrug
//   code de validation attendu                → guard
//   Jarvis répond                             → talk
//   Jarvis réfléchit (appel IA)               → think
//   tâche réussie                             → happy
//   musique en cours                          → dance (6 styles selon la musique : ui/music-style.js)
//   quota presque épuisé                      → tired
//   tâches en cours                           → work
//   veille                                    → sleep
//   sinon                                     → idle
(function () {
  const J = (window.JARVIS = window.JARVIS || {});
  const fold = (s) => String(s || '').toLowerCase().normalize('NFD').replace(/[̀-ͯ]/g, '')
    .replace(/[^\p{L}\p{N}\s]/gu, ' ').replace(/\s+/g, ' ').trim();

  // Apparences du cœur. Un nouveau personnage : une entrée ici, un fichier ui/characters/<id>.js,
  // et la même entrée dans HEART_STYLES (orchestrator/status.js).
  J.HEART_STYLES = {
    classic: 'Cœur holographique',
    bobot: 'Bobot (robot à visière)',
    marvin: 'Marvin (robot à grosse tête)',
    petitrobot: 'Petit Robot (robot rouge)',
    chat: 'Chat grincheux'
  };

  // Toutes les animations qu'un personnage doit savoir jouer (une animation absente retombe sur idle).
  const ACTIONS = ['idle', 'sleep', 'listen', 'think', 'talk', 'dance', 'mail', 'globe', 'code', 'calendar',
    'clock', 'show', 'guard', 'shrug', 'happy', 'tired', 'work', 'wave', 'bow',
    'extase', 'joie', 'contentement', 'serenite', 'melancolie', 'chagrin', 'abattement', 'detresse', 'accablement', 'desespoir'];

  // Glyphe du cœur classique (orb.js, orb-morph.js) → geste du personnage.
  const MORPH_ACTIONS = {
    music: 'dance', mail: 'mail', browser: 'globe', web: 'globe', code: 'code', terminal: 'code',
    calendar: 'calendar', agenda: 'calendar', security: 'guard', clock: 'clock', emoji: 'show',
    extase: 'extase', joie: 'joie', contentement: 'contentement', serenite: 'serenite',
    melancolie: 'melancolie', chagrin: 'chagrin', abattement: 'abattement', detresse: 'detresse',
    accablement: 'accablement', desespoir: 'desespoir'
  };
  // Un glyphe dure 2,4 s par défaut : un geste a besoin d'un peu plus pour se lire.
  const MIN_MORPH_MS = 4500;

  // Phrases de l'utilisateur → geste (dont les 10 niveaux d'émotions RAG).
  const TEXT_CUES = [
    [/^(?:(?:dis |hey |ok )?jarvis )?(?:bonjour|salut|coucou|hello|bonsoir|hey|yo)\b/, 'wave', 3200],
    [/\b(?:au revoir|bonne nuit|a plus tard|a demain|bye|a tout a l heure)\b/, 'wave', 3200],
    [/\bmerci\b/, 'bow', 2600],
    [/\b(?:danse|danses|danser|dansons|fais (?:moi |nous )?une danse)\b/, 'dance', 9000],
    [/\b(?:bravo|parfait)\b/, 'happy', 2800],
    // 10 : Extase
    [/\b(?:genial|ca marche au top|extraordinaire|merveilleux|magique)\b/, 'extase', 4500],
    // 9 : Joie
    [/\b(?:super|trop bien|excellent|bien joue|trop fort|felicitations)\b/, 'joie', 4000],
    // 8 : Contentement
    [/\b(?:tous? va bien|c est bon|impeccable|nickel|parfaitement satisfait)\b/, 'contentement', 4000],
    // 7 : Sérénité
    [/\b(?:tranquille|zen|aucun probleme|calme et pose|tout est en ordre)\b/, 'serenite', 4000],
    // 6 : Mélancolie
    [/\b(?:bof|comme d habitude|nostalgique|vague a l ame|sans plus)\b/, 'melancolie', 4000],
    // 5 : Chagrin
    [/\b(?:decu|c est dommage|triste|peine de coeur|mauvaise nouvelle)\b/, 'chagrin', 4000],
    // 4 : Abattement
    [/\b(?:j en ai marre|ras le bol|fatigue|decourage|trop epuise)\b/, 'abattement', 4000],
    // 3 : Détresse
    [/\b(?:bouge toi|panique|au secours|a l aide|je sais plus quoi faire)\b/, 'detresse', 4000],
    // 2 : Accablement
    [/\b(?:tu fai[st] n importe quoi|merde|trop lourd|insupportable|j en peux plus)\b/, 'accablement', 4000],
    // 1 : Désespoir
    [/\b(?:tu fais chier|c est foutu|aucun espoir|irrecuperable|tout est perdu)\b/, 'desespoir', 4500]
  ];
  // Paroles de Jarvis → geste (annonces spontanées : nouveau mail, rappel).
  const SAY_CUES = [
    [/\bnouveaux? (?:mails?|messages?|courriels?)\b/, 'mail', 5500],
    [/^(?:rappel|c est l heure)\b/, 'clock', 5000]
  ];

  const cue = { action: null, startedAt: 0, until: 0, data: null };

  // Geste explicite (ms : durée). Remplace le geste en cours.
  function play(action, ms = 3000, data = null, now = nowMs()) {
    if (!ACTIONS.includes(action)) return false;
    cue.action = action;
    cue.startedAt = now;
    cue.until = now + Math.max(500, ms);
    cue.data = data;
    return true;
  }

  function nowMs() {
    return typeof performance !== 'undefined' ? performance.now() : Date.now();
  }

  function matchCue(list, text) {
    const t = fold(text);
    if (!t) return null;
    for (const [re, action, ms] of list) if (re.test(t)) return { action, ms };
    return null;
  }

  // Phrase de l'utilisateur (dite ou tapée) : geste éventuel. Renvoie l'animation lancée.
  function cueForText(text, now) {
    const hit = matchCue(TEXT_CUES, text);
    if (!hit) return null;
    play(hit.action, hit.ms, null, now);
    return hit.action;
  }

  function cueForSay(text, now) {
    const hit = matchCue(SAY_CUES, text);
    if (!hit) return null;
    play(hit.action, hit.ms, null, now);
    return hit.action;
  }

  // Geste en cours : le plus récent entre le geste explicite et le glyphe du cœur.
  function activeCue(morph, now) {
    let best = null;
    if (cue.action && now < cue.until) best = { action: cue.action, startedAt: cue.startedAt, until: cue.until, data: cue.data };
    if (morph && morph.type) {
      const act = (morph.type === 'emotion' && morph.data && morph.data.action) ? morph.data.action : MORPH_ACTIONS[morph.type];
      if (act && ACTIONS.includes(act)) {
        const until = morph.startedAt + Math.max(morph.duration || 0, MIN_MORPH_MS);
        if (now < until && (!best || morph.startedAt >= best.startedAt)) {
          best = { action: act, startedAt: morph.startedAt, until, data: { type: morph.type, ...(morph.data || {}) } };
        }
      }
    }
    return best;
  }

  // Style et tempo de la musique (ui/music-style.js) : la danse en dépend.
  function danceData() {
    const m = J.musicStyle ? J.musicStyle.current() : null;
    return m ? { style: m.style, bpm: m.bpm, changedAt: m.changedAt } : null;
  }

  // s : { mood, voicePhase, thinking, speaking, sleep, musicPlaying, morph, now }
  // Renvoie { action, data, startedAt } : l'animation à jouer à cet instant.
  function pick(s = {}) {
    const now = s.now !== undefined ? s.now : nowMs();
    const phase = s.voicePhase || null;
    if (phase === 'user' || phase === 'wake') return { action: 'listen', data: { wake: phase === 'wake' } };
    const c = activeCue(s.morph, now);
    if (c) return c.action === 'dance' ? { ...c, data: { ...(c.data || {}), ...(danceData() || {}) } } : c;
    const mood = s.mood || 'idle';
    if (mood === 'error') return { action: 'shrug' };
    if (mood === 'awaiting') return { action: 'guard' };
    if (phase === 'reply' || s.speaking) return { action: 'talk' };
    if (phase === 'think' || s.thinking || mood === 'external') return { action: 'think' };
    if (mood === 'satisfied') return { action: 'happy' };
    // Compagnon de bureau (option) : seulement quand Jarvis n'écoute, ne parle et ne réfléchit pas.
    const comp = s.companion && s.companion.enabled ? s.companion : null;
    const react = comp ? companionReaction(comp) : null;
    if (react) return react;
    if (s.musicPlaying) return { action: 'dance', data: danceData() };
    if (mood === 'overload') return { action: 'tired' };
    if (mood === 'working') return { action: 'work' };
    const life = comp ? companionMood(comp, now) : null;
    if (life) return life;
    if ((s.sleep || 0) > 0.55) return { action: 'sleep' };
    return { action: 'idle' };
  }

  // Réaction brève du compagnon (repas, caresse, éclosion…) : passe devant la danse et le travail.
  const FLASH_ACTIONS = {
    friandise: 'happy', legumes: 'happy', grignotage: 'happy', croquettes: 'joie', epice: 'shrug',
    ronron: 'joie', rassasie: 'bow', hatch: 'extase', evolve: 'extase', retire: 'wave'
  };
  function companionReaction(c) {
    const act = c.flash && FLASH_ACTIONS[c.flash];
    return act ? { action: act, data: { companion: c.flash } } : null;
  }

  // État durable du compagnon : sommeil, fatigue, faim, bouderie. Le bébé fait souvent la sieste.
  function companionMood(c, now) {
    const fx = new Set(c.effects || []);
    if (c.stage === 'oeuf') return { action: 'idle', data: { companion: 'oeuf' } };
    if (fx.has('asleep') || fx.has('afk') || fx.has('stealth')) return { action: 'sleep' };
    if (fx.has('hot') || fx.has('tired') || fx.has('night') || fx.has('lowbattery')) return { action: 'tired' };
    if (fx.has('sulky')) return { action: 'chagrin' };
    if (fx.has('hungry') || fx.has('offline')) return { action: 'melancolie' };
    if (c.stage === 'bebe' && Math.floor(now / 60000) % 5 < 2) return { action: 'sleep' };
    return null;
  }

  function reset() {
    cue.action = null;
    cue.until = 0;
  }

  J.characterDirector = { pick, play, cueForText, cueForSay, reset, ACTIONS, MORPH_ACTIONS };
  J.characterCue = play;
  J.characterCueForText = (text) => cueForText(text);

  // Paroles de Jarvis (annonce d'un nouveau mail…). Les phrases de l'utilisateur passent par
  // J.morphForText (orb-morph.js), appelé pour les demandes dites comme tapées.
  const bridge = typeof window !== 'undefined' && window.jarvisBridge;
  if (bridge && bridge.onTaskEvent) {
    bridge.onTaskEvent((ev) => {
      if (!ev) return;
      if (ev.type === 'say' && ev.text) cueForSay(ev.text);
      else if (ev.type === 'emotion' && ev.action) play(ev.action, 6500, { score: ev.score, name: ev.name });
    });
  }
})();
