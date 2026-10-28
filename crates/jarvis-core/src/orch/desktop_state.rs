// État du bureau en direct (orchestrator/desktop-state.js). La veille du bureau
// (`jarvis-core desktop-watch`, jarvis-native::desktop) envoie un instantané à chaque changement :
// écrans, fenêtres, applications ouvertes sans fenêtre. Ce module garde cet état, le rend lisible
// (« Chrome », « écran 2 »), numérote ce qui est affiché dans une card (« la fenêtre 3 »,
// « Chrome 2 ») et transmet les commandes. Les titres de fenêtres ne sont jamais journalisés.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::rc::{Rc, Weak};

use serde_json::{json, Value};
use tokio::sync::oneshot;

use crate::js::{self, Emitter, J};
use crate::proc::LineChild;

pub fn fold(s: &str) -> String {
    let t = js::fold_lower(s);
    crate::re!(r"[^a-z0-9]+").replace_all(&t, " ").trim().to_string()
}

pub fn app_name_of(key: &str) -> Option<&'static str> {
    Some(match key {
        "chrome" => "Chrome",
        "msedge" => "Edge",
        "firefox" => "Firefox",
        "brave" => "Brave",
        "opera" => "Opera",
        "code" => "VS Code",
        "cursor" => "Cursor",
        "windowsterminal" | "wt" => "Terminal",
        "powershell" | "pwsh" => "PowerShell",
        "cmd" => "Invite de commandes",
        "explorer" => "Explorateur",
        "notepad" => "Bloc-notes",
        "mspaint" => "Paint",
        "calc" | "calculatorapp" => "Calculatrice",
        "winword" => "Word",
        "excel" => "Excel",
        "powerpnt" => "PowerPoint",
        "outlook" | "olk" => "Outlook",
        "onenote" => "OneNote",
        "teams" | "ms-teams" => "Teams",
        "spotify" => "Spotify",
        "discord" => "Discord",
        "slack" => "Slack",
        "signal" => "Signal",
        "whatsapp" => "WhatsApp",
        "telegram" => "Telegram",
        "zoom" => "Zoom",
        "steam" | "steamwebhelper" => "Steam",
        "obs64" => "OBS Studio",
        "vlc" => "VLC",
        "photoshop" => "Photoshop",
        "illustrator" => "Illustrator",
        "jarvis" => "JARVIS",
        "wireguard" => "WireGuard",
        "taskmgr" => "Gestionnaire des tâches",
        "systemsettings" => "Paramètres",
        "notion" => "Notion",
        "obsidian" => "Obsidian",
        "claude" => "Claude",
        "chatgpt" => "ChatGPT",
        "docker desktop" => "Docker Desktop",
        "onedrive" => "OneDrive",
        "dropbox" => "Dropbox",
        "freecad" => "FreeCAD",
        "blender" => "Blender",
        _ => return None,
    })
}

pub fn browser_of(key: &str) -> Option<&'static str> {
    Some(match key {
        "chrome" => "Chrome",
        "msedge" => "Edge",
        "brave" => "Brave",
        "firefox" => "Firefox",
        "opera" => "Opera",
        _ => return None,
    })
}

pub fn app_name(process: &str) -> String {
    let key = fold(process);
    app_name_of(&key)
        .or_else(|| app_name_of(&process.to_lowercase()))
        .map(String::from)
        .unwrap_or_else(|| if process.is_empty() { "Application".into() } else { js::capitalize(process) })
}

pub struct DesktopState {
    me: Weak<DesktopState>,
    log: Rc<dyn Fn(String)>,
    child: RefCell<Option<Rc<LineChild>>>,
    pub state: RefCell<Option<J>>,
    pub at: Cell<i64>,
    pending: RefCell<HashMap<u64, oneshot::Sender<J>>>,
    seq: Cell<u64>,
    stopped: Cell<bool>,
    restarts: RefCell<Vec<i64>>,
    numbered: RefCell<Vec<i64>>,
    browser_numbers: RefCell<HashMap<String, Vec<i64>>>,
    pub monitor_names: RefCell<HashMap<i64, String>>,
    pub events: Emitter,
}

