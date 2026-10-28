// Card « Devant » (orchestrator/devant.js) : applications actuellement lancées, une mini-card chacune.
// Un clic amène l'application au premier plan ; une application peut être retirée pour la session ou
// définitivement (data/devant-exclus.json) ; un sous-menu restaure les applications retirées.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use serde_json::json;

use super::desktop_state::fold;
use super::{OrchRef, Up};
use crate::js::{self, J};

/// Icône claire et reconnaissable selon l'application et le processus.
pub fn icon_for_app(app: &str, process: &str) -> &'static str {
    let s = format!("{} {}", fold(app), fold(process));
    let t = |re: &regex::Regex| re.is_match(&s);
    if t(crate::re!(r"chrome|edge|msedge|firefox|brave|opera|safari|browser|navigateur")) {
        "🌐"
    } else if t(crate::re!(r"code|cursor|windsurf|sublime|atom|ide|studio|pycharm|webstorm|intellij|dev")) {
        "💻"
    } else if t(crate::re!(r"terminal|powershell|pwsh|cmd|wt|bash|prompt|console|tmux|psmux")) {
        "📟"
    } else if t(crate::re!(r"spotify|music|deezer|apple music|foobar|winamp|vlc|player|audio")) {
        "🎵"
    } else if t(crate::re!(r"discord|slack|teams|telegram|whatsapp|signal|messenger|skype|chat")) {
        "💬"
    } else if t(crate::re!(r"explorer|explorateur|dossier|folder|fichiers")) {
        "📁"
    } else if t(crate::re!(r"paint|photoshop|gimp|figma|illustrator|blender|draw|canvas|image")) {
        "🎨"
    } else if t(crate::re!(r"notepad|bloc notes|word|excel|powerpoint|obsidian|notion|onenote|writer")) {
        "📝"
    } else if t(crate::re!(r"settings|parametres|taskmgr|gestionnaire|config|reglage|system")) {
        "⚙️"
    } else if t(crate::re!(r"steam|game|jeu|play|riot|epic")) {
        "🎮"
    } else {
        "🗔"
    }
}

fn key_of(s: &str) -> String {
    fold(s)
}

pub struct Devant {
    o: OrchRef,
    file: PathBuf,
    log: Rc<dyn Fn(String)>,
    /// (clé, entrée), ordre d'insertion
    temp: RefCell<Vec<(String, J)>>,
    perm: RefCell<Vec<(String, J)>>,
    pub showing_excluded: Cell<bool>,
}

fn remove(list: &mut Vec<(String, J)>, key: &str) -> Option<J> {
    let i = list.iter().position(|(k, _)| k == key)?;
    Some(list.remove(i).1)
}

fn set(list: &mut Vec<(String, J)>, key: &str, v: J) {
    match list.iter_mut().find(|(k, _)| k == key) {
        Some(x) => x.1 = v,
        None => list.push((key.to_string(), v)),
    }
}

impl Devant {
    pub fn new(o: OrchRef, file: PathBuf, log: Rc<dyn Fn(String)>) -> Self {
        let me = Self { o, file, log, temp: RefCell::new(vec![]), perm: RefCell::new(vec![]), showing_excluded: Cell::new(false) };
        me.load_permanent();
        me
    }

    fn load_permanent(&self) {
        let mut perm = vec![];
        if let Some(parsed) = js::read_json(&self.file) {
            let list = if parsed.is_array() { js::arr(&parsed) } else { js::arr(&parsed["permanent"]) };
            for item in list {
                if !js::truthy(&item["key"]) && !js::truthy(&item["app"]) {
                    continue;
                }
                let key = key_of(&if js::truthy(&item["key"]) { js::gs(&item, "key") } else { js::gs(&item, "app") });
                let app = if js::truthy(&item["app"]) { js::gs(&item, "app") } else { key.clone() };
                let icon = if js::truthy(&item["icon"]) { js::gs(&item, "icon") } else { icon_for_app(&js::gs(&item, "app"), &js::gs(&item, "process")).to_string() };
                let at = if js::truthy(&item["at"]) { item["at"].clone() } else { json!(js::now_ms()) };
                set(&mut perm, &key, json!({ "key": key, "app": app, "icon": icon, "type": "perm", "at": at }));
            }
        }
        *self.perm.borrow_mut() = perm;
    }

