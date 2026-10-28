// Registre des applications (orchestrator/app-index.js) : les raccourcis du menu Démarrer sont indexés,
// puis « ouvre Discord », « lance Blender » trouvent le bon raccourci par correspondance approchée,
// sans modèle. Seuls des fichiers .lnk des dossiers du menu Démarrer peuvent être lancés.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use serde_json::json;

use crate::js::{self, J};

pub fn fold(text: &str) -> String {
    let t = js::fold_lower(text);
    crate::re!(r"[^a-z0-9+]+").replace_all(&t, " ").trim().to_string()
}

pub fn levenshtein(a: &str, b: &str) -> usize {
    if a == b {
        return 0;
    }
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut prev: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut diag = prev[0];
        prev[0] = i;
        for j in 1..=b.len() {
            let tmp = prev[j];
            prev[j] = (prev[j] + 1).min(prev[j - 1] + 1).min(diag + if a[i - 1] == b[j - 1] { 0 } else { 1 });
            diag = tmp;
        }
    }
    prev[b.len()]
}

fn alias(q: &str) -> Option<&'static str> {
    Some(match q {
        "vs code" | "vscode" | "visual studio" => "visual studio code",
        "photoshop" => "adobe photoshop",
        "illustrator" => "adobe illustrator",
        "premiere" => "adobe premiere pro",
        "navigateur" | "chrome" => "google chrome",
        "powerpoint" => "powerpoint",
        "outlook" => "outlook",
        _ => return None,
    })
}

#[derive(Clone)]
pub struct AppEntry {
    pub name: String,
    pub key: String,
    pub file: PathBuf,
}

pub struct AppIndex {
    log: Rc<dyn Fn(String)>,
    pub roots: Vec<PathBuf>,
    pub entries: RefCell<Vec<AppEntry>>,
    built_at: Cell<i64>,
}

fn walk(dir: &Path, depth: u32, out: &mut Vec<AppEntry>) {
    if depth > 4 {
        return;
    }
    let Ok(items) = std::fs::read_dir(dir) else { return };
    for item in items.flatten() {
        let full = item.path();
        let Ok(ft) = item.file_type() else { continue };
        if ft.is_dir() {
            walk(&full, depth + 1, out);
            continue;
        }
        let name = item.file_name().to_string_lossy().into_owned();
        if !name.to_lowercase().ends_with(".lnk") {
            continue;
        }
        let name = name[..name.len() - 4].to_string();
        let key = fold(&name);
        let noise = crate::re!(r"\b(?:uninstall|desinstaller|desinstallation|readme|lisez moi|help|aide|documentation|release notes|license|licence|website|site web|manual|manuel)\b");
        if !key.is_empty() && !noise.is_match(&key) {
            out.push(AppEntry { name, key, file: full });
        }
    }
}

impl AppIndex {
    pub fn new(log: Rc<dyn Fn(String)>) -> Self {
        let mut roots = vec![];
        if let Some(a) = js::env("APPDATA") {
            roots.push(PathBuf::from(a).join("Microsoft").join("Windows").join("Start Menu").join("Programs"));
        }
        if let Some(p) = js::env("ProgramData") {
            roots.push(PathBuf::from(p).join("Microsoft").join("Windows").join("Start Menu").join("Programs"));
        }
        Self { log, roots, entries: RefCell::new(vec![]), built_at: Cell::new(0) }
    }

    pub fn build(&self) -> usize {
        let mut entries = vec![];
        for root in &self.roots {
            walk(root, 0, &mut entries);
        }
        let mut seen = HashSet::new();
        entries.retain(|e| seen.insert(e.key.clone()));
        let n = entries.len();
        *self.entries.borrow_mut() = entries;
        self.built_at.set(js::now_ms());
        (self.log)(format!("[applications] {n} raccourcis du menu Démarrer indexés"));
        n
    }

    pub fn ensure(&self) {
        if self.built_at.get() == 0 || js::now_ms() - self.built_at.get() > 30 * 60 * 1000 {
            self.build();
        }
    }

    /// Meilleure correspondance pour un nom dicté : (entrée, score).
    pub fn find(&self, spoken: &str) -> Option<(AppEntry, f64)> {
        self.ensure();
        let q = fold(spoken);
        let q = crate::re!(r"^(?:l |la |le |les |mon |ma |l application |le logiciel )").replace(&q, "").to_string();
        if q.is_empty() {
            return None;
        }
        let q = alias(&q).map(String::from).unwrap_or(q);
        let qlen = q.chars().count();
        let mut best: Option<(AppEntry, f64)> = None;
        for e in self.entries.borrow().iter() {
            let first = e.key.split(' ').next().unwrap_or("");
            let score = if e.key == q {
                1.0
            } else if first == q || e.key.starts_with(&format!("{q} ")) {
                0.93
            } else if e.key.split(' ').any(|w| w == q) && qlen >= 3 {
                0.85
            } else if qlen >= 4 && e.key.contains(&q) {
                0.8
            } else {
                let d = levenshtein(&q, &e.key).min(levenshtein(&q, first));
                let len = qlen.max(4);
                if d <= len / 4 {
                    0.75 - d as f64 * 0.05
                } else {
                    0.0
                }
            };
            if score > 0.0 {
                let better = match &best {
                    None => true,
                    Some((b, s)) => score > *s || (score == *s && e.key.chars().count() < b.key.chars().count()),
                };
                if better {
                    best = Some((e.clone(), score));
                }
            }
        }
        best.filter(|(_, s)| *s >= 0.7)
    }

    /// Lance un raccourci indexé (jamais un chemin arbitraire).
    pub fn open(&self, entry: &AppEntry) -> Result<J, String> {
        let file = &entry.file;
        let full = super::zones::resolve(&file.to_string_lossy()).to_lowercase();
        let allowed = full.ends_with(".lnk")
            && self.roots.iter().any(|r| full.starts_with(&format!("{}\\", super::zones::resolve(&r.to_string_lossy()).to_lowercase())));
        if !allowed || !file.exists() {
            return Err("raccourci non autorisé".into());
        }
        crate::proc::detached("explorer.exe", &[&file.to_string_lossy()]);
        Ok(json!({ "ok": true, "message": format!("Application {} lancée", entry.name) }))
    }

    pub fn names(&self) -> Vec<String> {
        self.ensure();
        self.entries.borrow().iter().map(|e| e.name.clone()).collect()
    }
}
