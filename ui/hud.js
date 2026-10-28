(function () {
  const J = (window.JARVIS = window.JARVIS || {});

  // Largeur d'un texte, mémorisée : measureText refait toute la mise en forme du texte, or le HUD
  // mesure les mêmes libellés à chaque image. La clé inclut la police et l'espacement des lettres
  // (les deux changent le résultat). À réserver aux textes stables, pas à un texte qui s'écrit.
  const widthCache = new Map();
  J.textWidth = function (g, text) {
    const key = `${g.font}|${g.letterSpacing}|${text}`;
    let w = widthCache.get(key);
    if (w === undefined) {
      w = g.measureText(text).width;
      if (widthCache.size >= 2000) widthCache.clear();
      widthCache.set(key, w);
    }
    return w;
  };

  // Définition des 5 thèmes de couleur du HUD (Étape 19)
  J.THEMES = {
    cyan: {
      id: 'cyan',
      name: 'Cyan Hologramme',
      primaryRgb: '56, 225, 255',
      primaryHex: '#38e1ff',
      secondaryHex: '#7ef2e2',
      bgHex: '#00040a',
      accentGlow: 'rgba(56, 225, 255, 0.4)',
      border: 'rgba(56, 225, 255, 0.28)'
    },
    gold: {
      id: 'gold',
      name: 'Stark Ambre & Or',
      primaryRgb: '255, 180, 32',
      primaryHex: '#ffb420',
      secondaryHex: '#ffd766',
      bgHex: '#080500',
      accentGlow: 'rgba(255, 180, 32, 0.4)',
      border: 'rgba(255, 180, 32, 0.32)'
    },
    matrix: {
      id: 'matrix',
      name: 'Émeraude Matrix',
      primaryRgb: '0, 255, 110',
      primaryHex: '#00ff6e',
      secondaryHex: '#70ffb0',
      bgHex: '#000802',
      accentGlow: 'rgba(0, 255, 110, 0.4)',
      border: 'rgba(0, 255, 110, 0.28)'
    },
    crimson: {
      id: 'crimson',
      name: 'Rubis Crimson',
      primaryRgb: '255, 48, 80',
      primaryHex: '#ff3050',
      secondaryHex: '#ff7895',
      bgHex: '#0a0004',
      accentGlow: 'rgba(255, 48, 80, 0.4)',
      border: 'rgba(255, 48, 80, 0.3)'
    },
    violet: {
      id: 'violet',
      name: 'Synthwave Améthyste',
      primaryRgb: '190, 85, 240',
      primaryHex: '#be55f0',
      secondaryHex: '#df95ff',
      bgHex: '#08000d',
      accentGlow: 'rgba(190, 85, 240, 0.4)',
      border: 'rgba(190, 85, 240, 0.3)'
    }
  };

  // Niveaux de densité d'affichage (Étape 19)
  J.DENSITIES = {
    compact: {
      id: 'compact',
      name: 'Compact',
      panelLines: 12,
      lineHeight: 16,
      panelWidthRatio: 0.36,
      fontTitle: '9px',
      fontText: '11px',
      margin: 8
    },
    normal: {
      id: 'normal',
      name: 'Normal',
      panelLines: 8,
      lineHeight: 19,
      panelWidthRatio: 0.42,
      fontTitle: '10px',
      fontText: '13px',
      margin: 12
    },
    large: {
      id: 'large',
      name: 'Large (Confort)',
      panelLines: 6,
      lineHeight: 24,
      panelWidthRatio: 0.48,
      fontTitle: '12px',
      fontText: '15px',
      margin: 16
    }
  };

  // Fond : halo d'ambiance très doux et circuit imprimé (circuit.js) figés une fois pour toutes dans ce calque ;
  // le courant qui y circule et les liaisons des cards sont dessinés par-dessus, image par image.
  J.buildHudLayer = function (w, h, dpr, themeId = 'cyan') {
    const c = document.createElement('canvas');
    c.width = Math.max(1, Math.floor(w * dpr));
    c.height = Math.max(1, Math.floor(h * dpr));
    const g = c.getContext('2d');
    g.scale(dpr, dpr);

    const theme = J.THEMES[themeId] || J.THEMES.cyan;
    const rgb = theme.primaryRgb;
    const cx = w / 2;
    const cy = h / 2;

    // Dégradé radial d'ambiance ultra-doux centré sur le cœur (sans aucun dessin ni trait fixe)
    if (g.createRadialGradient) {
      const grad = g.createRadialGradient(cx, cy, 30, cx, cy, Math.max(w, h) * 0.65);
      grad.addColorStop(0, `rgba(${rgb}, 0.03)`);
      grad.addColorStop(0.5, `rgba(${rgb}, 0.006)`);
      grad.addColorStop(1, 'rgba(0, 4, 10, 0)');
      g.fillStyle = grad;
      g.fillRect(0, 0, w, h);
    }
    if (J.circuit) J.circuit.bake(g, w, h, rgb);

    return c;
  };
})();
