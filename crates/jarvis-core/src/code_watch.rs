// Surveillance du code de Jarvis (core/code-watch.js de la v3) : toute modification des sources est
// signalée à l'interface (« Jarvis a été modifié »), et un clic ou « Jarvis, relance-toi » relance tout.
// En v4 tout est compilé (noyau, moteur vocal, coquille qui intègre ui/) : une modification demande
// toujours une recompilation, faite par `jarvis-core relaunch --build` (voir relaunch.rs).
// Au démarrage, des sources plus récentes que jarvis.exe (modifiées sans recompilation) sont signalées.

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::SystemTime;

use notify::{RecursiveMode, Watcher};
use serde_json::json;

use crate::js::{self, Timer, J};

fn is_source(p: &str) -> bool {
    crate::re!(r"(?i)\.(?:js|py|html|css|json|rs|toml|ps1)$").is_match(p)
}

fn is_ignored(p: &str) -> bool {
    crate::re!(r"(?i)(?:^|[\\/])(?:__pycache__|cache|node_modules|target|target-next|\.git|gen)(?:[\\/]|$)|\.log(?:\.\d+)?$|\.tmp\.js$").is_match(p)
}

fn mtime(p: &Path) -> Option<SystemTime> {
    std::fs::metadata(p).and_then(|m| m.modified()).ok()
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let s = p.to_string_lossy().to_string();
        if is_ignored(&s) {
            continue;
        }
        if p.is_dir() {
            walk(&p, out);
        } else if is_source(&s) {
            out.push(p);
        }
    }
}

/// Dossiers surveillés : (chemin relatif, nature).
const WATCHED: &[(&str, &str)] = &[
    ("crates/jarvis-core/src", "noyau"),
    ("crates/jarvis-core/data", "noyau"),
    ("crates/jarvis-voice/src", "noyau"),
    ("crates/jarvis-native/src", "noyau"),
    ("ui", "interface"),
    ("crates/jarvis-shell/src", "interface"),
    // Modules : interface (ui/, module.json) lue sur le disque ; partie noyau (core/) à recompiler.
    ("modules", "noyau"),
    ("crates/jarvis-app/src", "noyau"),
    ("crates/jarvis-modules/src", "noyau"),
];

/// Fichier d'interface (ui/, ou ui/ et fiche d'un module) : servi depuis le disque, rien à recompiler.
fn is_disk_ui(relf: &str) -> bool {
    if relf.starts_with("ui/") {
        return true;
    }
    let parts: Vec<&str> = relf.split('/').collect();
    parts.first() == Some(&"modules") && (parts.get(2) == Some(&"ui") || (parts.len() == 3 && parts[2] == "module.json"))
}

/// Exécutables compilés ensemble par la relance : la plus ancienne date fait foi (une compilation qui
/// échoue à moitié, disque plein par exemple, laisse un exécutable ancien à côté d'autres récents).
const EXES: &[&str] = &["jarvis.exe", "jarvis-core.exe", "jarvis-voice.exe"];

/// Date de compilation à retenir : la plus ancienne des exécutables présents.
pub fn oldest_build(times: &[Option<SystemTime>]) -> Option<SystemTime> {
    times.iter().flatten().min().copied()
}

pub struct CodeWatch {
    root: PathBuf,
    exe_dir: PathBuf,
    on_change: Rc<dyn Fn(J)>,
    log: Rc<dyn Fn(String)>,
    changes: Rc<RefCell<BTreeMap<String, String>>>,
    started_at: RefCell<SystemTime>,
    watcher: RefCell<Option<notify::RecommendedWatcher>>,
    timer: Rc<Timer>,
    ui_timer: Rc<Timer>,
    ui_files: Rc<RefCell<Vec<String>>>,
    /// ui/ servie depuis le disque par la coquille (voir jarvis-shell/src/disk_assets.rs).
    disk_ui: bool,
}

impl CodeWatch {
    pub fn new(root: PathBuf, on_change: Rc<dyn Fn(J)>, log: Rc<dyn Fn(String)>) -> Rc<Self> {
        let exe_dir = root.join("target").join("release");
        Rc::new(Self {
            root,
            exe_dir,
            on_change,
            log,
            changes: Rc::new(RefCell::new(BTreeMap::new())),
            started_at: RefCell::new(SystemTime::now()),
            watcher: RefCell::new(None),
            timer: Rc::new(Timer::new()),
            ui_timer: Rc::new(Timer::new()),
            ui_files: Rc::new(RefCell::new(vec![])),
            disk_ui: std::env::var("JARVIS_UI_EMBARQUEE").is_err(),
        })
    }

