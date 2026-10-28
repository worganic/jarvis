// Card « Checkup » : le cœur de Jarvis passé au scanner holographique (module Checkup : modules/checkup/ui/checkup.js › renderCheckup).
// Le personnage du cœur (ou le cœur holographique classique) est rendu en hologramme translucide, son
// squelette visible, entouré d'anneaux lumineux. Deux temps :
//   1. rotation : il fait un tour complet pendant qu'un faisceau descend de la tête aux pieds et le révèle ;
//   2. bilan : il s'immobilise face à toi ; le faisceau balaie de haut en bas en boucle et chaque organe
//      (fonction vitale de Jarvis, crates/jarvis-core/src/orch/checkup.rs) s'allume à la couleur de son état.
// Chaque cœur a sa signature de scan (export `scan` de ui/characters/<id>.js).
import * as THREE from '/vendor/three.module.min.js';
import { createProps } from '/characters/props.js';
import * as bobot from '/characters/bobot.js';
import * as marvin from '/characters/marvin.js';
import * as petitrobot from '/characters/petitrobot.js';
import * as chat from '/characters/chat.js';

const J = (window.JARVIS = window.JARVIS || {});
const CHARACTERS = { bobot, marvin, petitrobot, chat };
const CLASSIC_SCAN = { spin: 7, dir: 1, hue: 0x38e1ff, quirk: 'explode' };

const STATE_COLOR = { ok: 0x35ff9e, warn: 0xffc440, error: 0xff4d5e };
const TOP = 2.62;            // départ du faisceau (au-dessus de la tête)
const BOTTOM = 0.0;          // arrivée (au sol)
const SWEEP = 4.2;           // durée d'un balayage du bilan, en secondes
const SWEEP_PAUSE = 0.9;
const SETTLE = 0.7;          // le personnage finit son geste avant de s'immobiliser
const VIEW_Y = 1.22;
const VIEW_SPAN = 3.25;
const RING_Y = [0.03, 0.64, 1.25, 1.86, 2.5];

const ease = (x) => (x < 0.5 ? 4 * x * x * x : 1 - Math.pow(-2 * x + 2, 3) / 2);
const clamp = (v, a, b) => Math.max(a, Math.min(b, v));

function radialTexture(stops) {
  const c = document.createElement('canvas');
  c.width = c.height = 128;
  const g = c.getContext('2d');
  const grd = g.createRadialGradient(64, 64, 0, 64, 64, 64);
  for (const [at, col] of stops) grd.addColorStop(at, col);
  g.fillStyle = grd;
  g.fillRect(0, 0, 128, 128);
  const tex = new THREE.CanvasTexture(c);
  tex.colorSpace = THREE.SRGBColorSpace;
  return tex;
}

// Matière hologramme : bords lumineux (fresnel), fines lignes de balayage, bande éclatante au passage du
// faisceau ; tout ce qui est sous uCut est invisible (révélation de haut en bas pendant la rotation).
function holoMaterial(color) {
  return new THREE.ShaderMaterial({
    uniforms: {
      uColor: { value: new THREE.Color(color) }, uTime: { value: 0 }, uBeam: { value: 10 }, uCut: { value: -10 }, uPower: { value: 1 }
    },
    vertexShader: `
      varying vec3 vN; varying vec3 vV; varying float vY;
      void main() {
        vec4 wp = modelMatrix * vec4(position, 1.0);
        vY = wp.y;
        vN = normalize(mat3(modelMatrix) * normal);
        vV = normalize(cameraPosition - wp.xyz);
        gl_Position = projectionMatrix * viewMatrix * wp;
      }`,
    fragmentShader: `
      uniform vec3 uColor; uniform float uTime; uniform float uBeam; uniform float uCut; uniform float uPower;
      varying vec3 vN; varying vec3 vV; varying float vY;
      void main() {
        if (vY < uCut) discard;
        float f = pow(1.0 - abs(dot(normalize(vN), normalize(vV))), 2.2);
        float lines = 0.5 + 0.5 * sin(vY * 240.0 - uTime * 5.0);
        float band = exp(-pow((vY - uBeam) / 0.035, 2.0));
        float edge = uCut > -5.0 ? exp(-pow((vY - uCut) / 0.025, 2.0)) : 0.0;
        float a = (0.06 + f * 0.8 + lines * 0.05) * uPower + band * 0.8 + edge * 1.1;
        vec3 c = uColor * (0.5 + f * 0.9) + vec3(1.0) * (band * 0.55 + edge * 0.7);
        gl_FragColor = vec4(c * a, a);
      }`,
    transparent: true, depthWrite: false, blending: THREE.AdditiveBlending
  });
}

