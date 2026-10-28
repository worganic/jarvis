// Marvin : robot à grosse tête sphérique du cœur de Jarvis (ui/character.js), d'après le dessin de
// référence : tête blanche fendue à l'horizontale, deux yeux triangulaires verts sous la fente, cou noir
// annelé, buste en œuf rayé de trois bandes noires, lignes noires en V au bassin, mains noires
// articulées, grosses bottes à semelle noire. Robot dépressif : tête basse, épaules tombantes, soupirs.
//
// Ses yeux sont verts au repos et prennent la couleur de l'état de Jarvis le reste du temps (vert : tu
// parles, bleu : « Jarvis » entendu, jaune : il réfléchit, rouge : il répond). Sans bouche : quand il
// parle, ses yeux s'illuminent au rythme des syllabes.
//
// Modèle construit en primitives (aucun fichier 3D). Squelette, animations et gestes : ui/characters/rig.js.
// Repère : pieds à l'origine, face vers +z, ~2,4 de haut.
import { createRig, clamp, pulse } from './rig.js';

export const meta = { id: 'marvin', name: 'Marvin', height: 2.4, restColor: [110, 214, 70] };
// Card Checkup (ui/checkup.js) : rotation lente et solennelle, son gros cerveau s'allume en premier.
export const scan = { spin: 8.5, dir: 1, hue: 0x5cf2b0, quirk: 'brain', brainScale: 1.9 };

const HEAD_R = 0.545;
const HEAD_C = [0, 0.475, 0.06];   // centre de la tête par rapport au pivot du cou
const SEAM_Y = -0.2;               // fente horizontale, sous le centre de la tête
const DZ = 0.82;                   // aplatissement avant-arrière du buste

const DIMS = { hipY: 0.52, hipX: 0.17, hipDy: -0.02, shoulder: [0.33, 0.66, 0], upper: 0.3, lower: 0.33, thigh: 0.18, shin: 0.2, neckY: 0.76, headY: 0.1 };

// Mains (repère du buste, main droite du personnage) et accessoires (repère du personnage). La tête est
// énorme : les gestes « en l'air » passent sur le côté, pas au-dessus.
const ANCHORS = {
  hand: {
    rest: [-0.48, 0.06, 0.06], hip: [-0.45, 0.22, 0.0], sleep: [-0.42, 0.06, 0.18], ear: [-0.62, 1.08, 0.12],
    chin: [-0.1, 0.88, 0.48], belly: [-0.04, 0.35, 0.38], talk: [-0.45, 0.32, 0.32],
    danceDown: [-0.45, 0.5, 0.3], danceUp: [-0.7, 1.05, 0.15], discoLow: [-0.14, 0.2, 0.42], discoHigh: [-0.8, 1.02, 0.2],
    robot: [-0.68, 0.6, 0.18], step: [-0.5, 0.35, 0.24], mail: [-0.3, 0.46, 0.62], globe: [-0.62, 0.48, 0.42],
    type: [-0.18, 0.4, 0.52], present: [-0.66, 0.66, 0.34], watch: [-0.12, 0.72, 0.5], shield: [-0.18, 0.5, 0.5],
    stop: [-0.56, 0.9, 0.4], shrug: [-0.7, 0.45, 0.3], thumb: [-0.5, 0.8, 0.42], tired: [-0.4, -0.02, 0.2],
    wave: [-0.72, 1.0, 0.16], bowFront: [-0.06, 0.35, 0.44], bowBack: [-0.36, 0.22, -0.3]
  },
  prop: {
    zzz: [0.62, 2.22, 0.3], waves: [-0.86, 1.85, 0.15], dots: [0.55, 2.45, 0.15], notes: [0, 1.9, 0.1],
    envelope: [0, 0.98, 0.7], globe: [-0.75, 1.3, 0.45], keyboard: [0, 1.0, 0.62], screen: [1.0, 1.5, 0.3],
    panel: [1.0, 1.32, 0.3], shield: [0.14, 1.0, 0.66], question: [0.55, 2.45, 0.15], sparkles: [0, 1.8, 0.2],
    sweat: [0.5, 2.1, 0.32]
  },
  danceAmp: 0.85
};

