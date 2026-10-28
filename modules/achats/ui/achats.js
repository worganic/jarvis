// Module Achats : la fiche d'un produit (image, description, caractéristiques, prix de chaque boutique, actions).
// Le noyau (orch/shopping.rs) envoie l'événement « widget » kind product_view ; tout passe par listAction('produit', …).
(function () {
  const J = (window.JARVIS = window.JARVIS || {});
  const bridge = window.jarvisBridge;
  const K = J.widgetKit;
  if (!K) return;
  const { h, place, gripHandle, numBadge, closeButton } = K;

  // Image, marque, description, caractéristiques, toutes les boutiques avec leur prix (un clic ouvre la
  // page du marchand), et les actions : meilleure offre, bibliothèque de produits, actualiser les prix.
  // Tout passe par listAction('produit', identifiant, action) : jamais d'adresse venue de la fenêtre.
  function renderProductView(ev) {
    const key = 'product_view';
    const run = (id, action) => { if (bridge && bridge.listAction) Promise.resolve(bridge.listAction('produit', id, action)).catch(() => {}); };
    const act = (label, tip, id, action, cls = '') => h('button', {
      class: `w-note-btn${cls ? ` ${cls}` : ''}`, title: tip,
      onclick: (e) => {
        const b = e.currentTarget;
        b.disabled = true;
        setTimeout(() => { b.disabled = false; }, 1500);
        run(id, action);
      }
    }, label);
    const hasImage = typeof ev.image === 'string' && /^https:\/\//i.test(ev.image);
    const imgBox = h('div', { class: `w-prod-img${hasImage ? '' : ' w-prod-noimg'}` }, hasImage ? null : '🛒');
    if (hasImage) {
      const img = h('img', { src: ev.image, alt: '', referrerpolicy: 'no-referrer', draggable: 'false' });
      img.addEventListener('error', () => { imgBox.classList.add('w-prod-noimg'); imgBox.textContent = '🛒'; });
      imgBox.append(img);
    }
    const offers = Array.isArray(ev.offers) ? ev.offers : [];
    const el = h('div', { class: 'widget w-tool w-card-list w-wide w-product' },
      h('div', { class: 'w-head' },
        gripHandle(),
        numBadge(ev),
        h('span', { class: 'w-icon' }, '🛒'),
        h('span', { class: 'w-title w-mail-subject', title: ev.title || '' }, ev.title || 'Produit'),
        h('span', { class: 'w-source' }, 'PRODUIT'),
        closeButton(ev, key)),
      h('div', { class: 'w-prod-top' },
        imgBox,
        h('div', { class: 'w-prod-info' },
          ev.brand || ev.reference ? h('div', { class: 'w-prod-ref' }, [ev.brand, ev.reference].filter(Boolean).join(' · ')) : null,
          ev.best ? h('div', { class: 'w-prod-best' }, h('b', {}, ev.best.price), ` chez ${ev.best.store}`) : h('div', { class: 'w-prod-ref' }, 'prix non confirmé'),
          ev.description ? h('div', { class: 'w-prod-desc' }, ev.description) : null,
          ev.updatedAt ? h('div', { class: 'w-prod-ref' }, `dans ta bibliothèque · relevé le ${ev.updatedAt}`) : null)),
      Array.isArray(ev.specs) && ev.specs.length
        ? h('div', { class: 'w-prod-specs' }, ...ev.specs.map((s) => h('div', { class: 'w-prod-spec' }, h('span', {}, s.label), h('b', {}, s.value))))
        : null,
      Array.isArray(ev.history) && ev.history.length
        ? h('div', { class: 'w-prod-hist', title: 'Meilleurs prix relevés' }, 'Historique : ', ev.history.map((p) => `${p.at} ${p.price}`).join(' → '))
        : null,
      offers.length ? h('div', { class: 'w-rows w-prod-offers' }, ...offers.map((o) => h('button', {
        class: `w-row clickable${o.best ? ' tone-ok' : ''}`, title: 'Clic : ouvrir la page du marchand',
        onclick: () => run(o.id, 'ouvrir')
      },
      h('span', { class: 'w-row-ico' }, '↗'),
      h('span', { class: 'w-row-text' },
        h('span', { class: 'w-row-title' }, o.n ? `${o.n}. ${o.store}` : o.store), // « ouvre le produit 2 »
        h('span', { class: 'w-row-sub' }, [o.shipping, o.inStock === false ? 'indisponible' : '', o.note].filter(Boolean).join(' · '))),
      h('span', { class: 'w-row-meta' }, o.price || 'prix ?')))) : null,
      ev.footer ? h('div', { class: 'w-rows-foot' }, ev.footer) : null,
      h('div', { class: 'w-note-actions' },
        offers.length ? act('↗ Meilleure offre', 'Ouvrir la page de la meilleure offre', ev.refId, 'ouvrir', 'primary') : null,
        ev.keptId
          ? act('★ Dans ma bibliothèque', 'Retirer ce produit de la bibliothèque', ev.keptId, 'supprimer', 'on')
          : act('☆ Garder', 'Garder ce produit dans ma bibliothèque de produits', ev.refId, 'garder'),
        h('span', { class: 'w-note-spacer' }),
        act('↻ Actualiser les prix', 'Relancer la recherche des prix (l’IA relit les boutiques)', ev.keptId || ev.refId, 'actualiser')));
    place(key, el, 'product_view', 0);
  }

  K.register('product_view', renderProductView);
  K.vanish('product_view');
})();
