// Clics et saisies dans les applications (orchestrator/ui-control.js).
// Voie fiable d'abord : UI Automation (jarvis-core desktop-watch) trouve le bouton par son nom et l'active.
// En secours seulement : capture de la fenêtre, OCR Windows natif, clic au centre du texte trouvé.
// Rien trouvé : Jarvis le dit, il ne clique jamais au hasard.
// Sécurité : applications interdites (mots de passe, terminaux, registre, UAC, Jarvis) ; applications
// de confiance (data/applis-confiance.json) sans demander ; les autres : accord oral avant chaque action.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use serde_json::{json, Value};

use super::{OrchRef, Up};
use crate::js::{self, J};

pub fn forbidden_name(s: &str) -> bool {
    crate::re!(r"(?i)keepass|bitwarden|1password|lastpass|dashlane|enpass|proton ?pass|powershell|pwsh|^cmd$|windowsterminal|^terminal$|conhost|openconsole|regedit|mmc|taskmgr|gestionnaire des taches|consent|credentialuibroker|logonui|jarvis|desktop-watch").is_match(s)
}

fn norm(s: &str) -> String {
    let t = js::fold_lower(s).replace('&', "");
    crate::re!(r"[^a-z0-9]+").replace_all(&t, " ").trim().to_string()
}

pub struct UiControl {
    o: OrchRef,
    file: Option<PathBuf>,
    log: Rc<dyn Fn(String)>,
    pub trusted_apps: RefCell<Vec<String>>,
    /// Dernière liste : (hwnd, appli, éléments) — « clique sur le bouton 3 »
    pub last_list: RefCell<Option<(i64, String, Vec<J>)>>,
}

impl UiControl {
    pub fn new(o: OrchRef, file: Option<PathBuf>, log: Rc<dyn Fn(String)>) -> Self {
        let trusted = file.as_ref().and_then(|f| js::read_json(f)).map(|v| js::str_list(&v)).unwrap_or_default();
        Self { o, file, log, trusted_apps: RefCell::new(trusted), last_list: RefCell::new(None) }
    }

    fn save(&self) {
        let Some(f) = &self.file else { return };
        if let Err(e) = js::write_json(f, &json!(*self.trusted_apps.borrow())) {
            (self.log)(format!("applis de confiance : {e}"));
        }
    }

    pub fn forbidden(w: Option<&J>) -> bool {
        w.map(|w| forbidden_name(&js::gs(w, "process")) || forbidden_name(&js::gs(w, "app"))).unwrap_or(false)
    }

    pub fn trusted(&self, w: Option<&J>) -> bool {
        let Some(w) = w else { return false };
        let keys = [norm(&js::gs(w, "app")), norm(&js::gs(w, "process"))];
        self.trusted_apps.borrow().iter().any(|a| keys.contains(&norm(a)))
    }

    pub fn trust(&self, name: &str) -> bool {
        let clean = name.trim();
        if clean.is_empty() || self.trusted_apps.borrow().iter().any(|a| norm(a) == norm(clean)) {
            return false;
        }
        self.trusted_apps.borrow_mut().push(clean.to_string());
        self.save();
        true
    }

    pub fn untrust(&self, name: &str) -> bool {
        let before = self.trusted_apps.borrow().len();
        let n = norm(name);
        self.trusted_apps.borrow_mut().retain(|a| norm(a) != n && !norm(a).contains(&n));
        self.save();
        before != self.trusted_apps.borrow().len()
    }

