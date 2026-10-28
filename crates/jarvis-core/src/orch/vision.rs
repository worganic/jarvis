// Vision par la webcam (orchestrator/vision.js) : « regarde ce que je te montre », « qu'est-ce que je
// tiens ? ». Uniquement sur demande explicite : le HUD ouvre la caméra (compte à rebours de 2 s), prend
// une seule image et la ferme. Image dans workspace/vision/ (les 20 dernières), puis Claude Code la regarde.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;

use base64::Engine;
use serde_json::json;

use super::automations::base36;
use super::intent::Intent;
use super::{OrchRef, Up};
use crate::js::{self, Timer, J};

const KEEP: usize = 20;
const TIMEOUT_MS: u64 = 15000;
const MAX_BYTES: usize = 3 * 1024 * 1024;

pub fn is_vision(t: &str) -> bool {
    let re = crate::fre!(concat!(
        r"^(?:jarvis )?regarde(?: bien)? (?:ce que je (?:te )?(?:montre|tiens)|ca|ceci|cela|ce truc|cet objet|cette chose|ce papier|cette feuille|ce document|cette etiquette|moi|par la (?:webcam|camera))\b",
        r"|^(?:jarvis )?(?:qu est ce que|que|qu est ce qu|quel est ce que) (?:je )?(?:tiens|te montre|montre|porte|ai dans la main)\b",
        r"|^(?:jarvis )?(?:tu vois quoi|que vois tu|qu est ce que tu vois|dis moi ce que tu vois|decris ce que tu vois)\b(?!.*\b(?:ecran|fenetre|page)\b)",
        r"|^(?:jarvis )?(?:lis|dechiffre|traduis) (?:ce que je (?:te )?montre|ce papier|cette etiquette|ce ticket|cette feuille|ce document que je (?:te )?montre)\b",
        r"|\b(?:avec|par|sur) (?:la|ma|ta) (?:webcam|camera)\b",
        r"|^(?:jarvis )?(?:prends|fais|prend) (?:moi )?(?:une )?(?:photo|capture) (?:de moi|avec la (?:webcam|camera)|par la (?:webcam|camera))\b",
        r"|^(?:jarvis )?(?:comment|est ce que) (?:je suis|j ai l air)\b.*\b(?:habille|coiffe|tenue|look)\b"
    ));
    js::ftest(re, t)
}

pub struct Vision {
    o: OrchRef,
    dir_path: Option<PathBuf>,
    /// id → (question, minuteur)
    pending: RefCell<HashMap<String, (String, Timer)>>,
    counter: Cell<u64>,
}

impl Vision {
    pub fn new(o: OrchRef, dir: Option<PathBuf>) -> Self {
        Self { o, dir_path: dir, pending: RefCell::new(HashMap::new()), counter: Cell::new(0) }
    }

    fn dir(&self) -> PathBuf {
        let d = self.dir_path.clone().unwrap_or_else(|| crate::paths::workspace().join("vision"));
        let _ = std::fs::create_dir_all(&d);
        d
    }

    pub fn match_(&self, t: &str, raw: &str) -> Option<Intent> {
        if !is_vision(t) {
            return None;
        }
        let question = if raw.trim().is_empty() { t.trim().to_string() } else { raw.trim().to_string() };
        let o = self.o.clone();
        let q = question.clone();
        Some(Intent::new("vision", "Webcam", &question, move || {
            let o = o.clone();
            let q = q.clone();
            async move {
                o.up().vision.capture(&q);
                json!({ "say": "Montre-moi.", "display": "◉ Webcam : une image, sur ta demande", "quiet": true })
            }
        }))
    }

    /// Demande une image au HUD. Sans réponse (pas de HUD, caméra refusée) : Jarvis le dit.
    pub fn capture(&self, question: &str) -> String {
        self.counter.set(self.counter.get() + 1);
        let id = format!("cam{}{}", base36(js::now_ms() as u64), self.counter.get());
        let timer = Timer::new();
        let o = self.o.clone();
        let id2 = id.clone();
        timer.set(TIMEOUT_MS, move || {
            o.up().vision.fail(&id2, "pas de réponse du HUD");
        });
        self.pending.borrow_mut().insert(id.clone(), (question.to_string(), timer));
        let orch = self.o.up();
        orch.emit(json!({ "type": "camera_capture", "id": id, "countdown": 2 }));
        orch.record("webcam_demandee", json!({ "demande": js::slice_chars(question, 200) }));
        id
    }