// Boîte des maillages portés par une articulation (sans les articulations qui en dépendent).
function ownBox(joint, jointSet) {
  const box = new THREE.Box3();
  const visit = (o) => {
    if (o !== joint && jointSet.has(o)) return;
    if (o.isMesh && o.visible && o.geometry) {
      o.geometry.computeBoundingBox();
      const b = o.geometry.boundingBox.clone().applyMatrix4(o.matrixWorld);
      box.union(b);
    }
    for (const c of o.children) visit(c);
  };
  visit(joint);
  return box;
}

const wpos = (o) => o.getWorldPosition(new THREE.Vector3());

// --- Cœur holographique classique : noyau, couches filaires, anneaux d'orbite ---------------------------
function buildClassic(mats, hue) {
  const root = new THREE.Group();
  const center = new THREE.Vector3(0, 1.25, 0);
  const core = new THREE.Mesh(new THREE.SphereGeometry(0.26, 48, 32), mats.holo);
  core.position.copy(center);
  root.add(core);
  const layers = [];
  [0.44, 0.6, 0.76].forEach((r, i) => {
    const g = new THREE.Group();
    g.position.copy(center);
    const shell = new THREE.Mesh(new THREE.IcosahedronGeometry(r, 2 + i), mats.holo);
    const wire = new THREE.LineSegments(new THREE.WireframeGeometry(new THREE.IcosahedronGeometry(r, 1 + i)),
      new THREE.LineBasicMaterial({ color: hue, transparent: true, opacity: 0.32 - i * 0.06, blending: THREE.AdditiveBlending, depthWrite: false, clippingPlanes: mats.wire.clippingPlanes }));
    g.add(shell, wire);
    const orbit = new THREE.Mesh(new THREE.TorusGeometry(r + 0.04, 0.005, 6, 96), new THREE.MeshBasicMaterial({ color: hue, transparent: true, opacity: 0.6, blending: THREE.AdditiveBlending, depthWrite: false, clippingPlanes: mats.wire.clippingPlanes }));
    orbit.rotation.x = Math.PI / 2 + (i - 1) * 0.5;
    orbit.rotation.y = i * 0.7;
    g.add(orbit);
    root.add(g);
    layers.push(g);
  });
  const c = center;
  const organs = {
    memory: [[c.x, c.y + 0.68, c.z]],
    brain: [[c.x, c.y + 0.5, c.z]],
    eyes: [[c.x, c.y + 0.32, c.z + 0.66]],
    ears: [[c.x - 0.7, c.y + 0.25, c.z], [c.x + 0.7, c.y + 0.25, c.z]],
    mouth: [[c.x, c.y + 0.1, c.z + 0.74]],
    heart: [[c.x, c.y, c.z]],
    spine: [[c.x, c.y - 0.05, c.z - 0.6]],
    armR: [[c.x - 0.78, c.y - 0.15, c.z + 0.1]],
    armL: [[c.x + 0.78, c.y - 0.15, c.z + 0.1]],
    legs: [[c.x - 0.3, c.y - 0.68, c.z], [c.x + 0.3, c.y - 0.68, c.z]],
    nerves: []
  };
  return {
    root,
    layers,
    organs: () => organs,
    // Vue éclatée pendant la rotation : les couches s'écartent et tournent chacune à leur rythme.
    animate(dt, t, spinK) {
      layers.forEach((g, i) => {
        const k = 1 + 0.32 * Math.sin(Math.PI * clamp(spinK, 0, 1)) * (i + 1) / 3;
        g.scale.setScalar(k);
        g.rotation.y += dt * (0.25 + i * 0.18) * (i % 2 ? -1 : 1);
        g.rotation.x = Math.sin(t * 0.3 + i) * 0.15;
      });
      core.scale.setScalar(1 + 0.06 * Math.sin(t * 2.4));
    },
    skeleton: null,
    dispose() {}
  };
}

