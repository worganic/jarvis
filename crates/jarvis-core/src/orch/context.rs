// Contexte de travail (orchestrator/context.js) : « lance Claude Code ici », « où en est ce projet ? ».
// Le projet courant est déduit de la dernière fenêtre active qui n'est pas Jarvis : éditeur (dossier
// dans le titre), explorateur (dossier affiché), tout titre contenant un nom de projet ; à défaut le
// relais tmux en cours, puis le dernier projet dont on a parlé (30 min).

use std::cell::RefCell;

use serde_json::json;

use super::{OrchRef, Up};
use crate::js::{self, Timer, J};

pub fn is_here(folded: &str) -> bool {
    crate::re!(r"^(?:ici|la dedans|ce projet|ce dossier|ce repertoire|courant|actuel|ouvert|en cours|(?:le |mon )?projet (?:courant|actuel|ouvert|en cours)|le projet sur lequel je (?:travaille|suis))$").is_match(folded)
}

const RECENT_MS: i64 = 30 * 60000;

/// Dossier affiché dans une fenêtre de l'explorateur dont le titre est `title`.
async fn explorer_path(title: &str) -> Option<String> {
    let safe = title.replace('\'', "''");
    let script = format!(
        "$w = (New-Object -ComObject Shell.Application).Windows() | Where-Object {{ $_.LocationName -eq '{safe}' -or $_.LocationURL -like '*{safe}' }} | Select-Object -First 1; if ($w) {{ $w.Document.Folder.Self.Path }}"
    );
    let out = crate::proc::powershell(&script).await.ok()?;
    let out = out.trim().to_string();
    (!out.is_empty()).then_some(out)
}

pub struct ActiveContext {
    o: OrchRef,
    /// Dernière fenêtre active hors Jarvis : (processus, titre, instant).
    pub last: RefCell<Option<(String, String, i64)>>,
    /// Dernier projet dont on a parlé : { name, path, at }.
    pub last_project: RefCell<Option<J>>,
    timer: Timer,
}

impl ActiveContext {
    pub fn new(o: OrchRef) -> Self {
        Self { o, last: RefCell::new(None), last_project: RefCell::new(None), timer: Timer::new() }
    }

    fn is_jarvis(w: &J) -> bool {
        let proc_ = crate::re!(r"\.exe$").replace(&js::gs(w, "process").to_lowercase(), "").to_string();
        let title = js::gs(w, "title");
        title.is_empty() || proc_ == "jarvis" || crate::re!(r"(?i)^JARVIS$|Jarvis · Administration|^tmux · jarvis$").is_match(&title)
    }

    pub async fn poll(&self) {
        let o = self.o.up();
        let w = if o.desktop_state.ready() { o.desktop_state.foreground() } else { Some(super::system::foreground_window().await) };
        if let Some(w) = w {
            if !Self::is_jarvis(&w) {
                *self.last.borrow_mut() = Some((js::gs(&w, "process"), js::gs(&w, "title"), js::now_ms()));
            }
        }
    }

    pub fn start(&self) {
        let o = self.o.clone();
        js::spawn(async move { o.up().context.poll().await });
        let o2 = self.o.clone();
        self.o.up().desktop_state.on("foreground", move |_| {
            let o = o2.clone();
            js::spawn(async move { o.up().context.poll().await });
        });
        let o3 = self.o.clone();
        self.timer.every(5000, move || {
            let o = o3.clone();
            js::spawn(async move { o.up().context.poll().await });
        });
    }

    pub fn stop(&self) {
        self.timer.clear();
    }

    /// Un projet vient d'être nommé : il devient « ce projet ».
    pub fn remember(&self, project: &J) {
        if js::truthy(&project["path"]) {
            *self.last_project.borrow_mut() = Some(json!({ "name": project["name"], "path": project["path"], "at": js::now_ms() }));
        }
    }

    /// Projet courant : { name, path, how, … } ou None.
    pub async fn current_project(&self) -> Option<J> {
        let o = self.o.up();
        let projects = o.projects.clone();
        self.poll().await;
        let last = self.last.borrow().clone();
        let with_how = |mut p: J, how: &str| {
            p["how"] = json!(how);
            p
        };
        if let Some((process, title, _)) = last {
            let proc_ = crate::re!(r"\.exe$").replace(&process.to_lowercase(), "").to_string();
            if crate::re!(r"^(?:code|cursor|windsurf|code - insiders|idea64|webstorm64|pycharm64|phpstorm64|rider64)$").is_match(&proc_) {
                let parts: Vec<String> = crate::re!(r"\s+[-–—]\s+")
                    .split(&title)
                    .map(|s| crate::re!(r"^[●•*]\s*").replace(s, "").trim().to_string())
                    .filter(|s| !s.is_empty())
                    .collect();
                for part in parts.iter().rev() {
                    if crate::re!(r"(?i)^(?:Visual Studio Code|Cursor|Windsurf|IntelliJ IDEA|WebStorm|PyCharm|PhpStorm|Rider)").is_match(part) {
                        continue;
                    }
                    if let Some(p) = projects.find(part) {
                        if js::num(&p["score"]).unwrap_or(0.0) >= 80.0 {
                            return Some(with_how(p, "éditeur"));
                        }
                    }
                }
            }
            if proc_ == "explorer" {
                if let Some(dir) = explorer_path(&title).await {
                    if let Some(p) = projects.of_path(&dir) {
                        return Some(with_how(p, "explorateur"));
                    }
                }
            }
            // Tout titre qui contient le nom d'un projet (le nom le plus long l'emporte).
            let t = format!(" {} ", super::projects::fold_name(&title));
            let mut best: Option<J> = None;
            for p in projects.list() {
                let f = js::gs(&p, "folded");
                let longer = best.as_ref().map(|b| f.chars().count() > js::gs(b, "folded").chars().count()).unwrap_or(true);
                if f.chars().count() >= 4 && t.contains(&format!(" {f} ")) && longer {
                    best = Some(p);
                }
            }
            if let Some(b) = best {
                return Some(with_how(b, "fenêtre"));
            }
        }
        if let Some(label) = o.tmux_voice.relay_label() {
            let l = crate::re!(r"^Claude · ").replace(&label, "").to_string();
            if let Some(p) = projects.find(&l) {
                if js::num(&p["score"]).unwrap_or(0.0) >= 80.0 {
                    return Some(with_how(p, "relais"));
                }
            }
        }
        let lp = self.last_project.borrow().clone();
        if let Some(lp) = lp {
            if js::now_ms() - lp["at"].as_i64().unwrap_or(0) < RECENT_MS {
                return Some(with_how(lp, "conversation"));
            }
        }
        None
    }
}