impl DesktopState {
    pub fn new(log: Rc<dyn Fn(String)>) -> Rc<Self> {
        Rc::new_cyclic(|me| Self {
            me: me.clone(),
            log,
            child: RefCell::new(None),
            state: RefCell::new(None),
            at: Cell::new(0),
            pending: RefCell::new(HashMap::new()),
            seq: Cell::new(0),
            stopped: Cell::new(true),
            restarts: RefCell::new(vec![]),
            numbered: RefCell::new(vec![]),
            browser_numbers: RefCell::new(HashMap::new()),
            monitor_names: RefCell::new(HashMap::new()),
            events: Emitter::default(),
        })
    }

    pub fn ready(&self) -> bool {
        self.state.borrow().is_some() && self.child.borrow().is_some()
    }

    pub fn on(&self, event: &str, f: impl Fn(&J) + 'static) {
        self.events.on(event, f);
    }

    pub fn start(&self) -> bool {
        self.stopped.set(false);
        self.spawn();
        true
    }

    fn spawn(&self) {
        let exe = std::env::current_exe().unwrap_or_default();
        let me = self.me.clone();
        let me2 = self.me.clone();
        let me3 = self.me.clone();
        let child = LineChild::spawn(
            &exe,
            &["desktop-watch".to_string()],
            &[],
            move |line| {
                if let (Some(s), Ok(msg)) = (me.upgrade(), serde_json::from_str::<J>(&line)) {
                    s.on_message(msg);
                }
            },
            |_| {},
            move |code| {
                if let Some(s) = me2.upgrade() {
                    s.on_exit(code);
                }
            },
        );
        match child {
            Ok(c) => {
                crate::proc::tie_to_core(c.pid);
                *self.child.borrow_mut() = Some(c);
                js::spawn(async move {
                    if let Some(s) = me3.upgrade() {
                        s.request("snapshot", json!({}), 5000).await;
                    }
                });
            }
            Err(e) => (self.log)(format!("veille du bureau : {e}")),
        }
    }

    fn on_exit(&self, code: Option<i32>) {
        self.child.borrow_mut().take();
        for (_, tx) in self.pending.borrow_mut().drain() {
            let _ = tx.send(json!({ "ok": false, "error": "veille du bureau arrêtée" }));
        }
        if self.stopped.get() {
            return;
        }
        let now = js::now_ms();
        let mut r = self.restarts.borrow_mut();
        r.retain(|t| now - t < 60000);
        if r.len() >= 5 {
            (self.log)("veille du bureau : trop d’arrêts, abandon".into());
            return;
        }
        r.push(now);
        (self.log)(format!("veille du bureau arrêtée (code {}) : relance", code.map(|c| c.to_string()).unwrap_or_else(|| "null".into())));
        let me = self.me.clone();
        js::set_timeout(1500, move || {
            if let Some(s) = me.upgrade() {
                if !s.stopped.get() {
                    s.spawn();
                }
            }
        });
    }

    pub fn stop(&self) {
        self.stopped.set(true);
        if let Some(c) = self.child.borrow_mut().take() {
            c.close_stdin();
            js::set_timeout(800, move || c.kill());
        }
    }

    fn on_message(&self, msg: J) {
        match js::gs(&msg, "t").as_str() {
            "state" => self.set_state(msg),
            "result" => {
                if js::truthy(&msg["state"]) {
                    self.set_state(msg["state"].clone());
                }
                let id = msg["id"].as_u64().unwrap_or(0);
                if let Some(tx) = self.pending.borrow_mut().remove(&id) {
                    let _ = tx.send(msg);
                }
            }
            "log" => (self.log)(format!("veille du bureau : {}", js::gs(&msg, "message"))),
            _ => {}
        }
    }

    // Nouvel état : événements « app ouverte / fermée » et « premier plan ».
    fn set_state(&self, s: J) {
        let before = self.state.borrow_mut().replace(s.clone());
        self.at.set(js::now_ms());
        if let Some(before) = before {
            let apps = |st: &J| -> Vec<String> {
                let mut seen = HashSet::new();
                js::arr(&st["windows"]).iter().map(|w| fold(&js::gs(w, "process"))).filter(|p| seen.insert(p.clone())).collect()
            };
            let a = apps(&before);
            let b = apps(&s);
            for p in &b {
                if !a.contains(p) {
                    self.events.emit("app_opened", &json!({ "process": p, "app": app_name(p) }));
                }
            }
            for p in &a {
                if !b.contains(p) {
                    self.events.emit("app_closed", &json!({ "process": p, "app": app_name(p) }));
                }
            }
            if before["fg"] != s["fg"] {
                if let Some(w) = js::arr(&s["windows"]).iter().find(|x| x["hwnd"] == s["fg"]) {
                    self.events.emit("foreground", &self.describe(w));
                }
            }
        }
        self.events.emit("change", &s);
    }

