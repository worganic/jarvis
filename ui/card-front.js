// Un clic dans une card la passe au-dessus des autres (comme une fenêtre). Les cards de widgets.js (Bureau, Terminal,
// mails, listes, Actions…) se superposent dans #widgets ; les cards Audio et Matériel sont à part (position fixe) ; la
// card Conversation est dessinée sur un canevas : au départ derrière les cards, elle passe sur le canevas de devant
// (#stage-front, voir app.js › drawTranscript) dès qu'on la clique. On classe donc quatre groupes entre eux, et chaque
// card à l'intérieur de #widgets.
(function () {
  const J = (window.JARVIS = window.JARVIS || {});
  const root = document.getElementById('widgets');
  const stage = document.getElementById('stage');
  if (!root || !stage) return;

  const BASE = 4; // même niveau que #widgets dans la feuille de style
  const order = ['widgets', 'audio', 'hw']; // de l'arrière vers l'avant ; au départ : comme dans la feuille de style
  let widgetZ = 100;

  const elements = () => ({ widgets: root, audio: document.getElementById('audio-panel'), hw: document.getElementById('hw-panels'), conversation: document.getElementById('stage-front') });

  function applyGroups() {
    const els = elements();
    order.forEach((g, i) => { if (els[g]) els[g].style.zIndex = String(BASE + i); });
  }

  function raise(group) {
    if (order.includes(group) && order[order.length - 1] === group) return;
    if (order.includes(group)) order.splice(order.indexOf(group), 1);
    order.push(group);
    applyGroups();
  }

  const convRaised = () => order.includes('conversation');
  // La Conversation est-elle devant les cards HTML (donc ses clics ne passent plus par elles) ?
  const convAbove = () => convRaised() && order.indexOf('conversation') > order.indexOf('widgets');
  // Card de #widgets passée devant toutes les autres (clic, ou ouverture : widgets.js › place, modules).
  function front(card) {
    if (!card || !root.contains(card)) return;
    raise('widgets');
    if (Number(card.style.zIndex || 0) <= widgetZ) card.style.zIndex = String(++widgetZ);
  }
  J.cardFront = { convRaised, convAbove, front };

  function inConversation(e) {
    const b = J.conversationCard && J.conversationCard.box ? J.conversationCard.box() : null;
    return Boolean(b && e.clientX >= b.x && e.clientX <= b.x + b.w && e.clientY >= b.y && e.clientY <= b.y + b.h);
  }

  function groupOf(el) {
    if (el.closest('#audio-panel')) return 'audio';
    if (el.closest('#hw-panels')) return 'hw';
    if (el.closest('#widgets')) return 'widgets';
    return null;
  }

  // La Conversation est devant : le navigateur envoie le clic à la card dessous. On le renvoie au canevas, qui traite
  // la Conversation (glisser, boutons, lignes). Le « click » qui suit est renvoyé de la même façon.
  let forwarding = false;
  const mouse = (type, e) => stage.dispatchEvent(new MouseEvent(type, { bubbles: true, cancelable: true, clientX: e.clientX, clientY: e.clientY, button: 0, buttons: type === 'mousedown' ? 1 : 0, shiftKey: e.shiftKey, ctrlKey: e.ctrlKey, altKey: e.altKey }));

  document.addEventListener('pointerdown', (e) => {
    if (e.button !== 0 || !(e.target instanceof Element)) return;
    // La Conversation est devant et c'est elle qu'on voit sous le curseur : le clic est pour elle, pas pour la card dessous.
    if (convAbove() && inConversation(e)) {
      forwarding = true;
      e.preventDefault(); // pas de « mousedown » pour la card dessous
      e.stopPropagation();
      mouse('mousedown', e);
      return;
    }
    const group = groupOf(e.target);
    if (group) {
      raise(group);
      // Dans #widgets : la card cliquée passe devant les autres (tant qu'elle n'est pas déjà la dernière).
      if (group === 'widgets') front(e.target.closest('.widget'));
    } else if (e.target === stage && inConversation(e)) {
      raise('conversation'); // clic sur la partie visible de la Conversation : elle passe devant
    }
  }, true);

  document.addEventListener('click', (e) => {
    if (!forwarding) return;
    forwarding = false;
    e.preventDefault();
    e.stopPropagation();
    mouse('click', e);
  }, true);
  // Annuler le « pointerdown » supprime aussi les événements souris qui suivent (mousemove, mouseup) : sans eux, la
  // Conversation resterait « saisie » et suivrait le curseur. On les renvoie à la fenêtre, où app.js les écoute.
  const windowMouse = (type, e) => window.dispatchEvent(new MouseEvent(type, { bubbles: true, cancelable: true, clientX: e.clientX, clientY: e.clientY, button: 0, buttons: type === 'mousemove' ? 1 : 0, shiftKey: e.shiftKey, ctrlKey: e.ctrlKey, altKey: e.altKey }));
  document.addEventListener('pointermove', (e) => { if (forwarding) windowMouse('mousemove', e); }, true);
  document.addEventListener('pointerup', (e) => {
    if (forwarding) windowMouse('mouseup', e);
    // Pas de « click » (glisser) : le renvoi s'arrête, pour ne pas détourner le clic suivant.
    setTimeout(() => { forwarding = false; }, 60);
  }, true);
  document.addEventListener('pointercancel', (e) => {
    if (forwarding) windowMouse('mouseup', e);
    forwarding = false;
  }, true);
})();
