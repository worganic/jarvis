// Module modèle : une card minimale, à copier pour créer un module (voir modules/README.md).
// Elle utilise l'en-tête commun des cards (J.cardHeader : poignée, numéro, titre, 📍 📌 –) et la passerelle
// (window.jarvisBridge : bridge.call('méthode', …) pour le noyau, bridge.onTaskEvent(cb) pour ses événements).
(function () {
  const J = (window.JARVIS = window.JARVIS || {});
  const bridge = window.jarvisBridge;
  const ID = 'modele'; // = l'id de module.json (et le nom du dossier)
  const info = (J.modules && J.modules[ID]) || {};

  const card = document.createElement('div');
  card.className = 'widget w-active mod-card';
  card.id = `mod-${ID}`;
  const hdr = J.cardHeader({
    key: ID,
    no: J.CARD_NO[ID],
    title: (info.card && info.card.titre) || 'MODÈLE',
    onCollapse: () => card.classList.toggle('mod-collapsed')
  });
  const body = document.createElement('div');
  body.className = 'mod-body';
  body.textContent = 'Bonjour depuis le module modèle.';
  card.append(hdr.head, body);
  document.body.appendChild(card);

  // Déplacement à la souris par l'en-tête.
  let drag = null;
  hdr.head.addEventListener('pointerdown', (e) => {
    if (e.target.closest('button')) return;
    const r = card.getBoundingClientRect();
    drag = { dx: e.clientX - r.left, dy: e.clientY - r.top };
    hdr.head.setPointerCapture(e.pointerId);
  });
  hdr.head.addEventListener('pointermove', (e) => {
    if (!drag) return;
    card.style.left = `${e.clientX - drag.dx}px`;
    card.style.top = `${e.clientY - drag.dy}px`;
  });
  hdr.head.addEventListener('pointerup', () => { drag = null; });

  // Exemple d'événement du noyau : l'heure du statut, si présente.
  if (bridge && bridge.onTaskEvent) {
    bridge.onTaskEvent((ev) => {
      if (ev && ev.type === 'status' && ev.time) body.textContent = `Statut reçu à ${ev.time}.`;
    });
  }
})();
