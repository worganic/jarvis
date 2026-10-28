// Bobot : petit robot 3D du cœur de Jarvis (ui/character.js) — tête blanche à visière noire, grands
// yeux lumineux, antenne, lumière de poitrine. Ses yeux prennent la couleur de l'état de Jarvis (vert :
// tu parles, bleu : « Jarvis » entendu, jaune : il réfléchit, rouge : il répond ; au repos, le thème).
//
// Modèle construit en primitives (aucun fichier 3D à charger). Squelette, animations et gestes :
// ui/characters/rig.js. Repère : pieds à l'origine, face vers +z, ~2,45 de haut antenne comprise.
import { createRig, clamp } from './rig.js';

export const meta = { id: 'bobot', name: 'Bobot', height: 2.45 };
// Card Checkup (ui/checkup.js) : rotation franche, sa visière balaie avec le faisceau.
export const scan = { spin: 6, dir: 1, hue: 0x38e1ff, quirk: 'visor' };

const HEAD_C = 0.38;                // centre de la tête au-dessus du pivot du cou
const HEAD_R = 0.42;
const HEAD_S = [1.12, 0.94, 1.0];

const DIMS = { hipY: 0.66, hipX: 0.15, hipDy: -0.02, shoulder: [0.37, 0.55, 0], upper: 0.3, lower: 0.28, thigh: 0.26, shin: 0.25, neckY: 0.72, headY: 0.06 };

const ANCHORS = {
  hand: {
    rest: [-0.45, 0.0, 0.07], hip: [-0.5, 0.12, 0.02], sleep: [-0.38, -0.02, 0.16], ear: [-0.6, 0.98, 0.1],
    chin: [-0.12, 0.8, 0.36], belly: [-0.02, 0.28, 0.42], talk: [-0.42, 0.24, 0.3],
    danceDown: [-0.42, 0.45, 0.28], danceUp: [-0.3, 1.12, 0.18], discoLow: [-0.12, 0.12, 0.42], discoHigh: [-0.74, 1.0, 0.2],
    robot: [-0.6, 0.5, 0.16], step: [-0.46, 0.3, 0.22], mail: [-0.29, 0.58, 0.52], globe: [-0.5, 0.55, 0.42],
    type: [-0.17, 0.34, 0.5], present: [-0.6, 0.74, 0.34], watch: [-0.1, 0.66, 0.46], shield: [-0.16, 0.48, 0.46],
    stop: [-0.5, 0.84, 0.36], shrug: [-0.64, 0.4, 0.3], thumb: [-0.45, 0.72, 0.4], tired: [-0.36, -0.07, 0.2],
    wave: [-0.62, 0.98, 0.16], bowFront: [-0.06, 0.3, 0.42], bowBack: [-0.34, 0.18, -0.3]
  },
  prop: {
    zzz: [0.5, 1.92, 0.25], waves: [-0.82, 1.86, 0.15], dots: [0.42, 2.38, 0.15], notes: [0, 1.75, 0.1],
    envelope: [0, 1.24, 0.6], globe: [-0.58, 1.52, 0.42], keyboard: [0, 0.98, 0.56], screen: [0.86, 1.62, 0.3],
    panel: [0.88, 1.6, 0.3], shield: [0.12, 1.12, 0.62], question: [0.42, 2.36, 0.15], sparkles: [0, 1.7, 0.1],
    sweat: [0.5, 1.98, 0.28]
  }
};

export function create(THREE, { props }) {
  return createRig(THREE, { props, dims: DIMS, anchors: ANCHORS, build: (j, kit) => build(THREE, j, kit) });
}

