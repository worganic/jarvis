// Panneau « Prononciation » de la page Voix : mots que la voix lit mal (noms de projets, anglais,
// sigles) et la façon de les dire. Appliqué juste avant la synthèse ; l'écran garde l'orthographe.
(function () {
  const A = window.JarvisAdmin;
  if (!A || !A.sections.voice) return;
  const { h, panel, api, sections, render, toast, textInput, select } = A;

  const state = { data: null, loading: false, spoken: '', test: 'Airbus Helicopters livre le H160 à Marseille.', forVoice: '', compareVoice: '' };

  async function load(method = 'pronunciationState', ...args) {
    state.loading = true;
    const res = await api(method, ...args);
    state.loading = false;
    if (res && res.ok) state.data = res;
    else if (res && res.error) toast(`✕ ${res.error}`, true);
    render(true);
    return res;
  }

  function pronunciationPanel() {
    if (!state.data && !state.loading) load();
    const d = state.data || { entries: [], builtin: [], voices: [] };
    const voiceName = (id) => (d.voices.find((v) => v.id === id) || {}).name || id;
    // Règle pour toutes les voix, ou seulement pour la voix choisie (chacune a ses défauts).
    const forVoice = select({ '': 'Toutes les voix', ...(d.voice ? { [d.voice]: `Seulement ${voiceName(d.voice)}` } : {}) }, state.forVoice || '', (v) => { state.forVoice = v; });
    const word = textInput('', { placeholder: 'mot écrit (ex. coaster)', autocomplete: 'off', spellcheck: 'false' });
    const say = textInput('', { placeholder: 'comme le dire (ex. côsteur)', autocomplete: 'off' });
    const add = async () => {
      if (!word.value.trim() || !say.value.trim()) return;
      const res = await load('pronunciationSet', word.value, say.value, state.forVoice || '');
      if (res && res.ok) toast(`✓ « ${word.value.trim()} » se dira « ${say.value.trim()} »`);
    };
    say.addEventListener('keydown', (e) => { if (e.key === 'Enter') add(); });
    const test = textInput(state.test, { placeholder: 'phrase à écouter' });
    test.addEventListener('input', () => { state.test = test.value; });
    const listen = async () => {
      const res = await api('pronunciationTest', test.value);
      if (res && res.ok) { state.spoken = res.spoken; render(true); }
    };
    test.addEventListener('keydown', (e) => { if (e.key === 'Enter') listen(); });

    return panel('Prononciation', `${d.entries.length} mot${d.entries.length > 1 ? 's' : ''}`,
      h('div', { class: 'help', style: 'margin-bottom:10px' },
        'Réécriture juste avant la voix (l’écran garde l’orthographe). Automatique : élisions oubliées (« je écoute » → « j’écoute »), noms de projets découpés (« lego-rollerCoaster » → « lego roller coaster »), sigles et mots techniques. Pour un mot mal dit, écris-le comme il se prononce en français. À la voix : « prononce coaster comme côsteur ».'),
      h('div', { class: 'row', style: 'gap:8px; flex-wrap:wrap' }, word, say, forVoice,
        h('button', { class: 'btn primary', onclick: add }, 'Ajouter')),
      h('div', { class: 'help', style: 'margin-top:6px' }, 'Un mot ou une expression de 4 mots au plus (« Airbus Helicopters »). Plus simple : card Diction du HUD (« Jarvis, corrige ta prononciation ») pour toucher le mot dans ses dernières phrases.'),
      d.entries.length ? h('table', { style: 'margin-top:12px' },
        h('thead', {}, h('tr', {}, h('th', {}, 'Mot'), h('th', {}, 'Dit'), h('th', {}, 'Voix'), h('th', {}, ''))),
        h('tbody', {}, ...d.entries.map((e) => h('tr', {},
          h('td', {}, h('strong', {}, e.word)),
          h('td', {}, e.say),
          h('td', { class: 'help' }, e.voice ? voiceName(e.voice) : 'toutes'),
          h('td', { class: 'actions', style: 'white-space:nowrap' },
            h('button', { class: 'btn ghost', title: 'Écouter', onclick: () => api('pronunciationTest', e.word) }, '▶'),
            h('button', { class: 'btn ghost', title: 'Supprimer', onclick: () => load('pronunciationRemove', e.word, e.voice || '') }, '✕'))))))
        : h('div', { class: 'help', style: 'margin-top:10px' }, 'Aucun mot personnel pour l’instant.'),
      h('div', { class: 'row', style: 'gap:8px; margin-top:14px' }, test,
        h('button', { class: 'btn', onclick: listen }, '▶ Écouter')),
      state.spoken ? h('div', { class: 'help', style: 'margin-top:6px' }, `Dit : « ${state.spoken} »`) : null,
      multilingual(d, test),
      h('details', { style: 'margin-top:12px' },
        h('summary', { class: 'help', style: 'cursor:pointer' }, `Lexique intégré (${d.builtin.length} mots, remplaçables par les tiens)`),
        h('div', { class: 'chips', style: 'margin-top:8px' },
          ...d.builtin.map((e) => h('span', { class: 'chip passive' }, `${e.word} → ${e.say}`)))));
  }

  // Voix multilingues : elles disent les noms anglais naturellement au milieu du français. Comparaison
  // avec la voix actuelle sur la même phrase, sans changer de voix (le choix se fait dans les réglages de voix).
  function multilingual(d, test) {
    const multi = d.voices.filter((v) => /Multilingual/.test(v.id));
    if (!multi.length) return null;
    const pickVoice = select(Object.fromEntries(multi.map((v) => [v.id, v.name])), state.compareVoice || multi[0].id, (v) => { state.compareVoice = v; });
    return h('div', { style: 'margin-top:14px' },
      h('div', { class: 'help' }, 'Voix multilingues : elles prononcent l’anglais à l’anglaise au milieu du français (Airbus Helicopters, Bluetooth…). Compare sur la phrase ci-dessus :'),
      h('div', { class: 'row', style: 'gap:8px; margin-top:6px' }, pickVoice,
        h('button', { class: 'btn', onclick: async () => {
          const text = test.value.trim() || 'Airbus Helicopters livre le H160 à Marseille.';
          await api('pronunciationTest', text);
          await api('pronunciationCompare', text, state.compareVoice || multi[0].id);
          toast('▶ voix actuelle, puis voix multilingue');
        } }, '▶ Comparer')));
  }

  const original = sections.voice;
  sections.voice = () => {
    const out = original();
    return [...(Array.isArray(out) ? out : [out]), pronunciationPanel()];
  };
})();
