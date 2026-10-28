// Fond « circuit imprimé » du HUD : le cœur est le processeur, des pistes en partent comme les broches d'une
// carte mère, et le courant y circule selon l'activité de Jarvis. Les liaisons vers les cards sont routées
// SUR ce circuit (elles suivent les pistes quand c'est possible) : une card s'ouvre au bout d'une piste.
//
// Coût : le circuit est généré une fois (graine fixe : toujours le même) et figé dans le calque de fond
// (hud.js › buildHudLayer) ; par image, seules les impulsions sont dessinées (sprite prérendu, sans shadowBlur,
// 40 au plus) ; les routes des cards sont mises en cache et ne sont recalculées que si une card change de case.
(function () {
  const J = (window.JARVIS = window.JARVIS || {});

  const PITCH = 22;                 // pas de la grille (unités virtuelles du HUD)
  const DIRS = [[1, 0], [1, 1], [0, 1], [-1, 1], [-1, 0], [-1, -1], [0, -1], [1, -1]]; // pas de 45°, y vers le bas
  const TURN_COST = [0, 0.35, 2.2]; // tout droit, 45°, 90° (pas de demi-tour)
  const TRACE_COST = 0.55;          // une route préfère suivre une piste existante
  const MAX_PULSES = 40;
  const ROUTE_CACHE_MAX = 80;

  // Générateur pseudo-aléatoire à graine (mulberry32) : le même circuit à chaque lancement.
  function rng(seed) {
    let a = seed >>> 0;
    return () => {
      a = (a + 0x6d2b79f5) >>> 0;
      let t = a;
      t = Math.imul(t ^ (t >>> 15), t | 1);
      t ^= t + Math.imul(t ^ (t >>> 7), t | 61);
      return ((t ^ (t >>> 14)) >>> 0) / 4294967296;
    };
  }

  let layout = null;  // { W, H, cols, rows, ox, oy, cx, cy, clearR, occ, heartMask, traces, vias, pads, chips }
  let heart = { cx: 0, cy: 0, R: 0 };
  const routes = new Map();
  const pulses = [];
  const flashes = [];
  const sprites = new Map();
  let spawnDebt = 0;
  let lastRouteMs = 0;

  // ---------------------------------------------------------------------------------------------
  // Génération
  // ---------------------------------------------------------------------------------------------

  function build(W, H) {
    if (layout && layout.W === W && layout.H === H) return layout;
    const cols = Math.max(4, Math.floor(W / PITCH) - 1);
    const rows = Math.max(4, Math.floor(H / PITCH) - 1);
    const ox = (W - (cols - 1) * PITCH) / 2;
    const oy = (H - (rows - 1) * PITCH) / 2;
    const cx = W / 2;
    const cy = H / 2;
    const minDim = Math.min(W, H);
    const clearR = minDim * 0.125;
    const N = cols * rows;
    const occ = new Uint16Array(N);       // n° de piste + 1 (0 : libre)
    const reserved = new Uint8Array(N);   // 1 : zone réservée (cœur, en-tête, console…)
    const heartMask = new Uint8Array(N);  // cases sous le cœur : interdites aux routes (sauf départ)
    const at = (i, j) => j * cols + i;
    const px = (i) => ox + i * PITCH;
    const py = (j) => oy + j * PITCH;
    for (let j = 0; j < rows; j++) {
      for (let i = 0; i < cols; i++) {
        const x = px(i);
        const y = py(j);
        const k = at(i, j);
        if (Math.hypot(x - cx, y - cy) < clearR + PITCH * 0.5) { reserved[k] = 1; heartMask[k] = 1; }
        // En-tête « J.A.R.V.I.S. » et ses lignes d'état, contrôles de fenêtre, console et barre du bas.
        if ((x < 380 && y < 110) || (x > W - 230 && y < 60) || y > H - 58) reserved[k] = 1;
      }
    }

    const random = rng(0x4a415256); // « JARV »
    const traces = [];
    const vias = [];
    const pads = [];
    const joints = [];
    const chips = [];

    const inside = (i, j) => i >= 1 && j >= 1 && i < cols - 1 && j < rows - 1;
    // Une case est prise si elle est occupée ou si une case voisine appartient à une autre piste
    // (espacement d'une case entre pistes, pas de croisement en X).
    function blockedFor(i, j, id, from) {
      if (!inside(i, j)) return true;
      const k = at(i, j);
      if (reserved[k] || occ[k]) return true;
      for (const [dx, dy] of DIRS) {
        const ni = i + dx;
        const nj = j + dy;
        if (!inside(ni, nj)) continue;
        const o = occ[at(ni, nj)];
        if (o && o !== id + 1) return true;
        if (o === id + 1 && from && !(ni === from[0] && nj === from[1]) && !(Math.abs(ni - from[0]) <= 1 && Math.abs(nj - from[1]) <= 1)) return true;
      }
      return false;
    }

    // Une piste : suite de pas (direction, longueur) appliquée depuis une case. Tours de ±45° seulement.
    // opts.parent : embranchement en T d'une piste existante (sa voisine immédiate est tolérée au départ).
    function lay(i, j, plan, opts = {}) {
      const id = traces.length;
      if (!inside(i, j) || reserved[at(i, j)] || occ[at(i, j)]) return null;
      const parentId = opts.parent === undefined ? -1 : opts.parent;
      // Départ d'un embranchement : collé à sa piste mère, donc on ne vérifie que la case elle-même.
      if (parentId < 0 && blockedFor(i, j, id, null)) return null;
      const cells = [[i, j]];
      occ[at(i, j)] = id + 1;
      let ci = i;
      let cj = j;
      for (const [dir, len] of plan) {
        const [dx, dy] = DIRS[dir];
        let stop = false;
        for (let s = 0; s < len; s++) {
          const ni = ci + dx;
          const nj = cj + dy;
          // Diagonale : pas de croisement avec une autre piste qui passerait entre les deux cases.
          if (dx && dy) {
            const a = occ[at(ci + dx, cj)];
            const b = occ[at(ci, cj + dy)];
            if ((a && a !== id + 1) || (b && b !== id + 1)) { stop = true; break; }
          }
          const nearParent = parentId >= 0 && cells.length < 2;
          if (nearParent ? (!inside(ni, nj) || reserved[at(ni, nj)] || occ[at(ni, nj)]) : blockedFor(ni, nj, id, [ci, cj])) { stop = true; break; }
          ci = ni;
          cj = nj;
          occ[at(ci, cj)] = id + 1;
          cells.push([ci, cj]);
        }
        if (stop) break;
      }
      if (cells.length < (opts.min || 3)) {
        for (const [a, b] of cells) occ[at(a, b)] = 0;
        return null;
      }
      let pts = corners(cells.map(([a, b]) => ({ x: px(a), y: py(b) })));
      if (opts.from) pts = [opts.from, ...pts]; // l'embranchement part de la piste mère
      const width = opts.bus ? 1.35 : random() < 0.22 ? 1.7 : 1.0;
      const t = { id, pts, ...measure(pts), bus: Boolean(opts.bus), cells: cells.length, width, cellList: cells };
      traces.push(t);
      vias.push(pts[pts.length - 1]);
      if (opts.from) joints.push(opts.from);
      else if (!opts.bus) pads.push(pts[0]);
      return t;
    }

    // Embranchement en T depuis le milieu d'une piste orthogonale : départ perpendiculaire, puis 45°.
    function branchFrom(t) {
      const cells = t.cellList;
      if (!cells || cells.length < 6) return;
      const k = 2 + Math.floor(random() * (cells.length - 4));
      const [a, b] = cells[k];
      const [pa, pb] = cells[k - 1];
      const dirAlong = DIRS.findIndex(([dx, dy]) => dx === a - pa && dy === b - pb);
      if (dirAlong < 0 || dirAlong % 2) return; // seulement sur un tronçon droit
      const side = random() < 0.5 ? 2 : 6;
      const out = (dirAlong + side) % 8;
      const [ox2, oy2] = DIRS[out];
      const turn = (out + (random() < 0.5 ? 1 : 7)) % 8;
      lay(a + ox2, b + oy2, [[out, 1], [turn, 1 + Math.floor(random() * 2)], ...plan(turn % 2 ? (turn + (random() < 0.5 ? 1 : 7)) % 8 : turn, 3 + Math.floor(random() * 7))],
        { min: 4, parent: t.id, from: { x: px(a), y: py(b) } });
    }

    // Plan aléatoire : longues lignes droites, courtes diagonales, tours de ±45°.
    function plan(dir, total) {
      const steps = [];
      let d = dir;
      let left = total;
      while (left > 0) {
        const diag = d % 2 === 1;
        const len = Math.min(left, diag ? 1 + Math.floor(random() * 3) : 3 + Math.floor(random() * 9));
        steps.push([d, len]);
        left -= len;
        // Après une diagonale on revient à l'orthogonale voisine ; sinon tour de 45° une fois sur deux.
        if (diag) d = (d + (random() < 0.5 ? 1 : 7)) % 8;
        else if (random() < 0.55) d = (d + (random() < 0.5 ? 1 : 7)) % 8;
      }
      return steps;
    }

    // 1. Broches du processeur : des pistes rayonnent du pourtour du cœur.
    const BUS = 24;
    for (let n = 0; n < BUS; n++) {
      const ang = (n / BUS) * Math.PI * 2 + (random() - 0.5) * 0.12;
      const rr = clearR + PITCH * 0.9;
      const i = Math.round((cx + Math.cos(ang) * rr - ox) / PITCH);
      const j = Math.round((cy + Math.sin(ang) * rr - oy) / PITCH);
      const dir8 = ((Math.round(ang / (Math.PI / 4)) % 8) + 8) % 8;
      // Départ dans l'axe de la broche, puis virage vers l'horizontale ou la verticale la plus proche.
      const first = [dir8, 2 + Math.floor(random() * 2)];
      const ortho = dir8 % 2 === 0 ? dir8 : (random() < 0.5 ? (dir8 + 1) % 8 : (dir8 + 7) % 8);
      const t = lay(i, j, [first, ...plan(ortho, 8 + Math.floor(random() * 18))], { bus: true, min: 4 });
      if (t) t.heartEnd = 0; // l'extrémité 0 touche le cœur
    }

    // 2. Empreintes de puces vers les bords, avec quelques pistes qui partent de leurs broches.
    for (let tries = 0; tries < 40 && chips.length < 5; tries++) {
      const w = 4 + Math.floor(random() * 4);
      const h = 2 + Math.floor(random() * 2);
      const edgeBias = random() < 0.5 ? random() * 0.22 : 0.78 + random() * 0.22;
      const i0 = Math.floor(edgeBias * (cols - w - 4)) + 2;
      const j0 = Math.floor(random() * (rows - h - 4)) + 2;
      let free = true;
      for (let j = j0 - 2; j <= j0 + h + 1 && free; j++) for (let i = i0 - 2; i <= i0 + w + 1 && free; i++) {
        if (!inside(i, j) || reserved[at(i, j)] || occ[at(i, j)]) free = false;
      }
      if (!free) continue;
      for (let j = j0; j <= j0 + h; j++) for (let i = i0; i <= i0 + w; i++) reserved[at(i, j)] = 1;
      chips.push({ x: px(i0) - 6, y: py(j0) - 6, w: w * PITCH + 12, h: h * PITCH + 12, pins: w });
      for (let p = 1; p < w; p += 2) {
        if (random() < 0.6) lay(i0 + p, j0 - 2, plan(6, 3 + Math.floor(random() * 8)), { min: 3 });
        if (random() < 0.6) lay(i0 + p, j0 + h + 2, plan(2, 3 + Math.floor(random() * 8)), { min: 3 });
      }
    }

    // 3. Pistes et bus parallèles disséminés (moins denses près du centre).
    for (let tries = 0; tries < 520; tries++) {
      const i = 1 + Math.floor(random() * (cols - 2));
      const j = 1 + Math.floor(random() * (rows - 2));
      const d = Math.hypot(px(i) - cx, py(j) - cy) / (minDim * 0.5);
      if (random() > Math.min(1, d * 0.9)) continue;
      const dir = Math.floor(random() * 4) * 2; // départ orthogonal
      const steps = plan(dir, 5 + Math.floor(random() * 16));
      const lanes = random() < 0.35 ? 2 + Math.floor(random() * 3) : 1;
      const [pdx, pdy] = DIRS[(dir + 2) % 8];
      for (let l = 0; l < lanes; l++) lay(i + pdx * l * 2, j + pdy * l * 2, steps, { min: 3 });
    }

    // 4. Embranchements en T : le circuit se ramifie comme une vraie carte.
    for (const t of traces.slice()) if (random() < 0.45) branchFrom(t);
    for (const t of traces) delete t.cellList;

    layout = { W, H, cols, rows, ox, oy, cx, cy, clearR, occ, heartMask, reserved, traces, vias, pads, joints, chips };
    routes.clear();
    pulses.length = 0;
    flashes.length = 0;
    return layout;
  }

  // Points d'angle seulement (les cases alignées sont fusionnées).
  function corners(pts) {
    if (pts.length < 3) return pts;
    const out = [pts[0]];
    for (let k = 1; k < pts.length - 1; k++) {
      const a = out[out.length - 1];
      const b = pts[k];
      const c = pts[k + 1];
      const d1x = Math.sign(b.x - a.x);
      const d1y = Math.sign(b.y - a.y);
      const d2x = Math.sign(c.x - b.x);
      const d2y = Math.sign(c.y - b.y);
      if (d1x !== d2x || d1y !== d2y) out.push(b);
    }
    out.push(pts[pts.length - 1]);
    return out;
  }

  function measure(pts) {
    const segs = [];
    let total = 0;
    for (let k = 1; k < pts.length; k++) {
      const d = Math.hypot(pts[k].x - pts[k - 1].x, pts[k].y - pts[k - 1].y);
      segs.push(d);
      total += d;
    }
    return { segs, total };
  }

  function pointAt(route, dist) {
    const { pts, segs } = route;
    let remain = Math.max(0, dist);
    for (let k = 0; k < segs.length; k++) {
      if (remain <= segs[k] || k === segs.length - 1) {
        const f = segs[k] === 0 ? 0 : Math.min(1, remain / segs[k]);
        return { x: pts[k].x + (pts[k + 1].x - pts[k].x) * f, y: pts[k].y + (pts[k + 1].y - pts[k].y) * f };
      }
      remain -= segs[k];
    }
    return pts[pts.length - 1];
  }

  // Tracé aux coins adoucis (arcTo) ; `upTo` : longueur dessinée (croissance d'une piste).
  function stroke(g, route, upTo, radius = 5) {
    const { pts, segs } = route;
    if (!pts || pts.length < 2) return;
    const limit = upTo === undefined ? route.total : upTo;
    g.beginPath();
    g.moveTo(pts[0].x, pts[0].y);
    let done = 0;
    for (let k = 1; k < pts.length; k++) {
      const seg = segs[k - 1];
      if (done + seg >= limit) {
        const f = seg === 0 ? 0 : (limit - done) / seg;
        g.lineTo(pts[k - 1].x + (pts[k].x - pts[k - 1].x) * f, pts[k - 1].y + (pts[k].y - pts[k - 1].y) * f);
        break;
      }
      if (k < pts.length - 1 && upTo === undefined) {
        const r = Math.min(radius, seg / 2, segs[k] / 2);
        g.arcTo(pts[k].x, pts[k].y, pts[k + 1].x, pts[k + 1].y, r);
      } else {
        g.lineTo(pts[k].x, pts[k].y);
      }
      done += seg;
    }
    g.stroke();
  }

  // ---------------------------------------------------------------------------------------------
  // Rendu figé (calque de fond)
  // ---------------------------------------------------------------------------------------------

  function bake(g, W, H, rgb) {
    const L = build(W, H);
    g.save();
    g.lineCap = 'round';
    g.lineJoin = 'round';
    for (const t of L.traces) {
      g.lineWidth = t.width || 1;
      g.strokeStyle = `rgba(${rgb}, ${t.bus ? 0.11 : t.width > 1.5 ? 0.06 : 0.075})`;
      stroke(g, t, undefined, 4);
    }
    // Jonctions en T : petite pastille pleine.
    g.fillStyle = `rgba(${rgb}, 0.14)`;
    for (const p of L.joints) {
      g.beginPath();
      g.arc(p.x, p.y, 1.9, 0, Math.PI * 2);
      g.fill();
    }
    // Vias (anneau percé) et pastilles.
    g.lineWidth = 1;
    g.strokeStyle = `rgba(${rgb}, 0.13)`;
    for (const v of L.vias) {
      g.beginPath();
      g.arc(v.x, v.y, 2.6, 0, Math.PI * 2);
      g.stroke();
    }
    g.fillStyle = `rgba(${rgb}, 0.10)`;
    for (const p of L.pads) g.fillRect(p.x - 2.5, p.y - 2.5, 5, 5);
    // Puces : boîtier, encoche, broches.
    for (const c of L.chips) {
      g.fillStyle = `rgba(${rgb}, 0.025)`;
      g.fillRect(c.x, c.y, c.w, c.h);
      g.strokeStyle = `rgba(${rgb}, 0.10)`;
      g.strokeRect(c.x + 0.5, c.y + 0.5, c.w - 1, c.h - 1);
      g.beginPath();
      g.arc(c.x + 8, c.y + 8, 2.5, 0, Math.PI * 2);
      g.stroke();
      g.strokeStyle = `rgba(${rgb}, 0.09)`;
      g.beginPath();
      for (let p = 0; p <= c.pins; p++) {
        const x = c.x + 6 + p * PITCH;
        g.moveTo(x, c.y);
        g.lineTo(x, c.y - 5);
        g.moveTo(x, c.y + c.h);
        g.lineTo(x, c.y + c.h + 5);
      }
      g.stroke();
    }
    // Anneau de socle autour du cœur (le processeur est posé dessus).
    g.strokeStyle = `rgba(${rgb}, 0.08)`;
    g.setLineDash([2, 5]);
    g.beginPath();
    g.arc(L.cx, L.cy, L.clearR + PITCH * 0.35, 0, Math.PI * 2);
    g.stroke();
    g.setLineDash([]);
    g.restore();
  }

  // ---------------------------------------------------------------------------------------------
  // Routage (A* sur la grille, tours de 45° au plus 90°, préférence pour les pistes existantes)
  // ---------------------------------------------------------------------------------------------

  function cellOf(L, x, y) {
    const i = Math.max(0, Math.min(L.cols - 1, Math.round((x - L.ox) / PITCH)));
    const j = Math.max(0, Math.min(L.rows - 1, Math.round((y - L.oy) / PITCH)));
    return [i, j];
  }

  function obstacleMask(L, boxes, margin) {
    const m = new Uint8Array(L.cols * L.rows);
    for (const b of boxes || []) {
      if (!b) continue;
      const i0 = Math.max(0, Math.floor((b.x - margin - L.ox) / PITCH));
      const i1 = Math.min(L.cols - 1, Math.ceil((b.x + b.w + margin - L.ox) / PITCH));
      const j0 = Math.max(0, Math.floor((b.y - margin - L.oy) / PITCH));
      const j1 = Math.min(L.rows - 1, Math.ceil((b.y + b.h + margin - L.oy) / PITCH));
      for (let j = j0; j <= j1; j++) for (let i = i0; i <= i1; i++) m[j * L.cols + i] = 1;
    }
    return m;
  }

  // Tas binaire minimal (priorité = f).
  function Heap() {
    const keys = [];
    const pri = [];
    return {
      get size() { return keys.length; },
      push(k, p) {
        keys.push(k); pri.push(p);
        let n = keys.length - 1;
        while (n > 0) {
          const parent = (n - 1) >> 1;
          if (pri[parent] <= pri[n]) break;
          [keys[parent], keys[n]] = [keys[n], keys[parent]];
          [pri[parent], pri[n]] = [pri[n], pri[parent]];
          n = parent;
        }
      },
      pop() {
        const top = keys[0];
        const lk = keys.pop();
        const lp = pri.pop();
        if (keys.length) {
          keys[0] = lk; pri[0] = lp;
          let n = 0;
          for (;;) {
            const l = 2 * n + 1;
            const r = l + 1;
            let s = n;
            if (l < keys.length && pri[l] < pri[s]) s = l;
            if (r < keys.length && pri[r] < pri[s]) s = r;
            if (s === n) break;
            [keys[s], keys[n]] = [keys[n], keys[s]];
            [pri[s], pri[n]] = [pri[n], pri[s]];
            n = s;
          }
        }
        return top;
      }
    };
  }

  // Chemin de cases entre deux cases ; null si aucun (ou trop long à trouver).
  function astar(L, start, goal, blocked) {
    const { cols, rows } = L;
    const N = cols * rows;
    const S = N * 9;
    const gScore = new Float32Array(S).fill(Infinity);
    const came = new Int32Array(S).fill(-1);
    const startK = start[1] * cols + start[0];
    const goalK = goal[1] * cols + goal[0];
    const h = (k) => {
      const dx = Math.abs((k % cols) - goal[0]);
      const dy = Math.abs(Math.floor(k / cols) - goal[1]);
      return (Math.max(dx, dy) + (Math.SQRT2 - 1) * Math.min(dx, dy)) * TRACE_COST;
    };
    const open = Heap();
    const s0 = startK * 9 + 8;
    gScore[s0] = 0;
    open.push(s0, h(startK));
    let expanded = 0;
    while (open.size && expanded < 40000) {
      const s = open.pop();
      expanded++;
      const k = Math.floor(s / 9);
      const pd = s % 9;
      if (k === goalK) {
        const path = [];
        for (let c = s; c !== -1; c = came[c]) {
          const kk = Math.floor(c / 9);
          path.push([kk % cols, Math.floor(kk / cols)]);
        }
        return path.reverse();
      }
      const ci = k % cols;
      const cj = Math.floor(k / cols);
      const gs = gScore[s];
      for (let d = 0; d < 8; d++) {
        let turn = 0;
        if (pd !== 8) {
          const diff = Math.abs(d - pd);
          turn = Math.min(diff, 8 - diff);
          if (turn > 2) continue;
        }
        const [dx, dy] = DIRS[d];
        const ni = ci + dx;
        const nj = cj + dy;
        if (ni < 0 || nj < 0 || ni >= cols || nj >= rows) continue;
        const nk = nj * cols + ni;
        if (blocked[nk] && nk !== goalK) continue;
        if (dx && dy && (blocked[cj * cols + ni] || blocked[nj * cols + ci])) continue; // pas de coin coupé
        const step = (dx && dy ? Math.SQRT2 : 1) * (L.occ[nk] ? TRACE_COST : 1) + TURN_COST[turn];
        const ns = nk * 9 + d;
        const ng = gs + step;
        if (ng < gScore[ns]) {
          gScore[ns] = ng;
          came[ns] = s;
          open.push(ns, ng + h(nk));
        }
      }
    }
    return null;
  }

  function finishRoute(pts) {
    const clean = [pts[0]];
    for (let k = 1; k < pts.length; k++) {
      const p = pts[k];
      const q = clean[clean.length - 1];
      if (Math.abs(p.x - q.x) > 0.5 || Math.abs(p.y - q.y) > 0.5) clean.push(p);
    }
    const c = corners(clean);
    return { pts: c, ...measure(c), grads: {} };
  }

  function boxesSig(L, boxes) {
    let s = '';
    for (const b of boxes || []) {
      if (!b) continue;
      s += `${Math.round(b.x / PITCH)},${Math.round(b.y / PITCH)},${Math.round(b.w / PITCH)},${Math.round(b.h / PITCH)};`;
    }
    return s;
  }

  function remember(key, route) {
    routes.set(key, route);
    if (routes.size > ROUTE_CACHE_MAX) routes.delete(routes.keys().next().value);
    return route;
  }

  // Pastille de connexion au bord d'une boîte, face à un point (côté gauche/droite, sinon haut/bas).
  function padFacing(L, box, toward) {
    const midX = box.x + box.w / 2;
    const midY = box.y + box.h / 2;
    const dx = toward.x - midX;
    const dy = toward.y - midY;
    if (Math.abs(dx) * 1.1 >= Math.abs(dy) || box.h >= box.w) {
      const sx = dx < 0 ? -1 : 1;
      const [, j] = cellOf(L, midX, Math.max(box.y + 10, Math.min(box.y + box.h - 10, midY)));
      const y = Math.max(box.y + 8, Math.min(box.y + box.h - 8, L.oy + j * PITCH));
      const x = sx < 0 ? box.x : box.x + box.w;
      return { x, y, nx: sx, ny: 0 };
    }
    const sy = dy < 0 ? -1 : 1;
    const [i] = cellOf(L, Math.max(box.x + 10, Math.min(box.x + box.w - 10, midX)), midY);
    const x = Math.max(box.x + 8, Math.min(box.x + box.w - 8, L.ox + i * PITCH));
    return { x, y: sy < 0 ? box.y : box.y + box.h, nx: 0, ny: sy };
  }

  // Case juste devant une pastille (hors de la boîte et de sa marge).
  function frontCell(L, pad, margin) {
    const reach = margin + PITCH * 0.6;
    return cellOf(L, pad.x + pad.nx * reach, pad.y + pad.ny * reach);
  }

  // Cœur → card : d'une broche du cœur jusqu'à la pastille de la card, en contournant les autres cards.
  function routeFromHeart(box, obstacles) {
    const L = layout;
    if (!L || !box) return null;
    const { cx, cy } = heart.R ? heart : L;
    const pad = padFacing(L, box, { x: cx, y: cy });
    const margin = 6;
    const goal = frontCell(L, pad, margin);
    const key = `h|${goal}|${pad.nx},${pad.ny}|${Math.round(pad.y)}|${boxesSig(L, obstacles)}`;
    if (routes.has(key)) return routes.get(key); // y compris « pas de route » : on ne recalcule pas à chaque image
    const t0 = performance.now();
    const ang = Math.atan2(pad.y - cy, pad.x - cx);
    const dir8 = ((Math.round(ang / (Math.PI / 4)) % 8) + 8) % 8;
    const u = DIRS[dir8];
    const un = Math.hypot(u[0], u[1]);
    const rr = L.clearR + PITCH * 0.9;
    const start = cellOf(L, cx + (u[0] / un) * rr, cy + (u[1] / un) * rr);
    const blocked = obstacleMask(L, obstacles, margin);
    for (let k = 0; k < blocked.length; k++) if (L.heartMask[k]) blocked[k] = 1;
    blocked[start[1] * L.cols + start[0]] = 0;
    const path = astar(L, start, goal, blocked);
    lastRouteMs = performance.now() - t0;
    if (!path) return remember(key, null);
    // La piste sort de sous le cœur dans l'axe de sa broche (jamais de biais).
    const sx0 = L.ox + start[0] * PITCH;
    const sy0 = L.oy + start[1] * PITCH;
    const pts = [
      { x: sx0 - u[0] * PITCH * 2.4, y: sy0 - u[1] * PITCH * 2.4 },
      ...path.map(([i, j]) => ({ x: L.ox + i * PITCH, y: L.oy + j * PITCH })),
      { x: pad.x, y: pad.y }
    ];
    // Dernier tronçon droit jusqu'à la pastille : on aligne la dernière case sur l'axe de la pastille.
    const last = pts[pts.length - 2];
    if (pad.nx) last.y = pad.y; else last.x = pad.x;
    const route = finishRoute(pts);
    route.pad = pad;
    return remember(key, route);
  }

  // Card → widget (ou toute boîte → boîte) : de pastille à pastille, en contournant le reste.
  function routeBetween(fromBox, toBox, obstacles) {
    const L = layout;
    if (!L || !fromBox || !toBox) return null;
    const a = padFacing(L, fromBox, { x: toBox.x + toBox.w / 2, y: toBox.y + toBox.h / 2 });
    const b = padFacing(L, toBox, { x: fromBox.x + fromBox.w / 2, y: fromBox.y + fromBox.h / 2 });
    const margin = 6;
    const start = frontCell(L, a, margin);
    const goal = frontCell(L, b, margin);
    const key = `b|${start}|${goal}|${a.nx},${a.ny},${b.nx},${b.ny}|${boxesSig(L, obstacles)}`;
    if (routes.has(key)) return routes.get(key); // y compris « pas de route » : on ne recalcule pas à chaque image
    const t0 = performance.now();
    const blocked = obstacleMask(L, obstacles, margin);
    for (let k = 0; k < blocked.length; k++) if (L.heartMask[k]) blocked[k] = 1;
    blocked[start[1] * L.cols + start[0]] = 0;
    const path = astar(L, start, goal, blocked);
    lastRouteMs = performance.now() - t0;
    if (!path) return remember(key, null);
    const cells = path.map(([i, j]) => ({ x: L.ox + i * PITCH, y: L.oy + j * PITCH }));
    if (a.nx) cells[0].y = a.y; else cells[0].x = a.x;
    if (b.nx) cells[cells.length - 1].y = b.y; else cells[cells.length - 1].x = b.x;
    const route = finishRoute([{ x: a.x, y: a.y }, ...cells, { x: b.x, y: b.y }]);
    return remember(key, route);
  }

  // ---------------------------------------------------------------------------------------------
  // Courant : impulsions sur les pistes et éclairs vers les cards qui s'ouvrent
  // ---------------------------------------------------------------------------------------------

  // Point lumineux prérendu (une couleur = un sprite), dessiné en mode additif. La couleur du cœur varie en
  // continu : elle est arrondie par paliers de 16 pour ne pas recréer un sprite à chaque image.
  function sprite(color) {
    const q = String(color).split(',').map((v) => Math.min(255, Math.round((parseFloat(v) || 0) / 16) * 16));
    const rgb = q.length === 3 ? q.join(', ') : String(color);
    let s = sprites.get(rgb);
    if (s) return s;
    s = document.createElement('canvas');
    s.width = s.height = 32;
    const g = s.getContext('2d');
    const grad = g.createRadialGradient(16, 16, 0, 16, 16, 16);
    grad.addColorStop(0, 'rgba(255, 255, 255, 1)');
    grad.addColorStop(0.18, `rgba(${rgb}, 0.95)`);
    grad.addColorStop(0.45, `rgba(${rgb}, 0.28)`);
    grad.addColorStop(1, `rgba(${rgb}, 0)`);
    g.fillStyle = grad;
    g.fillRect(0, 0, 32, 32);
    if (sprites.size > 16) sprites.clear();
    sprites.set(rgb, s);
    return s;
  }

  function spawn(L, kind) {
    const buses = L.traces.filter((t) => t.bus);
    const pool = kind === 'out' || kind === 'in' ? buses : L.traces;
    if (!pool.length) return;
    const tr = pool[Math.floor(Math.random() * pool.length)];
    const inward = kind === 'in';
    const fast = kind === 'out' || kind === 'in';
    pulses.push({
      route: tr,
      d: inward ? tr.total : 0,
      dir: inward ? -1 : 1,
      v: (fast ? 150 : 70) + Math.random() * 70,
      a: 0
    });
  }

  // ctx : { enabled, mood: 'idle'|'think'|'speak'|'listen', level 0-1, music 0-1, sleep 0-1 }
  function update(dt, now, ctx) {
    const L = layout;
    if (!L) return;
    if (!ctx || !ctx.enabled) {
      pulses.length = 0;
      return;
    }
    const awake = 1 - Math.min(1, ctx.sleep || 0);
    let rate = 0.45;      // impulsions par seconde au repos
    let kind = 'any';
    if (ctx.mood === 'think') { rate = 7; kind = 'out'; }
    else if (ctx.mood === 'speak') { rate = 2 + 10 * (ctx.level || 0); kind = 'out'; }
    else if (ctx.mood === 'listen') { rate = 2 + 10 * (ctx.level || 0); kind = 'in'; }
    if (ctx.music) rate = Math.max(rate, 1 + 7 * ctx.music);
    rate *= awake;
    if (awake < 0.2) rate = 0;
    spawnDebt += rate * dt;
    while (spawnDebt >= 1 && pulses.length < MAX_PULSES) {
      spawn(L, kind);
      spawnDebt -= 1;
    }
    if (pulses.length >= MAX_PULSES) spawnDebt = 0;
    for (let k = pulses.length - 1; k >= 0; k--) {
      const p = pulses[k];
      p.d += p.dir * p.v * dt;
      p.a = Math.min(1, p.a + dt * 4);
      if (p.d < -4 || p.d > p.route.total + 4) pulses.splice(k, 1);
    }
    for (let k = flashes.length - 1; k >= 0; k--) {
      if (now - flashes[k].at > flashes[k].dur + 1400) flashes.splice(k, 1);
    }
  }

  function draw(g, now, rgb, alpha = 1) {
    if (!layout || (!pulses.length && !flashes.length)) return;
    const spr = sprite(rgb);
    g.save();
    g.globalCompositeOperation = 'lighter';
    for (const p of pulses) {
      const fadeEnds = Math.min(1, p.d / 18, (p.route.total - p.d) / 18);
      const a = 0.85 * p.a * Math.max(0, fadeEnds) * alpha;
      if (a <= 0.01) continue;
      const head = pointAt(p.route, p.d);
      const tail = pointAt(p.route, p.d - p.dir * 9);
      g.globalAlpha = a * 0.45;
      g.drawImage(spr, tail.x - 6, tail.y - 6, 12, 12);
      g.globalAlpha = a;
      g.drawImage(spr, head.x - 8, head.y - 8, 16, 16);
    }
    // Éclairs : la piste s'allume depuis le cœur jusqu'à la boîte qui s'ouvre, puis s'éteint.
    for (const f of flashes) {
      if (!f.route) continue;
      const k = Math.min(1, (now - f.at) / f.dur);
      const fade = now - f.at > f.dur ? Math.max(0, 1 - (now - f.at - f.dur) / 1400) : 1;
      const len = f.route.total * (1 - Math.pow(1 - k, 3));
      const c = f.rgb || rgb;
      g.globalAlpha = 0.55 * fade * alpha;
      g.strokeStyle = `rgba(${c}, 0.9)`;
      g.lineWidth = 1.4;
      stroke(g, f.route, len);
      if (k < 1) {
        const head = pointAt(f.route, len);
        g.globalAlpha = alpha;
        g.drawImage(sprite(c), head.x - 9, head.y - 9, 18, 18);
      }
    }
    g.restore();
  }

  // Une boîte s'ouvre (widget, card de liste) : éclair du cœur jusqu'à elle, le long du circuit.
  function flashTo(box, obstacles, rgb) {
    const route = routeFromHeart(box, obstacles);
    if (!route) return null;
    flashes.push({ route, at: performance.now(), dur: 420, rgb: rgb || null });
    if (flashes.length > 8) flashes.shift();
    return route;
  }

  J.circuit = {
    PITCH,
    build,
    bake,
    setHeart(cx, cy, R) { heart = { cx, cy, R }; },
    routeFromHeart,
    routeBetween,
    pointAt,
    stroke,
    sprite,
    update,
    draw,
    flashTo,
    layout: () => layout,
    stats: () => ({ traces: layout ? layout.traces.length : 0, pulses: pulses.length, flashes: flashes.length, routes: routes.size, lastRouteMs })
  };
})();
