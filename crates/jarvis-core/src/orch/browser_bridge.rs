// Pont avec l'extension Chrome « Jarvis — onglets » (orchestrator/browser-bridge.js).
// Serveur WebSocket 127.0.0.1:8796, réservé à l'extension : l'origine doit être
// chrome-extension://<identifiant fixe> (clé du manifeste) et la connexion locale. L'extension envoie
// fenêtres et onglets à chaque changement ; Jarvis numérote les instances (« Chrome 2 ») et les pages
// (« 2.3 ») et transmet les commandes (aller sur un onglet, le fermer, en ouvrir un, lire, cliquer…).

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::sync::{mpsc, oneshot};
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
use tokio_tungstenite::tungstenite::Message;

use super::desktop_state::DesktopState;
use crate::js::{self, Emitter, J};

pub const EXTENSION_ID: &str = "ndeppeiooobikljcjapcdiobichcgaep";

fn fold(s: &str) -> String {
    let t = js::fold_lower(s);
    crate::re!(r"[^a-z0-9]+").replace_all(&t, " ").trim().to_string()
}

pub fn host(url: &str) -> String {
    reqwest::Url::parse(url).ok().and_then(|u| u.host_str().map(|h| h.trim_start_matches("www.").to_string())).unwrap_or_default()
}

pub struct BrowserBridge {
    pub port: u16,
    extension_id: String,
    log: Rc<dyn Fn(String)>,
    client: RefCell<Option<(u64, mpsc::UnboundedSender<Message>)>>,
    client_seq: Cell<u64>,
    pub windows: RefCell<Vec<J>>,
    pub at: Cell<i64>,
    pending: RefCell<HashMap<u64, oneshot::Sender<J>>>,
    seq: Cell<u64>,
    page_numbers: RefCell<HashMap<String, i64>>,
    instance_numbers: RefCell<HashMap<i64, i64>>,
    pub events: Emitter,
    server: RefCell<Option<tokio::task::JoinHandle<()>>>,
}

impl BrowserBridge {
    pub fn new(log: Rc<dyn Fn(String)>) -> Rc<Self> {
        let port = js::env("JARVIS_BROWSER_PORT").and_then(|p| p.parse().ok()).unwrap_or(8796);
        Rc::new(Self {
            port,
            extension_id: EXTENSION_ID.into(),
            log,
            client: RefCell::new(None),
            client_seq: Cell::new(0),
            windows: RefCell::new(vec![]),
            at: Cell::new(0),
            pending: RefCell::new(HashMap::new()),
            seq: Cell::new(0),
            page_numbers: RefCell::new(HashMap::new()),
            instance_numbers: RefCell::new(HashMap::new()),
            events: Emitter::default(),
            server: RefCell::new(None),
        })
    }

    pub fn on(&self, event: &str, f: impl Fn(&J) + 'static) {
        self.events.on(event, f);
    }

    pub fn start(self: &Rc<Self>) {
        let me = Rc::downgrade(self);
        let port = self.port;
        let log = self.log.clone();
        let origin_ok = format!("chrome-extension://{}", self.extension_id);
        let task = js::spawn(async move {
            let listener = match tokio::net::TcpListener::bind(("127.0.0.1", port)).await {
                Ok(l) => l,
                Err(e) => {
                    log(format!("extension Chrome : port {port} indisponible ({e})"));
                    return;
                }
            };
            loop {
                let Ok((stream, addr)) = listener.accept().await else { continue };
                let Some(bridge) = me.upgrade() else { break };
                let origin_ok = origin_ok.clone();
                let log = log.clone();
                js::spawn(async move {
                    let local = addr.ip().is_loopback();
                    let cb = |req: &Request, resp: Response| -> Result<Response, ErrorResponse> {
                        let origin = req.headers().get("origin").and_then(|v| v.to_str().ok()).unwrap_or("").to_string();
                        if origin == origin_ok && local {
                            Ok(resp)
                        } else {
                            log(format!("extension Chrome : connexion refusée (origine {})", if origin.is_empty() { "aucune" } else { &origin }));
                            let mut r = ErrorResponse::new(None);
                            *r.status_mut() = tokio_tungstenite::tungstenite::http::StatusCode::FORBIDDEN;
                            Err(r)
                        }
                    };
                    let cfg = tokio_tungstenite::tungstenite::protocol::WebSocketConfig::default().max_message_size(Some(8 * 1024 * 1024));
                    let Ok(ws) = tokio_tungstenite::accept_hdr_async_with_config(stream, cb, Some(cfg)).await else { return };
                    bridge.serve(ws).await;
                });
            }
        });
        *self.server.borrow_mut() = Some(task);
    }

