// Module « Agents » (card 122) : créer un agent de surveillance et d'action à partir d'une demande en langage
// naturel (zone de texte), lister les agents actifs ou en pause, les exécuter à la demande, consulter leurs
// derniers rapports, les modifier (par prompt IA ou formulaire complet) et les supprimer.
(function () {
  const J = (window.JARVIS = window.JARVIS || {});
  const bridge = window.jarvisBridge;
  const root = document.getElementById('widgets');
  if (!root || !J.cardHeader || !J.widgets) return;

  const KEY = 'agents';
  // Card du HUD (pas du noyau) qui disparaît entièrement à la fermeture (sinon : fantôme, contenu laissé à l'écran).
  if (J.widgetKit) {
    J.widgetKit.ownKey(KEY);
    J.widgetKit.vanish(KEY);
  }
  const MARGIN = 12;
  const ui = () => parseFloat(document.documentElement.style.getPropertyValue('--ui')) || 1;
  const call = (op, p = {}) => (bridge && bridge.call ? bridge.call('agents', op, p) : Promise.resolve({ ok: false }));

  const EXAMPLES = [
    {
      label: '✉ Résumé quotidien des mails',
      prompt: 'Surveille mes mails et résume-moi une fois par jour mes mails à 18h00'
    },
    {
      label: '⚙ Surveillance PC (RAM / CPU / GPU)',
      prompt: 'Surveille mon PC et dès que la RAM, le CPU ou le GPU sont trop utilisés indique-le-moi et propose-moi des conseils pour améliorer mes performances'
    },
    {
      label: '🏉 Matchs Rugby TOP 14 (TV)',
      prompt: "J'adore le rugby alors indique-moi chaque jour ou sur demande la liste des matchs de rugby du TOP 14 avec la date, l'heure et la chaîne de diffusion"
    }
  ];

  function h(tag, attrs = {}, ...children) {
    const e = document.createElement(tag);
    for (const [k, v] of Object.entries(attrs || {})) {
      if (v === undefined || v === null || v === false) continue;
      if (k === 'class') e.className = v;
      else if (k === 'value') e.value = v;
      else if (k === 'checked') e.checked = Boolean(v);
      else if (k.startsWith('on') && typeof v === 'function') e.addEventListener(k.slice(2), v);
      else e.setAttribute(k, v === true ? '' : v);
    }
    for (const c of children.flat()) {
      if (c !== null && c !== undefined && c !== false) {
        e.append(c instanceof Node ? c : document.createTextNode(String(c)));
      }
    }
    return e;
  }

  let state = {
    agents: [],
    modeles: [],
    moteurActif: 'claude',
    modeleActif: 'Claude · Haiku 4.5',
    telegramConfigure: false
  };
  let editingId = null; // id de l'agent en cours d'édition, ou null
  let expandedId = null; // id de l'agent dont le rapport détaillé est déplié
  let statusMsg = null; // { text, err }
  let busy = false;

  // Brouillon de création (zone de texte principale)
  let createForm = {
    prompt: '',
    choixModele: 'auto',
    sortieMode: 'voix',
    telegram: false
  };

  function setStatus(text, err = false) {
    statusMsg = text ? { text, err } : null;
    renderStatus();
  }

  const card = h('div', { class: 'widget w-tool w-card-list w-agents' });
  const head = h(
    'div',
    { class: 'w-head' },
    J.widgetKit && J.widgetKit.gripHandle ? J.widgetKit.gripHandle() : null,
    h('span', { class: 'w-title' }, 'AGENTS'),
    h('button', { class: 'w-close', title: 'Fermer', onclick: (e) => { e.stopPropagation(); J.widgets.close(KEY); } }, '×')
  );
  const body = h('div', { class: 'ag-body' });
  const statusBox = h('div', { class: 'ag-status-box' });
  card.append(head, body, statusBox);
  if (J.widgetKit) {
    if (J.widgetKit.makeDraggable) J.widgetKit.makeDraggable(card, KEY);
    if (J.widgetKit.normalizeHead) J.widgetKit.normalizeHead(KEY, card);
  }
  const pill = card.querySelector('.w-mini-pill');

  function renderStatus() {
    statusBox.replaceChildren();
    if (!statusMsg) return;
    statusBox.append(
      h('div', { class: 'ag-status' + (statusMsg.err ? ' err' : ' ok') }, statusMsg.text)
    );
  }

  function placeDefault() {
    const S = ui();
    const saved = (() => {
      try {
        return JSON.parse(
          localStorage.getItem((window.JARVIS_POS_PREFIX || 'jarvis.') + 'widgetPos.' + KEY) || 'null'
        );
      } catch {
        return null;
      }
    })();
    const x = saved && Number.isFinite(saved.x) ? saved.x : Math.max(MARGIN, Math.round((window.innerWidth - 520 * S) / 2));
    const y = saved && Number.isFinite(saved.y) ? saved.y : Math.max(MARGIN, Math.round(80 * S));
    card.style.left = `${x}px`;
    card.style.top = `${y}px`;
  }

  async function refresh() {
    const r = await call('etat');
    if (r && r.ok) {
      state = r;
      if (pill) pill.textContent = `◈ AGENTS (${(r.agents || []).length})`;
      render();
    }
  }

  async function open() {
    let rec = J.widgets.cards.get(KEY);
    if (!rec || !rec.el || !rec.el.isConnected) {
      root.appendChild(card);
      placeDefault();
      rec = { el: card, timer: null, kind: KEY, state: 'active', fadeTimer: null, persist: true, lastActive: performance.now() };
      J.widgets.cards.set(KEY, rec);
    }
    clearTimeout(rec.fadeTimer); // rouverte pendant sa disparition : elle reste
    if (J.widgetKit && J.widgetKit.setFolded) J.widgetKit.setFolded(KEY, false);
    card.classList.remove('w-ghost', 'w-fading', 'w-collapsed', 'w-leaving');
    card.classList.add('w-active');
    rec.state = 'active';
    rec.lastActive = performance.now();
    if (J.widgetKit && J.widgetKit.keepInView) J.widgetKit.keepInView(card);
    if (J.cardFront && J.cardFront.front) J.cardFront.front(card); // card qui s'ouvre : devant les autres
    await refresh();
  }

  function formatAgo(iso) {
    if (!iso) return 'Jamais exécuté';
    const d = new Date(iso);
    if (isNaN(d.getTime())) return String(iso);
    const diffS = Math.max(0, Math.floor((Date.now() - d.getTime()) / 1000));
    if (diffS < 60) return `Il y a ${diffS} s`;
    if (diffS < 3600) return `Il y a ${Math.floor(diffS / 60)} min`;
    const hm = d.toLocaleTimeString('fr-FR', { hour: '2-digit', minute: '2-digit' });
    const sameDay = d.toDateString() === new Date().toDateString();
    return sameDay ? `Aujourd'hui à ${hm}` : `${d.toLocaleDateString('fr-FR', { day: '2-digit', month: '2-digit' })} à ${hm}`;
  }

  function parseModelSelection(val) {
    if (!val || val === 'auto') return { moteur: 'auto', modele: 'auto' };
    if (val.startsWith('gemini')) return { moteur: 'agy', modele: val };
    return { moteur: 'claude', modele: val };
  }

  function currentModelSelectValue(agent) {
    if (!agent || agent.moteur === 'auto' || agent.modele === 'auto') return 'auto';
    return agent.modele || 'auto';
  }

  // --- Vue principale : création par demande + liste des agents -------------------------------------------

  function render() {
    body.replaceChildren();
    if (editingId) {
      const ag = (state.agents || []).find((a) => a.id === editingId);
      if (ag) {
        body.append(renderEditView(ag));
        renderStatus();
        return;
      }
      editingId = null;
    }
    body.append(renderCreateSection(), renderAgentsList());
    renderStatus();
  }

  function renderCreateSection() {
    const box = h('div', { class: 'ag-create-box' });

    const headRow = h(
      'div',
      { class: 'ag-sec-head' },
      h('span', { class: 'ag-sec-title' }, '✦ NOUVEL AGENT DEPUIS UNE DEMANDE'),
      h('span', { class: 'ag-engine-pill', title: 'Moteur IA actif de Jarvis (minimum AGY 3.8 Flash ou Claude Haiku)' }, `IA : ${state.modeleActif || 'Auto'}`)
    );

    const ta = h('textarea', {
      class: 'ag-textarea',
      rows: 3,
      placeholder:
        'Décris ce que l’agent doit surveiller et faire…\nEx : « Surveille mes mails et résume-moi une fois par jour mes mails » ou « Surveille mon PC (RAM, CPU, GPU)… »',
      value: createForm.prompt,
      oninput: (e) => {
        createForm.prompt = e.target.value;
      }
    });

    // Exemples cliquables en 1 clic
    const exRow = h('div', { class: 'ag-examples' }, h('span', { class: 'ag-ex-lbl' }, 'Exemples :'));
    for (const ex of EXAMPLES) {
      exRow.append(
        h(
          'button',
          {
            class: 'ag-chip',
            title: ex.prompt,
            onclick: () => {
              createForm.prompt = ex.prompt;
              ta.value = ex.prompt;
              ta.focus();
            }
          },
          ex.label
        )
      );
    }

    // Réglages rapides : modèle IA (min 3.8 Flash / Haiku) + canal de restitution
    const modelSelect = h(
      'select',
      {
        class: 'ag-select',
        onchange: (e) => {
          createForm.choixModele = e.target.value;
        }
      },
      ...(state.modeles && state.modeles.length
        ? state.modeles
        : [
            { id: 'auto', label: `Automatique (${state.modeleActif})` },
            { id: 'gemini-3.8-flash-high', label: 'AGY · Gemini 3.8 Flash High' },
            { id: 'gemini-3.8-flash-medium', label: 'AGY · Gemini 3.8 Flash Medium' },
            { id: 'gemini-3.8-flash-low', label: 'AGY · Gemini 3.8 Flash Low' },
            { id: 'haiku', label: 'Claude · Haiku 4.5' },
            { id: 'sonnet', label: 'Claude · Sonnet 4.6' }
          ]
      ).map((m) => h('option', { value: m.id, selected: createForm.choixModele === m.id }, m.label))
    );

    const outSelect = h(
      'select',
      {
        class: 'ag-select',
        onchange: (e) => {
          createForm.sortieMode = e.target.value;
        }
      },
      h('option', { value: 'voix', selected: createForm.sortieMode === 'voix' }, '🔊 Voix + Card HUD'),
      h('option', { value: 'visuel', selected: createForm.sortieMode === 'visuel' }, '👁 Card HUD seule (muet)'),
      h('option', { value: 'silencieux', selected: createForm.sortieMode === 'silencieux' }, '🔇 Silencieux (historique)')
    );

    const tgCheck = h(
      'label',
      { class: 'ag-chk', title: 'Envoyer aussi les rapports de cet agent sur Telegram' },
      h('input', {
        type: 'checkbox',
        checked: createForm.telegram,
        onchange: (e) => {
          createForm.telegram = e.target.checked;
        }
      }),
      '✈ Telegram'
    );

    const createAction = async (runNow) => {
      if (busy) return;
      const p = (createForm.prompt || '').trim();
      if (p.length < 4) {
        setStatus('Écris une demande dans la zone de texte (ou clique sur un exemple).', true);
        return;
      }
      busy = true;
      setStatus('Analyse de la demande et mise en place de l’agent (3.8 Flash / Haiku)…');
      const { moteur, modele } = parseModelSelection(createForm.choixModele);
      const r = await call('creer', {
        prompt: p,
        moteur,
        modele,
        sortie: { mode: createForm.sortieMode, telegram: createForm.telegram }
      });
      busy = false;
      if (!r || !r.ok) {
        setStatus((r && r.error) || 'Impossible de créer l’agent.', true);
        return;
      }
      createForm.prompt = '';
      setStatus(`Agent « ${r.agent.nom} » créé et activé.`);
      await refresh();
      if (runNow && r.agent && r.agent.id) {
        runAgentNow(r.agent.id, r.agent.nom);
      }
    };

    const optRow = h(
      'div',
      { class: 'ag-create-opts' },
      modelSelect,
      outSelect,
      tgCheck,
      h(
        'div',
        { class: 'ag-create-btns' },
        h(
          'button',
          {
            class: 'ag-btn primary',
            onclick: () => createAction(false)
          },
          '✦ Créer l’agent'
        ),
        h(
          'button',
          {
            class: 'ag-btn',
            title: 'Créer l’agent et lancer immédiatement une première exécution',
            onclick: () => createAction(true)
          },
          '▶ Créer & tester'
        )
      )
    );

    box.append(headRow, ta, exRow, optRow);
    return box;
  }

  async function runAgentNow(id, nom) {
    setStatus(`Exécution de « ${nom} » en cours…`);
    expandedId = id;
    render();
    const r = await call('executer', { id });
    await refresh();
    if (r && r.ok) {
      setStatus(`« ${nom} » a terminé son analyse.`);
    } else {
      setStatus((r && r.error) || `Échec de l’exécution de « ${nom} ».`, true);
    }
  }

  function triggerBadges(a) {
    const badges = [];
    const d = a.declencheurs || {};
    if (d.horaire && d.horaire.actif) {
      if (d.horaire.type === 'intervalle') {
        badges.push(h('span', { class: 'ag-badge trig' }, `⏱ Toutes les ${d.horaire.minutes || 60} min`));
      } else {
        badges.push(h('span', { class: 'ag-badge trig' }, `⏱ Chaque jour à ${d.horaire.heure || '09:00'}`));
      }
    }
    if (d.materiel && d.materiel.actif) {
      badges.push(
        h(
          'span',
          { class: 'ag-badge hw' },
          `⚙ CPU>${d.materiel.cpuPct || 85}% · RAM>${d.materiel.ramPct || 85}% · GPU>${d.materiel.gpuPct || 85}%`
        )
      );
    }
    if (d.mails && d.mails.actif) {
      badges.push(
        h(
          'span',
          { class: 'ag-badge mail' },
          d.mails.mode === 'nouveau_mail' ? '✉ Nouveaux mails' : '✉ Résumé mails'
        )
      );
    }
    if (Array.isArray(a.sources) && a.sources.includes('web')) {
      badges.push(h('span', { class: 'ag-badge web' }, '🌐 Recherche Web'));
    }
    if (d.demande && d.demande.actif && Array.isArray(d.demande.motsCles) && d.demande.motsCles.length) {
      badges.push(
        h(
          'span',
          { class: 'ag-badge voice', title: `Dis par exemple : « ${d.demande.motsCles[0]} »` },
          `🗣 « ${d.demande.motsCles.slice(0, 2).join(' », « ')} »`
        )
      );
    }
    badges.push(h('span', { class: 'ag-badge model' }, `🤖 ${a.modeleLibelle || 'Auto'}`));
    const sm = (a.sortie && a.sortie.mode) || 'voix';
    const outLabel = sm === 'visuel' ? '👁 HUD' : sm === 'silencieux' ? '🔇 Muet' : '🔊 Voix + HUD';
    badges.push(h('span', { class: 'ag-badge out' }, outLabel + (a.sortie && a.sortie.telegram ? ' · ✈ TG' : '')));
    return badges;
  }

  function renderAgentsList() {
    const agents = state.agents || [];
    const wrap = h('div', { class: 'ag-list-wrap' });
    const activos = agents.filter((a) => a.actif).length;
    wrap.append(
      h(
        'div',
        { class: 'ag-sec-head' },
        h('span', { class: 'ag-sec-title' }, `MES AGENTS (${agents.length})`),
        h('span', { class: 'ag-sec-sub' }, `${activos} en surveillance active`)
      )
    );

    if (!agents.length) {
      wrap.append(
        h(
          'div',
          { class: 'ag-empty' },
          'Aucun agent configuré. Saisis une demande ci-dessus ou clique sur un exemple (Mails, Performances PC, Rugby TOP 14) pour mettre en place ton premier agent autonome.'
        )
      );
      return wrap;
    }

    const list = h('div', { class: 'ag-list' });
    for (const a of agents) {
      const isExpanded = expandedId === a.id;
      const item = h('div', { class: 'ag-item' + (a.actif ? ' is-active' : ' is-paused') + (a.enCours ? ' is-running' : '') });

      // Ligne supérieure : icône, titre, interrupteur actif/pause, boutons d'action
      const toggleBtn = h(
        'button',
        {
          class: 'ag-toggle' + (a.actif ? ' on' : ''),
          title: a.actif ? 'Agent actif (cliquer pour mettre en pause)' : 'Agent en pause (cliquer pour activer la surveillance)',
          onclick: async () => {
            await call('basculer', { id: a.id, actif: !a.actif });
            await refresh();
          }
        },
        a.actif ? '● ACTIF' : '○ PAUSE'
      );

      let confirmDelete = false;
      const delBtn = h(
        'button',
        {
          class: 'ag-btn-sm danger',
          title: 'Supprimer cet agent',
          onclick: async (e) => {
            const btn = e.currentTarget;
            if (!confirmDelete) {
              confirmDelete = true;
              btn.textContent = 'Confirmer ?';
              setTimeout(() => {
                confirmDelete = false;
                if (btn.isConnected) btn.textContent = '🗑';
              }, 3000);
              return;
            }
            await call('supprimer', { id: a.id });
            setStatus(`Agent « ${a.nom} » supprimé.`);
            await refresh();
          }
        },
        '🗑'
      );

      const topRow = h(
        'div',
        { class: 'ag-item-top' },
        h('span', { class: 'ag-ico' }, a.icone || '◈'),
        h(
          'div',
          { class: 'ag-title-col' },
          h('div', { class: 'ag-name' }, a.nom),
          h('div', { class: 'ag-desc' }, a.description)
        ),
        toggleBtn,
        h(
          'div',
          { class: 'ag-actions' },
          h(
            'button',
            {
              class: 'ag-btn-sm run',
              disabled: Boolean(a.enCours),
              title: 'Exécuter cet agent immédiatement',
              onclick: () => runAgentNow(a.id, a.nom)
            },
            a.enCours ? '⏳…' : '▶ Exécuter'
          ),
          h(
            'button',
            {
              class: 'ag-btn-sm',
              title: 'Modifier cet agent (par demande texte ou formulaire détaillé)',
              onclick: () => {
                editingId = a.id;
                setStatus(null);
                render();
              }
            },
            '✎ Modifier'
          ),
          delBtn
        )
      );

      const badgesRow = h('div', { class: 'ag-badges' }, ...triggerBadges(a));

      // Pied de carte : dernière exécution et bouton pour déplier le dernier rapport / historique
      const hasReport = Boolean(a.dernierResultat) || (Array.isArray(a.historique) && a.historique.length > 0);
      const metaRow = h(
        'div',
        { class: 'ag-meta-row' },
        h(
          'span',
          { class: 'ag-last-run' },
          a.enCours
            ? '⚡ Analyse IA en cours…'
            : `${formatAgo(a.derniereExecution)}${a.dernierDeclencheur ? ` · ${a.dernierDeclencheur}` : ''}${a.nbExecutions ? ` (${a.nbExecutions} exéc.)` : ''}`
        ),
        hasReport
          ? h(
              'button',
              {
                class: 'ag-link-btn',
                onclick: () => {
                  expandedId = isExpanded ? null : a.id;
                  render();
                }
              },
              isExpanded ? '▴ Masquer le rapport' : '▾ Voir le dernier rapport'
            )
          : null
      );

      item.append(topRow, badgesRow, metaRow);

      if (isExpanded && hasReport) {
        const repBox = h('div', { class: 'ag-report-box' });
        if (a.dernierResultat) {
          repBox.append(
            h(
              'div',
              { class: 'ag-report-head' },
              h('span', {}, `Dernier rapport (${a.dernierModele || a.modeleLibelle || 'IA'})`),
              h(
                'button',
                {
                  class: 'ag-link-btn',
                  onclick: async () => {
                    await call('effacer_historique', { id: a.id });
                    await refresh();
                  }
                },
                'Effacer l’historique'
              )
            ),
            h('pre', { class: 'ag-report-text' }, a.dernierResultat)
          );
        }
        if (Array.isArray(a.historique) && a.historique.length > 1) {
          const histList = h('div', { class: 'ag-hist-list' }, h('div', { class: 'ag-hist-title' }, 'Exécutions précédentes :'));
          for (const hItem of a.historique.slice(1, 5)) {
            histList.append(
              h(
                'div',
                { class: 'ag-hist-row' },
                h('span', { class: 'ag-hist-meta' }, `${formatAgo(hItem.at)} · ${hItem.declencheur || ''}`),
                h('span', { class: 'ag-hist-snip' }, (hItem.resultat || '').slice(0, 120) + '…')
              )
            );
          }
          repBox.append(histList);
        }
        item.append(repBox);
      }

      list.append(item);
    }

    wrap.append(list);
    return wrap;
  }

  // --- Vue d'édition d'un agent (Hybride : par demande IA + formulaire structuré complet) -----------------

  function renderEditView(a) {
    const d = JSON.parse(JSON.stringify(a.declencheurs || {}));
    d.horaire = Object.assign({ actif: false, type: 'quotidien', heure: '09:00', minutes: 60, jours: [1, 2, 3, 4, 5, 6, 7] }, d.horaire || {});
    d.materiel = Object.assign({ actif: false, cpuPct: 85, ramPct: 85, gpuPct: 85, gpuTemp: 83, dureeSecondes: 15, cooldownMinutes: 30 }, d.materiel || {});
    d.mails = Object.assign({ actif: false, mode: 'resume_jour', filtre: '' }, d.mails || {});
    d.demande = Object.assign({ actif: false, motsCles: [] }, d.demande || {});

    const editState = {
      nom: a.nom || '',
      icone: a.icone || '◈',
      description: a.description || '',
      promptOriginal: a.promptOriginal || '',
      choixModele: currentModelSelectValue(a),
      consigneIa: a.consigneIa || '',
      sources: new Set(Array.isArray(a.sources) ? a.sources : []),
      sortieMode: (a.sortie && a.sortie.mode) || 'voix',
      telegram: Boolean(a.sortie && a.sortie.telegram),
      declencheurs: d,
      motsClesStr: (d.demande.motsCles || []).join(', ')
    };

    const wrap = h('div', { class: 'ag-edit-wrap' });
    wrap.append(
      h(
        'div',
        { class: 'ag-sec-head' },
        h('span', { class: 'ag-sec-title' }, `✎ MODIFIER L'AGENT : ${a.nom}`),
        h(
          'button',
          {
            class: 'ag-btn-sm',
            onclick: () => {
              editingId = null;
              setStatus(null);
              render();
            }
          },
          '← Retour à la liste'
        )
      )
    );

    // 1. Re-génération / modification rapide par demande en langage naturel
    const promptTa = h('textarea', {
      class: 'ag-textarea',
      rows: 2,
      value: editState.promptOriginal,
      oninput: (e) => {
        editState.promptOriginal = e.target.value;
      }
    });

    const promptBox = h(
      'div',
      { class: 'ag-edit-section' },
      h('div', { class: 'ag-edit-lbl' }, 'Demande en langage naturel (réinterpréter avec l’IA) :'),
      promptTa,
      h(
        'button',
        {
          class: 'ag-btn-sm',
          onclick: async () => {
            if (!editState.promptOriginal.trim()) return;
            setStatus('Mise à jour de l’agent depuis la demande…');
            const r = await call('modifier_prompt', { id: a.id, prompt: editState.promptOriginal });
            if (r && r.ok) {
              editingId = null;
              setStatus(`Agent « ${r.agent.nom} » mis à jour.`);
              await refresh();
            } else {
              setStatus((r && r.error) || 'Échec de la mise à jour.', true);
            }
          }
        },
        '✦ Réanalyser cette demande avec l’IA'
      )
    );

    // 2. Identité, Modèle IA et Restitution
    const gridId = h(
      'div',
      { class: 'ag-edit-grid' },
      h(
        'label',
        { class: 'ag-field' },
        h('span', {}, 'Icône'),
        h('input', {
          class: 'ag-input sm',
          value: editState.icone,
          oninput: (e) => {
            editState.icone = e.target.value;
          }
        })
      ),
      h(
        'label',
        { class: 'ag-field grow' },
        h('span', {}, 'Nom de l’agent'),
        h('input', {
          class: 'ag-input',
          value: editState.nom,
          oninput: (e) => {
            editState.nom = e.target.value;
          }
        })
      ),
      h(
        'label',
        { class: 'ag-field' },
        h('span', {}, 'Modèle IA (min. 3.8 Flash / Haiku)'),
        h(
          'select',
          {
            class: 'ag-select',
            onchange: (e) => {
              editState.choixModele = e.target.value;
            }
          },
          ...(state.modeles || []).map((m) =>
            h('option', { value: m.id, selected: editState.choixModele === m.id }, m.label)
          )
        )
      )
    );

    const descInput = h(
      'label',
      { class: 'ag-field full' },
      h('span', {}, 'Description courte'),
      h('input', {
        class: 'ag-input',
        value: editState.description,
        oninput: (e) => {
          editState.description = e.target.value;
        }
      })
    );

    // 3. Déclencheurs : Horaire, Matériel (CPU/RAM/GPU), Mails, À la demande
    const trigBox = h(
      'div',
      { class: 'ag-edit-section' },
      h('div', { class: 'ag-edit-lbl' }, 'Capteurs & Déclencheurs de surveillance :'),
      // Horaire
      h(
        'div',
        { class: 'ag-trig-row' },
        h(
          'label',
          { class: 'ag-chk' },
          h('input', {
            type: 'checkbox',
            checked: editState.declencheurs.horaire.actif,
            onchange: (e) => {
              editState.declencheurs.horaire.actif = e.target.checked;
            }
          }),
          '⏱ Horaire :'
        ),
        h(
          'select',
          {
            class: 'ag-select sm',
            onchange: (e) => {
              editState.declencheurs.horaire.type = e.target.value;
            }
          },
          h('option', { value: 'quotidien', selected: editState.declencheurs.horaire.type === 'quotidien' }, 'Chaque jour à'),
          h('option', { value: 'intervalle', selected: editState.declencheurs.horaire.type === 'intervalle' }, 'Intervalle (min)')
        ),
        h('input', {
          class: 'ag-input sm',
          type: 'time',
          value: editState.declencheurs.horaire.heure,
          oninput: (e) => {
            editState.declencheurs.horaire.heure = e.target.value;
          }
        }),
        h('span', { class: 'ag-hint' }, 'ou toutes les'),
        h('input', {
          class: 'ag-input xs',
          type: 'number',
          min: 5,
          max: 1440,
          value: editState.declencheurs.horaire.minutes,
          oninput: (e) => {
            editState.declencheurs.horaire.minutes = parseInt(e.target.value, 10) || 60;
          }
        }),
        h('span', { class: 'ag-hint' }, 'min')
      ),
      // Matériel PC
      h(
        'div',
        { class: 'ag-trig-row' },
        h(
          'label',
          { class: 'ag-chk' },
          h('input', {
            type: 'checkbox',
            checked: editState.declencheurs.materiel.actif,
            onchange: (e) => {
              editState.declencheurs.materiel.actif = e.target.checked;
            }
          }),
          '⚙ Seuils PC :'
        ),
        h('span', { class: 'ag-hint' }, 'CPU % >'),
        h('input', {
          class: 'ag-input xs',
          type: 'number',
          min: 30,
          max: 99,
          value: editState.declencheurs.materiel.cpuPct,
          oninput: (e) => {
            editState.declencheurs.materiel.cpuPct = parseInt(e.target.value, 10) || 85;
          }
        }),
        h('span', { class: 'ag-hint' }, 'RAM % >'),
        h('input', {
          class: 'ag-input xs',
          type: 'number',
          min: 30,
          max: 99,
          value: editState.declencheurs.materiel.ramPct,
          oninput: (e) => {
            editState.declencheurs.materiel.ramPct = parseInt(e.target.value, 10) || 85;
          }
        }),
        h('span', { class: 'ag-hint' }, 'GPU % >'),
        h('input', {
          class: 'ag-input xs',
          type: 'number',
          min: 30,
          max: 99,
          value: editState.declencheurs.materiel.gpuPct,
          oninput: (e) => {
            editState.declencheurs.materiel.gpuPct = parseInt(e.target.value, 10) || 85;
          }
        })
      ),
      // Mails
      h(
        'div',
        { class: 'ag-trig-row' },
        h(
          'label',
          { class: 'ag-chk' },
          h('input', {
            type: 'checkbox',
            checked: editState.declencheurs.mails.actif,
            onchange: (e) => {
              editState.declencheurs.mails.actif = e.target.checked;
            }
          }),
          '✉ Surveillance Mails :'
        ),
        h(
          'select',
          {
            class: 'ag-select sm',
            onchange: (e) => {
              editState.declencheurs.mails.mode = e.target.value;
            }
          },
          h('option', { value: 'resume_jour', selected: editState.declencheurs.mails.mode === 'resume_jour' }, 'Résumé des mails'),
          h('option', { value: 'nouveau_mail', selected: editState.declencheurs.mails.mode === 'nouveau_mail' }, 'Dès qu’un nouveau mail arrive')
        ),
        h('input', {
          class: 'ag-input',
          placeholder: 'Filtre expéditeur ou sujet (optionnel)',
          value: editState.declencheurs.mails.filtre,
          oninput: (e) => {
            editState.declencheurs.mails.filtre = e.target.value;
          }
        })
      ),
      // Sur demande vocale
      h(
        'div',
        { class: 'ag-trig-row' },
        h(
          'label',
          { class: 'ag-chk' },
          h('input', {
            type: 'checkbox',
            checked: editState.declencheurs.demande.actif,
            onchange: (e) => {
              editState.declencheurs.demande.actif = e.target.checked;
            }
          }),
          '🗣 Sur demande (mots-clés) :'
        ),
        h('input', {
          class: 'ag-input grow',
          placeholder: 'Ex: matchs du top 14, programme rugby…',
          value: editState.motsClesStr,
          oninput: (e) => {
            editState.motsClesStr = e.target.value;
          }
        })
      )
    );

    // 4. Sources de données & Consigne IA
    const srcRow = h('div', { class: 'ag-trig-row' }, h('span', { class: 'ag-edit-lbl-inline' }, 'Sources fournies à l’IA :'));
    for (const [sid, slbl] of [
      ['mails', '✉ Mails Gmail'],
      ['materiel', '⚙ Télémétrie PC & Processus'],
      ['web', '🌐 Recherche Web en direct'],
      ['agenda', '▦ Google Agenda']
    ]) {
      srcRow.append(
        h(
          'label',
          { class: 'ag-chk' },
          h('input', {
            type: 'checkbox',
            checked: editState.sources.has(sid),
            onchange: (e) => {
              if (e.target.checked) editState.sources.add(sid);
              else editState.sources.delete(sid);
            }
          }),
          slbl
        )
      );
    }

    const consigneField = h(
      'label',
      { class: 'ag-field full' },
      h('span', {}, 'Consigne précise donnée à l’IA lors de l’exécution :'),
      h('textarea', {
        class: 'ag-textarea',
        rows: 3,
        value: editState.consigneIa,
        oninput: (e) => {
          editState.consigneIa = e.target.value;
        }
      })
    );

    // 5. Sortie & Enregistrement
    const bottomRow = h(
      'div',
      { class: 'ag-create-opts' },
      h(
        'select',
        {
          class: 'ag-select',
          onchange: (e) => {
            editState.sortieMode = e.target.value;
          }
        },
        h('option', { value: 'voix', selected: editState.sortieMode === 'voix' }, '🔊 Voix + Card HUD'),
        h('option', { value: 'visuel', selected: editState.sortieMode === 'visuel' }, '👁 Card HUD seule'),
        h('option', { value: 'silencieux', selected: editState.sortieMode === 'silencieux' }, '🔇 Silencieux (historique)')
      ),
      h(
        'label',
        { class: 'ag-chk' },
        h('input', {
          type: 'checkbox',
          checked: editState.telegram,
          onchange: (e) => {
            editState.telegram = e.target.checked;
          }
        }),
        '✈ Telegram'
      ),
      h(
        'div',
        { class: 'ag-create-btns' },
        h(
          'button',
          {
            class: 'ag-btn',
            onclick: () => {
              editingId = null;
              render();
            }
          },
          'Annuler'
        ),
        h(
          'button',
          {
            class: 'ag-btn primary',
            onclick: async () => {
              const { moteur, modele } = parseModelSelection(editState.choixModele);
              editState.declencheurs.demande.motsCles = editState.motsClesStr
                .split(',')
                .map((s) => s.trim())
                .filter(Boolean);
              const patch = {
                nom: editState.nom,
                icone: editState.icone,
                description: editState.description,
                promptOriginal: editState.promptOriginal,
                moteur,
                modele,
                declencheurs: editState.declencheurs,
                sources: [...editState.sources],
                consigneIa: editState.consigneIa,
                sortie: { mode: editState.sortieMode, telegram: editState.telegram }
              };
              const r = await call('modifier', { id: a.id, patch });
              if (r && r.ok) {
                editingId = null;
                setStatus(`Modifications enregistrées pour « ${r.agent.nom} ».`);
                await refresh();
              } else {
                setStatus((r && r.error) || 'Erreur lors de la sauvegarde.', true);
              }
            }
          },
          '💾 Enregistrer les modifications'
        )
      )
    );

    wrap.append(promptBox, gridId, descInput, trigBox, srcRow, consigneField, bottomRow);
    return wrap;
  }

  if (bridge && bridge.onTaskEvent) {
    bridge.onTaskEvent((ev) => {
      if (!ev) return;
      if (ev.type === 'agents_open') open();
      else if (ev.type === 'agents_changed' && card.isConnected && !card.classList.contains('w-ghost')) {
        refresh();
      }
    });
  }

  J.agentsCard = { open, refresh };
})();
