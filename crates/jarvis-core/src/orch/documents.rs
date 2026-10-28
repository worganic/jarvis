// RAG des documents locaux : l'utilisateur désigne des dossiers (Administration › Documents) ; Jarvis lit les fichiers
// texte, Markdown, CSV, Word (.docx) et PDF qu'ils contiennent, les découpe en passages, et l'IA les retrouve par le
// sens avec l'outil documents_chercher (lecture seule). Même moteur que les listes à fiches (collections/index.rs) :
// vecteurs en cache à côté (data/documents/vectors.json), clés = empreinte du texte, un passage inchangé n'est jamais
// recalculé. Le journal garde la trace des recherches, jamais le contenu des documents.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::UNIX_EPOCH;

use serde_json::{json, Value};

use super::collections::index::{Index, Passage};
use super::zones::{resolve, Zones};
use super::OrchRef;
use crate::js::{self, J};

const MAX_FOLDERS: usize = 10;
const MAX_FILES: usize = 3000;
const MAX_FILE_BYTES: u64 = 25 * 1024 * 1024;
const MAX_TEXT_CHARS: usize = 2_000_000;
const MAX_CHUNKS_PER_FILE: usize = 300;
const MAX_CHUNKS: usize = 8000;
const CHUNK_CHARS: usize = 900;
const SKIP_DIRS: &[&str] = &["node_modules", ".git", ".venv", "__pycache__", "appdata", "$recycle.bin", ".cache", "cache", "target"];
const EXTENSIONS: &[&str] = &["txt", "md", "markdown", "csv", "log", "docx", "pdf"];

#[derive(Clone, Default)]
struct FileEntry {
    mtime: u64,
    len: u64,
    chunks: Vec<String>,
}

pub struct Documents {
    o: OrchRef,
    dir: PathBuf,
    log: Rc<dyn Fn(String)>,
    pub index: Index,
    folders: RefCell<Vec<String>>,
    files: RefCell<HashMap<String, FileEntry>>,
    busy: Cell<bool>,
    last_scan: RefCell<String>,
}

// --- Lecture des fichiers ----------------------------------------------------------------------

fn decode_entities(s: &str) -> String {
    s.replace("&lt;", "<").replace("&gt;", ">").replace("&quot;", "\"").replace("&apos;", "'").replace("&amp;", "&")
}

/// Texte d'un document Word : paragraphes du word/document.xml.
pub fn docx_text(path: &Path) -> Result<String, String> {
    let file = std::fs::File::open(path).map_err(|e| e.to_string())?;
    let mut zip = zip::ZipArchive::new(file).map_err(|e| e.to_string())?;
    let mut xml = String::new();
    zip.by_name("word/document.xml").map_err(|e| e.to_string())?.read_to_string(&mut xml).map_err(|e| e.to_string())?;
    Ok(xml_to_text(&xml))
}

fn xml_to_text(xml: &str) -> String {
    let t = crate::re!(r"</w:p>|<w:br\s*/>|<w:cr\s*/>").replace_all(xml, "\n");
    let t = crate::re!(r"<w:tab\s*/>").replace_all(&t, " ");
    let t = crate::re!(r"<[^>]+>").replace_all(&t, "");
    decode_entities(&t)
}

fn pdf_text(path: &Path) -> Result<String, String> {
    // Certains PDF font paniquer l'extracteur : on ne laisse jamais cela atteindre le noyau.
    let p = path.to_path_buf();
    match std::panic::catch_unwind(move || pdf_extract::extract_text(&p)) {
        Ok(Ok(t)) => Ok(t),
        Ok(Err(e)) => Err(e.to_string()),
        Err(_) => Err("PDF illisible".into()),
    }
}

fn plain_text(path: &Path) -> Result<String, String> {
    let bytes = std::fs::read(path).map_err(|e| e.to_string())?;
    Ok(match String::from_utf8(bytes) {
        Ok(s) => s,
        Err(e) => encoding_rs::WINDOWS_1252.decode(e.as_bytes()).0.into_owned(),
    })
}

pub fn extract(path: &Path) -> Result<String, String> {
    let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("").to_lowercase();
    let text = match ext.as_str() {
        "docx" => docx_text(path)?,
        "pdf" => pdf_text(path)?,
        _ => plain_text(path)?,
    };
    Ok(js::slice_chars(&text, MAX_TEXT_CHARS))
}

