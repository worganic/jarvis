// Explorateur 3D d'un dossier Data, en grand par-dessus le HUD (noyau : orch/savoir/explore.rs).
// Événements : « savoir_3d » ouvre (maquette de l'IA, modèles .glb, pièces, fiche, liens), « savoir_3d_control »
// pilote (fonctionner, eclate, rayons, recentrer, piece, question, reponse), « savoir_3d_close » ferme.
// La maquette (formes simples nommées par pièce) devient une scène three.js : un groupe par pièce, et ce qui tourne
// (anime: rotation_*) pivote autour du centre de sa pièce — le rotor autour de son moyeu, pas chaque pale sur elle-même.
// Un modèle .glb libre se charge à la demande (bouton « Modèle réel ») ; ses pièces, non nommées, sont des repères
// placés sur sa boîte englobante. Seule la fenêtre meneuse l'affiche ; Échap ou ✕ le ferme (op « explorer_ferme »).
// Mode « Réaliste » (par défaut quand le noyau en a trouvé, orch/savoir/sketchfab.rs) : un vrai modèle fait par un
// graphiste, affiché par la visionneuse Sketchfab et piloté par sa Viewer API — animation, caméra, nœuds masqués
// (rayons X) ou déplacés (vue éclatée), clic sur une pièce. Les pièces du dossier sont reliées aux nœuds nommés du
// modèle par le noyau (op « pieces3d »). La maquette de formes simples ne sert plus que de schéma de secours.
import * as THREE from './vendor/three.module.min.js';
import { OrbitControls } from './vendor/OrbitControls.js';
import { studioEnvironment } from './three-studio.js';

const J = (window.JARVIS = window.JARVIS || {});
const bridge = window.jarvisBridge;

const SIZE = 4;            // plus grande dimension du produit, en unités de scène
const FLY_S = 0.9;         // vol de la caméra vers une pièce (s)
const IDLE_MS = 8000;      // sans geste depuis 8 s : la rotation automatique reprend
const SPIN = 7;            // vitesse des pièces qui tournent en marche (rad/s)
const POS = { haut: [0.5, 1, 0.5], bas: [0.5, 0, 0.5], avant: [0.5, 0.55, 1], arriere: [0.5, 0.55, 0], gauche: [0, 0.55, 0.5], droite: [1, 0.55, 0.5], centre: [0.5, 0.5, 0.5] };
const BODY = '_corps';     // éléments sans pièce : le corps du produit

const SK_API = 'https://static.sketchfab.com/api/sketchfab-viewer-1.12.1.js';
const SK_INIT = {
  autostart: 1, preload: 1, camera: 0, dnt: 1, transparent: 1,
  ui_hint: 0, ui_infos: 0, ui_controls: 0, ui_stop: 0, ui_help: 0, ui_settings: 0, ui_vr: 0, ui_ar: 0,
  ui_fullscreen: 0, ui_annotations: 0, ui_inspector: 0, ui_animations: 0, ui_watermark: 0
};

const call = (op, args) => Promise.resolve(bridge && bridge.call ? bridge.call('savoir', op, args) : null).catch(() => null);
const isLeader = () => (bridge && typeof bridge.isLeader === 'function' ? bridge.isLeader() : true);
const say = (text) => { if (bridge && bridge.call) bridge.call('utterance', text).catch(() => {}); };
const ease = (x) => 1 - Math.pow(1 - Math.min(1, Math.max(0, x)), 3);
const host = (u) => String(u || '').replace(/^https?:\/\/(?:www\.)?/, '').split('/')[0];

function h(tag, attrs = {}, ...kids) {
  const el = document.createElement(tag);
  for (const [k, v] of Object.entries(attrs || {})) {
    if (v == null || v === false) continue;
    if (k.startsWith('on')) el.addEventListener(k.slice(2), v);
    else if (k === 'class') el.className = v;
    else el.setAttribute(k, v === true ? '' : v);
  }
  for (const c of kids.flat()) if (c != null && c !== false) el.append(c instanceof Node ? c : document.createTextNode(String(c)));
  return el;
}

function accentColor() {
  const v = getComputedStyle(document.body).getPropertyValue('--cyan').trim() || '#38e1ff';
  return new THREE.Color(v);
}

let view = null;

// --- Maquette : formes de l'IA → objets three.js -----------------------------------------------------------

/** Axe d'une forme de révolution : la dimension qui se distingue des deux autres (y si elles se valent). */
function revolutionAxis(d) {
  const odd = d.map((v, i) => {
    const others = d.filter((_, j) => j !== i);
    return Math.abs(Math.log(v / Math.sqrt(others[0] * others[1])));
  });
  const i = odd.indexOf(Math.max(...odd));
  return odd[i] < 0.25 ? 1 : i;
}

/** Couche une géométrie construite sur l'axe y vers l'axe i (0 = x, 2 = z). */
function alongAxis(g, i) {
  if (i === 0) g.rotateZ(-Math.PI / 2);
  else if (i === 2) g.rotateX(Math.PI / 2);
  return g;
}

// Formes de révolution (cylindre, cône, capsule) : l'IA donne tantôt une forme debout qu'elle couche par sa rotation,
// tantôt directement ses dimensions dans le monde (longueur en z) avec, en plus, une rotation. L'axe est donc lu dans
// les dimensions ; la rotation ne s'applique qu'à une forme debout (`keepRotation`).
function geometryOf(e) {
  const d = Array.isArray(e.dimensions) ? e.dimensions.map((n) => Math.max(0.01, Math.abs(n))) : [1, 1, 1];
  const [a, b, c] = d;
  const axis = revolutionAxis(d);
  const keepRotation = axis === 1;
  switch (e.forme) {
    case 'sphere': return { g: new THREE.SphereGeometry(0.5, 32, 20), s: [a, b, c], keepRotation: true };
    case 'cylindre': return { g: alongAxis(new THREE.CylinderGeometry(0.5, 0.5, 1, 40), axis), s: [a, b, c], keepRotation };
    case 'cone': return { g: alongAxis(new THREE.ConeGeometry(0.5, 1, 40), axis), s: [a, b, c], keepRotation };
    case 'capsule': {
      const across = d.filter((_, j) => j !== axis);
      const r = Math.min(...across) / 2;
      const s = d.map((v, j) => (j === axis ? 1 : v / (2 * r)));
      return { g: alongAxis(new THREE.CapsuleGeometry(r, Math.max(0.001, d[axis] - 2 * r), 10, 28), axis), s, keepRotation };
    }
    case 'tore': {
      // Anneau à plat (plan x-z) : diamètre extérieur en x et z, épaisseur en y.
      const tube = Math.min(b, a / 2) / 2;
      const g = new THREE.TorusGeometry(Math.max(0.01, a / 2 - tube), tube, 18, 56);
      g.rotateX(Math.PI / 2);
      return { g, s: [1, 1, c / a], keepRotation: true };
    }
    default: return { g: new THREE.BoxGeometry(1, 1, 1), s: [a, b, c], keepRotation: true };
  }
}