    /// Niveau imposé à mcp__system__ui_agir selon l'application visée (None : réglage habituel).
    pub fn verdict(&self, input: &J) -> Option<&'static str> {
        let o = self.o.up();
        let ds = &o.desktop_state;
        let w = match input["hwnd"].as_i64() {
            Some(h) if h != 0 => ds.by_hwnd(h),
            _ => ds.find_window(&js::gs(input, "application"), false),
        };
        if Self::forbidden(w.as_ref()) || forbidden_name(&js::gs(input, "application")) {
            return Some("deny");
        }
        if self.trusted(w.as_ref()) {
            return Some("allow");
        }
        None
    }

    /// Fenêtre visée : une application nommée, sinon la fenêtre au premier plan (hors Jarvis).
    pub fn target(&self, spoken: &str) -> Option<J> {
        let o = self.o.up();
        let ds = &o.desktop_state;
        if !ds.ready() {
            return None;
        }
        if !spoken.is_empty() {
            return ds.find_window(spoken, false).or_else(|| ds.match_windows(spoken).into_iter().next());
        }
        let mut list: Vec<J> = ds.windows().into_iter().filter(|w| !js::gb(w, "min")).collect();
        list.sort_by(|a, b| js::gn_or(a, "z", 0.0).partial_cmp(&js::gn_or(b, "z", 0.0)).unwrap_or(std::cmp::Ordering::Equal));
        list.into_iter().next()
    }

    // --- Actions (appelées derrière local_tool) ---------------------------------------------------

    pub async fn list(&self, w: &J, limit: i64) -> J {
        let o = self.o.up();
        let r = o.desktop_state.request("uiFind", json!({ "hwnd": w["hwnd"], "limit": limit }), 12000).await;
        if !js::gb(&r, "ok") {
            return r;
        }
        let elements = js::arr(&r["elements"]);
        *self.last_list.borrow_mut() = Some((w["hwnd"].as_i64().unwrap_or(0), js::gs(w, "app"), elements.clone()));
        json!({ "ok": true, "elements": elements })
    }

    pub async fn click(&self, w: &J, text: &str) -> J {
        let app = js::gs(w, "app");
        if Self::forbidden(Some(w)) {
            return json!({ "ok": false, "error": format!("{app} : clics interdits pour ta sécurité") });
        }
        let o = self.o.up();
        let r = o.desktop_state.request("uiInvoke", json!({ "hwnd": w["hwnd"], "name": text }), 12000).await;
        if js::gb(&r, "ok") {
            let how = if js::gs(&r, "how") == "clic" { "accessibilité + clic" } else { "accessibilité" };
            return json!({ "ok": true, "how": how, "name": if js::truthy(&r["name"]) { js::gs(&r, "name") } else { text.to_string() } });
        }
        let ocr = self.ocr_click(w, text).await;
        if js::gb(&ocr, "ok") {
            return ocr;
        }
        let why = if js::truthy(&ocr["error"]) { format!(" ({})", js::gs(&ocr, "error")) } else { String::new() };
        json!({ "ok": false, "error": format!("je ne trouve pas « {text} » dans {app}{why}") })
    }

    pub async fn click_index(&self, w: &J, index: i64) -> J {
        let o = self.o.up();
        let r = o.desktop_state.request("uiInvoke", json!({ "hwnd": w["hwnd"], "name": "", "index": index }), 12000).await;
        if js::gb(&r, "ok") {
            json!({ "ok": true, "how": "accessibilité", "name": r["name"] })
        } else {
            json!({ "ok": false, "error": if js::truthy(&r["error"]) { js::gs(&r, "error") } else { "élément introuvable".into() } })
        }
    }

    pub async fn fill(&self, w: &J, field: &str, value: &str) -> J {
        if Self::forbidden(Some(w)) {
            return json!({ "ok": false, "error": format!("{} : saisie interdite pour ta sécurité", js::gs(w, "app")) });
        }
        let o = self.o.up();
        let r = o.desktop_state.request("uiSetValue", json!({ "hwnd": w["hwnd"], "field": field, "value": value }), 12000).await;
        if js::gb(&r, "ok") {
            json!({ "ok": true, "name": if js::truthy(&r["name"]) { js::gs(&r, "name") } else { field.to_string() } })
        } else {
            json!({ "ok": false, "error": if js::truthy(&r["error"]) { js::gs(&r, "error") } else { "champ introuvable".into() } })
        }
    }

    /// Suite de mots consécutifs qui forme le texte voulu (exacte d'abord, sinon qui le contient).
    pub fn locate(lines: &[J], text: &str) -> Option<J> {
        let want: Vec<String> = norm(text).split(' ').filter(|s| !s.is_empty()).map(String::from).collect();
        if want.is_empty() {
            return None;
        }
        let joined_want = want.join(" ");
        let mut best: Option<(i32, J)> = None;
        for line in lines {
            let words: Vec<(J, String)> = js::arr(&line["words"]).into_iter().map(|w| {
                let n = norm(&js::gs(&w, "t"));
                (w, n)
            }).filter(|(_, n)| !n.is_empty()).collect();
            for i in 0..words.len() {
                for j in i..words.len().min(i + want.len() + 2) {
                    let seq = &words[i..=j];
                    let joined = seq.iter().map(|(_, n)| n.as_str()).collect::<Vec<_>>().join(" ");
                    let score = if joined == joined_want {
                        3
                    } else if joined.starts_with(&joined_want) {
                        2
                    } else if want.len() == 1 && seq.len() == 1 && joined.contains(&want[0]) && want[0].chars().count() >= 4 {
                        1
                    } else {
                        0
                    };
                    if score > 0 && best.as_ref().map(|(s, _)| score > *s).unwrap_or(true) {
                        let f = |w: &J, k: &str| js::gn_or(w, k, 0.0);
                        let x1 = seq.iter().map(|(w, _)| f(w, "x")).fold(f64::INFINITY, f64::min);
                        let y1 = seq.iter().map(|(w, _)| f(w, "y")).fold(f64::INFINITY, f64::min);
                        let x2 = seq.iter().map(|(w, _)| f(w, "x") + f(w, "w")).fold(f64::NEG_INFINITY, f64::max);
                        let y2 = seq.iter().map(|(w, _)| f(w, "y") + f(w, "h")).fold(f64::NEG_INFINITY, f64::max);
                        let t = seq.iter().map(|(w, _)| js::gs(w, "t")).collect::<Vec<_>>().join(" ");
                        best = Some((score, json!({ "score": score, "x": ((x1 + x2) / 2.0).round() as i64, "y": ((y1 + y2) / 2.0).round() as i64, "text": t })));
                    }
                }
            }
        }
        best.map(|(_, b)| b)
    }

    async fn ocr_click(&self, w: &J, text: &str) -> J {
        let o = self.o.up();
        let ds = &o.desktop_state;
        ds.request("activate", json!({ "hwnd": w["hwnd"] }), 8000).await;
        js::sleep_ms(350).await;
        let file = std::env::temp_dir().join(format!("jarvis-ocr-{}.png", js::now_ms()));
        let cap = ds.request("capture", json!({ "hwnd": w["hwnd"], "file": file.to_string_lossy() }), 8000).await;
        if !js::gb(&cap, "ok") {
            return json!({ "ok": false, "error": if js::truthy(&cap["error"]) { js::gs(&cap, "error") } else { "capture impossible".into() } });
        }
        let path = file.to_string_lossy().to_string();
        let lines = tokio::task::spawn_blocking(move || jarvis_native::ocr::ocr_lines(&path, "fr-FR").unwrap_or_default()).await.unwrap_or_default();
        let _ = std::fs::remove_file(&file);
        let Some(hit) = Self::locate(&lines, text) else { return json!({ "ok": false, "error": "texte absent de la fenêtre" }) };
        let x = js::gn_or(&cap, "x", 0.0) as i64 + hit["x"].as_i64().unwrap_or(0);
        let y = js::gn_or(&cap, "y", 0.0) as i64 + hit["y"].as_i64().unwrap_or(0);
        let r = ds.request("click", json!({ "x": x, "y": y }), 8000).await;
        if js::gb(&r, "ok") {
            json!({ "ok": true, "how": "OCR + clic", "name": hit["text"] })
        } else {
            json!({ "ok": false, "error": r["error"] })
        }
    }

    /// Outil de l'IA : { action: lister | cliquer | remplir, application, cible, numero, champ, valeur }.
    pub async fn tool(&self, p: &J) -> J {
        let o = self.o.up();
        let w = match p["hwnd"].as_i64() {
            Some(h) if h != 0 => o.desktop_state.by_hwnd(h),
            _ => self.target(&js::gs(p, "application")),
        };
        let Some(w) = w else {
            let a = js::gs(p, "application");
            return json!({ "ok": false, "error": format!("aucune fenêtre ne correspond à « {} »", if a.is_empty() { "premier plan".into() } else { a }) });
        };
        match js::gs(p, "action").as_str() {
            "lister" => {
                let r = self.list(&w, 120).await;
                if !js::gb(&r, "ok") {
                    return r;
                }
                let elements: Vec<J> = js::arr(&r["elements"]).iter().enumerate().map(|(i, e)| json!({ "numero": i + 1, "nom": e["name"], "type": e["type"], "actif": e["enabled"] })).collect();
                json!({ "ok": true, "application": w["app"], "elements": elements })
            }
            "cliquer" => match js::num(&p["numero"]).filter(|n| *n != 0.0) {
                Some(n) => self.click_index(&w, n as i64 - 1).await,
                None => self.click(&w, &js::gs(p, "cible")).await,
            },
            "remplir" => self.fill(&w, &js::gs(p, "champ"), &js::gs(p, "valeur")).await,
            other => json!({ "ok": false, "error": format!("action inconnue : {other}") }),
        }
    }
}

#[allow(unused)]
fn _v(_: Value) {}
