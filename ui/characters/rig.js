// Squelette et animations communs aux personnages du cœur (ui/character.js).
//
// Un personnage (ui/characters/<id>.js) fournit ses proportions (dims), les positions de ses mains et
// de ses accessoires pour chaque geste (anchors), son modèle (build : maillages accrochés aux
// articulations, visage) et, s'il veut, sa personnalité (tune : retouche de chaque pose).
//
// Articulations : racine (pieds à l'origine, face vers +z) › bassin › buste › cou › tête ; bras (épaule,
// coude, poignet) ; jambes (hanche, genou, pied). Chaque animation calcule une pose à partir du temps :
// angles du buste et de la tête, position des mains (cinématique inverse à deux segments, plus simple
// à régler que des angles), jambes, expression et accessoires (ui/characters/props.js). Le passage
// d'une animation à l'autre est un fondu de 0,45 s ; clignements, parole et respiration s'ajoutent.
//
// anchors.hand : mains, repère du buste, données pour la main DROITE du personnage (x < 0) ; la gauche
// est le miroir. anchors.prop : accessoires, repère du personnage (« panel » : à droite de l'écran).

export const ACTION_NAMES = ['idle', 'sleep', 'listen', 'think', 'talk', 'dance', 'mail', 'globe', 'code', 'work',
  'calendar', 'clock', 'show', 'guard', 'shrug', 'happy', 'tired', 'wave', 'bow',
  'extase', 'joie', 'contentement', 'serenite', 'melancolie', 'chagrin', 'abattement', 'detresse', 'accablement', 'desespoir'];

const TAU = Math.PI * 2;
export const clamp = (v, a, b) => Math.max(a, Math.min(b, v));
export const smooth = (x) => {
  const k = clamp(x, 0, 1);
  return k * k * (3 - 2 * k);
};
export const pulse = (a, t0, t1) => smooth((a - t0) / Math.max(0.001, t1 - t0));
// Bruit lisse 1D (somme de sinus incommensurables) : regards, balancements.
export const wander = (t, s = 0) => 0.55 * Math.sin(t * 0.61 + s) + 0.3 * Math.sin(t * 1.37 + s * 2.1) + 0.15 * Math.sin(t * 2.71 + s * 3.7);
const mix3 = (a, b, w) => [a[0] + (b[0] - a[0]) * w, a[1] + (b[1] - a[1]) * w, a[2] + (b[2] - a[2]) * w];
const mirror = (v) => [-v[0], v[1], v[2]];

// Part du regard vers une card conservée selon le geste : tête libre au repos, presque rien quand il
// écoute (il fait face à celui qui parle), rien quand il dort.
const GAZE_WEIGHT = {
  default: 0.6, idle: 1, work: 1, dance: 0.8, happy: 0.8, tired: 0.6, think: 0.6, talk: 0.45, listen: 0.1, sleep: 0,
  guard: 0.4, shrug: 0.5, bow: 0.2, wave: 0.3
};

// Direction des coudes (sans unité : valable pour tous les personnages).
const R_POLE = [-0.7, 0.2, -1];
const R_HIP_POLE = [-1, 0.1, -0.2];

function basePose(H) {
  return {
    rootX: 0, rootY: 0, rootRotY: 0, rootRotZ: 0,
    torsoLean: 0.02, torsoSway: 0, torsoTwist: 0,
    headNod: 0, headTurn: 0, headTilt: 0,
    rH: [...H.rest], lH: mirror(H.rest), rP: [...R_POLE], lP: mirror(R_POLE),
    rThumb: 0,
    rHipFwd: 0, rHipOut: 0.03, rKnee: 0, lHipFwd: 0, lHipOut: 0.03, lKnee: 0,
    eyeOpen: 1, eyeHappy: 0, eyeTilt: 0, eyeX: 0, eyeY: 0, eyeScale: 1,
    mouthOpen: 0, mouthSmile: 0.7, blush: 0, glow: 1
  };
}

export function lerpPose(a, b, w) {
  const out = {};
  for (const k of Object.keys(b)) {
    const va = a[k];
    const vb = b[k];
    if (Array.isArray(vb)) out[k] = vb.map((x, i) => (Array.isArray(va) ? va[i] + (x - va[i]) * w : x));
    else out[k] = typeof va === 'number' ? va + (vb - va) * w : vb;
  }
  return out;
}

// --- Animations ----------------------------------------------------------------------------------------
// p : pose à modifier ; a : secondes depuis le début de l'animation ; t : horloge ; k : contexte
// ({ level, music, data, anchors, show(nom, [x, y, z], opts) }).

