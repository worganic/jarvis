// Section « Optimisation » de l'administration : toutes les options qui peuvent être coupées pour
// gagner en puissance et en mémoire. Les changements s'appliquent à chaud (sans redémarrer Jarvis) ;
// seule la voix redémarre quand on touche à ses réglages internes. Les « impacts » indiqués sont des
// estimations qualitatives : le panneau « Consommation actuelle » mesure, lui, les vrais chiffres.
(function () {
  const A = window.JarvisAdmin;
  if (!A) return;
  const { h, panel, field, select, toggle, save, api, sections, getSnapshot, confirmDialog } = A;

  const REFRESH_MS = 3000;
  let metricsTimer = null;
  let lastMetrics = null; // dernière mesure : réaffichée tout de suite quand la page est redessinée

  const perf = () => getSnapshot().config.performance;
  const lk = () => getSnapshot().config.voice.livekit;
  const setPerf = (patch) => save('setConfig', { performance: patch });
  const setVoice = async (patch) => {
    const res = await save('setConfig', { voice: { livekit: patch } });
    if (res && res.ok !== false) A.toast('✓ enregistré · la voix redémarre (quelques secondes)');
    return res;
  };

  // --- Préréglages -----------------------------------------------------------------------------

  // Un préréglage est « actif » quand toutes ses valeurs correspondent aux réglages courants.
  function presetActive(preset) {
    const p = perf();
    const l = lk();
    return Object.entries(preset.performance).every(([k, v]) => p[k] === v) &&
      Object.entries((preset.voice && preset.voice.livekit) || {}).every(([k, v]) => l[k] === v);
  }

  async function applyPreset(id, preset) {
    const voiceChanges = Object.entries((preset.voice && preset.voice.livekit) || {}).some(([k, v]) => lk()[k] !== v);
    const res = await save('setConfig', { performance: preset.performance, voice: preset.voice });
    if (res && res.ok !== false && (voiceChanges || preset.performance.voiceListening !== perf().voiceListening)) {
      A.toast(`✓ préréglage « ${preset.label} » appliqué · la voix se met à jour`);
    }
  }

  function presetsPanel() {
    const presets = Object.entries(getSnapshot().catalog.performancePresets || {});
    return panel('Préréglages', 'un clic règle toutes les options',
      h('div', { class: 'perf-preset-grid' }, ...presets.map(([id, p]) =>
        h('button', { class: `perf-preset${presetActive(p) ? ' on' : ''}`, onclick: () => applyPreset(id, p) },
          h('b', {}, p.label), h('span', {}, p.description)))),
      h('div', { class: 'help', style: 'margin-top: 12px' },
        'Aucun préréglage n’est actif si tu as réglé les options à la main. « Complet » correspond au comportement d’origine.'));
  }

  // --- Consommation réelle -------------------------------------------------------------------------

  function metricsView(m) {
    if (!m || !m.ok) return [h('div', { class: 'empty' }, m && m.error ? m.error : 'Mesure en cours…')];
    const fmt = (n) => (n == null ? '—' : `${n} Mo`);
    const rows = [
      h('tr', { class: 'total' }, h('td', {}, 'Jarvis v2 (coquille, fenêtres, noyau, voix), total'), h('td', { class: 'num' }, fmt(m.electron.mb)), h('td', { class: 'num' }, `${m.electron.cpu.toFixed(1)} % CPU`)),
      ...m.electron.rows.map((r) => h('tr', { class: 'sub' }, h('td', {}, r.label), h('td', { class: 'num' }, fmt(r.mb)), h('td', { class: 'num' }, `${r.cpu.toFixed(1)} %`))),
      ...m.voice.map((v) => h('tr', {}, h('td', {}, v.label), h('td', { class: 'num' }, fmt(v.mb)), h('td', { class: 'num' }, ''))),
    ];
    if (m.voicePaused) rows.push(h('tr', {}, h('td', {}, 'Voix (Python, Whisper)'), h('td', { class: 'num' }, 'arrêtée'), h('td', {})));
    return [
      h('table', { class: 'meter-table' }, h('tbody', {}, ...rows)),
      h('div', { class: 'help', style: 'margin-top: 10px' },
        `Mémoire sémantique : ${m.semanticLoaded ? 'modèle chargé en mémoire (environ 0,8 à 1,5 Go)' : 'modèle non chargé (déchargé après 5 min d’inactivité)'}. `,
        'Mesure rafraîchie toutes les 3 s tant que cette page est ouverte ; mesure de tout l’arbre de processus de Jarvis (coquille Tauri, WebView2, noyau, moteur vocal, IA).')
    ];
  }

  // Résumé du mode automatique : niveaux de charge et options actuellement réduites.
  function autoStatusView(auto, enabled) {
    if (!enabled) return [h('span', { class: 'badge off' }, 'désactivé')];
    if (!auto || !auto.enabled) return [h('span', { class: 'badge off' }, 'démarrage…')];
    const reduced = auto.labels && auto.labels.length;
    return [
      h('span', { class: `badge ${reduced ? 'warn' : 'ok'}` }, reduced ? 'réduction en cours' : 'aucune réduction'),
      h('span', {}, `Charge CPU : niveau ${auto.cpuLevel}/3 · Mémoire : niveau ${auto.ramLevel}/2`),
      reduced ? h('span', {}, `Options réduites : ${auto.labels.join(', ')}`) : h('span', { class: 'help' }, 'Le PC n’est pas chargé durablement : tout est au niveau réglé ci-dessous.')
    ];
  }

  async function refreshMetrics() {
    const host = document.getElementById('perf-metrics');
    const active = document.getElementById('section-perf');
    if (!host || !active || !active.classList.contains('active')) {
      clearInterval(metricsTimer);
      metricsTimer = null;
      return;
    }
    const res = await api('perfMetrics');
    if (res && res.ok) lastMetrics = res;
    const again = document.getElementById('perf-metrics');
    if (again) again.replaceChildren(...metricsView(res));
    const autoBox = document.getElementById('perf-auto');
    if (autoBox && res && res.ok) autoBox.replaceChildren(...autoStatusView(res.auto, perf().autoMode));
  }

  function ensureMetricsTimer() {
    if (metricsTimer) return;
    metricsTimer = setInterval(refreshMetrics, REFRESH_MS);
    setTimeout(refreshMetrics, 50);
  }

  // --- Options ------------------------------------------------------------------------------------

  // Une option : interrupteur, ce qu'elle coupe, et l'impact attendu (estimation).
  function option(label, key, help, impact, { disabled = false, invert = false } = {}) {
    const value = Boolean(perf()[key]);
    const sw = toggle(label, invert ? !value : value, (v) => setPerf({ [key]: invert ? !v : v }));
    if (disabled) sw.querySelector('input').disabled = true;
    return h('div', { class: `opt${disabled ? ' disabled' : ''}` },
      sw,
      h('div', { class: 'help' }, help),
      impact ? h('div', { class: 'impact' }, `Impact : ${impact}`) : null);
  }

  function autoPanel() {
    const catalog = getSnapshot().catalog.autoPerf;
    const t = catalog && catalog.timings;
    const rows = [];
    if (catalog) {
      for (const [name, label, unit] of [['cpu', 'Processeur', 'CPU'], ['ram', 'Mémoire vive', 'RAM']]) {
        const ch = catalog.channels[name];
        ch.tiers.forEach((labels, i) => rows.push(h('tr', {},
          h('td', {}, `${label} · niveau ${i + 1}`),
          h('td', { class: 'num' }, `≥ ${ch.up[i]} %`),
          h('td', { class: 'num' }, `retour < ${ch.down[i]} %`),
          h('td', {}, labels.join(', ')))));
      }
    }
    return panel('Mode automatique', 'selon la charge du PC',
      option('Adapter les options à la charge du PC', 'autoMode',
        'Jarvis réduit tout seul certaines options quand le processeur ou la mémoire du PC sont chargés durablement, puis les rétablit quand le PC se calme. Tes réglages enregistrés ne sont jamais modifiés : désactive le mode et tout revient exactement comme tu l’as réglé.',
        'aucun quand le PC est au calme ; allège l’affichage sous charge'),
      h('div', { class: 'status-line', id: 'perf-auto', style: 'margin: 10px 0 0 52px; align-items: center' }, ...autoStatusView(lastMetrics && lastMetrics.auto, perf().autoMode)),
      t ? h('div', { class: 'help', style: 'margin: 14px 0 6px' },
        `Pour éviter tout clignotement : la décision repose sur la moyenne des ${t.windowS} dernières secondes ; une réduction n’a lieu qu’après ${t.upHoldS} s de charge soutenue, un rétablissement qu’après ${t.downHoldS} s de calme, et jamais plus d’un changement (d’un seul niveau) toutes les ${t.cooldownS} s. Si le PC se recharge dès qu’on allège, le calme exigé avant de rétablir double (jusqu’à ${t.maxDownHoldS / 60} min). L’écoute orale n’est jamais touchée.`) : null,
      rows.length ? h('table', { class: 'meter-table' }, h('tbody', {}, ...rows)) : null);
  }

  // --- Espace disque des compilations (target/) -----------------------------------------------------

  const gb = (n) => {
    const g = (n || 0) / 1073741824;
    return g >= 10 ? `${Math.round(g)} Go` : `${g.toFixed(1).replace('.', ',')} Go`;
  };
  let buildBusy = '';

  async function buildAction(method, busyLabel) {
    buildBusy = busyLabel;
    A.render(true);
    const res = await api(method);
    buildBusy = '';
    if (res && res.state) getSnapshot().buildSpace = res.state;
    if (res && res.ok && method === 'buildSpaceSweep') {
      A.toast(`✓ ${gb(res.liberes)} libérés${res.echecs ? ` · ${res.echecs} élément(s) verrouillé(s) laissé(s)` : ''}`);
    }
    A.render(true);
  }

  async function sweep() {
    const m = (getSnapshot().buildSpace || {}).mesure;
    const ok = await confirmDialog('EFFACER LES ANCIENNES COPIES',
      `Effacer ${m ? gb(m.anciennes) : 'les anciennes copies'} de compilations des crates de Jarvis ? Les dépendances et la compilation actuelle sont gardées ; Cargo recompile ce qui manquerait.`,
      'Effacer', true);
    if (ok) buildAction('buildSpaceSweep', 'Effacement en cours…');
  }

  function buildPanel() {
    const st = getSnapshot().buildSpace || {};
    const m = st.mesure;
    const cfg = getSnapshot().config.buildSpace || {};
    const busy = Boolean(buildBusy || st.occupe);
    const rows = m ? (m.profils || []).map((p) => h('tr', {},
      h('td', {}, `target/${p.nom}`), h('td', { class: 'num' }, gb(p.octets)), h('td', { class: 'num' }, `dont ${gb(p.anciennes)} d’anciennes copies`))) : [];
    const thresholds = [20, 40, 60, 100, 200];
    if (!thresholds.includes(cfg.alertGb)) thresholds.push(cfg.alertGb);
    return panel('Espace disque des compilations', 'dossier target/',
      h('div', { class: 'status-line', style: 'align-items: center' },
        m ? h('span', { class: `badge ${st.depasse ? 'warn' : 'ok'}` }, st.depasse ? 'seuil dépassé' : 'sous le seuil') : h('span', { class: 'badge off' }, 'pas encore mesuré'),
        m ? h('span', {}, `Total : ${gb(m.octets)} · anciennes copies : ${gb(m.anciennes)} · mesuré le ${A.fmtDate(new Date(m.mesureLe).toISOString())}`) : null),
      rows.length ? h('table', { class: 'meter-table', style: 'margin-top: 10px' }, h('tbody', {}, ...rows)) : null,
      h('div', { style: 'display: flex; gap: 10px; margin: 14px 0 6px; flex-wrap: wrap' },
        h('button', { class: 'btn danger', disabled: busy || !m || !m.anciennes, onclick: sweep }, buildBusy === 'Effacement en cours…' ? buildBusy : 'Effacer les anciennes copies'),
        h('button', { class: 'btn ghost', disabled: busy, onclick: () => buildAction('buildSpaceScan', 'Mesure en cours…') }, buildBusy === 'Mesure en cours…' ? buildBusy : 'Mesurer maintenant')),
      h('div', { class: 'help' },
        'À chaque nouvelle version, Cargo recompile les crates de Jarvis sous un autre nom et garde les copies précédentes (plusieurs centaines de Mo chacune). L’effacement ne touche que ces anciennes copies : la compilation actuelle et les dépendances restent. La mesure est refaite toutes les 6 h.'),
      h('div', { style: 'height: 12px' }),
      toggle('Alerte quand les compilations prennent trop de place', cfg.alert !== false, (v) => setBuild({ alert: v })),
      h('div', { class: 'opt', style: 'margin-top: 10px' },
        field('Seuil d’alerte', select(thresholds.sort((a, b) => a - b).map((n) => [String(n), `${n} Go`]), String(cfg.alertGb), (v) => setBuild({ alertGb: Number(v) })),
          'Au-delà, Jarvis le signale (une fois par jour au plus) et ce panneau passe en orange.')));
  }

  const setBuild = async (patch) => {
    const res = await save('setConfig', { buildSpace: patch });
    const r = await api('buildSpaceState');
    if (r && r.state) getSnapshot().buildSpace = r.state;
    A.render(true);
    return res;
  };

  sections.perf = () => {
    ensureMetricsTimer();
    const p = perf();
    const l = lk();
    const voiceOff = !p.voiceListening;
    const monitorOff = !p.hardwareMonitoring;

    return [
      h('h1', {}, 'Optimisation'),
      h('p', { class: 'lead' },
        'Coupe ce dont tu n’as pas besoin pour gagner en puissance et en mémoire. Tout s’applique tout de suite, sans redémarrer Jarvis, et se rétablit aussi facilement. Les impacts indiqués sont des estimations ; la mesure réelle est juste en dessous.'),

      presetsPanel(),

      autoPanel(),

      panel('Consommation actuelle', 'mesure réelle', h('div', { id: 'perf-metrics' }, ...metricsView(lastMetrics))),

      buildPanel(),

      panel('Écoute orale', 'micro, transcription, parole',
        option('Écoute orale et transcription', 'voiceListening',
          'Micro, transcription (Whisper sur le GPU) et réponses parlées. Désactivée, Jarvis passe en mode texte : le service vocal Python est arrêté, tu tapes tes commandes et lis les réponses.',
          'élevé (processus Python, modèle Whisper en mémoire GPU)'),
        h('div', { class: `opt${voiceOff ? ' disabled' : ''}` },
          field('Précision du décodage', (() => {
            const el = select([['1', '1 · rapide (moins de CPU/GPU)'], ['2', '2 · équilibré (défaut)'], ['3', '3 · précis'], ['5', '5 · très précis (le plus lourd)']], String(l.sttBeam), (v) => setVoice({ sttBeam: Number(v) }));
            el.disabled = voiceOff;
            return el;
          })(), 'Nombre d’hypothèses gardées par Whisper à la fin d’une phrase. La voix redémarre au changement.'),
          h('div', { style: 'height: 12px' }),
          (() => {
            const sw = toggle('Référence audio du PC (anti-écho renforcé)', l.systemAudioReference, (v) => setVoice({ systemAudioReference: v }));
            sw.querySelector('input').disabled = voiceOff;
            return sw;
          })(),
          h('div', { class: 'help', style: 'margin-left: 52px; margin-top: 5px' }, 'Capte le son du PC pour retirer musique et vidéos de ta voix. Utile avec des haut-parleurs ; inutile au casque.'),
          h('div', { class: 'impact', style: 'margin-left: 52px; margin-top: 3px' }, 'Impact : faible à moyen (capture audio continue)'))),

      panel('Mémoire et services', 'arrière-plan',
        option('Mémoire sémantique (recherche par le sens)', 'semanticMemory',
          'Modèle EmbeddingGemma pour retrouver un souvenir ou une commande par le sens. Désactivée, la recherche devient lexicale (mots exacts) et le modèle n’est jamais chargé.',
          'élevé (environ 0,8 à 1,5 Go de RAM quand le modèle est chargé ; ~0,8 Go mesuré)'),
        option('Surveillance du matériel (CPU, RAM)', 'hardwareMonitoring',
          'Mesure le CPU et la RAM toutes les 1,5 s et les affiche en bas à gauche.',
          'faible (une mesure toutes les 1,5 s, plus le dessin de la ligne)'),
        option('Mode éco automatique', 'autoEco',
          'Bascule sur un cœur épuré et une cadence réduite quand le CPU ou la RAM dépassent 90 %, et revient sous 85 %. Exige la surveillance du matériel.',
          'protège la machine en pleine charge ; sans effet au repos', { disabled: monitorOff }),
        option('Surveillance du périphérique audio', 'audioDeviceWatch',
          'Détecte le passage casque ↔ enceintes toutes les 20 s pour rouvrir le micro. Désactivée, le périphérique n’est lu qu’au démarrage.',
          'faible (une vérification toutes les 20 s)'),
        option('Journal de mesure', 'metricsLog',
          'Écrit une ligne « [metrics] » par minute dans data/logs/jarvis.log (mémoire et CPU de chaque processus de Jarvis).',
          'très faible')),

      panel('Affichage', 'cœur, cards, graphisme',
        option('Cœur détaillé', 'orbDetail',
          'Blobs lumineux, anneaux ondulants, ondes de réflexion et balayage à l’intérieur du cœur. Désactivé, le cœur garde ses anneaux et sa couleur d’humeur mais devient plus simple.',
          'moyen (dégradés et tracés recalculés à chaque image)'),
        option('Couronne vocale réactive', 'voiceWaveform',
          'Les traits qui réagissent à ta voix autour du cœur.',
          'faible à moyen'),
        option('Cards d’action et branches', 'cards',
          'Les cards Musique, Gmail, Chrome, Agenda, leurs liaisons vers le cœur et les branches de tâches. Désactivées, l’avancement reste visible dans l’en-tête et le journal.',
          'moyen à élevé quand plusieurs tâches sont actives'),
        option('Fond : circuit imprimé et circulation', 'hudBackground',
          'Le circuit imprimé du fond (le cœur en processeur, ses pistes), le courant qui y circule selon l’activité de Jarvis, et les traces de rémanence. Désactivé, le fond est uni et son calque (de la taille de la fenêtre) est libéré ; les cards restent reliées au cœur par leurs pistes.',
          'moyen ; libère aussi ~10 à 20 Mo de mémoire vidéo selon la taille de la fenêtre'),
        h('div', { class: 'opt' },
          field('Cadence maximale de l’affichage', select(
            (getSnapshot().catalog.maxFps || [0, 60, 30, 20]).map((n) => [String(n), n === 0 ? 'Libre (suit l’écran)' : `${n} images/s`]),
            String(p.maxFps), (v) => setPerf({ maxFps: Number(v) })),
          'Plafonne le nombre d’images dessinées par seconde en activité (au repos, Jarvis descend déjà à ~20). La valeur réelle est proche de celle choisie, au rythme de ton écran près.'),
          h('div', { class: 'impact', style: 'margin-top: 3px' }, 'Impact : élevé sur un écran 100 Hz ou plus'))),
    ];
  };
})();
