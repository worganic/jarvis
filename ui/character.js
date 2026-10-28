// Personnage 3D au centre du cœur (réglage « Apparence du cœur » : administration › Interface, ou
// « affiche Marvin » / « cœur classique » à la voix). Rendu WebGL (three.js r170, ui/vendor) dans un
// canvas hors écran, recopié à chaque image dans le canvas 2D du HUD par J.drawOrb (orb.js) : anneaux,
// couronne vocale, cards et liaisons restent dessinés comme avant, seule la zone centrale change.
//
// L'animation jouée est choisie par ui/character-director.js ; le modèle et ses gestes sont dans
// ui/characters/<id>.js (un fichier par personnage), les accessoires communs dans ui/characters/props.js.
// Sans WebGL, ou si un personnage échoue, le cœur classique reprend sa place.
import * as THREE from './vendor/three.module.min.js';
import { studioEnvironment } from './three-studio.js';
import { createProps } from './characters/props.js';
import { createGear } from './characters/companion-gear.js';
import * as bobot from './characters/bobot.js';
import * as marvin from './characters/marvin.js';
import * as petitrobot from './characters/petitrobot.js';
import * as chat from './characters/chat.js';

const J = (window.JARVIS = window.JARVIS || {});
const CHARACTERS = { bobot, marvin, petitrobot, chat };

const MAX_PX = 720;          // côté maximal du rendu, en pixels
const RELEASE_MS = 60000;    // personnage non affiché depuis 1 min : mémoire graphique rendue
const VIEW_SPAN = 3.1;       // hauteur de scène visible (unités du personnage)
const VIEW_Y = 1.12;         // hauteur du centre de la vue (pieds à 0)
const HALF = 1.42;           // demi-côté du rendu, en rayons de la zone centrale du cœur

let stage = null;
const failed = new Set();    // personnages en échec (WebGL absent, erreur de construction)
let lastDrawAt = 0;

const rgb = (c) => new THREE.Color().setRGB(c[0] / 255, c[1] / 255, c[2] / 255, THREE.SRGBColorSpace);

function radialTexture(inner, outer) {
  const c = document.createElement('canvas');
  c.width = c.height = 128;
  const g = c.getContext('2d');
  const grd = g.createRadialGradient(64, 64, 0, 64, 64, 64);
  grd.addColorStop(0, inner);
  grd.addColorStop(1, outer);
  g.fillStyle = grd;
  g.fillRect(0, 0, 128, 128);
  const tex = new THREE.CanvasTexture(c);
  tex.colorSpace = THREE.SRGBColorSpace;
  return tex;
}

function createStage(id) {
  const def = CHARACTERS[id];
  const canvas = document.createElement('canvas');
  canvas.width = canvas.height = 256;
  const renderer = new THREE.WebGLRenderer({ canvas, alpha: true, antialias: true, premultipliedAlpha: true, powerPreference: 'low-power' });
  renderer.setClearColor(0x000000, 0);
  renderer.toneMapping = THREE.ACESFilmicToneMapping;
  renderer.toneMappingExposure = 1.05;
  renderer.outputColorSpace = THREE.SRGBColorSpace;

  const scene = new THREE.Scene();
  scene.environment = studioEnvironment(renderer);
  const camera = new THREE.PerspectiveCamera(26, 1, 0.1, 50);
  const dist = (VIEW_SPAN / 2) / Math.tan(THREE.MathUtils.degToRad(13));
  camera.position.set(0, VIEW_Y + 0.35, dist);
  camera.lookAt(0, VIEW_Y, 0);

  scene.add(new THREE.HemisphereLight(0xcfe6ff, 0x0a1222, 0.7));
  const key = new THREE.DirectionalLight(0xffffff, 2.1);
  key.position.set(2.2, 4, 5);
  scene.add(key);
  // Contre-jour à la couleur de l'état de Jarvis : le personnage se fond dans le HUD.
  const rim = new THREE.DirectionalLight(0x38e1ff, 2.4);
  rim.position.set(-3, 2.5, -4);
  scene.add(rim);
  const rim2 = new THREE.DirectionalLight(0x38e1ff, 1.2);
  rim2.position.set(3.5, 1, -3);
  scene.add(rim2);

  // Socle holographique : anneau, disque, tirets qui tournent, ombre de contact.
  const accent = new THREE.Color(0x38e1ff);
  const holo = (opacity) => new THREE.MeshBasicMaterial({ color: accent.clone(), transparent: true, opacity, blending: THREE.AdditiveBlending, depthWrite: false, side: THREE.DoubleSide, toneMapped: false });
  const base = new THREE.Group();
  const ringMat = holo(0.85);
  const discMat = holo(0.1);
  const dashMat = holo(0.6);
  const ring = new THREE.Mesh(new THREE.TorusGeometry(0.66, 0.012, 6, 96), ringMat);
  ring.rotation.x = Math.PI / 2;
  const disc = new THREE.Mesh(new THREE.CircleGeometry(0.66, 64), discMat);
  disc.rotation.x = -Math.PI / 2;
  const dashes = new THREE.Group();
  for (let i = 0; i < 12; i++) {
    const d = new THREE.Mesh(new THREE.RingGeometry(0.72, 0.76, 8, 1, (i / 12) * Math.PI * 2, 0.32), dashMat);
    d.rotation.x = -Math.PI / 2;
    dashes.add(d);
  }
  const shadow = new THREE.Mesh(new THREE.PlaneGeometry(1.3, 1.3), new THREE.MeshBasicMaterial({ map: radialTexture('rgba(0,0,0,0.55)', 'rgba(0,0,0,0)'), transparent: true, depthWrite: false }));
  shadow.rotation.x = -Math.PI / 2;
  shadow.position.y = 0.004;
  ring.position.y = disc.position.y = dashes.position.y = 0.006;
  base.add(disc, shadow, ring, dashes);
  scene.add(base);

  const props = createProps(THREE);
  const character = def.create(THREE, { props });
  scene.add(character.root);
  // Compagnon de bureau (option) : œuf, taille selon le stade, accessoires de la branche, aura.
  const gear = createGear(THREE, character, def.meta);
  scene.add(gear.world);

  const s = {
    id, canvas, renderer, scene, camera, character, props, gear, accent, eyes: rgb(def.meta.restColor || [56, 225, 255]), eyesTo: new THREE.Color(), rest: def.meta.restColor || null, rim, rim2, base, dashes,
    tint: [ringMat, discMat, dashMat], px: 0, lost: false
  };
  canvas.addEventListener('webglcontextlost', (e) => {
    e.preventDefault();
    s.lost = true;
  });
  return s;
}

