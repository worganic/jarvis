// Accessoires des personnages du cœur (ui/character.js) : enveloppe, globe, clavier, écran de code,
// panneaux (agenda, horloge, image), bouclier, notes de musique, Zzz, étincelles, points de réflexion,
// point d'interrogation, ondes sonores, goutte de sueur et montre. Communs à tous les personnages :
// chacun les place à sa façon (ui/characters/<id>.js), en coordonnées de son repère (pieds à l'origine).
//
// À chaque image : begin() → show(nom, x, y, z, options) pour chaque accessoire voulu → update().
// Un accessoire non demandé rétrécit et disparaît ; un accessoire demandé grandit (léger rebond).

const TAU = Math.PI * 2;
const easeOutBack = (x) => {
  const c = 1.70158;
  return 1 + (c + 1) * Math.pow(x - 1, 3) + c * Math.pow(x - 1, 2);
};

export function createProps(THREE) {
  const root = new THREE.Group();
  const items = {};
  const accent = new THREE.Color(0x38e1ff);
  const accentMats = []; // matériaux teintés par la couleur de l'état de Jarvis
  const disposables = [];

  // --- Outils ------------------------------------------------------------------------------------

  function holo(opacity = 0.9, opts = {}) {
    const m = new THREE.MeshBasicMaterial({
      color: accent.clone(), transparent: true, opacity, blending: THREE.AdditiveBlending,
      depthWrite: false, side: THREE.DoubleSide, toneMapped: false, ...opts
    });
    accentMats.push(m);
    disposables.push(m);
    return m;
  }

  function lineMat(opacity = 0.9) {
    const m = new THREE.LineBasicMaterial({ color: accent.clone(), transparent: true, opacity, blending: THREE.AdditiveBlending, depthWrite: false, toneMapped: false });
    accentMats.push(m);
    disposables.push(m);
    return m;
  }

  function canvasTex(w, h) {
    const c = document.createElement('canvas');
    c.width = w;
    c.height = h;
    const tex = new THREE.CanvasTexture(c);
    tex.colorSpace = THREE.SRGBColorSpace;
    tex.anisotropy = 4;
    disposables.push(tex);
    return { c, g: c.getContext('2d'), tex };
  }

  // Sprite d'un caractère (♪, Z, ?, ✦) : une texture partagée par caractère.
  const glyphTex = new Map();
  function glyphSprite(ch, opacity = 1) {
    let tex = glyphTex.get(ch);
    if (!tex) {
      const t = canvasTex(128, 128);
      t.g.font = 'bold 96px "Segoe UI Symbol", "Segoe UI", sans-serif';
      t.g.textAlign = 'center';
      t.g.textBaseline = 'middle';
      t.g.shadowColor = 'rgba(255,255,255,0.9)';
      t.g.shadowBlur = 14;
      t.g.fillStyle = '#ffffff';
      t.g.fillText(ch, 64, 70);
      t.tex.needsUpdate = true;
      tex = t.tex;
      glyphTex.set(ch, tex);
    }
    const m = new THREE.SpriteMaterial({ map: tex, color: accent.clone(), transparent: true, opacity, blending: THREE.AdditiveBlending, depthWrite: false, toneMapped: false });
    accentMats.push(m);
    disposables.push(m);
    return new THREE.Sprite(m);
  }

  function item(name, group, update) {
    group.visible = false;
    root.add(group);
    items[name] = { name, group, update: update || null, vis: 0, want: false, shownAt: 0, pos: new THREE.Vector3(), rotY: 0, opts: null };
  }

  function rr(g, x, y, w, h, r) {
    g.beginPath();
    g.moveTo(x + r, y);
    g.arcTo(x + w, y, x + w, y + h, r);
    g.arcTo(x + w, y + h, x, y + h, r);
    g.arcTo(x, y + h, x, y, r);
    g.arcTo(x, y, x + w, y, r);
    g.closePath();
  }

  const rgbOf = (c) => `${Math.round(c.r * 255)}, ${Math.round(c.g * 255)}, ${Math.round(c.b * 255)}`;

  // Fond de panneau holographique (verre sombre, cadre lumineux) partagé par les panneaux.
  function panelBack(g, w, h, rgb) {
    g.clearRect(0, 0, w, h);
    rr(g, 6, 6, w - 12, h - 12, 22);
    g.fillStyle = 'rgba(4, 14, 26, 0.82)';
    g.fill();
    g.lineWidth = 5;
    g.strokeStyle = `rgba(${rgb}, 0.95)`;
    g.shadowColor = `rgba(${rgb}, 0.9)`;
    g.shadowBlur = 16;
    g.stroke();
    g.shadowBlur = 0;
    g.globalAlpha = 0.12;
    g.fillStyle = `rgb(${rgb})`;
    for (let y = 10; y < h - 10; y += 6) g.fillRect(10, y, w - 20, 1.5);
    g.globalAlpha = 1;
  }

  function makePanel(name, w, h, draw) {
    const tx = canvasTex(w, h);
    const mat = new THREE.MeshBasicMaterial({ map: tx.tex, transparent: true, depthWrite: false, side: THREE.DoubleSide, toneMapped: false });
    disposables.push(mat);
    const geo = new THREE.PlaneGeometry(0.78 * (w / Math.max(w, h)), 0.78 * (h / Math.max(w, h)));
    disposables.push(geo);
    const mesh = new THREE.Mesh(geo, mat);
    const group = new THREE.Group();
    group.add(mesh);
    let key = '';
    item(name, group, (it, dt, t, ctx) => {
      const k = draw.key(ctx, it);
      if (k !== key) {
        key = k;
        draw.paint(tx.g, w, h, rgbOf(accent), ctx, it);
        tx.tex.needsUpdate = true;
      }
      mesh.position.y = Math.sin(t * 1.6) * 0.02;
      mesh.rotation.y = Math.sin(t * 0.9) * 0.06;
    });
    return { mesh, group };
  }

  // --- Enveloppe (courrier) -------------------------------------------------------------------------
  {
    const group = new THREE.Group();
    const W = 0.56;
    const H = 0.38;
    const paper = new THREE.MeshStandardMaterial({ color: 0xf2f5fb, roughness: 0.55, metalness: 0.0 });
    const inside = new THREE.MeshStandardMaterial({ color: 0xc9d3e3, roughness: 0.7, side: THREE.DoubleSide });
    disposables.push(paper, inside);
    const bodyGeo = new THREE.BoxGeometry(W, H, 0.025);
    disposables.push(bodyGeo);
    const body = new THREE.Mesh(bodyGeo, paper);
    group.add(body);
    // Plis du dos de l'enveloppe (V du bas).
    const folds = new THREE.BufferGeometry().setFromPoints([
      new THREE.Vector3(-W / 2, -H / 2, 0.014), new THREE.Vector3(0, -0.01, 0.014),
      new THREE.Vector3(0, -0.01, 0.014), new THREE.Vector3(W / 2, -H / 2, 0.014)
    ]);
    disposables.push(folds);
    group.add(new THREE.LineSegments(folds, new THREE.LineBasicMaterial({ color: 0x9aa8bd })));
    // Lettre qui sort de l'enveloppe.
    const letterTex = canvasTex(256, 192);
    {
      const g = letterTex.g;
      g.fillStyle = '#ffffff';
      g.fillRect(0, 0, 256, 192);
      g.fillStyle = '#8fa3bf';
      for (let i = 0; i < 6; i++) g.fillRect(28, 34 + i * 24, i === 0 ? 120 : 200 - (i % 3) * 30, 7);
      letterTex.tex.needsUpdate = true;
    }
    const letterMat = new THREE.MeshStandardMaterial({ map: letterTex.tex, roughness: 0.6 });
    const letterGeo = new THREE.PlaneGeometry(W * 0.86, H * 0.9);
    disposables.push(letterMat, letterGeo);
    const letter = new THREE.Mesh(letterGeo, letterMat);
    letter.position.z = 0.002;
    group.add(letter);
    // Rabat articulé sur le bord du haut, avec son cachet.
    const flapShape = new THREE.Shape();
    flapShape.moveTo(-W / 2, 0);
    flapShape.lineTo(W / 2, 0);
    flapShape.lineTo(0, -H * 0.62);
    flapShape.closePath();
    const flapGeo = new THREE.ShapeGeometry(flapShape);
    disposables.push(flapGeo);
    const hinge = new THREE.Group();
    hinge.position.set(0, H / 2, 0.014);
    const flap = new THREE.Mesh(flapGeo, inside);
    hinge.add(flap);
    const sealMat = new THREE.MeshStandardMaterial({ color: 0xd8283c, roughness: 0.35, emissive: 0x400008 });
    const sealGeo = new THREE.CylinderGeometry(0.045, 0.045, 0.012, 24);
    disposables.push(sealMat, sealGeo);
    const seal = new THREE.Mesh(sealGeo, sealMat);
    seal.rotation.x = Math.PI / 2;
    seal.position.set(0, -H * 0.56, 0.008);
    hinge.add(seal);
    group.add(hinge);
    // Contour holographique (rappel du HUD).
    const edgeGeo = new THREE.EdgesGeometry(bodyGeo);
    disposables.push(edgeGeo);
    const edges = new THREE.LineSegments(edgeGeo, lineMat(0.8));
    edges.scale.setScalar(1.04);
    group.add(edges);
    item('envelope', group, (it, dt, t) => {
      const a = t - it.shownAt;
      const open = Math.min(1, Math.max(0, (a - 0.45) / 0.55));
      hinge.rotation.x = -2.75 * (1 - Math.pow(1 - open, 3));
      const slide = Math.min(1, Math.max(0, (a - 0.95) / 0.6));
      letter.position.y = 0.2 * (1 - Math.pow(1 - slide, 3));
      // La lettre sort par le haut : elle passe devant le corps une fois l'enveloppe ouverte.
      letter.position.z = slide > 0 ? 0.016 : 0.002;
      group.rotation.z = Math.sin(t * 2.2) * 0.05;
    });
  }

  // --- Globe (navigateur, web) ------------------------------------------------------------------------
  {
    const group = new THREE.Group();
    const spin = new THREE.Group();
    const R = 0.24;
    const pts = [];
    for (let i = 0; i < 8; i++) {
      const a = (i / 8) * Math.PI;
      for (let k = 0; k < 48; k++) {
        const b0 = (k / 48) * TAU;
        const b1 = ((k + 1) / 48) * TAU;
        pts.push(new THREE.Vector3(Math.cos(a) * Math.sin(b0) * R, Math.cos(b0) * R, Math.sin(a) * Math.sin(b0) * R));
        pts.push(new THREE.Vector3(Math.cos(a) * Math.sin(b1) * R, Math.cos(b1) * R, Math.sin(a) * Math.sin(b1) * R));
      }
    }
    for (let j = 1; j < 6; j++) {
      const y = Math.cos((j / 6) * Math.PI) * R;
      const r = Math.sqrt(R * R - y * y);
      for (let k = 0; k < 48; k++) {
        const b0 = (k / 48) * TAU;
        const b1 = ((k + 1) / 48) * TAU;
        pts.push(new THREE.Vector3(Math.cos(b0) * r, y, Math.sin(b0) * r));
        pts.push(new THREE.Vector3(Math.cos(b1) * r, y, Math.sin(b1) * r));
      }
    }
    const wire = new THREE.BufferGeometry().setFromPoints(pts);
    disposables.push(wire);
    spin.add(new THREE.LineSegments(wire, lineMat(0.85)));
    const coreGeo = new THREE.SphereGeometry(R * 0.97, 32, 20);
    disposables.push(coreGeo);
    spin.add(new THREE.Mesh(coreGeo, holo(0.16)));
    spin.rotation.z = 0.35;
    group.add(spin);
    // Anneau d'orbite et satellite.
    const orbitGeo = new THREE.TorusGeometry(R * 1.35, 0.006, 6, 64);
    const satGeo = new THREE.SphereGeometry(0.025, 12, 8);
    disposables.push(orbitGeo, satGeo);
    const orbit = new THREE.Mesh(orbitGeo, holo(0.6));
    orbit.rotation.x = Math.PI / 2.3;
    const sat = new THREE.Mesh(satGeo, holo(1));
    group.add(orbit, sat);
    item('globe', group, (it, dt, t) => {
      spin.rotation.y = t * 0.9;
      // Satellite sur l'anneau d'orbite (incliné de π/2,3 autour de x).
      const a = t * 2.2;
      sat.position.set(Math.cos(a) * R * 1.35, -Math.sin(a) * R * 1.35 * Math.cos(Math.PI / 2.3), -Math.sin(a) * R * 1.35 * Math.sin(Math.PI / 2.3));
      group.position.y += Math.sin(t * 2) * 0.03;
    });
  }

  // --- Clavier holographique (code, travail) ----------------------------------------------------------
  {
    const t = canvasTex(512, 180);
    const mat = new THREE.MeshBasicMaterial({ map: t.tex, transparent: true, depthWrite: false, blending: THREE.AdditiveBlending, side: THREE.DoubleSide, toneMapped: false });
    const geo = new THREE.PlaneGeometry(0.74, 0.26);
    disposables.push(mat, geo);
    const mesh = new THREE.Mesh(geo, mat);
    mesh.rotation.x = -0.75;
    const group = new THREE.Group();
    group.add(mesh);
    let last = -1;
    item('keyboard', group, (it, dt, time, ctx) => {
      const frame = Math.floor(time * 12);
      if (frame === last) return;
      last = frame;
      const g = t.g;
      const rgb = rgbOf(accent);
      g.clearRect(0, 0, 512, 180);
      rr(g, 4, 4, 504, 172, 16);
      g.fillStyle = `rgba(${rgb}, 0.10)`;
      g.fill();
      g.lineWidth = 3;
      g.strokeStyle = `rgba(${rgb}, 0.9)`;
      g.stroke();
      const busy = ctx && ctx.typing !== undefined ? ctx.typing : 1;
      for (let r = 0; r < 4; r++) {
        const n = r === 3 ? 1 : 11 - r;
        const kw = r === 3 ? 220 : 38;
        const x0 = r === 3 ? 146 : 22 + r * 12;
        for (let k = 0; k < n; k++) {
          const lit = Math.random() < 0.09 * busy;
          rr(g, x0 + k * 43, 18 + r * 38, kw, 30, 5);
          g.fillStyle = lit ? `rgba(${rgb}, 0.95)` : `rgba(${rgb}, 0.22)`;
          g.fill();
        }
      }
      t.tex.needsUpdate = true;
    });
  }

  // --- Écran de code ----------------------------------------------------------------------------------
  {
    const W = 384;
    const H = 272;
    const t = canvasTex(W, H);
    const mat = new THREE.MeshBasicMaterial({ map: t.tex, transparent: true, depthWrite: false, side: THREE.DoubleSide, toneMapped: false });
    const geo = new THREE.PlaneGeometry(0.74, 0.52);
    disposables.push(mat, geo);
    const mesh = new THREE.Mesh(geo, mat);
    const group = new THREE.Group();
    group.add(mesh);
    const words = ['const', 'jarvis', '=', 'await', 'run(', 'task', ');', 'if', '(ok)', 'return', 'emit(', 'say', '{', '}', 'for', 'let', 'i', 'of', 'cards', 'heart.draw()'];
    const lines = [];
    let seed = 7;
    const rnd = () => ((seed = (seed * 16807) % 2147483647) / 2147483647);
    for (let i = 0; i < 40; i++) {
      const indent = Math.floor(rnd() * 3);
      const n = 2 + Math.floor(rnd() * 4);
      const parts = [];
      for (let k = 0; k < n; k++) parts.push(words[Math.floor(rnd() * words.length)]);
      lines.push({ indent, parts });
    }
    let last = -1;
    item('screen', group, (it, dt, time) => {
      const frame = Math.floor(time * 8);
      if (frame === last) return;
      last = frame;
      const g = t.g;
      const rgb = rgbOf(accent);
      panelBack(g, W, H, rgb);
      g.font = 'bold 19px Consolas, monospace';
      g.textBaseline = 'top';
      const scroll = Math.floor((time - it.shownAt) * 3);
      const colors = [`rgba(${rgb}, 1)`, 'rgba(255, 210, 120, 1)', 'rgba(150, 255, 190, 1)', 'rgba(235, 245, 255, 1)'];
      for (let row = 0; row < 9; row++) {
        const L = lines[(scroll + row) % lines.length];
        let x = 26 + L.indent * 22;
        const y = 22 + row * 26;
        g.fillStyle = 'rgba(160, 190, 220, 0.5)';
        g.fillText(String(((scroll + row) % 99) + 1).padStart(2, '0'), 8, y);
        L.parts.forEach((w, k) => {
          g.fillStyle = colors[(w.length + k) % colors.length];
          g.fillText(w, x + 20, y);
          x += g.measureText(`${w} `).width;
        });
      }
      if (Math.floor(time * 3) % 2) {
        g.fillStyle = `rgba(${rgb}, 1)`;
        g.fillRect(46, 22 + 8 * 26, 11, 20);
      }
      t.tex.needsUpdate = true;
    });
  }

  // --- Panneaux : agenda, horloge, image (emoji) ------------------------------------------------------
  const MONTHS = ['JANVIER', 'FÉVRIER', 'MARS', 'AVRIL', 'MAI', 'JUIN', 'JUILLET', 'AOÛT', 'SEPTEMBRE', 'OCTOBRE', 'NOVEMBRE', 'DÉCEMBRE'];
  const DAYS = ['DIMANCHE', 'LUNDI', 'MARDI', 'MERCREDI', 'JEUDI', 'VENDREDI', 'SAMEDI'];

  makePanel('calendar', 300, 300, {
    key: () => `${new Date().toDateString()}|${rgbOf(accent)}`,
    paint(g, w, h, rgb) {
      panelBack(g, w, h, rgb);
      const d = new Date();
      g.textAlign = 'center';
      g.fillStyle = `rgba(${rgb}, 1)`;
      g.font = 'bold 26px Consolas, monospace';
      g.fillText(`${MONTHS[d.getMonth()]} ${d.getFullYear()}`, w / 2, 46);
      g.font = 'bold 15px Consolas, monospace';
      g.fillStyle = 'rgba(200, 225, 245, 0.75)';
      'LMMJVSD'.split('').forEach((c, i) => g.fillText(c, 42 + i * 36, 78));
      const first = new Date(d.getFullYear(), d.getMonth(), 1);
      const offset = (first.getDay() + 6) % 7;
      const days = new Date(d.getFullYear(), d.getMonth() + 1, 0).getDate();
      g.font = 'bold 17px Consolas, monospace';
      for (let n = 1; n <= days; n++) {
        const k = n - 1 + offset;
        const x = 42 + (k % 7) * 36;
        const y = 108 + Math.floor(k / 7) * 32;
        if (n === d.getDate()) {
          g.fillStyle = `rgba(${rgb}, 0.95)`;
          g.beginPath();
          g.arc(x, y - 6, 15, 0, TAU);
          g.fill();
          g.fillStyle = '#02131f';
        } else {
          g.fillStyle = 'rgba(225, 240, 255, 0.9)';
        }
        g.fillText(String(n), x, y);
      }
    }
  });

  makePanel('clock', 300, 300, {
    key: () => `${Math.floor(Date.now() / 1000)}|${rgbOf(accent)}`,
    paint(g, w, h, rgb, ctx) {
      panelBack(g, w, h, rgb);
      const d = new Date();
      const date = ctx && ctx.data && ctx.data.mode === 'date';
      g.textAlign = 'center';
      g.textBaseline = 'middle';
      g.shadowColor = `rgba(${rgb}, 0.9)`;
      g.shadowBlur = 12;
      g.fillStyle = 'rgba(235, 250, 255, 1)';
      if (date) {
        g.font = 'bold 30px Consolas, monospace';
        g.fillText(DAYS[d.getDay()], w / 2, 82);
        g.font = 'bold 110px Consolas, monospace';
        g.fillText(String(d.getDate()), w / 2, 160);
        g.font = 'bold 30px Consolas, monospace';
        g.fillText(MONTHS[d.getMonth()], w / 2, 240);
      } else {
        g.font = 'bold 76px Consolas, monospace';
        g.fillText(`${String(d.getHours()).padStart(2, '0')}h${String(d.getMinutes()).padStart(2, '0')}`, w / 2, 140);
        g.font = 'bold 22px Consolas, monospace';
        g.fillStyle = 'rgba(200, 225, 245, 0.85)';
        g.fillText(`${DAYS[d.getDay()]} ${d.getDate()}`, w / 2, 212);
      }
      g.shadowBlur = 0;
      if (!date) {
        const s = d.getSeconds() / 60;
        g.lineWidth = 6;
        g.strokeStyle = `rgba(${rgb}, 0.9)`;
        g.beginPath();
        g.arc(w / 2, h / 2, 128, -Math.PI / 2, -Math.PI / 2 + s * TAU);
        g.stroke();
      }
    }
  });

  makePanel('billboard', 300, 300, {
    key: (ctx) => `${(ctx && ctx.data && ctx.data.emoji) || ''}|${(ctx && ctx.data && ctx.data.caption) || ''}|${rgbOf(accent)}`,
    paint(g, w, h, rgb, ctx) {
      panelBack(g, w, h, rgb);
      const data = (ctx && ctx.data) || {};
      const emoji = data.emoji || '✨';
      g.textAlign = 'center';
      g.textBaseline = 'middle';
      g.font = `${data.caption ? 150 : 180}px "Segoe UI Emoji", "Apple Color Emoji", "Noto Color Emoji", sans-serif`;
      g.fillText(emoji, w / 2, data.caption ? 128 : h / 2 + 6);
      if (data.caption) {
        g.font = 'bold 54px Consolas, monospace';
        g.shadowColor = `rgba(${rgb}, 0.9)`;
        g.shadowBlur = 12;
        g.fillStyle = 'rgba(235, 250, 255, 1)';
        g.fillText(String(data.caption), w / 2, 246);
        g.shadowBlur = 0;
      }
    }
  });

  // --- Bouclier (code de validation) ------------------------------------------------------------------
  {
    const s = new THREE.Shape();
    s.moveTo(0, 0.3);
    s.quadraticCurveTo(0.14, 0.3, 0.25, 0.22);
    s.lineTo(0.23, 0.0);
    s.quadraticCurveTo(0.19, -0.2, 0, -0.32);
    s.quadraticCurveTo(-0.19, -0.2, -0.23, 0.0);
    s.lineTo(-0.25, 0.22);
    s.quadraticCurveTo(-0.14, 0.3, 0, 0.3);
    const geo = new THREE.ExtrudeGeometry(s, { depth: 0.035, bevelEnabled: true, bevelThickness: 0.015, bevelSize: 0.015, bevelSegments: 3, curveSegments: 16 });
    geo.translate(0, 0, -0.02);
    const mat = new THREE.MeshStandardMaterial({ color: 0x1a2433, metalness: 0.7, roughness: 0.28, emissive: 0x000000 });
    disposables.push(geo, mat);
    const mesh = new THREE.Mesh(geo, mat);
    const group = new THREE.Group();
    group.add(mesh);
    const rim = new THREE.Line(new THREE.BufferGeometry().setFromPoints(s.getPoints(40).map((p) => new THREE.Vector3(p.x * 1.02, p.y * 1.02, 0.035))), lineMat(1));
    disposables.push(rim.geometry);
    group.add(rim);
    // Cadenas : anse + corps.
    const lockGeo = new THREE.BoxGeometry(0.12, 0.1, 0.02);
    const shackleGeo = new THREE.TorusGeometry(0.04, 0.012, 8, 20, Math.PI);
    disposables.push(lockGeo, shackleGeo);
    const lock = new THREE.Mesh(lockGeo, holo(0.95));
    lock.position.set(0, -0.03, 0.04);
    const shackle = new THREE.Mesh(shackleGeo, holo(0.95));
    shackle.position.set(0, 0.02, 0.04);
    group.add(lock, shackle);
    item('shield', group, (it, dt, t) => {
      mat.emissive.copy(accent).multiplyScalar(0.12 + 0.08 * Math.sin(t * 4));
      group.rotation.y = Math.sin(t * 1.3) * 0.12;
    });
  }

  // --- Notes de musique, Zzz, étincelles ---------------------------------------------------------------
  function floaters(name, chars, count, spawnEvery, life, spread, rise, size = 0.11) {
    const group = new THREE.Group();
    const pool = [];
    for (let i = 0; i < count; i++) {
      const sp = glyphSprite(chars[i % chars.length]);
      sp.visible = false;
      group.add(sp);
      pool.push({ sp, born: -99, x: 0, drift: 0, size: 0.1 });
    }
    let nextAt = 0;
    let k = 0;
    item(name, group, (it, dt, t, ctx) => {
      const every = typeof spawnEvery === 'function' ? spawnEvery(ctx) : spawnEvery;
      if (t >= nextAt) {
        const p = pool[k++ % pool.length];
        p.born = t;
        p.x = (Math.random() - 0.5) * spread;
        p.drift = (Math.random() - 0.5) * spread * 0.6;
        p.size = size * (1 + Math.random() * 0.6);
        nextAt = t + every;
      }
      for (const p of pool) {
        const a = (t - p.born) / life;
        if (a < 0 || a >= 1) {
          p.sp.visible = false;
          continue;
        }
        p.sp.visible = true;
        p.sp.position.set(p.x + p.drift * a + Math.sin(a * 6 + p.x * 10) * 0.03, a * rise, 0);
        p.sp.material.opacity = Math.min(1, a * 5) * (1 - a);
        p.sp.scale.setScalar(p.size * (0.7 + 0.5 * a));
      }
    });
  }
  // Notes de musique : une par temps (plus rare en calme, deux par temps en électro), au rythme détecté.
  const noteEvery = (ctx) => {
    const d = ctx && ctx.data;
    const bpm = d && d.bpm ? Math.max(60, Math.min(180, d.bpm)) : 120;
    const k = d && d.style === 'calme' ? 2 : d && d.style === 'electro' ? 0.5 : 1;
    return Math.max(0.16, Math.min(1.4, (60 / bpm) * k));
  };
  floaters('notes', ['♪', '♫', '♬'], 9, noteEvery, 1.8, 1.4, 0.9);
  floaters('zzz', ['Z', 'z'], 4, 1.1, 3.2, 0.12, 0.45, 0.2);
  floaters('sparkles', ['✦', '✧', '★'], 10, 0.14, 1.1, 1.3, 0.6);

  // --- Points de réflexion (« … ») ---------------------------------------------------------------------
  {
    const group = new THREE.Group();
    const geo = new THREE.SphereGeometry(0.045, 16, 10);
    disposables.push(geo);
    const dots = [0, 1, 2].map((i) => {
      const m = new THREE.Mesh(geo, holo(1));
      m.position.x = (i - 1) * 0.13;
      group.add(m);
      return m;
    });
    item('dots', group, (it, dt, t) => {
      dots.forEach((d, i) => {
        const p = Math.max(0, Math.sin(t * 5 - i * 0.9));
        d.position.y = p * 0.05;
        d.scale.setScalar(0.75 + 0.45 * p);
      });
    });
  }

  // --- Point d'interrogation --------------------------------------------------------------------------
  {
    const group = new THREE.Group();
    const sp = glyphSprite('?');
    sp.scale.setScalar(0.32);
    group.add(sp);
    item('question', group, (it, dt, t) => {
      sp.position.y = Math.sin(t * 3) * 0.03;
      sp.material.rotation = Math.sin(t * 2.4) * 0.2;
    });
  }

  // --- Ondes sonores (écoute) -------------------------------------------------------------------------
  {
    const group = new THREE.Group();
    const arcs = [0, 1, 2].map(() => {
      const geo = new THREE.TorusGeometry(0.1, 0.012, 6, 24, Math.PI * 0.7);
      disposables.push(geo);
      const m = new THREE.Mesh(geo, holo(0.9));
      m.rotation.z = Math.PI - Math.PI * 0.35;
      group.add(m);
      return m;
    });
    item('waves', group, (it, dt, t, ctx) => {
      const lvl = ctx ? Math.min(1, ctx.level * 2) : 0.5;
      arcs.forEach((m, i) => {
        const a = (t * 1.4 + i / 3) % 1;
        m.scale.setScalar(0.6 + a * 1.6);
        m.material.opacity = (1 - a) * (0.35 + 0.65 * lvl);
      });
    });
  }

  // --- Goutte de sueur (fatigue, quota épuisé) ---------------------------------------------------------
  {
    const group = new THREE.Group();
    const mat = new THREE.MeshPhysicalMaterial({ color: 0x8fd8ff, roughness: 0.05, transmission: 0, transparent: true, opacity: 0.85, clearcoat: 1 });
    const geo = new THREE.SphereGeometry(0.05, 16, 12);
    const tip = new THREE.ConeGeometry(0.035, 0.07, 16);
    disposables.push(mat, geo, tip);
    const drop = new THREE.Group();
    const ball = new THREE.Mesh(geo, mat);
    const cone = new THREE.Mesh(tip, mat);
    cone.position.y = 0.045;
    drop.add(ball, cone);
    group.add(drop);
    item('sweat', group, (it, dt, t) => {
      const a = ((t - it.shownAt) % 2.2) / 2.2;
      drop.position.y = -a * 0.22;
      mat.opacity = 0.85 * (1 - a);
    });
  }

  // --- Montre (le personnage la porte au poignet le temps de regarder l'heure) -------------------------
  {
    const group = new THREE.Group();
    const bandGeo = new THREE.TorusGeometry(0.068, 0.02, 10, 28);
    const faceGeo = new THREE.CylinderGeometry(0.045, 0.045, 0.02, 24);
    const band = new THREE.MeshStandardMaterial({ color: 0x1b2230, roughness: 0.4, metalness: 0.5 });
    disposables.push(bandGeo, faceGeo, band);
    const ring = new THREE.Mesh(bandGeo, band);
    ring.rotation.x = Math.PI / 2;
    const face = new THREE.Mesh(faceGeo, holo(1));
    face.position.z = 0.075;
    face.rotation.x = Math.PI / 2;
    group.add(ring, face);
    item('watch', group, null);
  }

  // --- API ---------------------------------------------------------------------------------------------

  function begin() {
    for (const it of Object.values(items)) {
      it.want = false;
      it.opts = null;
    }
  }

  // Demande l'accessoire `name` à la position (x, y, z) du repère du personnage. opts.parent :
  // objet auquel l'accrocher (main, poignet) ; opts.rotY / opts.rotX : orientation ; opts.scale.
  function show(name, x = 0, y = 0, z = 0, opts = {}) {
    const it = items[name];
    if (!it) return;
    it.want = true;
    it.opts = opts;
    const parent = opts.parent || root;
    if (it.group.parent !== parent) {
      parent.add(it.group);
      it.vis = Math.min(it.vis, 0.01);
    }
    if (it.vis < 0.02) it.pos.set(x, y, z);
    else it.pos.lerp(new THREE.Vector3(x, y, z), 0.25);
  }

  function update(dt, t, ctx = {}) {
    if (ctx.accent) accent.copy(ctx.accent);
    for (const m of accentMats) m.color.copy(accent);
    for (const it of Object.values(items)) {
      const target = it.want ? 1 : 0;
      if (target && it.vis <= 0.001) it.shownAt = t;
      it.vis += (target - it.vis) * Math.min(1, dt * (target ? 7 : 10));
      if (it.vis < 0.005 && !target) {
        it.vis = 0;
        it.group.visible = false;
        continue;
      }
      it.group.visible = true;
      const o = it.opts || {};
      it.group.position.copy(it.pos);
      it.group.rotation.set(o.rotX || 0, o.rotY || 0, o.rotZ || 0);
      it.group.scale.setScalar(Math.max(0.001, easeOutBack(Math.min(1, it.vis))) * (o.scale || 1));
      if (it.update) it.update(it, dt, t, ctx);
    }
  }

  function dispose() {
    for (const d of disposables) if (d && d.dispose) d.dispose();
  }

  return { root, items, begin, show, update, dispose, accent };
}