// Personnalité : tête lourde, épaules basses, regards las ; au repos, un grand soupir toutes les 9 s.
function tune(p, name, a, t) {
  p.headNod += 0.06;
  p.torsoLean += 0.04;
  p.eyeOpen *= 0.88;
  if (name === 'sleep') p.headNod = 0.32;
  if (name === 'idle') {
    p.headTurn *= 0.6;
    const c = t % 9;
    const sigh = pulse(c, 0.2, 1.0) * (1 - pulse(c, 1.8, 2.8));
    p.rootY -= 0.03 * sigh;
    p.headNod += 0.12 * sigh;
    p.torsoLean += 0.05 * sigh;
    p.eyeOpen *= 1 - 0.45 * sigh;
  }
}

export function create(THREE, { props }) {
  return createRig(THREE, { props, dims: DIMS, anchors: ANCHORS, tune, build: (j, kit) => build(THREE, j, kit) });
}

// Profil du buste (rayon, hauteur au-dessus du bassin), tourné autour de l'axe vertical.
const TORSO = [[0.0, -0.06], [0.14, -0.04], [0.24, 0.02], [0.31, 0.12], [0.345, 0.25], [0.35, 0.36], [0.335, 0.48],
  [0.3, 0.58], [0.27, 0.66], [0.22, 0.72], [0.12, 0.765], [0.0, 0.775]];
function torsoR(y) {
  for (let i = 1; i < TORSO.length; i++) {
    const [r0, y0] = TORSO[i - 1];
    const [r1, y1] = TORSO[i];
    if (y <= y1) return r0 + (r1 - r0) * clamp((y - y0) / (y1 - y0), 0, 1);
  }
  return 0;
}

