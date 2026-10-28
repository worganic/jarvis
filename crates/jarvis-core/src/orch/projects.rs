// Projets de l'utilisateur (orchestrator/projects.js) : chaque dossier du dossier racine (réglable
// dans Administration › Projets) est un projet ; un sous-dossier qui porte ses propres marqueurs en
// est un aussi. Sert au routeur local, à l'outil MCP « projets » et à l'administration. Lecture seule.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use serde_json::{json, Value};

use crate::js::{self, J};

const MARKERS: &[&str] = &["package.json", ".git", "Cargo.toml", "pyproject.toml", "requirements.txt", "composer.json", "go.mod", "CLAUDE.md", "README.md", "index.html"];
const LIST_TTL: i64 = 30000;
const INFO_TTL: i64 = 60000;

fn skip(name: &str) -> bool {
    crate::re!(r"(?i)^(?:\.|node_modules$|vendor$|dist$|build$|target$|__pycache__$|\$)").is_match(name)
}

/// Nom replié pour comparer un nom dit à un nom de dossier (« lego-rollerCoaster » → « lego roller coaster »).
pub fn fold_name(text: &str) -> String {
    let t = crate::re!(r"([a-z])([A-Z])").replace_all(text, "$1 $2");
    let t = js::fold_lower(&t);
    let t = crate::re!(r"\b\d{4}[-_ ]\d{2}[-_ ]\d{2}\b").replace_all(&t, " ");
    let t = crate::re!(r"\bcopie\b").replace_all(&t, " ");
    let t = crate::re!(r"[^a-z0-9]+").replace_all(&t, " ");
    crate::re!(r"\s+").replace_all(&t, " ").trim().to_string()
}

pub fn edit_distance(a: &str, b: &str) -> usize {
    super::app_index::levenshtein(a, b)
}

fn exists(p: &Path) -> bool {
    p.exists()
}

fn read_text(file: &Path, max: usize) -> String {
    std::fs::read_to_string(file).map(|t| t.chars().take(max).collect()).unwrap_or_default()
}

async fn git(dir: &str, args: &[&str]) -> Option<String> {
    let mut a = vec!["-C".to_string(), dir.to_string()];
    a.extend(args.iter().map(|s| s.to_string()));
    let o = crate::proc::run("git", &a, None, 4000).await.ok()?;
    (o.code == 0).then(|| o.stdout.trim().to_string())
}

/// Première phrase utile d'un README / CLAUDE.md (ni titre, ni image, ni badge, ni code).
fn first_paragraph(text: &str) -> String {
    let mut in_code = false;
    for raw in text.lines() {
        let line = raw.trim();
        if line.starts_with("```") {
            in_code = !in_code;
            continue;
        }
        if in_code || line.is_empty() || crate::re!(r"^(?:#|!\[|\[!\[|<|>|---|===|\||-\s*\[|\*\*\*)").is_match(line) {
            continue;
        }
        let clean = crate::re!(r"[*_`]").replace_all(line, "");
        let clean = crate::re!(r"\[([^\]]+)\]\([^)]*\)").replace_all(&clean, "$1").trim().to_string();
        let n = clean.chars().count();
        if n >= 12 {
            return if n > 220 { format!("{}…", clean.chars().take(217).collect::<String>()) } else { clean };
        }
    }
    String::new()
}