/// Découpe un texte en passages d'environ CHUNK_CHARS caractères, sur les fins de paragraphe quand c'est possible.
pub fn chunk(title: &str, text: &str) -> Vec<String> {
    let mut chunks: Vec<String> = vec![];
    let mut cur = String::new();
    let flush = |cur: &mut String, chunks: &mut Vec<String>| {
        let body = cur.trim();
        if body.chars().count() >= 20 {
            chunks.push(format!("« {title} » : {body}"));
        }
        cur.clear();
    };
    for para in text.split('\n').map(|p| crate::re!(r"\s+").replace_all(p, " ").trim().to_string()).filter(|p| !p.is_empty()) {
        // Un très long paragraphe est coupé en tranches.
        let pieces: Vec<String> = if para.chars().count() > CHUNK_CHARS * 3 / 2 {
            para.chars().collect::<Vec<_>>().chunks(CHUNK_CHARS).map(|c| c.iter().collect()).collect()
        } else {
            vec![para]
        };
        for piece in pieces {
            if cur.chars().count() + piece.chars().count() > CHUNK_CHARS && !cur.is_empty() {
                flush(&mut cur, &mut chunks);
            }
            if !cur.is_empty() {
                cur.push(' ');
            }
            cur.push_str(&piece);
        }
        if chunks.len() >= MAX_CHUNKS_PER_FILE {
            return chunks;
        }
    }
    flush(&mut cur, &mut chunks);
    chunks.truncate(MAX_CHUNKS_PER_FILE);
    chunks
}

fn mtime_of(md: &std::fs::Metadata) -> u64 {
    md.modified().ok().and_then(|t| t.duration_since(UNIX_EPOCH).ok()).map(|d| d.as_secs()).unwrap_or(0)
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>, depth: usize) {
    if depth > 8 || out.len() >= MAX_FILES {
        return;
    }
    let Ok(rd) = std::fs::read_dir(dir) else { return };
    for e in rd.flatten() {
        let p = e.path();
        let name = e.file_name().to_string_lossy().to_lowercase();
        let Ok(ft) = e.file_type() else { continue };
        if ft.is_dir() {
            if !name.starts_with('.') && !SKIP_DIRS.contains(&name.as_str()) {
                walk(&p, out, depth + 1);
            }
        } else if ft.is_file() && !name.starts_with("~$") {
            let ext = p.extension().and_then(|x| x.to_str()).unwrap_or("").to_lowercase();
            if EXTENSIONS.contains(&ext.as_str()) {
                out.push(p);
            }
        }
        if out.len() >= MAX_FILES {
            return;
        }
    }
}

// --- Le service -----------------------------------------------------------------------------------

impl Documents {
    pub fn new(o: OrchRef, dir: PathBuf, log: Rc<dyn Fn(String)>) -> Self {
        let _ = std::fs::create_dir_all(&dir);
        let folders = js::read_json(&dir.join("dossiers.json")).map(|v| js::str_list(&v["dossiers"])).unwrap_or_default();
        let mut files = HashMap::new();
        if let Some(Value::Object(m)) = js::read_json(&dir.join("fichiers.json")) {
            for (path, v) in m {
                files.insert(path, FileEntry { mtime: v["mtime"].as_u64().unwrap_or(0), len: v["len"].as_u64().unwrap_or(0), chunks: js::str_list(&v["chunks"]) });
            }
        }
        Self {
            o,
            index: Index::new(dir.join("vectors.json")),
            dir,
            log,
            folders: RefCell::new(folders),
            files: RefCell::new(files),
            busy: Cell::new(false),
            last_scan: RefCell::new(String::new()),
        }
    }

    fn save_folders(&self) {
        let _ = js::write_json(&self.dir.join("dossiers.json"), &json!({ "dossiers": self.folders.borrow().clone() }));
    }

    fn save_files(&self) {
        let mut m = serde_json::Map::new();
        for (p, f) in self.files.borrow().iter() {
            m.insert(p.clone(), json!({ "mtime": f.mtime, "len": f.len, "chunks": f.chunks }));
        }
        let _ = js::write_json_atomic(&self.dir.join("fichiers.json"), &Value::Object(m));
    }

    pub fn folders(&self) -> Vec<String> {
        self.folders.borrow().clone()
    }