function makeActions(A) {
  const H = A.hand;
  const P = A.prop;
  const onHip = (p, side) => {
    if (side === 'r') {
      p.rH = [...H.hip];
      p.rP = [...R_HIP_POLE];
    } else {
      p.lH = mirror(H.hip);
      p.lP = mirror(R_HIP_POLE);
    }
  };

  // Mains qui tapent sur un clavier posé devant le buste.
  function typing(p, t, speed) {
    const f = 15 * speed + 4;
    p.rH = [H.type[0] + 0.03 * Math.sin(t * f * 0.37), H.type[1] + 0.035 * Math.max(0, Math.sin(t * f)), H.type[2]];
    p.lH = [-H.type[0] + 0.03 * Math.sin(t * f * 0.41 + 1), H.type[1] + 0.035 * Math.max(0, Math.sin(t * f + 1.9)), H.type[2]];
    p.rP = [-1, -0.6, -0.3];
    p.lP = [1, -0.6, -0.3];
    p.torsoLean = 0.08;
    p.eyeOpen = 0.82;
    p.mouthSmile = 0.4;
  }

  // Présente un panneau d'une main (« l » : panneau à droite de l'écran), l'autre main sur la hanche.
  function present(p, side, t) {
    const s = side === 'l' ? 1 : -1;
    const hand = [-H.present[0] * s, H.present[1] + 0.02 * Math.sin(t * 1.5), H.present[2]];
    if (s > 0) {
      p.lH = hand;
      p.lP = [1, -0.7, -0.2];
      onHip(p, 'r');
    } else {
      p.rH = hand;
      p.rP = [-1, -0.7, -0.2];
      onHip(p, 'l');
    }
    p.headTurn = 0.36 * s;
    p.headNod = 0.02;
    p.eyeX = 0.03 * s;
    p.mouthSmile = 0.9;
  }

  function dancePose(move, beat, amp) {
    const p = basePose(H);
    const ph = beat * Math.PI;
    const bend = 0.5 + 0.5 * Math.cos(TAU * beat); // genoux pliés sur le temps
    p.rHipFwd = p.lHipFwd = 0.2 * bend * amp;
    p.rKnee = p.lKnee = 0.42 * bend * amp;
    p.headNod = 0.1 * bend * amp;
    const s = Math.sin(ph);
    if (move === 0) {
      // Poings levés en alternance.
      p.rH = mix3(H.danceDown, H.danceUp, Math.max(0, s) * amp);
      p.lH = mix3(mirror(H.danceDown), mirror(H.danceUp), Math.max(0, -s) * amp);
      p.rP = [-1, -0.2, -0.6];
      p.lP = [1, -0.2, -0.6];
      p.torsoSway = 0.08 * s * amp;
    } else if (move === 1) {
      // Disco : un doigt vers le ciel, puis vers le sol.
      p.rH = mix3(H.discoLow, H.discoHigh, 0.5 + 0.5 * Math.tanh(3 * s));
      p.rP = [-1, -0.3, -0.4];
      onHip(p, 'l');
      p.rootX = 0.06 * s * amp;
      p.torsoSway = -0.1 * s * amp;
      p.headTilt = 0.12 * s;
    } else if (move === 2) {
      // Robot : avant-bras à angle droit qui montent et descendent en opposition.
      const q = Math.tanh(4 * s);
      p.rH = [H.robot[0], H.robot[1] + 0.22 * q, H.robot[2]];
      p.lH = [-H.robot[0], H.robot[1] - 0.22 * q, H.robot[2]];
      p.rP = [-1, -0.5, -0.5];
      p.lP = [1, -0.5, -0.5];
      p.headTurn = 0.3 * q;
      p.rootRotY = 0.14 * q;
    } else {
      // Pas chassés : une jambe puis l'autre, bras qui balancent, buste qui pivote.
      const half = Math.sin(ph / 2);
      p.rootX = 0.1 * half * amp;
      p.rootRotY = 0.45 * half;
      p.rHipFwd = 0.3 * Math.max(0, s) * amp;
      p.rKnee = 0.65 * Math.max(0, s) * amp;
      p.lHipFwd = 0.3 * Math.max(0, -s) * amp;
      p.lKnee = 0.65 * Math.max(0, -s) * amp;
      p.rH = [H.step[0], H.step[1] + 0.16 * s, H.step[2] + 0.12 * s];
      p.lH = [-H.step[0], H.step[1] - 0.16 * s, H.step[2] - 0.12 * s];
      p.rP = [-1, -0.3, -0.6];
      p.lP = [1, -0.3, -0.6];
    }
    p.rootY = 0.025 * bend * amp;
    return p;
  }

  // Danses par style de musique : chacune renvoie une pose complète pour le temps `beat` (en battements) et l'amplitude `amp`.
  const STYLE_DANCES = {
    // Calme : balancement lent sur quatre temps, bras qui se balancent, genoux à peine pliés.
    calme(beat, amp) {
      const p = basePose(H);
      const w = Math.sin((beat * Math.PI) / 2);
      p.rootX = 0.05 * w * amp;
      p.torsoSway = 0.07 * w * amp;
      p.headTilt = 0.1 * w;
      p.headNod = 0.04 * Math.sin(beat * Math.PI);
      p.rH = [H.step[0], H.step[1] + 0.1 * w, H.step[2] + 0.08 * w];
      p.lH = [-H.step[0], H.step[1] - 0.1 * w, H.step[2] - 0.08 * w];
      p.rP = [-1, -0.3, -0.6];
      p.lP = [1, -0.3, -0.6];
      p.rHipFwd = p.lHipFwd = 0.05 * (0.5 + 0.5 * Math.cos(beat * Math.PI)) * amp;
      p.rKnee = p.lKnee = 0.12 * (0.5 + 0.5 * Math.cos(beat * Math.PI)) * amp;
      return p;
    },
    // Électro : rebond rapide, poings qui pompent un temps sur deux, les deux bras en l'air sur le « drop » (tous les 8 temps).
    electro(beat, amp) {
      const p = basePose(H);
      const bend = 0.5 + 0.5 * Math.cos(Math.PI * 2 * beat);
      const s = Math.sin(Math.PI * beat);
      const drop = smooth(((beat % 8) - 6) / 0.4) * (1 - smooth(((beat % 8) - 7.8) / 0.2));
      p.rootY = 0.04 * bend * amp;
      p.rHipFwd = p.lHipFwd = 0.18 * bend * amp;
      p.rKnee = p.lKnee = 0.38 * bend * amp;
      p.headNod = 0.12 * bend * amp;
      p.rH = mix3(mix3(H.danceDown, H.danceUp, Math.max(0, s) * amp), H.danceUp, drop);
      p.lH = mix3(mix3(mirror(H.danceDown), mirror(H.danceUp), Math.max(0, -s) * amp), mirror(H.danceUp), drop);
      p.rP = [-1, -0.2, -0.6];
      p.lP = [1, -0.2, -0.6];
      p.torsoSway = 0.05 * s * amp;
      return p;
    },
    // Rock : tête qui frappe le temps, guitare imaginaire, cornes levées tous les 8 temps.
    rock(beat, amp) {
      const p = basePose(H);
      const bang = Math.max(0, Math.sin(Math.PI * beat));
      const horns = smooth(((beat % 8) - 7) / 0.3) * (1 - smooth(((beat % 8) - 7.9) / 0.1));
      p.headNod = 0.1 + 0.32 * bang * amp;
      p.torsoLean = 0.06 + 0.14 * bang * amp;
      p.rH = mix3([H.type[0], H.type[1] + 0.12 * Math.sin(Math.PI * 4 * beat), H.type[2]], H.danceUp, horns);
      p.lH = [-H.type[0] - 0.08, H.type[1] + 0.1, H.type[2] + 0.08];
      p.rP = [-1, -0.6, -0.3];
      p.lP = [1, -0.6, -0.3];
      p.rHipOut = p.lHipOut = 0.12;
      p.rKnee = p.lKnee = 0.25 * (0.5 + 0.5 * Math.cos(Math.PI * 2 * beat)) * amp;
      return p;
    },
    // Rap : rebond bas, buste penché, un bras « swag », l'autre main sur la hanche, tête à contre-temps.
    rap(beat, amp) {
      const p = basePose(H);
      const bend = 0.5 + 0.5 * Math.cos(Math.PI * beat);
      const q = Math.sin((Math.PI * beat) / 2);
      p.rootY = 0.03 * bend * amp;
      p.rHipFwd = p.lHipFwd = 0.12 * bend * amp;
      p.rKnee = p.lKnee = 0.3 * bend * amp;
      p.torsoLean = 0.1;
      p.headNod = 0.1 * Math.sin(Math.PI * beat + Math.PI / 2) * amp;
      p.headTilt = 0.1 * q;
      p.rH = mix3(H.discoLow, H.discoHigh, 0.5 + 0.5 * Math.tanh(3 * q));
      p.rP = [-1, -0.3, -0.4];
      onHip(p, 'l');
      p.rootRotY = 0.12 * q;
      return p;
    },
    // Latino : déhanché, buste qui pivote en sens inverse, bras qui ondulent en décalé.
    latino(beat, amp) {
      const p = basePose(H);
      const sway = Math.sin(Math.PI * beat);
      p.rootX = 0.08 * sway * amp;
      p.torsoTwist = 0.25 * sway * amp;
      p.torsoSway = -0.12 * sway * amp;
      p.headTilt = 0.1 * sway;
      p.rHipFwd = 0.15 * Math.max(0, sway) * amp;
      p.lHipFwd = 0.15 * Math.max(0, -sway) * amp;
      p.rKnee = 0.3 * Math.max(0, sway) * amp;
      p.lKnee = 0.3 * Math.max(0, -sway) * amp;
      p.rH = [H.step[0], H.step[1] + 0.22 * Math.sin(Math.PI * beat + 0.5), H.step[2] + 0.1];
      p.lH = [-H.step[0], H.step[1] + 0.22 * Math.sin(Math.PI * beat + 0.5 + Math.PI), H.step[2] + 0.1];
      p.rP = [-1, -0.3, -0.6];
      p.lP = [1, -0.3, -0.6];
      return p;
    }
  };

  return {
    idle(p, a, t) {
      p.rootX = 0.025 * Math.sin(t * 0.5);
      p.torsoSway = 0.03 * Math.sin(t * 0.5 + 0.6);
      p.headTurn = 0.28 * wander(t * 0.35, 1);
      p.headNod = 0.05 * wander(t * 0.5, 4);
      p.headTilt = 0.07 * wander(t * 0.3, 7);
      p.eyeX = 0.02 * wander(t * 0.4, 2);
      p.rH = [H.rest[0], H.rest[1] + 0.012 * Math.sin(t * 1.1), H.rest[2]];
      p.lH = [-H.rest[0], H.rest[1] + 0.012 * Math.sin(t * 1.1 + 1.3), H.rest[2]];
    },

    sleep(p, a, t, k) {
      p.torsoLean = 0.12 + 0.025 * Math.sin(t * 1.1);
      p.headNod = 0.42;
      p.headTilt = 0.14;
      p.rHipFwd = p.lHipFwd = 0.1;
      p.rKnee = p.lKnee = 0.22;
      p.rH = [...H.sleep];
      p.lH = mirror(H.sleep);
      p.eyeOpen = 0.06;
      p.mouthSmile = 0.3;
      p.glow = 0.35;
      k.show('zzz', P.zzz);
    },

    listen(p, a, t, k) {
      const wake = k.data && k.data.wake;
      p.torsoLean = 0.1;
      p.headTilt = -0.16;
      p.headTurn = 0.16;
      p.headNod = 0.05 * Math.min(1, k.level * 3) * Math.sin(t * 8);
      p.rH = [...H.ear];
      p.rP = [-1, -0.4, -0.1];
      onHip(p, 'l');
      p.eyeScale = 1.12;
      p.eyeX = -0.015;
      p.mouthSmile = wake ? 1 : 0.6;
      p.eyeHappy = wake ? 0.35 : 0;
      k.show('waves', P.waves);
    },

    think(p, a, t, k) {
      p.rH = [...H.chin];
      p.rP = [-1, -0.7, 0];
      p.lH = mirror(H.belly);
      p.lP = [1, -0.6, 0.2];
      p.headNod = -0.12;
      p.headTilt = 0.12 + 0.04 * Math.sin(t * 0.8);
      p.headTurn = 0.15 + 0.08 * wander(t * 0.4, 3);
      p.torsoSway = 0.03 * Math.sin(t * 0.9);
      p.eyeX = 0.03;
      p.eyeY = 0.035;
      p.eyeOpen = 0.85;
      p.mouthSmile = 0.2;
      p.lHipFwd = 0.06 * Math.max(0, Math.sin(t * 5));
      k.show('dots', P.dots);
    },

    talk(p, a, t) {
      const s1 = Math.sin(t * 1.7);
      const s2 = Math.sin(t * 2.3 + 1);
      const s3 = Math.sin(t * 1.3 + 2);
      p.rH = [H.talk[0] - 0.07 * s1, H.talk[1] + 0.1 * s2, H.talk[2] + 0.06 * s3];
      p.lH = [-H.talk[0] + 0.07 * s3, H.talk[1] - 0.04 + 0.1 * Math.sin(t * 2.1 + 2.4), H.talk[2] - 0.02 + 0.06 * s1];
      p.rP = [-1, -0.3, -0.6];
      p.lP = [1, -0.3, -0.6];
      p.headNod = 0.05 * Math.sin(t * 3.1);
      p.headTilt = 0.08 * Math.sin(t * 1.1);
      p.headTurn = 0.12 * Math.sin(t * 0.7);
      p.torsoTwist = 0.06 * Math.sin(t * 0.9);
      p.mouthSmile = 0.5;
    },

    dance(p, a, t, k) {
      // Style et tempo de la musique (ui/music-style.js) : six danses, sur le tempo détecté (120 bpm à défaut).
      const style = k.data && k.data.style ? k.data.style : 'pop';
      const bpm = k.data && k.data.bpm ? clamp(k.data.bpm, 60, 180) : 120;
      const beat = (t * bpm) / 60;
      const amp = (0.75 + 0.5 * clamp(k.music || 0, 0, 1)) * (A.danceAmp || 1);
      const styled = STYLE_DANCES[style];
      if (styled) {
        Object.assign(p, styled(beat, amp));
      } else {
        // Pop : quatre pas qui s'enchaînent toutes les 4 s (fondu de 0,4 s).
        const move = Math.floor(a / 4);
        const into = a - move * 4;
        const cur = dancePose(move % 4, beat, amp);
        Object.assign(p, into < 0.4 && move > 0 ? lerpPose(dancePose((move - 1) % 4, beat, amp), cur, smooth(into / 0.4)) : cur);
      }
      p.eyeHappy = style === 'calme' ? 0.6 : 1;
      p.mouthSmile = style === 'rock' ? 0.6 : 1;
      k.show('notes', P.notes);
    },

    mail(p, a, t, k) {
      const up = pulse(a, 0, 0.45);
      p.rH = mix3(H.rest, H.mail, up);
      p.lH = mix3(mirror(H.rest), mirror(H.mail), up);
      p.rP = [-1, -0.6, -0.2];
      p.lP = [1, -0.6, -0.2];
      p.headNod = 0.12 + 0.03 * Math.sin(t * 2);
      p.headTilt = 0.08 * Math.sin(t * 1.2);
      p.eyeHappy = a > 1.2 ? 1 : 0;
      p.eyeY = -0.02;
      p.mouthSmile = 1;
      k.show('envelope', P.envelope, { rotX: -0.12 });
    },

    globe(p, a, t, k) {
      p.rH = [...H.globe];
      p.rP = [-1, -0.8, 0];
      onHip(p, 'l');
      p.headTurn = -0.38;
      p.headNod = 0.06;
      p.eyeX = -0.03;
      p.mouthSmile = 0.8;
      k.show('globe', P.globe);
    },

    code(p, a, t, k) {
      typing(p, t, 1);
      // Regard : le clavier, puis l'écran, en alternance.
      const look = smooth(0.5 + 0.5 * Math.sin(t * 1.2) * 1.6);
      p.headNod = 0.24 - 0.3 * look;
      p.headTurn = 0.36 * look;
      p.eyeX = 0.025 * look;
      k.show('keyboard', P.keyboard);
      k.show('screen', P.screen, { rotY: -0.4 });
    },

    work(p, a, t, k) {
      typing(p, t, 0.5);
      p.headNod = 0.2 + 0.05 * Math.sin(t * 0.7);
      p.headTurn = 0.1 * wander(t * 0.4, 5);
      k.show('keyboard', P.keyboard, { scale: 0.9 });
    },

    calendar(p, a, t, k) {
      present(p, 'l', t);
      k.show('calendar', P.panel, { rotY: -0.32 });
    },

    clock(p, a, t, k) {
      // Coup d'œil à la montre, puis l'heure (ou la date) s'affiche à côté.
      const look = 1 - pulse(a, 1.3, 1.7);
      present(p, 'l', t);
      if (look > 0) {
        p.lH = mix3(p.lH, mirror(H.watch), look);
        p.lP = mix3(p.lP, [1, -1, 0], look);
        p.headNod = 0.26 * look + p.headNod * (1 - look);
        p.headTurn = 0.12 * look + p.headTurn * (1 - look);
        p.eyeY = -0.03 * look;
      }
      k.show('watch', A.watch || [0, -0.02, 0], { parent: k.anchors.lWrist });
      if (a > 1.2) k.show('clock', P.panel, { rotY: -0.32 });
    },

    show(p, a, t, k) {
      present(p, 'r', t);
      p.eyeHappy = 0.3;
      k.show('billboard', mirror(P.panel), { rotY: 0.32 });
    },

    guard(p, a, t, k) {
      p.lH = mirror(H.shield);
      p.lP = [1, -0.5, 0];
      p.rH = [...H.stop];
      p.rP = [-1, -0.6, -0.2];
      p.headNod = -0.04;
      p.headTurn = 0.06 * Math.sin(t * 0.8);
      p.eyeOpen = 0.72;
      p.eyeTilt = -0.16;
      p.mouthSmile = 0;
      k.show('shield', P.shield);
    },

    shrug(p, a, t, k) {
      const shake = Math.max(0, 1 - a / 1.1);
      p.headTurn = 0.32 * Math.sin(a * 15) * shake;
      p.headTilt = 0.22 * (1 - shake);
      const up = pulse(a, 0.2, 0.6);
      p.rH = mix3(H.rest, H.shrug, up);
      p.lH = mix3(mirror(H.rest), mirror(H.shrug), up);
      p.rP = [-0.4, -1, -0.6];
      p.lP = [0.4, -1, -0.6];
      p.eyeTilt = 0.22;
      p.eyeOpen = 0.9;
      p.mouthSmile = -0.8;
      k.show('question', P.question);
    },

    happy(p, a, t, k) {
      const hop = Math.max(0, Math.sin(a * 9)) * Math.max(0, 1 - a / 1.8);
      p.rootY = 0.07 * hop;
      p.rH = [...H.thumb];
      p.rP = [-1, -0.8, -0.2];
      p.rThumb = 1;
      onHip(p, 'l');
      p.eyeHappy = 1;
      p.blush = 1;
      p.mouthSmile = 1;
      p.headTilt = -0.1;
      k.show('sparkles', P.sparkles);
    },

    tired(p, a, t, k) {
      p.torsoLean = 0.22;
      p.headNod = 0.3;
      p.headTilt = 0.1 * Math.sin(t * 0.6);
      p.rootX = 0.03 * Math.sin(t * 0.4);
      p.rH = [...H.tired];
      p.lH = mirror(H.tired);
      p.rHipFwd = p.lHipFwd = 0.08;
      p.rKnee = p.lKnee = 0.16;
      p.eyeOpen = 0.45;
      p.eyeTilt = 0.2;
      p.mouthSmile = -0.6;
      p.glow = 0.7;
      k.show('sweat', P.sweat);
    },

    wave(p, a) {
      const up = pulse(a, 0, 0.35);
      const sway = Math.sin(a * 11) * 0.11 * up;
      p.rH = mix3(H.rest, [H.wave[0] + sway, H.wave[1], H.wave[2]], up);
      p.rP = [-1, -0.5, -0.2];
      p.headTilt = -0.12 * up;
      p.rootY = 0.03 * Math.max(0, Math.sin(a * 6)) * Math.max(0, 1 - a);
      p.eyeHappy = 1;
      p.mouthSmile = 1;
    },

    bow(p, a) {
      const k = pulse(a, 0, 0.5) * (1 - pulse(a, 1.6, 2.2));
      p.torsoLean = 0.55 * k;
      p.headNod = 0.22 * k;
      p.rH = mix3(H.rest, H.bowFront, k);
      p.rP = [-1, -0.5, 0.3];
      p.lH = mix3(mirror(H.rest), mirror(H.bowBack), k);
      p.lP = [1, 0, 0.5];
      p.eyeHappy = k > 0.3 ? 1 : 0;
      p.mouthSmile = 1;
    },

    // --- Les 10 niveaux d'émotions canoniques (score 10 à 1) ---

    // 10 : L'extase (ou l'euphorie) : joie intense et débordante, ravissement absolu
    extase(p, a, t, k) {
      const hop = Math.max(0, Math.sin(t * 8));
      p.rootY = 0.08 * hop;
      p.torsoLean = -0.06;
      p.torsoSway = 0.06 * Math.sin(t * 3.5);
      p.headNod = -0.16 + 0.04 * Math.sin(t * 4);
      p.headTilt = 0.08 * Math.sin(t * 2);
      p.rH = [-0.62, 1.25 + 0.05 * hop, 0.22];
      p.lH = [0.62, 1.25 + 0.05 * hop, 0.22];
      p.rP = [-1, -0.2, -0.6];
      p.lP = [1, -0.2, -0.6];
      p.eyeHappy = 1;
      p.blush = 1;
      p.mouthSmile = 1;
      p.glow = 1.35;
      k.show('sparkles', P.sparkles, { scale: 1.3 });
    },

    // 10 : L'extase (ou l'euphorie) : joie intense et débordante, sentiment de ravissement absolu
    extase(p, a, t, k) {
      // Flottement aérien en apesanteur, très haut
      p.rootY = 0.12 + 0.05 * Math.sin(t * 3.5);
      p.torsoLean = -0.08 + 0.02 * Math.sin(t * 2.5);
      p.headNod = -0.22 + 0.03 * Math.sin(t * 3);
      // Bras déployés vers le ciel
      p.rH = [-0.62, 1.35, 0.22];
      p.lH = [0.62, 1.35, 0.22];
      p.rP = [-0.8, -0.6, 0.3];
      p.lP = [0.8, -0.6, 0.3];
      p.eyeHappy = 1;
      p.blush = 1;
      p.mouthSmile = 1;
      p.glow = 1.45;
      k.show('sparkles', P.sparkles, { scale: 1.4 });
    },

    // 9 : La joie : émotion vive, chaleureuse et communicative
    joie(p, a, t, k) {
      // Petits rebonds dynamiques joyeux
      const bop = Math.abs(Math.sin(t * 4.5));
      p.rootY = 0.05 * bop;
      const clap = Math.sin(t * 8);
      p.rH = [-0.22 - 0.1 * Math.max(0, clap), 0.65 + 0.04 * clap, 0.42];
      p.lH = [0.22 + 0.1 * Math.max(0, clap), 0.65 + 0.04 * clap, 0.42];
      p.rP = [-1, -0.6, -0.2];
      p.lP = [1, -0.6, -0.2];
      p.headNod = 0.08 * Math.sin(t * 4);
      p.headTilt = -0.12 * Math.sin(t * 2.5);
      p.eyeHappy = 1;
      p.blush = 0.8;
      p.mouthSmile = 1;
      p.glow = 1.25;
      k.show('sparkles', P.sparkles, { scale: 0.9 });
    },

    // 8 : Le contentement : état d'équilibre et de satisfaction paisible, vrai bien-être intérieur
    contentement(p, a, t) {
      p.rootY = 0;
      p.rH = mix3(H.rest, H.hip, 0.85);
      p.rP = [-1, 0, -0.3];
      p.lH = [0.18, 0.4, 0.35];
      p.lP = [1, -0.5, 0];
      p.headNod = 0.07 * Math.sin(t * 2);
      p.headTilt = 0.06 * Math.cos(t * 1.5);
      p.torsoSway = 0.025 * Math.sin(t * 1.2);
      p.eyeHappy = 0.65;
      p.mouthSmile = 0.85;
      p.blush = 0.4;
      p.glow = 1.08;
    },

    // 7 : La sérénité : calme profond de l'esprit, absence de trouble, neutralité positive
    serenite(p, a, t) {
      p.rootY = 0;
      const breath = Math.sin(t * 1.0);
      p.torsoLean = 0.01 + 0.015 * breath;
      p.rH = [H.rest[0], H.rest[1] + 0.01 * breath, H.rest[2]];
      p.lH = [-H.rest[0], H.rest[1] + 0.01 * breath, H.rest[2]];
      p.headNod = 0;
      p.headTurn = 0.05 * Math.sin(t * 0.4);
      p.headTilt = 0;
      p.eyeOpen = 0.95;
      p.mouthSmile = 0.5;
      p.glow = 1.0;
    },

    // 6 : La mélancolie : tristesse douce et poétique, souvent teintée de nostalgie, vague à l'âme
    melancolie(p, a, t) {
      p.rootY = 0;
      p.rKnee = p.lKnee = 0.08;
      p.torsoLean = 0.16 + 0.02 * Math.sin(t * 0.8);
      p.headTilt = 0.24 + 0.04 * Math.sin(t * 0.8);
      p.headTurn = 0.18;
      p.headNod = 0.10;
      p.eyeX = 0.04;
      p.eyeY = -0.02;
      p.eyeOpen = 0.82;
      p.rH = mix3(H.rest, H.chin, 0.75);
      p.rP = [-1, -0.5, 0];
      p.lH = mirror(H.rest);
      p.torsoSway = 0.03 * Math.sin(t * 0.6);
      p.mouthSmile = 0.1;
      p.glow = 0.88;
    },

    // 5 : Le chagrin : douleur morale liée à une perte, déception affective bien identifiable
    chagrin(p, a, t) {
      p.rootY = 0;
      p.rHipFwd = p.lHipFwd = 0.14;
      p.rKnee = p.lKnee = 0.26;
      p.torsoLean = 0.25;
      p.headNod = 0.22 + 0.03 * Math.sin(t * 1.5);
      p.headTilt = 0.08 * Math.sin(t * 1.2);
      // Mains tremblantes qui montent essuyer les larmes près du visage
      const tremor = 0.015 * Math.sin(t * 10);
      p.rH = [-0.18 + tremor, 0.76 + tremor, 0.38];
      p.rP = [-1, -0.7, 0.1];
      p.lH = [0.18 - tremor, 0.76 + tremor, 0.38];
      p.lP = [1, -0.7, 0.1];
      p.eyeOpen = 0.65;
      p.eyeTilt = 0.20;
      p.eyeY = -0.04;
      p.mouthSmile = -0.65;
      p.glow = 0.75;
    },

    // 4 : L'abattement : s'accroupit lourdement sous le poids de la fatigue, dos courbé mais tête bien visible
    abattement(p, a, t, k) {
      p.rootY = 0.02; // reste parfaitement calé sur le sol
      p.rHipFwd = p.lHipFwd = 0.55;
      p.rKnee = p.lKnee = 1.15;
      p.rHipOut = p.lHipOut = 0.15;
      p.torsoLean = 0.35 + 0.02 * Math.sin(t * 1.2);
      // Tête baissée avec découragement mais bien haute et visible au centre du hublot
      p.headNod = 0.25 + 0.03 * Math.sin(t * 0.8);
      p.headTilt = 0.06;
      // Bras posés sur les cuisses / genoux
      p.rH = [-0.28, 0.22, 0.35];
      p.lH = [0.28, 0.22, 0.35];
      p.rP = [-0.6, -1, -0.3];
      p.lP = [0.6, -1, -0.3];
      p.eyeOpen = 0.45;
      p.eyeTilt = 0.22;
      p.mouthSmile = -0.75;
      p.glow = 0.65;
      k.show('sweat', [P.sweat[0], P.sweat[1] - 0.22, P.sweat[2]]);
    },

    // 3 : La détresse : tombe à genoux, les mains plaquées sur la tête en panique, regard terrifié
    detresse(p, a, t, k) {
      p.rootY = 0.02; // à genoux sur le sol, ne descend pas sous terre
      p.rHipFwd = 0.95;
      p.rKnee = 1.65;
      p.lHipFwd = 0.80;
      p.lKnee = 1.50;
      p.torsoLean = 0.18;
      // Tremblements aigus de panique dans tout le corps
      const shake = 0.02 * Math.sin(t * 22);
      p.torsoSway = 0.03 * Math.sin(t * 18);
      // Mains plaquées sur la tête / tempes en panique
      p.rH = [-0.38 + shake, 0.92, 0.32];
      p.lH = [0.38 - shake, 0.92, 0.32];
      p.rP = [-1, -0.4, 0];
      p.lP = [1, -0.4, 0];
      // Tête bien visible au centre, tournée vers l'utilisateur avec effroi
      p.headTurn = 0.16 * Math.sin(t * 3.5);
      p.headNod = 0.05 + 0.05 * Math.sin(t * 5);
      p.headTilt = 0.08 * Math.cos(t * 3);
      p.eyeOpen = 1.35;
      p.eyeTilt = -0.25;
      p.eyeX = 0.04 * Math.sin(t * 4);
      p.mouthSmile = -0.85;
      p.mouthOpen = 0.42;
      p.glow = 0.55;
      k.show('question', [P.question[0], P.question[1] - 0.25, P.question[2]]);
    },

    // 2 : L'accablement : totalement à genoux, prostré, mains posées en avant au sol, tête brisée
    accablement(p, a, t, k) {
      p.rootY = 0.03; // calé sur le sol
      p.rHipFwd = p.lHipFwd = 1.15;
      p.rKnee = p.lKnee = 1.85;
      p.rHipOut = p.lHipOut = 0.25;
      p.torsoLean = 0.45 + 0.02 * Math.sin(t * 0.9);
      // Tête baissée mais visage parfaitement visible et orienté vers l'avant
      p.headNod = 0.32;
      p.headTilt = 0.12 * Math.sin(t * 0.8);
      p.headTurn = 0.10 * Math.sin(t * 1.2);
      // Mains posées sur le sol devant les genoux, bras affaissés
      p.rH = [-0.25, 0.06, 0.38];
      p.lH = [0.25, 0.06, 0.38];
      p.rP = [-0.2, -1, 0];
      p.lP = [0.2, -1, 0];
      p.eyeOpen = 0.25;
      p.eyeTilt = 0.32;
      p.mouthSmile = -0.95;
      p.glow = 0.42;
      k.show('sweat', [P.sweat[0], P.sweat[1] - 0.35, P.sweat[2]]);
    },

    // 1 : Le désespoir : allongé / effondré sur le sol, tête sur les bras tournée vers nous avec désespoir total
    desespoir(p, a, t) {
      p.rootY = 0.04; // repose sur le socle à terre, 100% dans le hublot
      p.rHipFwd = p.lHipFwd = 1.28;
      p.rKnee = p.lKnee = 2.0;
      p.rHipOut = p.lHipOut = 0.35;
      // Torse étalé en avant au ras du sol
      p.torsoLean = 0.62 + 0.012 * Math.sin(t * 2.2);
      // Bras allongés au sol de chaque côté de la tête, sans force
      p.rH = [-0.30, -0.02, 0.42];
      p.lH = [0.30, -0.02, 0.42];
      p.rP = [0, -1, 0];
      p.lP = [0, -1, 0];
      // TÊTE PARFAITEMENT VISIBLE : posée sur ses bras au sol, relevée et tournée vers l'utilisateur !
      p.headNod = 0.18 + 0.015 * Math.sin(t * 1.5);
      p.headTilt = 0.24; // tête inclinée de côté sur son bras
      p.headTurn = 0.14; // visage tourné vers la caméra
      p.eyeOpen = 0.14;
      p.eyeTilt = 0.42;
      p.eyeY = -0.03;
      p.mouthSmile = -1.0;
      p.glow = 0.32;
    }
  };
}