    async fn serve(self: Rc<Self>, ws: tokio_tungstenite::WebSocketStream<tokio::net::TcpStream>) {
        let (mut sink, mut stream) = ws.split();
        let (tx, mut rx) = mpsc::unbounded_channel::<Message>();
        // Une nouvelle extension remplace l'ancienne.
        if let Some((_, old)) = self.client.borrow_mut().take() {
            let _ = old.send(Message::Close(None));
        }
        let id = self.client_seq.get() + 1;
        self.client_seq.set(id);
        *self.client.borrow_mut() = Some((id, tx));
        (self.log)("extension Chrome reliée".into());
        let writer = js::spawn(async move {
            while let Some(m) = rx.recv().await {
                let close = matches!(m, Message::Close(_));
                if sink.send(m).await.is_err() || close {
                    break;
                }
            }
        });
        while let Some(Ok(msg)) = stream.next().await {
            let text = match msg {
                Message::Text(t) => t.to_string(),
                Message::Binary(b) => String::from_utf8_lossy(&b).to_string(),
                Message::Close(_) => break,
                _ => continue,
            };
            if let Ok(v) = serde_json::from_str::<J>(&text) {
                self.on_message(v);
            }
        }
        writer.abort();
        let current = self.client.borrow().as_ref().map(|(i, _)| *i == id).unwrap_or(false);
        if !current {
            return;
        }
        *self.client.borrow_mut() = None;
        self.windows.borrow_mut().clear();
        let pending: Vec<oneshot::Sender<J>> = self.pending.borrow_mut().drain().map(|(_, s)| s).collect();
        for p in pending {
            let _ = p.send(json!({ "ok": false, "error": "extension déconnectée" }));
        }
        self.events.emit("change", &Value::Null);
    }

    pub fn stop(&self) {
        if let Some((_, c)) = self.client.borrow_mut().take() {
            let _ = c.send(Message::Close(None));
        }
        if let Some(t) = self.server.borrow_mut().take() {
            t.abort();
        }
    }

    pub fn connected(&self) -> bool {
        self.client.borrow().as_ref().map(|(_, tx)| !tx.is_closed()).unwrap_or(false)
    }

    fn on_message(&self, msg: J) {
        let t = js::gs(&msg, "t");
        if t == "tabs" && msg["windows"].is_array() {
            *self.windows.borrow_mut() = js::arr(&msg["windows"]);
            self.at.set(js::now_ms());
            self.events.emit("change", &Value::Null);
        } else if t == "result" {
            if msg["windows"].is_array() {
                *self.windows.borrow_mut() = js::arr(&msg["windows"]);
                self.at.set(js::now_ms());
            }
            if let Some(id) = msg["id"].as_u64() {
                if let Some(p) = self.pending.borrow_mut().remove(&id) {
                    let _ = p.send(msg);
                }
            }
        }
    }

    pub async fn request(&self, cmd: &str, args: J, timeout_ms: u64) -> J {
        if !self.connected() {
            return json!({ "ok": false, "error": "extension Chrome non reliée" });
        }
        let id = self.seq.get() + 1;
        self.seq.set(id);
        let (tx, rx) = oneshot::channel();
        self.pending.borrow_mut().insert(id, tx);
        let mut msg = json!({ "id": id, "cmd": cmd });
        js::assign(&mut msg, args);
        let sent = self.client.borrow().as_ref().map(|(_, c)| c.send(Message::text(msg.to_string())).is_ok()).unwrap_or(false);
        if !sent {
            self.pending.borrow_mut().remove(&id);
            return json!({ "ok": false, "error": "envoi impossible" });
        }
        match tokio::time::timeout(std::time::Duration::from_millis(timeout_ms), rx).await {
            Ok(Ok(v)) => v,
            Ok(Err(_)) => json!({ "ok": false, "error": "extension déconnectée" }),
            Err(_) => {
                self.pending.borrow_mut().remove(&id);
                json!({ "ok": false, "error": "délai dépassé" })
            }
        }
    }

