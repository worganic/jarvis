// Section « Maison » de l'administration : lampes Philips Hue (pont local) et prises Shelly ou Kasa, en plus des ampoules
// Yeelight (détectées toutes seules). Réseau local seulement : seules les adresses privées sont acceptées.
(function () {
  const A = window.JarvisAdmin;
  if (!A) return;
  const { h, panel, api, sections, render, toast } = A;

  const state = { data: null, loading: false, hueIp: '', plug: { nom: '', type: 'shelly', ip: '' } };

  async function load() {
    if (state.loading) return;
    state.loading = true;
    const res = await api('homeState');
    state.loading = false;
    if (res && res.ok) state.data = res;
    render(true);
  }

  async function act(label, method, ...args) {
    const res = await api(method, ...args);
    if (res && res.ok) {
      state.data = res.state || res;
      toast(label);
      render(true);
    } else toast((res && (res.error || res.message)) || 'Échec', true);
    return res;
  }

  sections.maison = () => {
    if (!state.data && !state.loading) load();
    const d = state.data;
    if (!d) return [h('h1', {}, 'Maison'), h('div', { class: 'empty' }, 'Chargement…')];
    const hue = d.hue || {};
    const ip = h('input', { type: 'text', placeholder: 'Adresse du pont, ex. 192.168.1.20', value: state.hueIp, style: 'min-width:240px' });
    ip.addEventListener('input', () => { state.hueIp = ip.value; });
    const nom = h('input', { type: 'text', placeholder: 'Nom, ex. cafetière', value: state.plug.nom });
    nom.addEventListener('input', () => { state.plug.nom = nom.value; });
    const type = h('select', {}, h('option', { value: 'shelly' }, 'Shelly (Wi-Fi)'), h('option', { value: 'kasa' }, 'TP-Link Kasa'));
    type.value = state.plug.type;
    type.addEventListener('change', () => { state.plug.type = type.value; });
    const pip = h('input', { type: 'text', placeholder: 'Adresse, ex. 192.168.1.30', value: state.plug.ip });
    pip.addEventListener('input', () => { state.plug.ip = pip.value; });
    return [
      h('h1', {}, 'Maison'),
      h('p', { class: 'lead' }, "Pilote à la voix tes lampes Philips Hue et tes prises connectées : « allume la cafetière », « éteins le salon », « mets le salon à 40 pour cent ». Les ampoules Yeelight sont détectées toutes seules (Contrôle LAN activé). Tout passe par le réseau local ; les adresses d'Internet sont refusées."),
      panel('Pont Philips Hue', hue.associe ? `associé · ${(hue.lampes || []).length} lampe(s)` : 'non associé',
        hue.associe
          ? h('div', {}, h('div', { class: 'dim' }, `Pont ${hue.ip} · ${(hue.lampes || []).join(', ') || 'aucune lampe'}`),
            h('div', { class: 'row' }, h('button', { class: 'btn ghost', onclick: () => act('Lampes relues', 'homeHueRefresh') }, '⟳ Relire les lampes'), h('button', { class: 'btn danger', onclick: () => act('Pont oublié', 'homeHueForget') }, 'Oublier le pont')))
          : h('div', {}, h('p', { class: 'dim' }, "1. Appuie sur le bouton rond du pont Hue. 2. Saisis son adresse (visible dans l'application Hue ou sur ton routeur) et clique sur Associer dans les 30 secondes."),
            h('div', { class: 'row' }, ip, h('button', { class: 'btn primary', onclick: () => act('Pont associé', 'homeHuePair', state.hueIp.trim()) }, 'Associer')))),
      panel('Prises connectées', `${(d.prises || []).length}`,
        h('div', { class: 'row' }, nom, type, pip, h('button', { class: 'btn primary', onclick: () => { const p = state.plug; act('Prise ajoutée', 'homePlugAdd', p.nom.trim(), p.type, p.ip.trim()).then((r) => { if (r && r.ok) state.plug = { nom: '', type: 'shelly', ip: '' }; }); } }, 'Ajouter')),
        (d.prises || []).length
          ? h('table', {}, h('thead', {}, h('tr', {}, h('th', {}, 'Nom'), h('th', {}, 'Type'), h('th', {}, 'Adresse'), h('th', {}, ''))),
            h('tbody', {}, ...d.prises.map((p) => h('tr', {}, h('td', {}, p.nom), h('td', {}, p.type), h('td', {}, p.ip),
              h('td', {}, h('button', { class: 'btn ghost', onclick: () => act(`${p.nom} basculée`, 'homeTest', p.nom) }, 'Tester'), ' ', h('button', { class: 'btn danger', onclick: () => act('Prise retirée', 'homePlugRemove', p.nom) }, 'Retirer'))))))
          : h('div', { class: 'empty' }, 'Aucune prise déclarée.'))
    ];
  };
})();
