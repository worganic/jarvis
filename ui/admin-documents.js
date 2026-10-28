// Section « Documents » de l'administration : les dossiers que Jarvis a le droit de lire (PDF, Word, texte, Markdown, CSV)
// pour répondre aux questions sur tes documents (outil documents_chercher de l'IA, lecture seule). Seuls les chiffres
// sont affichés ici, jamais le contenu.
(function () {
  const A = window.JarvisAdmin;
  if (!A) return;
  const { h, panel, api, sections, render, toast, fmtDate } = A;

  const state = { data: null, loading: false, path: '', timer: null };

  async function load() {
    if (state.loading) return;
    state.loading = true;
    const res = await api('documentsState');
    state.loading = false;
    if (res && res.ok) state.data = res;
    render(true);
    // Analyse en cours : on revient voir où elle en est.
    clearTimeout(state.timer);
    if (res && res.stats && res.stats.analyse) state.timer = setTimeout(load, 3000);
  }

  const previous = A.onJournal;
  A.onJournal = (ev) => {
    if (previous) previous(ev);
    if (ev && ev.type === 'documents' && document.getElementById('section-documents').classList.contains('active')) load();
  };

  async function add() {
    const path = state.path.trim();
    if (!path) return;
    const res = await api('documentsAdd', path);
    if (res && res.ok) { state.path = ''; state.data = res; toast('Dossier ajouté : analyse en cours…'); render(true); setTimeout(load, 1500); } else toast((res && (res.error || res.message)) || 'Échec', true);
  }

  async function scan() {
    const res = await api('documentsScan');
    if (res && res.ok) { toast('Analyse lancée'); setTimeout(load, 800); } else toast((res && res.error) || 'Échec', true);
  }

  async function remove(path) {
    const res = await api('documentsRemove', path);
    if (res && res.ok) { state.data = res; render(true); }
  }

  sections.documents = () => {
    if (!state.data && !state.loading) load();
    const d = state.data;
    if (!d) return [h('h1', {}, 'Documents'), h('div', { class: 'empty' }, 'Chargement…')];
    const s = d.stats || {};
    const input = h('input', { type: 'text', placeholder: 'Dossier à lire, ex. C:\Users\moi\Documents\Contrats', value: state.path, style: 'min-width:360px' });
    input.addEventListener('input', () => { state.path = input.value; });
    input.addEventListener('keydown', (e) => { if (e.key === 'Enter') add(); });
    return [
      h('h1', {}, 'Documents'),
      h('p', { class: 'lead' }, "Choisis les dossiers que Jarvis peut lire : PDF, Word (.docx), texte, Markdown et CSV y sont découpés en passages et retrouvés par le sens. Ensuite demande par exemple « que dit mon bail sur le préavis ? » : l'IA cherche dans ces documents (lecture seule) et seuls les extraits utiles lui sont transmis. Rien n'est lu hors de ces dossiers ; les zones interdites sont refusées."),
      panel('Dossiers suivis', `${(d.dossiers || []).length}`,
        h('div', { class: 'row' }, input, h('button', { class: 'btn primary', onclick: add }, 'Ajouter'), h('button', { class: 'btn ghost', onclick: scan, title: 'Relit les dossiers : seuls les fichiers nouveaux ou modifiés sont repris' }, '⟳ Analyser maintenant')),
        (d.dossiers || []).length
          ? h('table', {}, h('thead', {}, h('tr', {}, h('th', {}, 'Dossier'), h('th', {}, 'Fichiers'), h('th', {}, ''))),
            h('tbody', {}, ...d.dossiers.map((f) => h('tr', {}, h('td', {}, f.chemin + (f.existe ? '' : ' (introuvable)')), h('td', {}, String(f.fichiers)), h('td', {}, h('button', { class: 'btn danger', onclick: () => remove(f.chemin) }, 'Retirer'))))))
          : h('div', { class: 'empty' }, 'Aucun dossier suivi.')),
      panel('Index', s.analyse ? 'analyse en cours…' : (s.derniereAnalyse ? `dernière analyse ${fmtDate(s.derniereAnalyse)}` : 'pas encore analysé'),
        h('div', { class: 'dim' }, `${s.fichiers || 0} fichier(s) · ${s.passages || 0} passage(s) · ${s.vecteurs || 0} vecteur(s) · modèle ${s.modele || '—'} ${s.modeleInstalle ? '(installé)' : "(non installé : recherche par mots seulement)"}. Formats : ${(s.extensions || []).join(', ')}.`))
    ];
  };
})();
