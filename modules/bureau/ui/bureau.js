// Module Bureau : la card 1 (fenêtres de chaque écran, applications en arrière-plan, emplacements 1-4). C'est la card
// de liste compacte du système (renderRows, mode compact : miniature / agrandie au clic sur ⇔, réductible en bouton
// jaune) ; le module y ajoute, tout en bas, le bouton de capture d'écran. Noyau : orch/desktop_intents.rs, desktop_state.rs.
(function () {
  const J = (window.JARVIS = window.JARVIS || {});
  const bridge = window.jarvisBridge;
  const K = J.widgetKit;
  if (!K) return;
  const { h } = K;

  // Capture d'écran : outil de capture de Windows, avec le curseur de sélection de la zone (méthode screenClip).
  function shotButton() {
    return h('button', {
      class: 'w-shot', title: "Capture d'écran : choisir la zone à capturer",
      onclick: (e) => { e.stopPropagation(); if (bridge && bridge.screenClip) bridge.screenClip(); }
    }, '📷');
  }

  K.footButtons((ev) => (ev.tool === 'bureau' && ev.compact ? shotButton() : null));
})();