// --- Personnage : hologramme, squelette, organes ---------------------------------------------------------
function buildCharacter(def, mats) {
  const props = createProps(THREE);
  const ch = def.create(THREE, { props });
  if (props.root.parent) props.root.parent.remove(props.root); // pas d'accessoires sous le scanner
  const joints = ch.joints;
  const jointList = [joints.root, joints.hips, joints.torso, joints.neck, joints.head,
    ...['r', 'l'].flatMap((s) => [joints.arms[s].shoulder, joints.arms[s].elbow, joints.arms[s].wrist, joints.legs[s].hip, joints.legs[s].knee, joints.legs[s].foot])];
  const jointSet = new Set(jointList);
  const meshes = [];
  ch.root.traverse((o) => {
    if (o.isMesh) meshes.push(o);
    else if (o.isPoints || o.isLine || o.isSprite) o.visible = false;
  });
  for (const m of meshes) {
    m.material = mats.holo;
    const w = new THREE.Mesh(m.geometry, mats.wire);
    w.raycast = () => {};
    m.add(w);
  }

  // Squelette : os (lignes) et articulations (points), dessinés par-dessus l'hologramme.
  const A = joints.arms, L = joints.legs;
  const headTop = new THREE.Object3D();
  joints.head.add(headTop);
  const bones = [
    [joints.neck, joints.head], [joints.head, headTop],
    [joints.neck, A.r.shoulder], [A.r.shoulder, A.r.elbow], [A.r.elbow, A.r.wrist],
    [joints.neck, A.l.shoulder], [A.l.shoulder, A.l.elbow], [A.l.elbow, A.l.wrist],
    [joints.hips, L.r.hip], [L.r.hip, L.r.knee], [L.r.knee, L.r.foot],
    [joints.hips, L.l.hip], [L.l.hip, L.l.knee], [L.l.knee, L.l.foot]
  ];
  const spineBones = [[joints.hips, joints.neck]];
  const pts = [joints.hips, joints.neck, joints.head, headTop, A.r.shoulder, A.r.elbow, A.r.wrist, A.l.shoulder, A.l.elbow, A.l.wrist, L.r.hip, L.r.knee, L.r.foot, L.l.hip, L.l.knee, L.l.foot];
  const lineMat = (opacity) => new THREE.LineBasicMaterial({ color: 0xbff4ff, transparent: true, opacity, blending: THREE.AdditiveBlending, depthTest: false, depthWrite: false, clippingPlanes: mats.wire.clippingPlanes });
  const boneGeo = new THREE.BufferGeometry();
  boneGeo.setAttribute('position', new THREE.BufferAttribute(new Float32Array(bones.length * 6), 3));
  const spineGeo = new THREE.BufferGeometry();
  spineGeo.setAttribute('position', new THREE.BufferAttribute(new Float32Array(6), 3));
  const ptGeo = new THREE.BufferGeometry();
  ptGeo.setAttribute('position', new THREE.BufferAttribute(new Float32Array(pts.length * 3), 3));
  const boneLines = new THREE.LineSegments(boneGeo, lineMat(0.7));
  const spineLine = new THREE.LineSegments(spineGeo, lineMat(0.9));
  const ptMat = new THREE.PointsMaterial({ color: 0xffffff, size: 7, sizeAttenuation: false, map: mats.dot, transparent: true, blending: THREE.AdditiveBlending, depthTest: false, depthWrite: false, clippingPlanes: mats.wire.clippingPlanes });
  const points = new THREE.Points(ptGeo, ptMat);
  for (const o of [boneLines, spineLine, points]) {
    o.frustumCulled = false;
    o.renderOrder = 5;
  }
  const v = new THREE.Vector3();
  const skeleton = {
    group: [boneLines, spineLine, points],
    bonesMat: boneLines.material, spineMat: spineLine.material, pointsMat: ptMat,
    // Positions dans le repère du plateau tournant (parent commun).
    sync(parent) {
      const inv = parent.matrixWorld.clone().invert();
      const put = (arr, i, o) => {
        o.getWorldPosition(v).applyMatrix4(inv);
        arr[i] = v.x; arr[i + 1] = v.y; arr[i + 2] = v.z;
      };
      const b = boneGeo.attributes.position.array;
      bones.forEach(([p, q], i) => { put(b, i * 6, p); put(b, i * 6 + 3, q); });
      boneGeo.attributes.position.needsUpdate = true;
      const s = spineGeo.attributes.position.array;
      put(s, 0, spineBones[0][0]);
      put(s, 3, spineBones[0][1]);
      spineGeo.attributes.position.needsUpdate = true;
      const pp = ptGeo.attributes.position.array;
      pts.forEach((o, i) => put(pp, i * 3, o));
      ptGeo.attributes.position.needsUpdate = true;
    },
    dispose() {
      boneGeo.dispose(); spineGeo.dispose(); ptGeo.dispose();
      boneLines.material.dispose(); spineLine.material.dispose(); ptMat.dispose();
    }
  };

  return {
    root: ch.root,
    joints,
    skeleton,
    animate(dt, t, action) {
      ch.update(dt, t, { action, level: 0, speaking: false, music: 0, power: 1 });
    },
    // Organes, placés une fois le personnage immobile (repère du plateau, face caméra).
    organs(parent) {
      parent.updateMatrixWorld(true);
      const inv = parent.matrixWorld.clone().invert();
      const local = (p) => p.clone().applyMatrix4(inv);
      const hb = ownBox(joints.head, jointSet);
      const hc = hb.isEmpty() ? wpos(joints.head) : hb.getCenter(new THREE.Vector3());
      const hs = hb.isEmpty() ? new THREE.Vector3(0.5, 0.5, 0.5) : hb.getSize(new THREE.Vector3());
      const tb = ownBox(joints.torso, jointSet);
      const tc = tb.isEmpty() ? wpos(joints.neck).lerp(wpos(joints.hips), 0.5) : tb.getCenter(new THREE.Vector3());
      const ts = tb.isEmpty() ? new THREE.Vector3(0.4, 0.4, 0.3) : tb.getSize(new THREE.Vector3());
      const at = (x, y, z) => local(new THREE.Vector3(x, y, z)).toArray();
      const elbow = (s) => local(wpos(A[s].elbow)).toArray();
      const knee = (s) => local(wpos(L[s].knee)).toArray();
      return {
        memory: [at(hc.x, hc.y + hs.y * 0.34, hc.z)],
        brain: [at(hc.x, hc.y + hs.y * 0.12, hc.z)],
        eyes: [at(hc.x, hc.y + hs.y * 0.02, hb.isEmpty() ? hc.z + 0.3 : hb.max.z - 0.02)],
        ears: [at(hc.x - hs.x * 0.47, hc.y, hc.z), at(hc.x + hs.x * 0.47, hc.y, hc.z)],
        mouth: [at(hc.x, hc.y - hs.y * 0.26, hb.isEmpty() ? hc.z + 0.3 : hb.max.z - 0.04)],
        heart: [at(tc.x + ts.x * 0.12, tc.y + ts.y * 0.16, tc.z + ts.z * 0.3)],
        spine: [at(tc.x, tc.y, tb.isEmpty() ? tc.z - 0.15 : tb.min.z + 0.04)],
        armR: [elbow('r')],
        armL: [elbow('l')],
        legs: [knee('r'), knee('l')],
        nerves: []
      };
    },
    dispose() {
      skeleton.dispose();
      ch.dispose();
      props.dispose();
    }
  };
}

