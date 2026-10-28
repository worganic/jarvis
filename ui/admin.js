// Page d'administration de Jarvis (chantier v3, étape 3).
// Tout passe par window.jarvisAdmin.call(méthode, …) : le processus principal n'accepte que les
// méthodes de la liste blanche (orchestrator/admin.js) et seulement depuis cette fenêtre.
(function () {
  const bridge = window.jarvisAdmin;
  const THEME_RGB = {
    cyan: '56, 225, 255',
    gold: '255, 180, 32',
    matrix: '0, 255, 110',
    crimson: '255, 48, 80',
    violet: '190, 85, 240'
  };
  // Icône de chaque apparence du cœur (orchestrator/status.js, HEART_STYLES).
  const HEART_ICONS = { classic: '◉', bobot: '🤖', marvin: '⚪', petitrobot: '🔴', chat: '😾' };

  let S = null;             // dernier instantané complet
  let current = localStorage.getItem('admin.section') || 'general';
  const sections = {};      // id -> fonction de rendu
  const pendingRefresh = new Set();
  let enrollState = { active: false, targetName: '', current: 0, target: 8, message: '' };

  // --- Outils DOM ------------------------------------------------------------------------------

  function h(tag, attrs = {}, ...children) {
    const el = document.createElement(tag);
    for (const [k, v] of Object.entries(attrs || {})) {
      if (v === undefined || v === null || v === false) continue;
      if (k === 'class') el.className = v;
      else if (k === 'text') el.textContent = v;
      else if (k === 'html') el.innerHTML = v;
      else if (k.startsWith('on') && typeof v === 'function') el.addEventListener(k.slice(2), v);
      else if (k === 'value') el.value = v;
      else if (k === 'checked') el.checked = Boolean(v);
      else el.setAttribute(k, v === true ? '' : v);
    }
    for (const child of children.flat()) {
      if (child === null || child === undefined || child === false) continue;
      el.append(child instanceof Node ? child : document.createTextNode(String(child)));
    }
    return el;
  }

  function panel(title, tag, ...children) {
    return h('div', { class: 'panel' }, h('h2', {}, title, tag ? h('span', { class: 'tag' }, tag) : null), ...children);
  }

  function field(label, control, help, wide) {
    return h('div', { class: `field${wide ? ' wide' : ''}` }, h('label', {}, label), control, help ? h('div', { class: 'help' }, help) : null);
  }

  // Lien d'une personne avec l'utilisateur principal (empreintes vocales).
  const SPEAKER_TYPES = { moi: 'Moi', femme: 'Femme / compagne', mari: 'Mari / compagnon', fille: 'Fille', fils: 'Fils', famille: 'Famille', ami: 'Ami(e)', invite: 'Invité(e)', autre: 'Autre' };

  function select(options, value, onchange) {
    const el = h('select', { onchange: (e) => onchange(e.target.value) });
    const entries = Array.isArray(options) ? options.map((o) => (typeof o === 'string' ? [o, o] : o)) : Object.entries(options);
    for (const [val, label] of entries) el.append(h('option', { value: val, selected: String(val) === String(value) }, label));
    el.value = value == null ? '' : value;
    return el;
  }

  // Compagnon de bureau (tamagotchi, orch/companion) : désactivé par défaut ; ses réglages vivent dans data/companion.json.
  function companionBlock() {
    const box = h('div', { class: 'grid', style: 'margin-top:14px' }, h('div', { class: 'help' }, 'Compagnon de bureau : chargement…'));
    let confirmAt = 0;
    const draw = (c) => {
      const cfg = c.config || {};
      const set = async (patch) => {
        const r = await api('companionSet', patch).catch(() => null);
        if (r && r.state) draw(r.state);
      };
      const restart = h('button', { class: 'btn', onclick: async () => {
        if (Date.now() - confirmAt > 4000) {
          confirmAt = Date.now();
          restart.textContent = 'Confirmer : nouvel œuf ?';
          return;
        }
        const r = await api('companionRestart').catch(() => null);
        if (r && r.state) draw(r.state);
      } }, '⟲ Recommencer un œuf');
      box.replaceChildren(
        field('Compagnon de bureau (tamagotchi)', toggle('Le cœur naît d’un œuf, grandit et évolue selon l’usage du PC', Boolean(c.enabled), (v) => set({ enabled: v })),
          'Œuf → bébé → ado → adulte, branche Gamer, Dev, Artiste ou Chill selon les applications au premier plan (le nom du programme seulement). Il vit dans le widget et dans le cœur du HUD, mange les fichiers qu’on lui dépose (extension seulement), réagit au CPU, à la musique, à la nuit. Jarvis reste prioritaire : dès qu’il écoute, parle ou travaille, le compagnon s’efface.'),
        field('Durée du cycle', select([['7', '7 jours'], ['30', '30 jours']], String(cfg.cycleDays || 30), (v) => set({ cycleDays: Number(v) })),
          'Jours où le PC sert (un PC éteint ne le fait pas vieillir). À la fin, il part en voyage (souvenir dans sa card) et laisse un nouvel œuf qui hérite d’un peu de sa branche.'),
        field('Annonce des évolutions', toggle('Le dire à voix haute', Boolean(cfg.voiceAnnounce), (v) => set({ voiceAnnounce: v })),
          'Sinon écrite seulement. Jamais par-dessus ta voix, jamais pendant une sourdine.'),
        field('Mode furtif', toggle('Presque invisible en jeu et en plein écran', cfg.autoStealth !== false, (v) => set({ autoStealth: v }))),
        field('Promenade', toggle('Se promène le long de la barre des tâches au repos', cfg.walk !== false, (v) => set({ walk: v })),
          'Après 2 minutes de calme ; il revient à sa place dès que Jarvis s’active ou que la souris approche. Pas en Bureau JARVIS.'),
        c.enabled ? field('Aujourd’hui', h('div', {}, h('div', {}, c.describe || ''), restart), 'Card Compagnon (N° 123) : « ouvre le compagnon ».') : null);
    };
    api('companionState').then(draw).catch(() => box.replaceChildren(h('div', { class: 'help' }, 'Compagnon de bureau indisponible (noyau à mettre à jour).')));
    return box;
  }

  function toggle(label, checked, onchange) {
    return h('label', { class: 'switch' },
      h('input', { type: 'checkbox', checked, onchange: (e) => onchange(e.target.checked) }),
      h('span', { class: 'track' }),
      h('span', { class: 'label' }, label));
  }

  function range(min, max, step, value, format, onchange) {
    const out = h('output', {}, format(value));
    const input = h('input', {
      type: 'range', min, max, step, value,
      oninput: (e) => { out.textContent = format(Number(e.target.value)); },
      onchange: (e) => onchange(Number(e.target.value))
    });
    return h('div', { class: 'range' }, input, out);
  }

  function textInput(value, attrs = {}) {
    return h('input', { type: 'text', value: value || '', ...attrs });
  }

  function fmtDate(iso) {
    if (!iso) return '';
    const d = new Date(iso);
    return d.toLocaleDateString('fr-FR', { day: '2-digit', month: '2-digit', year: '2-digit' }) +
      ' ' + d.toLocaleTimeString('fr-FR', { hour: '2-digit', minute: '2-digit' });
  }

  // --- Appels et retours ------------------------------------------------------------------------

  let toastTimer = null;
  function toast(text, isError) {
    const el = document.getElementById('save-state');
    el.textContent = text;
    el.classList.toggle('error', Boolean(isError));
    el.classList.add('show');
    clearTimeout(toastTimer);
    toastTimer = setTimeout(() => el.classList.remove('show'), isError ? 5000 : 1800);
  }

  async function api(method, ...args) {
    try {
      const res = await bridge.call(method, ...args);
      if (res && res.snapshot) setSnapshot(res.snapshot, true);
      if (res && res.ok === false) toast(`✕ ${res.error || 'échec'}`, true);
      return res;
    } catch (err) {
      toast(`✕ ${err.message || err}`, true);
      return { ok: false, error: String(err.message || err) };
    }
  }

  async function reload() {
    const snap = await bridge.call('snapshot');
    if (snap) setSnapshot(snap, true);
  }
  const load = reload;

  async function save(method, ...args) {
    const res = await api(method, ...args);
    if (res && res.ok !== false && !res.challenge) toast('✓ enregistré');
    if (res && res.challenge) return challengeFlow(res.challenge, res.confirm || confirmFor(method));
    return res;
  }

  function confirmFor(method) {
    return { memoryPurgeRequest: 'memoryPurgeConfirm', zoneAdd: 'zoneAddConfirm', securitySet: 'securitySetConfirm' }[method] || null;
  }

  // Code à 4 chiffres : affiché, puis retapé (geste délibéré, comme sur le HUD).
  function challengeFlow(ch, confirmMethod) {
    return new Promise((resolve) => {
      const input = h('input', { type: 'text', class: 'code-input', maxlength: '4', inputmode: 'numeric', autocomplete: 'off' });
      const error = h('div', { class: 'help', style: 'color: var(--err); min-height: 18px' });
      openModal('VALIDATION PAR CODE', [
        h('div', {}, ch.label),
        h('div', { class: 'help' }, 'Recopie ce code pour confirmer. Il expire dans deux minutes.'),
        h('div', { class: 'code-display' }, ch.code),
        input,
        error
      ], [
        h('button', { class: 'btn ghost', onclick: () => { closeModal(); resolve({ ok: false, cancelled: true }); } }, 'Annuler'),
        h('button', {
          class: 'btn primary',
          onclick: async () => {
            const res = await bridge.call(confirmMethod, ch.id, input.value);
            if (res && res.ok) {
              if (res.snapshot) setSnapshot(res.snapshot, true);
              closeModal();
              toast('✓ validé');
              resolve(res);
            } else {
              error.textContent = (res && res.error) || 'code incorrect';
              input.select();
            }
          }
        }, 'Confirmer')
      ]);
      input.addEventListener('keydown', (e) => {
        if (e.key === 'Enter') document.querySelector('#modal-actions .btn.primary').click();
      });
      setTimeout(() => input.focus(), 30);
    });
  }

  function confirmDialog(title, text, okLabel = 'Confirmer', danger = false) {
    return new Promise((resolve) => {
      openModal(title, [h('div', {}, text)], [
        h('button', { class: 'btn ghost', onclick: () => { closeModal(); resolve(false); } }, 'Annuler'),
        h('button', { class: `btn ${danger ? 'danger' : 'primary'}`, onclick: () => { closeModal(); resolve(true); } }, okLabel)
      ]);
    });
  }

  function openModal(title, body, actions) {
    document.getElementById('modal-title').textContent = title;
    const b = document.getElementById('modal-body');
    const a = document.getElementById('modal-actions');
    b.replaceChildren(...body);
    a.replaceChildren(...actions);
    document.getElementById('modal').classList.remove('hidden');
  }

  function closeModal() {
    document.getElementById('modal').classList.add('hidden');
  }

  // --- Instantané et rendu ------------------------------------------------------------------

  function applyTheme(theme) {
    document.documentElement.style.setProperty('--rgb', THEME_RGB[theme] || THEME_RGB.cyan);
  }

  function setSnapshot(snap, force) {
    S = snap;
    applyTheme(S.status.theme);
    renderFoot();
    render(force);
  }

  // Ne pas écraser une saisie en cours : le rendu attend que le champ perde le focus.
  function render(force) {
    const host = document.getElementById(`section-${current}`);
    const typing = host && host.contains(document.activeElement) &&
      /^(INPUT|TEXTAREA)$/.test(document.activeElement.tagName) && document.activeElement.type !== 'range' &&
      document.activeElement.type !== 'checkbox';
    if (typing && !force) {
      pendingRefresh.add(current);
      return;
    }
    pendingRefresh.delete(current);
    const scroll = document.getElementById('content').scrollTop;
    // Les panneaux facultatifs absents (null, false) ne doivent pas s'afficher comme du texte.
    host.replaceChildren(...[].concat(sections[current] ? sections[current]() : []).filter((n) => n !== null && n !== undefined && n !== false));
    document.getElementById('content').scrollTop = scroll;
  }

  document.addEventListener('focusout', () => {
    setTimeout(() => {
      if (pendingRefresh.has(current) && !document.getElementById(`section-${current}`).contains(document.activeElement)) render(true);
    }, 150);
  });

  function renderFoot() {
    const v = S.voice || {};
    const backend = v.backend === 'engine' ? (v.ready ? 'moteur vocal v2' : 'démarrage…') : v.fallback ? 'arrêtée (échecs répétés)' : 'arrêtée';
    const vClass = v.backend === 'engine' ? (v.ready ? 'ok' : 'warn') : 'err';
    const j = S.journal || {};
    document.getElementById('nav-foot').replaceChildren(
      h('div', {}, h('span', { class: `dot ${vClass}` }), `Voix : ${backend}`),
      h('div', {}, h('span', { class: `dot ${j.ok ? 'ok' : 'err'}` }), `Journal : ${j.count || 0} entrées ${j.ok ? 'intact' : 'ALTÉRÉ'}`),
      h('div', {}, h('span', { class: 'dot ok' }), `Mémoire : ${(S.memory || []).length} souvenirs`),
      buildFoot()
    );
  }

  // Compilations (target/) au-delà du seuil d'alerte : rappel cliquable vers Optimisation.
  function buildFoot() {
    const b = S.buildSpace || {};
    if (!b.mesure || !b.depasse) return null;
    const g = Math.round((b.mesure.octets || 0) / 1073741824);
    return h('div', { style: 'cursor: pointer', title: 'Administration › Optimisation', onclick: () => selectSection('perf') },
      h('span', { class: 'dot warn' }), `Compilations : ${g} Go`);
  }

  // « mcp#gmail » : section, puis panneau à mettre en avant (Jarvis ouvre cette page de lui-même
  // quand il lui manque des informations, voir orchestrator/index.js › requestSetup).
  function selectSection(target) {
    const [id, focus] = String(target || 'general').split('#');
    window.JarvisAdmin.focus = focus ? { section: id, target: focus, at: Date.now() } : null;
    current = id;
    localStorage.setItem('admin.section', id);
    document.querySelectorAll('#nav button').forEach((b) => b.classList.toggle('active', b.dataset.section === id));
    document.querySelectorAll('.section').forEach((s) => s.classList.toggle('active', s.id === `section-${id}`));
    if (S) render(true);
    document.getElementById('content').scrollTop = 0;
  }

  // --- Économie de tokens (v5) : niveau, consommation des 7 derniers jours, raccourcis ------------

  const kfmt = (n) => (n >= 1e6 ? `${(n / 1e6).toFixed(1)} M` : n >= 1e3 ? `${Math.round(n / 1e3)} k` : String(n || 0));

  function economyPanel() {
    const st = S.status;
    const eco = S.economy || { history: [], macros: [] };
    const today = st.today || { calls: 0, in: 0, out: 0, local: 0, ignored: 0 };
    const rows = (eco.history || []).slice().reverse().map((d) => {
      const input = (d.input || 0) + (d.cacheRead || 0) + (d.cacheWrite || 0);
      return h('tr', {},
        h('td', {}, d.date),
        h('td', {}, String(d.calls || 0)),
        h('td', {}, `${d.estimated ? '≈ ' : ''}${kfmt(input)}`),
        h('td', {}, kfmt(d.output || 0)),
        h('td', {}, String((d.local || 0) + (d.ignored || 0))));
    });
    return panel('Économie de tokens', `aujourd’hui : ${today.calls} appel${today.calls > 1 ? 's' : ''} IA · ${today.local + today.ignored} sans IA`,
      h('div', { class: 'grid' },
        field('Niveau', select([
          ['normal', 'Normal — ce que Jarvis ne sait pas faire seul part à l’IA'],
          ['local', 'Local d’abord — politesses, commandes, reformulations, micro-appels'],
          ['strict', 'Strict — Jarvis demande avant chaque appel à une IA']
        ], st.economyLevel || 'local', (v) => save('setStatus', { economyLevel: v })),
        'Les actions locales (heure, musique, mails, agenda, fichiers, météo, calcul, dictée…) ne consomment jamais de tokens.', true)
      ),
      rows.length ? h('table', { class: 'table', style: 'margin-top:14px' },
        h('thead', {}, h('tr', {}, h('th', {}, 'Jour'), h('th', {}, 'Appels IA'), h('th', {}, 'Tokens entrants'), h('th', {}, 'Tokens sortants'), h('th', {}, 'Sans IA'))),
        h('tbody', {}, ...rows)) : null,
      h('div', { class: 'help', style: 'margin-top:14px' },
        'Raccourcis personnels : dis « quand je dis mode travail, ouvre Visual Studio Code et mets de la musique ».'),
      ...(eco.macros || []).map((m) => h('div', { class: 'status-line' },
        h('span', {}, h('b', {}, m.name), ` → ${m.steps.join(' · ')}`),
        h('button', { class: 'btn small', onclick: () => save('macroDelete', m.name) }, 'Supprimer')))
    );
  }

  // --- Section : Général ------------------------------------------------------------------------

  sections.general = () => {
    const st = S.status;
    const cat = S.catalog;
    const engine = st.engine === 'agy' ? 'agy' : 'claude';
    const usage = st.usage;
    const chain = (Array.isArray(st.aiChain) && st.aiChain.length
      ? st.aiChain.slice(0, 4)
      : [{ engine, model: st.requested || st.model || 'auto' }]
    ).map((it) => {
      const eng = it && it.engine === 'agy' ? 'agy' : 'claude';
      const models = (cat.models && cat.models[eng]) || ['auto'];
      const mdl = it && models.includes(it.model) ? it.model : 'auto';
      return { engine: eng, model: mdl };
    });

    const saveChain = (next) => save('setStatus', { aiChain: next.slice(0, 4) });
    const updateSlot = (idx, patch) => {
      const next = chain.map((it, i) => {
        if (i !== idx) return { ...it };
        const merged = { ...it, ...patch };
        const eng = merged.engine === 'agy' ? 'agy' : 'claude';
        const allowed = (cat.models && cat.models[eng]) || ['auto'];
        const mdl = allowed.includes(merged.model) ? merged.model : 'auto';
        return { engine: eng, model: mdl };
      });
      saveChain(next);
    };
    const swapSlots = (i, j) => {
      if (i < 0 || j < 0 || i >= chain.length || j >= chain.length || i === j) return;
      const next = chain.map((it) => ({ ...it }));
      const tmp = next[i];
      next[i] = next[j];
      next[j] = tmp;
      saveChain(next);
    };
    const makeDefault = (idx) => {
      if (idx <= 0 || idx >= chain.length) return;
      const next = chain.map((it) => ({ ...it }));
      const [picked] = next.splice(idx, 1);
      next.unshift(picked);
      saveChain(next);
    };
    const removeSlot = (idx) => {
      if (chain.length <= 1) return;
      saveChain(chain.filter((_, i) => i !== idx));
    };
    const addSlot = () => {
      if (chain.length >= 4) return;
      const last = chain[chain.length - 1] || { engine: 'claude', model: 'auto' };
      const nextEng = last.engine === 'claude' ? 'agy' : 'claude';
      saveChain([...chain, { engine: nextEng, model: 'auto' }]);
    };

    return [
      h('h1', {}, 'Général'),
      h('p', { class: 'lead' }, 'Moteur qui exécute les demandes, modèle, effort et apparence. Chaque changement s’applique immédiatement et est inscrit au journal scellé.'),
      panel('Moteur d’exécution', `modèle actif : ${st.activeModel || st.model || '—'} · ${chain.length}/4 IA configurée${chain.length > 1 ? 's' : ''}`,
        h('div', { class: 'hint' }, 'Configurez jusqu’à 4 IA par ordre de priorité. La 1re est utilisée par défaut ; si elle échoue (quota atteint, panne, erreur), Jarvis bascule automatiquement sur la 2e, puis la 3e et la 4e.'),
        h('div', { style: 'display:flex; flex-direction:column; gap:10px; margin-bottom:14px;' },
          ...chain.map((slot, idx) => {
            const isDefault = idx === 0;
            const models = (cat.models && cat.models[slot.engine]) || ['auto'];
            return h('div', {
              style: `display:grid; grid-template-columns: 150px minmax(170px, 1fr) minmax(210px, 1.3fr) auto; gap:10px; align-items:center; padding:10px 12px; border:1px solid ${isDefault ? 'var(--line-strong)' : 'var(--line)'}; background:${isDefault ? 'rgba(var(--rgb), 0.09)' : 'rgba(0,0,0,0.28)'};`
            },
              h('div', { style: 'display:flex; flex-direction:column; gap:2px;' },
                h('span', { class: isDefault ? 'badge ok' : 'cat', style: 'align-self:flex-start;' },
                  isDefault ? '★ 1 · PAR DÉFAUT' : `#${idx + 1} · SECOURS ${idx}`),
                h('span', { class: 'help', style: 'font-size:10px;' },
                  isDefault ? 'Exécutée en premier' : `Relais si #${idx} échoue`)
              ),
              select(cat.engines, slot.engine, (v) => updateSlot(idx, { engine: v })),
              select(models.map((m) => [m, m === 'auto' ? 'Automatique (selon la complexité)' : m]), slot.model, (v) => updateSlot(idx, { model: v })),
              h('div', { class: 'row', style: 'gap:6px; flex-wrap:nowrap; justify-content:flex-end;' },
                !isDefault ? h('button', {
                  class: 'btn',
                  style: 'padding:5px 9px; font-size:11px;',
                  title: 'Définir comme IA par défaut (passer en 1re position)',
                  onclick: () => makeDefault(idx)
                }, '★ Défaut') : null,
                h('button', {
                  class: 'btn',
                  style: 'padding:5px 8px; font-size:11px;',
                  disabled: idx === 0,
                  title: 'Monter (intervertir avec l’IA au-dessus)',
                  onclick: () => swapSlots(idx, idx - 1)
                }, '▲'),
                h('button', {
                  class: 'btn',
                  style: 'padding:5px 8px; font-size:11px;',
                  disabled: idx === chain.length - 1,
                  title: 'Descendre (intervertir avec l’IA en dessous)',
                  onclick: () => swapSlots(idx, idx + 1)
                }, '▼'),
                h('button', {
                  class: 'btn danger',
                  style: 'padding:5px 9px; font-size:11px;',
                  disabled: chain.length <= 1,
                  title: 'Supprimer cette IA de la chaîne',
                  onclick: () => removeSlot(idx)
                }, '✕')
              )
            );
          })
        ),
        h('div', { class: 'row', style: 'margin-bottom:16px; gap:10px;' },
          h('button', {
            class: 'btn primary',
            disabled: chain.length >= 4,
            onclick: addSlot
          }, chain.length >= 4 ? 'Maximum atteint (4/4 IA)' : `+ Ajouter une IA de secours (${chain.length}/4)`),
          chain.length >= 2 ? h('button', {
            class: 'btn',
            title: 'Intervertir l’IA par défaut (#1) et la première IA de secours (#2)',
            onclick: () => swapSlots(0, 1)
          }, '⇄ Intervertir #1 et #2') : null
        ),
        h('div', { class: 'grid' },
          field('Effort', select(cat.efforts, st.effort || 'medium', (v) => save('setStatus', { effort: v }))),
          field('Économie de quota', toggle('Modèle léger pour les demandes simples', st.economyMode, (v) => save('setStatus', { economyMode: v })))
        ),
        usage ? h('div', { class: 'status-line', style: 'margin-top:14px' },
          h('span', {}, 'Quota 5 h : ', h('b', {}, `${usage.five.pct} %`)),
          h('span', {}, 'Quota 7 jours : ', h('b', {}, `${usage.week.pct} %`)),
          h('span', { class: 'help' }, `relevé ${fmtDate(new Date(usage.at).toISOString())}`)
        ) : null
      ),
      economyPanel(),
      panel('Surveillance des agents IA', 'Claude Code & AGY',
        h('div', { class: 'grid' },
          field('Surveillance permanente', toggle('Surveiller en continu Claude Code et AGY', (S.config.aiWatch || {}).enabled !== false, (v) => save('setConfig', { aiWatch: { enabled: v } })),
            'Jarvis surveille en arrière-plan vos sessions Claude Code et AGY sur la machine.'),
          field('Alerte fin de prompt', toggle('Prévenir quand un prompt est terminé', (S.config.aiWatch || {}).notifyDone !== false, (v) => save('setConfig', { aiWatch: { notifyDone: v } })),
            'Notification dès que l’agent a achevé son exécution.'),
          field('Alerte question / attente', toggle('Prévenir quand l’agent attend une réponse', (S.config.aiWatch || {}).notifyWaiting !== false, (v) => save('setConfig', { aiWatch: { notifyWaiting: v } })),
            'Notification dès qu’une question ou une validation est requise.'),
          field('Annonce vocale', toggle('Annoncer à voix haute', (S.config.aiWatch || {}).speak !== false, (v) => save('setConfig', { aiWatch: { speak: v } })),
            'Jarvis le dit vocalement en plus de l’affichage sur le HUD.')
        )
      ),
      announcementsPanel(),
      presencePanel(),
      panel('Interface', null,
        h('div', { class: 'field wide' }, h('label', {}, 'Thème'),
          h('div', { class: 'swatches' }, ...Object.entries(cat.themes).map(([id, name]) =>
            h('div', { class: `swatch${st.theme === id ? ' active' : ''}`, onclick: () => save('setStatus', { theme: id }) },
              h('i', { style: `background: rgb(${THEME_RGB[id]}); box-shadow: 0 0 8px rgb(${THEME_RGB[id]})` }), name)))),
        h('div', { class: 'field wide', style: 'margin-top:14px' },
          h('div', { style: 'display: flex; justify-content: space-between; align-items: center; margin-bottom: 4px;' },
            h('label', { style: 'margin: 0;' }, 'Apparence du cœur'),
            h('a', {
              href: `http://127.0.0.1:${(window.__JARVIS__ && window.__JARVIS__.port) || 8797}/tools/character-preview.html?style=${encodeURIComponent(st.heartStyle || 'classic')}`,
              target: '_blank',
              style: 'font-size: 11px; text-decoration: none; color: var(--accent); cursor: pointer; display: inline-flex; align-items: center; gap: 4px;',
              title: 'Ouvrir dans un navigateur toutes les animations et réglages de chaque type de cœur',
              onclick: (e) => {
                e.preventDefault();
                api('openHeartPreview', st.heartStyle || 'classic');
              }
            }, '👁 Voir toutes les animations et réglages dans le navigateur ↗')
          ),
          h('div', { class: 'swatches' }, ...Object.entries(cat.heartStyles || { classic: 'Cœur holographique' }).map(([id, name]) =>
            h('div', { class: `swatch heart-style${(st.heartStyle || 'classic') === id ? ' active' : ''}`, onclick: () => save('setStatus', { heartStyle: id }) },
              h('span', { class: 'heart-icon' }, HEART_ICONS[id] || '◉'), name))),
          h('div', { class: 'help' }, 'Le personnage réagit à ce que fait Jarvis : il écoute, réfléchit, parle, danse sur la musique, montre le courrier, l’heure, l’agenda… À la voix : « affiche Marvin », « affiche le chat grincheux », « cœur classique ».')),
        h('div', { class: 'grid', style: 'margin-top:14px' },
          field('Voix du personnage', toggle('Le personnage parle avec sa propre voix', st.characterVoice !== false, (v) => save('setStatus', { characterVoice: v })),
            `Bobot, Marvin (robot las et grincheux), Petit Robot et le chat ont chacun leur voix ; le cœur classique garde la voix choisie dans Voix.${st.speech ? ` Voix actuelle : ${st.speech.name}.` : ''}`),
          field('Caractère du personnage', toggle('Le personnage donne son ton aux réponses', st.characterPersona !== false, (v) => save('setStatus', { characterPersona: v })),
            'Marvin répond avec lassitude, le chat en bougonnant, Petit Robot avec entrain. Le contenu des réponses ne change pas.')),
        companionBlock(),
        h('div', { class: 'grid', style: 'margin-top:14px' },
          field('Danse des personnages', select(Object.entries(cat.danceStyles || { auto: 'Automatique (selon la musique)' }), st.danceStyle || 'auto', (v) => save('setStatus', { danceStyle: v })),
            'Quand une musique joue, les personnages du cœur dansent selon son style (calme, pop, électro, rock, rap, latino), d’après le titre, l’artiste et la playlist, ou le tempo et l’énergie du son. Choisis un style pour l’imposer.'),
          field('Densité d’affichage', select(cat.densities, st.density, (v) => save('setStatus', { density: v }))),
          field('Sourdine contextuelle', select(cat.contextMute, (st.contextMute || {}).mode || 'auto', (v) => save('setStatus', { contextMute: v })),
            'Jarvis se tait pendant les jeux, visios et enregistrements.'),
          field('Musique', toggle('Jarvis se tait tant qu’une musique est lancée',
            (S.config.audio || {}).musicSilence !== false, (v) => save('setConfig', { audio: { musicSilence: v } })),
            'Même principe que la sourdine pendant une visio : ses réponses restent écrites à l’écran, sans voix, jusqu’à l’arrêt de la musique. Sans effet si la sourdine contextuelle est sur « Jamais ».'),
          field('Visio Google Meet', toggle('Voix coupée tant qu’une réunion Meet est ouverte : Jarvis répond par écrit',
            (S.config.visio || {}).meetSilence !== false, (v) => save('setConfig', { visio: { meetSilence: v } })),
            'Même si une autre fenêtre est au premier plan. Sans effet si la sourdine contextuelle est sur « Jamais ».'),
          field('Écoute pendant une visio', toggle('Mettre l’écoute de Jarvis en pause tant qu’une réunion Meet est ouverte',
            (S.config.visio || {}).pauseListening !== false, (v) => save('setConfig', { visio: { pauseListening: v } })),
            'Évite que Jarvis transcrive la réunion en tâche de fond (charge du processeur, micro partagé) : les interlocuteurs t’entendent mieux. L’écoute reprend à la fin de la réunion ; F3 la rétablit à la main.'),
          field('Démarrage', toggle('Lancer Jarvis avec Windows', st.autostart, (v) => save('setStatus', { autostart: v })))
        ),
        cardColorsBlock(st),
        cardNumbersBlock(st, S.cardsList || [])
      )
    ];
  };

  // --- Section : Voix & LiveKit -------------------------------------------------------------

  sections.voice = () => {
    const st = S.status;
    const cat = S.catalog;
    const vcfg = S.config.voice;
    const lk = vcfg.livekit;
    const v = S.voice || {};
    const spk = S.speaker || {};
    const setLk = (patch) => save('setConfig', { voice: { livekit: patch } });
    const rateNum = parseInt(String(st.voiceRate || '+0%').replace('%', ''), 10) || 0;
    const badge = v.backend === 'engine'
      ? h('span', { class: 'badge ok' }, 'Moteur vocal v2 actif')
      : v.backend === 'classic' ? h('span', { class: 'badge warn' }, 'Ancien service actif') : h('span', { class: 'badge off' }, 'Arrêtée');
    return [
      h('h1', {}, 'Voix'),
      h('p', { class: 'lead' }, 'Moteur vocal unifié v2, entièrement local : anti-écho WebRTC (AEC3) avec le son du PC comme référence, VAD Silero, mot d’éveil « Jarvis », Whisper large-v3-turbo sur GPU, voix edge-tts avec Piper en secours hors ligne.'),
      panel('Moteur vocal', null,
        h('div', { class: 'status-line', style: 'margin-bottom:14px' }, badge,
          v.fallback ? h('span', { class: 'badge warn' }, `repli : ${v.fallback}`) : null,
          v.missing ? h('span', { class: 'badge err' }, v.missing) : null),
        h('div', { class: 'grid' },
          null,
          null,
          field('Service', h('div', { class: 'row' }, h('button', { class: 'btn', onclick: () => save('voiceRestart') }, '↻ Redémarrer la voix')))
        )
      ),
      panel('Empreintes vocales & Reconnaissance des locuteurs', 'WeSpeaker CAM++ (1:N)',
        h('p', { class: 'hint' }, 'Filtre acoustique biométrique local : Jarvis identifie chaque personne par son empreinte vocale. En filtrage strict, seules les voix reconnues sont prises en compte ; une voix douteuse ou inconnue s’affiche en orange ou en rouge au-dessus de la barre de commande, avec un bouton par personne : un clic apprend la phrase (et l’exécute si elle est récente), « Nouvelle personne » enregistre une nouvelle voix.'),
        h('div', { class: 'status-line', style: 'margin-bottom:14px' },
          enrollState.active
            ? h('span', { class: 'badge warn' }, `🎙 Enregistrement en cours pour ${enrollState.targetName || 'l’utilisateur'} (${enrollState.current}/${enrollState.target})`)
            : (spk.speakers && spk.speakers.length > 0
                ? h('span', { class: 'badge ok' }, `👤 ${spk.speakers.length} profil${spk.speakers.length > 1 ? 's' : ''} enregistré${spk.speakers.length > 1 ? 's' : ''}`)
                : (spk.enrolled
                    ? h('span', { class: 'badge ok' }, `👤 1 profil enregistré`)
                    : h('span', { class: 'badge warn' }, 'Aucune empreinte enregistrée'))),
          spk.enabled ? h('span', { class: 'badge ok' }, 'Filtrage actif') : h('span', { class: 'badge off' }, 'Filtrage suspendu')
        ),
        enrollState.active
          ? h('div', { style: 'background: rgba(var(--rgb), 0.08); border: 1px solid rgba(var(--rgb), 0.35); border-radius: 8px; padding: 14px; margin-bottom: 16px;' },
              h('div', { style: 'font-weight: 600; margin-bottom: 6px; display: flex; align-items: center; justify-content: space-between;' },
                h('span', {}, `Écoute active : phrase ${enrollState.current + 1} sur ${enrollState.target} pour ${enrollState.targetName || 'l’utilisateur'}`),
                h('span', { class: 'badge warn' }, 'Micro en écoute')
              ),
              h('p', { class: 'hint', style: 'margin-bottom: 10px;' },
                'Prononcez 3 phrases naturelles et distinctes (ex. : « Jarvis, quel temps fait-il ? », « Raconte-moi une anecdote », « Quelle heure est-il ? »).'
              ),
              h('div', { style: 'font-size: 0.95em; color: var(--fg); margin-bottom: 12px; min-height: 20px;' },
                enrollState.message || 'Parlez maintenant à voix normale...'
              ),
              h('div', { class: 'row' },
                h('button', {
                  class: 'btn ghost',
                  onclick: () => api('speakerCancel').then(() => {
                    enrollState = { active: false, targetName: '', current: 0, target: 3, message: '' };
                    render();
                  })
                }, 'Annuler l’enregistrement')
              )
            )
          : null,
        // Liste des profils vocaux
        ((spk.speakers && spk.speakers.length > 0) ? h('div', { style: 'margin-bottom: 14px;' },
          h('div', { style: 'font-weight: 600; font-size: 0.9em; margin-bottom: 8px; color: var(--fg); opacity: 0.8;' }, 'Profils vocaux reconnus :'),
          ...spk.speakers.map((s) => h('div', { class: 'row', style: 'background: rgba(var(--rgb), 0.05); border: 1px solid rgba(var(--rgb), 0.15); border-radius: 6px; padding: 8px 12px; margin-bottom: 6px; justify-content: space-between; align-items: center;' },
            h('div', { style: 'display: flex; align-items: center; gap: 10px;' },
              h('span', { style: 'font-size: 1.2em;' }, '👤'),
              h('div', {},
                h('div', { style: 'font-weight: 600;' },
                  s.name,
                  s.isPrimary ? h('span', { class: 'badge ok', style: 'margin-left: 8px; font-size: 0.75em;' }, 'Principal') : null
                ),
                h('div', { class: 'help', style: 'font-size: 0.8em;' },
                  `${s.samplesCount ? `${s.samplesCount} phrases d’enregistrement` : 'créé depuis le HUD'}${s.learned ? ` · ${s.learned} phrase${s.learned > 1 ? 's' : ''} apprise${s.learned > 1 ? 's' : ''}` : ''}${s.enrolledAt ? ` · ${fmtDate(s.enrolledAt)}` : ''}`
                )
              )
            ),
            h('div', { class: 'row', style: 'gap: 6px;' },
              select(SPEAKER_TYPES, s.type || (s.isPrimary ? 'moi' : 'autre'), async (t) => {
                await api('speakerSetType', s.id, t);
                reload();
              }),
              !s.isPrimary ? h('button', {
                class: 'btn ghost',
                title: 'Définir comme locuteur principal',
                onclick: async () => {
                  await api('speakerPrimary', s.id);
                  reload();
                }
              }, '★ Principal') : null,
              h('button', {
                class: 'btn ghost',
                title: 'Réenregistrer la voix',
                disabled: enrollState.active,
                onclick: async () => {
                  enrollState = { active: true, targetName: s.name, current: 0, target: 3, message: `Micro à l’écoute : prononcez la 1ère phrase pour ${s.name}...` };
                  render();
                  await api('speakerEnroll', { name: s.name, count: 8 });
                }
              }, '🎙 Réenregistrer'),
              h('button', {
                class: 'btn ghost',
                title: 'Supprimer ce profil',
                disabled: enrollState.active,
                onclick: async () => {
                  const ok = await confirmDialog(`Supprimer le profil vocal de ${s.name} ?`, "Jarvis ne reconnaîtra plus cette personne.", "Supprimer", true);
                  if (ok) {
                    await api('speakerDelete', s.id);
                    reload();
                  }
                }
              }, '🗑')
            )
          ))
        ) : null),
        // Formulaire d'ajout d'un nouveau profil vocal
        (() => {
          let nameInput, addBtn, typeInput;
          return h('div', { class: 'field wide', style: 'background: rgba(var(--rgb), 0.03); border: 1px dashed rgba(var(--rgb), 0.25); border-radius: 6px; padding: 12px; margin-bottom: 16px;' },
            h('label', { style: 'margin-bottom: 6px;' }, 'Ajouter un nouveau profil vocal'),
            h('div', { class: 'row', style: 'gap: 8px;' },
              nameInput = h('input', {
                type: 'text',
                placeholder: 'Prénom (ex: Marie, Lucas, Sophie...)',
                style: 'max-width: 260px;',
                disabled: enrollState.active,
                onkeydown: (e) => { if (e.key === 'Enter') addBtn.click(); }
              }),
              typeInput = select(SPEAKER_TYPES, 'famille', () => {}),
              addBtn = h('button', {
                class: 'btn primary',
                disabled: enrollState.active,
                onclick: async () => {
                  const val = (nameInput.value || '').trim();
                  if (!val) {
                    nameInput.focus();
                    return;
                  }
                  const cap = val.charAt(0).toUpperCase() + val.slice(1);
                  enrollState = { active: true, targetName: cap, current: 0, target: 3, message: `Micro à l’écoute : prononcez la 1ère phrase pour ${cap}...` };
                  render();
                  await api('speakerEnroll', { name: cap, count: 8, type: typeInput.value });
                }
              }, '🎙 Enregistrer son empreinte')
            ),
            h('div', { class: 'help' }, 'Cette personne prononce 8 phrases variées. Plus simple : quand Jarvis affiche sa phrase en rouge, clique « Nouvelle personne ».')
          );
        })(),
        h('div', { class: 'grid' },
          field('Filtrage actif', toggle('Ne répondre qu’aux voix reconnues', spk.enabled, (val) => api('speakerSet', { enabled: val }).then(reload))),
          field('Voix non reconnue', select({ reject: 'Ignorée (Jarvis demande qui parle)', signal: 'Prise en compte, sans nom' }, spk.mode || 'reject',
            (m) => api('speakerOptions', { mode: m }).then(reload)),
            'Strict : seules les voix reconnues sont exécutées ; les autres attendent un clic sur « Qui parle ? ».'),
          field('Marge de doute', range(0, 0.3, 0.05, spk.doubtMargin != null ? spk.doubtMargin : 0.15, (n) => `${Math.round(n * 100)} pts`, (n) => api('speakerOptions', { doubtMargin: n }).then(reload)),
            'Sous le seuil, dans cette marge : voix « douteuse » (orange). En dessous : voix inconnue (rouge).'),
          field('Tolérance près du seuil', range(0, 0.2, 0.02, spk.nearMiss != null ? spk.nearMiss : 0.12, (n) => (n === 0 ? 'aucune' : `${Math.round(n * 100)} pts`), (n) => api('speakerOptions', { nearMiss: n }).then(reload)),
            'Quand tu dis « Jarvis » et que ta voix est tout près du seuil (ex. 0,40 pour 0,50 : 12 points par défaut), la demande est exécutée au lieu d’être ignorée. Les actions sensibles gardent leur code de validation. 0 : désactivé.'),
          field('Apprentissage', toggle('Demander qui parle et apprendre les voix', spk.learning !== false, (val) => api('speakerOptions', { learning: val }).then(reload)),
            'Aussi dans la zone CONVERSATION du HUD : bouton APPRENTISSAGE (ou Maj+F5). Coupé, une voix non reconnue est ignorée sans question et les empreintes ne changent plus.'),
          field('Apprentissage automatique', toggle('Apprendre les phrases reconnues avec assurance', spk.autoLearn !== false && spk.learning !== false, (val) => api('speakerOptions', { autoLearn: val }).then(reload)),
            'Au plus une phrase toutes les 90 s, seulement quand la voix est reconnue bien au-dessus du seuil : l’empreinte s’adapte à ton micro et à ta pièce.'),
          field('Sensibilité (seuil)', range(0.40, 0.85, 0.05, spk.threshold || 0.65, (n) => `${Math.round(n * 100)} %`, (n) => api('speakerSet', { threshold: n }).then(reload)),
            'Plus le seuil est élevé (65-75%), plus le filtrage est strict contre les films et la musique.'),
          field('Réinitialisation', h('div', { class: 'row' },
            spk.enrolled ? h('button', {
              class: 'btn',
              disabled: enrollState.active,
              onclick: async () => {
                const ok = await confirmDialog("Effacer toutes les empreintes vocales ?", "Jarvis réagira de nouveau à toutes les voix environnantes (films, musiques, tiers).", "Tout effacer", true);
                if (ok) {
                  await api('speakerReset');
                  reload();
                }
              }
            }, '🗑 Effacer tous les profils') : null
          ))
        )
      ),
      panel('Voix de Jarvis', null,
        h('div', { class: 'grid' },
          field('Voix', select(cat.voices.map((x) => [x.key, `${x.name} · ${x.gender} · ${x.origin}`]),
            (cat.voices.find((x) => x.id === st.voice) || {}).key || 'denise', (val) => save('setStatus', { voice: val }))),
          field('Débit', range(-50, 50, 5, rateNum, (n) => `${n >= 0 ? '+' : ''}${n} %`, (n) => save('setStatus', { voiceRate: `${n >= 0 ? '+' : ''}${n}%` }))),
          field('Essai', h('div', { class: 'row' }, h('button', { class: 'btn', onclick: () => api('previewVoice') }, '▶ Écouter la voix'))),
          field('Mode d’écoute', select(cat.listening, st.listeningMode, (val) => save('setStatus', { listeningMode: val })))
        )
      ),
      panel('Tours de parole', 'moteur v2',
        h('p', { class: 'hint' }, 'Jarvis relit la fin de ta phrase pendant le silence : si elle semble finie, il répond après le délai minimal ; si elle appelle une suite (« et », « de », « le projet », « euh », « … », un verbe sans complément), il t’attend jusqu’au délai maximal. Il te coupe trop tôt : monte le délai minimal (0,7 à 0,9 s).'),
        h('div', { class: 'grid' },
          null,
          field('Délai minimal', range(0.1, 1.5, 0.05, lk.minEndpointingDelay, (n) => `${n.toFixed(2)} s`, (n) => setLk({ minEndpointingDelay: n })),
            'Silence qui termine une phrase qui semble finie. Plus court = plus réactif, plus long = plus de temps pour réfléchir.'),
          field('Délai maximal', range(0.5, 6, 0.1, lk.maxEndpointingDelay, (n) => `${n.toFixed(1)} s`, (n) => setLk({ maxEndpointingDelay: n })),
            'Silence toléré quand la phrase semble inachevée.'),
          field('Silence de segmentation', range(0.15, 1, 0.05, lk.vadSilence, (n) => `${n.toFixed(2)} s`, (n) => setLk({ vadSilence: n })))
        )
      ),
      panel('Interruptions', null,
        h('div', { class: 'grid' },
          field('Parler pendant qu’il parle', toggle('Autorisé', lk.allowInterruptions, (val) => setLk({ allowInterruptions: val })),
            'Jarvis baisse la voix et t’écoute ; un « oui » la fait remonter, une vraie demande le coupe.'),
          field('Délai avant qu’il t’écoute', range(0.2, 2, 0.1, lk.minInterruptionDuration, (n) => `${n.toFixed(1)} s`, (n) => setLk({ minInterruptionDuration: n })),
            'Voix nette et maintenue nécessaire. 0,3 s : réactif. Plus haut si son propre écho le fait baisser la voix.'),
          field('Fausse interruption', toggle('Reprendre la phrase après un bruit', lk.resumeFalseInterruption, (val) => setLk({ resumeFalseInterruption: val })))
        )
      ),
      panel('Traitement audio', 'module WebRTC',
        h('div', { class: 'grid' },
          field('Anti-écho', toggle('Annulation d’écho (AEC3)', lk.echoCancellation, (val) => setLk({ echoCancellation: val }))),
          field('Bruit', toggle('Réduction de bruit', lk.noiseSuppression, (val) => setLk({ noiseSuppression: val }))),
          field('Gain', toggle('Gain automatique', lk.autoGainControl, (val) => setLk({ autoGainControl: val }))),
          field('Son du PC', toggle('Retirer vidéos et musique du micro', lk.systemAudioReference !== false, (val) => setLk({ systemAudioReference: val })),
            'Le son de sortie de Windows sert de référence à l’anti-écho. Il n’est ni enregistré ni transmis.'),
          field('Port local', h('input', { type: 'number', min: 1024, max: 65000, value: lk.port, onchange: (e) => setLk({ port: Number(e.target.value) }) }),
            `Signalisation ${lk.port}, TCP ${lk.port + 1}, UDP ${lk.port + 2} — 127.0.0.1 uniquement.`)
        )
      ),
      panel('Transcription', 'Whisper local (GPU)',
        h('div', { class: 'grid' },
          field('Modèle', select({
            turbo: 'large-v3-turbo (généraliste, rapide)',
            'fr-distil': 'Whisper français distillé (affiné, rapide)',
            fr: 'Whisper français complet (affiné, plus lent, ~3 Go de mémoire vidéo)'
          }, lk.sttModel || 'turbo', (val) => setLk({ sttModel: val })),
          'Changer de modèle redémarre la voix. Compare-les d’abord avec le banc d’essai ci-dessous.'),
          field('Précision du décodage', range(1, 5, 1, lk.sttBeam || 2, (n) => `faisceau ${n}`, (n) => setLk({ sttBeam: n })),
            '1 = le plus rapide, 5 = le plus précis (un peu plus lent).')
        )
      ),
      panel('Mot d’éveil', 'détecteurs acoustiques',
        h('p', { class: 'hint' }, 'Le détecteur phonétique (sherpa-onnx) reconnaît « Jarvis » sans entraînement. Le modèle openWakeWord, entraîné pour « Jarvis » avec wakeword-generator (voix françaises de synthèse), se dépose dans data/models/oww/ ; ses deux modèles de prétraitement sont téléchargés au premier usage. « Accord » réduit les faux réveils, « L’un ou l’autre » les oublis. Changer de mode redémarre la voix.'),
        h('div', { class: 'grid' },
          field('Détecteur', select({
            phonetique: 'Phonétique seul (par défaut)',
            oww: 'openWakeWord seul',
            union: 'L’un ou l’autre',
            accord: 'Les deux d’accord (à 1,5 s près)'
          }, lk.wakeEngine || 'phonetique', (val) => setLk({ wakeEngine: val }))),
          field('Modèle openWakeWord', textInput(lk.wakeModel || 'jarvis.onnx', { onchange: (e) => setLk({ wakeModel: e.target.value.trim() || 'jarvis.onnx' }) }),
            'Nom du fichier dans data/models/oww/.'),
          field('Seuil openWakeWord', range(0.2, 0.9, 0.05, lk.wakeThreshold || 0.5, (n) => n.toFixed(2), (n) => setLk({ wakeThreshold: n })),
            'Plus bas : réagit plus facilement ; plus haut : moins de faux réveils.')
        )
      ),
      corpusPanel()
    ];
  };

  // --- Assistant voix : mise en place guidée, étape par étape (chantier v6) ------------------
  // Détecte ce qui est déjà fait, bascule casque / micro du PC, fait lire les phrases une à une
  // (passage automatique à la suivante), puis lance le calibrage et le choix du modèle.

  const MIN_PHRASES = 20; // minimum conseillé par configuration (idéal : toutes)
  const ENROLL_LINES = [
    'Bonjour Jarvis, je suis en train d’enregistrer ma voix.',
    'Aujourd’hui il fait plutôt beau et je travaille depuis ce matin.',
    'Peux-tu me lire mes derniers mails, s’il te plaît ?',
    'Mets un peu de musique douce pendant que je réfléchis.',
    'Je dois appeler le garage demain pour la révision de la voiture.',
    'Rappelle-moi d’acheter du pain et des fruits en rentrant.',
    'Quelle est la météo prévue pour le week-end à Lyon ?',
    'Merci Jarvis, c’est parfait, on continue comme ça.'
  ];
  const wiz = { data: null, step: null, guide: null, timer: null, busy: false, applied: false };
  // Choix « voix de synthèse » à l'étape 7 (pas d'autre appareil pour la série négative).
  const wizSynth = () => { try { return localStorage.getItem('wiz.synth') === '1'; } catch { return false; } };
  const setWizSynth = (on) => { try { localStorage.setItem('wiz.synth', on ? '1' : '0'); } catch {} };

  function wizLoop(fast) {
    clearTimeout(wiz.timer);
    wiz.timer = setTimeout(async () => {
      if (current !== 'assistant') {
        wiz.timer = null; // relancé au retour sur la section
        return;
      }
      const data = await bridge.call('voiceSetupState').catch(() => null);
      if (data && current === 'assistant') {
        wiz.data = data;
        guideTick();
        render(true);
      }
      const active = (wiz.guide && wiz.guide.running) || enrollState.active || (data && data.bench && data.bench.running)
        || (data && data.corpus.status && ['ecoute', 'capte'].includes(data.corpus.status.state));
      wizLoop(active);
    }, fast ? 700 : 3000);
  }

  // Lecture guidée : dès qu'une phrase est enregistrée, la suivante démarre toute seule.
  function guideNext(config) {
    const d = wiz.data;
    const skipped = (wiz.guide && wiz.guide.skipped) || new Set();
    return d.corpus.phrases.find((p) => !p.takes[config] && !skipped.has(p.id)) || null;
  }

  async function guideStart(config) {
    wiz.guide = { config, running: true, skipped: (wiz.guide && wiz.guide.config === config && wiz.guide.skipped) || new Set(), waitingId: null, startedAt: 0, message: '' };
    await guideLaunch();
  }

  async function guideLaunch() {
    const g = wiz.guide;
    if (!g || !g.running) return;
    const next = guideNext(g.config);
    if (!next) {
      g.running = false;
      g.message = 'Toutes les phrases de cette configuration sont enregistrées.';
      render(true);
      return;
    }
    g.waitingId = next.id;
    g.startedAt = Date.now();
    g.message = '';
    await api('corpusRecord', { action: 'phrase', id: next.id });
    wizLoop(true);
    render(true);
  }

  function guideTick() {
    const g = wiz.guide;
    const st = wiz.data && wiz.data.corpus.status;
    if (!g || !g.running || !g.waitingId) return;
    const mine = st && st.item && st.item.id === g.waitingId;
    if (mine && st.state === 'enregistre') {
      g.waitingId = null;
      setTimeout(guideLaunch, 900); // courte respiration avant la phrase suivante
    } else if (mine && st.state === 'expire') {
      g.running = false;
      g.waitingId = null;
      g.message = 'Je n’ai rien entendu pendant 20 secondes : la lecture est en pause. Clique sur « Reprendre » quand tu es prêt.';
    } else if (!mine && Date.now() - g.startedAt > 30000) {
      g.running = false;
      g.waitingId = null;
      g.message = 'La voix ne répond pas (Jarvis est-il démarré ?). Clique sur « Reprendre » pour réessayer.';
    }
  }

  async function guidePause() {
    if (wiz.guide) {
      wiz.guide.running = false;
      wiz.guide.waitingId = null;
    }
    await api('corpusRecord', { action: 'stop' });
    render(true);
  }

  function wizSteps(d) {
    const primary = (d.speaker.speakers || []).find((s) => s.isPrimary) || (d.speaker.speakers || [])[0] || null;
    const configs = (primary && primary.configs) || [];
    const count = (c) => d.corpus.phrases.filter((p) => p.takes[c]).length;
    const total = d.corpus.phrases.length;
    const thresholds = d.speaker.thresholds || {};
    return [
      { id: 'casque', title: 'Passer sur le casque', done: d.audio.mode === 'casque' || (configs.includes('casque') && count('casque') >= MIN_PHRASES) },
      { id: 'enroll-casque', title: 'Empreinte vocale au casque', done: configs.includes('casque') },
      { id: 'corpus-casque', title: `Lire les phrases au casque (${count('casque')}/${total})`, done: count('casque') >= MIN_PHRASES },
      { id: 'pc', title: 'Passer sur le micro de l’ordinateur', done: d.audio.mode === 'haut-parleurs' || (configs.includes('haut-parleurs') && count('haut-parleurs') >= MIN_PHRASES) },
      { id: 'enroll-pc', title: 'Empreinte vocale sur l’ordinateur', done: configs.includes('haut-parleurs') },
      { id: 'corpus-pc', title: `Lire les phrases sur l’ordinateur (${count('haut-parleurs')}/${total})`, done: count('haut-parleurs') >= MIN_PHRASES },
      { id: 'negatif', title: `Autres voix (${wizSynth() ? 'voix de synthèse' : `${d.corpus.negatives.count} segments`})`, done: d.corpus.negatives.count >= 8 || wizSynth() },
      { id: 'calibrer', title: 'Calibrer le filtre de voix', done: Object.keys(thresholds).length > 0 },
      { id: 'modele', title: 'Choisir le modèle de transcription', done: Boolean(d.corpus.report && !d.corpus.report.synthetique) },
      { id: 'fin', title: 'Terminé', done: false }
    ];
  }

  sections.assistant = () => {
    if (!wiz.data) {
      wizLoop(true);
      return [h('h1', {}, 'Assistant voix'), h('p', { class: 'lead' }, 'Chargement…')];
    }
    if (!wiz.timer) wizLoop(false);
    const d = wiz.data;
    const steps = wizSteps(d);
    const firstTodo = steps.find((s) => !s.done) || steps[steps.length - 1];
    const stepId = wiz.step && steps.some((s) => s.id === wiz.step) ? wiz.step : firstTodo.id;
    const index = steps.findIndex((s) => s.id === stepId);
    const doneCount = steps.filter((s) => s.done).length;
    const go = (id) => { wiz.step = id; render(true); };
    return [
      h('h1', {}, 'Assistant voix'),
      h('p', { class: 'lead' }, 'Mise en place de la reconnaissance vocale, étape par étape : ton empreinte, tes phrases au casque puis sur l’ordinateur, une série « vidéo », puis le calibrage. Tout reste sur ce PC. Compte 15 à 20 minutes en tout ; tu peux t’arrêter et reprendre à tout moment.'),
      h('div', { class: 'wiz' },
        h('ol', { class: 'wiz-steps' }, ...steps.map((s, i) => h('li', {
          class: `${s.done ? 'done' : ''} ${s.id === stepId ? 'current' : ''}`,
          onclick: () => go(s.id)
        }, h('span', { class: 'wiz-num' }, s.done ? '✓' : String(i + 1)), s.title))),
        h('div', { class: 'wiz-card' },
          h('div', { class: 'wiz-progress' }, h('i', { style: `width:${Math.round((doneCount / (steps.length - 1)) * 100)}%` })),
          h('div', { class: 'wiz-kicker' }, `Étape ${index + 1} sur ${steps.length}`),
          ...wizStep(stepId, d, steps),
          h('div', { class: 'wiz-nav' },
            index > 0 ? h('button', { class: 'btn ghost', onclick: () => go(steps[index - 1].id) }, '← Précédent') : h('span'),
            index < steps.length - 1 ? h('button', { class: 'btn', onclick: () => go(steps[index + 1].id) }, 'Suivant →') : null)))
    ];
  };

  function wizStep(id, d, steps) {
    const primary = (d.speaker.speakers || []).find((s) => s.isPrimary) || (d.speaker.speakers || [])[0] || null;
    const name = (primary && primary.name) || d.userName || 'Johann';
    const done = (sid) => steps.find((s) => s.id === sid).done;
    const audioLine = h('div', { class: 'status-line' }, 'Sortie audio actuelle : ',
      h('b', {}, d.audio.mode === 'casque' ? 'casque' : d.audio.mode === 'haut-parleurs' ? 'micro et haut-parleurs du PC' : 'inconnue'));
    const switchTo = (mode) => async () => {
      wiz.busy = true;
      render(true);
      await save('audioSwitch', mode);
      wiz.busy = false;
      setTimeout(() => wizLoop(true), 1500); // la voix rouvre ses périphériques
    };
    const enroll = (config) => {
      const has = primary && (primary.configs || []).includes(config);
      if (enrollState.active) {
        return [
          h('div', { class: 'wiz-big' }, `🎙 Parle maintenant : phrase ${Math.min(enrollState.current + 1, enrollState.target)} sur ${enrollState.target}`),
          h('div', { class: 'wiz-meter' }, h('i', { style: `width:${Math.round((enrollState.current / enrollState.target) * 100)}%` })),
          h('p', { class: 'hint' }, 'Lis les phrases ci-dessous à voix normale, une par une, avec une petite pause entre chacune :'),
          h('ol', { class: 'wiz-lines' }, ...ENROLL_LINES.map((l, i) => h('li', { class: i < enrollState.current ? 'done' : i === enrollState.current ? 'current' : '' }, l))),
          h('button', { class: 'btn ghost', onclick: () => save('speakerCancel') }, 'Annuler')
        ];
      }
      return [
        h('p', {}, has
          ? `✓ L’empreinte de ${name} est déjà enregistrée pour cette configuration. Tu peux la refaire si Jarvis te reconnaît mal.`
          : `Jarvis va apprendre ta voix dans cette configuration. Tu liras 8 phrases courtes affichées ici.`),
        h('p', { class: 'hint' }, `Même prénom (« ${name} ») : cette empreinte s’ajoute à ton profil, elle ne crée pas une autre personne.`),
        h('button', { class: `btn ${has ? '' : 'primary'}`, onclick: () => api('speakerEnroll', { name, count: 8 }) }, has ? '↻ Refaire l’empreinte' : '🎙 Commencer (8 phrases)')
      ];
    };
    const reader = (config, label) => {
      const g = wiz.guide && wiz.guide.config === config ? wiz.guide : null;
      const count = d.corpus.phrases.filter((p) => p.takes[config]).length;
      const total = d.corpus.phrases.length;
      const st = d.corpus.status;
      const phrase = g && g.waitingId ? d.corpus.phrases.find((p) => p.id === g.waitingId) : null;
      const heard = st && phrase && st.item && st.item.id === phrase.id && st.state === 'capte';
      const wrongMode = d.audio.mode && d.audio.mode !== config;
      return [
        wrongMode ? h('div', { class: 'wiz-warn' }, `⚠ La sortie audio n’est pas « ${label} ». Reviens à l’étape de bascule avant de lire.`) : null,
        h('div', { class: 'wiz-meter' }, h('i', { style: `width:${Math.round((count / total) * 100)}%` })),
        h('div', { class: 'status-line' }, h('span', {}, h('b', {}, `${count}/${total}`), ' phrases enregistrées'),
          h('span', { class: 'help' }, count >= MIN_PHRASES ? '✓ minimum atteint — continuer jusqu’au bout améliore la mesure' : `minimum conseillé : ${MIN_PHRASES}`)),
        phrase
          ? h('div', { class: `wiz-phrase ${heard ? 'heard' : ''}` }, h('div', { class: 'wiz-kicker' }, heard ? 'Je t’entends… finis la phrase puis marque une pause' : 'Lis à voix haute :'), phrase.text)
          : h('div', { class: 'wiz-phrase idle' }, g && g.message ? g.message : count >= total ? '✓ Toutes les phrases sont enregistrées.' : 'Clique sur « Commencer » : la première phrase s’affiche, lis-la, puis la suivante arrive toute seule.'),
        h('div', { class: 'row' },
          g && g.running
            ? [h('button', { class: 'btn', onclick: guidePause }, '⏸ Pause'),
               h('button', { class: 'btn ghost', onclick: async () => { if (g.waitingId) g.skipped.add(g.waitingId); await api('corpusRecord', { action: 'stop' }); g.waitingId = null; guideLaunch(); } }, 'Passer cette phrase')]
            : h('button', { class: 'btn primary', disabled: count >= total || wrongMode, onclick: () => guideStart(config) }, g ? '▶ Reprendre' : '▶ Commencer la lecture guidée'))
      ];
    };

    switch (id) {
      case 'casque':
        return [h('h2', {}, 'Mets ton casque'),
          h('p', {}, 'On commence avec le casque (micro du casque et son dans les oreilles). Jarvis bascule lui-même la sortie : ne change pas le périphérique depuis les paramètres de Windows.'),
          audioLine,
          d.audio.mode === 'casque'
            ? h('div', { class: 'wiz-ok' }, '✓ Le casque est actif. Passe à l’étape suivante.')
            : h('button', { class: 'btn primary', disabled: wiz.busy, onclick: switchTo('casque') }, wiz.busy ? 'Bascule…' : '🎧 Passer sur le casque')];
      case 'enroll-casque':
        return [h('h2', {}, 'Empreinte vocale au casque'), ...enroll('casque')];
      case 'corpus-casque':
        return [h('h2', {}, 'Lecture des phrases au casque'),
          h('p', { class: 'hint' }, 'Ces enregistrements servent à mesurer la reconnaissance et à régler le filtre de voix. Pendant la lecture, Jarvis ne répond pas.'),
          ...reader('casque', 'casque')];
      case 'pc':
        return [h('h2', {}, 'Passe sur le micro de l’ordinateur'),
          h('p', {}, 'Retire le casque. Jarvis bascule sur le micro et les haut-parleurs du PC.'),
          audioLine,
          d.audio.mode === 'haut-parleurs'
            ? h('div', { class: 'wiz-ok' }, '✓ Le micro de l’ordinateur est actif. Place-toi à ta distance habituelle de l’écran.')
            : h('button', { class: 'btn primary', disabled: wiz.busy, onclick: switchTo('pc') }, wiz.busy ? 'Bascule…' : '🖥 Passer sur le micro du PC')];
      case 'enroll-pc':
        return [h('h2', {}, 'Empreinte vocale sur l’ordinateur'), ...enroll('haut-parleurs')];
      case 'corpus-pc':
        return [h('h2', {}, 'Lecture des phrases sur l’ordinateur'),
          h('p', { class: 'hint' }, 'Mêmes phrases, cette fois au micro de l’ordinateur, à ta distance habituelle.'),
          ...reader('haut-parleurs', 'micro et haut-parleurs du PC')];
      case 'negatif': {
        const st = d.corpus.status;
        const running = st && st.mode === 'negatif' && ['ecoute', 'capte'].includes(st.state);
        const finished = st && st.mode === null && st.state === 'termine' && st.item && st.item.kind === 'negatif';
        return [h('h2', {}, 'Série « autres voix » : ne parle pas'),
          h('p', {}, 'Jarvis doit apprendre à ignorer les voix qui ne sont pas la tienne : télévision, radio, téléphone, personnes dans la pièce.'),
          h('div', { class: 'wiz-ok' }, 'Le son joué par cet ordinateur (YouTube, musique, visio) est déjà retiré du micro par l’anti-écho : une vidéo lancée sur le PC ne s’entend plus du tout. C’est pour ça qu’elle ne donne aucun segment.'),
          h('p', {}, h('b', {}, 'Lance donc la vidéo sur un autre appareil : '), 'ton téléphone ou une tablette, posé à 30 à 50 cm du micro, volume assez fort (plus fort qu’une conversation). Le micro atténue beaucoup les sons lointains : de loin, presque rien n’est capté. Clique sur le bouton, puis ne dis rien pendant 60 secondes.'),
          h('div', { class: 'status-line' }, h('span', {}, 'Déjà enregistré : ', h('b', {}, `${d.corpus.negatives.count} segments (${d.corpus.negatives.seconds} s)`))),
          running
            ? [h('div', { class: 'wiz-big' }, `🎬 Écoute… ${st.segments || 0} segment(s) captés`), h('button', { class: 'btn ghost', onclick: () => api('corpusRecord', { action: 'stop' }) }, 'Arrêter')]
            : h('button', { class: 'btn primary', onclick: async () => { await api('corpusRecord', { action: 'negatif', seconds: 60 }); wizLoop(true); } }, d.corpus.negatives.count ? '🎬 Refaire une série (60 s)' : '🎬 Lancer la série (60 s)'),
          finished && !st.segments
            ? h('div', { class: 'wiz-warn' }, 'Série terminée sans aucun segment. Si la vidéo passait sur ce PC, c’est normal : l’anti-écho la retire. Rejoue-la sur un téléphone ou une télé près du micro, un peu plus fort.')
            : null,
          d.corpus.negatives.count && d.corpus.negatives.count < 8 ? h('p', { class: 'hint' }, 'Peu de segments captés : monte un peu le son de l’appareil et relance une série.') : null,
          h('div', { style: 'margin-top:8px' },
            h('p', {}, h('b', {}, 'Ou, sans appareil : '), 'Jarvis compare ton empreinte à une dizaine de voix de synthèse (hommes et femmes, accents de France, Belgique, Suisse et Canada). C’est un bon étalon des « autres voix », même s’il vaut moins qu’une vraie télé dans la pièce.'),
            wizSynth()
              ? h('div', { class: 'row' }, h('span', { class: 'badge ok' }, '✓ Voix de synthèse retenues pour le calibrage'),
                h('button', { class: 'btn ghost', onclick: () => { setWizSynth(false); render(true); } }, 'Annuler ce choix'))
              : h('button', { class: 'btn', onclick: () => { setWizSynth(true); wiz.step = 'calibrer'; render(true); } }, '🗣 Utiliser des voix de synthèse et continuer'))];
      }
      case 'calibrer': {
        const bench = d.bench;
        const rep = d.corpus.speakerReport;
        const running = bench && bench.running && bench.kind === 'speaker';
        const ready = done('corpus-casque') || done('corpus-pc');
        return [h('h2', {}, 'Calibrer le filtre de voix'),
          h('p', {}, 'Jarvis compare tes phrases et la série « vidéo » à ton empreinte, puis propose un seuil par durée de phrase : le plus sévère possible envers les vidéos, sans te rejeter.'),
          !ready ? h('div', { class: 'wiz-warn' }, 'Enregistre d’abord au moins une série de phrases.') : null,
          !done('negatif') ? h('p', { class: 'hint' }, 'Sans « autres voix » (étape 7), les seuils sont calculés sur tes phrases seules.') : null,
          wizSynth() || (d.corpus.negatives.count > 0 && d.corpus.negatives.count < 8)
            ? h('p', { class: 'hint' }, 'Les voix de synthèse sont ajoutées aux autres voix pour ce calibrage (préparation ~1 min la première fois).') : null,
          running ? h('div', { class: 'wiz-big' }, `⚖ Calibrage en cours… ${((bench.log || []).slice(-1)[0] || '').slice(0, 90)}`)
            : h('button', { class: 'btn primary', disabled: !ready, onclick: async () => {
              wiz.applied = false;
              const synthetic = wizSynth() || d.corpus.negatives.count < 8;
              await save('benchRun', { kind: 'speaker', synthetic });
              wizLoop(true);
            } }, rep ? '↻ Recalibrer' : '⚖ Calibrer'),
          rep && rep.voix_synthese ? h('p', { class: 'hint' }, 'Calibrage fait avec les voix de synthèse comme « autres voix ».') : null,
          rep && rep.tranches ? h('table', { style: 'margin:12px 0' },
            h('thead', {}, h('tr', {}, h('th', {}, 'Phrases'), h('th', {}, 'Seuil conseillé'), h('th', {}, 'Tes phrases rejetées'), h('th', {}, 'Vidéo acceptée'))),
            h('tbody', {}, ...Object.entries(rep.tranches).filter(([, t]) => t.phrases).map(([b, t]) => h('tr', {},
              h('td', {}, { court: 'courtes (< 1,2 s)', moyen: 'moyennes', long: 'longues (> 2,5 s)' }[b] || b),
              h('td', {}, String(t.seuil)),
              h('td', {}, `${t.recommande.rejets_a_tort}/${t.phrases} (avant : ${t.actuel.rejets_a_tort})`),
              h('td', {}, `${t.recommande.acceptes_a_tort}/${t.negatifs} (avant : ${t.actuel.acceptes_a_tort})`))))) : null,
          rep && rep.seuils && Object.keys(rep.seuils).length
            ? h('button', { class: 'btn primary', onclick: async () => { await save('speakerApplyThresholds', rep.seuils); wiz.applied = true; wizLoop(true); } }, done('calibrer') ? '✓ Seuils appliqués — réappliquer' : '✓ Appliquer ces seuils')
            : null,
          bench && !bench.running && bench.kind === 'speaker' && bench.code ? h('pre', { class: 'help' }, (bench.log || []).slice(-6).join('\n')) : null];
      }
      case 'modele': {
        const bench = d.bench;
        const rep = d.corpus.report;
        const running = bench && bench.running && bench.kind === 'stt';
        const best = rep && rep.resultats ? rep.resultats.find((r) => r.label === rep.meilleur) : null;
        const bestModel = best ? (best.label.split(' ')[0]) : null;
        return [h('h2', {}, 'Choisir le modèle de transcription (facultatif)'),
          h('p', {}, 'Jarvis rejoue tes phrases avec chaque modèle et mesure les erreurs, les commandes comprises et la latence. Compte quelques minutes.'),
          running ? h('div', { class: 'wiz-big' }, `⚗ Mesure en cours… ${((bench.log || []).slice(-1)[0] || '').slice(0, 80)}`)
            : h('button', { class: 'btn primary', disabled: !(done('corpus-casque') || done('corpus-pc')), onclick: async () => { await save('benchRun', { kind: 'stt' }); wizLoop(true); } }, rep ? '↻ Refaire la mesure' : '⚗ Comparer les modèles'),
          rep && rep.resultats ? h('table', { style: 'margin:12px 0' },
            h('thead', {}, h('tr', {}, h('th', {}, 'Réglage'), h('th', {}, 'Erreurs'), h('th', {}, 'Commandes'), h('th', {}, 'Latence'))),
            h('tbody', {}, ...rep.resultats.map((r) => h('tr', {},
              h('td', {}, r.label === rep.meilleur ? h('b', {}, `★ ${r.label}`) : r.label),
              h('td', {}, `${r.wer} %`), h('td', {}, r.commandes == null ? '—' : `${r.commandes} %`), h('td', {}, `${r.latence_ms} ms`))))) : null,
          bestModel && ['turbo', 'fr-distil', 'fr'].includes(bestModel)
            ? h('div', { class: 'row' },
              h('span', {}, 'Modèle actuel : ', h('b', {}, d.sttModel)),
              bestModel !== d.sttModel
                ? h('button', { class: 'btn primary', onclick: () => save('setConfig', { voice: { livekit: { sttModel: bestModel } } }) }, `Utiliser ${bestModel} (redémarre la voix)`)
                : h('span', { class: 'badge ok' }, '✓ le meilleur modèle est déjà utilisé'))
            : null];
      }
      default:
        return [h('h2', {}, steps.filter((s) => s.id !== 'fin').every((s) => s.done) ? '✓ Tout est en place' : 'Presque fini'),
          h('ul', { class: 'wiz-summary' }, ...steps.filter((s) => s.id !== 'fin').map((s) => h('li', { class: s.done ? 'done' : '' }, `${s.done ? '✓' : '○'} ${s.title}`))),
          h('p', {}, 'Tu peux revenir sur le casque quand tu veux. Si Jarvis te reconnaît moins bien un jour (nouveau micro, pièce différente), refais simplement l’empreinte et le calibrage.'),
          d.audio.mode !== 'casque' ? h('button', { class: 'btn', onclick: switchTo('casque') }, '🎧 Revenir sur le casque') : null];
    }
  }

  // --- Corpus vocal, banc d'essai et calibrage (chantier v6) --------------------------------

  let corpusTimer = null;
  function corpusPanel() {
    const box = h('div', {}, h('div', { class: 'help' }, 'Chargement du corpus…'));
    const refresh = async () => {
      const st = await bridge.call('corpusState').catch(() => null);
      if (!st || !box.isConnected) return;
      box.replaceChildren(...renderCorpus(st, refresh));
      clearTimeout(corpusTimer);
      const busy = (st.status && ['ecoute', 'capte'].includes(st.status.state)) || (st.bench && st.bench.running);
      corpusTimer = setTimeout(() => { if (box.isConnected && current === 'voice') refresh(); }, busy ? 1000 : 5000);
    };
    setTimeout(refresh, 0);
    return panel('Corpus vocal & mesures', 'enregistré sur ce PC uniquement', box);
  }

  function renderCorpus(st, refresh) {
    const configs = ['casque', 'haut-parleurs'];
    const done = (p) => configs.filter((c) => p.takes[c]).length;
    const total = st.phrases.length;
    const count = (c) => st.phrases.filter((p) => p.takes[c]).length;
    const status = st.status;
    const listening = status && ['ecoute', 'capte'].includes(status.state);
    const recorded = count('casque') + count('haut-parleurs') > 0;
    const record = async (id) => { await api('corpusRecord', { action: 'phrase', id }); setTimeout(refresh, 300); };
    const rows = st.phrases.map((p) => h('tr', {},
      h('td', {}, p.text),
      h('td', {}, configs.map((c) => (p.takes[c] ? `✓ ${c} (${p.takes[c].duration} s)` : '')).filter(Boolean).join(' · ') || '—'),
      h('td', {}, h('button', { class: 'btn small', disabled: listening, onclick: () => record(p.id) }, done(p) ? '↻ Relire' : '🎙 Lire'))));
    const bench = st.bench;
    const report = st.report;
    const spk = st.speakerReport;
    return [
      h('p', { class: 'hint' },
        'Lis chaque phrase après avoir cliqué sur « Lire » : Jarvis garde l’enregistrement au lieu d’y répondre. ',
        'Fais une série au casque puis une série sur les haut-parleurs (la configuration est détectée d’après la sortie audio). ',
        'La série « négative » enregistre une vidéo ou de la musique pendant que tu te tais : ce que Jarvis ne doit jamais prendre pour une demande.'),
      h('div', { class: 'status-line', style: 'margin-bottom:12px' },
        h('span', {}, 'Casque : ', h('b', {}, `${count('casque')}/${total}`)),
        h('span', {}, 'Haut-parleurs : ', h('b', {}, `${count('haut-parleurs')}/${total}`)),
        h('span', {}, 'Négatifs : ', h('b', {}, `${st.negatives.count} segments (${st.negatives.seconds} s)`)),
        listening ? h('span', { class: 'badge warn' },
          status.mode === 'negatif' ? `🎙 Série négative en cours (${status.segments || 0} segments)` : `🎙 Parle maintenant : « ${(status.item && status.item.text) || ''} »`) : null),
      h('div', { class: 'row', style: 'margin-bottom:12px' },
        h('button', { class: 'btn', disabled: listening, onclick: async () => { await api('corpusRecord', { action: 'negatif', seconds: 60 }); setTimeout(refresh, 300); } }, '🎬 Série négative (60 s)'),
        listening ? h('button', { class: 'btn ghost', onclick: async () => { await api('corpusRecord', { action: 'stop' }); setTimeout(refresh, 300); } }, '■ Arrêter') : null,
        h('button', { class: 'btn primary', disabled: (bench && bench.running) || !recorded, title: recorded ? '' : 'Enregistre d’abord des phrases', onclick: async () => { await save('benchRun', { kind: 'stt' }); setTimeout(refresh, 500); } }, '⚗ Comparer les modèles'),
        h('button', { class: 'btn', disabled: (bench && bench.running) || !recorded, title: recorded ? '' : 'Enregistre d’abord des phrases', onclick: async () => { await save('benchRun', { kind: 'speaker' }); setTimeout(refresh, 500); } }, '⚖ Calibrer le filtre de voix'),
        h('button', { class: 'btn danger', onclick: async () => {
          if (await confirmDialog('SUPPRIMER LE CORPUS', 'Tous les enregistrements de ta voix et de la série négative seront effacés de ce PC.', 'Supprimer', true)) {
            await save('corpusClear');
            refresh();
          }
        } }, 'Tout supprimer')),
      recorded ? null : h('div', { class: 'help', style: 'margin-bottom:10px' },
        'Les mesures se lancent une fois des phrases enregistrées : clique sur « 🎙 Lire » en face d’une phrase, puis lis-la à voix haute.'),
      bench ? h('pre', { class: 'help', style: 'white-space:pre-wrap;max-height:160px;overflow:auto' },
        `${bench.running ? 'Mesure en cours…' : `Mesure terminée (code ${bench.code})`}\n${(bench.log || []).slice(-12).join('\n')}`) : null,
      report && report.resultats ? h('table', { style: 'margin:10px 0' },
        h('thead', {}, h('tr', {}, h('th', {}, `Réglage${report.synthetique ? ' (voix de synthèse)' : ''}`), h('th', {}, 'Erreurs par mot'), h('th', {}, 'Commandes comprises'), h('th', {}, 'Faux « Jarvis »'), h('th', {}, 'Latence'))),
        h('tbody', {}, ...report.resultats.map((r) => h('tr', {},
          h('td', {}, r.label === report.meilleur ? h('b', {}, `★ ${r.label}`) : r.label),
          h('td', {}, `${r.wer} %`),
          h('td', {}, r.commandes == null ? '—' : `${r.commandes} %`),
          h('td', {}, `${r.negatifs_jarvis}/${r.negatifs}`),
          h('td', {}, `${r.latence_ms} ms`))))) : null,
      spk && spk.tranches ? h('div', { style: 'margin:10px 0' },
        h('div', { class: 'help' }, `Calibrage du filtre de voix (${fmtDate(spk.date)}) — seuil actuel ${spk.seuil_actuel}`),
        ...Object.entries(spk.tranches).map(([b, t]) => h('div', { class: 'status-line' },
          h('span', {}, h('b', {}, b), ` : ${t.phrases} phrases, ${t.negatifs} négatifs`),
          h('span', {}, `actuel : ${t.actuel.rejets_a_tort} rejets à tort, ${t.actuel.acceptes_a_tort} acceptés à tort`),
          h('span', {}, `recommandé ${t.seuil} : ${t.recommande.rejets_a_tort} / ${t.recommande.acceptes_a_tort}`),
          t.conflit ? h('span', { class: 'badge warn' }, 'chevauchement') : null)),
        spk.seuils && Object.keys(spk.seuils).length
          ? h('button', { class: 'btn primary', style: 'margin-top:8px', onclick: () => save('speakerApplyThresholds', spk.seuils) }, 'Appliquer les seuils recommandés')
          : null) : null,
      h('table', {}, h('thead', {}, h('tr', {}, h('th', {}, 'Phrase à lire'), h('th', {}, 'Prises'), h('th', {}, ''))), h('tbody', {}, ...rows))
    ];
  }

  // --- Section : Profil & préférences --------------------------------------------------------

  sections.profile = () => {
    const p = S.config.profile;
    const cat = S.catalog;
    const setP = (patch) => save('setConfig', { profile: patch });
    const instructions = h('textarea', { placeholder: 'Ex. : Ne me parle jamais de la météo le matin. Donne toujours les durées en minutes.' }, p.instructions || '');
    const prefs = S.memory.filter((e) => e.category === 'dialogue' && !e.pinned);
    const newPref = textInput('', { placeholder: 'Ex. : Ne m’interromps pas quand je réfléchis à voix haute.' });
    return [
      h('h1', {}, 'Profil & préférences'),
      h('p', { class: 'lead' }, 'La façon dont Jarvis te parle. Ces réglages construisent sa consigne système et sont aussi inscrits dans la mémoire RAG (catégorie « dialogue »).'),
      panel('Identité et façon de parler', null,
        h('div', { class: 'grid' },
          field('Prénom utilisé', textInput(p.name, { placeholder: S.userName ? `mémorisé : ${S.userName}` : 'ex. Julien', onchange: (e) => setP({ name: e.target.value.trim() }) })),
          field('Tutoiement', select({ tu: 'Tutoiement', vous: 'Vouvoiement' }, p.address, (v) => setP({ address: v }))),
          field('Appellation', textInput(p.honorific, { placeholder: 'ex. Monsieur, Chef… (facultatif)', onchange: (e) => setP({ honorific: e.target.value.trim() }) })),
          field('Ton', select(cat.tones, p.tone, (v) => setP({ tone: v }))),
          field('Longueur des réponses', select(cat.verbosity, p.verbosity, (v) => setP({ verbosity: v }))),
          field('Humour', range(0, 2, 1, p.humor, (n) => ['Aucun', 'Léger', 'Marqué'][n], (n) => setP({ humor: n }))),
          field('Consignes personnelles', h('div', {}, instructions,
            h('div', { class: 'row end', style: 'margin-top:8px' }, h('button', { class: 'btn primary', onclick: () => setP({ instructions: instructions.value.trim() }) }, 'Enregistrer les consignes'))), null, true)
        )
      ),
      panel('Consigne transmise à Jarvis', 'aperçu', h('div', { class: 'mono-block' }, S.personaPreview || '')),
      panel('Préférences de dialogue (RAG)', `${prefs.length} préférence(s)`,
        h('p', { class: 'hint' }, 'Règles libres ajoutées à la mémoire : elles passent en tête de ce que Jarvis sait de toi.'),
        prefs.length
          ? h('table', {}, h('tbody', {}, ...prefs.map((e) => h('tr', {},
            h('td', {}, e.text),
            h('td', { class: 'actions' }, h('button', { class: 'btn ghost', onclick: () => save('memoryDelete', e.id) }, 'Retirer'))))))
          : h('div', { class: 'empty' }, 'Aucune préférence libre pour l’instant.'),
        h('div', { class: 'row', style: 'margin-top:10px' }, newPref,
          h('button', { class: 'btn primary', onclick: async () => { if (newPref.value.trim()) { await save('memoryAdd', newPref.value.trim(), 'dialogue'); } } }, 'Ajouter'))
      )
    ];
  };

  // --- Section : Mémoire (RAG) ---------------------------------------------------------------

  let memoryFilter = { q: '', cat: '' };
  let editing = null;

  const EMOTION_LEVEL_NAMES = {
    10: "L'extase (ou l'euphorie)",
    9: "La joie",
    8: "Le contentement",
    7: "La sérénité",
    6: "La mélancolie",
    5: "Le chagrin",
    4: "L'abattement",
    3: "La détresse",
    2: "L'accablement",
    1: "Le désespoir"
  };

  const EMOTION_OPTIONS = [
    ['10', "10 · L'extase (ou l'euphorie) — ravissement absolu"],
    ['9', "9 · La joie — vive et chaleureuse"],
    ['8', "8 · Le contentement — satisfaction paisible"],
    ['7', "7 · La sérénité — calme profond de l'esprit"],
    ['6', "6 · La mélancolie — tristesse douce, nostalgie"],
    ['5', "5 · Le chagrin — déception, douleur morale"],
    ['4', "4 · L'abattement — perte d'énergie, découragé"],
    ['3', "3 · La détresse — angoisse aiguë, impuissance"],
    ['2', "2 · L'accablement — épuisement sous le fardeau"],
    ['1', "1 · Le désespoir — rupture d'espérance, sans issue"]
  ];
  let emotionFilter = { q: '', score: '' };
  let editingEmotion = null;

  // Annonces spontanées (mails, colis, fin de tâche, rappels…) : heures calmes et écart minimal.
  function announcementsPanel() {
    const a = S.config.announcements || {};
    const time = (key, value) => h('input', { type: 'time', value: value || '', onchange: (e) => save('setConfig', { announcements: { [key]: e.target.value } }) });
    return panel('Annonces spontanées', S.announcementsPending ? `${S.announcementsPending} en attente` : null,
      h('p', { class: 'hint' }, 'Tout ce que Jarvis dit sans qu’on lui parle (mail, colis, fin de tâche Claude, recherche, rappel). Pendant la sourdine (visio, musique), une annonce est affichée puis dite à la fin. Pendant les heures calmes, elle attend ton retour si tu es absent depuis 15 minutes ; les rappels et programmations passent toujours. Plusieurs annonces rapprochées sont regroupées.'),
      h('div', { class: 'grid' },
        field('Heures calmes', toggle('Ne rien annoncer la nuit quand je suis absent', a.quietHours !== false, (v) => save('setConfig', { announcements: { quietHours: v } }))),
        field('Début', time('quietStart', a.quietStart)),
        field('Fin', time('quietEnd', a.quietEnd)),
        field('Écart minimal', select([['0', 'Aucun'], ['15', '15 secondes'], ['30', '30 secondes'], ['60', '1 minute'], ['120', '2 minutes']], String(a.minGapSeconds ?? 30), (v) => save('setConfig', { announcements: { minGapSeconds: Number(v) } })),
          'Entre deux annonces spontanées ; les suivantes sont regroupées.')));
  }

  // Présence par webcam : arrivée, départ, mode vie privée (aucune image gardée).
  function presencePanel() {
    const p = S.config.presence || {};
    const st = S.presence || {};
    const tag = !p.enabled ? 'désactivée' : st.error ? 'erreur' : st.present ? `présent · ${st.faces} visage${st.faces > 1 ? 's' : ''}${st.privacy ? ' · vie privée' : ''}` : 'absent';
    return panel('Présence par webcam', tag,
      h('p', { class: 'hint' }, 'Le HUD envoie une petite image par seconde au noyau, qui compte les visages sur le PC (modèle YuNet, téléchargé une fois et vérifié) puis l’oublie : rien n’est enregistré ni envoyé ailleurs. À ton arrivée après une absence, Jarvis fait le briefing du matin s’il est attendu, sinon il dit les annonces gardées. Absent, les heures calmes s’appliquent. Avec d’autres personnes devant l’écran, les cards sont floutées et les mails ne sont pas annoncés.'),
      st.error ? h('div', { class: 'help', style: 'color: var(--err)' }, st.error) : null,
      h('div', { class: 'grid' },
        field('Activer', toggle('Détecter ma présence avec la webcam', p.enabled === true, (v) => save('setConfig', { presence: { enabled: v } }))),
        field('Absent après', select([['2', '2 minutes sans visage'], ['5', '5 minutes sans visage'], ['10', '10 minutes sans visage'], ['20', '20 minutes sans visage']], String(p.awayMinutes || 5), (v) => save('setConfig', { presence: { awayMinutes: Number(v) } }))),
        field('Vie privée', toggle('Flouter les cards quand d’autres personnes sont là', p.privacy !== false, (v) => save('setConfig', { presence: { privacy: v } }))),
        field('Au départ', toggle('Mettre la musique en pause', p.pauseMusic === true, (v) => save('setConfig', { presence: { pauseMusic: v } })))));
  }

  // Mémoire automatique : réglage, analyse à la demande et journal des dernières analyses.
  function memoryAutoPanel() {
    const cfg = S.config.memoryAuto || {};
    const auto = S.memoryAuto || { journal: [], pending: 0 };
    const statusLabel = { 'analysée': 'Souvenirs retenus', 'rien à retenir': 'Rien à retenir', 'impossible': 'Analyse impossible' };
    const journal = (auto.journal || []).map((j) => h('li', {},
      h('strong', {}, statusLabel[j.statut] || j.statut), ` · ${fmtDate(j.at)} · ${j.phrases} phrase${j.phrases > 1 ? 's' : ''}`,
      j.motif ? h('div', { class: 'help' }, j.motif) : null,
      ...(j.retenus || []).map((r) => h('div', {}, r.type === 'correction' ? `✎ ${r.avant} → ${r.texte}` : `+ [${r.categorie}] ${r.texte}`)),
      (j.ecartes || []).length ? h('div', { class: 'help' }, `Écarté : ${j.ecartes.join(' ; ')}`) : null));
    return panel('Mémoire automatique', cfg.enabled ? `${auto.pending || 0} phrase(s) en attente` : 'désactivée',
      h('p', { class: 'hint' }, 'Après quelques minutes de silence, Claude Haiku relit les phrases où tu parles de toi et propose quoi retenir ou corriger. Jarvis vérifie que chaque proposition s’appuie sur tes phrases et ne contient aucun secret. Seules les phrases à la première personne sont gardées, en mémoire vive, puis effacées après l’analyse. Rien n’est jamais oublié automatiquement.'),
      field('Activer', toggle('Retenir automatiquement ce que je dis de moi', cfg.enabled === true, (v) => save('setConfig', { memoryAuto: { enabled: v } }))),
      field('Analyse après', select([['2', '2 minutes de silence'], ['5', '5 minutes de silence'], ['10', '10 minutes de silence'], ['30', '30 minutes de silence']], String(cfg.idleMinutes || 5), (v) => save('setConfig', { memoryAuto: { idleMinutes: Number(v) } }))),
      h('div', { class: 'row' }, h('button', { class: 'btn ghost', disabled: !auto.pending, onclick: () => save('memoryAutoAnalyze') }, 'Analyser maintenant')),
      journal.length ? h('ul', { class: 'help' }, ...journal) : h('div', { class: 'empty' }, 'Aucune analyse pour l’instant.'));
  }

  sections.memory = () => {
    const entries = S.memory.slice().reverse();
    const categories = [...new Set(S.memory.map((e) => e.category))].sort();
    const q = memoryFilter.q.toLowerCase();
    const shown = entries.filter((e) => (!memoryFilter.cat || e.category === memoryFilter.cat) && (!q || e.text.toLowerCase().includes(q)));
    const search = h('input', { type: 'search', placeholder: 'Rechercher…', value: memoryFilter.q, oninput: (e) => { memoryFilter.q = e.target.value; render(true); setTimeout(() => { const s = document.querySelector('#section-memory input[type=search]'); if (s) { s.focus(); s.setSelectionRange(s.value.length, s.value.length); } }, 0); } });
    const catFilter = select([['', 'Toutes les catégories'], ...categories.map((c) => [c, c])], memoryFilter.cat, (v) => { memoryFilter.cat = v; render(true); });
    const newCat = textInput('', { placeholder: 'catégorie', list: 'mem-cats', style: 'max-width:180px' });
    const newText = textInput('', { placeholder: 'Ce que Jarvis doit retenir…' });
    // Souvenir temporaire : dernier jour où il reste vrai ; vide = durable.
    const today = new Date(Date.now() - new Date().getTimezoneOffset() * 60000).toISOString().slice(0, 10);
    const newExpires = textInput('', { type: 'date', min: today, title: 'Oublié après ce jour (vide : durable)', style: 'max-width:160px' });
    const expiryLabel = (d) => new Date(`${d}T12:00:00`).toLocaleDateString('fr-FR', { day: 'numeric', month: 'long', year: d.slice(0, 4) === today.slice(0, 4) ? undefined : 'numeric' });
    const rows = shown.map((e) => {
      if (editing === e.id) {
        const eCat = textInput(e.category, { list: 'mem-cats' });
        const eText = h('textarea', { style: 'min-height:60px' }, e.text);
        const eExpires = textInput(e.expires || '', { type: 'date', title: 'Oublié après ce jour (vide : durable)' });
        return h('tr', {},
          h('td', { style: 'width:170px' }, eCat),
          h('td', {}, eText, h('label', { class: 'help' }, 'Jusqu’au ', eExpires, ' (vide : durable)')),
          h('td', { class: 'actions' },
            h('button', { class: 'btn primary', onclick: async () => { const r = await save('memoryUpdate', e.id, eText.value, eCat.value, eExpires.value); if (r && r.ok) { editing = null; render(true); } } }, 'Enregistrer'),
            h('button', { class: 'btn ghost', onclick: () => { editing = null; render(true); } }, 'Annuler')));
      }
      return h('tr', {},
        h('td', { style: 'width:170px' }, h('span', { class: 'cat' }, e.category), e.pinned ? h('span', { class: 'pin', title: 'Issu du profil de dialogue' }, '◆ profil') : null),
        h('td', {}, e.text, h('div', { class: 'help' }, fmtDate(e.updatedAt || e.createdAt), e.expires ? h('span', { class: 'pin', title: 'Souvenir temporaire : oublié après ce jour' }, `⏳ jusqu’au ${expiryLabel(e.expires)}`) : null)),
        h('td', { class: 'actions' }, e.pinned
          ? h('button', { class: 'btn ghost', onclick: () => selectSection('profile') }, 'Profil →')
          : [h('button', { class: 'btn ghost', onclick: () => { editing = e.id; render(true); } }, 'Modifier'),
            h('button', { class: 'btn ghost', onclick: async () => { if (await confirmDialog('OUBLIER', `Oublier « ${e.text} » ?`, 'Oublier', true)) save('memoryDelete', e.id); } }, 'Oublier')]));
    });

    // --- Panneau RAG : Mot phrase énervé (Émotions) ---
    const rawEmotions = S.emotions || [];
    const emoQ = emotionFilter.q.toLowerCase();
    const emoShown = rawEmotions.filter((e) => {
      if (emotionFilter.score && String(e.score) !== emotionFilter.score) return false;
      if (emoQ && !e.text.toLowerCase().includes(emoQ)) return false;
      return true;
    });

    const emoSearch = h('input', {
      type: 'search',
      placeholder: 'Filtrer les mots ou phrases…',
      value: emotionFilter.q,
      oninput: (e) => {
        emotionFilter.q = e.target.value;
        render(true);
        setTimeout(() => {
          const s = document.querySelector('#emo-search-input');
          if (s) { s.focus(); s.setSelectionRange(s.value.length, s.value.length); }
        }, 0);
      },
      id: 'emo-search-input'
    });

    const emoScoreFilter = select([
      ['', 'Tous les niveaux (1 à 10)'],
      ...EMOTION_OPTIONS.map(([val, label]) => [val, `${val} · ${EMOTION_LEVEL_NAMES[val] || label}`])
    ], emotionFilter.score, (v) => { emotionFilter.score = v; render(true); });

    let newEmoText, newEmoScore;
    const addEmoForm = h('div', { class: 'row', style: 'margin-bottom: 12px; gap: 8px;' },
      newEmoText = textInput('', { placeholder: 'Ex. : génial, merci, ras le bol, tu fais chier, bouge toi...', style: 'flex: 2;' }),
      newEmoScore = select(EMOTION_OPTIONS, '10', () => {}),
      h('button', {
        class: 'btn primary',
        onclick: async () => {
          const val = newEmoText.value.trim();
          if (!val) { newEmoText.focus(); return; }
          const sc = Number(newEmoScore.value) || 10;
          await save('emotionAdd', val, sc);
          newEmoText.value = '';
          render(true);
        }
      }, '+ Ajouter')
    );

    const emoRows = emoShown.map((e) => {
      if (editingEmotion === e.id) {
        let editInput, editSelect;
        return h('tr', {},
          h('td', {}, editInput = textInput(e.text, { style: 'width: 100%;' })),
          h('td', { style: 'width: 220px;' }, editSelect = select(EMOTION_OPTIONS, String(e.score), () => {})),
          h('td', { class: 'actions' },
            h('button', {
              class: 'btn primary',
              onclick: async () => {
                const val = editInput.value.trim();
                if (!val) return;
                const sc = Number(editSelect.value) || 10;
                await save('emotionUpdate', e.id, val, sc);
                editingEmotion = null;
                render(true);
              }
            }, 'Enregistrer'),
            h('button', { class: 'btn ghost', onclick: () => { editingEmotion = null; render(true); } }, 'Annuler')
          )
        );
      }

      let badgeClass = 'badge';
      if (e.score >= 8) badgeClass = 'badge ok';
      else if (e.score >= 5) badgeClass = 'badge';
      else if (e.score >= 3) badgeClass = 'badge warn';
      else badgeClass = 'badge err';

      const levelTitle = EMOTION_LEVEL_NAMES[e.score] || `Niveau ${e.score}`;

      return h('tr', {},
        h('td', {}, h('b', {}, e.text), h('div', { class: 'help' }, levelTitle)),
        h('td', { style: 'width: 140px;' },
          h('span', { class: badgeClass }, `${e.score} / 10`)
        ),
        h('td', { class: 'actions' },
          h('button', { class: 'btn ghost', onclick: () => { editingEmotion = e.id; render(true); } }, 'Modifier'),
          h('button', {
            class: 'btn ghost',
            onclick: async () => {
              if (await confirmDialog('SUPPRIMER', `Supprimer « ${e.text} » ?`, 'Supprimer', true)) {
                await save('emotionDelete', e.id);
                render(true);
              }
            }
          }, 'Supprimer')
        )
      );
    });

    const emoPanel = panel('Mot phrase énervé (Émotions)', `${emoShown.length} / ${rawEmotions.length}`,
      h('p', { class: 'hint' },
        'Système RAG émotions : liste des expressions et mots associés à une note de 1 à 10 (1 = le plus énervé / désespoir, 10 = le plus heureux / extase). ' +
        'Chaque message de l’utilisateur est analysé en temps réel et active le niveau d’émotion correspondant sur le cœur de JARVIS.'
      ),
      addEmoForm,
      h('div', { class: 'row', style: 'margin-bottom: 12px; gap: 8px;' },
        emoSearch,
        h('div', { style: 'width: 280px;' }, emoScoreFilter)
      ),
      emoRows.length
        ? h('table', {},
            h('thead', {}, h('tr', {}, h('th', {}, 'Expression ou mot'), h('th', {}, 'Note (1 à 10)'), h('th', {}, ''))),
            h('tbody', {}, ...emoRows)
          )
        : h('div', { class: 'empty' }, 'Aucune expression trouvée.'),
      h('div', { class: 'row', style: 'margin-top: 14px; border-top: 1px dashed rgba(var(--rgb), 0.15); padding-top: 10px;' },
        h('button', {
          class: 'btn ghost',
          onclick: async () => {
            if (await confirmDialog('RÉINITIALISER', 'Restaurer la liste d’expressions par défaut ?', 'Restaurer', false)) {
              await save('emotionReset');
              render(true);
            }
          }
        }, '↻ Restaurer les expressions par défaut')
      )
    );

    return [
      h('h1', {}, 'Mémoire (RAG)'),
      h('p', { class: 'lead' }, 'Tout ce que Jarvis sait de toi : identité, goûts, habitudes, projets. Il retrouve ces souvenirs par le sens (EmbeddingGemma, local) et les utilise dans chaque réponse.'),
      h('datalist', { id: 'mem-cats' }, ...[...new Set(['identité', 'préférences', 'habitudes', 'projets', 'lieu', 'dialogue', ...categories])].map((c) => h('option', { value: c }))),
      panel('Ajouter un souvenir', null,
        h('div', { class: 'row' }, newCat, newText, newExpires,
          h('button', { class: 'btn primary', onclick: () => { if (newText.value.trim()) save('memoryAdd', newText.value.trim(), newCat.value.trim() || 'divers', newExpires.value); } }, 'Ajouter')),
        h('p', { class: 'hint' }, 'Date facultative : un souvenir temporaire (congés, déplacement…) est oublié de lui-même après ce jour.')),
      memoryAutoPanel(),
      panel('Souvenirs', `${shown.length} / ${S.memory.length}`,
        h('div', { class: 'row', style: 'margin-bottom:12px' }, search, h('div', { style: 'width:240px' }, catFilter)),
        rows.length ? h('table', {}, h('thead', {}, h('tr', {}, h('th', {}, 'Catégorie'), h('th', {}, 'Souvenir'), h('th', {}, ''))), h('tbody', {}, ...rows))
          : h('div', { class: 'empty' }, 'Aucun souvenir ne correspond.')),
      emoPanel,
      panel('Zone sensible', null,
        h('div', { class: 'row' }, h('div', {}, 'Effacer toute la mémoire. Le profil de dialogue est conservé ; Jarvis refera connaissance au prochain démarrage.'),
          h('div', { class: 'spacer' }),
          h('button', { class: 'btn danger', onclick: () => save('memoryPurgeRequest') }, 'Tout effacer…')))
    ];
  };

  // --- Section : Zones & vocabulaire ---------------------------------------------------------

  sections.zones = () => {
    const levels = S.catalog.zoneLevels;
    const zName = textInput('', { placeholder: 'nom (ex. Photos)' });
    const zPath = textInput('', { placeholder: 'chemin absolu (ex. D:\\Photos)' });
    let zRead = 'free';
    let zWrite = 'code';
    const term = textInput('', { placeholder: 'mot ou nom propre (ex. Worganic)' });
    const heard = textInput('', { placeholder: 'entendu (ex. worganique)' });
    const right = textInput('', { placeholder: 'correct (ex. Worganic)' });
    const voc = S.vocabulary;
    return [
      h('h1', {}, 'Zones & vocabulaire'),
      h('p', { class: 'lead' }, 'Les dossiers où Jarvis peut lire et écrire, et les mots que la reconnaissance vocale doit connaître.'),
      panel('Zones d’accès', `${S.zones.length} zone(s)`,
        h('p', { class: 'hint' }, 'Hors zones : lecture sur code, écriture interdite. Les données internes de Jarvis restent inaccessibles quoi qu’il arrive. Ouvrir l’écriture libre exige un code.'),
        h('table', {},
          h('thead', {}, h('tr', {}, h('th', {}, 'Zone'), h('th', {}, 'Chemin'), h('th', {}, 'Lecture'), h('th', {}, 'Écriture'), h('th', {}, ''))),
          h('tbody', {}, ...S.zones.map((z) => h('tr', {},
            h('td', {}, z.name),
            h('td', { style: 'word-break: break-all' }, z.path),
            h('td', {}, select(levels, z.read, (v) => save('zoneAdd', z.name, z.path, v, z.write))),
            h('td', {}, select(levels, z.write, (v) => save('zoneAdd', z.name, z.path, z.read, v))),
            h('td', { class: 'actions' }, z.id === 'workspace' ? h('span', { class: 'help' }, 'protégée')
              : h('button', { class: 'btn ghost', onclick: async () => { if (await confirmDialog('RETIRER LA ZONE', `Retirer la zone « ${z.name} » ?`, 'Retirer', true)) save('zoneRemove', z.id); } }, 'Retirer')))))),
        h('div', { class: 'row', style: 'margin-top:12px' }, zName, zPath,
          select(Object.entries(levels).map(([k, l]) => [k, `lecture : ${l}`]), zRead, (v) => { zRead = v; }),
          select(Object.entries(levels).map(([k, l]) => [k, `écriture : ${l}`]), zWrite, (v) => { zWrite = v; }),
          h('button', { class: 'btn primary', onclick: () => save('zoneAdd', zName.value.trim(), zPath.value.trim(), zRead, zWrite) }, 'Ajouter'))
      ),
      panel('Mots appris', `${voc.learned.length}`,
        voc.learned.length ? h('div', { class: 'chips' }, ...voc.learned.map((t) => h('span', { class: 'chip' }, t, h('button', { title: 'Retirer', onclick: () => save('vocabRemove', t) }, '×'))))
          : h('div', { class: 'empty' }, 'Aucun mot appris.'),
        h('div', { class: 'row', style: 'margin-top:12px' }, term, h('button', { class: 'btn primary', onclick: () => { if (term.value.trim()) save('vocabAdd', term.value.trim()); } }, 'Apprendre'))
      ),
      panel('Corrections de reconnaissance', `${voc.corrections.length}`,
        voc.corrections.length ? h('table', {}, h('tbody', {}, ...voc.corrections.map((c) => h('tr', {},
          h('td', {}, `« ${c.entendu} »`), h('td', {}, '→'), h('td', {}, c.correct),
          h('td', { class: 'actions' }, h('button', { class: 'btn ghost', onclick: () => save('vocabRemove', c.entendu) }, 'Retirer'))))))
          : h('div', { class: 'empty' }, 'Aucune correction.'),
        h('div', { class: 'row', style: 'margin-top:12px' }, heard, h('span', {}, '→'), right,
          h('button', { class: 'btn primary', onclick: () => save('vocabCorrect', heard.value.trim(), right.value.trim()) }, 'Ajouter'))
      ),
      panel('Vocabulaire transmis à Whisper', `${voc.active.length} termes`,
        h('div', { class: 'chips' }, ...voc.active.map((t) => h('span', { class: 'chip passive' }, t))))
    ];
  };

  // --- Section : Widget ------------------------------------------------------------------------

  sections.widget = () => {
    const w = S.config.widget;
    const setW = (patch) => save('setConfig', { widget: patch });
    return [
      h('h1', {}, 'Widget'),
      h('p', { class: 'lead' }, 'L’orbe discrète toujours au premier plan (Ctrl+Maj+W) : au-dessus des jeux et logiciels, sans gêner. Seul le cœur de l’orbe réagit à la souris ; le fond laisse passer les clics.'),
      panel('Apparence', null,
        h('div', { class: 'grid' },
          field('Taille', range(56, 200, 4, w.size, (n) => `${n} × ${n} px`, (n) => setW({ size: n }))),
          field('Opacité', range(0.3, 1, 0.05, w.opacity, (n) => `${Math.round(n * 100)} %`, (n) => setW({ opacity: n })))
        )),
      panel('Clics', null,
        h('div', { class: 'grid' },
          field('Fond traversable', toggle('Les clics traversent le fond translucide', w.clickThrough, (v) => setW({ clickThrough: v }))),
          field('Mode fantôme', toggle('Tout traverse, même le cœur (Ctrl+Maj+G pour en sortir)', w.ghost, (v) => setW({ ghost: v })))
        ))
    ];
  };

  // Couleurs des cards du HUD : type 1 (toujours actives : Bureau, Audio, Matériel, Conversation, Actions) et type 2
  // (temporaires : Terminal, mails, musique, listes…), en mode normal et réduit en bouton.
  const CARD_COLOR_DEFAULTS = { t1n: '', t1r: '#ffd640', t2n: '#be8cff', t2r: '#be8cff' };
  function cardColorsBlock(st) {
    const cur = { ...CARD_COLOR_DEFAULTS, ...(st.cardColors || {}) };
    const themeHex = '#' + String(THEME_RGB[st.theme] || '56, 225, 255').split(',').map((n) => Number(n).toString(16).padStart(2, '0')).join('');
    const apply = (patch) => save('setStatus', { cardColors: { ...cur, ...patch } });
    const picker = (key, label, help) => {
      const input = h('input', { type: 'color', value: cur[key] || themeHex, style: 'width:46px;height:28px;padding:0;border:0;background:none;cursor:pointer' });
      input.addEventListener('change', () => apply({ [key]: input.value }));
      return field(label, h('div', { style: 'display:flex;gap:10px;align-items:center' }, input,
        h('button', { class: 'btn', type: 'button', onclick: () => apply({ [key]: CARD_COLOR_DEFAULTS[key] }) }, 'Par défaut')), help);
    };
    return h('div', { style: 'margin-top:14px' },
      h('label', {}, 'Couleurs des cards'),
      h('div', { class: 'help', style: 'margin-bottom:8px' }, 'Type 1 : cards toujours actives (Bureau, Audio, Matériel, Conversation, Actions). Type 2 : cards temporaires (Terminal, mails, musique, listes…). Chacune a une couleur ouverte et une couleur réduite en bouton.'),
      h('div', { class: 'grid' },
        picker('t1n', 'Type 1 · ouverte', 'Par défaut : la couleur du thème.'),
        picker('t1r', 'Type 1 · réduite'),
        picker('t2n', 'Type 2 · ouverte'),
        picker('t2r', 'Type 2 · réduite')));
  }

  // Toutes les cards du HUD et leur numéro : type 1 (toujours actives) de 1 à 99, type 2 (temporaires) de 100 à 199.
  // Le numéro sert à la voix (« ferme la card 2 » = 102) ; on peut le changer, sans doublon.
  function cardNumbersBlock(st, list) {
    const custom = { ...(st.cardNumbers || {}) };
    const inputs = new Map();
    const err = h('div', { class: 'help', style: 'color:#ff9aa8;min-height:16px' });
    const range = (type) => (type === 1 ? [1, 99] : [100, 199]);
    const apply = () => {
      const next = {};
      const seen = new Map();
      for (const c of list) {
        const input = inputs.get(c.key);
        const v = Number(input.value);
        const [lo, hi] = range(c.type);
        if (!Number.isInteger(v) || v < lo || v > hi) { err.textContent = `${c.label} : le numéro doit être entre ${lo} et ${hi}.`; return; }
        if (seen.has(v)) { err.textContent = `Le numéro ${v} est utilisé par « ${seen.get(v)} » et « ${c.label} ».`; return; }
        seen.set(v, c.label);
        // Mémorisé : un numéro déjà personnalisé, ou changé maintenant ; remettre le numéro par défaut le réinitialise.
        if ((custom[c.key] !== undefined || v !== c.number) && v !== c.default) next[c.key] = v;
      }
      err.textContent = '';
      save('setStatus', { cardNumbers: next });
    };
    const row = (c) => {
      const input = h('input', { type: 'number', value: String(c.number == null ? '' : c.number), min: String(range(c.type)[0]), max: String(range(c.type)[1]), style: 'width:80px' });
      input.addEventListener('change', apply);
      inputs.set(c.key, input);
      return h('tr', {},
        h('td', {}, c.label), h('td', { class: 'help' }, c.open ? 'ouverte' : ''), h('td', {}, input),
        h('td', { class: 'help' }, c.default ? `défaut ${c.default}` : ''));
    };
    const table = (title, type) => {
      const rows = list.filter((c) => c.type === type);
      return h('div', { style: 'margin-top:8px' }, h('div', { class: 'help', style: 'font-weight:700' }, title),
        h('table', { class: 'tbl', style: 'width:100%' }, h('tbody', {}, ...rows.map(row))));
    };
    return h('div', { style: 'margin-top:14px' },
      h('label', {}, 'Numéros des cards'),
      h('div', { class: 'help', style: 'margin-bottom:4px' }, 'Le numéro sert à la voix (« ferme la card 2 » = card 102). Type 1 : cards toujours actives, de 1 à 99. Type 2 : cards temporaires, de 100 à 199. Un numéro ne peut servir qu\'à une card ; les cards ouvertes prennent le nouveau numéro tout de suite.'),
      table('Type 1 · toujours actives (1 à 99)', 1),
      table('Type 2 · temporaires (100 à 199)', 2),
      err);
  }

  // --- Section : Bureau JARVIS ---------------------------------------------------------------------

  sections.desk = () => {
    const d = S.config.desk || { enabled: false, desktop: 2, monitor: 0, otherScreens: true, wallpaper: false, wallpaperDim: 0.35 };
    const setD = (patch) => save('setConfig', { desk: patch });
    const tauri = window.__TAURI__;
    const monitors = [['0', 'Écran principal'], ...[1, 2, 3, 4].map((n) => [String(n), `Écran ${n}`])];
    return [
      h('h1', {}, 'Bureau JARVIS'),
      h('p', { class: 'lead' }, 'Un bureau virtuel de Windows devient l’interface JARVIS : le HUD remplace le fond d’écran et les icônes, les applications s’ouvrent par-dessus comme d’habitude, et le cœur se détache au premier plan dès qu’une fenêtre le recouvre. Les autres bureaux gardent leur fond d’écran et leurs icônes, avec le seul widget : un clic sur son cœur ramène au Bureau JARVIS.'),
      panel('Activation', null,
        h('div', { class: 'grid' },
          field('Bureau JARVIS', toggle('Le HUD sert de fond à un bureau virtuel (créé au besoin)', d.enabled, (v) => setD({ enabled: v })),
            'Ctrl+Maj+B, Ctrl+Maj+J, l’icône de la zone de notification ou un clic sur le widget : aller au Bureau JARVIS.'),
          field('Bureau virtuel', select([1, 2, 3, 4, 5].map((n) => [String(n), `Bureau ${n}`]), String(d.desktop || 2), (v) => setD({ desktop: Number(v) })),
            'Ordre de la vue des tâches (Win+Tab).'),
          field('Aller au Bureau JARVIS', h('button', {
            class: 'btn', disabled: !d.enabled,
            onclick: () => { if (tauri && tauri.core) tauri.core.invoke('desk_goto'); }
          }, 'Y aller (Ctrl+Maj+B)'))
        )),
      panel('Écrans', null,
        h('div', { class: 'grid' },
          field('Écran du HUD', select(monitors, String(d.monitor || 0), (v) => setD({ monitor: Number(v) })), 'Numérotation de Windows (Paramètres › Affichage).'),
          field('Autres écrans', toggle('Fond JARVIS sans icônes sur les autres écrans du bureau', d.otherScreens, (v) => setD({ otherScreens: v })))
        )),
      panel('Fond', null,
        h('div', { class: 'grid' },
          field('Fond d’écran de Windows', toggle('Garder l’image de fond de Windows sous les effets et les cards (sans les icônes)', !!d.wallpaper, (v) => setD({ wallpaper: v })),
            'Au lieu du fond noir. Suit le fond choisi dans Windows (diaporama et Windows à la une compris) et son ajustement (remplir, ajuster, mosaïque…).'),
          field('Voile', range(0, 0.9, 0.05, d.wallpaperDim ?? 0.35, (v) => `${Math.round(v * 100)} %`, (v) => setD({ wallpaperDim: v })),
            'Voile de la couleur du thème posé sur l’image, pour garder les effets et le texte lisibles. 0 % : image intacte.')
        )),
      panel('Cœur toujours visible', null,
        h('p', { class: 'help' }, 'Quand une application recouvre le centre du HUD, le cœur passe au premier plan, à sa place. Seul le cœur réagit à la souris (le reste laisse passer les clics) : double-clic pour réduire les fenêtres qui cachent le HUD, clic droit pour le menu. Quand le HUD est entièrement caché ou sur un autre bureau, il ne se dessine plus.'))
    ];
  };

  // Sections remplies par les étapes suivantes (journal, sécurité, connecteurs).
  for (const [id, title, text] of [
    ['mcp', 'Connecteurs MCP', 'Relier Jarvis à des outils externes (Gmail, Drive, Agenda, Spotify, Chrome…).'],
    ['security', 'Sécurité', 'Actions soumises à accord et niveau exigé pour chacune.'],
    ['journal', 'Journal & dialogues', 'Toutes les actions et conversations, inscrites au journal scellé.']
  ]) {
    if (!sections[id]) sections[id] = () => [h('h1', {}, title), h('p', { class: 'lead' }, text)];
  }

  // Les étapes suivantes enregistrent leurs sections ici.
  window.JarvisAdmin = { h, panel, field, select, toggle, range, textInput, fmtDate, api, save, toast, challengeFlow, confirmDialog, sections, render, getSnapshot: () => S, selectSection };

  // --- Démarrage ---------------------------------------------------------------------------------

  document.querySelectorAll('#nav button').forEach((b) => b.addEventListener('click', () => selectSection(b.dataset.section)));
  document.getElementById('btn-close').addEventListener('click', () => bridge.window('close'));
  document.getElementById('btn-min').addEventListener('click', () => bridge.window('minimize'));
  document.addEventListener('keydown', (e) => {
    if (e.key === 'Escape') {
      if (!document.getElementById('modal').classList.contains('hidden')) closeModal();
      else bridge.window('close');
    }
  });

  // Rafraîchissement quand l'orchestrateur signale un changement (mémoire, réglages, journal…).
  let refreshTimer = null;
  const REFRESH_ON = new Set(['memory', 'status', 'journal', 'theme_config', 'heart_style_config', 'density_config', 'vocabulary', 'voice_backend', 'admin_refresh', 'security', 'mcp', 'speaker_enrolled', 'speaker_reset', 'speaker_status', 'speaker_deleted', 'speaker_renamed', 'speaker_updated', 'build_space']);
  bridge.onEvent((ev) => {
    if (!ev) return;
    if (ev.type === 'speaker_enroll_start') {
      const name = ev.name || 'Johann';
      enrollState = { active: true, targetName: name, current: 0, target: ev.count || 3, message: `Micro à l’écoute pour ${name} : prononcez votre 1ère phrase...` };
      render(true);
      return;
    }
    if (ev.type === 'speaker_enroll_progress') {
      enrollState = {
        active: !ev.completed,
        targetName: ev.targetName || enrollState.targetName || '',
        current: ev.current || 0,
        target: ev.target || 3,
        message: ev.message || (ev.completed ? 'Empreinte enregistrée !' : `Échantillon ${ev.current}/${ev.target} capté. Continuez...`)
      };
      render(true);
      return;
    }
    if (ev.type === 'speaker_enrolled') {
      const spk = ev.speaker || (ev.status && ev.status.speaker);
      const name = spk ? spk.name : enrollState.targetName;
      enrollState = { active: false, targetName: '', current: 0, target: 3, message: '' };
      toast(`✓ Empreinte vocale enregistrée${name ? ` (${name})` : ''}`);
      reload();
      return;
    }
    if (ev.type === 'speaker_enroll_cancel') {
      enrollState = { active: false, targetName: '', current: 0, target: 3, message: '' };
      toast('Enrôlement annulé');
      render(true);
      return;
    }
    if (!REFRESH_ON.has(ev.type)) return;
    if ((ev.type === 'journal' || ev.type === 'security' || ev.type === 'mcp' || ev.type === 'failures' || ev.type === 'documents') && window.JarvisAdmin.onJournal) window.JarvisAdmin.onJournal(ev);
    clearTimeout(refreshTimer);
    refreshTimer = setTimeout(async () => {
      const snap = await bridge.call('snapshot');
      if (snap) setSnapshot(snap, false);
    }, 250);
  });

  (async () => {
    selectSection(current);
    const snap = await bridge.call('snapshot');
    setSnapshot(snap, true);
  })();
})();
