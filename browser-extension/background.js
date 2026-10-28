// Extension « Jarvis — onglets » : relie Chrome à Jarvis (core/browser-bridge.js) sur ce PC.
//
// - Envoie à Jarvis la liste des fenêtres et des onglets (titre, adresse, actif, son, épinglé),
//   à chaque changement. Les fenêtres de navigation privée ne sont jamais envoyées.
// - Exécute les commandes de Jarvis : aller sur un onglet, le fermer, en ouvrir un, le recharger,
//   lire le texte d'une page, cliquer sur un élément, remplir un champ, lire le titre et les
//   participants d'une réunion Google Meet (enregistrement des réunions, orchestrator/meetings.js).
// Connexion : WebSocket ws://127.0.0.1:8796 (rien ne sort du PC). Jarvis n'accepte que cette
// extension (identifiant fixe, vérifié par l'origine de la connexion).

const PORT_DEFAULT = 8796;
let ws = null;
let retryMs = 2000;
let retryTimer = null;
let pushTimer = null;
let status = { connected: false, since: 0, error: '' };

async function port() {
  try {
    const { port: p } = await chrome.storage.local.get('port');
    return Number(p) || PORT_DEFAULT;
  } catch {
    return PORT_DEFAULT;
  }
}

function setStatus(patch) {
  status = { ...status, ...patch };
  chrome.storage.local.set({ status }).catch(() => {});
  chrome.action.setBadgeText({ text: status.connected ? '' : '!' }).catch(() => {});
  chrome.action.setBadgeBackgroundColor({ color: '#c0392b' }).catch(() => {});
}

let connecting = false;
async function connect() {
  clearTimeout(retryTimer);
  // Plusieurs déclencheurs au réveil (installation, démarrage, alarme) : une seule connexion.
  if (connecting || (ws && (ws.readyState === WebSocket.OPEN || ws.readyState === WebSocket.CONNECTING))) return;
  connecting = true;
  let url;
  try { url = `ws://127.0.0.1:${await port()}/`; } finally { connecting = false; }
  let sock;
  try {
    sock = new WebSocket(url);
  } catch (err) {
    setStatus({ connected: false, error: String(err && err.message || err) });
    scheduleRetry();
    return;
  }
  ws = sock;
  sock.onopen = () => {
    retryMs = 2000;
    setStatus({ connected: true, since: Date.now(), error: '' });
    send({ t: 'hello', version: chrome.runtime.getManifest().version, agent: navigator.userAgent });
    pushTabs();
  };
  sock.onmessage = (e) => {
    let msg;
    try { msg = JSON.parse(e.data); } catch { return; }
    if (msg && msg.cmd) handle(msg);
  };
  sock.onclose = () => {
    if (ws === sock) ws = null;
    setStatus({ connected: false, error: status.connected ? 'Jarvis s’est déconnecté' : 'Jarvis ne répond pas (lancé ?)' });
    scheduleRetry();
  };
  sock.onerror = () => {};
}

function scheduleRetry() {
  clearTimeout(retryTimer);
  retryTimer = setTimeout(connect, retryMs);
  retryMs = Math.min(retryMs * 2, 30000);
}

function send(obj) {
  if (ws && ws.readyState === WebSocket.OPEN) ws.send(JSON.stringify(obj));
}

// --- État des onglets -------------------------------------------------------------------------------

async function snapshot() {
  const wins = await chrome.windows.getAll({ populate: true, windowTypes: ['normal'] });
  return wins.filter((w) => !w.incognito).map((w) => ({
    id: w.id, focused: w.focused, state: w.state,
    left: w.left, top: w.top, width: w.width, height: w.height,
    tabs: (w.tabs || []).map((t) => ({
      id: t.id, index: t.index, title: t.title || '', url: t.url || t.pendingUrl || '',
      active: t.active, audible: Boolean(t.audible), muted: Boolean(t.mutedInfo && t.mutedInfo.muted),
      pinned: t.pinned, status: t.status, groupId: t.groupId
    }))
  }));
}

// Regroupé : au plus un envoi toutes les 250 ms.
function pushTabs() {
  clearTimeout(pushTimer);
  pushTimer = setTimeout(async () => {
    try { send({ t: 'tabs', windows: await snapshot() }); } catch {}
  }, 250);
}

