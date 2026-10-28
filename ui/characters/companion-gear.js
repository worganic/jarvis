// Compagnon de bureau (tamagotchi) : ce qui s'ajoute au personnage du cœur selon son stade et sa branche.
//   œuf     : le personnage est caché, une coque teintée à la couleur de Jarvis qui se fissure en chauffant ;
//   bébé    : le personnage à 60 % de sa taille, sans accessoire ;
//   ado     : 82 %, un accessoire de sa branche ;
//   adulte  : taille réelle, accessoires complets et aura.
// Branches : gamer (casque, lueur RGB), dev (lunettes, tasse), artiste (béret, taches de peinture),
// chill (plaid, pop-corn). Effets (Zzz, sueur, notes, étincelles) : accessoires communs de props.js,
// demandés par le crochet `extra` du squelette (rig.js), donc par-dessus le geste joué.
// Les accessoires se placent d'après la boîte de la tête et des poignets : ils vont aux 4 personnages.

const STAGE_SCALE = { oeuf: 1, bebe: 0.6, ado: 0.82, adulte: 1 };

export function createGear(THREE, character, meta = {}) {
  const j = character.joints || {};
  const disposables = [];
  const keep = (x) => { disposables.push(x); return x; };
  const std = (color, opts = {}) => keep(new THREE.MeshStandardMaterial({ color, roughness: 0.45, metalness: 0.1, ...opts }));
  const glow = (color, opacity = 0.7) => keep(new THREE.MeshBasicMaterial({ color, transparent: true, opacity, blending: THREE.AdditiveBlending, depthWrite: false, toneMapped: false, side: THREE.DoubleSide }));
  const mesh = (geo, mat, parent, x = 0, y = 0, z = 0) => {
    const m = new THREE.Mesh(keep(geo), mat);
    m.position.set(x, y, z);
    parent.add(m);
    return m;
  };

  // Repères de la tête : centre et rayon, mesurés une fois dans la pose de repos.
  const headBox = new THREE.Box3();
  const headPos = new THREE.Vector3();
  let hc = new THREE.Vector3(0, 0.35, 0);
  let hr = 0.4;
  if (j.head) {
    character.root.updateMatrixWorld(true);
    headBox.setFromObject(j.head);
    j.head.getWorldPosition(headPos);
    if (!headBox.isEmpty()) {
      const c = headBox.getCenter(new THREE.Vector3());
      const size = headBox.getSize(new THREE.Vector3());
      hc = c.sub(headPos);
      hr = Math.max(0.2, Math.min(0.7, Math.max(size.x, size.y) / 2));
    }
  }
  const height = meta.height || 2.4;

  // --- Accessoires de tête et de mains ------------------------------------------------------------
  const onHead = new THREE.Group();
  if (j.head) j.head.add(onHead);
  onHead.position.copy(hc);
  const wristR = j.arms && j.arms.r ? j.arms.r.wrist : null;
  const wristL = j.arms && j.arms.l ? j.arms.l.wrist : null;
  const handR = new THREE.Group();
  const handL = new THREE.Group();
  if (wristR) wristR.add(handR);
  if (wristL) wristL.add(handL);
  const onBody = new THREE.Group();
  if (j.torso) j.torso.add(onBody);

  const parts = {};
  const part = (name, parent, build) => {
    const g = new THREE.Group();
    g.visible = false;
    parent.add(g);
    build(g);
    parts[name] = g;
  };

  part('casque', onHead, (g) => {
    const dark = std(0x1b2230, { metalness: 0.4 });
    mesh(new THREE.TorusGeometry(hr * 1.04, hr * 0.07, 8, 32, Math.PI), dark, g);
    for (const s of [-1, 1]) {
      const cup = mesh(new THREE.CylinderGeometry(hr * 0.32, hr * 0.32, hr * 0.22, 20), dark, g, s * hr * 1.04, 0, 0);
      cup.rotation.z = Math.PI / 2;
      mesh(new THREE.TorusGeometry(hr * 0.26, hr * 0.04, 6, 20), glow(0xff3df0, 0.9), cup, 0, s * hr * 0.12, 0).rotation.x = Math.PI / 2;
    }
  });
  part('rgb', onBody, (g) => {
    const ring = mesh(new THREE.TorusGeometry(0.5, 0.025, 6, 48), glow(0xff3df0, 0.75), g, 0, -0.1, 0);
    ring.rotation.x = Math.PI / 2;
    g.userData.ring = ring;
  });
  part('lunettes', onHead, (g) => {
    const frame = std(0x111111, { metalness: 0.6, roughness: 0.25 });
    const lens = keep(new THREE.MeshPhysicalMaterial({ color: 0x9fd8ff, transparent: true, opacity: 0.35, roughness: 0.05, transmission: 0.5 }));
    for (const s of [-1, 1]) {
      mesh(new THREE.TorusGeometry(hr * 0.24, hr * 0.035, 8, 24), frame, g, s * hr * 0.3, -hr * 0.05, hr * 0.98);
      mesh(new THREE.CircleGeometry(hr * 0.23, 20), lens, g, s * hr * 0.3, -hr * 0.05, hr * 0.97);
    }
    mesh(new THREE.BoxGeometry(hr * 0.14, hr * 0.04, hr * 0.04), frame, g, 0, -hr * 0.02, hr * 1.0);
  });
  part('tasse', handR, (g) => {
    const cup = mesh(new THREE.CylinderGeometry(0.07, 0.06, 0.13, 18, 1, true), std(0xffffff), g, 0, -0.06, 0.06);
    mesh(new THREE.CircleGeometry(0.066, 18), std(0x4a2b16), cup, 0, 0.05, 0).rotation.x = -Math.PI / 2;
    mesh(new THREE.TorusGeometry(0.035, 0.012, 6, 14), std(0xffffff), cup, 0.075, 0, 0).rotation.y = Math.PI / 2;
  });
  part('beret', onHead, (g) => {
    const b = mesh(new THREE.SphereGeometry(hr * 0.78, 24, 12), std(0x7a1830, { roughness: 0.9 }), g, hr * 0.12, hr * 0.82, 0);
    b.scale.set(1, 0.32, 1);
    b.rotation.z = -0.25;
    mesh(new THREE.CylinderGeometry(hr * 0.04, hr * 0.04, hr * 0.12, 8), std(0x7a1830), g, hr * 0.1, hr * 1.02, 0);
  });
  part('peinture', onBody, (g) => {
    const colors = [0xff5050, 0x50c8ff, 0xffd84a, 0x7cff6a, 0xc06aff];
    for (let i = 0; i < 7; i++) {
      const a = (i / 7) * Math.PI * 1.6 - 0.8;
      const dot = mesh(new THREE.SphereGeometry(0.035 + (i % 3) * 0.01, 10, 8), std(colors[i % colors.length], { roughness: 0.3 }), g, Math.sin(a) * 0.3, 0.15 + (i % 4) * 0.1, Math.cos(a) * 0.3);
      dot.scale.z = 0.4;
    }
  });
  part('plaid', onBody, (g) => {
    const tex = keep(plaidTexture(THREE));
    const cape = mesh(new THREE.CylinderGeometry(0.42, 0.55, 0.6, 24, 1, true, Math.PI * 0.6, Math.PI * 1.8), keep(new THREE.MeshStandardMaterial({ map: tex, roughness: 0.95, side: THREE.DoubleSide })), g, 0, 0.32, 0);
    cape.rotation.y = Math.PI;
  });
  part('popcorn', handL, (g) => {
    const box = mesh(new THREE.CylinderGeometry(0.08, 0.06, 0.15, 12, 1, true), keep(new THREE.MeshStandardMaterial({ map: keep(stripeTexture(THREE)), side: THREE.DoubleSide })), g, 0, -0.06, 0.06);
    for (let i = 0; i < 6; i++) mesh(new THREE.SphereGeometry(0.03, 8, 6), std(0xfff3c4, { roughness: 0.9 }), box, Math.cos(i) * 0.04, 0.085 + (i % 2) * 0.02, Math.sin(i) * 0.04);
  });

  // --- Monde : œuf et aura (hors du squelette, qui est caché ou mis à l'échelle) ------------------
  const world = new THREE.Group();
  const eggMat = keep(new THREE.MeshPhysicalMaterial({ color: 0xeaf6ff, roughness: 0.25, clearcoat: 1, clearcoatRoughness: 0.1, emissive: 0x38e1ff, emissiveIntensity: 0.08 }));
  const egg = mesh(new THREE.SphereGeometry(0.5, 40, 28), eggMat, world, 0, 0.62, 0);
  egg.scale.set(0.82, 1.08, 0.82);
  const crackTex = keep(crackTexture(THREE));
  const crackMat = keep(new THREE.MeshBasicMaterial({ map: crackTex, transparent: true, opacity: 0, depthWrite: false, toneMapped: false }));
  mesh(new THREE.SphereGeometry(0.505, 40, 28), crackMat, egg);
  egg.visible = false;
  const aura = new THREE.Group();
  world.add(aura);
  const auraMat = glow(0x38e1ff, 0.55);
  for (let i = 0; i < 2; i++) {
    const r = mesh(new THREE.TorusGeometry(0.62 + i * 0.12, 0.012, 6, 64), auraMat, aura, 0, 0.35 + i * 0.9, 0);
    r.rotation.x = Math.PI / 2;
  }
  aura.visible = false;

  let state = { stage: 'adulte', accessories: [], effects: [], flash: null, warmth: 0, enabled: false };
  let scale = 1;
  let glitchUntil = 0;

  function set(c) {
    state = c && c.enabled ? c : { stage: 'adulte', accessories: [], effects: [], flash: null, warmth: 0, enabled: false };
    if (state.flash === 'epice' && glitchUntil < performance.now()) glitchUntil = performance.now() + 5000;
  }

  // Appelé par rig.js (k.show) à chaque image, après le geste : effets du moment.
  function extra(k, action) {
    if (!state.enabled) return;
    const fx = new Set(state.effects || []);
    const top = height * 0.95 * scale;
    if (action === 'sleep' || fx.has('asleep')) k.show('zzz', [0.35, top, 0]);
    if (fx.has('hot')) k.show('sweat', [0.28, top * 0.9, 0.2]);
    if (fx.has('music')) k.show('notes', [0, top, 0]);
    if (['hatch', 'evolve', 'croquettes', 'friandise', 'ronron'].includes(state.flash)) k.show('sparkles', [0, top * 0.7, 0]);
  }

  function update(dt, t, accent) {
    const on = state.enabled;
    const isEgg = on && state.stage === 'oeuf';
    const want = on ? STAGE_SCALE[state.stage] || 1 : 1;
    scale += (want - scale) * Math.min(1, dt * 2);
    character.root.scale.setScalar(scale);
    character.root.visible = !isEgg;
    egg.visible = isEgg;
    if (isEgg) {
      const warm = Math.max(0, Math.min(1, (state.warmth || 0) / 100));
      // L'œuf tangue de plus en plus en chauffant, les fissures apparaissent au-delà de 40 %.
      egg.rotation.z = Math.sin(t * (1.5 + warm * 4)) * (0.03 + warm * 0.12);
      egg.position.y = 0.62 + Math.abs(Math.sin(t * 2.2)) * warm * 0.04;
      crackMat.opacity = Math.max(0, (warm - 0.4) / 0.6);
      if (accent) eggMat.emissive.copy(accent);
      eggMat.emissiveIntensity = 0.06 + warm * 0.25;
    }
    const acc = new Set(on && !isEgg ? state.accessories || [] : []);
    for (const [name, g] of Object.entries(parts)) g.visible = acc.has(name);
    if (parts.rgb.visible) parts.rgb.userData.ring.material.color.setHSL((t * 0.15) % 1, 1, 0.55);
    aura.visible = acc.has('aura');
    if (aura.visible) {
      aura.rotation.y = t * 0.6;
      auraMat.opacity = 0.35 + 0.2 * Math.sin(t * 2);
      if (accent) auraMat.color.copy(accent);
    }
    // Plat épicé : petit bug rigolo pendant 5 s (le personnage tremble et clignote).
    const glitch = on && performance.now() < glitchUntil;
    if (glitch) {
      character.root.position.x += (Math.random() - 0.5) * 0.05;
      character.root.visible = !isEgg && Math.random() > 0.08;
    }
  }

  function dispose() {
    for (const d of disposables) if (d && d.dispose) d.dispose();
  }

  return { world, set, extra, update, dispose };
}