    pub async fn request(&self, cmd: &str, args: J, timeout_ms: u64) -> J {
        let child = self.child.borrow().clone();
        let Some(child) = child else { return json!({ "ok": false, "error": "veille du bureau indisponible" }) };
        let id = self.seq.get() + 1;
        self.seq.set(id);
        let (tx, rx) = oneshot::channel();
        self.pending.borrow_mut().insert(id, tx);
        let mut msg = json!({ "id": id, "cmd": cmd });
        js::assign(&mut msg, args);
        if !child.write_line(&msg.to_string()).await {
            self.pending.borrow_mut().remove(&id);
            return json!({ "ok": false, "error": "écriture impossible" });
        }
        match tokio::time::timeout(std::time::Duration::from_millis(timeout_ms), rx).await {
            Ok(Ok(r)) => r,
            Ok(Err(_)) => json!({ "ok": false, "error": "veille du bureau arrêtée" }),
            Err(_) => {
                self.pending.borrow_mut().remove(&id);
                json!({ "ok": false, "error": "délai dépassé" })
            }
        }
    }

    // --- Lecture ----------------------------------------------------------------------------------

    pub fn monitors(&self) -> Vec<J> {
        let st = self.state.borrow();
        st.as_ref()
            .map(|s| js::arr(&s["monitors"]))
            .unwrap_or_default()
            .into_iter()
            .map(|mut m| {
                let n = m["n"].as_i64().unwrap_or(0);
                m["label"] = json!(self.monitor_label(n));
                m
            })
            .collect()
    }

    pub fn monitor_label(&self, n: i64) -> String {
        if let Some(name) = self.monitor_names.borrow().get(&n) {
            return name.clone();
        }
        let primary = self
            .state
            .borrow()
            .as_ref()
            .and_then(|s| js::arr(&s["monitors"]).into_iter().find(|m| m["n"].as_i64() == Some(n)))
            .map(|m| js::gb(&m, "primary"))
            .unwrap_or(false);
        format!("Écran {n}{}", if primary { " (principal)" } else { "" })
    }

    /// Fenêtre enrichie : nom de l'application, page pour un navigateur.
    pub fn describe(&self, w: &J) -> J {
        let process = js::gs(w, "process");
        let browser = browser_of(&fold(&process));
        let title = js::gs(w, "title");
        let page = browser.map(|_| crate::re!(r"(?i) [-–—] (?:Google Chrome|Microsoft\x{200b}? Edge|Mozilla Firefox|Brave|Opera)(?: \(.*\))?$").replace(&title, "").into_owned());
        let state = if js::gb(w, "fg") {
            "au premier plan"
        } else if js::gb(w, "min") {
            "réduite"
        } else if js::gb(w, "max") {
            "agrandie"
        } else {
            "ouverte"
        };
        json!({
            "hwnd": w["hwnd"], "pid": w["pid"], "process": process, "app": app_name(&process), "title": title,
            "page": page, "browser": browser, "monitor": w["monitor"], "min": w["min"], "max": w["max"], "fg": w["fg"], "z": w["z"],
            "rect": { "x": w["x"], "y": w["y"], "w": w["w"], "h": w["h"] },
            "state": state
        })
    }

    /// Fenêtres visibles (sauf JARVIS), par écran puis de la plus en avant à la plus en arrière.
    pub fn windows_filtered(&self, monitor: Option<i64>, include_jarvis: bool) -> Vec<J> {
        let st = self.state.borrow();
        let mut list: Vec<J> = st
            .as_ref()
            .map(|s| js::arr(&s["windows"]))
            .unwrap_or_default()
            .iter()
            .filter(|w| include_jarvis || !js::gs(w, "process").eq_ignore_ascii_case("jarvis"))
            .map(|w| self.describe(w))
            .filter(|w| monitor.is_none() || w["monitor"].as_i64() == monitor)
            .collect();
        let k = |w: &J| (w["monitor"].as_i64().unwrap_or(0), js::gb(w, "min") as i64, w["z"].as_i64().unwrap_or(0));
        list.sort_by_key(k);
        list
    }

