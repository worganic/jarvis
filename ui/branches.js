(function () {
  const J = (window.JARVIS = window.JARVIS || {});
  // Largeur de texte mémorisée (hud.js) ; repli sur measureText si hud.js n'est pas chargé.
  const tw = (g, text) => (J.textWidth ? J.textWidth(g, text) : g.measureText(text).width);

  // Contour des cards selon l'état de la tâche (v2) :
  //   JAUNE en cours · ROUGE tant que l'IA externe travaille dessus · VERT terminée
  //   BLEU réponse attendue (Jarvis t'écoute) · AMBRE validation par code · MAGENTA échec · GRIS annulée
  J.COLORS = {
    running: { line: '255, 205, 40', text: '#fff1b8', label: 'EN COURS' },
    ai: { line: '255, 60, 70', text: '#ffc7cb', label: 'IA SOLLICITÉE' },
    awaiting: { line: '255, 150, 30', text: '#ffd7a8', label: 'VALIDATION REQUISE' },
    waiting_reply: { line: '90, 170, 255', text: '#cfe4ff', label: 'RÉPONSE ATTENDUE' },
    done: { line: '53, 255, 158', text: '#b6ffd8', label: 'TERMINÉ' },
    error: { line: '235, 80, 210', text: '#ffc9f3', label: 'ÉCHEC' },
    cancelled: { line: '150, 160, 175', text: '#d8dde6', label: 'ANNULÉE' },
    idle: { line: '56, 225, 255', text: '#bff4ff', label: 'PRÊT' } // card sans tâche
  };

  J.COLORS.music = { line: '56, 225, 255', text: '#bff4ff', label: 'LECTURE EN COURS' };

  J.taskColor = function (task) {
    if (!task) return J.COLORS.idle;
    if (task.id === 'music-live') return J.COLORS.music; // lecture musicale : pas une tâche
    if (task.cancelled) return J.COLORS.cancelled;
    if (task.status === 'running' && task.aiActive) return J.COLORS.ai;
    return J.COLORS[task.status] || J.COLORS.running;
  };

  // Liaisons cœur → cards : couleur courante du cœur (orb.js), sinon celle de la card.
  function heartLine(fallback) {
    return (J.heartLine && J.heartLine()) || fallback;
  }

  // Une tâche encore active peut être arrêtée en fermant sa card (bouton ×).
  function isActiveTask(task) {
    return Boolean(task && task.id !== 'music-live' && !task.endedAt && ['running', 'awaiting', 'waiting_reply'].includes(task.status));
  }

  // Bouton × en haut à droite d'une card ; sa zone cliquable est rangée dans owner.closeBox.
  function closeButton(g, owner, x, y, w, line, alpha, hovered) {
    const size = 14;
    const bx = x + w - size - 6;
    const by = y + 5;
    owner.closeBox = { x: bx - 3, y: by - 3, w: size + 6, h: size + 6 };
    g.save();
    g.globalAlpha = alpha;
    if (hovered) {
      g.fillStyle = `rgba(${line}, 0.22)`;
      g.fillRect(bx - 2, by - 2, size + 4, size + 4);
    }
    g.strokeStyle = `rgba(${line}, ${hovered ? 1 : 0.75})`;
    g.lineWidth = hovered ? 1.8 : 1.3;
    g.beginPath();
    g.moveTo(bx + 3, by + 3);
    g.lineTo(bx + size - 3, by + size - 3);
    g.moveTo(bx + size - 3, by + 3);
    g.lineTo(bx + 3, by + size - 3);
    g.stroke();
    g.restore();
  }

  // Point lumineux (paquet de données, impulsion) : halo par disques additifs plutôt que
  // shadowBlur, l'une des opérations canvas les plus coûteuses, recalculée à chaque image
  // pour chaque point en transit. `line` est un triplet « r, g, b ».
  function glowDot(g, x, y, radius, line, core) {
    g.save();
    g.globalCompositeOperation = 'lighter';
    g.fillStyle = `rgba(${line}, 0.16)`;
    g.beginPath();
    g.arc(x, y, radius * 3.4, 0, Math.PI * 2);
    g.fill();
    g.fillStyle = `rgba(${line}, 0.32)`;
    g.beginPath();
    g.arc(x, y, radius * 2, 0, Math.PI * 2);
    g.fill();
    g.restore();
    g.fillStyle = core;
    g.beginPath();
    g.arc(x, y, radius, 0, Math.PI * 2);
    g.fill();
  }

  // --- Pistes du circuit (circuit.js) ------------------------------------------------------------
  // Les liaisons cœur → card et card → widget sont des pistes routées sur le circuit imprimé du fond :
  // elles contournent les cards et widgets visibles (`linkObstacles`, relevés à chaque image) et suivent les
  // pistes existantes quand c'est possible. Sans circuit, repli sur l'ancien tracé direct.
  let linkObstacles = [];
  function obstaclesExcept(...owners) {
    const out = [];
    for (const o of linkObstacles) if (!owners.includes(o.owner)) out.push(o.box);
    return out;
  }
  function asRoute(pts) {
    const { segs, total } = lengths(pts);
    return { pts, segs, total, grads: {} };
  }
  const ease = (x) => 1 - Math.pow(1 - Math.max(0, Math.min(1, x)), 3);
  function ptAt(route, d) {
    return J.circuit ? J.circuit.pointAt(route, d) : pointAt(route.pts, route.segs, d);
  }
  function strokeRoute(g, route, upTo) {
    if (J.circuit) J.circuit.stroke(g, route, upTo);
    else tracePartial(g, route.pts, route.segs, upTo === undefined ? route.total : upTo);
  }
  // Dégradé couleur du cœur → couleur de l'état, gardé avec la route (recréé seulement si une couleur change).
  function gradientFor(g, route, from, to) {
    const key = `${from}|${to}`;
    if (!route.grads) route.grads = {};
    let gr = route.grads[key];
    if (!gr) {
      const a = route.pts[0];
      const b = route.pts[route.pts.length - 1];
      gr = g.createLinearGradient(a.x, a.y, b.x, b.y);
      gr.addColorStop(0, `rgba(${from}, 1)`);
      gr.addColorStop(1, `rgba(${to}, 1)`);
      if (Object.keys(route.grads).length > 6) route.grads = {};
      route.grads[key] = gr;
    }
    return gr;
  }
  // Impulsion lumineuse (sprite prérendu de circuit.js, mode additif) ; repli sur glowDot.
  function spriteAt(g, rgb, p, size, alpha) {
    if (!p) return;
    if (!J.circuit) {
      glowDot(g, p.x, p.y, size / 5, rgb, '#ffffff');
      return;
    }
    g.save();
    g.globalCompositeOperation = 'lighter';
    g.globalAlpha *= alpha;
    g.drawImage(J.circuit.sprite(rgb), p.x - size / 2, p.y - size / 2, size, size);
    g.restore();
  }

  // Une boîte s'ouvre (widget, card de liste) : éclair du cœur jusqu'à elle le long du circuit (widgets.js).
  J.flashToBox = function (box) {
    if (!J.circuit || !box || !J.circuit.layout()) return null;
    const inside = (b) => {
      const mx = b.x + b.w / 2;
      const my = b.y + b.h / 2;
      return mx >= box.x && mx <= box.x + box.w && my >= box.y && my <= box.y + box.h;
    };
    const obstacles = linkObstacles.filter((o) => !inside(o.box)).map((o) => o.box);
    return J.circuit.flashTo(box, obstacles, heartLine((J.COLORS.idle || {}).line || '56, 225, 255'));
  };

  // Le thème colore les cards au repos et la lecture musicale ; les états d'une tâche (jaune en
  // cours, rouge IA, vert terminée…) gardent leurs couleurs fixes, quel que soit le thème.
  J.setThemeColors = function (themeId) {
    const theme = (J.THEMES && J.THEMES[themeId]) || { primaryRgb: '56, 225, 255', secondaryHex: '#bff4ff' };
    J.COLORS.idle = { line: theme.primaryRgb, text: theme.secondaryHex || '#bff4ff', label: 'PRÊT' };
    J.COLORS.music = { line: theme.primaryRgb, text: theme.secondaryHex || '#bff4ff', label: 'LECTURE EN COURS' };
  };

  // Anciens emplacements fixes conservés en repli de sécurité
  J.SLOTS = [
    { a: -152, radial: 0.16, col: 0.62 },
    { a: -118, radial: 0.20, col: 0.34 },
    { a: -62, radial: 0.18, col: 0.46 },
    { a: -24, radial: 0.14, col: 0.72 },
    { a: 22, radial: 0.15, col: 0.66 },
    { a: 64, radial: 0.19, col: 0.38 },
    { a: 122, radial: 0.17, col: 0.44 },
    { a: 158, radial: 0.13, col: 0.74 }
  ];

  // Découpage et retour à la ligne intelligent pour les textes des encarts et cards
  function wrapCardText(g, text, maxW) {
    if (!text) return [];
    const paragraphs = String(text).split('\n');
    const lines = [];

    for (const para of paragraphs) {
      const words = para.trim().split(/\s+/);
      if (words.length === 0 || (words.length === 1 && words[0] === '')) continue;

      let currentLine = '';
      for (const word of words) {
        const testLine = currentLine ? `${currentLine} ${word}` : word;
        const testW = (g && g.measureText) ? g.measureText(testLine).width : testLine.length * 6.8;

        if (testW <= maxW) {
          currentLine = testLine;
        } else {
          if (currentLine) lines.push(currentLine);
          const wordW = (g && g.measureText) ? g.measureText(word).width : word.length * 6.8;
          if (wordW > maxW) {
            let sub = '';
            for (const c of word) {
              const testSub = sub + c;
              const subW = (g && g.measureText) ? g.measureText(testSub).width : testSub.length * 6.8;
              if (subW > maxW) {
                if (sub) lines.push(sub);
                sub = c;
              } else {
                sub = testSub;
              }
            }
            currentLine = sub;
          } else {
            currentLine = word;
          }
        }
      }
      if (currentLine) lines.push(currentLine);
    }
    return lines.length ? lines : [''];
  }

  // Calcul dimensionnel précis et adaptatif de l'encart
  function getPanelMetrics(task, end, sx, viewW) {
    const pad = 14;
    const maxAllowedW = Math.min(500, Math.max(260, Math.round((viewW || 1200) * 0.42)));
    const maxTextW = maxAllowedW - pad * 2 - 16;
    const labelLen = (task.label || 'TÂCHE').length * 8.5;
    const statusLen = (task.status === 'awaiting' ? 32 : (task.status === 'waiting_reply' ? 36 : 12)) * 7.5;

    let detailLines = [];
    let maxDetailLen = 0;
    if (task.detail) {
      const avgCharW = 6.8;
      const charsPerLine = Math.max(20, Math.floor(maxTextW / avgCharW));
      const totalChars = String(task.detail).length;
      const lineCount = Math.max(1, Math.ceil(totalChars / charsPerLine));
      for (let i = 0; i < lineCount; i++) {
        detailLines.push(true);
      }
      maxDetailLen = Math.min(totalChars * avgCharW, maxTextW);
    }

    const w = Math.max(220, Math.min(maxAllowedW, Math.max(labelLen, statusLen, maxDetailLen) + pad * 2 + 18));
    const detailH = detailLines.length > 0 ? (detailLines.length * 16 + 10) : 0;
    const h = Math.max(54, 24 + detailH + 18 + pad);
    const x = Math.min(Math.max(sx > 0 ? end.x + 10 : end.x - 10 - w, 14), (viewW || 1200) - 14 - w);
    const y = end.y - h / 2;
    return { w, h, x, y, cut: 10 };
  }

  // --- Placeur libre et étagé autour du cœur --------------------------------
  // Positionne harmonieusement un encart à gauche ou à droite sans collision
  J.allocateSlot = function (cx, cy, R, W, H, existingTasks, panelsState, taskData) {
    const activeBoxes = [];

    // Recueillir les boîtes englobantes des tâches déjà actives
    if (existingTasks) {
      for (const t of existingTasks) {
        if (t.slot && t.slot.box) {
          activeBoxes.push(t.slot.box);
        }
      }
    }

    // Définition des zones interdites (IHM)
    const margin = 16;
    const forbiddenZones = [
      { x: 0, y: 0, w: W, h: 125 },             // En-tête haut
      { x: 0, y: H - 110, w: W, h: 110 }         // Console bas
    ];

    if (panelsState && panelsState.showTranscript) {
      forbiddenZones.push({ x: 0, y: H - 330, w: 460, h: 330 });
    }
    if (panelsState && (panelsState.showMemory || panelsState.showStatus)) {
      forbiddenZones.push({ x: W - 460, y: H - 420, w: 460, h: 420 });
    }
    if (panelsState && panelsState.showJournal) {
      forbiddenZones.push({ x: W - 600, y: 110, w: 600, h: 470 });
    }

    function collides(box) {
      if (box.x < margin || box.x + box.w > W - margin || box.y < margin || box.y + box.h > H - margin) {
        return true;
      }
      const boxCenterX = box.x + box.w / 2;
      const boxCenterY = box.y + box.h / 2;
      const distCore = Math.hypot(boxCenterX - cx, boxCenterY - cy);
      if (distCore < R * 1.25 + Math.min(box.w, box.h) * 0.45) {
        return true;
      }
      for (const z of forbiddenZones) {
        if (box.x < z.x + z.w && box.x + box.w > z.x && box.y < z.y + z.h && box.y + box.h > z.y) {
          return true;
        }
      }
      for (const ob of activeBoxes) {
        if (box.x < ob.x + ob.w + 18 && box.x + box.w + 18 > ob.x && box.y < ob.y + ob.h + 14 && box.y + box.h + 14 > ob.y) {
          return true;
        }
      }
      return false;
    }

    // Équilibrer gauche et droite selon l'occupation
    let leftCount = 0;
    let rightCount = 0;
    for (const ob of activeBoxes) {
      if (ob.x + ob.w / 2 < cx) leftCount++;
      else rightCount++;
    }

    // Paliers verticaux logiques étagés autour du centre
    const yOffsets = [-90, 80, -180, 170, 0, -260, 250];

    // Essayer les paliers ordonnés sur le côté le moins encombré d'abord
    const preferRightFirst = rightCount <= leftCount;
    const sideOrder = preferRightFirst ? [1, -1] : [-1, 1];

    for (const sx of sideOrder) {
      // Distance horizontale propre : juste en dehors du cœur
      const colDist = R + 70;
      const endX = sx > 0
        ? Math.min(cx + colDist + 40, W - 320)
        : Math.max(cx - colDist - 40, 280);

      for (const dy of yOffsets) {
        const endY = Math.min(Math.max(cy + dy, 150), H - 150);
        const box = getPanelMetrics(taskData || {}, { x: endX, y: endY }, sx, W);
        if (!collides(box)) {
          const aDeg = (Math.atan2(endY - cy, endX - cx) * 180) / Math.PI;
          return {
            a: aDeg,
            endX,
            endY,
            sx,
            box
          };
        }
      }
    }

    // Repli de secours : balayage angulaire simple si écran très encombré
    for (let trial = 0; trial < 20; trial++) {
      const sx = Math.random() < 0.5 ? 1 : -1;
      const aDeg = sx > 0 ? -45 + Math.random() * 90 : 135 + Math.random() * 90;
      const aRad = (aDeg * Math.PI) / 180;
      const radialDist = R * 1.35 + Math.random() * 120;
      const endX = Math.min(Math.max(cx + Math.cos(aRad) * radialDist, 220), W - 220);
      const endY = Math.min(Math.max(cy + Math.sin(aRad) * radialDist, 150), H - 150);
      const box = getPanelMetrics(taskData || {}, { x: endX, y: endY }, sx, W);
      if (!collides(box)) {
        return { a: aDeg, endX, endY, sx, box };
      }
    }

    // Repli ultime
    const fallbackSx = rightCount <= leftCount ? 1 : -1;
    const fallbackEndX = fallbackSx > 0 ? cx + R + 120 : cx - R - 120;
    const fallbackBox = getPanelMetrics(taskData || {}, { x: fallbackEndX, y: cy }, fallbackSx, W);
    return { a: fallbackSx > 0 ? 0 : 180, endX: fallbackEndX, endY: cy, sx: fallbackSx, box: fallbackBox };
  };

  // Trajectoire épurée et logique reliant le cœur à la card (Chantier B2)
  // Sortie directionnelle depuis le cœur -> dégagement horizontal -> décroché 45° -> entrée horizontale
  J.branchGeometry = function (cx, cy, R, slot, W, H) {
    const endX = slot.endX;
    const endY = slot.endY;
    const sx = slot.sx !== undefined ? slot.sx : (endX >= cx ? 1 : -1);

    // 1. Point d'attache sur la périphérie du cœur, orienté précisément vers la card
    const targetAngle = Math.atan2(endY - cy, endX - cx);
    const p0 = {
      x: cx + Math.cos(targetAngle) * (R * 1.02),
      y: cy + Math.sin(targetAngle) * (R * 1.02)
    };

    const pEnd = { x: endX, y: endY };
    const dy = endY - p0.y;
    const absDy = Math.abs(dy);
    const absDx = Math.abs(endX - p0.x);

    // 2. Si la card est quasiment à la même hauteur, liaison directe horizontale
    if (absDy < 8) {
      return { pts: [p0, pEnd], sx, W };
    }

    // 3. Dégagement horizontal net depuis le pourtour du cœur
    const leadOut = Math.min(26, absDx * 0.2);
    const p1 = { x: p0.x + sx * leadOut, y: p0.y };
    const remX = Math.abs(endX - p1.x);

    // 4. Décroché à 45° propre rejoignant la hauteur de la card
    if (remX > absDy + 20) {
      // Espace suffisant : départ horizontal -> coude 45° -> segment horizontal d'entrée
      const midX = p1.x + sx * Math.max(10, (remX - absDy) * 0.4);
      const p2 = { x: midX, y: p0.y };
      const p3 = { x: midX + sx * absDy, y: endY };
      return { pts: [p0, p1, p2, p3, pEnd], sx, W };
    } else {
      // Espace compact : transition directe
      const midX = p1.x + sx * (remX * 0.45);
      const p2 = { x: midX, y: endY };
      return { pts: [p0, p1, p2, pEnd], sx, W };
    }
  };

  function lengths(pts) {
    const segs = [];
    let total = 0;
    for (let i = 1; i < pts.length; i++) {
      const d = Math.hypot(pts[i].x - pts[i - 1].x, pts[i].y - pts[i - 1].y);
      segs.push(d);
      total += d;
    }
    return { segs, total };
  }

  function pointAt(pts, segs, dist) {
    let remain = dist;
    for (let i = 0; i < segs.length; i++) {
      if (remain <= segs[i] || i === segs.length - 1) {
        const f = segs[i] === 0 ? 0 : Math.min(1, remain / segs[i]);
        return {
          x: pts[i].x + (pts[i + 1].x - pts[i].x) * f,
          y: pts[i].y + (pts[i + 1].y - pts[i].y) * f
        };
      }
      remain -= segs[i];
    }
    return pts[pts.length - 1];
  }

  function tracePartial(g, pts, segs, dist) {
    g.beginPath();
    g.moveTo(pts[0].x, pts[0].y);
    let remain = dist;
    for (let i = 0; i < segs.length; i++) {
      if (remain >= segs[i]) {
        g.lineTo(pts[i + 1].x, pts[i + 1].y);
        remain -= segs[i];
      } else {
        const f = segs[i] === 0 ? 0 : remain / segs[i];
        g.lineTo(pts[i].x + (pts[i + 1].x - pts[i].x) * f, pts[i].y + (pts[i + 1].y - pts[i].y) * f);
        break;
      }
    }
    g.stroke();
  }

  // Rendu de l'encart (boîte biseautée et typographie adaptative)
  function panel(g, task, end, sx, t, col, viewW, alpha) {
    const effAlpha = alpha !== undefined ? alpha : 1.0;
    if (effAlpha <= 0.005) return;

    const pad = 14;
    const maxAllowedW = Math.min(500, Math.max(260, Math.round((viewW || 1200) * 0.42)));
    const maxTextW = maxAllowedW - pad * 2 - 16;

    g.font = '600 12px Consolas, "Courier New", monospace';
    g.letterSpacing = '1.5px';
    const wLabel = (g.measureText ? tw(g, task.label.toUpperCase()) : task.label.length * 8.5);

    const statusLine =
      task.status === 'awaiting'
        ? (task.validationType === 'voice' ? 'ACCORD ORAL [OUI / NON]' : `CODE ${task.code || ''} → SAISIR POUR VALIDER`)
        : task.status === 'waiting_reply'
        ? 'RÉPONSE ATTENDUE · PARLEZ OU ÉCRIVEZ'
        : (task.background && task.status === 'running' ? `EN FOND · ${col.label}` : col.label);

    g.font = '10px Consolas, "Courier New", monospace';
    g.letterSpacing = '0.5px';
    const wStatus = (g.measureText ? tw(g, statusLine) : statusLine.length * 7.5);

    let detailLines = [];
    let wDetail = 0;
    if (task.detail) {
      g.font = '11px Consolas, "Courier New", monospace';
      g.letterSpacing = '0.5px';
      detailLines = wrapCardText(g, task.detail, maxTextW);
      for (const line of detailLines) {
        const lw = (g.measureText ? tw(g, line) : line.length * 6.8);
        wDetail = Math.max(wDetail, lw);
      }
    }

    const w = Math.max(220, Math.min(maxAllowedW, Math.max(wLabel, wStatus, wDetail) + pad * 2 + 18));
    const detailH = detailLines.length > 0 ? (detailLines.length * 16 + 10) : 0;
    const h = Math.max(54, 24 + detailH + 18 + pad);

    const x = Math.min(Math.max(sx > 0 ? end.x + 10 : end.x - 10 - w, 14), (viewW || 1200) - 14 - w);
    const y = end.y - h / 2;
    const cut = 10;

    // Enregistrement de la boîte de collision pour les interactions souris (drag & drop)
    task.panelBox = { x, y, w, h, cut };
    if (task.slot) {
      task.slot.box = { x, y, w, h };
    }

    g.save();
    g.globalAlpha = effAlpha;

    // Effet d'élévation holographique si l'encart est saisi par la souris
    if (task.isDragging) {
      g.shadowColor = `rgba(${col.line}, 0.85)`;
      g.shadowBlur = 18;
    }

    g.beginPath();
    g.moveTo(x + cut, y);
    g.lineTo(x + w, y);
    g.lineTo(x + w, y + h - cut);
    g.lineTo(x + w - cut, y + h);
    g.lineTo(x, y + h);
    g.lineTo(x, y + cut);
    g.closePath();
    g.fillStyle = task.isDragging
      ? (task.background ? 'rgba(26, 14, 48, 0.92)' : 'rgba(4, 28, 38, 0.90)')
      : (task.background ? 'rgba(16, 8, 32, 0.78)' : 'rgba(2, 16, 22, 0.72)');
    g.fill();
    g.lineWidth = task.isDragging ? 1.6 : 1;
    g.strokeStyle = task.isDragging ? `rgba(${col.line}, 0.95)` : `rgba(${col.line}, 0.55)`;
    g.stroke();
    g.shadowBlur = 0;

    const isLeftEdge = sx > 0;
    const edgeX = isLeftEdge ? x : x + w;
    const edgeY1 = isLeftEdge ? (y + cut) : (y + 1);
    const edgeY2 = isLeftEdge ? (y + h - 1) : (y + h - cut);
    g.beginPath();
    g.moveTo(edgeX, edgeY1);
    g.lineTo(edgeX, edgeY2);
    g.lineWidth = task.isDragging ? 3 : 2;
    g.strokeStyle = `rgba(${col.line}, 1.0)`;
    g.stroke();

    // Poignée de préhension holographique discrète (grip handle), à gauche du bouton ×
    const gripX = isLeftEdge ? x + w - 38 : x + 10;
    const gripY = y + 8;
    const gripAlpha = task.isDragging ? 0.9 : (task.isHovered ? 0.65 : 0.28);
    closeButton(g, task, x, y, w, col.line, 1, task.closeHover);
    g.fillStyle = `rgba(${col.line}, ${gripAlpha})`;
    for (let r = 0; r < 2; r++) {
      for (let c = 0; c < 3; c++) {
        g.fillRect(gripX + c * 3, gripY + r * 3, 1.5, 1.5);
      }
    }

    g.textAlign = 'left';
    g.textBaseline = 'middle';
    g.font = '600 12px Consolas, "Courier New", monospace';
    g.letterSpacing = '1.5px';
    g.fillStyle = col.text;
    g.fillText(task.label.toUpperCase(), x + pad, y + 18);

    if (detailLines.length > 0) {
      g.font = '11px Consolas, "Courier New", monospace';
      g.letterSpacing = '0.5px';
      g.fillStyle = task.background ? 'rgba(215, 195, 255, 0.75)' : 'rgba(190, 240, 255, 0.75)';
      const detailStartY = y + 36;
      for (let i = 0; i < detailLines.length; i++) {
        g.fillText(detailLines[i], x + pad, detailStartY + i * 16);
      }
    }

    const blink = (task.status === 'awaiting' || task.status === 'waiting_reply') ? 0.45 + 0.55 * Math.abs(Math.sin(t * 3)) : 0.75;
    g.font = '10px Consolas, "Courier New", monospace';
    g.letterSpacing = '0.5px';
    g.fillStyle = `rgba(${col.line}, ${blink})`;
    g.fillText(statusLine, x + pad, y + h - 14);
    g.letterSpacing = '0px';

    g.restore();
  }

  // --- Dessin de la branche active (avec trait reliant au cœur) ----------------
  J.drawBranch = function (g, task, geo, t, fadeAlpha) {
    const effAlpha = fadeAlpha !== undefined ? fadeAlpha : 1.0;
    if (effAlpha <= 0.005) return;

    const col = J.taskColor(task);
    const link = heartLine(col.line); // liaison cœur → card : couleur du cœur
    const { pts, sx } = geo;
    const { segs, total } = lengths(pts);
    const drawn = total * task.grow;
    if (drawn < 2) return;

    g.save();
    g.globalAlpha = effAlpha;
    g.lineCap = 'round';
    g.lineJoin = 'round';

    // Halo du tracé
    g.lineWidth = 3.5;
    g.strokeStyle = `rgba(${link}, 0.12)`;
    tracePartial(g, pts, segs, drawn);

    g.lineWidth = 1.4;
    g.strokeStyle = `rgba(${link}, ${task.status === 'awaiting' ? 0.6 + 0.4 * Math.abs(Math.sin(t * 3)) : 0.85})`;
    if (task.status === 'running') {
      g.setLineDash(task.background ? [5, 8] : [7, 9]);
      g.lineDashOffset = -t * (task.background ? 45 : 70);
    }
    tracePartial(g, pts, segs, drawn);
    g.setLineDash([]);

    // Jonctions aux coudes
    g.fillStyle = `rgba(${link}, 0.9)`;
    for (let i = 1; i < pts.length - 1; i++) {
      const p = pts[i];
      g.beginPath();
      g.arc(p.x, p.y, 2.2, 0, Math.PI * 2);
      g.fill();
    }

    // Paquets de données en circulation
    if (task.status === 'running' && task.grow > 0.99) {
      for (let i = 0; i < 2; i++) {
        const f = (t * 0.32 + i * 0.5) % 1;
        const p = pointAt(pts, segs, total * f);
        glowDot(g, p.x, p.y, 2.6, link, '#eafdff');
      }
    }

    // Encart d'action
    if (task.grow > 0.94) {
      const end = pts[pts.length - 1];
      g.fillStyle = `rgba(${link}, 0.95)`;
      g.beginPath();
      g.arc(end.x, end.y, 3.4, 0, Math.PI * 2);
      g.fill();
      panel(g, task, end, sx, t, col, geo.W, effAlpha);
    }

    g.restore();
  };

  // --- Dessin de rémanence : card ET liaison s'estompant doucement sur le fond ---
  // Crée le fond vivant et dynamique de Jarvis au fil des actions exécutées
  J.drawRemanence = function (g, trace, now) {
    const age = now - trace.depositedAt;
    const progress = age / trace.duration; // 0 à 1
    if (progress >= 1) return;

    // Opacité ténue qui décroît doucement vers 0 sur 3 à 4 minutes (et s'efface si la trace ne
    // trouve aucune place libre, voir app.js › placement sans chevauchement)
    const remAlpha = (1 - progress) * 0.22 * (trace.fade === undefined ? 1 : trace.fade);
    if (remAlpha <= 0.005) return;

    const pad = 12;
    const col = trace.color || J.COLORS.done;
    const w = trace.box.w;
    const h = trace.box.h;
    const x = trace.box.x;
    const y = trace.box.y;
    const cut = 10;
    const sx = trace.sx;

    g.save();
    g.globalAlpha = remAlpha;

    // 1. Liaison rémanente reliant le cœur à la card
    if (trace.geo && trace.geo.pts && trace.geo.pts.length > 1) {
      const pts = trace.geo.pts;
      g.beginPath();
      g.moveTo(pts[0].x, pts[0].y);
      for (let i = 1; i < pts.length; i++) {
        g.lineTo(pts[i].x, pts[i].y);
      }
      g.lineWidth = 1.0;
      g.setLineDash([4, 6]); // Pointillé fin style trace holographique résiduelle
      g.strokeStyle = `rgba(${col.line}, 0.55)`;
      g.stroke();
      g.setLineDash([]);

      // Pastilles discrètes aux coudes
      g.fillStyle = `rgba(${col.line}, 0.65)`;
      for (let i = 1; i < pts.length - 1; i++) {
        g.beginPath();
        g.arc(pts[i].x, pts[i].y, 1.8, 0, Math.PI * 2);
        g.fill();
      }
    }

    // 2. Silhouette de l'encart biseauté sans fond lourd
    g.beginPath();
    g.moveTo(x + cut, y);
    g.lineTo(x + w, y);
    g.lineTo(x + w, y + h - cut);
    g.lineTo(x + w - cut, y + h);
    g.lineTo(x, y + h);
    g.lineTo(x, y + cut);
    g.closePath();
    g.fillStyle = 'rgba(2, 16, 22, 0.4)';
    g.fill();
    g.lineWidth = 1;
    g.setLineDash([3, 4]); // Aspect trace holographique résiduelle
    g.strokeStyle = `rgba(${col.line}, 0.5)`;
    g.stroke();
    g.setLineDash([]);

    const isLeftEdge = sx > 0;
    const edgeX = isLeftEdge ? x : x + w;
    const edgeY1 = isLeftEdge ? (y + cut) : (y + 2);
    const edgeY2 = isLeftEdge ? (y + h - 2) : (y + h - cut);
    g.beginPath();
    g.moveTo(edgeX, edgeY1);
    g.lineTo(edgeX, edgeY2);
    g.lineWidth = 1.5;
    g.strokeStyle = `rgba(${col.line}, 0.65)`;
    g.stroke();

    g.textAlign = 'left';
    g.textBaseline = 'middle';
    g.font = '600 11px Consolas, "Courier New", monospace';
    g.letterSpacing = '1px';
    g.fillStyle = col.text;
    g.fillText(trace.label.toUpperCase(), x + pad, y + 14);

    if (trace.detail) {
      g.font = '10px Consolas, "Courier New", monospace';
      g.letterSpacing = '0px';
      g.fillStyle = 'rgba(190, 240, 255, 0.45)';
      g.fillText(trace.detail, x + pad, y + 31);
    }

    g.font = '9px Consolas, "Courier New", monospace';
    g.fillStyle = `rgba(${col.line}, 0.5)`;
    g.fillText(col.label, x + pad, y + h - 12);

    g.restore();
  };

  // ===============================================================================================
  // SYSTÈME DE CARDS D'ACTION PERMANENTES & LIAISONS WIDGETS (v3-14)
  // Chaque card a son type d'action, son emplacement d'origine, reste en fond transparent (idle),
  // s'active avec une impulsion lumineuse + effet d'écriture progressive (typewriter),
  // se relie à son widget correspondant, et s'estompe très lentement lors de la fermeture.
  // ===============================================================================================

  J.ACTION_CARD_DEFS = [
    { type: 'mail', category: 'COMMUNICATION', defaultLabel: 'EMAILS & GMAIL', icon: '✉', linkedWidgetKey: 'mail', dx: 110, dy: -120 },
    { type: 'music', category: 'MÉDIA AUDIO', defaultLabel: 'MUSIQUE & LECTEUR', icon: '♫', linkedWidgetKey: 'music', dx: 110, dy: 120 },
    { type: 'agenda', category: 'PLANNING', defaultLabel: 'AGENDA & DU JOUR', icon: '▦', linkedWidgetKey: 'agenda', dx: 270, dy: -230 },
    { type: 'browser', category: 'WEB & RECHERCHE', defaultLabel: 'NAVIGATEUR CHROME', icon: '◍', linkedWidgetKey: 'browser', dx: 270, dy: 230 },
    { type: 'system', category: 'SYSTÈME', defaultLabel: 'CONTRÔLE SYSTÈME', icon: '⬡', linkedWidgetKey: 'tool', dx: 150, dy: 0 },
    { type: 'assistant', category: 'INTELLIGENCE', defaultLabel: 'ASSISTANT IA', icon: '◈', linkedWidgetKey: null, dx: -390, dy: -40 }
  ];

  let actionCards = null;

  // Initialisation ou mise à jour des cards d'action permanentes
  J.initActionCards = function (cx, cy, R, W, H) {
    if (!actionCards) {
      actionCards = J.ACTION_CARD_DEFS.map((def) => {
        let savedPos = null;
        if (typeof localStorage !== 'undefined') {
          try {
            const raw = localStorage.getItem((window.JARVIS_POS_PREFIX || 'jarvis.') + 'cardPos.' + def.type);
            if (raw) savedPos = JSON.parse(raw);
          } catch {}
        }

        return {
          type: def.type,
          category: def.category,
          defaultLabel: def.defaultLabel,
          icon: def.icon,
          linkedWidgetKey: def.linkedWidgetKey,
          state: 'idle', // 'idle' | 'activating' | 'active' | 'fading'
          fadeAlpha: 0.14, // Mode fantôme transparent (0.14)
          boxAlpha: 0.14,
          secondaryAlpha: 0,
          titleAlpha: 0.14, // Textes des cards en mode fantôme (0.14)
          lastTitle: def.defaultLabel,
          pulseProgress: 0,
          widgetPulseProgress: 0,
          activatedAt: 0,
          fadingStartedAt: 0,
          activeTask: null,
          isDragging: false,
          isHovered: false,
          customPos: savedPos,
          typewriter: {
            progress: 1.0,
            activeLabel: def.defaultLabel,
            activeDetail: '',
            statusLine: 'EN ATTENTE'
          },
          panelBox: { x: 0, y: 0, w: 230, h: 48, cut: 10 }
        };
      });
    }

    // Mise à jour des positions par défaut si non déplacées manuellement
    for (const card of actionCards) {
      const def = J.ACTION_CARD_DEFS.find((d) => d.type === card.type);
      const w = 230;
      const h = (card.activeTask && card.activeTask.detail) ? 62 : 48;
      let x, y;

      if (card.customPos && Number.isFinite(card.customPos.x) && Number.isFinite(card.customPos.y)) {
        x = card.customPos.x;
        y = card.customPos.y;
      } else {
        const sx = def.dx >= 0 ? 1 : -1;
        const targetDist = R + Math.abs(def.dx);
        x = Math.max(16, Math.min(W - w - 16, sx > 0 ? cx + targetDist : cx - targetDist - w));
        y = Math.max(70, Math.min(H - h - 70, cy + def.dy));
      }

      card.panelBox = { x, y, w, h, cut: 10 };
      const cardCenterX = x + w / 2;
      const sx = cardCenterX >= cx ? 1 : -1;
      card.slot = {
        endX: sx > 0 ? x - 10 : x + w + 10,
        endY: y + h / 2,
        sx,
        box: { x, y, w, h }
      };
    }

    return actionCards;
  };

  J.getActionCards = function () {
    return actionCards || [];
  };

  // --- Logos vectoriels des sources média ---
  function drawSpotifyLogo(g, cx, cy, size) {
    g.save();
    const r = size / 2;
    g.fillStyle = '#1db954';
    g.beginPath();
    g.arc(cx, cy, r, 0, Math.PI * 2);
    g.fill();

    g.strokeStyle = '#020d06';
    g.lineCap = 'round';

    g.lineWidth = size * 0.11;
    g.beginPath();
    g.arc(cx - r * 0.05, cy + r * 0.46, r * 0.68, -Math.PI * 0.72, -Math.PI * 0.28);
    g.stroke();

    g.lineWidth = size * 0.095;
    g.beginPath();
    g.arc(cx - r * 0.04, cy + r * 0.50, r * 0.50, -Math.PI * 0.71, -Math.PI * 0.29);
    g.stroke();

    g.lineWidth = size * 0.08;
    g.beginPath();
    g.arc(cx - r * 0.03, cy + r * 0.54, r * 0.33, -Math.PI * 0.70, -Math.PI * 0.30);
    g.stroke();

    g.restore();
  }

  function drawJarvisAudioLogo(g, cx, cy, size, color = '#38e1ff') {
    g.save();
    const r = size / 2;
    g.fillStyle = 'rgba(2, 28, 42, 0.95)';
    g.strokeStyle = color;
    g.lineWidth = 1.1;
    g.beginPath();
    g.arc(cx, cy, r, 0, Math.PI * 2);
    g.fill();
    g.stroke();

    g.fillStyle = color;
    const barW = size * 0.14;
    g.fillRect(cx - barW * 1.8, cy - r * 0.4, barW, r * 0.8);
    g.fillRect(cx - barW * 0.5, cy - r * 0.7, barW, r * 1.4);
    g.fillRect(cx + barW * 0.8, cy - r * 0.3, barW, r * 0.6);
    g.restore();
  }

  function isMusicPlaying() {
    const m = J.currentMedia;
    if (m && m.active && m.status === 'playing') return true;
    if (J.musicPlayer && J.musicPlayer.isPlaying) return true;
    return false;
  }

  // Une card fantôme inutilisée depuis ce délai s'efface complètement (fondu de ~8 s) ; elle
  // revient dès qu'une tâche la réactive.
  const CARD_HIDE_AFTER_MS = 3 * 60 * 1000;
  J.cardVisible = (card) => card.visibility === undefined || card.visibility > 0.05;

  // Mise à jour continue (transparence à 90% au repos/lancement, 0% de transparence en active)
  J.updateActionCards = function (dt, now) {
    if (!actionCards) return;

    const musicPlaying = isMusicPlaying();
    // Cards ouvertes (type 2) : leur titre remplace le fantôme d'une card d'action du même nom (« MES PROJETS »).
    const openTitles = new Set();
    const wcards = J.widgets && J.widgets.cards;
    if (wcards) {
      for (const rec of wcards.values()) {
        if (rec.state !== 'active' || !rec.el || !rec.el.isConnected) continue;
        const t = rec.el.querySelector('.w-head .w-title');
        if (t && t.textContent) openTitles.add(t.textContent.trim().toUpperCase());
      }
    }

    for (const card of actionCards) {
      if (card.visibility === undefined) card.visibility = 1;
      // Tension de la piste : s'allume vite à l'ouverture, s'éteint en fondu (~1 s) après la tâche.
      const glowTarget = card.state === 'active' || card.state === 'activating' ? 1 : 0;
      const glow = card.linkGlow || 0;
      card.linkGlow = glow + (glowTarget - glow) * Math.min(1, dt * (glowTarget > glow ? 8 : 1.1));
      if (card.state === 'idle') {
        if (!card.idleSince) card.idleSince = now;
      } else {
        card.idleSince = 0;
      }
      // Fantôme devenu inutile : la vraie card du même nom est ouverte. Il s'efface vite et ne revient qu'avec une nouvelle tâche.
      if (card.state !== 'idle') card.superseded = false;
      else if (card.lastTitle && openTitles.has(String(card.lastTitle).trim().toUpperCase())) card.superseded = true;
      const hide = card.superseded || (card.idleSince && now - card.idleSince > CARD_HIDE_AFTER_MS && !card.isDragging);
      card.visibility = hide ? Math.max(0, card.visibility - dt / (card.superseded ? 0.8 : 8)) : Math.min(1, card.visibility + dt / 0.3);

      if (card.boxAlpha === undefined) card.boxAlpha = card.fadeAlpha || 0.10;
      if (card.secondaryAlpha === undefined) card.secondaryAlpha = (card.state === 'active' ? 1.0 : 0);
      if (card.titleAlpha === undefined) card.titleAlpha = (card.state === 'active' ? 1.0 : 0.10);
      if (!card.lastTitle) card.lastTitle = card.defaultLabel;

      // RÈGLE ABSOLUE : une card active comme la musique DOIT RESTER À 100% VISIBLE tant que la musique joue !
      if (card.type === 'music' && musicPlaying) {
        card.state = 'active';
        card.boxAlpha = 1.0;
        card.fadeAlpha = 1.0;
        card.titleAlpha = 1.0;
        card.secondaryAlpha = 1.0;
        card.pulseProgress = 1.0;
        card.widgetPulseProgress = 1.0;
        card.openProgress = 1.0;
        card.typewriter.progress = 1.0;

        const m = J.currentMedia;
        const rawApp = (m && m.app) ? String(m.app).toLowerCase() : '';
        let sourceName = 'SPOTIFY';
        let sourceKey = 'spotify';
        if (rawApp.includes('spotify')) {
          sourceName = 'SPOTIFY';
          sourceKey = 'spotify';
        } else if (rawApp.includes('chrome') || rawApp.includes('msedge') || rawApp.includes('firefox')) {
          sourceName = 'CHROME';
          sourceKey = 'browser';
        } else if (rawApp.includes('vlc')) {
          sourceName = 'VLC';
          sourceKey = 'vlc';
        } else if (rawApp.includes('jarvis')) {
          sourceName = 'JARVIS AUDIO';
          sourceKey = 'jarvis';
        } else if (rawApp) {
          sourceName = rawApp.split(/[_.]/)[0].toUpperCase();
          sourceKey = 'generic';
        }

        card.mediaSource = sourceName;
        card.mediaSourceKey = sourceKey;
        card.lastTitle = 'LECTURE · ' + sourceName;

        const trackTitle = (m && m.title) ? m.title : 'Musique en cours';
        const trackArtist = (m && m.artist) ? m.artist : 'Artiste';
        const trackAlbum = (m && (m.playlist || m.album)) ? (' · ' + (m.playlist || m.album)) : '';

        if (!card.activeTask || card.activeTask.id !== 'music-live') {
          card.activeTask = {
            id: 'music-live',
            label: card.lastTitle,
            detail: `${trackTitle}\n${trackArtist}${trackAlbum}`,
            status: 'running',
            background: false
          };
        } else {
          card.activeTask.label = card.lastTitle;
          card.activeTask.detail = `${trackTitle}\n${trackArtist}${trackAlbum}`;
          card.activeTask.status = 'running';
        }
        continue;
      }

      // Si la musique vient de s'arrêter, la tâche music-live se termine et entre en fading
      if (card.type === 'music' && !musicPlaying && card.activeTask && card.activeTask.id === 'music-live') {
        card.activeTask.status = 'done';
        card.activeTask = null;
        card.state = 'fading';
        card.fadingStartedAt = now;
      }

      if (card.state === 'activating') {
        // 1. L'impulsion part du cœur et suit la piste (~0,35 s) ; 2. à son arrivée sur la pastille, la card
        // se déplie depuis ce bord (~0,25 s) ; 3. puis l'impulsion continue vers le widget relié.
        card.pulseProgress = Math.min(1.0, card.pulseProgress + dt * 2.6);
        const arrived = card.pulseProgress >= 0.92;
        if (arrived) {
          card.openProgress = Math.min(1.0, (card.openProgress || 0) + dt * 4.2);
          card.boxAlpha = Math.min(1.0, card.boxAlpha + dt * 4.5);
          card.titleAlpha = Math.min(1.0, card.titleAlpha + dt * 5.0);
          card.secondaryAlpha = Math.min(1.0, card.secondaryAlpha + dt * 3.5);
        }
        card.fadeAlpha = card.boxAlpha;

        // Dès que la card est à moitié ouverte, l'impulsion Card -> Widget se déclenche
        if ((card.openProgress || 0) >= 0.5) {
          card.widgetPulseProgress = Math.min(1.0, card.widgetPulseProgress + dt * 2.8);
        }

        // Progression de l'écriture progressive (typewriter) initiale
        if ((card.openProgress || 0) >= 0.3) {
          card.typewriter.progress = Math.min(1.0, card.typewriter.progress + dt * 2.5);
        }

        if (card.boxAlpha >= 0.99 && card.pulseProgress >= 0.99 && (card.openProgress || 0) >= 0.99) {
          card.state = 'active';
          card.boxAlpha = 1.0;
          card.fadeAlpha = 1.0;
          card.titleAlpha = 1.0;
          card.secondaryAlpha = 1.0;
          card.typewriter.progress = 1.0;
        }
      } else if (card.state === 'active') {
        card.boxAlpha = 1.0;
        card.fadeAlpha = 1.0;
        card.titleAlpha = 1.0;
        card.secondaryAlpha = 1.0;
        card.typewriter.progress = 1.0;
      } else if (card.state === 'fading') {
        // 1. Les cards s'estompent doucement vers le mode fantôme (boxAlpha = 0.14) sur ~22s
        card.boxAlpha = Math.max(0.14, card.boxAlpha - dt / 22.0);
        card.fadeAlpha = card.boxAlpha;

        // 2. Ensuite les textes secondaires s'effacent en premier
        card.secondaryAlpha = Math.max(0, card.secondaryAlpha - dt / 4.0);

        // 3. Et les titres s'estompent vers 0.14 jusqu'à la nouvelle instruction
        card.titleAlpha = Math.max(0.14, card.titleAlpha - dt / 18.0);
        card.typewriter.progress = 1.0;

        if (card.boxAlpha <= 0.145 && card.secondaryAlpha <= 0.01) {
          card.state = 'idle';
          card.boxAlpha = 0.14;
          card.fadeAlpha = 0.14;
          card.secondaryAlpha = 0;
          card.titleAlpha = 0.14;
          card.pulseProgress = 0;
          card.widgetPulseProgress = 0;
        }
      } else {
        // État 'idle' : repos en mode fantôme transparent (alpha 0.14)
        card.boxAlpha = 0.14;
        card.fadeAlpha = 0.14;
        card.secondaryAlpha = 0;
        card.titleAlpha = 0.14;
        card.pulseProgress = 0;
        card.widgetPulseProgress = 0;
        card.typewriter.progress = 1.0;
      }
    }
  };

  // Tracé technologique de la liaison entre une Card d'action et son Widget HTML
  J.drawCardToWidgetLink = function (g, card, widgetBox, t, now) {
    if (!card || !widgetBox) return;

    const cb = card.panelBox || (card.slot && card.slot.box);
    const wb = widgetBox;
    if (!cb || !wb) return;

    // Détermination des points de connexion optimaux
    const cardCenterX = cb.x + cb.w / 2;
    const cardCenterY = cb.y + cb.h / 2;
    const widgetCenterX = wb.x + wb.w / 2;
    const widgetCenterY = wb.y + wb.h / 2;

    let pStart, pEnd;

    // Choix du bord de départ sur la card
    if (widgetCenterY < cb.y - 10) {
      pStart = { x: cardCenterX, y: cb.y }; // Sortie par le haut
    } else if (widgetCenterY > cb.y + cb.h + 10) {
      pStart = { x: cardCenterX, y: cb.y + cb.h }; // Sortie par le bas
    } else if (widgetCenterX > cb.x + cb.w) {
      pStart = { x: cb.x + cb.w, y: cardCenterY }; // Sortie par la droite
    } else {
      pStart = { x: cb.x, y: cardCenterY }; // Sortie par la gauche
    }

    // Choix du bord d'arrivée sur le widget
    if (cardCenterY > wb.y + wb.h + 10) {
      pEnd = { x: widgetCenterX, y: wb.y + wb.h }; // Entrée par le bas
    } else if (cardCenterY < wb.y - 10) {
      pEnd = { x: widgetCenterX, y: wb.y }; // Entrée par le haut
    } else if (cardCenterX < wb.x) {
      pEnd = { x: wb.x, y: widgetCenterY }; // Entrée par la gauche
    } else {
      pEnd = { x: wb.x + wb.w, y: widgetCenterY }; // Entrée par la droite
    }

    // Piste routée sur le circuit (circuit.js) ; repli : tracé orthogonal direct.
    let route = J.circuit ? (J.circuit.routeBetween(cb, wb, obstaclesExcept(card, `w:${card.linkedWidgetKey}`)) || J.circuit.routeBetween(cb, wb, [])) : null;
    if (!route) {
      const midX = pStart.x + (pEnd.x - pStart.x) * 0.5;
      route = asRoute([pStart, { x: midX, y: pStart.y }, { x: midX, y: pEnd.y }, pEnd]);
    }

    const col = card.activeTask ? J.taskColor(card.activeTask) : J.COLORS.idle;
    const effAlpha = card.fadeAlpha * (card.visibility === undefined ? 1 : card.visibility);
    const glow = card.linkGlow || 0;
    const a = route.pts[0];
    const b = route.pts[route.pts.length - 1];

    g.save();
    g.lineCap = 'round';
    g.lineJoin = 'round';
    // Au repos : piste fine et pleine, pastilles aux deux bouts.
    g.save();
    g.globalAlpha *= 0.5 * Math.max(0.35, Math.min(1, effAlpha * 1.6));
    g.lineWidth = 1.05;
    g.strokeStyle = `rgba(${col.line}, 0.5)`;
    strokeRoute(g, route);
    g.restore();
    g.fillStyle = `rgba(${col.line}, ${0.3 + 0.6 * glow})`;
    g.fillRect(a.x - 2.5, a.y - 2.5, 5, 5);
    g.fillRect(b.x - 2.5, b.y - 2.5, 5, 5);
    // Sous tension : la piste s'allume de la card vers le widget après l'arrivée de l'impulsion du cœur.
    const wp = card.state === 'activating' ? ease(card.widgetPulseProgress || 0) : 1;
    const lit = route.total * wp;
    if (glow > 0.01) {
      g.globalAlpha *= glow;
      g.strokeStyle = `rgba(${col.line}, 1)`;
      g.save();
      g.globalAlpha *= 0.22;
      g.lineWidth = 3.5;
      strokeRoute(g, route, wp < 1 ? lit : undefined);
      g.restore();
      g.save();
      g.globalAlpha *= 0.85;
      g.lineWidth = 1.4;
      strokeRoute(g, route, wp < 1 ? lit : undefined);
      g.restore();
    }
    g.restore();
    if (glow > 0.01 && wp > 0 && wp < 1) spriteAt(g, col.line, ptAt(route, lit), 18, glow);
    else if (glow > 0.5 && card.state === 'active') spriteAt(g, col.line, ptAt(route, route.total * ((t * 0.4) % 1)), 12, 0.8 * glow);
  };

  // Rendu complet d'une Card d'action (Liaison Cœur -> Card, Boîte adaptative, Transparence progressive, Persistance du Titre)
  // mode : 'links' (pistes seulement), 'box' (card seulement) ou rien (les deux). drawAllActionCards dessine
  // toutes les pistes avant toutes les cards : une piste ne passe jamais par-dessus une card.
  J.drawActionCard = function (g, card, cx, cy, R, t, now, widgetBox, W, H, mode) {
    if (!card) return;
    const doLinks = mode !== 'box';
    const doBox = mode !== 'links';

    const col = card.activeTask ? J.taskColor(card.activeTask) : J.COLORS.idle;
    const link = heartLine(col.line); // liaison cœur → card : couleur du cœur
    const secAlpha = (card.secondaryAlpha !== undefined ? card.secondaryAlpha : (card.state === 'active' ? 1.0 : 0));
    const boxAlpha = (card.boxAlpha !== undefined ? card.boxAlpha : (card.fadeAlpha !== undefined ? card.fadeAlpha : 0.10));
    const titleAlpha = (card.state === 'active' || boxAlpha >= 0.99) ? 1.0 : (card.titleAlpha !== undefined ? card.titleAlpha : 0.10);
    const effAlpha = boxAlpha;
    const pad = 14;
    const maxAllowedW = Math.min(500, Math.max(260, Math.round(W * 0.42)));
    const maxTextW = maxAllowedW - pad * 2 - 16;

    let w = 210;
    let h = 48;
    let typedDetailLines = [];
    let fullDetailLines = [];

    // Titre : conservé jusqu'à la nouvelle instruction (à 90% de transparence au repos, 100% opaque quand actif)
    const displayTitle = card.lastTitle || (card.activeTask && card.activeTask.label ? card.activeTask.label.toUpperCase() : card.defaultLabel);
    const progress = (card.state === 'activating') ? ((card.typewriter && typeof card.typewriter.progress === 'number') ? card.typewriter.progress : 1.0) : 1.0;
    const charCount = Math.floor(displayTitle.length * progress);
    const typedLabel = displayTitle.slice(0, charCount);

    g.font = '600 12px Consolas, "Courier New", monospace';
    g.letterSpacing = '1.5px';
    const wHeader = (g.measureText ? tw(g, `${card.icon}  ${displayTitle}`) : (displayTitle.length + 3) * 8.5);

    const statusLine = card.activeTask
      ? (card.activeTask.status === 'awaiting'
          ? (card.activeTask.validationType === 'voice' ? 'ACCORD ORAL [OUI / NON]' : `CODE ${card.activeTask.code || ''} → SAISIR`)
          : (card.activeTask.status === 'waiting_reply'
              ? 'RÉPONSE ATTENDUE · PARLEZ OU ÉCRIVEZ'
              : (card.activeTask.background && card.activeTask.status === 'running' ? `EN FOND · ${col.label}` : col.label)))
      : 'TERMINÉ';

    g.font = '10px Consolas, "Courier New", monospace';
    g.letterSpacing = '0.5px';
    const wStatus = (g.measureText ? tw(g, statusLine) : statusLine.length * 7.5);

    let wDetail = 0;
    if (card.activeTask && card.activeTask.detail) {
      const detailKey = `${card.activeTask.detail}|${Math.round(maxTextW)}`;
      if (card._cachedDetailKey !== detailKey || !card._cachedDetailLines) {
        g.font = '11px Consolas, "Courier New", monospace';
        g.letterSpacing = '0.5px';
        fullDetailLines = wrapCardText(g, card.activeTask.detail, maxTextW);
        let maxLw = 0;
        for (const line of fullDetailLines) {
          const lw = (g.measureText ? tw(g, line) : line.length * 6.8);
          maxLw = Math.max(maxLw, lw);
        }
        card._cachedDetailLines = fullDetailLines;
        card._cachedDetailW = maxLw;
        card._cachedDetailKey = detailKey;
      }
      fullDetailLines = card._cachedDetailLines;
      wDetail = card._cachedDetailW;
      typedDetailLines = fullDetailLines;
    }

    const contentW = card.activeTask ? Math.max(wHeader, wStatus, wDetail) : wHeader;
    const minCardW = (card.type === 'music' && (card.state === 'active' || isMusicPlaying())) ? 280 : (card.activeTask ? 220 : 210);
    w = Math.max(minCardW, Math.min(maxAllowedW, contentW + pad * 2 + 18));
    const detailH = (card.activeTask && fullDetailLines.length > 0) ? (fullDetailLines.length * 16 + 10) : 0;
    const statusH = card.activeTask ? 18 : 0;
    h = Math.max(48, Math.round(36 + detailH + statusH));

    // Mise à jour de la boîte panelBox et du slot selon la taille adaptative
    const cut = 10;
    let x, y;
    if (card.customPos && Number.isFinite(card.customPos.x) && Number.isFinite(card.customPos.y)) {
      x = Math.max(14, Math.min(W - w - 14, card.customPos.x));
      y = Math.max(60, Math.min(H - h - 60, card.customPos.y));
    } else {
      const def = J.ACTION_CARD_DEFS.find((d) => d.type === card.type) || { dx: 110, dy: 0 };
      const sxDef = def.dx >= 0 ? 1 : -1;
      const targetDist = R + Math.abs(def.dx);
      x = Math.max(14, Math.min(W - w - 14, sxDef > 0 ? cx + targetDist : cx - targetDist - w));
      y = Math.max(60, Math.min(H - h - 60, cy + def.dy));
    }

    // Placement sans chevauchement (app.js, settleCards) : la card glisse depuis sa position
    // d'origine (homeBox) vers une place libre quand elle est sous un élément plus visible.
    card.homeBox = { x, y, w, h };
    if (card.layoutX || card.layoutY) {
      x = Math.max(14, Math.min(W - w - 14, x + (card.layoutX || 0)));
      y = Math.max(60, Math.min(H - h - 60, y + (card.layoutY || 0)));
    }

    card.panelBox = { x, y, w, h, cut };
    const cardCenterX = x + w / 2;
    const sx = cardCenterX >= cx ? 1 : -1;
    card.slot = {
      endX: sx > 0 ? x - 10 : x + w + 10,
      endY: y + h / 2,
      sx,
      box: { x, y, w, h }
    };

    // 1. Liaison Cœur -> Card : une piste du circuit, de la broche du cœur jusqu'à la pastille de la card.
    if (doLinks) {
      let route = null;
      if (J.circuit) route = J.circuit.routeFromHeart(card.panelBox, obstaclesExcept(card)) || J.circuit.routeFromHeart(card.panelBox, []);
      if (!route) route = asRoute(J.branchGeometry(cx, cy, R, card.slot, W, H).pts);
      card._route = route;
      const total = route.total;
      const glow = card.linkGlow || 0;
      const opening = card.state === 'activating';
      const travel = opening ? ease((card.pulseProgress || 0) / 0.92) : 1;
      const lit = total * travel;
      const end = route.pts[route.pts.length - 1];

      g.save();
      g.lineCap = 'round';
      g.lineJoin = 'round';
      // Piste au repos : fine et pleine, dans la couleur du cœur (elle fait partie du circuit).
      g.save();
      g.globalAlpha *= 0.55 * Math.max(0.35, Math.min(1, boxAlpha * 1.6));
      g.lineWidth = 1.1;
      g.strokeStyle = `rgba(${link}, 0.55)`;
      strokeRoute(g, route);
      g.restore();
      // Pastille de connexion au bord de la card.
      g.fillStyle = `rgba(${glow > 0.3 ? col.line : link}, ${0.35 + 0.6 * glow})`;
      g.fillRect(end.x - 3, end.y - 3, 6, 6);
      // Piste sous tension : dégradé couleur du cœur → couleur de l'état ; elle s'allume depuis le cœur à
      // l'ouverture et s'éteint en fondu quand la tâche est finie.
      if (glow > 0.01) {
        const grad = gradientFor(g, route, link, col.line);
        g.globalAlpha *= glow;
        g.strokeStyle = grad;
        g.save();
        g.globalAlpha *= 0.22;
        g.lineWidth = 4;
        strokeRoute(g, route, opening ? lit : undefined);
        g.restore();
        g.save();
        g.globalAlpha *= 0.9;
        g.lineWidth = 1.5;
        strokeRoute(g, route, opening ? lit : undefined);
        g.restore();
      }
      g.restore();

      // Ouverture : l'impulsion part du cœur, suit la piste, puis éclate sur la pastille.
      if (opening && travel < 1) spriteAt(g, link, ptAt(route, lit), 20, 1);
      if (opening && travel >= 1 && (card.openProgress || 0) < 0.7) spriteAt(g, col.line, end, 32 * (1 - (card.openProgress || 0)), 0.9);
      // Tâche en cours : flux de données du cœur vers la card.
      const task = card.activeTask;
      if (card.state === 'active' && task && (isActiveTask(task) || task.id === 'music-live')) {
        for (let k = 0; k < 3; k++) {
          const f = ((t * 90) / Math.max(60, total) + k / 3) % 1;
          spriteAt(g, col.line, ptAt(route, total * f), 12, 0.85);
        }
      }
      // Fin : une impulsion revient au cœur (verte terminé, rouge échec).
      if (card.returnPulse) {
        const k = (now - card.returnPulse.at) / 650;
        if (k >= 1 || k < 0) card.returnPulse = null;
        else spriteAt(g, card.returnPulse.rgb, ptAt(route, total * (1 - ease(k))), 18, 1 - k * 0.3);
      }
      // Card -> widget : même principe.
      if (widgetBox) J.drawCardToWidgetLink(g, card, widgetBox, t, now);
    }
    if (!doBox) return;

    // 2. Boîte de la Card d'action
    g.save();
    // Ouverture depuis la pastille : la card se déplie à partir du bord relié au circuit.
    const open = card.state === 'activating' ? (card.openProgress || 0) : 1;
    if (open < 1) {
      const pad = card._route && card._route.pad;
      const e = ease(open);
      const fromLeft = pad ? pad.nx < 0 : (card.slot && card.slot.sx > 0);
      g.beginPath();
      if (pad && pad.ny) {
        if (pad.ny < 0) g.rect(x - 4, y - 4, w + 8, (h + 8) * e);
        else g.rect(x - 4, y + h + 4 - (h + 8) * e, w + 8, (h + 8) * e);
      } else if (fromLeft) g.rect(x - 4, y - 4, (w + 8) * e, h + 8);
      else g.rect(x + w + 4 - (w + 8) * e, y - 4, (w + 8) * e, h + 8);
      g.clip();
    }

    if (card.isDragging) {
      g.shadowColor = `rgba(${col.line}, 0.9)`;
      g.shadowBlur = 12;
    } else if (card.state === 'activating' && card.pulseProgress < 0.95) {
      g.shadowColor = `rgba(${col.line}, 0.4)`;
      g.shadowBlur = 6;
    }

    // Silhouette biseautée
    g.beginPath();
    g.moveTo(x + cut, y);
    g.lineTo(x + w, y);
    g.lineTo(x + w, y + h - cut);
    g.lineTo(x + w - cut, y + h);
    g.lineTo(x, y + h);
    g.lineTo(x, y + cut);
    g.closePath();

    // Fond : 100% opaque quand active (aucune transparence), transparent à 90% (alpha 0.10) au repos/lancement
    let bgA;
    if (card.isDragging || card.state === 'active' || boxAlpha >= 0.99) {
      bgA = 1.0; // Plus aucune transparence
    } else {
      bgA = 0.10 + 0.90 * Math.max(0, Math.min(1, (boxAlpha - 0.10) / 0.90));
    }
    g.fillStyle = (card.isDragging || card.state === 'active' || boxAlpha >= 0.99)
      ? 'rgba(4, 28, 38, 1.0)'
      : `rgba(2, 16, 24, ${bgA})`;
    g.fill();

    // Bordure : pleine et opaque quand active, discrète au repos
    const borderA = (card.isDragging || card.state === 'active' || boxAlpha >= 0.99)
      ? 1.0
      : (0.15 + 0.85 * Math.max(0, Math.min(1, (boxAlpha - 0.10) / 0.90)));
    g.lineWidth = card.isDragging ? 1.8 : ((card.state === 'active' || boxAlpha >= 0.99) ? 1.4 : 1.0);
    g.strokeStyle = `rgba(${col.line}, ${borderA})`;
    if (boxAlpha < 0.6) {
      g.setLineDash([3, 4]);
    }
    g.stroke();
    g.setLineDash([]);
    g.shadowBlur = 0;

    // Bord d'accent vertical
    const isLeftEdge = sx > 0;
    const edgeX = isLeftEdge ? x : x + w;
    const edgeY1 = isLeftEdge ? (y + cut) : (y + 1);
    const edgeY2 = isLeftEdge ? (y + h - 1) : (y + h - cut);
    g.beginPath();
    g.moveTo(edgeX, edgeY1);
    g.lineTo(edgeX, edgeY2);
    g.lineWidth = card.isDragging ? 3 : 2;
    g.strokeStyle = `rgba(${col.line}, ${(card.state === 'active' || boxAlpha >= 0.99) ? 1.0 : (0.15 + 0.85 * boxAlpha)})`;
    g.stroke();

    // Bouton × : seulement pendant qu'une tâche tourne dans la card (la card elle-même est permanente).
    const cardTaskActive = isActiveTask(card.activeTask);
    if (cardTaskActive) closeButton(g, card, x, y, w, col.line, Math.max(0.35, boxAlpha), card.closeHover);
    else card.closeBox = null;

    // Poignée de déplacement (grip handle)
    const gripX = isLeftEdge ? x + w - (cardTaskActive ? 38 : 18) : x + 10;
    const gripY = y + 8;
    const gripAlpha = card.isDragging ? 0.95 : (card.isHovered ? 0.75 : 0.20 * boxAlpha);
    g.fillStyle = `rgba(${col.line}, ${gripAlpha})`;
    for (let r = 0; r < 2; r++) {
      for (let c = 0; c < 3; c++) {
        g.fillRect(gripX + c * 3, gripY + r * 3, 1.5, 1.5);
      }
    }

    g.textAlign = 'left';
    g.textBaseline = 'middle';

    // 1. Titre : RESTE VISIBLE jusqu'à la nouvelle instruction (à 90% de transparence au repos, 100% opaque quand actif)
    g.font = '600 12px Consolas, "Courier New", monospace';
    g.letterSpacing = '1.5px';
    g.fillStyle = `rgba(${col.line}, ${titleAlpha})`;
    const headerY = y + (secAlpha > 0.01 ? 18 : h / 2);
    g.fillText(`${card.icon}  ${typedLabel}`, x + pad, headerY);

    // Badge de source et Logo pour la card de musique (ex: SPOTIFY, JARVIS AUDIO)
    if (card.type === 'music' && (card.state === 'active' || boxAlpha >= 0.7 || isMusicPlaying())) {
      const sourceName = card.mediaSource || 'SPOTIFY';
      const sourceKey = card.mediaSourceKey || 'spotify';
      const isSpotify = sourceKey === 'spotify';

      g.save();
      g.font = '700 9.5px Consolas, "Courier New", monospace';
      g.letterSpacing = '0.8px';
      const txtW = (g.measureText ? tw(g, sourceName) : sourceName.length * 6.2);
      const badgeW = Math.round(txtW + 28);
      const badgeH = 18;
      const isLeft = sx > 0;
      const badgeX = isLeft ? (x + w - pad - badgeW - 14) : (x + w - pad - badgeW);
      const badgeY = y + 8;

      // Fond du badge biseauté/arrondi
      g.beginPath();
      if (typeof g.roundRect === 'function') {
        g.roundRect(badgeX, badgeY, badgeW, badgeH, 3);
      } else {
        g.rect(badgeX, badgeY, badgeW, badgeH);
      }
      g.fillStyle = isSpotify ? 'rgba(8, 35, 18, 0.95)' : 'rgba(4, 24, 36, 0.95)';
      g.fill();
      g.lineWidth = 1;
      g.strokeStyle = isSpotify ? 'rgba(29, 185, 84, 0.9)' : `rgba(${col.line}, 0.75)`;
      if (isSpotify) {
        g.shadowColor = 'rgba(29, 185, 84, 0.4)';
        g.shadowBlur = 6;
      }
      g.stroke();
      g.shadowBlur = 0;

      // Logo vectoriel de la source
      const logoSize = 12;
      const logoCx = badgeX + 10;
      const logoCy = badgeY + badgeH / 2;
      if (isSpotify) {
        drawSpotifyLogo(g, logoCx, logoCy, logoSize);
      } else {
        drawJarvisAudioLogo(g, logoCx, logoCy, logoSize, col.text);
      }

      // Nom du logiciel / source
      g.textAlign = 'left';
      g.textBaseline = 'middle';
      g.fillStyle = isSpotify ? '#1db954' : col.text;
      g.fillText(sourceName, badgeX + 19, badgeY + badgeH / 2);
      g.restore();
    }

    // Curseur d'écriture holographique clignotant lors de la frappe initiale
    if (card.state === 'activating' && progress < 0.99) {
      const textW = (g.measureText ? g.measureText(`${card.icon}  ${typedLabel}`).width : (typedLabel.length + 3) * 8.5);
      if (Math.sin(t * 8) > 0) {
        g.fillStyle = `rgba(${col.line}, 0.9)`;
        g.fillText('▋', x + pad + textW + 3, headerY);
      }
    }

    // 2. Détail multi-lignes sans aucun chevauchement : s'efface en premier
    if (secAlpha > 0.01 && typedDetailLines.length > 0) {
      g.font = '11px Consolas, "Courier New", monospace';
      g.letterSpacing = '0.5px';
      g.fillStyle = `rgba(190, 240, 255, ${0.85 * secAlpha})`;
      const detailStartY = y + 36;
      for (let i = 0; i < typedDetailLines.length; i++) {
        const lineY = detailStartY + i * 16;
        g.fillText(typedDetailLines[i], x + pad, lineY);
      }
    }

    // 3. Ligne de statut : s'efface en premier
    if (secAlpha > 0.01) {
      g.font = '10px Consolas, "Courier New", monospace';
      g.letterSpacing = '0.5px';
      const blink = (card.activeTask && (card.activeTask.status === 'awaiting' || card.activeTask.status === 'waiting_reply'))
        ? 0.45 + 0.55 * Math.abs(Math.sin(t * 3))
        : 0.8;
      g.fillStyle = `rgba(${col.line}, ${blink * secAlpha})`;
      const statusY = y + h - 14;
      g.fillText(statusLine, x + pad, statusY);
    }

    g.letterSpacing = '0px';
    g.restore();
  };

  // Piste du réseau vers un élément sans card d'action (widget, panneau, console) : fine, dans la couleur du
  // cœur, avec une pastille de connexion ; une impulsion la parcourt de temps en temps (décalée par élément),
  // pour que le réseau reste vivant sans coût notable.
  const phaseOf = new Map();
  function networkPhase(owner) {
    let p = phaseOf.get(owner);
    if (p === undefined) {
      let h = 0;
      for (let i = 0; i < owner.length; i++) h = (h * 31 + owner.charCodeAt(i)) >>> 0;
      p = (h % 1000) / 1000;
      if (phaseOf.size > 200) phaseOf.clear();
      phaseOf.set(owner, p);
    }
    return p;
  }
  function drawNetworkLink(g, item, t) {
    if (!J.circuit) return;
    // Enfermé par d'autres éléments : la piste passe dessous (elles sont dessinées après), il reste relié.
    const route = J.circuit.routeFromHeart(item.box, obstaclesExcept(item.owner)) || J.circuit.routeFromHeart(item.box, []);
    if (!route) return;
    const link = heartLine((J.COLORS.idle || {}).line || '56, 225, 255');
    const end = route.pts[route.pts.length - 1];
    g.save();
    g.lineCap = 'round';
    g.lineJoin = 'round';
    g.globalAlpha *= item.strong ? 0.5 : 0.36;
    g.lineWidth = 1.1;
    g.strokeStyle = `rgba(${link}, 0.6)`;
    strokeRoute(g, route);
    g.globalAlpha = Math.min(1, g.globalAlpha * 1.6);
    g.fillStyle = `rgba(${link}, 0.7)`;
    g.fillRect(end.x - 3, end.y - 3, 6, 6);
    g.restore();
    // Une impulsion toutes les ~7 s, du cœur vers l'élément (0,9 s de trajet).
    const period = 7;
    const k = (((t / period + networkPhase(item.owner)) % 1) * period) / 0.9;
    if (k < 1) spriteAt(g, link, ptAt(route, route.total * ease(k)), 12, 0.75 * (1 - k * 0.4));
  }

  // Dessine l'ensemble des cards d'action permanentes et leurs liaisons
  J.drawAllActionCards = function (g, cx, cy, R, t, now, widgetBoxes, W, H) {
    if (!actionCards) {
      J.initActionCards(cx, cy, R, W, H);
    }

    const boxes = widgetBoxes || (J.getWidgetBoxes ? J.getWidgetBoxes() : {});
    if (J.circuit) {
      J.circuit.build(W, H); // même sans fond affiché : les liaisons restent routées sur la grille
      J.circuit.setHeart(cx, cy, R);
    }
    // Obstacles des routes : cards visibles et widgets ouverts (positions de l'image précédente).
    linkObstacles = [];
    for (const card of actionCards) {
      const vis = card.visibility === undefined ? 1 : card.visibility;
      if (vis >= 0.05 && card.panelBox) linkObstacles.push({ owner: card, box: card.panelBox });
    }
    for (const key of Object.keys(boxes)) {
      const b = boxes[key];
      if (b && (b.state === 'active' || b.state === 'fading')) linkObstacles.push({ owner: `w:${key}`, box: b });
    }
    const panels = J.networkPanels || [];
    for (const p of panels) linkObstacles.push({ owner: p.key, box: p.box });

    // Réseau : tout élément de l'interface est branché au cœur. Les widgets reliés à une card d'action le sont
    // par elle (card → widget) ; tous les autres widgets ouverts et les panneaux ont leur propre piste.
    const viaCard = new Set();
    for (const card of actionCards) {
      const vis = card.visibility === undefined ? 1 : card.visibility;
      if (card.linkedWidgetKey && vis >= 0.01) viaCard.add(card.linkedWidgetKey);
    }
    const network = panels.map((p) => ({ owner: p.key, box: p.box, strong: false }));
    for (const key of Object.keys(boxes)) {
      const b = boxes[key];
      if (b && (b.state === 'active' || b.state === 'fading') && !viaCard.has(key)) network.push({ owner: `w:${key}`, box: b, strong: b.state === 'active' });
    }
    for (const item of network) drawNetworkLink(g, item, t);

    // Toutes les pistes d'abord, puis toutes les cards : une piste ne passe jamais par-dessus une card.
    for (const mode of ['links', 'box']) {
      for (const card of actionCards) {
        const vis = card.visibility === undefined ? 1 : card.visibility;
        if (vis < 0.01) continue; // effacée après plusieurs minutes sans usage
        const widgetBox = card.linkedWidgetKey ? boxes[card.linkedWidgetKey] : null;
        g.save();
        g.globalAlpha *= vis;
        J.drawActionCard(g, card, cx, cy, R, t, now, widgetBox, W, H, mode);
        g.restore();
      }
    }
  };

  // Association intelligente d'une tâche à une Card d'action
  J.mapTaskToActionCard = function (task) {
    if (!actionCards) return null;

    const s = `${task.label || ''} ${task.detail || ''} ${task.id || ''}`.toLowerCase();
    let targetType = 'assistant';

    if (/(?:mail|gmail|email|courriel|message|inbox)/i.test(s)) {
      targetType = 'mail';
    } else if (/(?:music|musique|spotify|titre|morceau|chanson|audio|media)/i.test(s)) {
      targetType = 'music';
    } else if (/(?:agenda|planning|reunion|rendez-vous|calendrier|calendar)/i.test(s)) {
      targetType = 'agenda';
    } else if (/(?:chrome|web|navigateur|recherche|google|url|site|browser)/i.test(s)) {
      targetType = 'browser';
    } else if (/(?:system|volume|luminosite|application|fenetre|verrouiller|veille|ocr|ecran)/i.test(s)) {
      targetType = 'system';
    }

    const card = actionCards.find((c) => c.type === targetType) || actionCards.find((c) => c.state === 'idle') || actionCards[0];
    if (!card) return null;

    card.activeTask = task;
    card.activatedAt = performance.now(); // ordre d'apparition : la plus récente cède la place
    card.lastTitle = (task.label || card.defaultLabel).toUpperCase();
    card.state = 'activating';
    card.fadeAlpha = Math.max(card.fadeAlpha, 0.25);
    card.boxAlpha = 0.25;
    card.secondaryAlpha = 0;
    card.titleAlpha = 0.95;
    card.pulseProgress = 0;
    card.widgetPulseProgress = 0;
    card.openProgress = 0; // la card se dépliera à l'arrivée de l'impulsion du cœur
    card.returnPulse = null;
    card.typewriter.progress = 0;
    card.typewriter.activeLabel = task.label || card.defaultLabel;
    card.typewriter.activeDetail = task.detail || '';
    card.activatedAt = performance.now();

    return card;
  };

  // Fin d'une tâche : la card entre dans sa phase d'estompage lent
  J.finishActionCard = function (taskId, status) {
    if (!actionCards) return;

    for (const card of actionCards) {
      if (card.activeTask && (card.activeTask.id === taskId || !taskId)) {
        // RÈGLE ABSOLUE : si c'est la musique et que la musique joue, la card RESTE ACTIVE à 100% !
        if (card.type === 'music' && isMusicPlaying()) {
          continue;
        }
        if (card.activeTask) {
          card.activeTask.status = status || 'done';
          card.activeTask.endedAt = performance.now();
          // Le résultat revient au cœur le long de la piste (vert terminé, rouge/magenta échec).
          const back = J.COLORS[status === 'error' ? 'error' : 'done'] || J.COLORS.done;
          if (card.state !== 'fading') card.returnPulse = { at: performance.now(), rgb: back.line }; // une seule fois
        }
        card.state = 'fading';
        card.fadingStartedAt = performance.now();
      }
    }
  };
})();

