// Recherche locale de fichiers et de dossiers (orchestrator/file-search.js), sans IA.
// Portée : les zones déclarées lisibles, SAUF la zone Projets (E:\www). Moteur : parcours du disque
// borné, puis index Windows Search s'il a été écourté. Seuls les noms sont comparés.

use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::time::{Duration, Instant};

use serde_json::json;

use super::zones::resolve;
use super::{OrchRef, Up};
use crate::js::{self, J};

pub fn is_executable(path: &str) -> bool {
    crate::re!(r"(?i)\.(?:exe|bat|cmd|com|ps1|psm1|vbs|vbe|js|jse|wsf|wsh|msi|msp|scr|hta|jar|py|pyw|lnk|reg|cpl|dll|sys)$").is_match(path)
}

const SKIP_DIRS: &[&str] = &["node_modules", ".git", ".venv", "__pycache__", "appdata", "$recycle.bin", ".cache", "cache"];

fn fold(text: &str) -> String {
    js::fold_lower(text)
}

/// Dossiers personnels connus : « ouvre mes téléchargements ».
pub fn known_folder(name: &str) -> Option<PathBuf> {
    let home = js::home_dir();
    Some(home.join(match name {
        "bureau" => "Desktop",
        "documents" => "Documents",
        "telechargements" => "Downloads",
        "images" | "photos" => "Pictures",
        "musique" => "Music",
        "videos" => "Videos",
        _ => return None,
    }))
}

pub const KNOWN_FOLDERS: &[&str] = &["bureau", "documents", "telechargements", "images", "photos", "musique", "videos"];

/// PowerShell en -EncodedCommand (les blocs try/catch sur plusieurs lignes restent entiers).
async fn run_powershell_encoded(script: &str, timeout_ms: u64) -> Option<String> {
    use base64::Engine;
    let utf16: Vec<u8> = script.encode_utf16().flat_map(|u| u.to_le_bytes()).collect();
    let encoded = base64::engine::general_purpose::STANDARD.encode(utf16);
    let args: Vec<String> = ["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-EncodedCommand", &encoded].iter().map(|s| s.to_string()).collect();
    let o = crate::proc::run("powershell.exe", &args, None, timeout_ms).await.ok()?;
    (o.code == 0).then_some(o.stdout)
}

pub struct FileSearch {
    o: OrchRef,
    log: Rc<dyn Fn(String)>,
}

impl FileSearch {
    pub fn new(o: OrchRef, log: Rc<dyn Fn(String)>) -> Self {
        Self { o, log }
    }

    pub fn scopes(&self) -> Vec<String> {
        let all = self.o.up().zones.list();
        let excluded: Vec<String> = all.iter().filter(|z| js::gs(z, "id") == "projects").map(|z| resolve(&js::gs(z, "path")).to_lowercase()).collect();
        let within = |p: &str, root: &str| p == root || p.starts_with(&format!("{root}\\"));
        all.iter()
            .filter(|z| js::gs(z, "read") != "forbidden" && js::gs(z, "id") != "projects")
            .map(|z| resolve(&js::gs(z, "path")))
            .filter(|p| {
                let low = p.to_lowercase();
                let in_projects = excluded.iter().any(|r| within(&low, r));
                !in_projects || crate::re!(r"(?i)\\jarvis\\workspace$").is_match(&low)
            })
            .filter(|p| Path::new(p).exists())
            .collect()
    }

    /// kind : file | folder | any. Jusqu'à `limit` chemins, les plus récents d'abord.
    pub async fn search(&self, query: &str, kind: &str, limit: usize) -> Vec<String> {
        let q = crate::re!(r"[^a-z0-9 ._-]+").replace_all(&fold(query), " ").into_owned();
        let words: Vec<String> = q.split_whitespace().filter(|w| w.chars().count() >= 2).take(5).map(String::from).collect();
        if words.is_empty() {
            return vec![];
        }
        let scopes = self.scopes();
        if scopes.is_empty() {
            return vec![];
        }
        let (results, complete) = {
            let (w, s, k) = (words.clone(), scopes.clone(), kind.to_string());
            tokio::task::spawn_blocking(move || walk(&w, &s, &k, limit, 1500)).await.unwrap_or((vec![], true))
        };
        if complete {
            return results;
        }
        let indexed = self.windows_search(&words, &scopes, kind, limit).await;
        if indexed.is_none() {
            (self.log)("[fichiers] index Windows Search indisponible : résultats partiels du parcours".into());
        }
        match indexed {
            Some(i) if !i.is_empty() => i,
            _ => results,
        }
    }