function build(THREE, j, { keep, mesh }) {
  const white = keep(new THREE.MeshPhysicalMaterial({ color: 0xf1f1ee, roughness: 0.42, metalness: 0.02, clearcoat: 0.6, clearcoatRoughness: 0.3, envMapIntensity: 1.0 }));
  const black = keep(new THREE.MeshPhysicalMaterial({ color: 0x15171b, roughness: 0.38, metalness: 0.1, clearcoat: 0.5, clearcoatRoughness: 0.25, envMapIntensity: 1.1 }));
  const seam = keep(new THREE.MeshStandardMaterial({ color: 0x55585f, roughness: 0.6 }));
  const slot = keep(new THREE.MeshStandardMaterial({ color: 0x8f949c, roughness: 0.5 }));
  const socket = keep(new THREE.MeshBasicMaterial({ color: 0x0b120a, side: THREE.DoubleSide }));
  const eyeMat = keep(new THREE.MeshBasicMaterial({ color: 0x6ed646, vertexColors: true, side: THREE.DoubleSide, toneMapped: false }));
  const eyeGlow = keep(new THREE.MeshBasicMaterial({ color: 0x6ed646, transparent: true, opacity: 0.22, blending: THREE.AdditiveBlending, depthWrite: false, side: THREE.DoubleSide, toneMapped: false }));
  const { hips, torso, neck, head, arms, legs } = j;

  // Tube qui suit des points (fentes, lignes du buste).
  const tube = (pts, radius, mat, parent, closed = false) =>
    mesh(new THREE.TubeGeometry(new THREE.CatmullRomCurve3(pts, closed), Math.max(16, pts.length * 3), radius, 6, closed), mat, parent);

  // --- Buste ------------------------------------------------------------------------------------------
  const shell = new THREE.Group();
  shell.scale.z = DZ;
  torso.add(shell);
  const lathe = (pts, phiStart = 0, phiLength = Math.PI * 2, segs = 48) =>
    new THREE.LatheGeometry(pts.map(([r, y]) => new THREE.Vector2(r, y)), segs, phiStart, phiLength);
  const body = mesh(lathe(TORSO), white, shell);
  // Bande noire collée au buste entre deux hauteurs, sur une portion de tour (phi = 0 : devant).
  const band = (y0, y1, phiStart, phiLength, mat = black, lift = 0.004) => {
    const pts = [];
    for (let i = 0; i <= 4; i++) {
      const y = y0 + ((y1 - y0) * i) / 4;
      pts.push([torsoR(y) + lift, y]);
    }
    return mesh(lathe(pts, phiStart, phiLength, 40), mat, shell);
  };
  band(0.585, 0.615, -1.2, 2.4);
  band(0.53, 0.56, -1.3, 2.6);
  band(0.475, 0.505, -1.4, 2.8);
  band(0.42, 0.6, Math.PI - 0.95, 1.9);                // grande plaque noire du dos
  // Fente du ventre.
  const slotMesh = mesh(new THREE.CapsuleGeometry(0.008, 0.1, 4, 8), slot, torso, 0, 0.37, DZ * torsoR(0.37) + 0.004);
  slotMesh.rotation.z = Math.PI / 2;
  // Lignes en V du bassin, posées sur la surface du buste.
  const onBody = (x, y) => new THREE.Vector3(x, y, DZ * Math.sqrt(Math.max(0, torsoR(y) ** 2 - x * x)) + 0.006);
  for (const s of [-1, 1]) {
    tube([onBody(0.31 * s, 0.2), onBody(0.24 * s, 0.1), onBody(0.13 * s, 0.02), onBody(0.03 * s, -0.035)], 0.011, black, torso);
  }
  // Col noir autour du cou.
  mesh(new THREE.TorusGeometry(0.13, 0.03, 10, 32), black, torso, 0, 0.75, -0.02).rotation.x = Math.PI / 2;

  // --- Cou annelé et tête ------------------------------------------------------------------------------
  mesh(new THREE.CylinderGeometry(0.12, 0.11, 0.14, 20), black, neck, 0, 0.05, -0.06);
  for (let i = 0; i < 4; i++) {
    mesh(new THREE.TorusGeometry(0.125 + i * 0.012, 0.022, 8, 28), black, neck, 0, 0.0 + i * 0.035, -0.06 - i * 0.004).rotation.x = Math.PI / 2;
  }
  const skull = new THREE.Group();
  skull.position.set(...HEAD_C);
  head.add(skull);
  mesh(new THREE.SphereGeometry(HEAD_R, 72, 54), white, skull);
  // Fente horizontale : de la tempe droite à la tempe gauche, en passant par l'avant.
  const sr = Math.sqrt(HEAD_R * HEAD_R - SEAM_Y * SEAM_Y) + 0.002;
  const seamPts = [];
  for (let i = 0; i <= 24; i++) {
    const u = -1.75 + (3.5 * i) / 24;
    seamPts.push(new THREE.Vector3(sr * Math.sin(u), SEAM_Y, sr * Math.cos(u)));
  }
  tube(seamPts, 0.006, seam, skull);
  // Panneaux ronds des côtés de la tête.
  for (const s of [-1, 1]) {
    const n = new THREE.Vector3(s, 0.12, -0.28).normalize();
    const e1 = new THREE.Vector3(0, 1, 0).addScaledVector(n, -n.y).normalize();
    const e2 = new THREE.Vector3().crossVectors(n, e1);
    const alpha = 0.66;
    const c = n.clone().multiplyScalar((HEAD_R + 0.002) * Math.cos(alpha));
    const rho = (HEAD_R + 0.002) * Math.sin(alpha);
    const pts = [];
    for (let i = 0; i < 40; i++) {
      const v = (i / 40) * Math.PI * 2;
      pts.push(c.clone().addScaledVector(e1, rho * Math.cos(v)).addScaledVector(e2, rho * Math.sin(v)));
    }
    tube(pts, 0.005, seam, skull, true);
  }

  // Yeux : triangles posés sur la sphère (maillage subdivisé projeté), bord supérieur sous la fente,
  // pointe décalée vers le nez. Une orbite sombre derrière, un halo devant.
  const N = 6;
  const triGeo = (withColor) => {
    const count = ((N + 1) * (N + 2)) / 2;
    const geo = new THREE.BufferGeometry();
    geo.setAttribute('position', new THREE.BufferAttribute(new Float32Array(count * 3), 3));
    if (withColor) geo.setAttribute('color', new THREE.BufferAttribute(new Float32Array(count * 3), 3));
    const id = (i, k) => i * (N + 1) - (i * (i - 1)) / 2 + k;
    const idx = [];
    for (let i = 0; i < N; i++) {
      for (let k = 0; k < N - i; k++) {
        idx.push(id(i, k), id(i + 1, k), id(i, k + 1));
        if (k < N - i - 1) idx.push(id(i + 1, k), id(i + 1, k + 1), id(i, k + 1));
      }
    }
    geo.setIndex(idx);
    return keep(geo);
  };
  // Remplit la géométrie : triangle A (haut, extérieur), B (haut, intérieur), C (pointe), projeté à r.
  const fillTri = (geo, A, B, C, r) => {
    const pos = geo.attributes.position;
    const col = geo.attributes.color;
    let v = 0;
    for (let i = 0; i <= N; i++) {
      for (let k = 0; k <= N - i; k++) {
        const u = i / N;
        const w = k / N;
        const x = A[0] + (B[0] - A[0]) * u + (C[0] - A[0]) * w;
        const y = A[1] + (B[1] - A[1]) * u + (C[1] - A[1]) * w;
        pos.setXYZ(v, x, y, Math.sqrt(Math.max(0, r * r - x * x - y * y)));
        if (col) {
          const b = 1 - 0.5 * w; // plus clair sous la fente, plus sombre vers la pointe
          col.setXYZ(v, b, b, b);
        }
        v++;
      }
    }
    pos.needsUpdate = true;
    if (col) col.needsUpdate = true;
    geo.computeBoundingSphere();
  };
  const TOP = SEAM_Y - 0.008;
  const eyes = [-1, 1].map((s) => {
    const sock = triGeo(false);
    const glowGeo = triGeo(true);
    const haloGeo = triGeo(false);
    mesh(sock, socket, skull);
    mesh(glowGeo, eyeMat, skull);
    mesh(haloGeo, eyeGlow, skull);
    return { s, sock, glowGeo, haloGeo };
  });
  // s : -1 pour l'œil droit du personnage (à gauche de l'écran), 1 pour le gauche.
  const shapeEye = (e, p, f) => {
    const s = e.s;
    const mid = 0.275 * s;
    const half = 0.105 * p.eyeScale;
    const depth = 0.09 * clamp(p.eyeOpen * f.blink * (1 + 0.3 * clamp(p.eyeHappy, 0, 1)), 0.04, 1.45);
    const shift = p.eyeX * 0.5;
    const inner = mid - s * half + shift;   // coin côté nez
    const outer = mid + s * half + shift;   // coin côté tempe
    // Pointe : vers le nez ; « triste » (eyeTilt > 0) la rapproche encore, « sévère » l'éloigne.
    const tipX = mid - s * (0.057 + 0.05 * p.eyeTilt) + p.eyeX * 0.8;
    const tipY = TOP - depth + p.eyeY * 0.4;
    fillTri(e.glowGeo, [outer, TOP], [inner, TOP], [tipX, tipY], HEAD_R + 0.004);
    fillTri(e.sock, [outer + 0.014 * s, TOP + 0.004], [inner - 0.014 * s, TOP + 0.004], [tipX, tipY - 0.016], HEAD_R + 0.002);
    fillTri(e.haloGeo, [outer + 0.03 * s, TOP + 0.01], [inner - 0.03 * s, TOP + 0.01], [tipX, tipY - 0.035], HEAD_R + 0.007);
  };

  // --- Bras : épaule, bras, coude, avant-bras épais, main noire à doigts ----------------------------
  const thumbs = {};
  for (const [side, a] of Object.entries(arms)) {
    const s = a.s;
    mesh(new THREE.SphereGeometry(0.1, 20, 14), black, a.shoulder, -0.02 * s, 0.02, 0);
    mesh(new THREE.SphereGeometry(0.094, 24, 16), white, a.shoulder, 0, -0.03, 0);
    mesh(new THREE.CapsuleGeometry(0.084, DIMS.upper - 0.16, 6, 16), white, a.shoulder, 0, -DIMS.upper / 2, 0);
    mesh(new THREE.SphereGeometry(0.07, 16, 12), black, a.elbow);
    mesh(new THREE.CapsuleGeometry(0.034, 0.05, 4, 10), black, a.elbow, 0.07 * s, 0, 0).rotation.x = Math.PI / 2;
    mesh(new THREE.CapsuleGeometry(0.088, DIMS.lower - 0.16, 6, 16), white, a.elbow, 0, -DIMS.lower / 2 + 0.01, 0);
    mesh(new THREE.TorusGeometry(0.062, 0.018, 8, 20), black, a.wrist, 0, 0.01, 0).rotation.x = Math.PI / 2;
    mesh(new THREE.SphereGeometry(0.06, 16, 12), black, a.wrist, 0, -0.06, 0).scale.set(1, 1.15, 0.72);
    for (const fx of [-0.032, 0, 0.032]) {
      const finger = mesh(new THREE.CapsuleGeometry(0.017, 0.05, 4, 8), black, a.wrist, fx, -0.125, 0.012);
      finger.rotation.x = 0.35;
    }
    const thumb = new THREE.Group();
    thumb.position.set(0.05 * -s, -0.045, 0.035);
    thumb.rotation.z = 0.6 * s;
    a.wrist.add(thumb);
    mesh(new THREE.CapsuleGeometry(0.019, 0.045, 4, 8), black, thumb, 0, 0.03, 0);
    thumbs[side] = thumb;
  }

  // --- Jambes : cuisse, genou noir, tibia, grosse botte à semelle noire ------------------------------
  for (const lg of Object.values(legs)) {
    mesh(new THREE.CapsuleGeometry(0.095, DIMS.thigh - 0.1, 6, 14), white, lg.hip, 0, -DIMS.thigh / 2, 0);
    mesh(new THREE.SphereGeometry(0.082, 16, 12), black, lg.knee);
    mesh(new THREE.TorusGeometry(0.088, 0.016, 8, 24), black, lg.knee, 0, -0.01, 0).rotation.x = Math.PI / 2;
    mesh(new THREE.CapsuleGeometry(0.088, DIMS.shin - 0.12, 6, 14), white, lg.knee, 0, -DIMS.shin / 2, 0);
    mesh(new THREE.SphereGeometry(1, 32, 18), white, lg.foot, 0, 0.01, 0.06).scale.set(0.15, 0.13, 0.24);
    mesh(new THREE.TorusGeometry(0.1, 0.016, 8, 24), black, lg.foot, 0, 0.1, 0.0).rotation.x = Math.PI / 2;
    mesh(new THREE.SphereGeometry(1, 32, 12), black, lg.foot, 0, -0.105, 0.06).scale.set(0.152, 0.024, 0.242);
  }

  const green = new THREE.Color();
  function animate(p, f) {
    const breathe = 1 + 0.01 * Math.sin(f.t * 1.4);
    body.scale.set(2 - breathe, breathe, 1);
    for (const e of eyes) shapeEye(e, p, f);
    // Sans bouche : les yeux s'illuminent au rythme de la parole.
    const power = f.power * p.glow;
    const talk = 1 + 0.45 * clamp(f.mouthOpen, 0, 1);
    green.copy(f.accent).multiplyScalar((0.3 + 0.75 * power) * talk);
    eyeMat.color.copy(green);
    eyeGlow.color.copy(f.accent);
    eyeGlow.opacity = 0.22 * power * talk;
  }

  return { animate, thumbs };
}