// --- Personnage ----------------------------------------------------------------------------------------
// spec : { props, dims, anchors, build(joints, kit) → { animate(p, f), thumbs?: { r }, dispose? }, tune? }
// dims : { hipY, hipX, shoulder: [x, y, z] (gauche), upper, lower, thigh, shin, neckY, headY }

export function createRig(THREE, spec) {
  const { props, dims: D, anchors, build } = spec;
  const ACTIONS = makeActions(anchors);
  const disposables = [];
  const keep = (x) => {
    disposables.push(x);
    return x;
  };
  const mesh = (geo, mat, parent, x = 0, y = 0, z = 0) => {
    const m = new THREE.Mesh(keep(geo), mat);
    m.position.set(x, y, z);
    if (parent) parent.add(m);
    return m;
  };

  // Articulations.
  const root = new THREE.Group();
  const hips = new THREE.Group();
  hips.position.y = D.hipY;
  root.add(hips);
  const torso = new THREE.Group();
  hips.add(torso);
  const neck = new THREE.Group();
  neck.position.y = D.neckY;
  torso.add(neck);
  const head = new THREE.Group();
  head.position.y = D.headY;
  neck.add(head);
  const arm = (s) => {
    const shoulder = new THREE.Group();
    shoulder.position.set(D.shoulder[0] * s, D.shoulder[1], D.shoulder[2]);
    torso.add(shoulder);
    const elbow = new THREE.Group();
    elbow.position.y = -D.upper;
    shoulder.add(elbow);
    const wrist = new THREE.Group();
    wrist.position.y = -D.lower;
    elbow.add(wrist);
    return { shoulder, elbow, wrist, s };
  };
  const leg = (s) => {
    const hip = new THREE.Group();
    hip.position.set(D.hipX * s, D.hipDy || 0, 0);
    hips.add(hip);
    const knee = new THREE.Group();
    knee.position.y = -D.thigh;
    hip.add(knee);
    const foot = new THREE.Group();
    foot.position.y = -D.shin;
    knee.add(foot);
    return { hip, knee, foot, s };
  };
  const arms = { r: arm(-1), l: arm(1) };
  const legs = { r: leg(-1), l: leg(1) };
  const joints = { root, hips, torso, neck, head, arms, legs };

  const model = build(joints, { keep, mesh });
  root.add(props.root);
  const thumbs = [];
  for (const side of ['r', 'l']) {
    const obj = model.thumbs && model.thumbs[side];
    if (obj) thumbs.push({ side, obj, rest: obj.quaternion.clone(), wrist: arms[side].wrist });
  }
  const anchorObjs = { lWrist: arms.l.wrist, rWrist: arms.r.wrist };

  // --- Cinématique inverse d'un bras (repère du buste) ---------------------------------------------
  const tmp = {
    S: new THREE.Vector3(), T: new THREE.Vector3(), n: new THREE.Vector3(), m: new THREE.Vector3(),
    E: new THREE.Vector3(), u: new THREE.Vector3(), f: new THREE.Vector3(), X: new THREE.Vector3(),
    Y: new THREE.Vector3(), Z: new THREE.Vector3(), M: new THREE.Matrix4(), P: new THREE.Vector3()
  };
  function solveArm(a, target, pole) {
    const { S, T, n, m, E, u, f, X, Y, Z, M, P } = tmp;
    const UP = D.upper;
    const LO = D.lower;
    S.copy(a.shoulder.position);
    T.set(target[0], target[1], target[2]);
    n.subVectors(T, S);
    let d = n.length();
    if (d < 1e-4) return;
    n.divideScalar(d);
    d = clamp(d, Math.abs(UP - LO) + 0.02, UP + LO - 0.002);
    const cosA = clamp((UP * UP + d * d - LO * LO) / (2 * UP * d), -1, 1);
    const sinA = Math.sqrt(1 - cosA * cosA);
    P.set(pole[0], pole[1], pole[2]);
    m.copy(P).addScaledVector(n, -P.dot(n));
    if (m.lengthSq() < 1e-6) m.set(0, 0, -1).addScaledVector(n, -n.z);
    m.normalize();
    E.copy(S).addScaledVector(n, UP * cosA).addScaledVector(m, UP * sinA);
    u.subVectors(E, S).normalize();
    f.copy(S).addScaledVector(n, d).sub(E).normalize();
    Y.copy(u).negate();
    Z.copy(f).addScaledVector(u, -f.dot(u));
    if (Z.lengthSq() < 1e-6) Z.copy(m).negate();
    Z.normalize();
    X.crossVectors(Y, Z).normalize();
    Z.crossVectors(X, Y).normalize();
    M.makeBasis(X, Y, Z);
    a.shoulder.quaternion.setFromRotationMatrix(M);
    a.elbow.rotation.set(-Math.acos(clamp(u.dot(f), -1, 1)), 0, 0);
  }

  function footDrop(hipFwd, knee, out) {
    const h = D.thigh * Math.cos(hipFwd) + D.shin * Math.cos(hipFwd - knee);
    return D.thigh + D.shin - h * Math.cos(out);
  }

  // --- Lecture de l'animation ----------------------------------------------------------------------
  let current = basePose(anchors.hand);
  let from = null;
  let fade = 1;
  let key = '';
  let actionT0 = 0;
  let nextBlink = 2;
  let blinkAt = -1;
  let lastHeadTurn = 0;
  let lastRootY = 0;
  const accent = new THREE.Color(0x38e1ff);
  const qWorld = new THREE.Quaternion();
  const qRoot = new THREE.Quaternion();
  // Regard vers une card (ui/character-gaze.js) : position lissée de la direction et du poids.
  let gazeX = 0;
  let gazeY = 0;
  let gazeW = 0;
  let backW = 0; // 0 : de face, 1 : de dos (moteur vocal pas encore activé)

  // ctx : { action, data, startedAt, level, speaking, music, accent (THREE.Color), power }
  function update(dt, t, ctx) {
    const name = ACTIONS[ctx.action] ? ctx.action : 'idle';
    // Un changement de style de danse (musique) repart d'un fondu, comme un changement d'animation.
    const k2 = `${name}|${ctx.startedAt || ''}|${name === 'dance' && ctx.data && ctx.data.style ? ctx.data.style : ''}`;
    if (k2 !== key) {
      // Nouvelle animation : fondu depuis la pose affichée.
      from = current;
      fade = 0;
      key = k2;
      actionT0 = t;
    }
    const a = t - actionT0;
    props.begin();
    const k = {
      level: ctx.level || 0, music: ctx.music || 0, data: ctx.data || null, anchors: anchorObjs,
      show: (n, at, o) => props.show(n, at[0], at[1], at[2], o)
    };
    const p = basePose(anchors.hand);
    ACTIONS[name](p, a, t, k);
    if (spec.tune) spec.tune(p, name, a, t, k);
    // Couche du compagnon de bureau (ui/characters/companion-gear.js) : effets par-dessus le geste.
    if (ctx.extra) ctx.extra(k, name, t);
    fade = Math.min(1, fade + dt / 0.45);
    const out = from && fade < 1 ? lerpPose(from, p, smooth(fade)) : p;
    current = out;

    // Couches ajoutées : clignement, parole.
    if (t > nextBlink && blinkAt < 0) blinkAt = t;
    let blink = 1;
    if (blinkAt >= 0) {
      const b = (t - blinkAt) / 0.16;
      if (b >= 1) {
        blinkAt = -1;
        nextBlink = t + 2.4 + Math.random() * 3.2;
      } else blink = Math.abs(1 - 2 * b);
    }
    let mouthOpen = out.mouthOpen;
    if (ctx.speaking) {
      const syl = Math.abs(Math.sin(t * 12.7) * Math.sin(t * 7.3 + 0.6));
      mouthOpen = Math.max(mouthOpen, clamp(0.2 + 0.85 * syl + 0.6 * (ctx.level || 0), 0, 1));
    }
    if (ctx.accent) accent.copy(ctx.accent);
    // Regard : la tête, les yeux et un peu du buste se tournent vers la card (sans toucher à la pose
    // mémorisée pour le fondu suivant). Le poids dépend du geste en cours.
    const gz = ctx.gaze;
    const want = gz ? gz.w * (GAZE_WEIGHT[name] === undefined ? GAZE_WEIGHT.default : GAZE_WEIGHT[name]) : 0;
    const follow = 1 - Math.exp(-dt * 4.5);
    if (gz) {
      gazeX += (gz.x - gazeX) * follow;
      gazeY += (gz.y - gazeY) * follow;
    }
    gazeW += (want - gazeW) * follow;
    backW += ((ctx.back ? 1 : 0) - backW) * (1 - Math.exp(-dt * 2.5));
    let shown = out;
    if (gazeW > 0.004) {
      shown = { ...out };
      shown.headTurn += gazeX * 0.6 * gazeW;
      shown.headNod += gazeY * 0.26 * gazeW;
      shown.torsoTwist += gazeX * 0.16 * gazeW;
      shown.eyeX += gazeX * 0.045 * gazeW;
      shown.eyeY -= gazeY * 0.03 * gazeW; // eyeY positif = regard vers le haut
    }
    apply(shown, dt, t, { action: name, a, blink, mouthOpen, level: ctx.level || 0, speaking: !!ctx.speaking, back: backW, power: ctx.power === undefined ? 1 : ctx.power });
    props.update(dt, t, { accent, level: ctx.level || 0, music: ctx.music || 0, data: ctx.data || null });
  }

  function apply(p, dt, t, f) {
    // Pieds au sol : la jambe la plus tendue porte le corps.
    const drop = Math.min(footDrop(p.rHipFwd, p.rKnee, p.rHipOut), footDrop(p.lHipFwd, p.lKnee, p.lHipOut));
    root.position.set(p.rootX, p.rootY - drop, 0);
    root.rotation.set(0, p.rootRotY + Math.PI * (f.back || 0), p.rootRotZ);
    torso.rotation.set(p.torsoLean, p.torsoTwist, p.torsoSway);
    head.rotation.set(p.headNod, p.headTurn, p.headTilt);

    solveArm(arms.r, p.rH, p.rP);
    solveArm(arms.l, p.lH, p.lP);
    // Pouce levé : orienté vers le haut du personnage, quelle que soit la position de la main.
    for (const th of thumbs) {
      const up = th.side === 'r' ? p.rThumb : 0;
      if (up > 0.01) {
        root.updateMatrixWorld(true);
        th.wrist.getWorldQuaternion(qWorld);
        root.getWorldQuaternion(qRoot);
        th.obj.quaternion.copy(th.rest).slerp(qWorld.invert().multiply(qRoot), up);
        th.obj.scale.setScalar(1 + 0.25 * up);
      } else {
        th.obj.quaternion.copy(th.rest);
        th.obj.scale.setScalar(1);
      }
    }

    for (const [lg, fwd, knee, outA] of [[legs.r, p.rHipFwd, p.rKnee, -p.rHipOut], [legs.l, p.lHipFwd, p.lKnee, p.lHipOut]]) {
      lg.hip.rotation.set(-fwd, 0, outA);
      lg.knee.rotation.set(knee, 0, 0);
      lg.foot.rotation.set(fwd - knee, 0, -outA);
    }

    // Vitesses de la tête et des sauts (antennes, oreilles… qui ballottent).
    const turnV = (p.headTurn - lastHeadTurn) / Math.max(dt, 1e-3);
    const jumpV = (root.position.y - lastRootY) / Math.max(dt, 1e-3);
    lastHeadTurn = p.headTurn;
    lastRootY = root.position.y;
    model.animate(p, { ...f, dt, t, accent, turnV, jumpV });
  }

  function dispose() {
    if (model.dispose) model.dispose();
    for (const d of disposables) if (d && d.dispose) d.dispose();
  }

  return { root, update, dispose, actions: Object.keys(ACTIONS), joints };
}