    // --- Instances et pages numérotées --------------------------------------------------------------

    /// Fenêtres Chrome dans l'ordre des instances de la veille du bureau, puis les autres :
    /// [{ chrome, desktop, instance }].
    pub fn numbered(&self, ds: Option<&DesktopState>) -> Vec<J> {
        let wins = self.windows.borrow().clone();
        let mut out: Vec<(J, J)> = vec![];
        let mut used = HashSet::new();
        let instances = ds.filter(|d| d.ready()).map(|d| d.browser_instances("chrome")).unwrap_or_default();
        let active_titles = |w: &J| js::arr(&w["tabs"]).into_iter().filter(|t| js::gb(t, "active")).map(|t| fold(&js::gs(&t, "title"))).collect::<Vec<_>>();
        for inst in instances {
            let page = fold(&js::gs(&inst, "page"));
            let found = wins
                .iter()
                .find(|x| !used.contains(&js::s(&x["id"])) && active_titles(x).iter().any(|t| *t == page))
                .or_else(|| wins.iter().find(|x| !used.contains(&js::s(&x["id"])) && active_titles(x).iter().any(|t| !page.is_empty() && (t.starts_with(&page) || page.starts_with(t.as_str())))));
            if let Some(w) = found {
                used.insert(js::s(&w["id"]));
                out.push((w.clone(), inst));
            }
        }
        for w in &wins {
            if !used.contains(&js::s(&w["id"])) {
                out.push((w.clone(), Value::Null));
            }
        }
        out.into_iter().enumerate().map(|(i, (c, d))| json!({ "chrome": c, "desktop": d, "instance": i + 1 })).collect()
    }

    /// Numéros de la card affichée (« va sur la page 2.3 » : l'onglet que l'on voyait).
    pub fn remember(&self, list: &[J]) {
        let mut pages = self.page_numbers.borrow_mut();
        let mut inst = self.instance_numbers.borrow_mut();
        pages.clear();
        inst.clear();
        for x in list {
            let n = x["instance"].as_i64().unwrap_or(0);
            inst.insert(n, x["chrome"]["id"].as_i64().unwrap_or(0));
            for (j, t) in js::arr(&x["chrome"]["tabs"]).iter().enumerate() {
                pages.insert(format!("{n}.{}", j + 1), t["id"].as_i64().unwrap_or(0));
            }
        }
    }

    pub fn all_tabs(&self) -> Vec<J> {
        self.windows
            .borrow()
            .iter()
            .flat_map(|w| {
                js::arr(&w["tabs"]).into_iter().map(move |mut t| {
                    t["windowId"] = w["id"].clone();
                    t
                })
            })
            .collect()
    }

    pub fn tab_by_id(&self, id: i64) -> Option<J> {
        self.all_tabs().into_iter().find(|t| t["id"].as_i64() == Some(id))
    }

    /// Onglet désigné : « 2.3 », ou des mots du titre / de l'adresse (« gmail », « youtube »).
    pub fn find_tabs(&self, spoken: &str, ds: Option<&DesktopState>) -> Vec<J> {
        let t = spoken.trim();
        if let Some(r) = crate::re!(r"^(\d{1,2})[ .,](\d{1,3})$").captures(t) {
            let key = format!("{}.{}", &r[1], &r[2]);
            if let Some(id) = self.page_numbers.borrow().get(&key).copied() {
                if let Some(tab) = self.tab_by_id(id) {
                    return vec![tab];
                }
            }
            let (a, b): (usize, usize) = (r[1].parse().unwrap_or(0), r[2].parse().unwrap_or(0));
            let list = self.numbered(ds);
            let Some(x) = a.checked_sub(1).and_then(|i| list.get(i)) else { return vec![] };
            let tabs = js::arr(&x["chrome"]["tabs"]);
            return match b.checked_sub(1).and_then(|j| tabs.get(j)) {
                Some(tab) => {
                    let mut tab = tab.clone();
                    tab["windowId"] = x["chrome"]["id"].clone();
                    vec![tab]
                }
                None => vec![],
            };
        }
        let words: Vec<String> = fold(t).split(' ').filter(|w| w.chars().count() > 1).map(String::from).collect();
        if words.is_empty() {
            return vec![];
        }
        self.all_tabs()
            .into_iter()
            .filter(|tab| {
                let hay = format!("{} {}", fold(&js::gs(tab, "title")), fold(&host(&js::gs(tab, "url"))));
                words.iter().all(|w| hay.contains(w.as_str()))
            })
            .collect()
    }