// --- Scène du scanner -------------------------------------------------------------------------------------
function createScan(canvas, opts) {
  const id = opts.heartStyle;
  const def = CHARACTERS[id] || null;
  const sig = (def && def.scan) || CLASSIC_SCAN;
  const hue = new THREE.Color(sig.hue || 0x38e1ff);

  const renderer = new THREE.WebGLRenderer({ canvas, alpha: true, antialias: true, premultipliedAlpha: true, powerPreference: 'low-power' });
  renderer.setClearColor(0x000000, 0);
  renderer.localClippingEnabled = true;
  const scene = new THREE.Scene();
  const camera = new THREE.PerspectiveCamera(26, 1, 0.1, 50);
  const dist = (VIEW_SPAN / 2) / Math.tan(THREE.MathUtils.degToRad(13));
  camera.position.set(0, VIEW_Y + 0.75, dist);
  camera.lookAt(0, VIEW_Y, 0);

  const disposables = [];
  const keep = (x) => { disposables.push(x); return x; };
  const clip = new THREE.Plane(new THREE.Vector3(0, 1, 0), 10);
  const mats = {
    holo: keep(holoMaterial(hue)),
    wire: keep(new THREE.MeshBasicMaterial({ color: hue, wireframe: true, transparent: true, opacity: 0.06, blending: THREE.AdditiveBlending, depthWrite: false, clippingPlanes: [clip] })),
    dot: keep(radialTexture([[0, 'rgba(255,255,255,1)'], [0.35, 'rgba(255,255,255,0.8)'], [1, 'rgba(255,255,255,0)']])),
    halo: keep(radialTexture([[0, 'rgba(255,255,255,0.95)'], [0.25, 'rgba(255,255,255,0.45)'], [1, 'rgba(255,255,255,0)']])),
    beam: keep(radialTexture([[0, 'rgba(255,255,255,0.05)'], [0.75, 'rgba(255,255,255,0.22)'], [0.93, 'rgba(255,255,255,0.9)'], [1, 'rgba(255,255,255,0)']]))
  };
  const add = (geo, mat) => new THREE.Mesh(keep(geo), keep(mat));
  const glow = (color, opacity, extra = {}) => new THREE.MeshBasicMaterial({ color, transparent: true, opacity, blending: THREE.AdditiveBlending, depthWrite: false, side: THREE.DoubleSide, ...extra });

  // Plateau tournant : le personnage (ou le cœur), son squelette et ses organes.
  const spinner = new THREE.Group();
  scene.add(spinner);
  const body = def ? buildCharacter(def, mats) : buildClassic(mats, hue);
  spinner.add(body.root);
  if (body.skeleton) for (const o of body.skeleton.group) spinner.add(o);

  // Anneaux empilés (comme une cabine de scan) et socle.
  const rings = RING_Y.map((y, i) => {
    const end = i === 0 || i === RING_Y.length - 1;
    const r = add(new THREE.TorusGeometry(end ? 0.86 : 0.78, end ? 0.011 : 0.006, 8, 128), glow(hue, end ? 0.85 : 0.32));
    r.rotation.x = Math.PI / 2;
    r.position.y = y;
    r.userData.base = end ? 0.85 : 0.32;
    scene.add(r);
    return r;
  });
  const disc = add(new THREE.CircleGeometry(0.86, 64), glow(hue, 0.08));
  disc.rotation.x = -Math.PI / 2;
  disc.position.y = 0.01;
  scene.add(disc);
  const dashes = new THREE.Group();
  for (let i = 0; i < 16; i++) {
    const d = add(new THREE.RingGeometry(0.92, 0.96, 8, 1, (i / 16) * Math.PI * 2, 0.24), glow(hue, 0.55));
    d.rotation.x = -Math.PI / 2;
    dashes.add(d);
  }
  dashes.position.y = 0.012;
  scene.add(dashes);

  // Faisceau : disque lumineux horizontal et son anneau.
  const beam = new THREE.Group();
  const beamDisc = add(new THREE.CircleGeometry(0.95, 64), glow(0xffffff, 0.55, { map: mats.beam }));
  beamDisc.rotation.x = -Math.PI / 2;
  beamDisc.material.color.copy(hue).lerp(new THREE.Color(0xffffff), 0.35);
  const beamRing = add(new THREE.TorusGeometry(0.95, 0.008, 8, 128), glow(0xffffff, 0.9));
  beamRing.rotation.x = Math.PI / 2;
  beam.add(beamDisc, beamRing);
  scene.add(beam);

  // Signature Bobot : trait de visière qui balaie le visage pendant la rotation.
  let visor = null;
  if (sig.quirk === 'visor' && body.joints) {
    visor = add(new THREE.PlaneGeometry(0.62, 0.012), glow(0xffffff, 0));
    visor.position.set(0, 0.38, 0.45);
    body.joints.head.add(visor);
  }

  // Organes : cœur lumineux + halo, un par emplacement (deux oreilles, deux genoux).
  const organGroup = new THREE.Group();
  spinner.add(organGroup);
  let organs = [];
  let vitals = {};
  const setVitals = (list) => {
    vitals = {};
    for (const v of list || []) vitals[v.id] = v;
  };
  setVitals(opts.vitals);
  function placeOrgans() {
    for (const o of organs) organGroup.remove(o.group);
    organs = [];
    const spots = body.organs(spinner);
    for (const [vid, places] of Object.entries(spots)) {
      places.forEach((p) => {
        const big = vid === 'brain' && sig.quirk === 'brain' ? sig.brainScale || 1.6 : 1;
        const g = new THREE.Group();
        g.position.fromArray(p);
        const core = add(new THREE.SphereGeometry(0.036 * big, 16, 12), glow(0xffffff, 0, { depthTest: false }));
        const halo = new THREE.Sprite(keep(new THREE.SpriteMaterial({ map: mats.halo, color: 0xffffff, transparent: true, opacity: 0, blending: THREE.AdditiveBlending, depthTest: false, depthWrite: false })));
        halo.scale.setScalar(0.34 * big);
        core.renderOrder = halo.renderOrder = 8;
        g.add(core, halo);
        organGroup.add(g);
        organs.push({ id: vid, group: g, core, halo, y: p[1], lit: 0, flash: 0, pos: new THREE.Vector3().fromArray(p) });
      });
    }
  }

  const st = {
    phase: 'spin', t0: performance.now() / 1000, last: performance.now() / 1000, frozenAt: 0, sweepAt: 0,
    revealed: new Set(), lastFrameCb: 0, raf: 0, lostAt: 0, disposed: false
  };
  const colorOf = (vid) => new THREE.Color(STATE_COLOR[(vitals[vid] && vitals[vid].state) || 'ok'] || STATE_COLOR.ok);

  function resize() {
    const w = Math.max(64, canvas.clientWidth || 300);
    const h = Math.max(64, canvas.clientHeight || 400);
    const px = Math.min(2, window.devicePixelRatio || 1);
    if (canvas.width !== Math.round(w * px) || canvas.height !== Math.round(h * px)) {
      renderer.setPixelRatio(px);
      renderer.setSize(w, h, false);
      camera.aspect = w / h;
      camera.updateProjectionMatrix();
    }
  }

  const tmpV = new THREE.Vector3();
  function project(v3) {
    tmpV.copy(v3).applyMatrix4(spinner.matrixWorld).project(camera);
    return [(tmpV.x * 0.5 + 0.5) * (canvas.clientWidth || 1), (-tmpV.y * 0.5 + 0.5) * (canvas.clientHeight || 1)];
  }

  function frame() {
    if (st.disposed) return;
    st.raf = requestAnimationFrame(frame);
    if (!canvas.isConnected) {
      if (!st.lostAt) st.lostAt = performance.now();
      else if (performance.now() - st.lostAt > 1500) api.dispose();
      return;
    }
    st.lostAt = 0;
    resize();
    const now = performance.now() / 1000;
    const dt = Math.min(0.1, now - st.last);
    st.last = now;
    const t = now - st.t0;
    mats.holo.uniforms.uTime.value = now;
    dashes.rotation.y = now * 0.3;

    let beamY;
    let scanY = null; // hauteur du faisceau pour l'allumage des organes (bilan seulement)
    let spinK = 1;
    if (st.phase === 'spin') {
      spinK = clamp(t / sig.spin, 0, 1);
      const e = ease(spinK);
      spinner.rotation.y = (sig.dir || 1) * Math.PI * 2 * e;
      beamY = TOP + (BOTTOM - TOP) * spinK;
      const cut = beamY - 0.02;
      mats.holo.uniforms.uCut.value = cut;
      clip.constant = -cut;
      // Signatures : petits sauts (Petit Robot), soupir (Chat), visière (Bobot).
      if (sig.quirk === 'hop') spinner.position.y = spinK < 0.3 ? 0.16 * Math.abs(Math.sin(spinK / 0.3 * Math.PI * 2)) : 0;
      if (visor) {
        visor.material.opacity = 0.9 * Math.sin(Math.PI * spinK);
        visor.position.y = 0.38 + 0.12 * Math.sin(t * 5);
      }
      if (def) body.animate(dt, t, 'idle');
      else body.animate(dt, now, spinK);
      if (spinK >= 1) {
        st.phase = 'settle';
        st.frozenAt = now;
        mats.holo.uniforms.uCut.value = -10;
        clip.constant = 10;
        if (visor) visor.material.opacity = 0;
      }
    } else {
      spinner.rotation.y = 0;
      spinner.position.y = 0;
      if (st.phase === 'settle') {
        const k = clamp((now - st.frozenAt) / SETTLE, 0, 1);
        if (def) body.animate(dt, t, 'idle');
        else body.animate(dt, now, 1 - k);
        if (sig.quirk === 'sigh') spinner.scale.set(1, 1 - 0.035 * Math.sin(Math.PI * k), 1);
        beamY = 10;
        if (k >= 1) {
          spinner.scale.set(1, 1, 1);
          st.phase = 'analyse';
          st.sweepAt = now;
          spinner.updateMatrixWorld(true);
          placeOrgans();
          if (sig.quirk === 'brain') for (const o of organs) if (o.id === 'brain') { o.lit = 1; o.flash = 1; }
          emit('analyse');
        }
      } else {
        // Bilan : personnage immobile, le faisceau balaie de haut en bas en boucle.
        const cycle = SWEEP + SWEEP_PAUSE;
        const k = ((now - st.sweepAt) % cycle) / SWEEP;
        beamY = k <= 1 ? TOP + (BOTTOM - TOP) * k : 10;
        scanY = k <= 1 ? beamY : -1; // balayage fini : tout ce qui restait au-dessus du sol est franchi
        if (!def) body.animate(dt, now, 0);
      }
    }
    if (body.skeleton) {
      spinner.updateMatrixWorld(true);
      if (st.phase !== 'analyse' || !st.skelFrozen) {
        body.skeleton.sync(spinner);
        if (st.phase === 'analyse') st.skelFrozen = true;
      }
    }

    beam.position.y = beamY;
    beam.visible = beamY < 5;
    // Nouveau balayage (ou début du bilan) : il part du haut, rien n'est encore franchi.
    if (scanY === null || st.prevScan === undefined || scanY > st.prevScan) st.prevScan = TOP + 0.05;
    mats.holo.uniforms.uBeam.value = beamY;
    for (const r of rings) {
      const near = beam.visible ? Math.exp(-Math.pow((r.position.y - beamY) / 0.08, 2)) : 0;
      r.material.opacity = r.userData.base + near * 0.6;
      r.scale.setScalar(1 + near * 0.04);
    }

    // Organes : allumés au passage du faisceau, à la couleur de leur état.
    for (const o of organs) {
      // Franchi par le faisceau depuis l'image précédente (robuste aux images espacées).
      const passing = scanY !== null && o.y <= st.prevScan && o.y > scanY;
      if (passing) {
        if (!o.lit) {
          o.lit = 1;
          if (!st.revealed.has(o.id)) {
            st.revealed.add(o.id);
            emit('reveal', o.id);
          }
        }
        o.flash = 1;
      }
      o.flash = Math.max(0, o.flash - dt * 1.6);
      const state = (vitals[o.id] && vitals[o.id].state) || 'ok';
      const pulse = state === 'error' ? (Math.sin(now * 9) > 0 ? 1 : 0.25) : state === 'warn' ? 0.45 + 0.55 * Math.pow(Math.sin(now * 2.6), 2) : 0.75 + 0.25 * Math.sin(now * 1.8);
      const a = o.lit * pulse;
      const col = colorOf(o.id);
      o.core.material.color.copy(col).lerp(new THREE.Color(0xffffff), 0.35 * o.flash);
      o.core.material.opacity = clamp(a + o.flash * 0.5, 0, 1);
      o.halo.material.color.copy(col);
      o.halo.material.opacity = clamp(a * 0.85 + o.flash * 0.6, 0, 1);
      o.halo.scale.setScalar((o.id === 'brain' && sig.quirk === 'brain' ? sig.brainScale || 1.6 : 1) * (0.34 + 0.16 * o.flash));
    }
    // Système nerveux et colonne : couleur du squelette selon leur état, une fois analysés.
    if (body.skeleton) {
      const nervesOn = st.revealed.has('nerves') || (st.phase === 'analyse' && beamY < 1.0);
      const sk = body.skeleton;
      const base = new THREE.Color(0xbff4ff);
      if (st.phase === 'analyse' && nervesOn) {
        const ns = (vitals.nerves && vitals.nerves.state) || 'ok';
        sk.bonesMat.color.copy(ns === 'ok' ? base : colorOf('nerves'));
        sk.bonesMat.opacity = ns === 'error' ? 0.45 + 0.4 * (Math.sin(now * 7) > 0 ? 1 : 0) : 0.7;
        sk.pointsMat.color.copy(ns === 'ok' ? new THREE.Color(0xffffff) : colorOf('nerves'));
        if (!st.revealed.has('nerves')) { st.revealed.add('nerves'); emit('reveal', 'nerves'); }
      } else {
        sk.bonesMat.color.copy(base);
        sk.pointsMat.color.set(0xffffff);
      }
      sk.spineMat.color.copy(st.revealed.has('spine') ? colorOf('spine') : base);
    } else if (st.phase === 'analyse' && !st.revealed.has('nerves') && beamY < 0.3) {
      st.revealed.add('nerves');
      emit('reveal', 'nerves');
    }

    if (scanY !== null) st.prevScan = scanY;
    renderer.render(scene, camera);

    // Positions des organes à l'écran (traits des étiquettes de la card), 10 fois par seconde.
    if (opts.onFrame && now - st.lastFrameCb > 0.1) {
      st.lastFrameCb = now;
      const points = {};
      for (const o of organs) if (!points[o.id]) points[o.id] = project(o.pos);
      opts.onFrame({ phase: st.phase, progress: spinK, beamY, points, lit: [...st.revealed] });
    }
  }

  function emit(kind, vid) {
    if (opts.onPhase) {
      try { opts.onPhase(kind, vid); } catch {}
    }
  }

  const api = {
    update(list) { setVitals(list); },
    // Diagnostic (console) : organes placés et leur hauteur.
    organs: () => organs.map((o) => [o.id, +o.y.toFixed(2)]),
    rescan() {
      st.phase = 'spin';
      st.t0 = performance.now() / 1000;
      st.revealed.clear();
      st.skelFrozen = false;
      for (const o of organs) organGroup.remove(o.group);
      organs = [];
      emit('spin');
    },
    dispose() {
      if (st.disposed) return;
      st.disposed = true;
      cancelAnimationFrame(st.raf);
      try {
        body.dispose();
        scene.traverse((o) => { if (o.geometry) o.geometry.dispose(); });
        for (const d of disposables) if (d && d.dispose) d.dispose();
        renderer.dispose();
        renderer.forceContextLoss();
      } catch {
        // rien de plus à libérer
      }
    }
  };
  emit('spin');
  st.raf = requestAnimationFrame(frame);
  return api;
}

J.checkupScan = {
  // Monte le scanner dans un canvas. opts : { heartStyle, vitals, onPhase(kind, id), onFrame(info) }.
  mount(canvas, opts) {
    return createScan(canvas, opts || {});
  }
};