    pub fn fail(&self, id: &str, reason: &str) -> J {
        let Some((_, timer)) = self.pending.borrow_mut().remove(id) else { return json!({ "ok": false, "error": "demande inconnue" }) };
        timer.clear();
        let o = self.o.up();
        o.record("webcam_echec", json!({ "motif": js::slice_chars(reason, 200) }));
        let refused = crate::re!(r"(?i)NotAllowed|Permission|refus").is_match(reason);
        let missing = crate::re!(r"(?i)NotFound|DevicesNotFound|aucune").is_match(reason);
        let text = if refused {
            "La webcam m'est refusée. Autorise-la quand Windows ou Jarvis te le demande, puis réessaie."
        } else if missing {
            "Je ne trouve pas de webcam sur ce PC."
        } else {
            "Je n'arrive pas à utiliser la webcam."
        };
        o.emit(json!({ "type": "say", "text": text }));
        json!({ "ok": false })
    }

    /// Image envoyée par le HUD (data URL JPEG ou PNG).
    pub fn frame(&self, id: &str, data_url: &str) -> J {
        if !self.pending.borrow().contains_key(id) {
            return json!({ "ok": false, "error": "demande inconnue ou expirée" });
        }
        let Some(m) = crate::re!(r"^data:image/(jpeg|png);base64,([A-Za-z0-9+/=]+)$").captures(data_url) else { return self.fail(id, "image illisible") };
        let bytes = base64::engine::general_purpose::STANDARD.decode(&m[2]).unwrap_or_default();
        if bytes.is_empty() || bytes.len() > MAX_BYTES {
            return self.fail(id, "image vide ou trop lourde");
        }
        let (question, timer) = self.pending.borrow_mut().remove(id).unwrap();
        timer.clear();
        let stamp = chrono::Utc::now().format("%Y%m%d-%H%M%S").to_string();
        let file = self.dir().join(format!("webcam-{stamp}.{}", if &m[1] == "png" { "png" } else { "jpg" }));
        if let Err(e) = std::fs::write(&file, &bytes) {
            return json!({ "ok": false, "error": e.to_string() });
        }
        self.prune();
        let o = self.o.up();
        o.record("webcam_image", json!({ "fichier": file.file_name().map(|f| f.to_string_lossy().to_string()), "octets": bytes.len() }));
        o.enqueue_ai(Self::prompt(&question, &file.to_string_lossy()), json!({ "priority": 1 }));
        json!({ "ok": true, "file": file.to_string_lossy() })
    }

    fn prompt(question: &str, file: &str) -> String {
        [
            format!("L'utilisateur vient de te montrer quelque chose à la webcam, à sa demande. La photo est ici : {file}"),
            "Ouvre-la avec l’outil Read (c’est une image) et réponds à sa demande, en une à trois phrases faites pour être dites à voix haute :".into(),
            format!("« {question} »"),
            "S’il te montre un objet, dis ce que c’est. S’il y a du texte lisible, lis l’essentiel. Si l’image est floue ou sombre, dis-le simplement.".into(),
            "Ne décris pas la personne au-delà de ce qui est demandé.".into(),
        ]
        .join("\n")
    }

    fn prune(&self) {
        let d = self.dir();
        let Ok(rd) = std::fs::read_dir(&d) else { return };
        let mut files: Vec<String> = rd
            .filter_map(|e| e.ok())
            .map(|e| e.file_name().to_string_lossy().to_string())
            .filter(|f| crate::re!(r"^webcam-.*\.(?:jpg|png)$").is_match(f))
            .collect();
        files.sort();
        let extra = files.len().saturating_sub(KEEP);
        for f in files.iter().take(extra) {
            let _ = std::fs::remove_file(d.join(f));
        }
    }
}