    // --- Outils (IA et routeur local) -----------------------------------------------------------------

    pub fn tool_list(&self, ds: Option<&DesktopState>) -> J {
        if !self.connected() {
            return json!({ "ok": false, "error": "extension Chrome non reliée" });
        }
        let list = self.numbered(ds);
        self.remember(&list);
        let instances: Vec<J> = list
            .iter()
            .map(|x| {
                let n = x["instance"].as_i64().unwrap_or(0);
                let d = &x["desktop"];
                let etat = if d.is_object() { d["state"].clone() } else if js::gs(&x["chrome"], "state") == "minimized" { json!("réduite") } else { json!("ouverte") };
                let onglets: Vec<J> = js::arr(&x["chrome"]["tabs"])
                    .iter()
                    .enumerate()
                    .map(|(j, t)| {
                        let mut o = json!({ "page": format!("{n}.{}", j + 1), "titre": t["title"], "adresse": t["url"], "affiche": t["active"] });
                        if js::gb(t, "audible") {
                            o["son"] = json!(true);
                        }
                        o
                    })
                    .collect();
                json!({ "numero": n, "ecran": if d.is_object() { d["monitor"].clone() } else { Value::Null }, "etat": etat, "onglets": onglets })
            })
            .collect();
        json!({ "ok": true, "instances": instances })
    }

    pub async fn tool_action(&self, p: &J, ds: Option<&DesktopState>) -> J {
        if !self.connected() {
            return json!({ "ok": false, "error": "extension Chrome non reliée" });
        }
        let action = js::gs(p, "action");
        if action == "lister" {
            return self.tool_list(ds);
        }
        if action == "ouvrir" {
            let url = js::gs(p, "url");
            let target = if crate::re!(r"(?i)^https?://").is_match(&url) {
                url
            } else if crate::re!(r"^[\w-]+(\.[\w-]+)+(/\S*)?$").is_match(&url) {
                format!("https://{url}")
            } else {
                format!("https://www.google.com/search?q={}", urlencoding::encode(&url))
            };
            return self.request("open", json!({ "url": target }), 8000).await;
        }
        let page = js::gs(p, "page");
        let tabs = self.find_tabs(&page, ds);
        if tabs.is_empty() {
            return json!({ "ok": false, "error": format!("aucune page ne correspond à « {page} »") });
        }
        if action == "fermer" {
            let ids: Vec<J> = tabs.iter().map(|t| t["id"].clone()).collect();
            let r = self.request("close", json!({ "tabIds": ids }), 8000).await;
            return if js::gb(&r, "ok") { json!({ "ok": true, "fermees": tabs.len(), "titres": tabs.iter().map(|t| t["title"].clone()).collect::<Vec<_>>() }) } else { r };
        }
        let tab = &tabs[0];
        let id = tab["id"].clone();
        match action.as_str() {
            "activer" => {
                let mut r = self.request("activate", json!({ "tabId": id }), 8000).await;
                r["titre"] = tab["title"].clone();
                r
            }
            "recharger" => self.request("reload", json!({ "tabId": id }), 8000).await,
            "lire" => self.request("read", json!({ "tabId": id, "max": 20000 }), 8000).await,
            "cliquer" => self.request("click", json!({ "tabId": id, "text": js::gs(p, "texte") }), 8000).await,
            "remplir" => self.request("fill", json!({ "tabId": id, "field": js::gs(p, "champ"), "value": js::gs(p, "valeur") }), 8000).await,
            other => json!({ "ok": false, "error": format!("action inconnue : {other}") }),
        }
    }
}