    /// Ajoute un dossier à lire. Les zones interdites en lecture sont refusées.
    pub fn add_folder(&self, path: &str, zones: &Zones) -> Result<J, String> {
        let resolved = resolve(path.trim());
        if resolved.is_empty() || !Path::new(&resolved).is_dir() {
            return Err("Ce dossier n'existe pas.".into());
        }
        if zones.check(&resolved, "read") == "forbidden" {
            return Err("Ce dossier est dans une zone interdite en lecture.".into());
        }
        {
            let mut f = self.folders.borrow_mut();
            if f.iter().any(|x| x.eq_ignore_ascii_case(&resolved)) {
                return Err("Ce dossier est déjà suivi.".into());
            }
            if f.len() >= MAX_FOLDERS {
                return Err(format!("{MAX_FOLDERS} dossiers au plus."));
            }
            f.push(resolved);
        }
        self.save_folders();
        Ok(self.state())
    }

    pub fn remove_folder(&self, path: &str) -> J {
        let target = resolve(path.trim()).to_lowercase();
        self.folders.borrow_mut().retain(|f| f.to_lowercase() != target);
        self.save_folders();
        let keep = self.folders.borrow().iter().map(|f| f.to_lowercase()).collect::<Vec<_>>();
        self.files.borrow_mut().retain(|p, _| keep.iter().any(|f| p.to_lowercase().starts_with(f.as_str())));
        self.save_files();
        self.state()
    }

    /// Relit les dossiers : seuls les fichiers nouveaux ou modifiés sont ré-extraits, les disparus sont oubliés.
    pub async fn scan(&self) -> Result<J, String> {
        if self.busy.replace(true) {
            return Err("Une analyse est déjà en cours.".into());
        }
        let res = self.scan_inner().await;
        self.busy.set(false);
        res
    }

    async fn scan_inner(&self) -> Result<J, String> {
        let mut paths: Vec<PathBuf> = vec![];
        for f in self.folders() {
            walk(Path::new(&f), &mut paths, 0);
        }
        let (mut read, mut failed) = (0usize, 0usize);
        let mut seen: Vec<String> = Vec::with_capacity(paths.len());
        let mut total: usize = self.files.borrow().values().map(|f| f.chunks.len()).sum();
        for p in paths {
            let key = p.to_string_lossy().to_string();
            seen.push(key.clone());
            let Ok(md) = std::fs::metadata(&p) else { continue };
            if md.len() > MAX_FILE_BYTES {
                continue;
            }
            let (mtime, len) = (mtime_of(&md), md.len());
            if self.files.borrow().get(&key).map(|f| f.mtime == mtime && f.len == len).unwrap_or(false) {
                continue;
            }
            if total >= MAX_CHUNKS {
                break;
            }
            let title = p.file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
            match extract(&p) {
                Ok(text) => {
                    let chunks = chunk(&title, &text);
                    total += chunks.len();
                    self.files.borrow_mut().insert(key, FileEntry { mtime, len, chunks });
                    read += 1;
                }
                Err(e) => {
                    (self.log)(format!("Documents : {title} illisible ({e})"));
                    self.files.borrow_mut().insert(key, FileEntry { mtime, len, chunks: vec![] });
                    failed += 1;
                }
            }
            js::sleep_ms(1).await; // laisse respirer le noyau entre deux fichiers
        }
        self.files.borrow_mut().retain(|p, _| seen.contains(p));
        self.save_files();
        *self.last_scan.borrow_mut() = js::iso_now();
        let passages = self.passages();
        let computed = self.index.ensure(&passages).await.unwrap_or(0);
        Ok(json!({ "ok": true, "lus": read, "illisibles": failed, "passages": passages.len(), "vecteursCalcules": computed }))
    }

    /// Lance l'analyse en tâche de fond (administration).
    pub fn scan_soon(&self) -> bool {
        let Some(o) = self.o.upgrade() else { return false };
        if self.busy.get() {
            return false;
        }
        js::spawn(async move {
            if let Err(e) = o.documents.scan().await {
                (o.documents.log)(format!("Documents : {e}"));
            }
            o.emit(json!({ "type": "documents" }));
        });
        true
    }

    fn passages(&self) -> Vec<Passage> {
        let files = self.files.borrow();
        let mut keys: Vec<&String> = files.keys().collect();
        keys.sort();
        keys.into_iter()
            .flat_map(|path| files[path].chunks.iter().enumerate().map(move |(i, t)| Passage { list: "documents".into(), element: format!("{path}#{i}"), text: t.clone(), ids: vec![] }))
            .take(MAX_CHUNKS)
            .collect()
    }

