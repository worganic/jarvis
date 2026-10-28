// Système unifié de journalisation et de traçabilité des erreurs Jarvis (orchestrator/logger.js).
// Centralise les journaux de l'application, les erreurs et les diagnostics système.

use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

use serde_json::{json, Value};

const MAX_LOG_SIZE: u64 = 5 * 1024 * 1024; // 5 Mo avant rotation

pub struct Logger {
    log_dir: PathBuf,
    app_log: PathBuf,
    error_log: PathBuf,
    // fichier → écritures faites (la rotation n'est testée qu'une fois sur 50)
    write_counts: Mutex<HashMap<PathBuf, u64>>,
}

/// Complément d'une ligne de journal : erreur (pile) ou détails JSON.
pub enum Extra<'a> {
    None,
    Error(&'a str),
    Details(&'a Value),
}

impl Logger {
    fn new(log_dir: PathBuf) -> Self {
        let l = Self {
            app_log: log_dir.join("jarvis.log"),
            error_log: log_dir.join("errors.log"),
            log_dir,
            write_counts: Mutex::new(HashMap::new()),
        };
        l.ensure_dir();
        l
    }

    fn ensure_dir(&self) {
        let _ = std::fs::create_dir_all(&self.log_dir);
    }

    fn rotate(file: &Path) {
        if let Ok(meta) = std::fs::metadata(file) {
            if meta.len() > MAX_LOG_SIZE {
                let backup = PathBuf::from(format!("{}.1", file.display()));
                let _ = std::fs::remove_file(&backup);
                let _ = std::fs::rename(file, backup);
            }
        }
    }

    fn format_time() -> String {
        chrono::Local::now().format("%Y-%m-%d %H:%M:%S%.3f").to_string()
    }

    fn write(&self, file: &Path, level: &str, source: &str, message: &str, extra: &Extra) {
        let count = {
            let mut counts = self.write_counts.lock().unwrap();
            let c = counts.entry(file.to_path_buf()).or_insert(0);
            let n = *c;
            *c += 1;
            n
        };
        if count % 50 == 0 {
            self.ensure_dir();
            Self::rotate(file);
        }
        let mut text = format!("[{}] [{}] [{}] {}\n", Self::format_time(), level.to_uppercase(), source, message);
        match extra {
            Extra::None => {}
            Extra::Error(stack) => text.push_str(&format!("  Stack: {stack}\n")),
            Extra::Details(v) => {
                if !v.is_null() {
                    text.push_str(&format!("  Détails: {}\n", serde_json::to_string_pretty(v).unwrap_or_default()));
                }
            }
        }
        let append = |f: &Path| -> std::io::Result<()> {
            let mut h = std::fs::OpenOptions::new().create(true).append(true).open(f)?;
            h.write_all(text.as_bytes())
        };
        if append(file).is_err() {
            self.ensure_dir();
            let _ = append(file);
        }
    }

    pub fn info(&self, source: &str, message: &str) {
        self.write(&self.app_log, "INFO", source, message, &Extra::None);
    }

    pub fn info_x(&self, source: &str, message: &str, extra: Extra) {
        self.write(&self.app_log, "INFO", source, message, &extra);
    }

    pub fn warn(&self, source: &str, message: &str) {
        self.warn_x(source, message, Extra::None);
    }

    pub fn warn_x(&self, source: &str, message: &str, extra: Extra) {
        self.write(&self.app_log, "WARN", source, message, &extra);
        self.write(&self.error_log, "WARN", source, message, &extra);
    }

    pub fn error(&self, source: &str, message: &str) {
        self.error_x(source, message, Extra::None);
    }

    pub fn error_x(&self, source: &str, message: &str, extra: Extra) {
        self.write(&self.app_log, "ERROR", source, message, &extra);
        self.write(&self.error_log, "ERROR", source, message, &extra);
    }

    pub fn critical(&self, source: &str, message: &str) {
        self.write(&self.app_log, "CRITICAL", source, message, &Extra::None);
        self.write(&self.error_log, "CRITICAL", source, message, &Extra::None);
    }

    // Lit seulement la fin d'un fichier (les erreurs récentes n'ont pas besoin des 5 Mo du journal).
    fn read_tail(file: &Path, max_bytes: u64) -> std::io::Result<String> {
        let mut f = std::fs::File::open(file)?;
        let size = f.metadata()?.len();
        let start = size.saturating_sub(max_bytes);
        f.seek(SeekFrom::Start(start))?;
        let mut buf = Vec::new();
        f.read_to_end(&mut buf)?;
        let text = String::from_utf8_lossy(&buf).into_owned();
        Ok(if start > 0 {
            match text.find('\n') {
                Some(i) => text[i + 1..].to_string(),
                None => String::new(),
            }
        } else {
            text
        })
    }

    /// Les N dernières erreurs, sous forme de liste structurée (la plus récente d'abord).
    pub fn recent_errors(&self, limit: usize) -> Vec<Value> {
        let Ok(content) = Self::read_tail(&self.error_log, 256 * 1024) else { return vec![] };
        let head = crate::re!(r"^\[(.*?)\] \[(ERROR|CRITICAL|WARN)\] \[(.*?)\] (.*)$");
        let mut entries: Vec<Value> = Vec::new();
        let mut current: Option<Value> = None;
        for line in content.split('\n') {
            let line = line.trim_end_matches('\r');
            if let Some(m) = head.captures(line) {
                if let Some(c) = current.take() {
                    entries.push(c);
                }
                current = Some(json!({
                    "time": &m[1], "level": &m[2], "source": &m[3], "message": &m[4], "details": []
                }));
            } else if let Some(c) = current.as_mut() {
                if !line.trim().is_empty() {
                    c["details"].as_array_mut().unwrap().push(json!(line.trim()));
                }
            }
        }
        if let Some(c) = current {
            entries.push(c);
        }
        let skip = entries.len().saturating_sub(limit);
        entries.into_iter().skip(skip).rev().collect()
    }

    /// Résumé en texte clair pour affichage ou commande vocale.
    pub fn summary_sentence(&self) -> String {
        let errors = self.recent_errors(5);
        if errors.is_empty() {
            return "Aucune erreur n'est enregistrée dans le journal. Tous les systèmes fonctionnent nominalement.".into();
        }
        let critical = errors.iter().filter(|e| matches!(e["level"].as_str(), Some("CRITICAL") | Some("ERROR"))).count();
        let latest = &errors[0];
        format!(
            "Il y a {} erreur(s) récente(s). Dernière erreur sur {} : {}.",
            critical,
            latest["source"].as_str().unwrap_or(""),
            latest["message"].as_str().unwrap_or("")
        )
    }

    pub fn clear_errors(&self) -> bool {
        if self.error_log.exists() {
            std::fs::write(&self.error_log, "").is_ok()
        } else {
            true
        }
    }
}

static GLOBAL: OnceLock<Logger> = OnceLock::new();

/// Journal partagé (le premier appel fixe le dossier).
pub fn init(log_dir: PathBuf) -> &'static Logger {
    GLOBAL.get_or_init(|| Logger::new(log_dir))
}

pub fn logger() -> &'static Logger {
    GLOBAL.get_or_init(|| Logger::new(crate::paths::data_dir().join("logs")))
}