    pub fn windows(&self) -> Vec<J> {
        self.windows_filtered(None, false)
    }

    /// Applications ouvertes sans fenêtre (assistants et services écartés).
    pub fn background(&self) -> Vec<J> {
        let noise = crate::re!(r"(?i)helper|crashpad|crash|update|updater|service|svc$|broker|host$|agent|daemon|elevat|setup|install|runtime|webview|nvidia|nvcontainer|nvdisplay|amd|radeon|igfx|intel|realtek|rtk|audio|dolby|nahimic|asus|armoury|lenovo|dell|hp[a-z]*$|logi|synaptics|elan|touchpad|node$|python|conhost|dllhost|backgroundtask|widgets|searchhost|startmenu|textinput|lockapp|smartscreen|securityhealth|msmpeng|nissrv|jarvis|desktop-watch|active-win|audio-device|screenshot|git|ssh|psmux|tmux|claude-code|pageant");
        let tools = crate::re!(r"(?i)^(?:openconsole|powershell|pwsh|cmd|uvx?|pip|npx?|bun|deno)$|mcp|plugin|launcher|feedprovider|widgetboard|cmdpal|packagemanager|khalmnpr|^lsb$|laclient|setpoint|phoneexperience|crossdevice|shellexperience|yourphone|gamebar|xbox|onedrivelauncher");
        let mut seen = HashSet::new();
        let st = self.state.borrow();
        st.as_ref()
            .map(|s| js::arr(&s["background"]))
            .unwrap_or_default()
            .into_iter()
            .filter(|b| !crate::re!(r"(?i)^(?:microsoft|windows)\b").is_match(&js::gs(b, "process")))
            .map(|b| {
                let p = js::gs(&b, "process");
                let p = if p.contains('.') { p.split('.').next().unwrap_or("").to_string() } else { p };
                (p, b["pid"].clone())
            })
            .filter(|(p, _)| !noise.is_match(p) && !tools.is_match(p))
            .map(|(p, pid)| json!({ "process": p, "app": app_name(&p), "pid": pid }))
            .filter(|b| seen.insert(js::gs(b, "app")))
            .collect()
    }

    pub fn foreground(&self) -> Option<J> {
        let st = self.state.borrow();
        let s = st.as_ref()?;
        js::arr(&s["windows"]).into_iter().find(|x| x["hwnd"] == s["fg"]).map(|w| json!({ "hwnd": w["hwnd"], "process": w["process"], "title": w["title"], "pid": w["pid"] }))
    }

    /// Instances d'un navigateur, numérotées dans l'ordre d'affichage.
    pub fn browser_instances(&self, browser: &str) -> Vec<J> {
        let key = fold(browser);
        self.windows()
            .into_iter()
            .filter(|w| fold(&js::gs(w, "process")) == key)
            .enumerate()
            .map(|(i, mut w)| {
                w["instance"] = json!(i + 1);
                w
            })
            .collect()
    }

    // --- Numéros des cards -------------------------------------------------------------------------

    pub fn remember_numbers(&self, list: &[J]) {
        *self.numbered.borrow_mut() = list.iter().map(|w| w["hwnd"].as_i64().unwrap_or(0)).collect();
    }

    pub fn remember_browser(&self, browser: &str, list: &[J]) {
        self.browser_numbers.borrow_mut().insert(fold(browser), list.iter().map(|w| w["hwnd"].as_i64().unwrap_or(0)).collect());
    }

    pub fn by_hwnd(&self, hwnd: i64) -> Option<J> {
        let st = self.state.borrow();
        let w = st.as_ref().map(|s| js::arr(&s["windows"])).unwrap_or_default().into_iter().find(|x| x["hwnd"].as_i64() == Some(hwnd))?;
        Some(self.describe(&w))
    }

