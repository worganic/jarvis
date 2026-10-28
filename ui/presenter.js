// Présentation animée d'un dossier Data, à la place du cœur (« présente-moi le H160 »).
// Le noyau (orch/savoir/present.rs) envoie le script vérifié (événement « presentation ») ; ce fichier dessine les
// scènes dans un panneau qui remplace le cœur (J.heartOverride, appelé par J.drawOrb dans orb.js) et le personnage
// du cœur à côté, qui présente (cœur classique : Bobot). Enchaînement : la fenêtre « meneuse » demande chaque scène
// (méthode savoir, op scene) quand Jarvis a fini d'en dire la narration ; le noyau la diffuse à toutes les fenêtres.
// Une question posée pendant la présentation arrive en « presentation_focus » (réponse, image ou pièce à montrer).
// La scène « exploration » (dossier en 3D) ouvre l'explorateur (ui/viewer3d.js), la scène « media » l'écran médias
// (ui/dossier-media.js) : la présentation s'y arrête et reste active jusqu'à leur fermeture. Les scènes « atouts »,
// « prix », « avis », « ecoute » et « liens » viennent des profils de dossier (produit, musique, film, lieu…).
(function () {
  const J = (window.JARVIS = window.JARVIS || {});
  const bridge = window.jarvisBridge;
  if (!bridge) return;
  const call = (op, args) => Promise.resolve(bridge.call('savoir', op, args)).catch(() => null);
  const isLeader = () => (typeof bridge.isLeader === 'function' ? bridge.isLeader() : true);

  const FADE = 0.55;            // fondu entre deux scènes (s)
  const AFTER_SPEECH_MS = 900;  // silence avant la scène suivante
  const AFTER_ANSWER_MS = 7000; // après une réponse, on laisse le temps d'une autre question
  const NO_VOICE_MS = 9000;     // voix coupée : la scène suit le rythme estimé de la lecture

  let pres = null;
  // { id, slug, sujet, scenes, total, n, style, images: Map, cur, prev, openAt, closeAt,
  //   paused, question, speechSeen, lastSpeakAt, sceneAt, asked, mode }

  // --- Événements ------------------------------------------------------------------------------------
  bridge.onTaskEvent((ev) => {
    if (!ev || !ev.type || !String(ev.type).startsWith('presentation')) return;
    const now = performance.now();
    if (ev.type === 'presentation') {
      const style = ev.personnage && ev.personnage !== 'classic' ? ev.personnage : 'bobot';
      pres = {
        id: ev.id, slug: ev.slug, sujet: ev.sujet || '', scenes: Array.isArray(ev.scenes) ? ev.scenes : [], total: (ev.scenes || []).length,
        n: -1, style, images: new Map(), cur: null, prev: null, openAt: now, closeAt: 0,
        paused: false, question: '', speechSeen: false, lastSpeakAt: 0, sceneAt: now, asked: 0, mode: 'scene'
      };
      for (const f of ev.images || []) loadImage(pres, f);
      if (isLeader()) setTimeout(() => request(0), 900);
      return;
    }
    if (!pres || ev.id !== pres.id) return;
    if (ev.type === 'presentation_scene') {
      showScene(pres.scenes[ev.n], now);
      pres.n = ev.n;
      pres.total = ev.total || pres.total;
      pres.mode = 'scene';
      pres.paused = false;
      pres.question = '';
      const kind = pres.scenes[ev.n] && pres.scenes[ev.n].type;
      if (J.characterCue) J.characterCue(kind === 'piece' || kind === 'exploration' ? 'globe' : kind === 'ecoute' || kind === 'media' ? 'think' : 'show', 2600);
    } else if (ev.type === 'presentation_pause') {
      pres.paused = true;
      pres.question = ev.question || '';
      if (ev.question && J.characterCue) J.characterCue('think', 4000);
    } else if (ev.type === 'presentation_focus') {
      const s = ev.scene || {};
      if (s.image) loadImage(pres, s.image);
      showScene({ ...s, question: ev.question || '' }, now);
      pres.mode = 'answer';
      pres.paused = false;
      if (J.characterCue) J.characterCue('show', 2600);
    } else if (ev.type === 'presentation_end') {
      pres.closeAt = now;
      const id = pres.id;
      setTimeout(() => { if (pres && pres.id === id) pres = null; }, 700);
    }
  });

  function showScene(scene, now) {
    if (!pres || !scene) return;
    pres.prev = pres.cur ? { ...pres.cur, endAt: now } : null;
    pres.cur = { scene, at: now };
    pres.sceneAt = now;
    pres.speechSeen = false;
    pres.lastSpeakAt = 0;
    pres.asked = 0;
  }

  function loadImage(p, file) {
    if (!file || p.images.has(file)) return;
    const img = new Image();
    p.images.set(file, img);
    call('image', { slug: p.slug, fichier: file }).then((r) => { if (r && r.src) img.src = r.src; });
  }

  function request(n) {
    if (!pres) return;
    pres.asked = performance.now();
    call('scene', { id: pres.id, n });
  }

  // Enchaînement (fenêtre meneuse) : la narration finie, scène suivante.
  setInterval(() => {
    if (!pres || pres.closeAt || !isLeader() || pres.paused || pres.n < 0) return;
    const now = performance.now();
    if (pres.asked && now - pres.asked < 5000) return;
    const speaking = Boolean(J.voice && J.voice.speaking);
    if (speaking) { pres.speechSeen = true; pres.lastSpeakAt = now; return; }
    if (pres.cur && pres.cur.scene && ['exploration', 'media'].includes(pres.cur.scene.type)) return; // l'explorateur ou l'écran médias prend la suite
    const dit = (pres.cur && pres.cur.scene && pres.cur.scene.dit) || '';
    const wait = pres.mode === 'answer' ? AFTER_ANSWER_MS : AFTER_SPEECH_MS;
    if (pres.speechSeen ? now - pres.lastSpeakAt > wait : now - pres.sceneAt > Math.max(NO_VOICE_MS, dit.length * 65)) request(pres.n + 1);
  }, 200);

  // --- Dessin ---------------------------------------------------------------------------------------
  const rgba = (c, a) => `rgba(${c[0]}, ${c[1]}, ${c[2]}, ${a})`;
  const ease = (x) => 1 - Math.pow(1 - Math.min(1, Math.max(0, x)), 3);

  function roundRect(g, x, y, w, h, r) {
    g.beginPath();
    g.moveTo(x + r, y);
    g.arcTo(x + w, y, x + w, y + h, r);
    g.arcTo(x + w, y + h, x, y + h, r);
    g.arcTo(x, y + h, x, y, r);
    g.arcTo(x, y, x + w, y, r);
    g.closePath();
  }

  function wrap(g, text, maxW, maxLines) {
    const words = String(text || '').split(/\s+/).filter(Boolean);
    const lines = [];
    let line = '';
    for (const w of words) {
      const test = line ? `${line} ${w}` : w;
      if (g.measureText(test).width > maxW && line) {
        lines.push(line);
        line = w;
        if (lines.length === maxLines) break;
      } else line = test;
    }
    if (lines.length < maxLines && line) lines.push(line);
    if (lines.length === maxLines && words.join(' ').length > lines.join(' ').length) lines[maxLines - 1] = `${lines[maxLines - 1].replace(/[\s,;.]*\S{0,3}$/, '')}…`;
    return lines;
  }

  function text(g, str, x, y, size, color, opts = {}) {
    g.font = `${opts.weight || 400} ${Math.round(size)}px 'Segoe UI', sans-serif`;
    g.fillStyle = color;
    g.textAlign = opts.align || 'left';
    g.textBaseline = opts.baseline || 'alphabetic';
    g.fillText(str, x, y);
  }

  // Image en mode « couvrir » avec un lent travelling (Ken Burns).
  function imageCover(g, img, x, y, w, h, age, alpha) {
    if (!img || !img.complete || !img.naturalWidth) return false;
    const k = 1.06 + 0.08 * Math.min(1, age / 12);
    const s = Math.max(w / img.naturalWidth, h / img.naturalHeight) * k;
    const iw = img.naturalWidth * s;
    const ih = img.naturalHeight * s;
    const dx = (iw - w) * (0.5 + 0.35 * Math.sin(age * 0.07));
    const dy = (ih - h) * 0.5;
    g.save();
    g.globalAlpha *= alpha;
    g.drawImage(img, x - dx, y - dy, iw, ih);
    g.restore();
    return true;
  }

  function imageContain(g, img, x, y, w, h) {
    if (!img || !img.complete || !img.naturalWidth) return null;
    const s = Math.min(w / img.naturalWidth, h / img.naturalHeight);
    const iw = img.naturalWidth * s;
    const ih = img.naturalHeight * s;
    const ix = x + (w - iw) / 2;
    const iy = y + (h - ih) / 2;
    g.drawImage(img, ix, iy, iw, ih);
    return { x: ix, y: iy, w: iw, h: ih };
  }

  // Nombre affiché qui défile jusqu'à sa valeur (« 6 050 », « 287 », « 1,5 »).
  function counting(value, k) {
    const m = String(value).match(/^([^\d-]*)(-?\d[\d\s.,]*)(.*)$/);
    if (!m) return String(value);
    const digits = m[2].replace(/\s/g, '');
    const decimals = (digits.match(/,(\d+)$/) || [])[1];
    const n = parseFloat(digits.replace(/\./g, '').replace(',', '.'));
    if (!Number.isFinite(n)) return String(value);
    const v = n * ease(k);
    const shown = decimals ? v.toFixed(decimals.length).replace('.', ',') : Math.round(v).toLocaleString('fr-FR');
    return `${m[1]}${shown}${m[3]}`;
  }

  const POS = { haut: [0.5, 0.22], bas: [0.5, 0.8], avant: [0.78, 0.5], arriere: [0.22, 0.5], gauche: [0.3, 0.5], droite: [0.7, 0.5], centre: [0.5, 0.5] };

  function drawScene(g, box, scene, age, alpha, accent) {
    if (!scene) return;
    const { x, y, w, h } = box;
    const A = accent;
    const white = (a) => `rgba(230, 248, 255, ${a * alpha})`;
    const img = (f) => (f && pres ? pres.images.get(f) : null);
    const appear = (i, step = 0.22) => ease((age - 0.2 - i * step) / 0.5);
    g.save();
    g.globalAlpha *= alpha;
    g.beginPath();
    g.rect(x, y, w, h);
    g.clip();
    switch (scene.type) {
      case 'titre':
      case 'conclusion': {
        if (imageCover(g, img(scene.image), x, y, w, h, age, 0.45)) {
          const shade = g.createLinearGradient(0, y, 0, y + h);
          shade.addColorStop(0, 'rgba(2, 8, 16, 0.2)');
          shade.addColorStop(1, 'rgba(2, 8, 16, 0.85)');
          g.fillStyle = shade;
          g.fillRect(x, y, w, h);
        }
        const k = appear(0, 0);
        text(g, scene.type === 'conclusion' ? 'EN RÉSUMÉ' : 'PRÉSENTATION', x + w / 2, y + h * 0.36, h * 0.055, rgba(A, 0.9 * k), { align: 'center', weight: 600 });
        g.font = `700 ${Math.round(h * 0.13)}px 'Segoe UI', sans-serif`;
        for (const [i, line] of wrap(g, scene.titre || '', w * 0.86, 2).entries()) {
          text(g, line, x + w / 2, y + h * (0.52 + i * 0.14) + (1 - k) * 20, h * 0.13, white(k), { align: 'center', weight: 700 });
        }
        if (scene.sousTitre) text(g, String(scene.sousTitre).toUpperCase(), x + w / 2, y + h * 0.78, h * 0.05, rgba(A, 0.8 * appear(1)), { align: 'center', weight: 600 });
        if (scene.type === 'conclusion') text(g, 'Pose-moi une question, ou dis « stop ».', x + w / 2, y + h * 0.9, h * 0.045, white(0.7 * appear(2)), { align: 'center' });
        break;
      }
      case 'atouts': {
        // Points forts à gauche, points faibles à droite, qui apparaissent un à un.
        const cols = [['POINTS FORTS', '✔', [80, 230, 150], scene.forts || []], ['POINTS FAIBLES', '✖', [255, 150, 90], scene.faibles || []]];
        cols.forEach(([title, mark, col, items], c) => {
          const cx = x + w * (c ? 0.53 : 0.04);
          const cw = w * 0.43;
          text(g, title, cx, y + h * 0.12, h * 0.05, rgba(col, 0.95 * appear(c, 0.4)), { weight: 700 });
          let ly = y + h * 0.24;
          items.forEach((it, i) => {
            const k = appear(i + 1 + c * items.length, 0.25);
            g.font = `500 ${Math.round(h * 0.046)}px 'Segoe UI', sans-serif`;
            const lines = wrap(g, it, cw - h * 0.08, 2);
            g.fillStyle = rgba(col, 0.08 * k);
            roundRect(g, cx, ly - h * 0.05, cw, lines.length * h * 0.06 + h * 0.035, 6);
            g.fill();
            text(g, mark, cx + h * 0.02, ly, h * 0.045, rgba(col, k), { weight: 700 });
            lines.forEach((line, j) => { g.font = `500 ${Math.round(h * 0.046)}px 'Segoe UI', sans-serif`; text(g, line, cx + h * 0.08, ly + j * h * 0.06, h * 0.046, white(k), { weight: 500 }); });
            ly += lines.length * h * 0.06 + h * 0.06;
          });
        });
        break;
      }
      case 'prix': {
        const rows = scene.lignes || [];
        text(g, 'PRIX RELEVÉS', x + w * 0.06, y + h * 0.12, h * 0.05, rgba(A, 0.9), { weight: 600 });
        const max = Math.max(...rows.map((r) => Number(r.prix) || 0), 1);
        const rh = Math.min(h * 0.15, (h * 0.78) / Math.max(1, rows.length));
        rows.forEach((r, i) => {
          const k = appear(i, 0.25);
          const ry = y + h * 0.2 + i * rh;
          const bw = (w * 0.5) * ((Number(r.prix) || 0) / max) * ease((age - 0.3 - i * 0.25) / 0.9);
          g.fillStyle = rgba(i === 0 ? [80, 230, 150] : A, 0.18 * k);
          roundRect(g, x + w * 0.3, ry, Math.max(4, bw), rh * 0.62, 5);
          g.fill();
          g.font = `500 ${Math.round(rh * 0.3)}px 'Segoe UI', sans-serif`;
          text(g, wrap(g, String(r.vendeur || ''), w * 0.22, 1)[0] || '', x + w * 0.06, ry + rh * 0.42, rh * 0.3, white(0.85 * k), { weight: 500 });
          const shown = counting(String(Math.round((Number(r.prix) || 0) * 100) / 100).replace('.', ','), Math.max(0, (age - 0.3 - i * 0.25) / 1.2));
          text(g, `${shown} ${r.devise || '€'}`, x + w * 0.94, ry + rh * 0.44, rh * 0.36, i === 0 ? `rgba(120, 240, 170, ${k * alpha})` : white(k), { align: 'right', weight: 700 });
          if (i === 0 && rows.length > 1) text(g, 'LE MOINS CHER', x + w * 0.31, ry + rh * 0.86, rh * 0.18, `rgba(120, 240, 170, ${0.9 * k * alpha})`, { weight: 700 });
        });
        break;
      }
      case 'avis': {
        const rows = scene.lignes || [];
        const cw = (w * 0.9) / Math.max(1, rows.length);
        rows.forEach((r, i) => {
          const k = appear(i, 0.35);
          const cx = x + w * 0.05 + i * cw;
          g.fillStyle = rgba(A, 0.07 * k);
          g.strokeStyle = rgba(A, 0.35 * k);
          roundRect(g, cx + 6, y + h * 0.1, cw - 12, h * 0.8, 10);
          g.fill();
          g.stroke();
          text(g, String(r.source || '').toUpperCase(), cx + cw / 2, y + h * 0.2, h * 0.042, rgba(A, 0.9 * k), { align: 'center', weight: 700 });
          const note = r.note ? `${r.note}${r.sur ? ` / ${r.sur}` : ''}` : '';
          if (note) text(g, note, cx + cw / 2, y + h * 0.38, h * 0.12, white(k), { align: 'center', weight: 700 });
          const n = parseFloat(String(r.note || '').replace(',', '.'));
          const of = parseFloat(String(r.sur || '5').replace(',', '.')) || 5;
          if (Number.isFinite(n)) {
            const stars = Math.max(0, Math.min(5, Math.round((n / of) * 5)));
            text(g, '★★★★★'.slice(0, stars) + '☆☆☆☆☆'.slice(0, 5 - stars), cx + cw / 2, y + h * 0.48, h * 0.05, `rgba(255, 210, 90, ${k * alpha})`, { align: 'center' });
          }
          wrap(g, r.resume || '', cw - 34, 5).forEach((line, j) => {
            g.font = `400 ${Math.round(h * 0.04)}px 'Segoe UI', sans-serif`;
            text(g, line, cx + cw / 2, y + h * (0.6 + j * 0.055), h * 0.04, white(0.85 * appear(i + 1, 0.35)), { align: 'center' });
          });
        });
        break;
      }
      case 'ecoute': {
        const cover = imageContain(g, img(scene.image), x + w * 0.04, y + h * 0.12, w * 0.36, h * 0.76);
        const tx = cover ? x + w * 0.45 : x + w * 0.08;
        const k = appear(0, 0);
        text(g, '♪ À ÉCOUTER', tx, y + h * 0.24, h * 0.05, rgba(A, 0.95 * k), { weight: 700 });
        g.font = `700 ${Math.round(h * 0.09)}px 'Segoe UI', sans-serif`;
        let ty = y + h * 0.38;
        for (const line of wrap(g, scene.titre || '', w * 0.5, 2)) { text(g, line, tx, ty, h * 0.09, white(k), { weight: 700 }); g.font = `700 ${Math.round(h * 0.09)}px 'Segoe UI', sans-serif`; ty += h * 0.1; }
        const sub = [scene.artiste, scene.genre, scene.annee].filter(Boolean).join(' · ');
        if (sub) text(g, sub, tx, ty + h * 0.02, h * 0.05, white(0.8 * appear(1)), {});
        // Égaliseur animé.
        for (let i = 0; i < 9; i++) {
          const bh = h * 0.08 * (0.35 + 0.65 * Math.abs(Math.sin(age * 3.2 + i * 0.9)));
          g.fillStyle = rgba(A, 0.7 * appear(1));
          g.fillRect(tx + i * h * 0.035, y + h * 0.78 - bh, h * 0.022, bh);
        }
        text(g, 'Dis « écoute-le » pour le lancer sur Spotify', tx, y + h * 0.88, h * 0.042, white(0.7 * appear(2)), {});
        break;
      }
      case 'liens': {
        text(g, 'POUR ALLER PLUS LOIN', x + w * 0.06, y + h * 0.12, h * 0.05, rgba(A, 0.9), { weight: 600 });
        const host = (u) => String(u || '').replace(/^https?:\/\/(?:www\.)?/, '').split('/')[0];
        (scene.liens || []).slice(0, 6).forEach((l, i) => {
          const k = appear(i, 0.18);
          const ly = y + h * (0.24 + i * 0.1);
          g.font = `500 ${Math.round(h * 0.045)}px 'Segoe UI', sans-serif`;
          text(g, `↗ ${wrap(g, l.titre || l.url, w * 0.6, 1)[0] || ''}`, x + w * 0.06, ly, h * 0.045, white(k), { weight: 500 });
          text(g, host(l.url), x + w * 0.94, ly, h * 0.036, rgba(A, 0.75 * k), { align: 'right' });
        });
        let cx = x + w * 0.06;
        (scene.boutons || []).forEach((b, i) => {
          const label = `${b.icone || '↗'} ${b.libelle}`;
          const k = appear(i + 6, 0.15);
          g.font = `600 ${Math.round(h * 0.04)}px 'Segoe UI', sans-serif`;
          const cw = g.measureText(label).width + h * 0.06;
          if (cx + cw > x + w * 0.95) return;
          g.fillStyle = rgba(A, 0.14 * k);
          roundRect(g, cx, y + h * 0.86, cw, h * 0.08, h * 0.04);
          g.fill();
          text(g, label, cx + h * 0.03, y + h * 0.915, h * 0.04, white(k), { weight: 600 });
          cx += cw + h * 0.02;
        });
        break;
      }
      case 'media':
      case 'exploration': {
        // Visible derrière l'explorateur, et dans les autres fenêtres.
        imageCover(g, img(scene.image), x, y, w, h, age, 0.3);
        const k = appear(0, 0);
        const spin = age * 1.6;
        g.strokeStyle = rgba(A, 0.8 * k);
        g.lineWidth = 2;
        for (let i = 0; i < 3; i++) {
          g.beginPath();
          g.ellipse(x + w / 2, y + h * 0.4, h * 0.16, h * 0.16 * Math.abs(Math.cos(spin + i * 1.05)), i * 1.05, 0, Math.PI * 2);
          g.stroke();
        }
        const media = scene.type === 'media';
        text(g, media ? 'ÉCOUTE ET VIDÉOS' : 'EXPLORATION 3D', x + w / 2, y + h * 0.72, h * 0.06, rgba(A, 0.95 * k), { align: 'center', weight: 600 });
        text(g, media ? 'Choisis un titre, une vidéo ou un lien' : 'Tourne autour, clique une pièce, dis « fais-le fonctionner »', x + w / 2, y + h * 0.83, h * 0.045, white(0.75 * appear(1)), { align: 'center' });
        break;
      }
      case 'image': {
        imageCover(g, img(scene.image), x, y, w, h, age, 1);
        if (scene.legende) {
          g.fillStyle = `rgba(2, 8, 16, ${0.7 * appear(1)})`;
          g.fillRect(x, y + h * 0.82, w, h * 0.18);
          g.font = `500 ${Math.round(h * 0.055)}px 'Segoe UI', sans-serif`;
          const line = wrap(g, scene.legende, w * 0.9, 1)[0] || '';
          text(g, line, x + w * 0.05, y + h * 0.93, h * 0.055, white(appear(1)), { weight: 500 });
        }
        break;
      }
      case 'chiffres': {
        const rows = scene.lignes || [];
        const n = rows.length || 1;
        const cols = n <= 2 ? n : 2;
        const lines = Math.ceil(n / cols);
        const tw = (w * 0.9) / cols;
        const th = (h * 0.8) / lines;
        rows.forEach((r, i) => {
          const k = appear(i, 0.3);
          const cx = x + w * 0.05 + (i % cols) * tw;
          const cy = y + h * 0.1 + Math.floor(i / cols) * th;
          g.fillStyle = rgba(A, 0.08 * k);
          g.strokeStyle = rgba(A, 0.45 * k);
          roundRect(g, cx + 6, cy + 6, tw - 12, th - 12, 8);
          g.fill();
          g.stroke();
          const value = counting(r.valeur, Math.max(0, (age - 0.2 - i * 0.3) / 1.4));
          g.font = `700 ${Math.round(th * 0.32)}px 'Segoe UI', sans-serif`;
          const unit = r.unite ? ` ${r.unite}` : '';
          text(g, `${value}${unit}`, cx + tw / 2, cy + th * 0.52, Math.min(th * 0.32, (tw * 1.6) / Math.max(4, `${value}${unit}`.length)), white(k), { align: 'center', weight: 700 });
          g.font = `500 ${Math.round(th * 0.12)}px 'Segoe UI', sans-serif`;
          text(g, wrap(g, String(r.cle).toUpperCase(), tw * 0.85, 1)[0] || '', cx + tw / 2, cy + th * 0.75, th * 0.12, rgba(A, 0.9 * k), { align: 'center', weight: 600 });
        });
        break;
      }
      case 'fiche': {
        const rows = scene.lignes || [];
        const rh = Math.min(h * 0.11, (h * 0.86) / Math.max(1, rows.length));
        text(g, 'FICHE TECHNIQUE', x + w * 0.06, y + h * 0.1, h * 0.05, rgba(A, 0.9), { weight: 600 });
        rows.forEach((r, i) => {
          const k = appear(i, 0.18);
          const ry = y + h * 0.16 + i * rh + (1 - k) * 12;
          g.fillStyle = rgba(A, 0.06 * k * (i % 2 ? 0.5 : 1));
          g.fillRect(x + w * 0.05, ry, w * 0.9, rh * 0.9);
          g.font = `400 ${Math.round(rh * 0.42)}px 'Segoe UI', sans-serif`;
          text(g, wrap(g, r.cle, w * 0.42, 1)[0] || '', x + w * 0.08, ry + rh * 0.6, rh * 0.42, `rgba(200, 235, 245, ${0.75 * k * alpha})`);
          g.font = `700 ${Math.round(rh * 0.45)}px 'Segoe UI', sans-serif`;
          text(g, wrap(g, [r.valeur, r.unite].filter(Boolean).join(' '), w * 0.42, 1)[0] || '', x + w * 0.92, ry + rh * 0.6, rh * 0.45, white(k), { align: 'right', weight: 700 });
        });
        break;
      }
      case 'chronologie': {
        const items = scene.items || [];
        const ly = y + h * 0.45;
        const k0 = ease((age - 0.1) / 1.2);
        g.strokeStyle = rgba(A, 0.7);
        g.lineWidth = 2;
        g.beginPath();
        g.moveTo(x + w * 0.06, ly);
        g.lineTo(x + w * 0.06 + w * 0.88 * k0, ly);
        g.stroke();
        const step = (w * 0.88) / Math.max(1, items.length);
        items.forEach((it, i) => {
          const k = appear(i, 0.45);
          const px = x + w * 0.06 + step * (i + 0.5);
          g.fillStyle = rgba(A, k);
          g.beginPath();
          g.arc(px, ly, 5 + 3 * k, 0, Math.PI * 2);
          g.fill();
          text(g, it.date || '', px, ly - h * 0.08, h * 0.065, white(k), { align: 'center', weight: 700 });
          g.font = `400 ${Math.round(h * 0.042)}px 'Segoe UI', sans-serif`;
          wrap(g, it.fait || '', step * 0.92, 4).forEach((line, j) => text(g, line, px, ly + h * 0.1 + j * h * 0.055, h * 0.042, white(0.85 * k), { align: 'center' }));
        });
        break;
      }
      case 'comparaison': {
        const k = appear(0, 0);
        text(g, scene.sujet || '', x + w * 0.25, y + h * 0.17, h * 0.075, white(k), { align: 'center', weight: 700 });
        text(g, 'VS', x + w * 0.5, y + h * 0.17, h * 0.06, rgba(A, k), { align: 'center', weight: 800 });
        text(g, scene.autre || '', x + w * 0.75, y + h * 0.17, h * 0.075, `rgba(255, 190, 120, ${k * alpha})`, { align: 'center', weight: 700 });
        (scene.points || []).forEach((p, i) => {
          const kk = appear(i + 1, 0.35);
          const py = y + h * (0.32 + i * 0.16) + (1 - kk) * 10;
          g.fillStyle = rgba(A, 0.07 * kk);
          roundRect(g, x + w * 0.06, py - h * 0.06, w * 0.88, h * 0.13, 6);
          g.fill();
          g.font = `500 ${Math.round(h * 0.05)}px 'Segoe UI', sans-serif`;
          text(g, `▸ ${wrap(g, p, w * 0.8, 1)[0] || ''}`, x + w * 0.09, py + h * 0.018, h * 0.05, white(kk), { weight: 500 });
        });
        break;
      }
      case 'piece':
      case 'reponse':
      case 'video': {
        const half = scene.type === 'video' ? null : imageContain(g, img(scene.image), x + w * 0.03, y + h * 0.08, w * 0.5, h * 0.84);
        if (half && scene.type === 'piece') {
          const [fx, fy] = POS[scene.position] || POS.centre;
          const px = half.x + half.w * fx;
          const py = half.y + half.h * fy;
          const pulse = 0.5 + 0.5 * Math.sin(age * 4);
          g.strokeStyle = rgba(A, 0.9);
          g.lineWidth = 2;
          g.beginPath();
          g.arc(px, py, Math.min(half.w, half.h) * (0.1 + 0.03 * pulse), 0, Math.PI * 2);
          g.stroke();
          g.beginPath();
          g.moveTo(px, py);
          g.lineTo(x + w * 0.56, y + h * 0.3);
          g.stroke();
        }
        const tx = half || scene.type === 'video' ? x + w * 0.57 : x + w * 0.08;
        const tw = half || scene.type === 'video' ? w * 0.39 : w * 0.84;
        if (scene.type === 'video') {
          const k = appear(0, 0);
          g.fillStyle = rgba(A, 0.15 * k);
          g.beginPath();
          g.arc(x + w * 0.28, y + h * 0.5, h * 0.22, 0, Math.PI * 2);
          g.fill();
          text(g, '▶', x + w * 0.29, y + h * 0.57, h * 0.2, rgba(A, k), { align: 'center', weight: 700 });
        }
        let ty = y + h * 0.22;
        if (scene.question) {
          g.font = `italic 400 ${Math.round(h * 0.045)}px 'Segoe UI', sans-serif`;
          for (const line of wrap(g, `« ${scene.question} »`, tw, 2)) { text(g, line, tx, ty, h * 0.045, rgba(A, 0.9 * appear(0, 0)), { weight: 400 }); g.font = `italic 400 ${Math.round(h * 0.045)}px 'Segoe UI', sans-serif`; ty += h * 0.06; }
          ty += h * 0.03;
        }
        const title = scene.type === 'piece' ? scene.nom : scene.type === 'video' ? scene.titre : '';
        if (title) {
          g.font = `700 ${Math.round(h * 0.085)}px 'Segoe UI', sans-serif`;
          for (const line of wrap(g, title, tw, 2)) { text(g, line, tx, ty, h * 0.085, white(appear(0, 0)), { weight: 700 }); g.font = `700 ${Math.round(h * 0.085)}px 'Segoe UI', sans-serif`; ty += h * 0.1; }
        }
        const body = scene.type === 'piece' ? scene.description : scene.type === 'video' ? String(scene.url || '').replace(/^https?:\/\/(?:www\.)?/, '').split('/')[0] : scene.dit;
        const bs = scene.type === 'reponse' && !half ? 0.07 : 0.05; // réponse sans image : en grand
        g.font = `400 ${Math.round(h * bs)}px 'Segoe UI', sans-serif`;
        for (const line of wrap(g, body || '', tw, 7)) { text(g, line, tx, ty, h * bs, white(0.88 * appear(1)), { weight: 400 }); g.font = `400 ${Math.round(h * bs)}px 'Segoe UI', sans-serif`; ty += h * bs * 1.4; }
        break;
      }
      default:
        break;
    }
    g.restore();
  }

  // Mise en avant : les autres cards (canvas : app.js ; HTML : classe « presenting ») passent à 10 % pendant la présentation.
  function presentK() {
    if (!pres) return 0;
    const now = performance.now();
    return ease((now - pres.openAt) / 600) * (pres.closeAt ? 1 - ease((now - pres.closeAt) / 600) : 1);
  }
  J.presentDim = () => 1 - 0.9 * presentK();
  setInterval(() => {
    const on = Boolean(pres && !pres.closeAt);
    if (document.body && document.body.classList.contains('presenting') !== on) document.body.classList.toggle('presenting', on);
  }, 150);

  J.heartOverride = {
    active: () => Boolean(pres),
    // Remplace le cœur : panneau des scènes centré sur le cœur, personnage à gauche (orb.js › J.drawOrb).
    draw(g, cx, cy, R, t, level, state, accent) {
      if (!pres) return;
      const now = performance.now();
      const A = accent || [56, 225, 255];
      const open = ease((now - pres.openAt) / 600);
      const close = pres.closeAt ? 1 - ease((now - pres.closeAt) / 600) : 1;
      const k = open * close;
      if (k <= 0.01) return;
      const W = R * 3.3 * (0.6 + 0.4 * k);
      const H = R * 2.0 * (0.6 + 0.4 * k);
      const x = cx - W / 2;
      const y = cy - H / 2;
      g.save();
      g.globalAlpha *= k;
      // Panneau holographique.
      const glow = g.createRadialGradient(cx, cy, R * 0.2, cx, cy, W * 0.7);
      glow.addColorStop(0, rgba(A, 0.12));
      glow.addColorStop(1, rgba(A, 0));
      g.fillStyle = glow;
      g.fillRect(x - W * 0.2, y - H * 0.2, W * 1.4, H * 1.4);
      roundRect(g, x, y, W, H, 14);
      g.fillStyle = 'rgba(2, 10, 20, 0.88)';
      g.fill();
      g.lineWidth = 1.5;
      g.strokeStyle = rgba(A, 0.75);
      g.stroke();
      const box = { x: x + 12, y: y + 34, w: W - 24, h: H - 58 };
      // En-tête : sujet, numéro de scène, état.
      text(g, `◈ ${pres.sujet}`.toUpperCase(), x + 16, y + 23, 12, rgba(A, 0.95), { weight: 600 });
      const status = pres.question ? 'QUESTION…' : pres.paused ? 'PAUSE' : pres.mode === 'answer' ? 'RÉPONSE' : '';
      text(g, `${status ? `${status}   ` : ''}${Math.max(0, pres.n) + 1} / ${pres.total}`, x + W - 16, y + 23, 11, rgba(A, 0.85), { align: 'right', weight: 600 });
      // Scènes : fondu de la précédente vers la nouvelle.
      if (pres.cur) {
        const age = (now - pres.cur.at) / 1000;
        const f = ease(age / FADE);
        if (pres.prev && f < 1) drawScene(g, box, pres.prev.scene, (now - pres.prev.at) / 1000, 1 - f, A);
        drawScene(g, box, pres.cur.scene, age, f, A);
      } else {
        text(g, 'Préparation de la présentation…', cx, cy, 15, rgba(A, 0.6 + 0.3 * Math.sin(t * 3)), { align: 'center', baseline: 'middle' });
      }
      // Barre de progression.
      const prog = pres.total ? (Math.max(0, pres.n) + (pres.cur ? Math.min(1, (now - pres.cur.at) / 12000) : 0)) / pres.total : 0;
      g.fillStyle = rgba(A, 0.15);
      g.fillRect(x + 14, y + H - 14, W - 28, 3);
      g.fillStyle = rgba(A, 0.85);
      g.fillRect(x + 14, y + H - 14, (W - 28) * Math.min(1, prog), 3);
      g.restore();
      // Le personnage présente, à gauche du panneau.
      if (J.character && J.character.ready(pres.style)) {
        const Rc = R * 0.55;
        const s = { ...(state || {}), heartStyle: pres.style, speaking: Boolean(J.voice && J.voice.speaking) };
        g.save();
        g.globalAlpha *= k;
        J.character.draw(g, x - Rc * 0.35, y + H - Rc * 1.05, Rc, t, level, s, A, false);
        g.restore();
      }
    }
  };
})();