    fn save_permanent(&self) {
        let list: Vec<J> = self.perm.borrow().iter().map(|(_, v)| v.clone()).collect();
        if let Err(e) = js::write_json(&self.file, &json!({ "permanent": list })) {
            (self.log)(format!("[devant] Erreur sauvegarde exclusions permanentes : {e}"));
        }
    }

    fn has(list: &RefCell<Vec<(String, J)>>, key: &str) -> bool {
        list.borrow().iter().any(|(k, _)| k == key)
    }

    pub fn is_excluded(&self, key: &str) -> bool {
        let k = key_of(key);
        Self::has(&self.temp, &k) || Self::has(&self.perm, &k)
    }

    pub fn exclusion_type(&self, key: &str) -> Option<&'static str> {
        let k = key_of(key);
        if Self::has(&self.perm, &k) {
            Some("perm")
        } else if Self::has(&self.temp, &k) {
            Some("temp")
        } else {
            None
        }
    }

    pub fn remove_temp(&self, app_or_key: &str, name: &str, icon: &str) -> J {
        let key = key_of(app_or_key);
        let app = [name, app_or_key, &key].iter().find(|s| !s.is_empty()).map(|s| s.to_string()).unwrap_or_default();
        let ic = if icon.is_empty() { icon_for_app(&app, &key).to_string() } else { icon.to_string() };
        let was_perm = remove(&mut self.perm.borrow_mut(), &key).is_some();
        set(&mut self.temp.borrow_mut(), &key, json!({ "key": key, "app": app, "icon": ic, "type": "temp", "at": js::now_ms() }));
        if was_perm {
            self.save_permanent();
        }
        json!({ "ok": true, "key": key, "type": "temp" })
    }

    pub fn remove_perm(&self, app_or_key: &str, name: &str, icon: &str) -> J {
        let key = key_of(app_or_key);
        let app = [name, app_or_key, &key].iter().find(|s| !s.is_empty()).map(|s| s.to_string()).unwrap_or_default();
        let ic = if icon.is_empty() { icon_for_app(&app, &key).to_string() } else { icon.to_string() };
        remove(&mut self.temp.borrow_mut(), &key);
        set(&mut self.perm.borrow_mut(), &key, json!({ "key": key, "app": app, "icon": ic, "type": "perm", "at": js::now_ms() }));
        self.save_permanent();
        json!({ "ok": true, "key": key, "type": "perm" })
    }

    pub fn restore(&self, app_or_key: &str) -> J {
        let key = key_of(app_or_key);
        remove(&mut self.temp.borrow_mut(), &key);
        if remove(&mut self.perm.borrow_mut(), &key).is_some() {
            self.save_permanent();
        }
        json!({ "ok": true, "key": key })
    }

    pub fn toggle_type(&self, app_or_key: &str) -> J {
        let key = key_of(app_or_key);
        let t = self.temp.borrow().iter().find(|(k, _)| *k == key).map(|(_, v)| v.clone());
        if let Some(cur) = t {
            return self.remove_perm(&js::gs(&cur, "key"), &js::gs(&cur, "app"), &js::gs(&cur, "icon"));
        }
        let p = self.perm.borrow().iter().find(|(k, _)| *k == key).map(|(_, v)| v.clone());
        if let Some(cur) = p {
            return self.remove_temp(&js::gs(&cur, "key"), &js::gs(&cur, "app"), &js::gs(&cur, "icon"));
        }
        json!({ "ok": false, "error": "Non trouvée" })
    }

    pub fn restore_all(&self, only_temp: bool) -> J {
        self.temp.borrow_mut().clear();
        if !only_temp {
            self.perm.borrow_mut().clear();
            self.save_permanent();
        }
        json!({ "ok": true })
    }

    fn excluded_list(&self, running: &[J]) -> Vec<J> {
        let keys: std::collections::HashSet<String> = running.iter().map(|w| key_of(&if js::truthy(&w["app"]) { js::gs(w, "app") } else { js::gs(w, "process") })).collect();
        let mut all: Vec<J> = vec![];
        for (list, typ) in [(&self.temp, "temp"), (&self.perm, "perm")] {
            for (k, v) in list.borrow().iter() {
                let mut x = v.clone();
                x["type"] = json!(typ);
                x["running"] = json!(keys.contains(k));
                all.push(x);
            }
        }
        all.sort_by(|a, b| js::gn_or(b, "at", 0.0).partial_cmp(&js::gn_or(a, "at", 0.0)).unwrap_or(std::cmp::Ordering::Equal));
        all
    }

    pub fn card(&self, show_excluded: Option<bool>) -> J {
        if let Some(s) = show_excluded {
            self.showing_excluded.set(s);
        }
        let o = self.o.up();
        let ds = &o.desktop_state;
        let windows = ds.windows();
        let mut active: Vec<J> = vec![];
        for w in &windows {
            let app = js::gs(w, "app");
            let process = js::gs(w, "process");
            let key = key_of(if app.is_empty() { &process } else { &app });
            if self.is_excluded(&key) {
                continue;
            }
            let monitor = w["monitor"].as_i64().unwrap_or(0);
            active.push(json!({
                "hwnd": w["hwnd"], "pid": w["pid"], "process": process, "app": app, "appKey": key, "title": w["title"], "page": w["page"],
                "monitor": monitor, "monitorLabel": ds.monitor_label(monitor), "state": w["state"],
                "fg": js::gb(w, "fg"), "min": js::gb(w, "min"), "max": js::gb(w, "max"), "z": w["z"].as_f64().unwrap_or(999.0),
                "icon": icon_for_app(&app, &process), "call": { "method": "desktopAction", "args": [w["hwnd"], "afficher"] }
            }));
        }
        active.sort_by(|a, b| {
            let (fa, fb) = (js::gb(a, "fg"), js::gb(b, "fg"));
            let (ma, mb) = (js::gb(a, "min"), js::gb(b, "min"));
            fb.cmp(&fa).then(ma.cmp(&mb)).then(js::gn_or(a, "z", 999.0).partial_cmp(&js::gn_or(b, "z", 999.0)).unwrap_or(std::cmp::Ordering::Equal))
        });
        json!({
            "title": "Devant", "tool": "devant", "devant": true, "count": active.len(), "showExcluded": self.showing_excluded.get(),
            "items": active, "excluded": self.excluded_list(&windows), "ttl": 0, "pinned": true,
            "footer": "Clic : 1er plan · Double-clic : centrée, entièrement visible"
        })
    }

    pub fn handle_action(&self, action: &str, app_key: &str) -> J {
        match action {
            "remove_temp" => self.remove_temp(app_key, "", ""),
            "remove_perm" => self.remove_perm(app_key, "", ""),
            "restore" => self.restore(app_key),
            "toggle_type" => self.toggle_type(app_key),
            "toggle_view" => {
                self.showing_excluded.set(!self.showing_excluded.get());
                json!({ "ok": true, "showingExcluded": self.showing_excluded.get() })
            }
            "show_excluded" => {
                self.showing_excluded.set(true);
                json!({ "ok": true, "showingExcluded": true })
            }
            "show_active" => {
                self.showing_excluded.set(false);
                json!({ "ok": true, "showingExcluded": false })
            }
            "restore_all" => self.restore_all(true),
            "restore_all_perm" => self.restore_all(false),
            _ => json!({ "ok": false, "error": "Action inconnue" }),
        }
    }
}