function build(THREE, j, { keep, mesh }) {
  const shell = keep(new THREE.MeshPhysicalMaterial({ color: 0xeef2f8, roughness: 0.3, metalness: 0.05, clearcoat: 1, clearcoatRoughness: 0.12, envMapIntensity: 1.1 }));
  const dark = keep(new THREE.MeshPhysicalMaterial({ color: 0x0a0f17, roughness: 0.12, metalness: 0.3, clearcoat: 1, clearcoatRoughness: 0.05, envMapIntensity: 1.4 }));
  const metal = keep(new THREE.MeshStandardMaterial({ color: 0x8c98ab, metalness: 0.85, roughness: 0.3, envMapIntensity: 1.2 }));
  const glow = keep(new THREE.MeshBasicMaterial({ color: 0x38e1ff, toneMapped: false }));
  const halo = keep(new THREE.MeshBasicMaterial({ color: 0x38e1ff, transparent: true, opacity: 0.28, blending: THREE.AdditiveBlending, depthWrite: false, toneMapped: false }));
  const blushMat = keep(new THREE.MeshBasicMaterial({ color: 0xff6b9a, transparent: true, opacity: 0, depthWrite: false, toneMapped: false }));
  const { hips, torso, neck, head, arms, legs } = j;

  mesh(new THREE.SphereGeometry(0.2, 32, 16), metal, hips).scale.set(1.1, 0.6, 0.9);

  // Buste : œuf blanc, ceinture sombre, lumière de poitrine.
  const body = mesh(new THREE.SphereGeometry(0.36, 48, 32), shell, torso, 0, 0.32, 0);
  body.scale.set(1, 1.08, 0.86);
  const belt = mesh(new THREE.TorusGeometry(0.335, 0.022, 10, 48), dark, torso, 0, 0.1, 0);
  belt.rotation.x = Math.PI / 2;
  belt.scale.set(1, 0.86, 1);
  const chest = new THREE.Group();
  chest.position.set(0, 0.38, 0.296);
  chest.rotation.x = -0.12;
  torso.add(chest);
  mesh(new THREE.CircleGeometry(0.1, 32), dark, chest, 0, 0, 0.004);
  mesh(new THREE.TorusGeometry(0.072, 0.014, 10, 32), glow, chest, 0, 0, 0.012);
  const chestCore = mesh(new THREE.CircleGeometry(0.045, 32), glow, chest, 0, 0, 0.014);
  const chestHalo = mesh(new THREE.CircleGeometry(0.14, 32), halo, chest, 0, 0, 0.016);

  // Cou et tête.
  mesh(new THREE.CylinderGeometry(0.07, 0.09, 0.1, 20), metal, neck, 0, 0.03, 0);
  mesh(new THREE.SphereGeometry(HEAD_R, 64, 48), shell, head, 0, HEAD_C, 0).scale.set(...HEAD_S);
  // Visière noire : calotte légèrement plus grande que le crâne, sur l'avant.
  const VR = HEAD_R * 1.012;
  mesh(new THREE.SphereGeometry(VR, 48, 32, Math.PI / 2 - 1.0, 2.0, 1.07, 0.98), dark, head, 0, HEAD_C, 0).scale.set(...HEAD_S);
  const va = VR * HEAD_S[0];
  const vb = VR * HEAD_S[1];
  const vc = VR * HEAD_S[2];
  // Pose un objet sur la visière (x, y relatifs au centre de la tête), tourné selon la normale.
  const zAxis = new THREE.Vector3(0, 0, 1);
  const face = (obj, x, y) => {
    const z = vc * Math.sqrt(Math.max(0.02, 1 - (x / va) ** 2 - (y / vb) ** 2));
    obj.position.set(x, HEAD_C + y, z + 0.006);
    obj.quaternion.setFromUnitVectors(zAxis, new THREE.Vector3(x / (va * va), y / (vb * vb), z / (vc * vc)).normalize());
  };

  // Yeux : disque lumineux (ouvert) ou arc (content), halo derrière.
  const eyes = [-1, 1].map((side) => {
    const g = new THREE.Group();
    const tilt = new THREE.Group();
    g.add(tilt);
    const disc = mesh(new THREE.CircleGeometry(1, 32), glow, tilt);
    const arc = mesh(new THREE.TorusGeometry(0.062, 0.017, 8, 24, Math.PI), glow, tilt);
    const h = mesh(new THREE.CircleGeometry(1, 32), halo, tilt, 0, 0, -0.002);
    head.add(g);
    return { g, tilt, disc, arc, halo: h, side };
  });
  const EYE_X = 0.15;
  const EYE_Y = 0.07;

  // Bouche : sourire (arc) ou bouche ouverte (disque), joues roses.
  const mouth = new THREE.Group();
  head.add(mouth);
  const smile = mesh(new THREE.TorusGeometry(0.055, 0.011, 8, 20, Math.PI), glow, mouth);
  const open = mesh(new THREE.CircleGeometry(1, 24), glow, mouth);
  face(mouth, 0, -0.17);
  const cheeks = [-1, 1].map((s) => {
    const c = mesh(new THREE.CircleGeometry(0.045, 20), blushMat, head);
    face(c, 0.26 * s, -0.08);
    c.position.z += 0.002;
    return c;
  });

  // Antenne (ressort amorti) et oreilles.
  const antenna = new THREE.Group();
  antenna.position.set(0, HEAD_C + HEAD_R * HEAD_S[1] - 0.02, 0);
  head.add(antenna);
  mesh(new THREE.CylinderGeometry(0.011, 0.016, 0.2, 10), metal, antenna, 0, 0.1, 0);
  const tip = mesh(new THREE.SphereGeometry(0.045, 20, 14), glow, antenna, 0, 0.22, 0);
  const tipHalo = mesh(new THREE.SphereGeometry(0.085, 20, 14), halo, antenna, 0, 0.22, 0);
  for (const s of [-1, 1]) {
    mesh(new THREE.CylinderGeometry(0.105, 0.105, 0.08, 32), shell, head, s * (HEAD_R * HEAD_S[0] - 0.01), HEAD_C, 0).rotation.z = Math.PI / 2;
    mesh(new THREE.TorusGeometry(0.07, 0.013, 10, 32), glow, head, s * (HEAD_R * HEAD_S[0] + 0.035), HEAD_C, 0).rotation.y = Math.PI / 2;
  }

  // Bras : épaule, bras, coude, avant-bras, main (moufle + pouce).
  const thumbs = {};
  for (const [side, a] of Object.entries(arms)) {
    const s = a.s;
    mesh(new THREE.SphereGeometry(0.088, 20, 14), metal, a.shoulder);
    mesh(new THREE.CapsuleGeometry(0.062, DIMS.upper - 0.12, 6, 16), shell, a.shoulder, 0, -DIMS.upper / 2, 0);
    mesh(new THREE.SphereGeometry(0.058, 16, 12), metal, a.elbow);
    mesh(new THREE.CapsuleGeometry(0.057, DIMS.lower - 0.12, 6, 16), shell, a.elbow, 0, -DIMS.lower / 2 + 0.01, 0);
    mesh(new THREE.TorusGeometry(0.052, 0.014, 8, 20), dark, a.wrist, 0, 0.02, 0).rotation.x = Math.PI / 2;
    mesh(new THREE.SphereGeometry(0.082, 20, 14), shell, a.wrist, 0, -0.05, 0).scale.set(1, 1.05, 0.82);
    const thumb = new THREE.Group();
    thumb.position.set(0.055 * -s, -0.03, 0.04);
    thumb.rotation.z = 0.6 * s;
    a.wrist.add(thumb);
    mesh(new THREE.CapsuleGeometry(0.024, 0.045, 4, 10), shell, thumb, 0, 0.03, 0);
    thumbs[side] = thumb;
  }

  // Jambes : cuisse, genou, tibia, pied.
  for (const lg of Object.values(legs)) {
    mesh(new THREE.CapsuleGeometry(0.072, DIMS.thigh - 0.12, 6, 14), metal, lg.hip, 0, -DIMS.thigh / 2, 0);
    mesh(new THREE.SphereGeometry(0.07, 16, 12), dark, lg.knee);
    mesh(new THREE.CapsuleGeometry(0.08, DIMS.shin - 0.14, 6, 14), shell, lg.knee, 0, -DIMS.shin / 2, 0);
    mesh(new THREE.SphereGeometry(0.13, 28, 16), shell, lg.foot, 0, -0.06, 0.05).scale.set(0.95, 0.52, 1.3);
  }

  const spring = { a: 0, v: 0, b: 0, w: 0 };
  const EYE_HALO = 0.28;

  function animate(p, f) {
    const { t, dt } = f;
    const breathe = 1 + 0.012 * Math.sin(t * 1.8);
    body.scale.set(2 - breathe, 1.08 * breathe, 0.86);

    const power = f.power * p.glow;
    glow.color.copy(f.accent).multiplyScalar(0.35 + 0.75 * power);
    halo.color.copy(f.accent);
    halo.opacity = EYE_HALO * power;
    const happy = clamp(p.eyeHappy, 0, 1);
    const openY = clamp(p.eyeOpen * f.blink, 0.04, 1.2);
    for (const e of eyes) {
      face(e.g, e.side * EYE_X + p.eyeX, EYE_Y + p.eyeY);
      e.tilt.rotation.z = p.eyeTilt * -e.side;
      const sc = p.eyeScale;
      e.disc.scale.set(0.068 * sc, Math.max(0.004, 0.092 * sc * openY * (1 - happy)), 1);
      e.disc.visible = happy < 0.98;
      e.arc.visible = happy > 0.02;
      e.arc.scale.set(sc, sc * happy * Math.max(0.3, f.blink), 1);
      e.halo.scale.set(0.13 * sc, 0.15 * sc * Math.max(0.25, openY), 1);
    }
    const sm = clamp(p.mouthSmile, -1, 1);
    const mo = clamp(f.mouthOpen, 0, 1);
    smile.visible = mo < 0.15 && Math.abs(sm) > 0.02;
    smile.rotation.z = sm >= 0 ? Math.PI : 0;
    smile.scale.set(0.6 + 0.4 * Math.abs(sm), Math.max(0.05, Math.abs(sm)), 1);
    smile.position.y = sm >= 0 ? 0.02 : -0.03;
    open.visible = mo >= 0.15;
    open.scale.set(0.05 + 0.012 * mo, 0.012 + 0.045 * mo, 1);
    blushMat.opacity = 0.55 * clamp(p.blush, 0, 1);
    for (const c of cheeks) c.visible = p.blush > 0.02;

    // Antenne : ressort amorti entraîné par les mouvements de la tête et les sauts.
    spring.v += (-40 * spring.a - 5 * spring.v - f.turnV * 0.9) * dt;
    spring.a = clamp(spring.a + spring.v * dt, -0.6, 0.6);
    spring.w += (-40 * spring.b - 5 * spring.w + f.jumpV * 1.2) * dt;
    spring.b = clamp(spring.b + spring.w * dt, -0.5, 0.5);
    antenna.rotation.set(spring.b + 0.04 * Math.sin(t * 1.3), 0, spring.a);
    const tipPulse = 0.85 + 0.15 * Math.sin(t * 3.2);
    tip.scale.setScalar(tipPulse);
    tipHalo.scale.setScalar(tipPulse * (1 + 0.25 * f.level));
    chestCore.scale.setScalar(0.9 + 0.1 * Math.sin(t * 2.4) + 0.25 * f.level);
    chestHalo.scale.setScalar(1 + 0.15 * Math.sin(t * 2.4));
  }

  return { animate, thumbs };
}
