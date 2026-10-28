// Section « Mes données » de l'administration : où sont les données, export en archive .jarvis (pour un autre PC, un
// autre compte Windows ou une installation neuve) et restauration d'une archive. Le noyau (crates/jarvis-core/src/
// transfer.rs) vérifie l'archive (empreintes, mot de passe) ; l'import est préparé puis appliqué à la relance de Jarvis,
// l'ancien dossier étant gardé à côté.
(function () {
  const A = window.JarvisAdmin;
  if (!A) return;
  const { h, panel, field, api, toast, challengeFlow, confirmDialog, sections, render } = A;

  let state = null; // dernière réponse de transferState
  let busy = false;
  let preview = null; // { nom, apercu }
  const pw = { export: '', export2: '', archive: '' };

  const ko = (n) => (n >= 1048576 ? `${(n / 1048576).toFixed(1)} Mo` : `${Math.max(1, Math.round(n / 1024))} Ko`);
  const when = (iso) => (iso ? new Date(iso).toLocaleString('fr-FR', { dateStyle: 'short', timeStyle: 'short' }) : '—');

  async function refresh() {
    const res = await api('transferState');
    if (res && res.ok) state = res;
    render();
  }

  async function restartNow() {
    if (window.jarvisAdmin && window.jarvisAdmin.direct) await window.jarvisAdmin.direct('restartJarvis');
    toast('Jarvis redémarre et applique l’import…');
  }

  async function doExport() {
    if (pw.export !== pw.export2) return toast('✕ les deux mots de passe diffèrent', true);
    if (!pw.export && !(await confirmDialog('SANS MOT DE PASSE',
      'Sans mot de passe, les secrets des connecteurs (Google, Spotify, Telegram, Nomade…) ne sont pas exportés : il faudra les ressaisir après la restauration. Le reste (mémoire, listes, réglages…) est exporté en clair.',
      'Exporter quand même'))) return;
    busy = true;
    render();
    const res = await api('transferExport', pw.export);
    busy = false;
    if (res && res.ok) {
      pw.export = pw.export2 = '';
      state = res.etat || state;
      toast(`✓ archive créée : ${res.fichiers} fichier(s), ${ko(res.octets)}${res.secrets ? ` · ${res.secrets} secret(s) ${res.chiffre ? 'chiffrés' : 'retirés'}` : ''}`);
    }
    render();
  }

  async function doPreview(nom) {
    busy = true;
    render();
    const res = await api('transferPreview', nom, pw.archive);
    busy = false;
    preview = res && res.ok ? { nom, apercu: res.apercu } : null;
    render();
  }

  async function doImport(nom) {
    const ok = await confirmDialog('RESTAURER UNE ARCHIVE',
      `Toutes tes données actuelles seront remplacées par celles de « ${nom} » au prochain lancement de Jarvis. Le dossier actuel n’est pas effacé : il est gardé à côté (donnees.avant-import-…), et les modèles vocaux y sont repris sans nouveau téléchargement.`,
      'Continuer', true);
    if (!ok) return;
    busy = true;
    render();
    const res = await api('transferImportRequest', nom, pw.archive);
    busy = false;
    render();
    if (!res || !res.challenge) return;
    const done = await challengeFlow(res.challenge, res.confirm);
    if (done && done.ok) {
      state = done.etat || state;
      render();
      if (await confirmDialog('IMPORT PRÊT', 'L’archive est vérifiée et préparée. Relancer Jarvis maintenant pour l’appliquer ?', 'Relancer maintenant')) restartNow();
    }
  }

  function previewView() {
    if (!preview) return null;
    const p = preview.apercu;
    return panel(`Aperçu de ${preview.nom}`, `créée le ${when(p.creeLe)} par Jarvis ${p.jarvis || '?'}`,
      h('table', { class: 'meter-table' }, h('tbody', {},
        ...(p.dossiers || []).map((d) => h('tr', {}, h('td', {}, d.nom), h('td', { class: 'num' }, `${d.fichiers} fichier(s)`), h('td', { class: 'num' }, ko(d.octets)))),
        h('tr', { class: 'total' }, h('td', {}, 'Total'), h('td', { class: 'num' }, `${p.fichiers} fichier(s)`), h('td', { class: 'num' }, ko(p.octets))))),
      h('div', { class: 'help', style: 'margin-top: 10px' },
        p.secrets === 'chiffres' ? 'Secrets des connecteurs inclus, chiffrés par le mot de passe de l’archive.'
          : `Secrets des connecteurs non inclus${(p.secretsRetiresDe || []).length ? ` (${p.secretsRetiresDe.join(', ')})` : ''} : à ressaisir après la restauration.`));
  }

  sections.donnees = () => {
    if (!state) {
      refresh();
      return [h('h1', {}, 'Mes données'), h('div', { class: 'empty' }, 'Chargement…')];
    }
    const archives = state.archives || [];
    return [
      h('h1', {}, 'Mes données'),
      h('p', { class: 'lead' }, 'Tes données (mémoire, savoir, listes, réglages, empreinte vocale, journal scellé…) sont rangées hors du programme : le recompiler, le mettre à jour ou le supprimer ne les touche pas. Ici, tu les emportes dans une archive et tu les restaures sur ce PC ou un autre.'),
      state.importEnAttente ? panel('Import en attente', 'appliqué à la prochaine relance',
        h('div', { class: 'row' },
          h('span', { class: 'badge warn' }, 'prêt'),
          h('span', {}, 'Une archive vérifiée attend la relance de Jarvis pour remplacer les données actuelles.'),
          h('button', { class: 'btn primary', onclick: restartNow }, 'Relancer maintenant'))) : null,
      panel('Emplacement', null,
        h('div', { class: 'row' },
          h('code', {}, state.donnees),
          h('button', { class: 'btn small', onclick: () => api('transferOpenFolder', 'donnees') }, 'Ouvrir le dossier')),
        h('div', { class: 'help', style: 'margin-top: 8px' }, 'Une sauvegarde automatique de ce dossier est aussi faite chaque jour (voir l’aide, « Sauvegarde et dépannage »).')),
      panel('Exporter mes données', 'archive .jarvis',
        field('Mot de passe de l’archive', h('input', { type: 'password', value: pw.export, autocomplete: 'new-password', oninput: (e) => { pw.export = e.target.value; } }),
          '8 caractères au moins. Il protège les secrets des connecteurs (mots de passe d’application, jetons) ; sans lui, ils ne sont pas exportés. Aucun moyen de le retrouver s’il est perdu.'),
        field('Confirmer le mot de passe', h('input', { type: 'password', value: pw.export2, autocomplete: 'new-password', oninput: (e) => { pw.export2 = e.target.value; } })),
        h('div', { class: 'row', style: 'margin-top: 10px' },
          h('button', { class: 'btn primary', disabled: busy, onclick: doExport }, busy ? 'Patiente…' : 'Créer l’archive'),
          h('span', { class: 'help' }, `Écrite dans ${state.dossier}. Les modèles (plusieurs Go), journaux de diagnostic et caches n’y sont pas : ils se retéléchargent ou se recalculent.`))),
      panel('Restaurer une archive', `${archives.length} archive(s) trouvée(s)`,
        h('div', { class: 'help' }, 'Place le fichier .jarvis dans le dossier des sauvegardes pour qu’il apparaisse ici.'),
        h('div', { class: 'row', style: 'margin: 8px 0' },
          h('button', { class: 'btn small', onclick: () => api('transferOpenFolder', 'sauvegardes') }, 'Ouvrir le dossier des sauvegardes'),
          h('button', { class: 'btn small ghost', onclick: refresh }, 'Actualiser')),
        field('Mot de passe de l’archive (si elle en a un)', h('input', { type: 'password', value: pw.archive, autocomplete: 'off', oninput: (e) => { pw.archive = e.target.value; } })),
        archives.length ? h('table', { class: 'meter-table', style: 'margin-top: 8px' }, h('tbody', {},
          ...archives.map((a) => h('tr', {},
            h('td', {}, a.nom),
            h('td', { class: 'num' }, ko(a.octets)),
            h('td', { class: 'num' }, when(a.modifie)),
            h('td', { class: 'num' },
              h('button', { class: 'btn small', disabled: busy, onclick: () => doPreview(a.nom) }, 'Aperçu'), ' ',
              h('button', { class: 'btn small danger', disabled: busy, onclick: () => doImport(a.nom) }, 'Restaurer'))))))
          : h('div', { class: 'empty' }, 'Aucune archive dans ce dossier.')),
      previewView(),
    ];
  };
})();