function canvasTex(THREE, w, h, draw) {
  const c = document.createElement('canvas');
  c.width = w;
  c.height = h;
  draw(c.getContext('2d'), w, h);
  const tex = new THREE.CanvasTexture(c);
  tex.colorSpace = THREE.SRGBColorSpace;
  return tex;
}

function plaidTexture(THREE) {
  return canvasTex(THREE, 128, 128, (g, w, h) => {
    g.fillStyle = '#7b2d26';
    g.fillRect(0, 0, w, h);
    g.globalAlpha = 0.55;
    g.fillStyle = '#1f3a5a';
    for (let i = 0; i < w; i += 32) { g.fillRect(i, 0, 12, h); g.fillRect(0, i, w, 12); }
    g.fillStyle = '#e6c27a';
    for (let i = 16; i < w; i += 32) { g.fillRect(i, 0, 3, h); g.fillRect(0, i, w, 3); }
  });
}

function stripeTexture(THREE) {
  return canvasTex(THREE, 64, 32, (g, w, h) => {
    for (let i = 0; i < 8; i++) {
      g.fillStyle = i % 2 ? '#ffffff' : '#e02b2b';
      g.fillRect((i * w) / 8, 0, w / 8, h);
    }
  });
}

function crackTexture(THREE) {
  return canvasTex(THREE, 256, 128, (g, w, h) => {
    g.clearRect(0, 0, w, h);
    g.strokeStyle = 'rgba(20,30,45,0.9)';
    g.lineWidth = 3;
    for (const x0 of [40, 120, 200]) {
      g.beginPath();
      let x = x0;
      let y = h * 0.3;
      g.moveTo(x, y);
      for (let i = 0; i < 6; i++) {
        x += (i % 2 ? 9 : -9);
        y += 9;
        g.lineTo(x, y);
      }
      g.stroke();
    }
  });
}