function releaseStage() {
  if (!stage) return;
  const s = stage;
  stage = null;
  try {
    s.character.dispose();
    s.props.dispose();
    s.gear.dispose();
    s.scene.traverse((o) => {
      if (o.geometry) o.geometry.dispose();
      if (o.material) {
        if (o.material.map) o.material.map.dispose();
        o.material.dispose();
      }
    });
    if (s.scene.environment) s.scene.environment.dispose();
    s.renderer.dispose();
    s.renderer.forceContextLoss();
  } catch {
    // rien à rendre de plus
  }
}

function ensureStage(id) {
  if (stage && (stage.id !== id || stage.lost)) releaseStage();
  if (stage) return stage;
  try {
    stage = createStage(id);
  } catch (err) {
    console.error(`Personnage « ${id} » indisponible :`, err);
    failed.add(id);
    stage = null;
  }
  return stage;
}

setInterval(() => {
  if (stage && performance.now() - lastDrawAt > RELEASE_MS) releaseStage();
}, 10000);

function musicLevel() {
  const m = J.musicLevel;
  return m && performance.now() - m.at < 1500 ? m.v : 0.4;
}

J.character = {
  list: () => Object.fromEntries(Object.entries(CHARACTERS).map(([id, c]) => [id, c.meta.name])),
  // Le personnage peut-il être dessiné (WebGL disponible, pas d'échec) ?
  ready: (id) => Boolean(CHARACTERS[id]) && !failed.has(id),

  // Dessine le personnage centré sur (cx, cy), à l'échelle de la zone centrale du cœur (rayon Rc).
  // s : état du cœur (orb.js, app.js) ; accent : couleur [r, g, b] de l'état de Jarvis ; rest : Jarvis
  // au repos (le personnage prend alors sa propre couleur, s'il en a une : Marvin a les yeux verts).
  draw(g, cx, cy, Rc, t, level, s, accent, rest) {
    const id = s && s.heartStyle;
    if (!this.ready(id)) return false;
    const st = ensureStage(id);
    if (!st) return false;
    try {
      const now = performance.now();
      lastDrawAt = now;
      const dt = Math.min(0.1, Math.max(0, (s && s.dt) || 0.016));
      const director = J.characterDirector;
      const morph = J.getOrbMorph ? J.getOrbMorph() : null;
      const pick = director ? director.pick({
        now, mood: s.mood, voicePhase: s.voicePhase, thinking: s.isThinking, speaking: s.speaking,
        sleep: s.sleep, musicPlaying: s.musicPlaying, morph, companion: s.companion || null
      }) : { action: 'idle' };
      st.gear.set(s.companion || null);

      if (accent) st.accent.setRGB(accent[0] / 255, accent[1] / 255, accent[2] / 255, THREE.SRGBColorSpace);
      const eyes = rest && st.rest ? st.rest : accent;
      if (eyes) st.eyes.lerp(st.eyesTo.setRGB(eyes[0] / 255, eyes[1] / 255, eyes[2] / 255, THREE.SRGBColorSpace), Math.min(1, dt * 6));
      for (const m of st.tint) m.color.copy(st.accent);
      st.rim.color.copy(st.accent);
      st.rim2.color.copy(st.accent);
      st.dashes.rotation.y = t * 0.25;

      const rv = s.reveal;
      st.character.update(dt, t, {
        action: pick.action, data: pick.data || null, startedAt: pick.startedAt || 0,
        level: level || 0, speaking: !!s.speaking, music: s.musicPlaying ? musicLevel() : 0,
        accent: st.eyes, power: rv ? Math.min(1, rv.glow + 0.2) : 1, back: !!s.heartOff,
        gaze: J.characterGaze ? J.characterGaze.get(cx, cy) : null,
        extra: s.companion ? st.gear.extra : null
      });
      st.gear.update(dt, t, st.eyes);

      // Taille du rendu : celle affichée, par paliers (pas de réallocation à chaque respiration).
      const half = Rc * HALF;
      const k = Math.abs(g.getTransform ? g.getTransform().a : 1) || 1;
      const want = Math.min(MAX_PX, Math.max(64, Math.ceil((half * 2 * k) / 32) * 32));
      if (want !== st.px) {
        st.px = want;
        st.renderer.setSize(want, want, false);
      }
      st.renderer.render(st.scene, st.camera);
      g.drawImage(st.canvas, cx - half, cy - half, half * 2, half * 2);
      return true;
    } catch (err) {
      console.error('Personnage du cœur : rendu impossible, retour au cœur classique.', err);
      failed.add(id);
      releaseStage();
      return false;
    }
  }
};
