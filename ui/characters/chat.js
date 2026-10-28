// Chat grincheux du cœur de Jarvis (ui/character.js), d'après le dessin de référence : grosse tête ronde
// en velours orange, oreilles à l'intérieur rose, sourcils froncés, gros yeux noirs brillants, bas du
// visage blanc, nez rose, petite moue, joues roses, moustaches, ventre et pattes blancs, queue au bout
// blanc. Bougon : bras croisés, tête détournée (avec un coup d'œil de temps en temps), queue qui fouette,
// oreilles qui se couchent quand ça ne va pas et se dressent quand il écoute.
//
// Les reflets de ses yeux prennent la couleur de l'état de Jarvis (blancs au repos).
// Squelette, animations et gestes : ui/characters/rig.js. Repère : pieds à l'origine, face vers +z,
// ~2,4 de haut oreilles comprises.
import { createRig, clamp, wander, makePatch, profileR, placeOn } from './rig.js';

export const meta = { id: 'chat', name: 'Chat grincheux', height: 2.4, restColor: [255, 255, 255] };
// Card Checkup (ui/checkup.js) : tourne à contre-sens, en soupirant (il déteste être examiné).
export const scan = { spin: 6.5, dir: -1, hue: 0xffb45c, quirk: 'sigh' };

const HEAD = [0.57, 0.5, 0.51];     // demi-axes de la tête
const HEAD_C = [0, 0.5, 0.03];      // centre de la tête par rapport au pivot du cou
const DZ = 0.85;                    // aplatissement avant-arrière du corps
const BODY = [[0, -0.05], [0.17, -0.03], [0.27, 0.04], [0.32, 0.16], [0.335, 0.3], [0.32, 0.46], [0.29, 0.62],
  [0.24, 0.76], [0.17, 0.86], [0.08, 0.93], [0, 0.95]];

const DIMS = { hipY: 0.42, hipX: 0.15, hipDy: -0.02, shoulder: [0.29, 0.78, 0], upper: 0.26, lower: 0.25, thigh: 0.14, shin: 0.14, neckY: 0.92, headY: 0.02 };

const ANCHORS = {
  hand: {
    rest: [-0.38, 0.3, 0.12], hip: [-0.38, 0.4, 0.05], sleep: [-0.3, 0.32, 0.2], ear: [-0.45, 1.0, 0.1],
    chin: [-0.06, 0.92, 0.42], belly: [-0.04, 0.5, 0.38], talk: [-0.36, 0.55, 0.32],
    danceDown: [-0.36, 0.6, 0.3], danceUp: [-0.55, 1.05, 0.15], discoLow: [-0.1, 0.4, 0.38], discoHigh: [-0.62, 1.08, 0.18],
    robot: [-0.52, 0.72, 0.15], step: [-0.4, 0.5, 0.22], mail: [-0.25, 0.68, 0.45], globe: [-0.45, 0.65, 0.38],
    type: [-0.15, 0.55, 0.45], present: [-0.5, 0.82, 0.3], watch: [-0.1, 0.78, 0.42], shield: [-0.16, 0.6, 0.45],
    stop: [-0.45, 0.95, 0.35], shrug: [-0.55, 0.6, 0.28], thumb: [-0.4, 0.88, 0.36], tired: [-0.32, 0.3, 0.2],
    wave: [-0.55, 1.08, 0.15], bowFront: [-0.05, 0.5, 0.4], bowBack: [-0.3, 0.4, -0.25]
  },
  prop: {
    zzz: [0.62, 2.22, 0.3], waves: [-0.86, 1.9, 0.15], dots: [0.74, 2.28, 0.15], notes: [0, 1.95, 0.1],
    envelope: [0, 1.12, 0.6], globe: [-0.75, 1.3, 0.45], keyboard: [0, 1.0, 0.58], screen: [1.0, 1.32, 0.3],
    panel: [1.0, 1.3, 0.3], shield: [0.12, 1.05, 0.62], question: [0.74, 2.28, 0.15], sparkles: [0, 1.8, 0.2],
    sweat: [0.55, 2.05, 0.35]
  },
  danceAmp: 0.9
};

// Bras croisés (main droite sur le bras gauche, et inversement).
const CROSS = { rH: [0.17, 0.62, 0.38], lH: [-0.19, 0.66, 0.42], rP: [-1, -0.7, 0.2], lP: [1, -0.7, 0.2] };
const CROSSED = ['idle', 'listen', 'talk', 'work'];
const SAD = ['shrug', 'tired', 'sleep'];
const PLEASED = ['happy', 'dance', 'wave', 'bow'];

