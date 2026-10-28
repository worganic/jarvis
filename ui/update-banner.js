// Bandeau « Jarvis a été modifié » : le noyau surveille le code (core/code-watch.js) et le signale.
// Un clic relance Jarvis entièrement (« Jarvis, relance-toi » fait de même à la voix).
// Jarvis installé (update.rs) : le même bandeau annonce une nouvelle version publiée (clic : la préparer, c.-à-d. la
// récupérer et la compiler en arrière-plan), sa préparation, puis la version prête (clic : installer et redémarrer).
(function () {
  const bridge = window.jarvisBridge;
  if (!bridge || !bridge.onTaskEvent) return;

  const el = document.createElement('button');
  el.id = 'update-banner';
  el.type = 'button';
  el.className = 'hidden';
  el.innerHTML = '<span class="ub-icon">⟳</span><span class="ub-text"><span class="ub-title"></span><span class="ub-sub"></span></span>';
  document.body.appendChild(el);
  const title = el.querySelector('.ub-title');
  const sub = el.querySelector('.ub-sub');
  let busy = false;
  let code = null; // dernier état du code surveillé (développement)
  let upd = null; // dernier état des mises à jour (Jarvis installé)

  // Ce que le bandeau propose : 'code' (relancer), 'apply' (installer), 'prepare' (préparer), 'wait' ou null.
  function mode() {
    if (code && code.changed) return 'code';
    if (!upd || !upd.installe) return null;
    if (upd.pret) return 'apply';
    if (upd.preparation) return 'wait';
    if (upd.disponible) return 'prepare';
    return null;
  }

  function render() {
    if (busy) return;
    const m = mode();
    el.title = '';
    if (m === 'code') {
      title.textContent = 'Jarvis a été modifié — cliquer pour relancer';
      const n = code.count || (code.files || []).length;
      sub.textContent = `${n} fichier${n > 1 ? 's' : ''} modifié${n > 1 ? 's' : ''}`
        + (code.needsBuild ? ' · à recompiler (quelques minutes)' : ' · relance en quelques secondes');
      el.title = (code.files || []).join('\n');
    } else if (m === 'apply') {
      title.textContent = `Mise à jour ${upd.pret} prête — cliquer pour installer et redémarrer`;
      sub.textContent = 'Jarvis se ferme, se met à jour puis se rouvre (moins d’une minute)';
    } else if (m === 'wait') {
      title.textContent = `Préparation de la mise à jour ${upd.preparationDe || ''}…`;
      sub.textContent = 'compilation en arrière-plan : Jarvis reste utilisable';
    } else if (m === 'prepare') {
      title.textContent = `Nouvelle version ${upd.disponible} — cliquer pour la préparer`;
      const notes = upd.nouveautes || [];
      const more = notes.length > 1 ? ` (+ ${notes.length - 1} autre${notes.length > 2 ? 's' : ''} nouveauté${notes.length > 2 ? 's' : ''})` : '';
      sub.textContent = notes.length ? `${notes[0].titre || ''}${more}` : 'récupérée et compilée sur ce PC (plusieurs minutes)';
      el.title = notes.map((v) => `${v.version} : ${v.titre}`).join('\n');
    }
    el.className = m === null ? 'hidden' : m === 'wait' ? 'busy' : '';
  }

  function restarting(rebuild) {
    busy = true;
    title.textContent = rebuild ? 'Recompilation de Jarvis…' : 'Redémarrage de Jarvis…';
    sub.textContent = rebuild ? 'Jarvis se ferme, se recompile puis se rouvre' : 'retour dans quelques secondes';
    el.className = 'busy';
  }

  el.addEventListener('click', () => {
    if (busy) return;
    const m = mode();
    if (m === 'apply') {
      busy = true;
      title.textContent = 'Installation de la mise à jour…';
      sub.textContent = 'Jarvis se ferme puis se rouvre';
      el.className = 'busy';
      bridge.call('updateApply').then((r) => { if (!r || !r.ok) { busy = false; render(); } }).catch(() => { busy = false; render(); });
      return;
    }
    if (m === 'prepare') {
      bridge.call('updatePrepare').catch(() => {});
      return;
    }
    if (m !== 'code') return;
    restarting(false);
    bridge.restartJarvis().then((r) => {
      if (r && r.ok) restarting(r.rebuild);
      else busy = false;
    }).catch(() => { busy = false; });
  });

  bridge.onTaskEvent((ev) => {
    if (!ev) return;
    if (ev.type === 'code_changed') {
      busy = false; // envoyé par le noyau (re)démarré : l'état « redémarrage » est terminé
      code = ev;
      render();
    } else if (ev.type === 'jarvis_restarting') restarting(ev.rebuild);
    else if (ev.type === 'update_state') {
      upd = ev;
      render();
    }
  });
  if (bridge.call) bridge.call('updateState').then((st) => { upd = st; render(); }).catch(() => {});
})();