for (const ev of [chrome.tabs.onCreated, chrome.tabs.onRemoved, chrome.tabs.onActivated, chrome.tabs.onMoved,
  chrome.tabs.onAttached, chrome.tabs.onDetached, chrome.tabs.onReplaced, chrome.windows.onCreated,
  chrome.windows.onRemoved, chrome.windows.onFocusChanged]) {
  ev.addListener(() => pushTabs());
}
chrome.tabs.onUpdated.addListener((id, change) => {
  if (change.title !== undefined || change.url !== undefined || change.status === 'complete' || change.audible !== undefined || change.pinned !== undefined || change.mutedInfo) pushTabs();
});

// --- Commandes ------------------------------------------------------------------------------------

async function focusTab(tabId) {
  const tab = await chrome.tabs.update(tabId, { active: true });
  await chrome.windows.update(tab.windowId, { focused: true, ...(await isMinimized(tab.windowId) ? { state: 'normal' } : {}) });
  return tab;
}

async function isMinimized(windowId) {
  try { return (await chrome.windows.get(windowId)).state === 'minimized'; } catch { return false; }
}

// Fonctions injectées dans la page (chrome.scripting) : autonomes, sans variable extérieure.
function pageRead(max) {
  const text = (document.body ? document.body.innerText : '').replace(/\n{3,}/g, '\n\n').trim();
  return { title: document.title, url: location.href, text: text.slice(0, max), length: text.length };
}

function pageClick(target) {
  const want = String(target.text || '').toLowerCase().trim();
  let el = null;
  if (target.selector) el = document.querySelector(target.selector);
  if (!el && want) {
    const nodes = [...document.querySelectorAll('a, button, [role="button"], [role="link"], [role="tab"], [role="menuitem"], input[type="submit"], input[type="button"], summary, label')];
    const label = (n) => String(n.innerText || n.value || n.getAttribute('aria-label') || n.title || '').toLowerCase().replace(/\s+/g, ' ').trim();
    const visible = (n) => { const r = n.getBoundingClientRect(); return r.width > 0 && r.height > 0; };
    el = nodes.find((n) => visible(n) && label(n) === want) || nodes.find((n) => visible(n) && label(n).includes(want));
  }
  if (!el) return { ok: false, error: `aucun élément « ${target.text || target.selector} » dans la page` };
  el.scrollIntoView({ block: 'center' });
  el.click();
  return { ok: true, clicked: String(el.innerText || el.value || el.getAttribute('aria-label') || el.tagName).trim().slice(0, 80) };
}

function pageFill(target) {
  const want = String(target.field || '').toLowerCase().trim();
  let el = target.selector ? document.querySelector(target.selector) : null;
  if (!el && want) {
    const fields = [...document.querySelectorAll('input:not([type="hidden"]):not([type="password"]), textarea, [contenteditable="true"]')];
    const names = (n) => {
      const id = n.id && document.querySelector(`label[for="${CSS.escape(n.id)}"]`);
      return [n.getAttribute('aria-label'), n.placeholder, n.name, id && id.innerText, n.closest('label') && n.closest('label').innerText]
        .filter(Boolean).map((s) => String(s).toLowerCase().trim());
    };
    el = fields.find((n) => names(n).some((s) => s === want)) || fields.find((n) => names(n).some((s) => s.includes(want)));
  }
  if (!el) return { ok: false, error: `aucun champ « ${target.field || target.selector} » dans la page` };
  if (el.type === 'password') return { ok: false, error: 'champ de mot de passe : refusé' };
  el.focus();
  if (el.isContentEditable) {
    el.textContent = target.value;
  } else {
    const proto = el.tagName === 'TEXTAREA' ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
    Object.getOwnPropertyDescriptor(proto, 'value').set.call(el, target.value);
  }
  el.dispatchEvent(new Event('input', { bubbles: true }));
  el.dispatchEvent(new Event('change', { bubbles: true }));
  return { ok: true };
}