    pub fn start(self: &Rc<Self>) {
        *self.started_at.borrow_mut() = SystemTime::now();
        // Sources modifiées depuis la dernière compilation : à recompiler dès la prochaine relance.
        let times: Vec<Option<SystemTime>> = EXES.iter().map(|e| mtime(&self.exe_dir.join(e))).collect();
        if let Some(exe_time) = oldest_build(&times) {
            let limit = exe_time + std::time::Duration::from_secs(2);
            for (dir, kind) in WATCHED {
                let mut files = vec![];
                walk(&self.root.join(dir), &mut files);
                for f in files {
                    if mtime(&f).map(|t| t > limit).unwrap_or(false) {
                        let rel = f.strip_prefix(&self.root).unwrap_or(&f).to_string_lossy().replace('/', "\\");
                        if self.disk_ui && is_disk_ui(&rel.replace('\\', "/")) {
                            continue; // l'interface est lue sur le disque : rien à recompiler
                        }
                        self.changes.borrow_mut().insert(rel, kind.to_string());
                    }
                }
            }
        }
        // Les événements arrivent sur un fil du système : relayés vers le fil du noyau.
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<PathBuf>();
        let watcher = notify::recommended_watcher(move |res: notify::Result<notify::Event>| {
            if let Ok(ev) = res {
                for p in ev.paths {
                    let _ = tx.send(p);
                }
            }
        });
        match watcher {
            Ok(mut w) => {
                for (dir, _) in WATCHED {
                    let p = self.root.join(dir);
                    if let Err(e) = w.watch(&p, RecursiveMode::Recursive) {
                        (self.log)(format!("[code] surveillance impossible de {dir} : {e}"));
                    }
                }
                *self.watcher.borrow_mut() = Some(w);
            }
            Err(e) => (self.log)(format!("[code] surveillance impossible : {e}")),
        }
        let me = Rc::downgrade(self);
        js::spawn(async move {
            while let Some(p) = rx.recv().await {
                let Some(me) = me.upgrade() else { break };
                me.touch(&p);
            }
        });
        if !self.changes.borrow().is_empty() {
            let me = Rc::downgrade(self);
            js::set_timeout(0, move || {
                if let Some(me) = me.upgrade() {
                    (me.on_change)(me.state());
                }
            });
        }
    }

    fn touch(&self, full: &Path) {
        let rel = full.strip_prefix(&self.root).unwrap_or(full).to_string_lossy().replace('/', "\\");
        if !is_source(&rel) || is_ignored(&rel) {
            return;
        }
        let relf = rel.replace('\\', "/");
        let Some((_, kind)) = WATCHED.iter().find(|(d, _)| relf.starts_with(d)) else { return };
        // Le système signale aussi de simples lectures : on ne retient qu'un fichier réellement
        // écrit depuis le démarrage.
        if full.exists() && mtime(full).map(|t| t < *self.started_at.borrow()).unwrap_or(false) {
            return;
        }
        if self.disk_ui && is_disk_ui(&relf) {
            // Interface lue sur le disque : les fenêtres se rechargent, sans recompilation ni relance.
            self.ui_files.borrow_mut().push(rel);
            let (files, on_change, log) = (self.ui_files.clone(), self.on_change.clone(), self.log.clone());
            self.ui_timer.set(400, move || {
                let list: Vec<String> = files.borrow_mut().drain(..).collect();
                log(format!("[code] interface modifiée ({} fichier(s)) : rechargement des fenêtres", list.len()));
                on_change(json!({ "type": "ui_changed", "files": list }));
            });
            return;
        }
        self.changes.borrow_mut().insert(rel, kind.to_string());
        let (changes, on_change, log) = (self.changes.clone(), self.on_change.clone(), self.log.clone());
        self.timer.set(1500, move || {
            let st = state_of(&changes.borrow());
            log(format!("[code] Jarvis a été modifié : {} fichier(s), recompilation nécessaire", js::s(&st["count"])));
            on_change(st);
        });
    }

    pub fn state(&self) -> J {
        state_of(&self.changes.borrow())
    }

    pub fn stop(&self) {
        self.timer.clear();
        self.ui_timer.clear();
        self.watcher.borrow_mut().take();
    }
}

fn state_of(changes: &BTreeMap<String, String>) -> J {
    let files: Vec<&String> = changes.keys().collect();
    json!({
        "type": "code_changed",
        "changed": !files.is_empty(),
        "files": files.iter().take(40).collect::<Vec<_>>(),
        "count": files.len(),
        // v4 : noyau, moteur vocal et interface sont compilés.
        "needsBuild": !files.is_empty()
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_plus_ancienne_compilation_fait_foi() {
        let t = |s: u64| Some(SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(s));
        assert_eq!(oldest_build(&[t(300), t(100), t(200)]), t(100), "noyau resté ancien : à recompiler");
        assert_eq!(oldest_build(&[t(300), None]), t(300), "exécutable absent ignoré");
        assert_eq!(oldest_build(&[None]), None);
    }
}
