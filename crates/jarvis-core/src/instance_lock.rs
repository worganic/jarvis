// Un seul noyau Jarvis à la fois (core/instance-lock.js de la v3).
//
// Deux verrous, tenus ouverts toute la vie du processus en mode exclusif Windows (aucun partage) :
// un second processus ne peut pas les ouvrir, et le système les libère de lui-même si le noyau
// s'arrête — même en plantant : jamais de verrou « périmé » à nettoyer.
//   - par dossier de données (data/runtime/noyau.lock) : toujours ;
//   - pour la session Windows (%TEMP%/jarvis-noyau.lock) : le « vrai » Jarvis, quel que soit son
//     dossier (v3, v4…), hors instances de test qui ont leurs propres données temporaires.
// Un fichier voisin (.json) dit qui tient le verrou, pour le message de refus.

use std::fs::File;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::js::{self, J};

pub struct InstanceLock {
    held: Vec<(File, PathBuf)>,
}

enum Try {
    Held(File),
    Busy(Option<J>),
    Error(String),
}

fn try_lock(file: &Path, info: &J) -> Try {
    use std::os::windows::fs::OpenOptionsExt;
    if let Some(dir) = file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    match std::fs::OpenOptions::new().read(true).write(true).create(true).truncate(false).share_mode(0).open(file) {
        Ok(f) => {
            let _ = std::fs::write(format!("{}.json", file.display()), js::pretty(info));
            Try::Held(f)
        }
        // ERROR_SHARING_VIOLATION (32) : un autre processus le tient.
        Err(e) if e.raw_os_error() == Some(32) => Try::Busy(js::read_json(Path::new(&format!("{}.json", file.display())))),
        Err(e) => Try::Error(e.to_string()),
    }
}

impl InstanceLock {
    /// Attente courte : pendant une relance, l'ancien noyau finit de s'arrêter ; son verrou se libère alors.
    pub fn acquire(data_dir: &Path, global: bool, wait_ms: u64, info: J, log: &dyn Fn(String)) -> Result<InstanceLock, (Option<J>, PathBuf)> {
        let mut details = json!({
            "pid": std::process::id(),
            "depuis": js::iso_now(),
            "dossier": data_dir.parent().map(|p| p.display().to_string()).unwrap_or_default()
        });
        js::assign(&mut details, info);
        let mut targets = vec![data_dir.join("runtime").join("noyau.lock")];
        if global {
            targets.push(std::env::temp_dir().join("jarvis-noyau.lock"));
        }
        let deadline = Instant::now() + Duration::from_millis(wait_ms);
        let mut held = vec![];
        for file in targets {
            loop {
                match try_lock(&file, &details) {
                    Try::Held(f) => {
                        held.push((f, file.clone()));
                        break;
                    }
                    Try::Error(e) => {
                        log(format!("[instance] verrou {} impossible ({e}) : démarrage sans lui", file.display()));
                        break;
                    }
                    Try::Busy(holder) => {
                        if Instant::now() >= deadline {
                            return Err((holder, file));
                        }
                        std::thread::sleep(Duration::from_millis(250));
                    }
                }
            }
        }
        Ok(InstanceLock { held })
    }

    pub fn release(&mut self) {
        for (f, file) in self.held.drain(..) {
            drop(f);
            let side = PathBuf::from(format!("{}.json", file.display()));
            if js::read_json(&side).map(|c| c["pid"] == json!(std::process::id())).unwrap_or(false) {
                let _ = std::fs::remove_file(side);
            }
        }
    }
}

impl Drop for InstanceLock {
    fn drop(&mut self) {
        self.release();
    }
}

pub fn describe_holder(holder: &Option<J>) -> String {
    let Some(h) = holder else { return "une autre instance".into() };
    let at = js::parse_date_ms(&js::gs(h, "depuis"))
        .and_then(|ms| chrono::DateTime::from_timestamp_millis(ms))
        .map(|d| format!(" depuis {}", d.with_timezone(&chrono::Local).format("%d/%m %H:%M")))
        .unwrap_or_default();
    let pid = if h["pid"].is_null() { "?".to_string() } else { js::s(&h["pid"]) };
    let dossier = js::gs(h, "dossier");
    format!("PID {pid}{at}{}", if dossier.is_empty() { String::new() } else { format!(", dossier {dossier}") })
}

#[allow(unused)]
fn _v(_: Value) {}