// Bougon : bras croisés, tête détournée avec un coup d'œil toutes les 7 s, sourcils froncés, moue.
function tune(p, name, a, t) {
  if (CROSSED.includes(name)) {
    p.rH = [...CROSS.rH];
    p.lH = [...CROSS.lH];
    p.rP = [...CROSS.rP];
    p.lP = [...CROSS.lP];
  }
  if (name === 'idle') {
    const c = t % 7;
    const glance = c > 5.2 && c < 6.4 ? 1 : 0;
    p.headTurn = (-0.26 + 0.06 * wander(t * 0.3, 2)) * (1 - glance) + 0.04 * glance;
    p.headNod -= 0.05;
    p.eyeX = glance ? 0 : -0.03;
  }
  if (!SAD.includes(name)) p.eyeTilt -= 0.16;
  p.eyeOpen *= 0.82;
  p.mouthSmile = PLEASED.includes(name) ? 0.2 : -0.7;
}

export function create(THREE, { props }) {
  return createRig(THREE, { props, dims: DIMS, anchors: ANCHORS, tune, build: (j, kit) => build(THREE, j, kit) });
}

function build(THREE, j, { keep, mesh }) {
  const velvet = (color, sheen) => keep(new THREE.MeshPhysicalMaterial({ color, roughness: 0.82, metalness: 0, sheen: 1, sheenColor: sheen, sheenRoughness: 0.45, envMapIntensity: 0.8 }));
  const fur = velvet(0xd98a52, 0xffc89c);
  const furDark = velvet(0xa85a2a, 0xd7895a);
  const cream = velvet(0xf5eee3, 0xffffff);
  const pink = velvet(0xf2a6a0, 0xffd6d2);
  const blush = keep(new THREE.MeshStandardMaterial({ color: 0xe9586a, roughness: 0.9, side: THREE.DoubleSide }));
  const nose = keep(new THREE.MeshPhysicalMaterial({ color: 0xe98b95, roughness: 0.4, clearcoat: 0.6 }));
  const eyeMat = keep(new THREE.MeshPhysicalMaterial({ color: 0x050506, roughness: 0.08, clearcoat: 1, clearcoatRoughness: 0.05, envMapIntensity: 1.6 }));
  const spark = keep(new THREE.MeshBasicMaterial({ color: 0xffffff, toneMapped: false }));
  const line = keep(new THREE.MeshStandardMaterial({ color: 0x3a2418, roughness: 0.7 }));
  const mouthIn = keep(new THREE.MeshStandardMaterial({ color: 0x3a1418, roughness: 0.8, side: THREE.DoubleSide }));
  const whiskerMat = keep(new THREE.LineBasicMaterial({ color: 0x111111 }));
  const { hips, torso, neck, head, arms, legs } = j;

  // --- Corps en poire, ventre blanc ------------------------------------------------------------------
  const shell = new THREE.Group();
  shell.scale.z = DZ;
  torso.add(shell);
  const body = mesh(new THREE.LatheGeometry(BODY.map(([r, y]) => new THREE.Vector2(r, y)), 48), fur, shell);
  const bellyOut = [];
  for (let i = 0; i < 32; i++) {
    const phi = (i / 32) * Math.PI * 2;
    bellyOut.push([0.2 * Math.cos(phi), 0.32 + 0.27 * Math.sin(phi)]);
  }
  const belly = makePatch(THREE, 5);
  belly.set(bellyOut, [0, 0.32], (x, y) => DZ * Math.sqrt(Math.max(0, profileR(BODY, y) ** 2 - x * x)), 0.006);
  mesh(belly.geo, cream, torso);
  mesh(new THREE.SphereGeometry(0.13, 24, 16), fur, neck, 0, 0.03, -0.02).scale.set(1.2, 0.7, 1);

  // --- Queue : chaîne de segments qui fouette, bout blanc ---------------------------------------------
  const tail = [];
  let parent = new THREE.Group();
  parent.position.set(0, 0.1, -0.25);
  parent.rotation.x = -2.25;
  torso.add(parent);
  const SEG = 8;
  for (let i = 0; i < SEG; i++) {
    const g = new THREE.Group();
    if (i) g.position.y = 0.095;
    parent.add(g);
    const r = 0.058 - i * 0.002;
    mesh(new THREE.CapsuleGeometry(r, 0.06, 4, 12), i >= SEG - 2 ? cream : fur, g, 0, 0.05, 0);
    tail.push({ g, base: i === 0 ? 0 : (i < 4 ? 0.36 : 0.22) });
    parent = g;
  }
  mesh(new THREE.SphereGeometry(0.06, 16, 12), cream, parent, 0, 0.11, 0);

  // --- Tête -----------------------------------------------------------------------------------------
  const skull = new THREE.Group();
  skull.position.set(...HEAD_C);
  head.add(skull);
  mesh(new THREE.SphereGeometry(1, 64, 48), fur, skull).scale.set(...HEAD);
  // Bas du visage blanc : calotte avant-bas de la tête.
  mesh(new THREE.SphereGeometry(1.008, 48, 24, Math.PI / 2 - 1.35, 2.7, 1.72, 1.2), cream, skull).scale.set(...HEAD);
  const on = (obj, x, y, lift) => placeOn(THREE, obj, x, y, [0, 0, 0], HEAD, lift);

  // Oreilles : cône aplati orange, intérieur rose ; pivot à la base pour les coucher ou les dresser.
  const ears = [-1, 1].map((s) => {
    const g = new THREE.Group();
    g.position.set(0.34 * s, 0.33, -0.03);
    g.rotation.order = 'ZXY';
    skull.add(g);
    const outer = new THREE.ConeGeometry(0.17, 0.32, 28);
    outer.translate(0, 0.16, 0);
    mesh(outer, fur, g).scale.z = 0.42;
    const inner = new THREE.ConeGeometry(0.11, 0.22, 24);
    inner.translate(0, 0.11, 0);
    const im = mesh(inner, pink, g, 0, 0.02, 0.035);
    im.scale.z = 0.28;
    return { g, s };
  });

  // Yeux : globe noir brillant, deux reflets, paupière orange ; sourcils de fourrure foncée.
  const eyes = [-1, 1].map((s) => {
    const g = new THREE.Group();
    on(g, 0.18 * s, 0.02, -0.03);
    skull.add(g);
    mesh(new THREE.SphereGeometry(0.075, 32, 24), eyeMat, g);
    mesh(new THREE.SphereGeometry(0.017, 10, 8), spark, g, -0.024, 0.028, 0.066);
    mesh(new THREE.SphereGeometry(0.009, 8, 6), spark, g, 0.022, -0.018, 0.071);
    const lid = new THREE.Group();
    lid.rotation.order = 'ZXY';
    g.add(lid);
    mesh(new THREE.SphereGeometry(0.079, 32, 12, 0, Math.PI * 2, 0, Math.PI / 2), fur, lid);
    const brow = new THREE.Group();
    on(brow, 0.19 * s, 0.135, 0.006);
    skull.add(brow);
    const tilt = new THREE.Group();
    brow.add(tilt);
    mesh(new THREE.CapsuleGeometry(0.022, 0.12, 4, 10), furDark, tilt).rotation.z = Math.PI / 2;
    return { s, g, lid, tilt };
  });

  // Nez, bouche (moue ou bouche ouverte), joues roses, moustaches.
  const noseM = mesh(new THREE.SphereGeometry(1, 20, 14), nose, skull);
  on(noseM, 0, -0.07, 0.0);
  noseM.scale.set(0.04, 0.028, 0.03);
  const mouth = new THREE.Group();
  on(mouth, 0, -0.16, 0.006);
  skull.add(mouth);
  const pout = mesh(new THREE.TorusGeometry(0.045, 0.008, 6, 16, Math.PI), line, mouth);
  const gape = mesh(new THREE.CircleGeometry(1, 20), mouthIn, mouth, 0, -0.01, 0.002);
  for (const s of [-1, 1]) {
    const c = mesh(new THREE.CircleGeometry(1, 24), blush, skull);
    on(c, 0.3 * s, -0.1, 0.006);
    c.scale.set(0.06, 0.04, 1);
  }
  const whiskers = [-1, 1].map((s) => {
    const g = new THREE.Group();
    on(g, 0.34 * s, -0.09, 0.004);
    g.quaternion.identity();
    skull.add(g);
    const pts = [];
    for (const [dy, slope] of [[0.03, 0.12], [0, 0], [-0.03, -0.12]]) {
      pts.push(new THREE.Vector3(0, dy, 0), new THREE.Vector3(0.42 * s, dy + slope * 0.42, -0.05));
    }
    g.add(new THREE.LineSegments(keep(new THREE.BufferGeometry().setFromPoints(pts)), whiskerMat));
    return { g, s };
  });

  // --- Bras dodus, pattes blanches -------------------------------------------------------------------
  const thumbs = {};
  for (const [side, a] of Object.entries(arms)) {
    const s = a.s;
    mesh(new THREE.SphereGeometry(0.085, 18, 14), fur, a.shoulder);
    mesh(new THREE.CapsuleGeometry(0.078, DIMS.upper - 0.1, 6, 14), fur, a.shoulder, 0, -DIMS.upper / 2, 0);
    mesh(new THREE.SphereGeometry(0.074, 16, 12), fur, a.elbow);
    mesh(new THREE.CapsuleGeometry(0.074, DIMS.lower - 0.1, 6, 14), fur, a.elbow, 0, -DIMS.lower / 2, 0);
    mesh(new THREE.SphereGeometry(0.078, 20, 14), cream, a.wrist, 0, -0.03, 0).scale.set(1, 0.95, 0.85);
    const thumb = new THREE.Group();
    thumb.position.set(0.05 * -s, -0.02, 0.04);
    thumb.rotation.z = 0.6 * s;
    a.wrist.add(thumb);
    mesh(new THREE.CapsuleGeometry(0.026, 0.04, 4, 10), cream, thumb, 0, 0.03, 0);
    thumbs[side] = thumb;
  }

  // --- Jambes courtes, grosses pattes blanches --------------------------------------------------------
  for (const lg of Object.values(legs)) {
    mesh(new THREE.CapsuleGeometry(0.095, 0.06, 6, 14), fur, lg.hip, 0, -DIMS.thigh / 2, 0);
    mesh(new THREE.CapsuleGeometry(0.088, 0.06, 6, 14), fur, lg.knee, 0, -DIMS.shin / 2, 0);
    mesh(new THREE.SphereGeometry(1, 32, 16), cream, lg.foot, 0, -0.035, 0.05).scale.set(0.12, 0.085, 0.17);
    for (const fx of [-0.035, 0.035]) {
      const toe = mesh(new THREE.CapsuleGeometry(0.004, 0.05, 2, 6), line, lg.foot, fx, -0.03, 0.205);
      toe.rotation.x = Math.PI / 2 - 0.5;
    }
  }

  // Oreilles selon l'humeur : dressées en écoute, couchées quand ça va mal, tombantes en dormant.
  const EAR = { listen: [-0.35, 0.15], happy: [-0.1, 0.25], dance: [-0.1, 0.3], shrug: [0.35, 0.95], guard: [0.25, 0.8], tired: [0.3, 0.7], sleep: [0.25, 0.6] };
  const earNow = { x: 0, z: 0.35 };
  let twitchAt = 3;
  function animate(p, f) {
    const { t, dt } = f;
    const breathe = 1 + 0.012 * Math.sin(t * 1.5);
    body.scale.set(2 - breathe, breathe, 1);
    const open = clamp(p.eyeOpen * f.blink, 0, 1);
    for (const e of eyes) {
      e.lid.rotation.set(-0.5 + (1 - open) * 2.0, 0, p.eyeTilt * -e.s * 1.2);
      e.tilt.rotation.z = e.s * (0.3 - 1.3 * p.eyeTilt);
      e.tilt.position.y = 0.02 * (open - 0.8);
    }
    spark.color.copy(f.accent).multiplyScalar(0.6 + 0.6 * f.power);
    const sm = clamp(p.mouthSmile, -1, 1);
    const mo = clamp(f.mouthOpen, 0, 1);
    pout.visible = mo < 0.2;
    pout.rotation.z = sm >= 0 ? Math.PI : 0;
    pout.scale.set(0.7 + 0.3 * Math.abs(sm), Math.max(0.2, Math.abs(sm)), 1);
    gape.visible = mo >= 0.2;
    gape.scale.set(0.035 + 0.01 * mo, 0.012 + 0.035 * mo, 1);
    for (const w of whiskers) w.g.rotation.z = w.s * (0.04 * Math.sin(t * 1.3) + 0.08 * mo);

    // Oreilles : position selon l'animation, petit frémissement de temps en temps.
    const [ex, ez] = EAR[f.action] || [0, 0.35];
    earNow.x += (ex - earNow.x) * Math.min(1, dt * 6);
    earNow.z += (ez - earNow.z) * Math.min(1, dt * 6);
    if (t > twitchAt + 0.25) twitchAt = t + 2.5 + Math.random() * 4;
    const twitch = t > twitchAt ? Math.sin((t - twitchAt) * 40) * 0.12 : 0;
    for (const e of ears) e.g.rotation.set(earNow.x + (e.s > 0 ? twitch : 0), 0, -e.s * earNow.z);

    // Queue : fouette lentement, vite quand il est contrarié, en rythme quand il danse.
    const angry = ['shrug', 'guard', 'tired'].includes(f.action);
    const speed = f.action === 'dance' ? Math.PI * 2 : angry ? 5 : 1.6;
    const amp = angry ? 0.32 : 0.18;
    tail.forEach((seg, i) => {
      seg.g.rotation.x = seg.base;
      seg.g.rotation.z = amp * Math.sin(t * speed - i * 0.55) * (i ? 1 : 0.4);
    });
  }

  return { animate, thumbs };
}
