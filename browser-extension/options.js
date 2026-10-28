// Page d'options / fenêtre de l'icône : état de la connexion à Jarvis, port.
const $ = (id) => document.getElementById(id);

async function refresh() {
  const status = await chrome.runtime.sendMessage({ type: 'status' }).catch(() => null);
  const el = $('state');
  if (status && status.connected) {
    el.className = 'state';
    el.textContent = `Relié à Jarvis depuis ${new Date(status.since).toLocaleTimeString('fr-FR')}.`;
  } else {
    el.className = 'state off';
    el.textContent = `Non relié : ${(status && status.error) || 'Jarvis ne répond pas'}. L’extension réessaie toute seule.`;
  }
}

(async () => {
  const { port } = await chrome.storage.local.get('port');
  $('port').value = port || 8796;
  $('save').onclick = async () => {
    await chrome.storage.local.set({ port: Number($('port').value) || 8796 });
    setTimeout(refresh, 1500);
  };
  $('retry').onclick = async () => {
    await chrome.runtime.sendMessage({ type: 'reconnect' }).catch(() => {});
    setTimeout(refresh, 1500);
  };
  refresh();
  setInterval(refresh, 3000);
})();