    /// Écran désigné à la voix : « écran 2 », « l'écran principal », « celui de gauche ».
    pub fn resolve_monitor(&self, spoken: &str) -> Option<i64> {
        let t = fold(spoken);
        let t = crate::re!(r"^(?:l |le |mon |sur )").replace(&t, "");
        let t = crate::re!(r"^ecran ").replace(&t, "").to_string();
        let mons = self.state.borrow().as_ref().map(|s| js::arr(&s["monitors"])).unwrap_or_default();
        let n = if crate::re!(r"^\d+$").is_match(&t) {
            t.parse::<i64>().ok()
        } else {
            match t.as_str() {
                "un" | "premier" => Some(1),
                "deux" | "second" | "deuxieme" => Some(2),
                "trois" | "troisieme" => Some(3),
                _ => None,
            }
        };
        if let Some(n) = n {
            if mons.iter().any(|m| m["n"].as_i64() == Some(n)) {
                return Some(n);
            }
        }
        if t.contains("principal") {
            return mons.iter().find(|m| js::gb(m, "primary")).and_then(|m| m["n"].as_i64());
        }
        if t.contains("secondaire") || t.contains("autre") {
            return mons.iter().find(|m| !js::gb(m, "primary")).and_then(|m| m["n"].as_i64());
        }
        let mut by_x = mons.clone();
        by_x.sort_by_key(|m| m["rect"]["x"].as_i64().unwrap_or(0));
        if t.contains("gauche") {
            return by_x.first().and_then(|m| m["n"].as_i64());
        }
        if t.contains("droite") {
            return by_x.last().and_then(|m| m["n"].as_i64());
        }
        self.monitor_names.borrow().iter().find(|(_, name)| fold(name) == t).map(|(k, _)| *k)
    }

    /// Fenêtre désignée : « la fenêtre 3 », « Chrome 2 », « Spotify », ou des mots du titre.
    pub fn find_window(&self, spoken: &str, titles: bool) -> Option<J> {
        let t = fold(spoken);
        let mut t = crate::re!(r"^(?:la |le |l |les |ma |mon )").replace(&t, "").to_string();
        let titled = crate::fre!(r"^fenetre (?!\d)").is_match(&t).unwrap_or(false);
        if titled {
            t = t.trim_start_matches("fenetre ").to_string();
        }
        let all = self.windows();
        if let Some(c) = crate::re!(r"^(?:fenetre|appli|application|numero|n) (\d{1,2})$").captures(&t) {
            let i: usize = c[1].parse().unwrap_or(0);
            let hwnd = self.numbered.borrow().get(i.wrapping_sub(1)).copied();
            return hwnd.and_then(|h| self.by_hwnd(h));
        }
        if let Some(c) = crate::re!(r"^(chrome|edge|brave|firefox|opera)(?: (?:instance |numero |fenetre )?(\d{1,2}))?$").captures(&t) {
            if let Some(num) = c.get(2) {
                let key = if &c[1] == "edge" { "msedge".to_string() } else { c[1].to_string() };
                let i: usize = num.as_str().parse().unwrap_or(0);
                let known = self.browser_numbers.borrow().get(&key).and_then(|v| v.get(i.wrapping_sub(1)).copied());
                if let Some(h) = known {
                    return self.by_hwnd(h);
                }
                return self.browser_instances(&key).into_iter().nth(i.wrapping_sub(1));
            }
        }
        let flat = t.replace(' ', "");
        let mut by_app: Vec<J> = all
            .iter()
            .filter(|w| {
                let a = fold(&js::gs(w, "app"));
                a == t || fold(&js::gs(w, "process")) == t || a.replace(' ', "") == flat
            })
            .cloned()
            .collect();
        if !by_app.is_empty() {
            by_app.sort_by_key(|w| w["z"].as_i64().unwrap_or(0));
            return by_app.into_iter().next();
        }
        if !titles && !titled {
            return None;
        }
        let words: Vec<&str> = t.split(' ').filter(|w| w.chars().count() > 2).collect();
        if words.is_empty() {
            return None;
        }
        let mut by_title: Vec<J> = all.into_iter().filter(|w| words.iter().all(|x| fold(&format!("{} {}", js::gs(w, "app"), js::gs(w, "title"))).contains(x))).collect();
        by_title.sort_by_key(|w| w["z"].as_i64().unwrap_or(0));
        by_title.into_iter().next()
    }