    async fn windows_search(&self, words: &[String], scopes: &[String], kind: &str, limit: usize) -> Option<Vec<String>> {
        let esc = |s: &str| s.replace('\'', "''");
        let scope_sql = scopes.iter().map(|p| format!("SCOPE='file:{}'", esc(&p.replace('\\', "/")))).collect::<Vec<_>>().join(" OR ");
        let name_sql = words.iter().map(|w| format!("System.FileName LIKE '%{}%'", esc(w))).collect::<Vec<_>>().join(" AND ");
        let kind_sql = match kind {
            "folder" => " AND System.ItemType = 'Directory'",
            "file" => " AND System.ItemType <> 'Directory'",
            _ => "",
        };
        let sql = format!("SELECT TOP {} System.ItemUrl, System.DateModified FROM SYSTEMINDEX WHERE ({scope_sql}) AND {name_sql}{kind_sql} ORDER BY System.DateModified DESC", limit * 2);
        let script = format!(
            "[Console]::OutputEncoding = [System.Text.Encoding]::UTF8\ntry {{\n  $c = New-Object -ComObject ADODB.Connection\n  $c.Open(\"Provider=Search.CollatorDSO;Extended Properties='Application=Windows';\")\n  $rs = New-Object -ComObject ADODB.Recordset\n  $rs.Open(\"{}\", $c)\n  $out = @()\n  while (-not $rs.EOF) {{ $out += [string]$rs.Fields.Item('System.ItemUrl').Value; $rs.MoveNext() }}\n  $rs.Close(); $c.Close()\n  \"OK\"\n  $out\n}} catch {{ \"ERR\" }}\n",
            sql.replace('"', "\"\"")
        );
        let out = run_powershell_encoded(&script, 8000).await?;
        let lines: Vec<String> = out.replace('\u{feff}', "").lines().map(|l| l.trim().to_string()).filter(|l| !l.is_empty()).collect();
        if lines.first().map(|l| l.as_str()) != Some("OK") {
            return None;
        }
        let paths: Vec<String> = lines[1..]
            .iter()
            .map(|u| {
                let p = crate::re!(r"(?i)^file:/*").replace(u, "").into_owned();
                let p = urlencoding::decode(&p).map(|c| c.into_owned()).unwrap_or(p);
                p.replace('/', "\\")
            })
            .collect();
        Some(filter(&paths, scopes).into_iter().take(limit).collect())
    }

    /// Ouvre un résultat : document avec son application, dossier dans l'Explorateur, exécutable
    /// ou script jamais lancé (son dossier est affiché, fichier sélectionné).
    pub fn open(&self, target: &str) -> Result<J, String> {
        let resolved = resolve(target);
        let p = Path::new(&resolved);
        if !p.exists() {
            return Err(format!("introuvable : {resolved}"));
        }
        let is_dir = p.is_dir();
        let exe = !is_dir && is_executable(&resolved);
        if exe {
            crate::proc::detached("explorer.exe", &[&format!("/select,{resolved}")]);
        } else {
            crate::proc::detached("explorer.exe", &[&resolved]);
        }
        Ok(json!({
            "ok": true, "dossier": is_dir, "executable": exe,
            "message": if is_dir { format!("Dossier ouvert : {resolved}") } else { format!("Fichier ouvert : {resolved}") }
        }))
    }
}

/// Parcours borné (profondeur 6, 30 000 entrées, budget en ms). Le booléen : tout a été vu.
fn walk(words: &[String], scopes: &[String], kind: &str, limit: usize, budget_ms: u64) -> (Vec<String>, bool) {
    let deadline = Instant::now() + Duration::from_millis(budget_ms);
    fn visit(dir: &Path, depth: u32, ctx: &mut (Vec<PathBuf>, usize, bool), words: &[String], kind: &str, limit: usize, deadline: Instant) {
        if depth > 6 {
            return;
        }
        if ctx.1 > 30000 || Instant::now() > deadline || ctx.0.len() >= limit * 3 {
            ctx.2 = false;
            return;
        }
        let Ok(items) = std::fs::read_dir(dir) else { return };
        for item in items.flatten() {
            ctx.1 += 1;
            let full = item.path();
            let is_dir = item.file_type().map(|t| t.is_dir()).unwrap_or(false);
            let name = fold(&item.file_name().to_string_lossy());
            if words.iter().all(|w| name.contains(w.as_str())) && (kind == "any" || (kind == "folder") == is_dir) {
                ctx.0.push(full.clone());
            }
            if is_dir && !SKIP_DIRS.contains(&name.as_str()) && !name.starts_with('.') {
                visit(&full, depth + 1, ctx, words, kind, limit, deadline);
            }
        }
    }
    let mut ctx: (Vec<PathBuf>, usize, bool) = (vec![], 0, true);
    for root in scopes {
        visit(Path::new(root), 0, &mut ctx, words, kind, limit, deadline);
    }
    let (found, _, complete) = ctx;
    let mut with_time: Vec<(PathBuf, i64)> = found
        .into_iter()
        .map(|p| {
            let t = std::fs::metadata(&p).and_then(|m| m.modified()).ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_millis() as i64).unwrap_or(0);
            (p, t)
        })
        .collect();
    with_time.sort_by_key(|(_, t)| -t);
    let paths: Vec<String> = with_time.into_iter().map(|(p, _)| p.to_string_lossy().into_owned()).collect();
    (filter(&paths, scopes).into_iter().take(limit).collect(), complete)
}

/// Dernier contrôle : chaque résultat doit rester dans une portée autorisée.
fn filter(paths: &[String], scopes: &[String]) -> Vec<String> {
    let roots: Vec<String> = scopes.iter().map(|p| p.to_lowercase()).collect();
    let mut seen = HashSet::new();
    paths
        .iter()
        .filter(|p| {
            let low = resolve(p).to_lowercase();
            if !seen.insert(low.clone()) {
                return false;
            }
            roots.iter().any(|r| low == *r || low.starts_with(&format!("{r}\\")))
        })
        .cloned()
        .collect()
}
