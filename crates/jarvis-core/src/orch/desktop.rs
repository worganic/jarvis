// Saisie et correction dans un document ouvert (orchestrator/desktop.js), sans IA :
// - le texte dicté est collé (presse-papier + Ctrl+V) dans la fenêtre au premier plan ;
// - la correction passe par Grammalecte, en local (tools/grammar-fix.py, Python facultatif).
// Garde-fou : la fenêtre au premier plan doit être un traitement de texte. Un terminal, l'Explorateur
// ou Jarvis lui-même ne reçoivent jamais de frappe. Presse-papier et touches : API Windows natives.

use std::path::PathBuf;

use serde_json::{json, Value};

use crate::js::{self, J};

pub fn doc_window(label: &str) -> bool {
    crate::re!(r"(?i)google docs|documents google|docs\.google|\bword\b|winword|bloc-notes|notepad|wordpad|libreoffice|soffice|writer|onenote|obsidian|notion|typora|google keep|google sheets|feuilles google").is_match(label)
}

fn never(process: &str) -> bool {
    crate::re!(r"(?i)powershell|pwsh|cmd\.exe|^cmd$|windowsterminal|conhost|terminal|explorer|jarvis|electron|taskmgr|regedit|mmc|keepass|bitwarden|1password").is_match(process)
}

/// Ponctuation dictée → signes. Whisper ponctue déjà souvent : seuls les mots explicites sont convertis.
pub fn spoken_punctuation(text: &str) -> String {
    let mut s = format!(" {} ", text.trim());
    let rules: [(&regex::Regex, &str); 10] = [
        (crate::re!(r"(?i)\s+nouveau paragraphe[.,]?\s+"), "\n\n"),
        (crate::re!(r"(?i)\s+(?:à la ligne|a la ligne|nouvelle ligne|retour à la ligne)[.,]?\s+"), "\n"),
        (crate::re!(r"(?i)\s+point d['’]interrogation[.,]?"), " ?"),
        (crate::re!(r"(?i)\s+point d['’]exclamation[.,]?"), " !"),
        (crate::re!(r"(?i)\s+points de suspension[.,]?"), "…"),
        (crate::re!(r"(?i)\s+point[- ]virgule[.,]?"), " ;"),
        (crate::re!(r"(?i)\s+deux[- ]points[.,]?"), " :"),
        (crate::re!(r"(?i)\s+virgule[.,]?"), ","),
        (crate::re!(r"(?i)\s+(?:ouvrez|ouvre) les guillemets[.,]?\s+"), " « "),
        (crate::re!(r"(?i)\s+(?:fermez|ferme) les guillemets[.,]?"), " »"),
    ];
    for (re, rep) in rules.iter() {
        s = re.replace_all(&s, *rep).into_owned();
    }
    // « point » en fin de ligne (anticipation de la v3 : le saut de ligne est gardé).
    s = crate::fre!(r"(?i)\s+point(?=\s*(?:\n|$))").replace_all(&s, ".").into_owned();
    // « point » suivi d'une majuscule.
    s = crate::fre!(r"(?i)\s+point[.,]?\s+(?=\p{Lu})").replace_all(&s, ". ").into_owned();
    let s = crate::re!(r"[ \t]+\n").replace_all(&s, "\n");
    let s = crate::re!(r"\n[ \t]+").replace_all(&s, "\n");
    crate::re!(r"^ +| +$").replace_all(&s, "").into_owned()
}

pub struct Desktop {
    pub python: Option<String>,
    grammar_script: PathBuf,
}

impl Desktop {
    pub fn new(python: Option<String>) -> Self {
        Self { python, grammar_script: crate::paths::root().join("tools").join("grammar-fix.py") }
    }

    pub async fn target_window() -> (bool, String, String) {
        let w = super::system::foreground_window().await;
        let process = js::gs(&w, "process");
        let title = js::gs(&w, "title");
        let ok = doc_window(&format!("{process} {title}")) && !never(&process);
        (ok, process, title)
    }

    fn refused(title: &str, process: &str) -> J {
        json!({ "ok": false, "refus": true, "fenetre": if title.is_empty() { process } else { title }, "message": "La fenêtre au premier plan n'est pas un document." })
    }

    /// Colle le texte dans le document au premier plan, puis restaure le presse-papier.
    pub async fn type_text(&self, text: &str) -> J {
        let (ok, process, title) = Self::target_window().await;
        if !ok {
            return Self::refused(&title, &process);
        }
        let t = text.to_string();
        let r = tokio::task::spawn_blocking(move || jarvis_native::input::paste(&t)).await;
        match r {
            Ok(Ok(())) => json!({ "ok": true, "fenetre": title, "caracteres": text.encode_utf16().count() }),
            Ok(Err(e)) => json!({ "ok": false, "error": e }),
            Err(e) => json!({ "ok": false, "error": e.to_string() }),
        }
    }

