// Petit Robot : robot rouge rayé d'orange du cœur de Jarvis (ui/character.js), d'après le dessin de
// référence : antenne en T, masque jaune, gros yeux blancs à pupilles noires, grande bouche ouverte et
// langue rose, oreilles orange, buste et « short » en trapèze, bras fins à anneaux orange, grosses mains
// rouges, chaussures orange. Enthousiaste : il sautille, sourit tout le temps, danse avec ampleur.
//
// Le point de jonction de l'antenne prend la couleur de l'état de Jarvis (le hublot et le liseré du
// cœur la montrent aussi). Squelette, animations et gestes : ui/characters/rig.js.
// Repère : pieds à l'origine, face vers +z, ~2,45 de haut antenne comprise.
import { createRig, clamp, makePatch, ellipsoidZ } from './rig.js';

export const meta = { id: 'petitrobot', name: 'Petit Robot', height: 2.45 };
// Card Checkup (ui/checkup.js) : deux petits sauts au départ, hologramme rouge-orangé.
export const scan = { spin: 5.5, dir: 1, hue: 0xff7a5c, quirk: 'hop' };

const HEAD = [0.35, 0.385, 0.35];   // demi-axes de la tête
const HEAD_C = [0, 0.37, 0.02];     // centre de la tête par rapport au pivot du cou
const EYE_R = 0.085;

const DIMS = { hipY: 0.6, hipX: 0.15, hipDy: 0.02, shoulder: [0.27, 0.71, 0], upper: 0.33, lower: 0.28, thigh: 0.23, shin: 0.25, neckY: 0.79, headY: 0.08 };

const ANCHORS = {
  hand: {
    rest: [-0.4, 0.12, 0.06], hip: [-0.36, 0.3, 0.0], sleep: [-0.36, 0.14, 0.18], ear: [-0.5, 1.1, 0.08],
    chin: [-0.08, 0.86, 0.32], belly: [-0.04, 0.45, 0.32], talk: [-0.4, 0.4, 0.3],
    danceDown: [-0.4, 0.55, 0.3], danceUp: [-0.5, 1.2, 0.12], discoLow: [-0.1, 0.3, 0.38], discoHigh: [-0.66, 1.15, 0.2],
    robot: [-0.55, 0.65, 0.18], step: [-0.42, 0.38, 0.22], mail: [-0.26, 0.62, 0.5], globe: [-0.5, 0.55, 0.4],
    type: [-0.16, 0.45, 0.48], present: [-0.56, 0.75, 0.32], watch: [-0.1, 0.72, 0.45], shield: [-0.16, 0.55, 0.45],
    stop: [-0.48, 0.92, 0.36], shrug: [-0.58, 0.5, 0.3], thumb: [-0.42, 0.82, 0.38], tired: [-0.34, 0.08, 0.2],
    wave: [-0.56, 1.1, 0.16], bowFront: [-0.06, 0.45, 0.4], bowBack: [-0.32, 0.3, -0.28]
  },
  prop: {
    zzz: [0.45, 2.2, 0.3], waves: [-0.7, 1.86, 0.15], dots: [0.45, 2.42, 0.15], notes: [0, 1.9, 0.1],
    envelope: [0, 1.18, 0.56], globe: [-0.62, 1.4, 0.42], keyboard: [0, 1.08, 0.56], screen: [0.85, 1.6, 0.3],
    panel: [0.85, 1.55, 0.3], shield: [0.12, 1.12, 0.6], question: [0.45, 2.42, 0.15], sparkles: [0, 1.75, 0.15],
    sweat: [0.38, 2.0, 0.28]
  },
  danceAmp: 1.15
};

// Enthousiaste : petits rebonds au repos, tête qui dodeline, sourire permanent (sauf tristesse).
function tune(p, name, a, t) {
  if (name === 'idle') {
    p.rootY += 0.012 * Math.abs(Math.sin(t * 2.6));
    p.headTilt += 0.06 * Math.sin(t * 1.3);
    p.eyeX *= 2;
  }
  if (!['shrug', 'tired', 'guard', 'sleep'].includes(name)) p.mouthSmile = Math.max(p.mouthSmile, 0.8);
}