// --- Outils de modélisation ------------------------------------------------------------------------

// Rayon d'un profil de révolution [[rayon, hauteur], …] (hauteurs croissantes) à la hauteur y.
export function profileR(profile, y) {
  for (let i = 1; i < profile.length; i++) {
    const [r0, y0] = profile[i - 1];
    const [r1, y1] = profile[i];
    if (y <= y1) return r0 + (r1 - r0) * clamp((y - y0) / (y1 - y0), 0, 1);
  }
  return 0;
}

// Pièce plaquée sur une surface courbe (bouche, joue, ventre…) : contour 2D [[x, y], …] autour d'un
// centre, maillée en anneaux concentriques pour épouser la courbure ; project(x, y) → z de la
// surface. set() peut être rappelé à chaque image (bouche qui s'ouvre) : seules les positions changent.
export function makePatch(THREE, rings = 4) {
  const geo = new THREE.BufferGeometry();
  let n = 0;
  let cw = null;
  function set(outline, center, project, lift = 0.004) {
    // Sens du contour : les triangles doivent toujours faire face à +z (vers la caméra).
    let area = 0;
    for (let i = 0; i < outline.length; i++) {
      const [x0, y0] = outline[i];
      const [x1, y1] = outline[(i + 1) % outline.length];
      area += x0 * y1 - x1 * y0;
    }
    if (outline.length !== n || (area < 0) !== cw) {
      n = outline.length;
      cw = area < 0;
      geo.setAttribute('position', new THREE.BufferAttribute(new Float32Array((1 + rings * n) * 3), 3));
      const idx = [];
      const v = (k, i) => 1 + (k - 1) * n + (i % n);
      const tri = (a, b, c) => (cw ? idx.push(a, c, b) : idx.push(a, b, c));
      for (let i = 0; i < n; i++) tri(0, v(1, i), v(1, i + 1));
      for (let k = 2; k <= rings; k++) {
        for (let i = 0; i < n; i++) {
          tri(v(k - 1, i), v(k, i), v(k, i + 1));
          tri(v(k - 1, i), v(k, i + 1), v(k - 1, i + 1));
        }
      }
      geo.setIndex(idx);
    }
    const pos = geo.attributes.position;
    pos.setXYZ(0, center[0], center[1], project(center[0], center[1]) + lift);
    for (let k = 1; k <= rings; k++) {
      const w = k / rings;
      for (let i = 0; i < n; i++) {
        const x = center[0] + (outline[i][0] - center[0]) * w;
        const y = center[1] + (outline[i][1] - center[1]) * w;
        pos.setXYZ(1 + (k - 1) * n + i, x, y, project(x, y) + lift);
      }
    }
    pos.needsUpdate = true;
    geo.computeVertexNormals();
    geo.computeBoundingSphere();
    return geo;
  }
  return { geo, set };
}

// Surface avant d'un ellipsoïde de demi-axes (a, b, c) centré à l'origine : z(x, y).
export const ellipsoidZ = (a, b, c) => (x, y) => c * Math.sqrt(Math.max(0, 1 - (x / a) ** 2 - (y / b) ** 2));

// Pose un objet sur l'avant d'un ellipsoïde (centre cx, cy, cz ; demi-axes a, b, c), tourné selon la
// normale de la surface (son axe z sort de la surface).
export function placeOn(THREE, obj, x, y, [cx, cy, cz], [a, b, c], lift = 0.004) {
  const z = c * Math.sqrt(Math.max(0.02, 1 - (x / a) ** 2 - (y / b) ** 2));
  const n = new THREE.Vector3(x / (a * a), y / (b * b), z / (c * c)).normalize();
  obj.position.set(cx + x + n.x * lift, cy + y + n.y * lift, cz + z + n.z * lift);
  obj.quaternion.setFromUnitVectors(new THREE.Vector3(0, 0, 1), n);
  return obj;
}
