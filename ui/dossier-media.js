// Écran médias d'un dossier Data (profils musique, film… : orch/savoir/explore.rs › open_media), en grand par-dessus
// le HUD comme l'explorateur 3D. Événements : « savoir_media » ouvre, « savoir_media_close » ferme.
// À gauche, de quoi écouter (▶ Spotify : lecture de Jarvis, op « ecouter ») et les vidéos ; au centre, le lecteur
// YouTube intégré (ou la couverture du dossier) ; à droite, points forts et faibles, prix, avis, puis les liens du
// profil et les sources. Seule la fenêtre meneuse l'affiche ; Échap ou ✕ le ferme (op « explorer_ferme »).
(function () {
  const J = (window.JARVIS = window.JARVIS || {});
  const bridge = window.jarvisBridge;
  if (!bridge || !bridge.onTaskEvent) return;
  const call = (op, args) => Promise.resolve(bridge.call('savoir', op, args)).catch(() => null);
  const isLeader = () => (typeof bridge.isLeader === 'function' ? bridge.isLeader() : true);
  const say = (text) => { if (bridge.call) bridge.call('utterance', text).catch(() => {}); };
  const host = (u) => String(u || '').replace(/^https?:\/\/(?:www\.)?/, '').split('/')[0];
  const price = (n) => (Math.round((Number(n) || 0) * 100) / 100).toLocaleString('fr-FR');

  function h(tag, attrs = {}, ...kids) {
    const el = document.createElement(tag);
    for (const [k, v] of Object.entries(attrs || {})) {
      if (v == null || v === false) continue;
      if (k.startsWith('on')) el.addEventListener(k.slice(2), v);
      else if (k === 'class') el.className = v;
      else el.setAttribute(k, v === true ? '' : v);
    }
    for (const c of kids.flat()) if (c != null && c !== false) el.append(c instanceof Node ? c : document.createTextNode(String(c)));
    return el;
  }

  let view = null;

  function toast(text) {
    if (!view) return;
    view.toast.textContent = text;
    view.toast.classList.add('on');
    clearTimeout(view.toastTimer);
    view.toastTimer = setTimeout(() => view && view.toast.classList.remove('on'), 2600);
  }

  function open_(url) {
    say(`Ouvre ${url}`);
    toast(`Ouverture de ${host(url)}…`);
  }

  async function spotify(query) {
    toast(`Spotify : ${query}…`);
    const r = await call('ecouter', { requete: query });
    if (r && r.say) toast(r.say);
  }

  /** Lecteur : une vidéo YouTube (sans cookie), ou la couverture du dossier. */
  function play(item) {
    if (!view) return;
    const stage = view.stage;
    stage.replaceChildren();
    view.playing = item ? item.yt : null;
    if (item && item.yt) {
      stage.append(
        h('iframe', { class: 'dm-player', src: `https://www.youtube-nocookie.com/embed/${item.yt}?autoplay=1&rel=0&modestbranding=1`, allow: 'autoplay; encrypted-media; picture-in-picture; fullscreen', allowfullscreen: true, referrerpolicy: 'strict-origin-when-cross-origin', title: item.titre || 'Vidéo' }),
        h('div', { class: 'dm-under' },
          h('b', {}, item.titre || ''),
          h('button', { class: 'v3-btn', title: 'Si la vidéo ne se lance pas ici', onclick: () => open_(`https://www.youtube.com/watch?v=${item.yt}`) }, '↗ Ouvrir sur YouTube')));
    } else {
      const d = view.data;
      const cover = h('div', { class: 'dm-cover' });
      if (d.image) call('image', { slug: d.slug, fichier: d.image }).then((r) => { if (r && r.src) cover.append(h('img', { src: r.src, alt: '', draggable: 'false' })); });
      stage.append(cover, h('div', { class: 'dm-under' }, h('div', { class: 'v3-desc' }, d.resume || '')));
    }
    for (const b of view.root.querySelectorAll('.dm-item')) b.classList.toggle('on', Boolean(view.playing) && b.dataset.yt === view.playing);
  }

  function listenRow(e) {
    const sub = [e.artiste, e.type, e.annee].filter(Boolean).join(' · ');
    const query = [e.titre, e.artiste].filter(Boolean).join(' ');
    return h('div', { class: 'dm-item', 'data-yt': e.yt || '' },
      h('div', { class: 'dm-txt' }, h('span', { class: 't' }, `♪ ${e.titre}`), sub ? h('span', { class: 's' }, sub) : null),
      h('div', { class: 'dm-acts' },
        h('button', { class: 'v3-btn primary', title: `Écouter « ${query} » sur Spotify`, onclick: () => spotify(query) }, '▶ Spotify'),
        e.yt ? h('button', { class: 'v3-btn', title: 'Regarder ici (YouTube)', onclick: () => play({ yt: e.yt, titre: e.titre }) }, '▶ YouTube')
          : e.youtube ? h('button', { class: 'v3-btn', onclick: () => open_(e.youtube) }, '↗ YouTube') : null));
  }

  function videoRow(v) {
    return h('button', { class: 'dm-item dm-video', 'data-yt': v.yt, title: v.url, onclick: () => play(v) },
      h('img', { src: `https://i.ytimg.com/vi/${v.yt}/mqdefault.jpg`, alt: '', draggable: 'false' }),
      h('div', { class: 'dm-txt' }, h('span', { class: 't' }, v.titre || 'Vidéo'), v.duree ? h('span', { class: 's' }, v.duree) : null));
  }

  function infoPanel(d) {
    const out = [];
    const forts = d.points_forts || [];
    const faibles = d.points_faibles || [];
    if (forts.length || faibles.length) {
      out.push(h('div', { class: 'v3-kicker' }, 'POINTS FORTS ET FAIBLES'),
        h('ul', { class: 'dm-list ok' }, ...forts.slice(0, 6).map((x) => h('li', {}, x))),
        h('ul', { class: 'dm-list ko' }, ...faibles.slice(0, 6).map((x) => h('li', {}, x))));
    }
    const prix = (d.prix || []).filter((p) => typeof p.prix === 'number').sort((a, b) => a.prix - b.prix);
    if (prix.length) {
      out.push(h('div', { class: 'v3-kicker' }, 'PRIX'),
        h('div', { class: 'v3-specs' }, ...prix.slice(0, 6).map((p, i) => h('div', { class: `v3-spec${i === 0 ? ' best' : ''}` },
          p.url ? h('span', { class: 'dm-link', onclick: () => open_(p.url) }, `${p.vendeur} ↗`) : h('span', {}, p.vendeur),
          h('b', {}, `${price(p.prix)} ${p.devise || '€'}`)))));
    }
    const avis = d.avis || [];
    if (avis.length) {
      out.push(h('div', { class: 'v3-kicker' }, 'AVIS'),
        ...avis.slice(0, 4).map((a) => h('div', { class: 'dm-avis' },
          h('b', {}, `${a.source}${a.note ? ` · ${a.note}${a.sur ? `/${a.sur}` : ''}` : ''}`),
          a.resume ? h('span', {}, a.resume) : null)));
    }
    if (!out.length) out.push(h('div', { class: 'v3-kicker' }, 'EN BREF'), h('div', { class: 'v3-desc' }, d.resume || ''));
    return out;
  }

  function linksPanel(d) {
    const out = [];
    const boutons = d.boutons || [];
    if (boutons.length) {
      out.push(h('div', { class: 'dm-chips' }, ...boutons.map((b) => h('button', {
        class: 'v3-btn', title: b.url || `Spotify : ${b.requete}`,
        onclick: () => (b.type === 'spotify' ? spotify(b.requete) : open_(b.url))
      }, `${b.icone || '↗'} ${b.libelle}`))));
    }
    const groups = new Map();
    for (const l of d.liens || []) {
      if (!groups.has(l.groupe)) groups.set(l.groupe, []);
      groups.get(l.groupe).push(l);
    }
    for (const [g, items] of groups) {
      out.push(h('div', { class: 'v3-group' }, `${g} (${items.length})`));
      for (const l of items.slice(0, 20)) out.push(h('button', { class: 'v3-link', title: `Ouvrir ${l.url}`, onclick: () => open_(l.url) }, h('span', { class: 't' }, l.titre), h('span', { class: 's' }, host(l.url))));
    }
    if (!out.length) out.push(h('div', { class: 'v3-empty' }, 'Aucun lien dans ce dossier.'));
    return out;
  }

  function onKey(e) {
    if (view && e.key === 'Escape') {
      e.preventDefault();
      e.stopImmediatePropagation();
      close(true);
    }
  }

  // Un clic dans le lecteur YouTube y envoie le clavier : il revient au HUD pour qu'Échap ferme l'écran.
  function onBlur() {
    setTimeout(() => {
      const el = document.activeElement;
      if (view && el && el.tagName === 'IFRAME' && view.root.contains(el)) { el.blur(); window.focus(); }
    }, 0);
  }

  function open(d) {
    if (view) close(false);
    const ecoute = d.ecoute || [];
    const videos = d.videos || [];
    const stage = h('div', { class: 'dm-stage' });
    const toastEl = h('div', { class: 'v3-toast' });
    const profil = d.profil || {};
    const root = h('div', { id: 'dossier-media', class: 'v3-opening' },
      h('div', { class: 'v3-top' },
        h('div', { class: 'v3-title' }, h('span', { class: 'v3-ico' }, profil.icone || '♪'), h('b', {}, d.sujet || ''), d.categorie ? h('span', { class: 'v3-cat' }, d.categorie) : null),
        profil.nom ? h('span', { class: 'v3-credit' }, profil.nom) : null,
        h('button', { class: 'v3-close', title: 'Fermer (Échap)', onclick: () => close(true) }, '✕')),
      h('div', { class: 'v3-left' },
        ecoute.length ? h('div', { class: 'v3-head' }, `À écouter (${ecoute.length})`) : null,
        h('div', { class: 'dm-col' }, ...ecoute.map(listenRow)),
        videos.length ? h('div', { class: 'v3-head', style: 'margin-top:12px' }, `Vidéos (${videos.length})`) : null,
        h('div', { class: 'dm-col' }, ...videos.map(videoRow))),
      stage,
      h('div', { class: 'v3-right' },
        h('div', { class: 'v3-panel' }, ...infoPanel(d)),
        h('div', { class: 'v3-panel v3-links-panel' }, h('div', { class: 'v3-head' }, 'Pour aller plus loin'), ...linksPanel(d))),
      toastEl,
      h('div', { class: 'v3-hint' }, 'Clic : écouter ou regarder · « écoute … » à la voix · Échap : fermer'));
    document.body.append(root);
    document.body.classList.add('viewer3d-open');
    view = { data: d, root, stage, toast: toastEl, playing: null };
    play(videos[0] || null);
    window.addEventListener('keydown', onKey, true);
    window.addEventListener('blur', onBlur);
    requestAnimationFrame(() => root.classList.remove('v3-opening'));
  }

  function close(notify) {
    if (!view) return;
    const v = view;
    view = null;
    window.removeEventListener('keydown', onKey, true);
    window.removeEventListener('blur', onBlur);
    document.body.classList.remove('viewer3d-open');
    v.root.classList.add('v3-closing');
    v.stage.replaceChildren(); // coupe la vidéo tout de suite
    setTimeout(() => v.root.remove(), 380);
    if (notify) call('explorer_ferme', { slug: v.data.slug });
  }

  bridge.onTaskEvent((ev) => {
    if (!ev || !ev.type) return;
    if (ev.type === 'savoir_media') { if (isLeader()) open(ev); }
    else if (ev.type === 'savoir_media_close') close(false);
  });

  J.dossierMedia = { isOpen: () => Boolean(view), close: () => close(true) };
})();
