// Section « Mise à jour » de l'administration (Jarvis installé par installer.ps1, crates/jarvis-core/src/update.rs) :
// version installée, nouvelle version publiée et ses nouveautés, préparation (récupérée et compilée en arrière-plan),
// installation avec redémarrage, réglage automatique / demander / manuel, et modules à installer ou retirer.
// En développement (Jarvis lancé depuis ses sources), la section explique la recompilation locale et la publication.
(function () {
  const A = window.JarvisAdmin;
  const direct = window.jarvisAdmin && window.jarvisAdmin.direct;
  if (!A || !direct) return;
  const { h, panel, field, select, save, sections, render, toast, getSnapshot, confirmDialog } = A;

  let st = null;
  let busy = false;
  const when = (iso) => (iso ? new Date(iso).toLocaleString('fr-FR', { dateStyle: 'short', timeStyle: 'short' }) : 'jamais');

  async function load() {
    st = await direct('updateState').catch(() => ({ installe: false }));
    render(true);
  }
  if (window.jarvisAdmin.onEvent) window.jarvisAdmin.onEvent((ev) => { if (ev && ev.type === 'update_state') { st = ev; render(); } });

  async function run(method, ...args) {
    busy = true;
    render(true);
    const res = await direct(method, ...args).catch((e) => ({ ok: false, error: String(e.message || e) }));
    busy = false;
    if (res && res.etat) st = res.etat;
    if (res && res.ok === false) toast(`✕ ${res.error || 'échec'}`, true);
    render(true);
    return res;
  }

  async function moduleAction(m, add) {
    const text = add
      ? `Installer « ${m.nom} » ? ${m.noyau ? 'Ce module a une partie noyau : il sera compilé (quelques minutes) puis installé au prochain redémarrage de Jarvis.' : 'Il apparaîtra sans redémarrer Jarvis.'}`
      : `Retirer « ${m.nom} » ? Ses données sont gardées et reviennent s’il est réinstallé.${m.noyau ? ' Le retrait d’une partie noyau se fait au prochain redémarrage.' : ''}`;
    if (!(await confirmDialog(add ? 'INSTALLER UN MODULE' : 'RETIRER UN MODULE', text, add ? 'Installer' : 'Retirer', !add))) return;
    const res = await run('updateModule', add ? 'ajouter' : 'retirer', m.id);
    if (res && res.ok) toast('Une fenêtre suit l’opération ; la liste se met à jour à la fin.');
  }

  function modulesPanel() {
    const cat = Array.isArray(st.catalogue) ? st.catalogue : [];
    if (!cat.length) return null;
    const have = new Set(Array.isArray(st.modules) ? st.modules : []);
    return panel('Modules installés et disponibles', `${have.size} / ${cat.length}`,
      h('div', { class: 'help' }, 'Le socle est toujours installé. Un module d’interface seule s’ajoute ou se retire aussitôt ; un module à partie noyau est compilé puis installé au redémarrage. Retirer un module garde ses données.'),
      h('table', { class: 'meter-table', style: 'margin-top: 8px' }, h('tbody', {},
        ...cat.map((m) => h('tr', {},
          h('td', {}, h('strong', {}, m.nom), h('div', { class: 'dim' }, m.description || '')),
          h('td', {}, m.socle ? h('span', { class: 'badge ok' }, 'socle') : have.has(m.id) ? h('span', { class: 'badge ok' }, 'installé') : h('span', { class: 'badge off' }, 'non installé')),
          h('td', {}, m.noyau ? 'compilé' : 'interface'),
          h('td', { class: 'num' }, m.socle ? '' : h('button', { class: `btn small${have.has(m.id) ? ' danger' : ''}`, disabled: busy, onclick: () => moduleAction(m, !have.has(m.id)) }, have.has(m.id) ? 'Retirer' : 'Installer')))))));
  }

  sections.maj = () => {
    if (!st) {
      load();
      return [h('h1', {}, 'Mise à jour'), h('div', { class: 'empty' }, 'Chargement…')];
    }
    if (!st.installe) {
      return [
        h('h1', {}, 'Mise à jour'),
        h('p', { class: 'lead' }, 'Cette copie de Jarvis tourne depuis ses sources (version de développement) : une modification du code se recompile et se relance par le bandeau « Jarvis a été modifié ».'),
        panel('Partager une nouvelle version', null,
          h('div', { class: 'help' }, 'pwsh tools/publier.ps1 publie la version de Cargo.toml sur le dépôt de diffusion (contrôles, export propre, tag) ; les Jarvis installés la voient à leur prochaine vérification. -Beta : canal bêta ; -Essai : contrôle sans rien envoyer ; -Modeles : empaquette les modèles vocaux.')),
      ];
    }
    const cfg = (getSnapshot().config || {}).update || {};
    const notes = st.nouveautes || [];
    return [
      h('h1', {}, 'Mise à jour'),
      h('p', { class: 'lead' }, `Jarvis ${st.actuelle} (canal ${st.canal}), installé depuis github.com/${st.depot}. Une nouvelle version est récupérée puis compilée sur ce PC pendant que Jarvis reste utilisable, puis installée en moins d’une minute au redémarrage ; la version précédente est gardée pour revenir en arrière.`),
      panel('État', `vérifié : ${when(st.verifieLe)}`,
        h('div', { class: 'row' },
          st.pret ? h('span', { class: 'badge ok' }, `${st.pret} prête`) : st.preparation ? h('span', { class: 'badge warn' }, `préparation de ${st.preparationDe || ''}…`)
            : st.disponible ? h('span', { class: 'badge warn' }, `${st.disponible} disponible`) : h('span', { class: 'badge ok' }, 'à jour'),
          h('button', { class: 'btn small', disabled: busy, onclick: () => run('updateCheck') }, 'Vérifier maintenant'),
          st.disponible && !st.pret && !st.preparation ? h('button', { class: 'btn primary', disabled: busy, onclick: () => run('updatePrepare') }, 'Préparer la mise à jour') : null,
          st.pret ? h('button', { class: 'btn primary', disabled: busy, onclick: () => run('updateApply') }, 'Installer et redémarrer') : null),
        st.erreur ? h('div', { class: 'help', style: 'color: var(--err); margin-top: 8px' }, st.erreur) : null,
        notes.length ? h('ul', { style: 'margin-top: 10px' }, ...notes.map((v) => h('li', {}, h('strong', {}, v.version), ` — ${v.titre || ''}`, v.date ? h('span', { class: 'dim' }, ` (${v.date})`) : null))) : null),
      panel('Réglage', null,
        field('Mises à jour', select([
          ['auto', 'Automatiques : préparées seules, installées à la fermeture de Jarvis'],
          ['demander', 'Me demander : Jarvis annonce la version, je choisis quand'],
          ['manuel', 'Manuelles : aucune vérification automatique'],
        ], cfg.mode || 'demander', (v) => save('setConfig', { update: { mode: v } })),
        'La vérification a lieu une minute après le démarrage puis toutes les 6 heures (sauf en manuel). Le canal (stable ou bêta) se choisit à l’installation.')),
      modulesPanel(),
    ];
  };
})();