// Page Google Meet : titre de la réunion et noms des participants (tuiles vidéo, liste
// « Participants » si elle est ouverte). Meet ne documente pas sa page : plusieurs repères, au mieux.
function pageMeet() {
  const clean = (s) => String(s || '').replace(/\s+/g, ' ').trim();
  let title = '';
  const titled = document.querySelector('[data-meeting-title]');
  if (titled) title = clean(titled.getAttribute('data-meeting-title') || titled.textContent);
  if (!title) {
    const m = document.title.match(/^Meet\s*[-–—:]\s*(.+)$/i);
    if (m && !/^[a-z]{3,4}-[a-z]{4}-[a-z]{3,4}$/i.test(m[1].trim())) title = clean(m[1]);
  }
  const names = new Set();
  const add = (raw) => {
    const s = clean(raw).replace(/\s*\((?:vous|you|moi|me)\)\s*$/i, '').replace(/\s*(?:présentation|presentation|presenting).*$/i, '');
    if (s.length >= 2 && s.length <= 60 && !/^(?:vous|you|moi|me)$/i.test(s) && !/[@\n]|https?:/.test(s)) names.add(s);
  };
  for (const tile of document.querySelectorAll('[data-participant-id]')) {
    const own = tile.querySelector('[data-self-name]');
    if (own) { add(own.getAttribute('data-self-name')); continue; }
    const label = tile.querySelector('.notranslate');
    if (label) add(label.textContent);
  }
  for (const item of document.querySelectorAll('[role="list"] [role="listitem"][aria-label]')) add(item.getAttribute('aria-label'));
  return { ok: true, title, participants: [...names].slice(0, 60), inCall: Boolean(document.querySelector('[data-participant-id]')) };
}

async function inPage(tabId, func, arg) {
  const [res] = await chrome.scripting.executeScript({ target: { tabId }, func, args: [arg] });
  return res ? res.result : { ok: false, error: 'page inaccessible' };
}

async function handle(msg) {
  const reply = (obj) => send({ t: 'result', id: msg.id, ...obj });
  try {
    switch (msg.cmd) {
      case 'snapshot': return reply({ ok: true, windows: await snapshot() });
      case 'activate': await focusTab(msg.tabId); return reply({ ok: true });
      case 'close': await chrome.tabs.remove([].concat(msg.tabIds || msg.tabId)); return reply({ ok: true });
      case 'reload': await chrome.tabs.reload(msg.tabId); return reply({ ok: true });
      case 'mute': await chrome.tabs.update(msg.tabId, { muted: Boolean(msg.muted) }); return reply({ ok: true });
      case 'open': {
        const url = String(msg.url || '');
        if (!/^https?:\/\//i.test(url)) return reply({ ok: false, error: 'adresse invalide (http ou https seulement)' });
        const tab = msg.newWindow
          ? (await chrome.windows.create({ url, focused: true })).tabs[0]
          : await chrome.tabs.create({ url, active: true, ...(msg.windowId ? { windowId: msg.windowId } : {}) });
        if (tab && tab.windowId) await chrome.windows.update(tab.windowId, { focused: true });
        return reply({ ok: true, tabId: tab && tab.id });
      }
      case 'read': return reply({ ok: true, ...(await inPage(msg.tabId, pageRead, Number(msg.max) || 20000)) });
      case 'click': return reply(await inPage(msg.tabId, pageClick, { text: msg.text, selector: msg.selector }));
      case 'fill': return reply(await inPage(msg.tabId, pageFill, { field: msg.field, selector: msg.selector, value: String(msg.value || '') }));
      case 'meet': return reply(await inPage(msg.tabId, pageMeet, null));
      default: return reply({ ok: false, error: `commande inconnue : ${msg.cmd}` });
    }
  } catch (err) {
    reply({ ok: false, error: String(err && err.message || err) });
  }
}

// --- Vie du service worker ----------------------------------------------------------------------------
// Chrome endort le service worker après 30 s d'inactivité : un message toutes les 20 s garde la
// connexion vivante ; l'alarme (1 min) le réveille et se reconnecte si Jarvis était fermé.
setInterval(() => send({ t: 'ping' }), 20000);
chrome.alarms.create('jarvis-reconnect', { periodInMinutes: 1 });
chrome.alarms.onAlarm.addListener((a) => { if (a.name === 'jarvis-reconnect') connect(); });
chrome.runtime.onStartup.addListener(connect);
chrome.runtime.onInstalled.addListener(connect);
chrome.storage.onChanged.addListener((changes) => {
  if (changes.port && ws) { try { ws.close(); } catch {} }
});
chrome.runtime.onMessage.addListener((msg, _sender, sendResponse) => {
  if (msg && msg.type === 'status') sendResponse(status);
  if (msg && msg.type === 'reconnect') { retryMs = 2000; connect(); sendResponse({ ok: true }); }
});
connect();
