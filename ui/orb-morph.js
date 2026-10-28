// Transformations du cœur selon la demande (dessins : orb.js) :
//   « quelle heure est-il ? »        → 22h23, anneau des secondes, date dessous
//   « quel jour sommes-nous ? »      → JEU. 24 SEPT.
//   « quel temps fait-il ? »         → pictogramme météo, puis température lue dans la réponse
//   un animal, un objet (« mon chien », « une fusée ») → silhouette holographique
//   musique en cours                  → note qui danse sur le son réel du PC (pc_level, moteur vocal)
// Chargé par le HUD et par le widget : chacun anime son propre cœur.
(function () {
  const J = (window.JARVIS = window.JARVIS || {});
  const fold = (s) => String(s || '').toLowerCase().normalize('NFD').replace(/[̀-ͯ]/g, '')
    .replace(/['’\-]/g, ' ').replace(/\s+/g, ' ');

  // Animaux : forme entière plutôt que tête quand l'emoji existe (la silhouette se reconnaît mieux).
  const SUBJECTS = [
    ['🐕', 'chien|chiens|chiot|chiots|toutou|labrador|berger allemand|caniche'], ['🐈', 'chat(?! ?gpt)|chats|chaton|chatons|minou'],
    ['🐎', 'cheval|chevaux|poney|poneys|jument|etalon'], ['🐄', 'vache|vaches'], ['🐂', 'taureau|taureaux|boeuf|boeufs'],
    ['🐖', 'cochon|cochons|porc|porcs|truie'], ['🐑', 'mouton|moutons|brebis|agneau|agneaux'], ['🐐', 'chevre|chevres|bouc'],
    ['🐓', 'poule|poules|coq|coqs|poulet|poulets'], ['🐤', 'poussin|poussins'], ['🦆', 'canard|canards'], ['🦃', 'dinde|dindon'],
    ['🐇', 'lapin|lapins|lievre|lievres'], ['🐀', 'rat|rats'], ['🐹', 'hamster|hamsters|cochon d inde'], ['🐿️', 'ecureuil|ecureuils'],
    ['🦔', 'herisson|herissons'], ['🦊', 'renard|renards'], ['🐺', 'loup|loups'], ['🐻', 'ours'], ['🐼', 'panda|pandas'],
    ['🐨', 'koala|koalas'], ['🦁', 'lion|lions|lionne'], ['🐅', 'tigre|tigres'], ['🐆', 'leopard|leopards|guepard|guepards|jaguar|panthere'],
    ['🐘', 'elephant|elephants'], ['🦒', 'girafe|girafes'], ['🦓', 'zebre|zebres'], ['🦏', 'rhinoceros|rhino'], ['🦛', 'hippopotame|hippopotames|hippo'],
    ['🐪', 'dromadaire|dromadaires'], ['🐫', 'chameau|chameaux'], ['🦘', 'kangourou|kangourous'], ['🐒', 'singe|singes|chimpanze'],
    ['🦍', 'gorille|gorilles'], ['🦧', 'orang outan'], ['🦌', 'cerf|cerfs|biche|biches|renne|rennes|chevreuil'], ['🐗', 'sanglier|sangliers'],
    ['🦫', 'castor|castors'], ['🦇', 'chauve souris'], ['🦅', 'aigle|aigles|faucon|faucons|vautour'], ['🦉', 'hibou|hiboux|chouette|chouettes'],
    ['🦜', 'perroquet|perroquets|perruche'], ['🕊️', 'colombe|pigeon|pigeons'], ['🐧', 'pingouin|pingouins|manchot|manchots'],
    ['🦩', 'flamant rose|flamants roses'], ['🦚', 'paon|paons'], ['🦢', 'cygne|cygnes'], ['🐦', 'oiseau|oiseaux|moineau|merle'],
    ['🐬', 'dauphin|dauphins'], ['🐋', 'baleine|baleines'], ['🦈', 'requin|requins'], ['🐟', 'poisson|poissons'], ['🐙', 'pieuvre|poulpe|poulpes'],
    ['🦀', 'crabe|crabes'], ['🦞', 'homard|homards'], ['🐢', 'tortue|tortues'], ['🐍', 'serpent|serpents|vipere'],
    ['🐊', 'crocodile|crocodiles|alligator'], ['🦎', 'lezard|lezards|gecko|iguane'], ['🐸', 'grenouille|grenouilles|crapaud'],
    ['🦕', 'dinosaure|dinosaures|diplodocus'], ['🦖', 't rex|tyrannosaure'], ['🦋', 'papillon|papillons'], ['🐝', 'abeille|abeilles|guepe'],
    ['🐜', 'fourmi|fourmis'], ['🕷️', 'araignee|araignees'], ['🦂', 'scorpion|scorpions'], ['🐌', 'escargot|escargots|limace'],
    ['🐞', 'coccinelle|coccinelles'], ['🦟', 'moustique|moustiques'], ['🦄', 'licorne|licornes'], ['🐉', 'dragon|dragons'],
    // Objets et thèmes courants
    // (« train » seul : « je suis en train de… » ; « auto » : « sourdine auto ».)
    ['🚗', 'voiture|voitures'], ['✈️', 'avion|avions'], ['🚆', 'le train|un train|du train|trains|tgv'], ['🚲', 'velo|velos|bicyclette'],
    ['🏍️', 'moto|motos'], ['⛵', 'bateau|bateaux|voilier'], ['🚀', 'fusee|fusees'], ['🏠', 'maison|maisons'], ['🌳', 'arbre|arbres'],
    ['🌸', 'fleur|fleurs'], ['🍕', 'pizza|pizzas'], ['☕', 'cafe'], ['🎂', 'gateau|anniversaire'], ['🎁', 'cadeau|cadeaux'],
    ['🌙', 'lune'], ['⭐', 'etoile|etoiles'], ['🪐', 'planete|planetes|saturne'], ['🌍', 'la terre|planete terre'],
    ['⚽', 'foot|football|ballon'], ['🏀', 'basket'], ['📱', 'telephone|smartphone'], ['💰', 'argent|euros'], ['🔥', 'feu|incendie'],
    ['⛰️', 'montagne|montagnes'], ['🏖️', 'plage|plages'], ['🎄', 'sapin|noel'], ['👻', 'fantome|fantomes'], ['🤖', 'robot|robots'],
    ['👽', 'extraterrestre|alien']
  ].map(([emoji, words]) => ({ emoji, re: new RegExp(`\\b(?:${words})\\b`) }));

  const WEATHER = [
    [/orage/, '⛈️'], [/neige|neig/, '❄️'], [/pluie|pleu|averse|bruine/, '🌧️'], [/brouillard|brume/, '🌫️'],
    [/partiellement|eclaircie|voile|peu nuageux/, '⛅'], [/nuage|couvert/, '☁️'], [/soleil|degage|ensoleille|beau temps/, '☀️']
  ];

  let weatherAskedAt = 0;

  // Sujet cité le plus tôt dans la phrase (« un chat et un chien » → le chat).
  function subjectOf(t) {
    let best = null;
    for (const s of SUBJECTS) {
      const m = s.re.exec(t);
      if (m && (!best || m.index < best.index)) best = { emoji: s.emoji, index: m.index };
    }
    return best ? best.emoji : null;
  }

  // Phrase de l'utilisateur : true si le cœur a pris une forme.
  J.morphForText = function (text) {
    // Personnage du cœur : « bonjour » → il salue, « merci » → il s'incline… (character-director.js)
    if (J.characterCueForText) J.characterCueForText(text);
    if (!J.triggerOrbMorph) return false;
    const t = ` ${fold(text)} `;
    if (/\b(?:quelle heure|l heure|heure est il|heure il est)\b/.test(t)) {
      J.triggerOrbMorph('clock', 9000, { mode: 'time' }, 2);
      return true;
    }
    if (/\b(?:quel jour|quelle date|on est le combien|la date|sommes nous)\b/.test(t)) {
      J.triggerOrbMorph('clock', 9000, { mode: 'date' }, 2);
      return true;
    }
    if (/\b(?:meteo|quel temps|temps fait il|temps fera t il|temps va t il faire|va t il pleuvoir|pleut il|il pleut|quelle temperature)\b/.test(t)) {
      weatherAskedAt = performance.now();
      J.triggerOrbMorph('emoji', 12000, { emoji: '⛅' }, 2);
      return true;
    }
    const emoji = subjectOf(t);
    if (emoji) {
      J.triggerOrbMorph('emoji', 7000, { emoji }, 2);
      return true;
    }
    return false;
  };

  // Réponse de Jarvis : la météo donne son pictogramme et sa température (« 23 degrés »).
  J.morphOnReply = function (text) {
    if (!weatherAskedAt || performance.now() - weatherAskedAt > 20000 || !J.triggerOrbMorph) return;
    const t = fold(text);
    const deg = t.match(/(-?\d+)\s*(?:degres|°)/);
    const icon = (WEATHER.find(([re]) => re.test(t)) || [])[1];
    if (!deg && !icon) return;
    weatherAskedAt = 0;
    J.triggerOrbMorph('emoji', 9000, { emoji: icon || '⛅', caption: deg ? `${deg[1]}°` : '' }, 2);
  };

  const bridge = window.jarvisBridge;
  if (!bridge || !bridge.onTaskEvent) return;
  bridge.onTaskEvent((ev) => {
    if (!ev) return;
    if (ev.type === 'voice_final') {
      if (ev.accepted && !ev.echo) J.morphForText(ev.text);
    } else if (ev.type === 'say') {
      if (ev.text) J.morphOnReply(ev.text);
    } else if (ev.type === 'emotion') {
      if (J.triggerOrbMorph) J.triggerOrbMorph('emotion', 6500, { score: ev.score, action: ev.action, label: ev.name }, 2);
    } else if (ev.type === 'pc_level') {
      J.musicLevel = { v: Math.max(0, Math.min(1, Number(ev.v) || 0)), at: performance.now() };
    } else if (ev.type === 'media') {
      if (J.setOrbMorphHold) J.setOrbMorphHold('music', Boolean(ev.active && ev.status === 'playing'));
    }
  });
})();