    /// Les passages les plus proches de la question : { fichier, chemin, extrait, score }.
    pub async fn search(&self, query: &str, limit: usize) -> Vec<J> {
        let passages = self.passages();
        let hits = self.index.search(query, &passages, limit.clamp(1, 12)).await;
        let by_id: HashMap<&str, &Passage> = passages.iter().map(|p| (p.element.as_str(), p)).collect();
        hits.iter()
            .filter_map(|h| {
                let p = by_id.get(h.element.as_str())?;
                let path = h.element.rsplit_once('#').map(|(p, _)| p).unwrap_or(&h.element);
                let name = Path::new(path).file_name().map(|n| n.to_string_lossy().to_string()).unwrap_or_default();
                Some(json!({ "fichier": name, "chemin": path, "extrait": js::slice_chars(&p.text, 900), "score": (h.score * 100.0).round() / 100.0 }))
            })
            .collect()
    }

    /// Texte renvoyé à l'IA par documents_chercher.
    pub async fn ai_search_text(&self, query: &str, limit: usize) -> String {
        if self.folders.borrow().is_empty() {
            return "Aucun dossier de documents n'est suivi : l'utilisateur doit en ajouter un dans Administration › Documents.".into();
        }
        let rows = self.search(query, limit).await;
        if rows.is_empty() {
            return "Aucun passage de document ne correspond.".into();
        }
        rows.iter().map(|r| format!("### {} ({})\n{}", js::gs(r, "fichier"), js::gs(r, "chemin"), js::gs(r, "extrait"))).collect::<Vec<_>>().join("\n\n")
    }

    /// Chiffres et réglages seulement (jamais le contenu) : page d'aide et administration.
    pub fn stats(&self) -> J {
        let files = self.files.borrow();
        let passages: usize = files.values().map(|f| f.chunks.len()).sum();
        let mut s = self.index.stats(passages);
        s["dossiers"] = json!(self.folders.borrow().len());
        s["fichiers"] = json!(files.len());
        s["analyse"] = json!(self.busy.get());
        s["derniereAnalyse"] = json!(self.last_scan.borrow().clone());
        s["extensions"] = json!(EXTENSIONS);
        s
    }

    pub fn state(&self) -> J {
        let files = self.files.borrow();
        let per_folder: Vec<J> = self
            .folders
            .borrow()
            .iter()
            .map(|f| {
                let low = f.to_lowercase();
                let n = files.keys().filter(|p| p.to_lowercase().starts_with(&low)).count();
                json!({ "chemin": f, "fichiers": n, "existe": Path::new(f).is_dir() })
            })
            .collect();
        json!({ "ok": true, "dossiers": per_folder, "stats": self.stats() })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn xml_word_en_texte() {
        let xml = "<w:p><w:r><w:t>Bonjour &amp; bienvenue</w:t></w:r></w:p><w:p><w:r><w:t>Deuxi\u{e8}me</w:t></w:r><w:tab/><w:r><w:t>ligne</w:t></w:r></w:p>";
        assert_eq!(xml_to_text(xml), "Bonjour & bienvenue\nDeuxi\u{e8}me ligne\n");
    }

    #[test]
    fn decoupage_en_passages() {
        let para = "Une phrase assez longue pour remplir un paragraphe de texte. ".repeat(8);
        let text = format!("{para}\n\n{para}\n{para}\ncourt");
        let chunks = chunk("contrat.pdf", &text);
        assert!(chunks.len() >= 2, "{}", chunks.len());
        assert!(chunks.iter().all(|c| c.starts_with("\u{ab} contrat.pdf \u{bb} : ")));
        assert!(chunks.iter().all(|c| c.chars().count() < CHUNK_CHARS + 200));
        assert!(chunk("vide.txt", "  \n \n").is_empty());
    }

    #[test]
    fn dossier_lu_de_bout_en_bout() {
        let dir = std::env::temp_dir().join(format!("jarvis-test-documents-{}", js::now_ms()));
        let sub = dir.join("notes");
        std::fs::create_dir_all(&sub).unwrap();
        std::fs::write(sub.join("recette.md"), "# Tarte aux pommes\n\nEplucher les pommes, puis les couper en fines lamelles avant de les disposer sur la p\u{e2}te.").unwrap();
        std::fs::write(dir.join("image.png"), b"pas du texte").unwrap();
        let mut found = vec![];
        walk(&dir, &mut found, 0);
        assert_eq!(found.len(), 1, "{found:?}");
        let text = extract(&found[0]).unwrap();
        assert!(text.contains("lamelles"));
        let _ = std::fs::remove_dir_all(dir);
    }
}