fn detect_tech(dir: &Path) -> Vec<String> {
    let mut tech: Vec<String> = vec![];
    let pkg_text = read_text(&dir.join("package.json"), 20000);
    if !pkg_text.is_empty() {
        let pkg: J = serde_json::from_str(&pkg_text).unwrap_or(json!({}));
        let deps = js::spread(&pkg["dependencies"], &pkg["devDependencies"]);
        let has = |n: &str| deps.get(n).is_some();
        if has("@tauri-apps/api") || has("@tauri-apps/cli") || exists(&dir.join("src-tauri")) {
            tech.push("Tauri".into());
        }
        if has("electron") {
            tech.push("Electron".into());
        }
        if has("@angular/core") {
            tech.push("Angular".into());
        }
        if has("next") {
            tech.push("Next.js".into());
        } else if has("react") {
            tech.push("React".into());
        }
        if has("vue") {
            tech.push("Vue".into());
        }
        if has("svelte") {
            tech.push("Svelte".into());
        }
        if has("express") || has("fastify") {
            tech.push("serveur Node".into());
        }
        if tech.is_empty() {
            tech.push("Node.js".into());
        }
    }
    if exists(&dir.join("Cargo.toml")) || exists(&dir.join("shell").join("Cargo.toml")) {
        tech.push("Rust".into());
    }
    if exists(&dir.join("pyproject.toml")) || exists(&dir.join("requirements.txt")) {
        tech.push("Python".into());
    }
    if exists(&dir.join("composer.json")) {
        tech.push("PHP".into());
    }
    if exists(&dir.join("go.mod")) {
        tech.push("Go".into());
    }
    if tech.is_empty() && exists(&dir.join("index.html")) {
        tech.push("site web".into());
    }
    let mut seen = std::collections::HashSet::new();
    tech.retain(|t| seen.insert(t.clone()));
    tech
}

pub fn is_project_dir(dir: &Path) -> bool {
    MARKERS.iter().any(|m| exists(&dir.join(m)))
}