    /// Toutes les fenêtres d'une application nommée ; à défaut, celles dont le titre contient les mots dits.
    pub fn match_windows(&self, spoken: &str) -> Vec<J> {
        let t = fold(spoken);
        let t = crate::re!(r"^(?:la |le |l |les |ma |mon )").replace(&t, "");
        let t = crate::re!(r"^(?:application|appli|logiciel|programme|fenetre) (?:de |d )?").replace(&t, "").to_string();
        if t.is_empty() {
            return vec![];
        }
        let all = self.windows();
        let flat = t.replace(' ', "");
        let by_app: Vec<J> = all
            .iter()
            .filter(|w| {
                let a = fold(&js::gs(w, "app"));
                let p = fold(&js::gs(w, "process"));
                a == t || p == t || a.replace(' ', "") == flat || p.replace(' ', "") == flat
            })
            .cloned()
            .collect();
        if !by_app.is_empty() {
            return by_app;
        }
        let words: Vec<&str> = t.split(' ').filter(|x| x.chars().count() > 1).collect();
        all.into_iter()
            .filter(|w| {
                let both = fold(&format!("{} {}", js::gs(w, "app"), js::gs(w, "title")));
                let title = fold(&js::gs(w, "title"));
                words.iter().all(|x| both.split(' ').any(|p| p == *x) || (title.contains(x) && x.chars().count() > 3))
            })
            .collect()
    }

    /// Application ouverte sans fenêtre, par son nom.
    pub fn match_background(&self, spoken: &str) -> Vec<J> {
        let t = fold(spoken).replace(' ', "");
        let st = self.state.borrow();
        st.as_ref()
            .map(|s| js::arr(&s["background"]))
            .unwrap_or_default()
            .into_iter()
            .filter(|b| {
                let p = js::gs(b, "process");
                fold(&p).replace(' ', "") == t || fold(&app_name(&p)).replace(' ', "") == t
            })
            .collect()
    }

    /// Ferme des fenêtres (WM_CLOSE) et attend leur disparition réelle → (fermées, restantes).
    pub async fn close_windows(&self, list: &[J], wait_ms: i64) -> (Vec<J>, Vec<J>) {
        for w in list {
            self.request("close", json!({ "hwnd": w["hwnd"] }), 5000).await;
        }
        let t0 = js::now_ms();
        let mut remaining: Vec<J> = list.to_vec();
        while js::now_ms() - t0 < wait_ms {
            js::sleep_ms(250).await;
            self.request("snapshot", json!({}), 5000).await;
            remaining = list.iter().filter(|w| self.by_hwnd(w["hwnd"].as_i64().unwrap_or(0)).is_some()).cloned().collect();
            if remaining.is_empty() {
                break;
            }
        }
        let closed = list.iter().filter(|w| !remaining.iter().any(|r| r["hwnd"] == w["hwnd"])).cloned().collect();
        (closed, remaining)
    }

    // --- Outils (IA et routeur local) --------------------------------------------------------------

    /// Lecture : état résumé, sans chemins d'exécutables.
    pub fn tool_state(&self, p: &J) -> J {
        if !self.ready() {
            return json!({ "ok": false, "error": "veille du bureau indisponible" });
        }
        let ecran = js::gs(p, "ecran");
        let monitor = if ecran.is_empty() { None } else { self.resolve_monitor(&ecran) };
        let list = self.windows_filtered(monitor, false);
        json!({
            "ok": true,
            "ecrans": self.monitors().iter().map(|m| json!({
                "numero": m["n"], "nom": m["label"], "principal": m["primary"],
                "resolution": format!("{}×{}", js::s(&m["rect"]["w"]), js::s(&m["rect"]["h"])), "echelle": m["scale"]
            })).collect::<Vec<_>>(),
            "fenetres": list.iter().enumerate().map(|(i, w)| json!({
                "numero": i + 1, "application": w["app"],
                "titre": if js::truthy(&w["page"]) { w["page"].clone() } else { w["title"].clone() },
                "ecran": w["monitor"], "etat": w["state"]
            })).collect::<Vec<_>>(),
            "arriere_plan": self.background().iter().map(|b| b["app"].clone()).collect::<Vec<_>>(),
            "premier_plan": list.iter().find(|w| js::gb(w, "fg")).map(|w| w["app"].clone()).unwrap_or(Value::Null)
        })
    }