    /// Terminal au premier plan (terminal_voice), hors garde-fou des documents : le texte n'est collé
    /// que si la fenêtre attendue est toujours devant (`hwnd`), et sans Entrée. `keys` : touches SendKeys.
    pub async fn paste_in_terminal(&self, hwnd: i64, text: &str, keys: &str) -> J {
        let (text, keys) = (text.to_string(), keys.to_string());
        let r = tokio::task::spawn_blocking(move || {
            // Une fenêtre de Jarvis (ou un focus perdu) a pris le premier plan : le terminal est ramené devant.
            let recovered = match jarvis_native::input::ensure_foreground(hwnd) {
                Ok(r) => r,
                Err(front) => return Ok(Err(front)),
            };
            if !keys.is_empty() {
                jarvis_native::input::send_keys(&keys);
            } else {
                jarvis_native::input::paste(&text)?;
            }
            Ok::<Result<bool, String>, String>(Ok(recovered))
        })
        .await;
        match r {
            Ok(Ok(Ok(recovered))) => json!({ "ok": true, "refocalise": recovered }),
            Ok(Ok(Err(front))) => json!({
                "ok": false, "autre": true, "devant": front,
                "message": if front.is_empty() { "Le terminal n’est plus au premier plan : rien n’a été tapé.".to_string() } else { format!("Le terminal n’est plus au premier plan ({front} est devant) : rien n’a été tapé.") }
            }),
            Ok(Err(e)) => json!({ "ok": false, "error": e }),
            Err(e) => json!({ "ok": false, "error": e.to_string() }),
        }
    }

    /// Ctrl+Z dans le document (annule la dernière saisie).
    pub async fn undo(&self) -> J {
        let (ok, process, title) = Self::target_window().await;
        if !ok {
            return Self::refused(&title, &process);
        }
        let _ = tokio::task::spawn_blocking(|| jarvis_native::input::ctrl('z')).await;
        json!({ "ok": true })
    }

    async fn copy_text(whole: bool) -> String {
        tokio::task::spawn_blocking(move || jarvis_native::input::copy_selection(whole)).await.unwrap_or_default()
    }

    /// Correction locale d'un texte avec Grammalecte.
    async fn grammar(&self, text: &str) -> Result<J, String> {
        let python = self.python.clone().filter(|_| self.grammar_script.exists()).ok_or("correcteur Grammalecte indisponible")?;
        let args = vec![self.grammar_script.to_string_lossy().to_string()];
        let out = crate::proc::run(&python, &args, Some(text), 30000).await?;
        serde_json::from_str(&out.stdout).map_err(|_| {
            let e = js::slice_chars(out.stderr.trim(), 200);
            if e.is_empty() { "réponse illisible".to_string() } else { e }
        })
    }

    /// Corrige la sélection (whole = false) ou tout le document (whole = true) et recolle le texte.
    pub async fn correct(&self, whole: bool) -> J {
        let (ok, process, title) = Self::target_window().await;
        if !ok {
            return Self::refused(&title, &process);
        }
        let original = Self::copy_text(whole).await;
        if original.trim().is_empty() {
            return json!({ "ok": true, "corrections": 0, "vide": true });
        }
        let result = match self.grammar(&original).await {
            Ok(r) => r,
            Err(e) => return json!({ "ok": false, "error": e }),
        };
        let mut fixed = js::gs(&result, "text");
        if fixed.ends_with('\n') && !original.ends_with('\n') {
            fixed.pop();
        }
        let fixes = js::arr(&result["fixes"]);
        if fixes.is_empty() || fixed == original {
            return json!({ "ok": true, "corrections": 0 });
        }
        // Recollage : la sélection (tout le document si whole) est encore active après Ctrl+C.
        let f2 = fixed.clone();
        let _ = tokio::task::spawn_blocking(move || {
            let _ = jarvis_native::input::clipboard_set(&f2);
            std::thread::sleep(std::time::Duration::from_millis(80));
            if whole {
                jarvis_native::input::ctrl('a');
                std::thread::sleep(std::time::Duration::from_millis(120));
            }
            jarvis_native::input::ctrl('v');
        })
        .await;
        json!({ "ok": true, "corrections": fixes.len(), "exemples": fixes.into_iter().take(3).collect::<Vec<Value>>() })
    }
}