fn mtime_ms(p: &Path) -> i64 {
    std::fs::metadata(p).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// Dernière modification visible : le dossier et ses entrées de premier niveau.
fn last_modified(dir: &Path) -> i64 {
    let mut latest = mtime_ms(dir);
    if let Ok(rd) = std::fs::read_dir(dir) {
        for e in rd.flatten().take(80) {
            let n = e.file_name().to_string_lossy().into_owned();
            if n == "node_modules" || n == ".git" {
                continue;
            }
            latest = latest.max(mtime_ms(&e.path()));
        }
    }
    latest
}

/// « il y a 3 jours », « aujourd'hui »… (pour être dit à voix haute).
pub fn ago(ms: i64) -> String {
    if ms == 0 {
        return "date inconnue".into();
    }
    let days = (js::now_ms() - ms).div_euclid(86_400_000);
    if days <= 0 {
        "aujourd'hui".into()
    } else if days == 1 {
        "hier".into()
    } else if days < 7 {
        format!("il y a {days} jours")
    } else if days < 30 {
        format!("il y a {} semaine{}", (days as f64 / 7.0).round() as i64, if days >= 14 { "s" } else { "" })
    } else if days < 365 {
        format!("il y a {} mois", (days as f64 / 30.0).round() as i64)
    } else {
        "il y a plus d'un an".into()
    }
}

/// Numéro dit (« 3 », « trois », « numéro 3 ») → nombre.
pub fn spoken_number(text: &str) -> Option<i64> {
    let s = fold_name(text);
    let s = crate::re!(r"^(?:numero|n|no) ").replace(&s, "").to_string();
    if crate::re!(r"^\d{1,3}$").is_match(&s) {
        return s.parse().ok();
    }
    Some(match s.as_str() {
        "un" | "une" | "premier" | "premiere" => 1,
        "deux" => 2,
        "trois" => 3,
        "quatre" => 4,
        "cinq" => 5,
        "six" => 6,
        "sept" => 7,
        "huit" => 8,
        "neuf" => 9,
        "dix" => 10,
        "onze" => 11,
        "douze" => 12,
        "treize" => 13,
        "quatorze" => 14,
        "quinze" => 15,
        "seize" => 16,
        "vingt" => 20,
        _ => return None,
    })
}

fn key_of(dir: &str) -> String {
    super::zones::resolve(dir).to_lowercase()
}

pub struct Projects {
    get_root: Box<dyn Fn() -> String>,
    prefs_file: Option<PathBuf>,
    on_change: Box<dyn Fn()>,
    log: Rc<dyn Fn(String)>,
    cache: RefCell<Option<(i64, String, Vec<J>)>>,
    infos: RefCell<HashMap<String, (i64, J)>>,
    pub prefs: RefCell<J>,
}

impl Projects {
    pub fn new(get_root: Box<dyn Fn() -> String>, prefs_file: Option<PathBuf>, on_change: Box<dyn Fn()>, log: Rc<dyn Fn(String)>) -> Self {
        let mut prefs = json!({ "favorites": [], "hidden": [], "numbered": [] });
        if let Some(saved) = prefs_file.as_ref().and_then(|f| js::read_json(f)) {
            for k in ["favorites", "hidden", "numbered"] {
                if saved[k].is_array() {
                    prefs[k] = saved[k].clone();
                }
            }
        }
        Self { get_root, prefs_file, on_change, log, cache: RefCell::new(None), infos: RefCell::new(HashMap::new()), prefs: RefCell::new(prefs) }
    }

    fn save_prefs(&self) {
        let Some(f) = &self.prefs_file else { return };
        if let Err(e) = js::write_json(f, &self.prefs.borrow()) {
            (self.log)(format!("projets : préférences non enregistrées ({e})"));
        }
    }

    fn pref_has(&self, list: &str, p: &J) -> bool {
        let k = key_of(&js::gs(p, "path"));
        js::str_list(&self.prefs.borrow()[list]).contains(&k)
    }

    pub fn is_favorite(&self, p: &J) -> bool {
        self.pref_has("favorites", p)
    }

    pub fn is_hidden(&self, p: &J) -> bool {
        self.pref_has("hidden", p)
    }

    fn set_pref(&self, list: &str, other: &str, p: &J, on: bool) -> bool {
        let k = key_of(&js::gs(p, "path"));
        let was = self.pref_has(list, p);
        {
            let mut prefs = self.prefs.borrow_mut();
            let mut l: Vec<String> = js::str_list(&prefs[list]).into_iter().filter(|x| *x != k).collect();
            if on {
                l.push(k.clone());
                let o: Vec<String> = js::str_list(&prefs[other]).into_iter().filter(|x| *x != k).collect();
                prefs[other] = json!(o);
            }
            prefs[list] = json!(l);
        }
        self.save_prefs();
        (self.on_change)();
        was != on
    }

    /// Favori : mis en avant. S'exclut avec « mis de côté ».
    pub fn set_favorite(&self, p: &J, on: bool) -> bool {
        self.set_pref("favorites", "hidden", p, on)
    }

    pub fn set_hidden(&self, p: &J, on: bool) -> bool {
        self.set_pref("hidden", "favorites", p, on)
    }

    /// Projets de premier niveau dans l'ordre de la card. scope : liste, tous, favoris, de_cote.
    pub async fn ordered(&self, scope: &str) -> Vec<J> {
        let tops: Vec<J> = self.list().into_iter().filter(|p| p["parent"].is_null()).collect();
        let keep: Vec<J> = tops
            .into_iter()
            .filter(|p| match scope {
                "tous" => true,
                "de_cote" => self.is_hidden(p),
                "favoris" => self.is_favorite(p),
                _ => !self.is_hidden(p),
            })
            .collect();
        let mut out = vec![];
        for p in keep {
            let mut v = js::spread(&p, &self.info(&p).await);
            v["favorite"] = json!(self.is_favorite(&p));
            v["hidden"] = json!(self.is_hidden(&p));
            out.push(v);
        }
        out.sort_by(|a, b| {
            (js::gb(b, "favorite") as i64 - js::gb(a, "favorite") as i64)
                .cmp(&0)
                .then((js::gb(a, "hidden") as i64).cmp(&(js::gb(b, "hidden") as i64)))
                .then(b["lastActivity"].as_i64().unwrap_or(0).cmp(&a["lastActivity"].as_i64().unwrap_or(0)))
        });
        out
    }

    /// La card affichée fixe les numéros : « lance Claude Code dans le projet 3 ».
    pub fn remember_numbers(&self, list: &[J]) {
        self.prefs.borrow_mut()["numbered"] = json!(list.iter().map(|p| key_of(&js::gs(p, "path"))).collect::<Vec<_>>());
        self.save_prefs();
        (self.on_change)();
    }

    pub fn by_number(&self, n: i64) -> Option<J> {
        let k = js::str_list(&self.prefs.borrow()["numbered"]).get((n - 1).max(0) as usize).cloned().filter(|_| n >= 1)?;
        self.list().into_iter().find(|p| key_of(&js::gs(p, "path")) == k)
    }

    pub fn number_of(&self, p: &J) -> Option<i64> {
        let k = key_of(&js::gs(p, "path"));
        js::str_list(&self.prefs.borrow()["numbered"]).iter().position(|x| *x == k).map(|i| i as i64 + 1)
    }

    /// Souvenir épinglé de la mémoire : la liste des projets, favoris et mis de côté compris.
    pub fn memory_text(&self) -> String {
        let tops: Vec<J> = self.list().into_iter().filter(|p| p["parent"].is_null()).collect();
        if tops.is_empty() {
            return String::new();
        }
        let label = |p: &J| match self.number_of(p) {
            Some(n) => format!("{n}. {}", js::gs(p, "name")),
            None => js::gs(p, "name"),
        };
        let by_num = |a: &J, b: &J| self.number_of(a).unwrap_or(999).cmp(&self.number_of(b).unwrap_or(999));
        let mut fav: Vec<J> = tops.iter().filter(|p| self.is_favorite(p)).cloned().collect();
        fav.sort_by(by_num);
        let hidden: Vec<J> = tops.iter().filter(|p| self.is_hidden(p)).cloned().collect();
        let mut others: Vec<J> = tops.iter().filter(|p| !self.is_favorite(p) && !self.is_hidden(p)).cloned().collect();
        others.sort_by(by_num);
        let parts: Vec<String> = [
            format!("Projets de l'utilisateur (dossier {}), numérotés comme dans la card « Mes projets » :", self.root()),
            if fav.is_empty() { String::new() } else { format!("favoris (mis en avant) : {} ;", fav.iter().map(label).collect::<Vec<_>>().join(", ")) },
            if others.is_empty() { String::new() } else { format!("autres projets : {} ;", others.iter().map(label).collect::<Vec<_>>().join(", ")) },
            if hidden.is_empty() {
                String::new()
            } else {
                format!(
                    "mis de côté (à ne pas citer quand je demande mes projets, sauf si je les demande explicitement) : {}.",
                    hidden.iter().map(|p| js::gs(p, "name")).collect::<Vec<_>>().join(", ")
                )
            },
        ]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect();
        crate::re!(r" ;$").replace(&parts.join(" "), ".").into_owned()
    }

    pub fn root(&self) -> String {
        (self.get_root)().trim().to_string()
    }

    pub fn invalidate(&self) {
        *self.cache.borrow_mut() = None;
        self.infos.borrow_mut().clear();
        (self.on_change)();
    }

    /// Liste légère (noms, chemins, groupes), mise en cache 30 s.
    pub fn list(&self) -> Vec<J> {
        let root = self.root();
        if let Some((at, r, items)) = self.cache.borrow().as_ref() {
            if *r == root && js::now_ms() - at < LIST_TTL {
                return items.clone();
            }
        }
        let mut items = vec![];
        let entries: Vec<std::fs::DirEntry> = std::fs::read_dir(&root).map(|rd| rd.flatten().collect()).unwrap_or_default();
        for e in entries {
            let name = e.file_name().to_string_lossy().into_owned();
            if !e.file_type().map(|t| t.is_dir()).unwrap_or(false) || skip(&name) {
                continue;
            }
            let dir = Path::new(&root).join(&name);
            items.push(json!({ "name": name, "path": dir.to_string_lossy(), "parent": null, "folded": fold_name(&name) }));
            let subs: Vec<std::fs::DirEntry> = std::fs::read_dir(&dir).map(|rd| rd.flatten().collect()).unwrap_or_default();
            for s in subs {
                let sname = s.file_name().to_string_lossy().into_owned();
                if !s.file_type().map(|t| t.is_dir()).unwrap_or(false) || skip(&sname) {
                    continue;
                }
                let sub = dir.join(&sname);
                if ["package.json", ".git", "Cargo.toml", "CLAUDE.md", "pyproject.toml", "composer.json"].iter().any(|m| exists(&sub.join(m))) {
                    items.push(json!({ "name": sname, "path": sub.to_string_lossy(), "parent": name, "folded": fold_name(&sname) }));
                }
            }
        }
        *self.cache.borrow_mut() = Some((js::now_ms(), root, items.clone()));
        items
    }

    /// Projet désigné par un nom dit (« stats ia », « jarvis v2 », « projet 3 »), avec son score.
    pub fn find(&self, spoken: &str) -> Option<J> {
        let s = crate::re!(r"(?i)^(?:le|la|les|l|mon|ma|du|de)\s+").replace(spoken, "");
        let s = crate::re!(r"(?i)^projet\s+").replace(&s, "");
        let want = fold_name(&s);
        if want.is_empty() {
            return None;
        }
        if let Some(n) = spoken_number(&want) {
            if let Some(mut p) = self.by_number(n) {
                p["score"] = json!(100);
                p["number"] = json!(n);
                return Some(p);
            }
        }
        let flat = want.replace(' ', "");
        let mut best: Option<J> = None;
        let mut best_score = 0;
        for p in self.list() {
            let name = js::gs(&p, "folded");
            let name_flat = name.replace(' ', "");
            let mut score = if name == want || name_flat == flat {
                100
            } else if name.starts_with(&want) || name_flat.starts_with(&flat) {
                80
            } else if name.contains(&want) || name_flat.contains(&flat) {
                60
            } else {
                let words: Vec<&str> = want.split(' ').collect();
                let target: Vec<&str> = name.split(' ').collect();
                let hits = words
                    .iter()
                    .filter(|w| target.iter().any(|n| n == *w || (w.chars().count() >= 4 && edit_distance(w, n) <= if w.chars().count() >= 7 { 2 } else { 1 })))
                    .count();
                if hits == words.len() {
                    40 + hits as i64
                } else if flat.chars().count() >= 5 && edit_distance(&flat, &name_flat) <= 2 {
                    35
                } else {
                    0
                }
            };
            if score > 0 && p["parent"].is_null() {
                score += 1;
            }
            if score > best_score {
                best_score = score;
                let mut b = p.clone();
                b["score"] = json!(score);
                best = Some(b);
            }
        }
        best
    }

    /// Projet qui contient un dossier (le plus précis).
    pub fn of_path(&self, dir: &str) -> Option<J> {
        if dir.is_empty() {
            return None;
        }
        let d = super::zones::resolve(dir).to_lowercase();
        let mut best: Option<(J, usize)> = None;
        for p in self.list() {
            let pp = super::zones::resolve(&js::gs(&p, "path")).to_lowercase();
            if (d == pp || d.starts_with(&format!("{pp}\\"))) && best.as_ref().map(|(_, l)| pp.len() > *l).unwrap_or(true) {
                best = Some((p, pp.len()));
            }
        }
        best.map(|(p, _)| p)
    }

    /// Détail d'un projet : description, technologies, git, dernière activité (cache 1 min).
    pub async fn info(&self, project: &J) -> J {
        let path = js::gs(project, "path");
        if let Some((at, info)) = self.infos.borrow().get(&path) {
            if js::now_ms() - at < INFO_TTL {
                return info.clone();
            }
        }
        let dir = PathBuf::from(&path);
        let mut description = String::new();
        let pkg = read_text(&dir.join("package.json"), 20000);
        if !pkg.is_empty() {
            description = serde_json::from_str::<J>(&pkg).map(|p| js::gs(&p, "description")).unwrap_or_default();
        }
        if description.is_empty() {
            for f in ["README.md", "CLAUDE.md", "README.txt"] {
                description = first_paragraph(&read_text(&dir.join(f), 20000));
                if !description.is_empty() {
                    break;
                }
            }
        }
        let name = js::gs(project, "name");
        let is_top = project["parent"].is_null();
        let subs: Vec<String> = if is_top { self.list().iter().filter(|p| js::gs(p, "parent") == name).map(|p| js::gs(p, "name")).collect() } else { vec![] };
        let mut info = json!({
            "name": name, "parent": project["parent"], "path": path, "description": description,
            "tech": detect_tech(&dir), "git": null, "lastActivity": last_modified(&dir), "subprojects": subs
        });
        if exists(&dir.join(".git")) {
            let (branch, last, status) = futures_util::join!(
                git(&path, &["rev-parse", "--abbrev-ref", "HEAD"]),
                git(&path, &["log", "-1", "--format=%ct%x09%s"]),
                git(&path, &["status", "--porcelain"])
            );
            let last = last.unwrap_or_default();
            let mut parts = last.splitn(2, '\t');
            let ts: i64 = parts.next().unwrap_or("").trim().parse().unwrap_or(0);
            let subject = parts.next().unwrap_or("").trim().to_string();
            let changes = status.map(|s| s.lines().filter(|l| !l.is_empty()).count()).unwrap_or(0);
            let at = ts * 1000;
            info["git"] = json!({ "branch": branch.unwrap_or_default(), "lastCommitAt": at, "lastCommit": subject, "changes": changes });
            if at > 0 {
                info["lastActivity"] = json!(info["lastActivity"].as_i64().unwrap_or(0).max(at));
            }
        }
        self.infos.borrow_mut().insert(path, (js::now_ms(), info.clone()));
        info
    }

    /// Projets triés du plus récemment actif au plus ancien.
    pub async fn recent(&self, limit: usize) -> Vec<J> {
        let tops: Vec<J> = self.list().into_iter().filter(|p| p["parent"].is_null() && !self.is_hidden(p)).collect();
        let mut infos = vec![];
        for p in &tops {
            infos.push(self.info(p).await);
        }
        infos.sort_by_key(|i| -i["lastActivity"].as_i64().unwrap_or(0));
        infos.into_iter().take(limit).collect()
    }

    /// Phrase parlée décrivant un projet.
    pub fn describe(&self, info: &J) -> String {
        let mut parts = vec![];
        let name = js::gs(info, "name");
        let label = if js::truthy(&info["parent"]) { format!("{name}, dans {}", js::gs(info, "parent")) } else { name };
        let desc = js::gs(info, "description");
        parts.push(if desc.is_empty() { format!("{label}.") } else { format!("{label} : {}.", crate::re!(r"[.\s]+$").replace(&desc, "")) });
        let tech = js::str_list(&info["tech"]);
        if !tech.is_empty() {
            parts.push(format!("Technologies : {}.", tech.join(", ")));
        }
        if js::truthy(&info["git"]) {
            let g = &info["git"];
            let at = g["lastCommitAt"].as_i64().unwrap_or(0);
            let branch = js::gs(g, "branch");
            if at > 0 {
                let lc = js::gs(g, "lastCommit");
                parts.push(format!("Dernier commit {}{}.", ago(at), if lc.is_empty() { String::new() } else { format!(" : {}", crate::re!(r"[.\s]+$").replace(&lc, "")) }));
            }
            let c = g["changes"].as_i64().unwrap_or(0);
            parts.push(if c > 0 {
                let s = if c > 1 { "s" } else { "" };
                format!("{c} fichier{s} modifié{s} non commité{s}{}.", if branch.is_empty() { String::new() } else { format!(" sur la branche {branch}") })
            } else {
                format!("Tout est commité{}.", if branch.is_empty() { String::new() } else { format!(" sur {branch}") })
            });
        } else {
            parts.push(format!("Dernière modification {}, pas de dépôt git.", ago(info["lastActivity"].as_i64().unwrap_or(0))));
        }
        let subs = js::str_list(&info["subprojects"]);
        if !subs.is_empty() {
            parts.push(format!("Il contient {}.", subs.join(", ")));
        }
        parts.join(" ")
    }

    /// Ligne ajoutée aux consignes de Claude Code et d'AGY.
    pub fn prompt_line(&self) -> String {
        let root = self.root();
        let tops: Vec<J> = self.list().into_iter().filter(|p| p["parent"].is_null() && !self.is_hidden(p)).collect();
        if tops.is_empty() {
            return String::new();
        }
        let fav: Vec<String> = tops.iter().filter(|p| self.is_favorite(p)).map(|p| js::gs(p, "name")).collect();
        let names: Vec<String> = tops.iter().filter(|p| !self.is_favorite(p)).map(|p| js::gs(p, "name")).collect();
        format!(
            "Projets de l'utilisateur (dossier {root}) : {}{}. Pour en parler, les comparer ou y travailler, appelle d'abord l'outil projets du serveur system (action lister ou infos).",
            if fav.is_empty() { String::new() } else { format!("favoris {} ; ", fav.join(", ")) },
            names.join(", ")
        )
    }

    /// Outil MCP « projets » (lecture seule).
    pub async fn tool(&self, p: &J) -> J {
        let root = self.root();
        if !Path::new(&root).exists() {
            return json!({ "ok": false, "error": format!("dossier des projets introuvable : {root}") });
        }
        let action = if js::gs(p, "action").is_empty() { "lister".to_string() } else { js::gs(p, "action") };
        if action == "infos" {
            let nom = js::gs(p, "nom");
            let Some(pr) = self.find(&nom) else {
                return json!({ "ok": false, "error": format!("aucun projet ne correspond à « {nom} »"), "projets": self.list().iter().filter(|x| x["parent"].is_null()).map(|x| x["name"].clone()).collect::<Vec<_>>() });
            };
            let info = self.info(&pr).await;
            let mut out = js::spread(&json!({ "ok": true }), &info);
            out["derniereActivite"] = json!(js::iso_of(info["lastActivity"].as_i64().unwrap_or(0)));
            out["resume"] = json!(self.describe(&info));
            return out;
        }
        let tous = p["tous"] == json!(true) || p["tous"] == json!("true");
        let all = self.list();
        let hidden_top: Vec<String> = all.iter().filter(|x| x["parent"].is_null() && self.is_hidden(x)).map(|x| js::gs(x, "name")).collect();
        let shown: Vec<J> = if tous { all.clone() } else { all.iter().filter(|x| !self.is_hidden(x) && !(js::truthy(&x["parent"]) && hidden_top.contains(&js::gs(x, "parent")))).cloned().collect() };
        let mut infos = vec![];
        for x in &shown {
            let mut i = self.info(x).await;
            i["favorite"] = json!(self.is_favorite(x));
            i["hidden"] = json!(self.is_hidden(x));
            infos.push(i);
        }
        infos.sort_by(|a, b| (js::gb(b, "favorite") as i64).cmp(&(js::gb(a, "favorite") as i64)).then(b["lastActivity"].as_i64().unwrap_or(0).cmp(&a["lastActivity"].as_i64().unwrap_or(0))));
        let projets: Vec<J> = infos
            .iter()
            .map(|i| {
                let mut v = json!({
                    "nom": i["name"], "chemin": i["path"], "description": i["description"], "technologies": i["tech"],
                    "derniereActivite": js::iso_of(i["lastActivity"].as_i64().unwrap_or(0))
                });
                if js::truthy(&i["parent"]) {
                    v["dans"] = i["parent"].clone();
                }
                if js::gb(i, "favorite") {
                    v["favori"] = json!(true);
                }
                if js::gb(i, "hidden") {
                    v["misDeCote"] = json!(true);
                }
                v
            })
            .collect();
        let mut out = json!({ "ok": true, "dossier": root, "projets": projets });
        if !tous && !hidden_top.is_empty() {
            out["misDeCote"] = json!(hidden_top);
            out["note"] = json!("projets mis de côté par l’utilisateur : ne les cite pas sauf s’il les demande (tous=true)");
        }
        out
    }
}

/// Claude Code dans une fenêtre de terminal ordinaire, ouverte dans le dossier du projet.
pub fn open_claude_window(dir: &str, name: &str, command: &str) {
    let cmd = if command.trim().is_empty() { "claude" } else { command.trim() };
    let title = format!("Claude · {name}");
    let mut args: Vec<&str> = vec!["/c", "start", &title, "/D", dir, "cmd", "/k"];
    args.extend(cmd.split_whitespace());
    crate::proc::detached_visible("cmd.exe", &args);
}

/// Claude Code dans une fenêtre PowerShell (pwsh s'il est installé), qui reste ouverte.
pub fn open_claude_powershell(dir: &str, name: &str, command: &str) {
    use base64::Engine;
    let exe = if crate::proc::which(&["pwsh.exe"], &[]).is_some() { "pwsh.exe" } else { "powershell.exe" };
    let quote = |s: &str| format!("'{}'", s.replace('\'', "''"));
    let cmd = if command.trim().is_empty() { "claude" } else { command.trim() };
    let script = format!("$Host.UI.RawUI.WindowTitle = {}; {cmd}", quote(&format!("Claude · {name}")));
    let utf16: Vec<u8> = script.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    let encoded = base64::engine::general_purpose::STANDARD.encode(utf16);
    let title = format!("Claude · {name}");
    crate::proc::detached_visible("cmd.exe", &["/c", "start", &title, "/D", dir, exe, "-NoLogo", "-NoExit", "-EncodedCommand", &encoded]);
}

#[allow(dead_code)]
fn _v(_: Value) {}