    /// Action sur une fenêtre : { action, cible, ecran, hwnd }.
    pub async fn tool_action(&self, p: &J) -> J {
        if !self.ready() {
            return json!({ "ok": false, "error": "veille du bureau indisponible" });
        }
        let action = js::gs(p, "action");
        let cible = js::gs(p, "cible");
        let ecran = js::gs(p, "ecran");
        if action == "tout_reduire" {
            let monitor = if ecran.is_empty() { Some(0) } else { self.resolve_monitor(&ecran) };
            if !ecran.is_empty() && monitor.is_none() {
                return json!({ "ok": false, "error": format!("écran « {ecran} » introuvable") });
            }
            let r = self.request("minimizeAll", json!({ "monitor": monitor.unwrap_or(0) }), 5000).await;
            if !js::gb(&r, "ok") {
                return r;
            }
            let n = r["count"].as_i64().unwrap_or(0);
            let s = if n > 1 { "s" } else { "" };
            return json!({ "ok": true, "message": format!("{n} fenêtre{s} réduite{s}") });
        }
        let hwnd = js::num(&p["hwnd"]).filter(|_| js::truthy(&p["hwnd"])).map(|f| f as i64);
        let w = match hwnd {
            Some(h) => self.by_hwnd(h),
            None => self.find_window(&cible, true),
        };
        let Some(w) = w else { return json!({ "ok": false, "error": format!("aucune fenêtre ne correspond à « {cible} »") }) };
        let label = w["app"].clone();
        let cmd = match action.as_str() {
            "afficher" => Some("activate"),
            "reduire" => Some("minimize"),
            "agrandir" => Some("maximize"),
            "restaurer" => Some("restore"),
            "fermer" => Some("close"),
            _ => None,
        };
        let fail = |r: &J| json!({ "ok": false, "error": if js::truthy(&r["error"]) { r["error"].clone() } else { json!("échec") } });
        if let Some(c) = cmd {
            let r = self.request(c, json!({ "hwnd": w["hwnd"] }), 5000).await;
            return if js::gb(&r, "ok") { json!({ "ok": true, "fenetre": label, "hwnd": w["hwnd"] }) } else { fail(&r) };
        }
        let mons = self.state.borrow().as_ref().map(|s| js::arr(&s["monitors"])).unwrap_or_default();
        match action.as_str() {
            "deplacer" => {
                let mut monitor = if ecran.is_empty() { None } else { self.resolve_monitor(&ecran) };
                if monitor.is_none() && mons.len() >= 2 {
                    monitor = mons.iter().find(|m| m["n"] != w["monitor"]).and_then(|m| m["n"].as_i64());
                }
                let monitor = monitor.unwrap_or(if w["monitor"].as_i64() == Some(1) { 2 } else { 1 });
                let r = self.request("move", json!({ "hwnd": w["hwnd"], "monitor": monitor }), 5000).await;
                if js::gb(&r, "ok") { json!({ "ok": true, "fenetre": label, "ecran": monitor }) } else { fail(&r) }
            }
            "centrer" => {
                let wanted = if ecran.is_empty() { "1".to_string() } else { ecran.clone() };
                let monitor = self.resolve_monitor(&wanted).or_else(|| if ecran.is_empty() { mons.iter().find(|m| js::gb(m, "primary")).and_then(|m| m["n"].as_i64()) } else { None });
                let Some(monitor) = monitor else { return json!({ "ok": false, "error": format!("écran « {wanted} » introuvable") }) };
                let r = self.request("center", json!({ "hwnd": w["hwnd"], "monitor": monitor }), 5000).await;
                if js::gb(&r, "ok") { json!({ "ok": true, "fenetre": label, "ecran": monitor }) } else { fail(&r) }
            }
            "gauche" | "droite" => {
                let r = self.request("snap", json!({ "hwnd": w["hwnd"], "side": if action == "gauche" { "left" } else { "right" } }), 5000).await;
                if js::gb(&r, "ok") { json!({ "ok": true, "fenetre": label }) } else { fail(&r) }
            }
            _ => json!({ "ok": false, "error": format!("action inconnue : {action}") }),
        }
    }
}