function materialOf(hex) {
  const color = new THREE.Color(/^#[0-9a-f]{6}$/i.test(hex || '') ? hex : '#c8d2da');
  const hsl = color.getHSL({});
  // Couleurs sombres et grises : métal ; couleurs vives : peinture.
  const metal = hsl.s < 0.25 ? (hsl.l < 0.35 ? 0.7 : 0.45) : 0.15;
  return new THREE.MeshStandardMaterial({ color, metalness: metal, roughness: hsl.l < 0.3 ? 0.35 : 0.42, envMapIntensity: 1.1 });
}

/** Maquette → { root, pieces: Map(id → info) } ; un groupe par pièce, pivots pour ce qui tourne. */
function buildMaquette(maquette) {
  const root = new THREE.Group();
  const pieces = new Map();
  const piece = (id) => {
    if (!pieces.has(id)) {
      const group = new THREE.Group();
      group.userData.piece = id;
      root.add(group);
      pieces.set(id, { id, group, spinners: [], meshes: [] });
    }
    return pieces.get(id);
  };
  const els = (maquette && Array.isArray(maquette.elements)) ? maquette.elements : [];
  // Éléments animés d'une même pièce et d'un même axe : un pivot commun, au centre de leurs positions.
  const spinGroups = new Map();
  for (const e of els) {
    if (!e.anime) continue;
    const k = `${e.piece || BODY}|${e.anime}`;
    if (!spinGroups.has(k)) spinGroups.set(k, []);
    spinGroups.get(k).push(e);
  }
  const pivots = new Map();
  for (const [k, list] of spinGroups) {
    const c = new THREE.Vector3();
    for (const e of list) c.add(new THREE.Vector3(...(e.position || [0, 0, 0])));
    c.divideScalar(list.length);
    const pivot = new THREE.Group();
    pivot.position.copy(c);
    const [id, anime] = k.split('|');
    const axis = anime === 'rotation_x' ? 'x' : anime === 'rotation_z' ? 'z' : 'y';
    const p = piece(id);
    p.group.add(pivot);
    p.spinners.push({ pivot, axis });
    pivots.set(k, pivot);
  }
  for (const e of els) {
    const { g, s, keepRotation } = geometryOf(e);
    const mesh = new THREE.Mesh(g, materialOf(e.couleur));
    mesh.scale.set(...s);
    const pos = new THREE.Vector3(...(e.position || [0, 0, 0]));
    if (Array.isArray(e.rotation) && keepRotation) mesh.rotation.set(...e.rotation);
    const p = piece(e.piece || BODY);
    const pivot = e.anime ? pivots.get(`${e.piece || BODY}|${e.anime}`) : null;
    if (pivot) {
      mesh.position.copy(pos.sub(pivot.position));
      pivot.add(mesh);
    } else {
      mesh.position.copy(pos);
      p.group.add(mesh);
    }
    mesh.userData.piece = p.id;
    p.meshes.push(mesh);
  }
  return { root, pieces };
}

/** Produit centré, posé au sol et ramené à SIZE unités. */
function normalize(obj) {
  const box = new THREE.Box3().setFromObject(obj);
  const size = box.getSize(new THREE.Vector3());
  const k = SIZE / Math.max(size.x, size.y, size.z, 0.001);
  obj.scale.multiplyScalar(k);
  const box2 = new THREE.Box3().setFromObject(obj);
  const c = box2.getCenter(new THREE.Vector3());
  obj.position.sub(new THREE.Vector3(c.x, box2.min.y, c.z));
  obj.updateMatrixWorld(true);
  return new THREE.Box3().setFromObject(obj);
}

// --- Scène ----------------------------------------------------------------------------------------------

function createStage(canvas) {
  const renderer = new THREE.WebGLRenderer({ canvas, antialias: true, alpha: true, powerPreference: 'high-performance' });
  renderer.setPixelRatio(Math.min(1.75, window.devicePixelRatio || 1));
  renderer.setClearColor(0x000000, 0);
  renderer.toneMapping = THREE.ACESFilmicToneMapping;
  renderer.toneMappingExposure = 1.05;
  renderer.outputColorSpace = THREE.SRGBColorSpace;
  renderer.shadowMap.enabled = true;
  renderer.shadowMap.type = THREE.PCFSoftShadowMap;
  const scene = new THREE.Scene();
  scene.environment = studioEnvironment(renderer);
  const accent = accentColor();
  scene.add(new THREE.HemisphereLight(0xdff4ff, 0x0a1420, 0.55));
  const key = new THREE.DirectionalLight(0xffffff, 2.2);
  key.position.set(5, 9, 6);
  key.castShadow = true;
  key.shadow.mapSize.set(2048, 2048);
  Object.assign(key.shadow.camera, { left: -5, right: 5, top: 5, bottom: -5, near: 1, far: 30 });
  scene.add(key);
  const rim = new THREE.DirectionalLight(accent, 1.4);
  rim.position.set(-6, 3, -6);
  scene.add(rim);
  // Sol : grille holographique et ombre douce.
  const grid = new THREE.GridHelper(24, 48, accent, accent);
  grid.material.transparent = true;
  grid.material.opacity = 0.12;
  grid.material.depthWrite = false;
  scene.add(grid);
  const floor = new THREE.Mesh(new THREE.PlaneGeometry(30, 30), new THREE.ShadowMaterial({ opacity: 0.35 }));
  floor.rotation.x = -Math.PI / 2;
  floor.position.y = 0.001;
  floor.receiveShadow = true;
  scene.add(floor);
  const camera = new THREE.PerspectiveCamera(38, 1, 0.05, 200);
  camera.position.set(6.5, 4.2, 7.5);
  const controls = new OrbitControls(camera, canvas);
  controls.enableDamping = true;
  controls.dampingFactor = 0.08;
  controls.minDistance = 1.2;
  controls.maxDistance = 24;
  controls.maxPolarAngle = Math.PI * 0.495;
  controls.target.set(0, 1.2, 0);
  controls.autoRotateSpeed = 0.9;
  return { renderer, scene, camera, controls, accent, grid };
}

function disposeTree(obj) {
  obj.traverse((o) => {
    if (o.geometry) o.geometry.dispose();
    const mats = Array.isArray(o.material) ? o.material : o.material ? [o.material] : [];
    for (const m of mats) {
      for (const k of ['map', 'normalMap', 'roughnessMap', 'metalnessMap', 'emissiveMap', 'aoMap']) if (m[k]) m[k].dispose();
      m.dispose();
    }
  });
}

// --- Interface -----------------------------------------------------------------------------------------

function buildDom(data) {
  const pieces = Array.isArray(data.pieces) ? data.pieces : [];
  const canvas = h('canvas', { class: 'v3-canvas' });
  const labels = h('div', { class: 'v3-labels' });
  const info = h('div', { class: 'v3-info-body' });
  const links = h('div', { class: 'v3-links' });
  const list = h('div', { class: 'v3-pieces' });
  const tools = h('div', { class: 'v3-tools' });
  const modeBox = h('div', { class: 'v3-mode' });
  const toast = h('div', { class: 'v3-toast' });
  const frameBox = h('div', { class: 'v3-framebox', style: 'display:none' });
  const models = h('div', { class: 'v3-models' });
  const credits = h('div', { class: 'v3-credits' });
  const root = h('div', { id: 'viewer3d', class: 'v3-opening' },
    canvas, frameBox, labels,
    h('div', { class: 'v3-top' },
      h('div', { class: 'v3-title' }, h('span', { class: 'v3-ico' }, '◆'), h('b', {}, data.sujet || ''), data.categorie ? h('span', { class: 'v3-cat' }, data.categorie) : null),
      modeBox,
      h('button', { class: 'v3-close', title: 'Fermer (Échap)', onclick: () => close(true) }, '✕')),
    h('div', { class: 'v3-left' }, h('div', { class: 'v3-head' }, `Pièces (${pieces.length})`), list, models),
    h('div', { class: 'v3-right' },
      h('div', { class: 'v3-panel' }, info),
      h('div', { class: 'v3-panel v3-links-panel' }, h('div', { class: 'v3-head' }, 'Pour aller plus loin'), links)),
    tools,
    toast,
    credits,
    h('div', { class: 'v3-hint' }, 'Glisser : tourner · Molette : zoom · Clic droit : déplacer · Double-clic : centrer · Échap : fermer'));
  return { root, canvas, frameBox, labels, info, links, list, models, credits, tools, modeBox, toast };
}

function showToast(text) {
  if (!view) return;
  view.dom.toast.textContent = text;
  view.dom.toast.classList.add('on');
  clearTimeout(view.toastTimer);
  view.toastTimer = setTimeout(() => view && view.dom.toast.classList.remove('on'), 2600);
}

function renderLinks(data) {
  const box = view.dom.links;
  const all = Array.isArray(data.liens) ? data.liens : [];
  if (!all.length) { box.append(h('div', { class: 'v3-empty' }, 'Aucun lien dans ce dossier.')); return; }
  const groups = new Map();
  for (const l of all) {
    if (!groups.has(l.groupe)) groups.set(l.groupe, []);
    groups.get(l.groupe).push(l);
  }
  const ICON = { Sources: '↗', 'Vidéos': '▶', 'Modèles 3D': '◆', Lectures: '¶' };
  for (const [g, items] of groups) {
    box.append(h('div', { class: 'v3-group' }, `${ICON[g] || '↗'} ${g} (${items.length})`));
    for (const l of items.slice(0, 25)) {
      box.append(h('button', { class: 'v3-link', title: `Ouvrir ${l.url}`, onclick: () => { say(`Ouvre ${l.url}`); showToast(`Ouverture de ${host(l.url)}…`); } },
        h('span', { class: 't' }, l.titre), h('span', { class: 's' }, host(l.url))));
    }
  }
}

/** Lignes de la fiche qui parlent de la pièce (mots de son nom). */
function relatedSpecs(data, p) {
  const words = String(p.nom || '').toLowerCase().normalize('NFD').replace(/[̀-ͯ]/g, '').split(/[^a-z0-9]+/).filter((w) => w.length >= 4);
  if (!words.length) return [];
  const fold = (s) => String(s || '').toLowerCase().normalize('NFD').replace(/[̀-ͯ]/g, '');
  return (data.fiche || []).filter((f) => words.some((w) => fold(f.cle).includes(w) || fold(f.valeur).includes(w))).slice(0, 6);
}

function renderInfo() {
  const { data, dom } = view;
  const box = dom.info;
  box.replaceChildren();
  const add = (...els) => box.append(...els.filter(Boolean));
  const spec = (f) => h('div', { class: 'v3-spec' }, h('span', {}, f.cle), h('b', {}, [f.valeur, f.unite].filter(Boolean).join(' ')));
  if (view.answer) {
    const a = view.answer;
    add(h('div', { class: 'v3-q' }, `« ${a.question} »`), h('div', { class: 'v3-a' }, a.reponse || 'Je cherche dans le dossier…'));
  }
  const p = view.selected ? (data.pieces || []).find((x) => x.id === view.selected) : null;
  if (p) {
    const specs = relatedSpecs(data, p);
    add(
      h('div', { class: 'v3-kicker' }, 'PIÈCE'),
      h('div', { class: 'v3-name' }, p.nom || p.id),
      p.description ? h('div', { class: 'v3-desc' }, p.description) : null,
      specs.length ? h('div', { class: 'v3-specs' }, ...specs.map(spec)) : null,
      h('div', { class: 'v3-actions' },
        h('button', { class: 'v3-btn primary', title: 'Jarvis répond à voix haute, à partir du dossier', onclick: () => { say(`Comment fonctionne ${p.nom || p.id} ?`); } }, '✦ Demander à Jarvis'),
        h('button', { class: 'v3-btn', onclick: () => select(null) }, 'Vue d’ensemble')));
    return;
  }
  if (!view.answer) {
    add(
      h('div', { class: 'v3-kicker' }, 'EN BREF'),
      data.resume ? h('div', { class: 'v3-desc' }, data.resume) : null,
      (data.fiche || []).length ? h('div', { class: 'v3-specs' }, ...(data.fiche || []).slice(0, 6).map(spec)) : null,
      h('div', { class: 'v3-tip' }, 'Clique une pièce, ou demande : « fais-le fonctionner », « vue éclatée », « montre-moi le rotor ».'));
  }
}

function renderPieceList() {
  const { data, dom } = view;
  dom.list.replaceChildren(...(data.pieces || []).map((p) => {
    const shown = !view.sk || view.pieces.has(p.id);
    const b = h('button', { class: `v3-piece${shown ? '' : ' off'}`, 'data-id': p.id, title: shown ? '' : 'Pièce non repérée sur ce modèle : sa fiche seulement', onclick: () => select(p.id), onmouseenter: () => hover(p.id), onmouseleave: () => hover(null) },
      h('span', { class: 'dot' }), p.nom || p.id);
    return b;
  }));
  if (!(data.pieces || []).length) dom.list.append(h('div', { class: 'v3-empty' }, 'Pas de pièce nommée.'));
}

function toolButton(label, tip, action) {
  return h('button', { class: 'v3-tool', title: tip, 'data-action': action, onclick: () => control(action) }, label);
}

function renderTools() {
  const { dom } = view;
  dom.tools.replaceChildren(...[
    toolButton('▶ Faire fonctionner', 'Met en marche ce qui bouge (rotors, hélices…)', 'fonctionner'),
    toolButton('⇲ Vue éclatée', 'Écarte les pièces les unes des autres', 'eclate'),
    toolButton('◐ Rayons X', 'Rend le produit transparent pour voir l’intérieur', 'rayons'),
    toolButton('🏷 Étiquettes', 'Affiche ou masque le nom des pièces', 'etiquettes'),
    view.sk ? null : toolButton('⟳ Rotation auto', 'Le produit tourne tout seul quand on n’y touche pas', 'rotation'),
    toolButton('⟲ Recentrer', 'Revenir à la vue d’ensemble', 'recentrer')].filter(Boolean));
  syncTools();
}

function syncTools() {
  if (!view) return;
  const s = view.sk;
  const on = s
    ? { fonctionner: s.playing, eclate: s.exploded, rayons: s.hidden, etiquettes: view.labelsOn }
    : { fonctionner: view.running, eclate: view.explodeTo > 0, rayons: view.xray, etiquettes: view.labelsOn, rotation: view.autoRotate };
  for (const b of view.dom.tools.querySelectorAll('.v3-tool')) {
    const a = b.dataset.action;
    b.classList.toggle('on', Boolean(on[a]));
    if (a === 'fonctionner') b.textContent = on.fonctionner ? '■ Arrêter' : '▶ Faire fonctionner';
  }
  for (const b of view.dom.list.querySelectorAll('.v3-piece')) b.classList.toggle('on', b.dataset.id === view.selected);
}

/** Modes disponibles : réaliste (Sketchfab), modèle .glb téléchargé, schéma (maquette de l'IA). */
function modesOf(data) {
  const out = [];
  if ((data.sketchfab || []).length) out.push(['sketchfab', 'Réaliste', 'Modèle fait par un graphiste (Sketchfab)']);
  if ((data.modeles || []).length) out.push(['glb', 'Modèle .glb', 'Modèle 3D libre téléchargé dans le dossier']);
  if (data.maquette && Array.isArray(data.maquette.elements) && data.maquette.elements.length) out.push(['maquette', 'Schéma', 'Maquette simplifiée construite par l’IA']);
  return out;
}

function renderModeSwitch() {
  const { data, dom } = view;
  const modes = modesOf(data);
  dom.modeBox.replaceChildren(...(modes.length > 1 ? modes.map(([m, label, tip]) => h('button', { class: `v3-seg${view.mode === m ? ' on' : ''}`, title: tip, onclick: () => setMode(m) }, label)) : []));
  // Crédits du modèle affiché (obligatoires) et lien vers sa page.
  const m = view.mode === 'sketchfab' ? view.skModel : view.mode === 'glb' ? (data.modeles || [])[0] : null;
  dom.credits.replaceChildren(...(m ? [h('button', { class: 'v3-credit', title: m.page ? `Ouvrir ${m.page}` : '', onclick: () => { if (m.page) { say(`Ouvre ${m.page}`); showToast('Ouverture de la page du modèle…'); } } },
    `◆ ${[m.nom || 'Modèle 3D', m.auteur ? `par ${m.auteur}` : '', m.licence].filter(Boolean).join(' · ')}${view.mode === 'sketchfab' ? ' · Sketchfab' : ''}${m.page ? ' ↗' : ''}`)] : []));
  // Autres modèles réalistes trouvés : un clic change de modèle.
  const list = data.sketchfab || [];
  dom.models.replaceChildren(...(list.length > 1 ? [h('div', { class: 'v3-head' }, `Modèles (${list.length})`),
    ...list.map((x) => h('button', { class: `v3-model${view.mode === 'sketchfab' && view.skModel && view.skModel.uid === x.uid ? ' on' : ''}`, title: `${x.nom} · ${x.auteur}${x.anime ? ' · animé' : ''}`, onclick: () => setMode('sketchfab', x) },
      x.vignette ? h('img', { src: x.vignette, alt: '', draggable: 'false' }) : null,
      h('span', {}, x.nom, x.anime ? h('em', {}, ' ▶') : null)))] : []));
}

// --- Pièces : survol, sélection, étiquettes ------------------------------------------------------------------

function pieceCenter(id) {
  const p = view.pieces.get(id);
  if (p && p.group && p.meshes.length) {
    const box = new THREE.Box3();
    for (const m of p.meshes) box.expandByObject(m);
    return { center: box.getCenter(new THREE.Vector3()), size: box.getSize(new THREE.Vector3()).length() };
  }
  if (p && p.anchor) return { center: p.anchor.clone(), size: SIZE * 0.35 };
  return null;
}

function paint() {
  if (!view) return;
  const A = view.stage.accent;
  for (const [id, p] of view.pieces) {
    const lit = id === view.hovered || id === view.selected;
    const dim = view.selected && id !== view.selected;
    for (const m of p.meshes) {
      const mat = m.material;
      if (!mat || !mat.emissive) continue;
      mat.emissive.copy(lit ? A : new THREE.Color(0, 0, 0));
      mat.emissiveIntensity = id === view.selected ? 0.35 : 0.22;
      const alpha = view.xray ? (id === view.selected ? 0.9 : 0.18) : dim ? 0.28 : 1;
      mat.transparent = alpha < 1;
      mat.opacity = alpha;
      mat.depthWrite = alpha >= 1;
    }
  }
  if (view.glb) {
    view.glb.traverse((o) => {
      if (!o.isMesh) return;
      for (const mat of Array.isArray(o.material) ? o.material : [o.material]) {
        const alpha = view.xray ? 0.2 : 1;
        mat.transparent = alpha < 1;
        mat.opacity = alpha;
        mat.depthWrite = alpha >= 1;
      }
    });
  }
}

function hover(id) {
  if (!view || view.hovered === id) return;
  view.hovered = id;
  paint();
}

function select(id, fly = true) {
  if (!view) return;
  view.selected = id && (view.data.pieces || []).some((p) => p.id === id) ? id : null;
  if (!view.selected) view.answer = null;
  paint();
  renderInfo();
  syncTools();
  if (fly && view.sk) {
    if (!view.selected || view.pieces.has(view.selected)) skFocus(view.selected);
  } else if (fly) {
    const pc = view.selected && view.pieces.has(view.selected) ? pieceCenter(view.selected) : null;
    if (pc) flyTo(pc.center, Math.max(1.6, Math.min(7, pc.size * 2.2)));
    else home();
  }
}

function flyTo(target, dist) {
  const { camera, controls } = view.stage;
  const dir = camera.position.clone().sub(controls.target).normalize();
  if (dir.y < 0.15) dir.y = 0.15;
  dir.normalize();
  view.fly = { t0: performance.now(), fromPos: camera.position.clone(), fromTarget: controls.target.clone(), toTarget: target.clone(), toPos: target.clone().add(dir.multiplyScalar(dist)) };
  touched();
}

function home() {
  if (view.sk) { skFocus(null); return; }
  const box = view.box;
  const c = box.getCenter(new THREE.Vector3());
  view.fly = { t0: performance.now(), fromPos: view.stage.camera.position.clone(), fromTarget: view.stage.controls.target.clone(), toTarget: c, toPos: c.clone().add(new THREE.Vector3(4.7, 2.7, 5.5)) };
}

function buildLabels() {
  const { dom, data } = view;
  dom.labels.replaceChildren();
  view.labels = [];
  for (const p of data.pieces || []) {
    if (!view.pieces.has(p.id)) continue;
    const el = h('button', { class: 'v3-label', onclick: () => select(p.id), onmouseenter: () => hover(p.id), onmouseleave: () => hover(null) }, p.nom || p.id);
    dom.labels.append(el);
    view.labels.push({ id: p.id, el });
  }
}

function updateLabels() {
  const { camera, renderer } = view.stage;
  const w = renderer.domElement.clientWidth;
  const hgt = renderer.domElement.clientHeight;
  for (const l of view.labels) {
    const pc = pieceCenter(l.id);
    const show = pc && (view.labelsOn || l.id === view.hovered || l.id === view.selected);
    if (!show) { l.el.style.display = 'none'; continue; }
    const v = pc.center.clone();
    v.y += Math.min(0.6, pc.size * 0.25);
    v.project(camera);
    if (v.z > 1) { l.el.style.display = 'none'; continue; }
    l.el.style.display = '';
    l.el.style.transform = `translate(${((v.x + 1) / 2) * w}px, ${((1 - v.y) / 2) * hgt}px) translate(-50%, -100%)`;
    l.el.classList.toggle('on', l.id === view.selected || l.id === view.hovered);
  }
}

// --- Mode réaliste : visionneuse Sketchfab -----------------------------------------------------------------

let skLoading = null;
function loadSketchfab() {
  if (window.Sketchfab) return Promise.resolve(window.Sketchfab);
  skLoading = skLoading || new Promise((resolve, reject) => {
    const s = document.createElement('script');
    s.src = SK_API;
    s.onload = () => resolve(window.Sketchfab);
    s.onerror = () => { skLoading = null; reject(new Error('visionneuse Sketchfab injoignable')); };
    document.head.append(s);
  });
  return skLoading;
}

/** Appel de la Viewer API en promesse : (erreur, valeur) ou une seule valeur selon la fonction ; null en échec. */
function sk(fn, ...args) {
  const api = view && view.sk && view.sk.api;
  if (!api || typeof api[fn] !== 'function') return Promise.resolve(null);
  return new Promise((resolve) => {
    try {
      api[fn](...args, (...r) => resolve(r.length >= 2 ? (r[0] ? null : r[1]) : r[0]));
    } catch { resolve(null); }
  });
}

const v3 = (a) => new THREE.Vector3(a[0], a[1], a[2]);

function stopSketchfab() {
  if (!view || !view.sk) return;
  clearInterval(view.sk.labelTimer);
  try { view.sk.api && view.sk.api.stop(); } catch {}
  view.sk = null;
  view.dom.frameBox.replaceChildren();
  view.dom.frameBox.style.display = 'none';
}

async function showSketchfab(model) {
  stopSketchfab();
  clearModel();
  buildLabels(); // plus d'étiquettes du modèle précédent
  view.dom.canvas.style.display = 'none';
  const box = view.dom.frameBox;
  box.style.display = '';
  const frame = h('iframe', { class: 'v3-frame', allow: 'autoplay; fullscreen; xr-spatial-tracking', title: model.nom || 'Modèle 3D' });
  box.replaceChildren(frame, h('div', { class: 'v3-loading' }, h('span', { class: 'v3-spinner' }), `Chargement de « ${model.nom} »…`));
  const token = (view.skToken = {});
  let api;
  try {
    const Sk = await loadSketchfab();
    api = await new Promise((resolve, reject) => {
      const client = new Sk('1.12.1', frame);
      setTimeout(() => reject(new Error('délai dépassé')), 45000);
      client.init(model.uid, { ...SK_INIT, success: (a) => { a.start(); a.addEventListener('viewerready', () => resolve(a)); }, error: () => reject(new Error('modèle indisponible')) });
    });
  } catch (err) {
    if (view && view.skToken === token) showToast(`Modèle réaliste indisponible (${err.message}).`);
    return false;
  }
  if (!view || view.skToken !== token) return false;
  const loading = box.querySelector('.v3-loading');
  if (loading) loading.lastChild.textContent = 'Repérage des pièces…';
  view.sk = { api, model, nodes: new Map(), parent: new Map(), pieceNodes: new Map(), envelope: [], anchors: new Map(), base: new Map(), anim: false, playing: false, hidden: false, exploded: false };
  const nodes = Object.values((await sk('getNodeMap')) || {});
  for (const n of nodes) {
    view.sk.nodes.set(n.instanceID, n);
    for (const c of n.children || []) view.sk.parent.set(c.instanceID, n.instanceID);
  }
  const anims = (await sk('getAnimations')) || [];
  if (anims.length) {
    view.sk.anim = true;
    await sk('setCycleMode', 'loop');
    await skRest();
  }
  const start = await sk('getCameraLookAt');
  api.addEventListener('click', onSketchfabClick, { pick: 'slow' });
  // Pièces du dossier ↔ nœuds nommés du modèle (calculé une fois par le noyau).
  const named = nodes.filter((n) => n.name && n.type !== 'Geometry');
  const r = await call('pieces3d', { slug: view.data.slug, uid: model.uid, noeuds: [...new Set(named.map((n) => n.name))] });
  if (!view || !view.sk || view.sk.api !== api) return false;
  const map = (r && r.correspondance) || { pieces: {}, enveloppe: [] };
  const idsOf = (names) => named.filter((n) => names.includes(n.name)).map((n) => n.instanceID);
  // Vue d'ensemble, position et cadrage de chaque pièce, mesurés sur l'image affichée.
  view.sk.pending = (view.data.pieces || []).map((p) => ({ id: p.id, ids: idsOf((map.pieces || {})[p.id] || []) })).filter((p) => p.ids.length);
  view.sk.envelope = idsOf(map.enveloppe || []);
  // Mesure vérifiée : chaque étiquette doit retomber sur sa pièce ; sinon (produit qui bougeait encore), on recommence.
  let m = null;
  for (let attempt = 0; attempt < 3 && view && view.sk && view.sk.api === api; attempt++) {
    if (view.sk.anim) await skRest();
    m = start ? await measure(start) : null;
    if (!m || (await verified(m))) break;
    await new Promise((r) => setTimeout(r, 800));
  }
  if (!view || !view.sk || view.sk.api !== api) return false;
  view.pieces = m ? m.pieces : new Map();
  view.sk.home = m ? m.home : start;
  view.sk.center = m ? m.center : null;
  view.sk.radius = m ? m.radius : 1;
  box.querySelector('.v3-loading')?.remove();
  buildLabels();
  view.sk.labelTimer = setInterval(updateSketchfabLabels, 120);
  renderPieceList();
  return true;
}

/** Touche l'image sur une grille de cols × rows points : [{ p: Vector3, id }] des surfaces atteintes. Mesure ce qui
 *  est réellement affiché — les boîtes englobantes de Sketchfab ignorent l'animation des modèles articulés. */
async function gridPick(cols, rows) {
  const f = view.dom.frameBox;
  const w = f.clientWidth;
  const hgt = f.clientHeight;
  const jobs = [];
  for (let i = 0; i < cols; i++) {
    for (let j = 0; j < rows; j++) jobs.push(sk('pickFromScreen', [w * (i + 0.5) / cols, hgt * (j + 0.5) / rows]));
  }
  const out = [];
  for (const hit of await Promise.all(jobs)) if (hit && hit.position3D) out.push({ p: v3(Array.from(hit.position3D)), id: hit.instanceID });
  return out;
}

/** Centre et rayon d'un nuage de points (rayon : 90 % des points, pour ignorer un sol ou un décor lointain). */
function cloud(points) {
  if (!points.length) return null;
  const c = points.reduce((a, b) => a.add(b), new THREE.Vector3()).divideScalar(points.length);
  const d = points.map((p) => p.distanceTo(c)).sort((a, b) => a - b);
  return { c, r: Math.max(d[Math.floor((d.length - 1) * 0.9)], 1e-3) };
}

/** Pièce (id du dossier) à laquelle appartient un nœud touché, en remontant l'arbre. */
function pieceOfNode(id) {
  for (let x = id, i = 0; x != null && i < 60; x = view.sk.parent.get(x), i++) {
    for (const p of view.sk.pending) if (p.ids.includes(x)) return p.id;
  }
  return null;
}

/** Cadrage d'un point vu depuis la direction `dir`, à la distance `dist`. */
const shotAt = (c, dir, dist) => ({ position: c.clone().add(dir.clone().multiplyScalar(dist)).toArray(), target: c.toArray() });

/** Animation au repos : en pause, ramenée au début (pose d'origine), le temps que l'image se stabilise. La lecture peut
 *  démarrer après la fin du chargement (HUD plus lent qu'un navigateur) : la pause est donc redemandée. */
async function skRest() {
  for (let i = 0; i < 4; i++) {
    await sk('pause');
    await sk('seekTo', 0);
    await new Promise((r) => setTimeout(r, 250));
    const a = await sk('getCurrentTime');
    await new Promise((r) => setTimeout(r, 150));
    const b = await sk('getCurrentTime');
    if (a === b && Math.abs(Number(b) || 0) < 0.05) return;
  }
}

/** Les pièces mesurées sont-elles bien sous leur étiquette ? On touche l'image là où chaque étiquette pointe. */
async function verified(m) {
  const list = [...m.pieces.values()];
  if (!list.length) return true;
  let ok = 0;
  for (const p of list) {
    const c = await sk('getWorldToScreenCoordinates', p.anchor.toArray());
    const xy = c && c.canvasCoord;
    if (!xy) continue;
    const hit = await sk('pickFromScreen', [xy[0], xy[1]]);
    const at = hit && hit.position3D ? v3(Array.from(hit.position3D)) : null;
    if ((hit && pieceOfNode(hit.instanceID) === p.id) || (at && at.distanceTo(p.anchor) < m.radius * 0.35)) ok++;
  }
  return ok * 2 >= list.length;
}

/** Vue d'ensemble et position de chaque pièce, mesurées sur l'image. */
async function measure(start) {
  const s = view.sk;
  const dir = v3(start.position).sub(v3(start.target)).normalize();
  // 1. Le produit entier, depuis la vue d'origine du modèle (après un cadrage automatique s'il est hors champ).
  let all = cloud((await gridPick(48, 28)).map((x) => x.p));
  if (!all) {
    await sk('focusOnVisibleGeometries');
    await new Promise((r) => setTimeout(r, 900));
    all = cloud((await gridPick(48, 28)).map((x) => x.p));
  }
  if (!all) return null;
  const home = shotAt(all.c, dir, all.r * 2.9);
  await sk('setCameraLookAt', home.position, home.target, 0);
  await new Promise((r) => setTimeout(r, 350));
  // 2. Chaque point touché, rendu à sa pièce.
  const hits = new Map();
  for (const x of await gridPick(64, 36)) {
    const id = pieceOfNode(x.id);
    if (!id) continue;
    if (!hits.has(id)) hits.set(id, []);
    hits.get(id).push(x.p);
  }
  // 3. Une pièce cachée (à l'intérieur) : isolée un instant, ses voisines masquées.
  for (const p of s.pending) {
    if (hits.has(p.id)) continue;
    const keep = new Set();
    for (const id of p.ids) for (let x = id; x != null; x = s.parent.get(x)) keep.add(x);
    const hide = [];
    for (const id of keep) {
      if (p.ids.includes(id)) continue;
      for (const c of (s.nodes.get(id) || {}).children || []) if (!keep.has(c.instanceID)) hide.push(c.instanceID);
    }
    for (const id of hide) await sk('hide', id);
    await new Promise((r) => setTimeout(r, 120));
    const pts = (await gridPick(48, 28)).map((x) => x.p);
    for (const id of hide) await sk('show', id);
    if (pts.length) hits.set(p.id, pts);
  }
  const pieces = new Map();
  for (const p of s.pending) {
    const cl = cloud(hits.get(p.id) || []);
    if (!cl) continue;
    pieces.set(p.id, { id: p.id, meshes: [], spinners: [], sk: p.ids, anchor: cl.c, shot: shotAt(cl.c, dir, Math.max(cl.r * 3.4, all.r * 0.7)) });
  }
  return { home, pieces, center: all.c, radius: all.r };
}

/** Clic dans la visionneuse : la pièce dont un nœud contient l'objet touché. */
function onSketchfabClick(info) {
  if (!view || !view.sk || !info || info.instanceID == null) return;
  let id = info.instanceID;
  for (let i = 0; i < 40 && id != null; i++) {
    for (const p of view.pieces.values()) if (p.sk && p.sk.includes(id)) { select(p.id); return; }
    id = view.sk.parent.get(id);
  }
}

async function updateSketchfabLabels() {
  if (!view || !view.sk) return;
  // Animation partie sans qu'on l'ait demandée : le produit se déplace, il est remis au repos et les étiquettes
  // attendent qu'il soit revenu à sa place.
  let moving = false;
  if (view.sk.anim && !view.sk.playing) {
    const t = await sk('getCurrentTime');
    if (!view || !view.sk) return;
    if (Math.abs(Number(t) || 0) > 0.05) {
      moving = true;
      if (!view.sk.resting) {
        view.sk.resting = true;
        skRest().finally(() => { if (view && view.sk) view.sk.resting = false; });
      }
    }
  }
  const off = view.sk.playing || view.sk.exploded || moving;
  const box = view.dom.frameBox.getBoundingClientRect();
  const root = view.dom.root.getBoundingClientRect();
  for (const l of view.labels) {
    const p = view.pieces.get(l.id);
    const show = p && !off && (view.labelsOn || l.id === view.selected);
    if (!show) { l.el.style.display = 'none'; continue; }
    const c = await sk('getWorldToScreenCoordinates', [p.anchor.x, p.anchor.y, p.anchor.z]);
    const xy = c && c.canvasCoord;
    if (!view || !xy) { l.el.style.display = 'none'; continue; }
    l.el.style.display = '';
    l.el.style.transform = `translate(${box.left - root.left + xy[0]}px, ${box.top - root.top + xy[1]}px) translate(-50%, -100%)`;
    l.el.classList.toggle('on', l.id === view.selected);
  }
}

/** Caméra vers une pièce (même direction de vue, plus près) ; sans pièce, vue d'origine. */
async function skFocus(id) {
  const p = id && view.pieces.get(id);
  const shot = p ? p.shot : view.sk.home;
  if (shot) await sk('setCameraLookAt', shot.position, shot.target, 1.2);
  else await sk('recenterCamera');
}

/** Vue éclatée : chaque pièce reliée s'écarte du centre, dans le repère de son parent. */
async function skExplode(on) {
  const s = view.sk;
  if (!s.center) return;
  const center = s.center;
  const reach = s.radius * 0.7;
  for (const p of view.pieces.values()) {
    const dir = p.anchor.clone().sub(center);
    if (dir.lengthSq() < 1e-6) dir.set(0, 0, 1);
    const world = dir.normalize().multiplyScalar(reach);
    for (const id of p.sk || []) {
      if (!s.base.has(id)) {
        const m = await sk('getMatrix', id);
        const pm = await sk('getMatrix', s.parent.get(id));
        if (!m || !m.local) continue;
        const inv = pm && pm.world ? new THREE.Matrix4().fromArray(Array.from(pm.world)).invert() : new THREE.Matrix4();
        s.base.set(id, { t: v3([m.local[12], m.local[13], m.local[14]]), m3: new THREE.Matrix3().setFromMatrix4(inv) });
      }
      const b = s.base.get(id);
      const t = on ? b.t.clone().add(world.clone().applyMatrix3(b.m3)) : b.t;
      sk('translate', id, t.toArray(), { duration: 0.9 });
    }
  }
}

async function skControl(action) {
  const s = view.sk;
  switch (action) {
    case 'fonctionner':
    case 'arreter':
      if (!s.anim) { showToast('Ce modèle n’a pas d’animation.'); return; }
      s.playing = action === 'arreter' ? false : !s.playing;
      await sk(s.playing ? 'play' : 'pause');
      if (!s.playing) await sk('seekTo', 0); // à l'arrêt, le produit revient à sa pose d'origine (étiquettes justes)
      break;
    case 'eclate':
    case 'assemble':
      if (!view.pieces.size) { showToast('Pièces encore inconnues pour ce modèle.'); return; }
      s.exploded = action === 'assemble' ? false : !s.exploded;
      await skExplode(s.exploded);
      break;
    case 'rayons':
      if (!s.envelope.length) { showToast('Pas de coque repérée dans ce modèle.'); return; }
      s.hidden = !s.hidden;
      for (const id of s.envelope) sk(s.hidden ? 'hide' : 'show', id);
      break;
    default: break;
  }
}

// --- Modes : maquette ou modèle réel ---------------------------------------------------------------------

function clearModel() {
  const { scene } = view.stage;
  for (const o of [view.model, view.glb]) {
    if (o) { scene.remove(o); disposeTree(o); }
  }
  view.model = null;
  view.glb = null;
  view.mixer = null;
  view.pieces = new Map();
}

function showThree() {
  stopSketchfab();
  view.dom.canvas.style.display = '';
}

function showMaquette() {
  showThree();
  clearModel();
  const { root, pieces } = buildMaquette(view.data.maquette);
  root.traverse((o) => { if (o.isMesh) { o.castShadow = true; o.receiveShadow = true; } });
  view.box = normalize(root);
  view.stage.scene.add(root);
  view.model = root;
  view.pieces = pieces;
  view.center = view.box.getCenter(new THREE.Vector3());
  // Direction d'éclatement : du centre du produit vers celui de chaque pièce.
  for (const [id, p] of pieces) {
    p.home = p.group.position.clone();
    const pc = pieceCenter(id);
    const dir = pc ? pc.center.clone().sub(view.center) : new THREE.Vector3(0, 1, 0);
    if (dir.lengthSq() < 1e-4) dir.set(0, 1, 0);
    p.explodeDir = id === BODY ? new THREE.Vector3() : dir.normalize().multiplyScalar(SIZE * 0.22 / view.model.scale.x);
  }
  buildLabels();
}

async function showGlb() {
  const m = (view.data.modeles || [])[0];
  if (!m) return false;
  showToast('Chargement du modèle 3D…');
  const r = await call('modele', { slug: view.data.slug, fichier: m.fichier });
  if (!view || !r || !r.src) { showToast('Modèle 3D indisponible (trop lourd ou introuvable).'); return false; }
  try {
    const { GLTFLoader } = await import('./vendor/GLTFLoader.js');
    const buf = await (await fetch(r.src)).arrayBuffer();
    const gltf = await new Promise((resolve, reject) => new GLTFLoader().parse(buf, '', resolve, reject));
    if (!view) return false;
    showThree();
    clearModel();
    const root = new THREE.Group();
    root.add(gltf.scene);
    root.traverse((o) => { if (o.isMesh) { o.castShadow = true; o.receiveShadow = true; } });
    view.box = normalize(root);
    view.stage.scene.add(root);
    view.glb = root;
    view.center = view.box.getCenter(new THREE.Vector3());
    if (gltf.animations && gltf.animations.length) {
      view.mixer = new THREE.AnimationMixer(gltf.scene);
      view.clips = gltf.animations;
    }
    // Repères des pièces (non nommées dans le modèle) : positions approchées sur la boîte englobante.
    for (const p of view.data.pieces || []) {
      const [fx, fy, fz] = POS[p.position] || POS.centre;
      const b = view.box;
      view.pieces.set(p.id, { id: p.id, meshes: [], spinners: [], anchor: new THREE.Vector3(b.min.x + (b.max.x - b.min.x) * fx, b.min.y + (b.max.y - b.min.y) * fy, b.min.z + (b.max.z - b.min.z) * fz) });
    }
    buildLabels();
    return true;
  } catch (err) {
    console.error('viewer3d :', err);
    showToast('Ce modèle 3D n’a pas pu être lu.');
    return false;
  }
}

async function setMode(mode, model) {
  if (!view) return false;
  const skModel = mode === 'sketchfab' ? model || (view.data.sketchfab || [])[0] : null;
  if (view.mode === mode && (!skModel || (view.skModel && view.skModel.uid === skModel.uid))) return true;
  const ok = mode === 'sketchfab' ? await showSketchfab(skModel) : mode === 'glb' ? await showGlb() : (showMaquette(), true);
  if (!view || !ok) return false;
  view.mode = mode;
  view.skModel = skModel;
  renderTools();
  renderPieceList();
  view.running = false;
  view.explodeTo = 0;
  view.explode = 0;
  select(null, false);
  home();
  renderModeSwitch();
  syncTools();
  paint();
  return true;
}

// --- Commandes -------------------------------------------------------------------------------------------

function control(action, ev = {}) {
  if (!view) return;
  if (view.sk && ['fonctionner', 'arreter', 'eclate', 'assemble', 'rayons'].includes(action)) {
    skControl(action).then(syncTools);
    return;
  }
  if (view.sk && action === 'rotation') return;
  switch (action) {
    case 'fonctionner':
    case 'arreter': {
      const movable = [...view.pieces.values()].some((p) => p.spinners.length) || (view.mixer && view.clips);
      if (action === 'fonctionner' && !view.running && !movable) { showToast('Aucune pièce mobile dans ce modèle.'); return; }
      view.running = action === 'arreter' ? false : !view.running;
      if (view.mixer && view.clips) {
        for (const c of view.clips) { const a = view.mixer.clipAction(c); if (view.running) a.play(); else a.stop(); }
      }
      break;
    }
    case 'eclate': view.explodeTo = view.explodeTo > 0 ? 0 : 1; if (view.mode === 'glb') showToast('La vue éclatée demande la maquette.'); break;
    case 'assemble': view.explodeTo = 0; break;
    case 'rayons': view.xray = !view.xray; paint(); break;
    case 'etiquettes': view.labelsOn = !view.labelsOn; break;
    case 'rotation': view.autoRotate = !view.autoRotate; break;
    case 'recentrer': view.selected = null; view.answer = null; paint(); renderInfo(); home(); break;
    case 'piece': select(ev.piece); break;
    case 'question': view.answer = { question: ev.question || '', reponse: '' }; renderInfo(); break;
    case 'reponse':
      view.answer = { question: ev.question || '', reponse: ev.reponse || '' };
      if (ev.piece && view.pieces.has(ev.piece)) select(ev.piece);
      else renderInfo();
      break;
    default: break;
  }
  syncTools();
}

function touched() {
  if (view) view.lastTouch = performance.now();
}

// --- Boucle ----------------------------------------------------------------------------------------------

function frame(now) {
  if (!view) return;
  view.raf = requestAnimationFrame(frame);
  const dt = Math.min(0.05, (now - (view.lastFrame || now)) / 1000);
  view.lastFrame = now;
  if (view.sk || view.mode === 'sketchfab') return;
  const { renderer, scene, camera, controls } = view.stage;
  const canvas = renderer.domElement;
  const w = canvas.clientWidth;
  const hgt = canvas.clientHeight;
  if (canvas.width !== Math.round(w * renderer.getPixelRatio()) || canvas.height !== Math.round(hgt * renderer.getPixelRatio())) {
    renderer.setSize(w, hgt, false);
    camera.aspect = w / Math.max(1, hgt);
    camera.updateProjectionMatrix();
  }
  // Entrée : le produit monte du sol et la caméra arrive en plongée.
  const k = ease((now - view.openAt) / 1600);
  if (view.model || view.glb) {
    const o = view.model || view.glb;
    o.position.y = (o.userData.baseY ?? (o.userData.baseY = o.position.y)) - (1 - k) * 0.6;
  }
  if (view.fly) {
    const f = ease((now - view.fly.t0) / (FLY_S * 1000));
    camera.position.lerpVectors(view.fly.fromPos, view.fly.toPos, f);
    controls.target.lerpVectors(view.fly.fromTarget, view.fly.toTarget, f);
    if (f >= 1) view.fly = null;
  }
  // Marche : les pièces mobiles accélèrent puis tournent, ralentissent à l'arrêt.
  view.speed += ((view.running ? 1 : 0) - view.speed) * Math.min(1, dt * 0.9);
  for (const p of view.pieces.values()) {
    for (const s of p.spinners) s.pivot.rotation[s.axis] += SPIN * view.speed * dt;
  }
  if (view.mixer) view.mixer.update(dt);
  // Vue éclatée.
  view.explode += (view.explodeTo - view.explode) * Math.min(1, dt * 3.5);
  if (view.model) for (const p of view.pieces.values()) if (p.home && p.explodeDir) p.group.position.copy(p.home).addScaledVector(p.explodeDir, view.explode);
  controls.autoRotate = view.autoRotate && !view.fly && now - view.lastTouch > IDLE_MS;
  controls.update();
  renderer.render(scene, camera);
  updateLabels();
}

// --- Ouverture, fermeture ---------------------------------------------------------------------------------

function onPointerDown(e) {
  view.down = { x: e.clientX, y: e.clientY };
  touched();
}

function pick(e) {
  const { renderer, camera } = view.stage;
  const r = renderer.domElement.getBoundingClientRect();
  const v = new THREE.Vector2(((e.clientX - r.left) / r.width) * 2 - 1, -((e.clientY - r.top) / r.height) * 2 + 1);
  const ray = new THREE.Raycaster();
  ray.setFromCamera(v, camera);
  const target = view.model || view.glb;
  if (!target) return null;
  const hit = ray.intersectObject(target, true).find((x) => x.object.visible && (!x.object.material || x.object.material.opacity > 0.15 || view.xray));
  if (!hit) return null;
  let o = hit.object;
  while (o && !o.userData.piece) o = o.parent;
  return { piece: o ? o.userData.piece : null, point: hit.point };
}

function onPointerUp(e) {
  if (!view || !view.down) return;
  const moved = Math.hypot(e.clientX - view.down.x, e.clientY - view.down.y);
  view.down = null;
  if (moved > 5 || e.button !== 0) return;
  const p = pick(e);
  if (p && p.piece && p.piece !== BODY && view.pieces.has(p.piece)) select(p.piece);
}

function onPointerMove(e) {
  if (!view || view.down || !view.model) return;
  const p = pick(e);
  hover(p && p.piece && p.piece !== BODY ? p.piece : null);
  view.stage.renderer.domElement.style.cursor = view.hovered ? 'pointer' : 'grab';
}

function onDblClick(e) {
  const p = pick(e);
  if (p && p.piece && p.piece !== BODY && view.pieces.has(p.piece)) select(p.piece);
  else if (p) flyTo(p.point, 3);
  else home();
}

// Un clic dans la visionneuse Sketchfab y envoie le clavier : il revient au HUD pour qu'Échap ferme l'explorateur
// (la souris continue d'agir dans la visionneuse).
function onBlur() {
  setTimeout(() => {
    const el = document.activeElement;
    if (view && el && el.tagName === 'IFRAME' && view.dom.frameBox.contains(el)) { el.blur(); window.focus(); }
  }, 0);
}

function onKey(e) {
  if (!view) return;
  if (e.key === 'Escape') {
    e.preventDefault();
    e.stopImmediatePropagation();
    close(true);
  }
}

function open(data) {
  if (view) close(false);
  const dom = buildDom(data);
  document.body.append(dom.root);
  let stage;
  try {
    stage = createStage(dom.canvas);
  } catch (err) {
    console.error('viewer3d :', err);
    dom.root.remove();
    return;
  }
  view = {
    data, dom, stage, mode: null, model: null, glb: null, pieces: new Map(), labels: [], box: new THREE.Box3(),
    selected: null, hovered: null, answer: null, running: false, speed: 0, explode: 0, explodeTo: 0,
    xray: false, labelsOn: true, autoRotate: true, fly: null, openAt: performance.now(), lastTouch: 0, presentation: data.presentation || null
  };
  document.body.classList.add('viewer3d-open');
  renderPieceList();
  renderTools();
  renderLinks(data);
  (async () => {
    for (const [m] of modesOf(data)) if (view && (await setMode(m))) break;
    if (!view) return;
    renderModeSwitch();
    renderInfo();
    if (view.mode === 'sketchfab' || !view.mode) return;
    // Arrivée en plongée, puis vue d'ensemble.
    const c = view.box.getCenter(new THREE.Vector3());
    view.stage.camera.position.set(c.x + 11, c.y + 9, c.z + 12);
    view.stage.controls.target.copy(c);
    home();
    view.fly.t0 = performance.now() + 150;
  })();
  const canvas = dom.canvas;
  canvas.addEventListener('pointerdown', onPointerDown);
  canvas.addEventListener('pointerup', onPointerUp);
  canvas.addEventListener('pointermove', onPointerMove);
  canvas.addEventListener('dblclick', onDblClick);
  canvas.addEventListener('wheel', touched, { passive: true });
  window.addEventListener('keydown', onKey, true);
  window.addEventListener('blur', onBlur);
  requestAnimationFrame(() => dom.root.classList.remove('v3-opening'));
  view.raf = requestAnimationFrame(frame);
}

function close(notify) {
  if (!view) return;
  stopSketchfab();
  const v = view;
  view = null;
  cancelAnimationFrame(v.raf);
  window.removeEventListener('keydown', onKey, true);
  window.removeEventListener('blur', onBlur);
  document.body.classList.remove('viewer3d-open');
  v.dom.root.classList.add('v3-closing');
  setTimeout(() => {
    try {
      for (const o of [v.model, v.glb]) if (o) disposeTree(o);
      v.stage.controls.dispose();
      v.stage.renderer.dispose();
      v.stage.renderer.forceContextLoss();
    } catch {}
    v.dom.root.remove();
  }, 380);
  if (notify) call('explorer_ferme', { slug: v.data.slug });
}

if (bridge && bridge.onTaskEvent) {
  bridge.onTaskEvent((ev) => {
    if (!ev || !ev.type || !String(ev.type).startsWith('savoir_3d')) return;
    if (ev.type === 'savoir_3d') { if (isLeader()) open(ev); }
    else if (ev.type === 'savoir_3d_close') close(false);
    else if (ev.type === 'savoir_3d_control') control(ev.action, ev);
  });
}

J.viewer3d = { isOpen: () => Boolean(view), close: () => close(true), debug: () => view };