export function create(THREE, { props }) {
  return createRig(THREE, { props, dims: DIMS, anchors: ANCHORS, tune, build: (j, kit) => build(THREE, j, kit) });
}

function build(THREE, j, { keep, mesh }) {
  const plastic = (color, extra = {}) => keep(new THREE.MeshPhysicalMaterial({ color, roughness: 0.34, metalness: 0.02, clearcoat: 0.8, clearcoatRoughness: 0.2, envMapIntensity: 1.0, ...extra }));
  const red = plastic(0xc81e2a);
  const redFlat = plastic(0xc81e2a, { flatShading: true });
  const orange = plastic(0xf27a1c);
  const orangeFlat = plastic(0xf27a1c, { flatShading: true });
  const yellow = plastic(0xf3c117);
  const white = plastic(0xffffff, { roughness: 0.18, clearcoat: 1 });
  const pupil = plastic(0x050506, { roughness: 0.1, clearcoat: 1 });
  const mouthIn = keep(new THREE.MeshStandardMaterial({ color: 0x3b0a0e, roughness: 0.8, side: THREE.DoubleSide }));
  const tongue = keep(new THREE.MeshStandardMaterial({ color: 0xff6f86, roughness: 0.55, side: THREE.DoubleSide }));
  const glow = keep(new THREE.MeshBasicMaterial({ color: 0x38e1ff, toneMapped: false }));
  const { hips, torso, neck, head, arms, legs } = j;

  // Trapèze (pyramide tronquée à base carrée), aplati d'avant en arrière, avec bande orange devant et derrière.
  const frustum = (rTop, rBottom, h, y, mat) => {
    // Le groupe aplatit (axe z du buste) ; le maillage, tourné d'un huitième de tour, a ses faces de face.
    const flat = new THREE.Group();
    flat.scale.z = 0.72;
    torso.add(flat);
    const m = mesh(new THREE.CylinderGeometry(rTop, rBottom, h, 4, 1), mat, flat, 0, y, 0);
    m.rotation.y = Math.PI / 4;
    const zt = rTop * Math.SQRT1_2 * 0.72;
    const zb = rBottom * Math.SQRT1_2 * 0.72;
    const tilt = Math.atan2(zt - zb, h);
    for (const s of [1, -1]) {
      const stripe = mesh(new THREE.BoxGeometry(0.1, h * 1.01, 0.012), orangeFlat, torso, 0, y, s * ((zt + zb) / 2 + 0.004));
      stripe.rotation.x = s * tilt;
    }
    return m;
  };

  // --- Buste, taille et « short » ------------------------------------------------------------------
  frustum(0.4, 0.3, 0.34, 0.17, redFlat);
  mesh(new THREE.CylinderGeometry(0.24, 0.24, 0.03, 28), red, torso, 0, 0.35, 0);
  mesh(new THREE.CylinderGeometry(0.2, 0.2, 0.035, 28), orange, torso, 0, 0.375, 0);
  frustum(0.33, 0.37, 0.4, 0.59, redFlat);
  for (const s of [-1, 1]) mesh(new THREE.SphereGeometry(0.06, 18, 12), orange, torso, DIMS.shoulder[0] * s, DIMS.shoulder[1], 0);
  for (const s of [-1, 1]) mesh(new THREE.SphereGeometry(0.055, 16, 12), orange, hips, 0.17 * s, 0.0, 0);

  // --- Cou, tête, masque, yeux, bouche, oreilles, antenne -----------------------------------------
  mesh(new THREE.CylinderGeometry(0.045, 0.05, 0.1, 16), red, neck, 0, 0.04, 0);
  const skull = new THREE.Group();
  skull.position.set(...HEAD_C);
  head.add(skull);
  mesh(new THREE.SphereGeometry(1, 64, 48), red, skull).scale.set(...HEAD);
  // Masque jaune autour des yeux, bande orange du front jusqu'au sommet.
  mesh(new THREE.SphereGeometry(1.012, 48, 24, Math.PI / 2 - 1.15, 2.3, 0.82, 0.9), yellow, skull).scale.set(...HEAD);
  mesh(new THREE.SphereGeometry(1.006, 8, 24, Math.PI / 2 - 0.16, 0.32, 0, 0.83), orange, skull).scale.set(...HEAD);
  // Yeux : globe blanc, pupille noire qui regarde, reflet, paupière jaune qui se ferme.
  const eyes = [-1, 1].map((s) => {
    const g = new THREE.Group();
    const [a, b, c] = HEAD;
    const x = 0.11 * s;
    const y = 0.07;
    g.position.set(x, y, c * Math.sqrt(1 - (x / a) ** 2 - (y / b) ** 2) - 0.03);
    skull.add(g);
    mesh(new THREE.SphereGeometry(EYE_R, 32, 24), white, g);
    const pup = new THREE.Group();
    g.add(pup);
    mesh(new THREE.SphereGeometry(0.042, 24, 16), pupil, pup).scale.z = 0.45;
    mesh(new THREE.SphereGeometry(0.011, 10, 8), white, pup, -0.013, 0.015, 0.02);
    const lid = new THREE.Group();
    lid.rotation.order = 'ZXY';
    g.add(lid);
    mesh(new THREE.SphereGeometry(EYE_R + 0.004, 32, 12, 0, Math.PI * 2, 0, Math.PI / 2), yellow, lid);
    return { s, g, pup, lid };
  });
  // Bouche en D (contour du haut droit, bas arrondi) et langue, plaquées sur la tête.
  const project = ellipsoidZ(...HEAD);
  const mouthPatch = makePatch(THREE, 5);
  const tonguePatch = makePatch(THREE, 4);
  mesh(mouthPatch.geo, mouthIn, skull);
  mesh(tonguePatch.geo, tongue, skull);
  keep(mouthPatch.geo);
  keep(tonguePatch.geo);
  const shapeMouth = (w, h, top) => {
    const outline = [];
    for (let i = 0; i <= 8; i++) outline.push([-w / 2 + (w * i) / 8, top]);
    for (let i = 1; i < 16; i++) {
      const phi = (i / 16) * Math.PI;
      outline.push([(w / 2) * Math.cos(phi), top - h * Math.sin(phi)]);
    }
    mouthPatch.set(outline, [0, top - h * 0.4], project, 0.004);
    const tongueOut = [];
    const cy = top - h * 0.72;
    for (let i = 0; i < 20; i++) {
      const phi = (i / 20) * Math.PI * 2;
      tongueOut.push([0.3 * w * Math.cos(phi), cy + Math.min(0.26 * h * Math.sin(phi), h * 0.24)]);
    }
    tonguePatch.set(tongueOut, [0, cy], project, 0.007);
  };
  for (const s of [-1, 1]) mesh(new THREE.CylinderGeometry(0.075, 0.075, 0.05, 28), orange, skull, (HEAD[0] - 0.005) * s, 0, 0).rotation.z = Math.PI / 2;
  const antenna = new THREE.Group();
  antenna.position.set(0, HEAD[1] - 0.015, 0);
  skull.add(antenna);
  mesh(new THREE.CylinderGeometry(0.018, 0.024, 0.14, 12), red, antenna, 0, 0.07, 0);
  const bar = mesh(new THREE.CapsuleGeometry(0.03, 0.2, 6, 14), orange, antenna, 0, 0.16, 0);
  bar.rotation.z = Math.PI / 2;
  const knob = mesh(new THREE.SphereGeometry(0.038, 16, 12), glow, antenna, 0, 0.16, 0.012);

  // --- Bras fins à anneaux orange, grosses mains rouges -------------------------------------------
  const thumbs = {};
  const ring = (parent, y) => mesh(new THREE.TorusGeometry(0.045, 0.02, 8, 20), orange, parent, 0, y, 0).rotation.x = Math.PI / 2;
  for (const [side, a] of Object.entries(arms)) {
    const s = a.s;
    mesh(new THREE.CylinderGeometry(0.035, 0.035, DIMS.upper, 12), red, a.shoulder, 0, -DIMS.upper / 2, 0);
    ring(a.elbow, 0);
    mesh(new THREE.SphereGeometry(0.038, 12, 10), red, a.elbow);
    mesh(new THREE.CylinderGeometry(0.035, 0.035, DIMS.lower, 12), red, a.elbow, 0, -DIMS.lower / 2, 0);
    ring(a.wrist, 0.01);
    mesh(new THREE.SphereGeometry(0.07, 20, 14), red, a.wrist, 0, -0.08, 0).scale.set(1, 1.1, 0.6);
    for (const fx of [-0.04, 0, 0.04]) {
      mesh(new THREE.CapsuleGeometry(0.026, 0.06, 4, 10), red, a.wrist, fx, -0.165, 0.008).rotation.x = 0.25;
    }
    const thumb = new THREE.Group();
    thumb.position.set(0.06 * -s, -0.06, 0.02);
    thumb.rotation.z = 0.7 * s;
    a.wrist.add(thumb);
    mesh(new THREE.CapsuleGeometry(0.026, 0.05, 4, 10), red, thumb, 0, 0.035, 0);
    thumbs[side] = thumb;
  }

  // --- Jambes fines, genoux orange, chaussures orange ---------------------------------------------
  for (const lg of Object.values(legs)) {
    mesh(new THREE.CylinderGeometry(0.04, 0.04, DIMS.thigh, 12), red, lg.hip, 0, -DIMS.thigh / 2, 0);
    ring(lg.knee, 0);
    mesh(new THREE.SphereGeometry(0.042, 12, 10), red, lg.knee);
    mesh(new THREE.CylinderGeometry(0.04, 0.04, DIMS.shin, 12), red, lg.knee, 0, -DIMS.shin / 2, 0);
    mesh(new THREE.SphereGeometry(1, 32, 16), orange, lg.foot, 0, -0.055, 0.05).scale.set(0.13, 0.085, 0.2);
    mesh(new THREE.SphereGeometry(1, 32, 10), plastic(0xd2601a), lg.foot, 0, -0.112, 0.05).scale.set(0.133, 0.022, 0.203);
  }

  const spring = { a: 0, v: 0 };
  function animate(p, f) {
    const { t, dt } = f;
    // Yeux : la pupille suit le regard ; la paupière descend pour cligner ou s'endormir, et penche
    // (fâché / triste) selon eyeTilt.
    const look = new THREE.Vector3(clamp(p.eyeX * 6, -0.6, 0.6), clamp(p.eyeY * 6, -0.5, 0.5), 1).normalize();
    const open = clamp(p.eyeOpen * f.blink, 0, 1);
    for (const e of eyes) {
      e.pup.position.copy(look).multiplyScalar(EYE_R - 0.012);
      e.pup.quaternion.setFromUnitVectors(new THREE.Vector3(0, 0, 1), look);
      e.pup.scale.setScalar(p.eyeScale);
      e.lid.rotation.set(-0.55 + (1 - open) * 2.05, 0, p.eyeTilt * -e.s);
    }
    const sm = clamp(p.mouthSmile, -1, 1);
    const mo = clamp(f.mouthOpen, 0, 1);
    const w = sm >= 0 ? 0.34 * (0.7 + 0.3 * sm) : 0.14;
    const h = sm >= 0 ? 0.035 + 0.095 * sm + 0.06 * mo : 0.04 + 0.05 * mo;
    shapeMouth(w, h, -0.1);
    // Antenne : ressort amorti, jonction à la couleur de l'état de Jarvis.
    spring.v += (-36 * spring.a - 4.5 * spring.v - f.turnV * 0.8 + f.jumpV * 0.6) * dt;
    spring.a = clamp(spring.a + spring.v * dt, -0.6, 0.6);
    antenna.rotation.z = spring.a + 0.05 * Math.sin(t * 1.7);
    glow.color.copy(f.accent).multiplyScalar(0.4 + 0.7 * f.power * p.glow);
    knob.scale.setScalar(0.9 + 0.15 * Math.sin(t * 3) + 0.3 * f.level);
  }

  return { animate, thumbs };
}
