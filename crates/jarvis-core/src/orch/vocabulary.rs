// Vocabulaire dynamique de la reconnaissance vocale (orchestrator/vocabulary.js). Whisper reconnaît
// mal les noms propres qu'il n'a jamais vus : on lui fournit une liste construite à partir de ce que
// Jarvis sait (mots appris, noms propres de la mémoire, applications, zones ajoutées), et des
// corrections « entendu → correct ». Le contenu des zones n'est jamais parcouru.

use std::cell::RefCell;
use std::collections::HashSet;
use std::path::PathBuf;

use serde_json::{json, Value};

use super::{OrchRef, Up};
use crate::js::{self, J};

const BASE_TERMS: &[&str] = &["Jarvis", "Claude Code", "AGY", "Gemini", "tmux", "Spotify", "Discord", "Chrome", "YouTube", "WhatsApp", "Notepad", "Steam", "VLC", "gmeet", "Google Meet", "card", "Nomade", "Listes"];
const DEFAULT_ZONES: &[&str] = &["workspace", "desktop", "documents", "downloads", "projects"];
const MAX_TERMS: usize = 60;
const NOISE: &[&str] = &[
    "copie", "copy", "cours", "projet", "projets", "archive", "archives", "poubelle", "corbeille", "verifier", "vérifier", "type", "design", "nouvel",
    "nouveau", "version", "old", "new", "backup", "tmp", "temp", "test", "tests", "final", "document", "documents", "site", "sites", "src", "lib",
    "data", "assets", "docs", "images", "public", "scripts", "tools", "config",
];

/// « Mes-Photos (2024) » → « Mes Photos ».
pub fn clean_name(name: &str) -> Option<String> {
    let n = crate::re!(r"(?i)\.[a-z0-9]{1,4}$").replace(name, "");
    let words: Vec<&str> = crate::re!(r"[\s\-_.()]+")
        .split(&n)
        .filter(|w| w.chars().count() >= 3)
        .filter(|w| !crate::re!(r"^\d+$").is_match(w) && !crate::re!(r"(?i)^v\d+$").is_match(w) && !crate::re!(r"\d{4}").is_match(w))
        .filter(|w| !NOISE.contains(&w.to_lowercase().as_str()))
        .collect();
    if words.is_empty() {
        return None;
    }
    if words.len() == 1 && words[0].chars().count() < 4 && !crate::re!(r"^[A-Z0-9]+$").is_match(words[0]) {
        return None;
    }
    Some(words.iter().take(3).copied().collect::<Vec<_>>().join(" "))
}

/// Noms propres d'un texte : majuscule hors début de phrase, ou sigle (OVH, GPU).
pub fn proper_nouns(text: &str) -> Vec<String> {
    let words: Vec<&str> = text.split_whitespace().collect();
    let mut out = vec![];
    for (i, raw) in words.iter().enumerate() {
        let w = crate::re!(r"^[^\p{L}\d]+|[^\p{L}\d]+$").replace_all(raw, "");
        let w = crate::re!(r"(?i)^[ldjmnst]['’]").replace(&w, "").to_string();
        if w.chars().count() < 2 {
            continue;
        }
        let prev = if i > 0 { words[i - 1] } else { "" };
        let start = i == 0 || crate::re!(r"[.!?:]$").is_match(prev);
        let acronym = crate::re!(r"^[A-Z0-9]{2,}$").is_match(&w) && crate::re!(r"[A-Z]").is_match(&w);
        let capitalized = crate::re!(r"^\p{Lu}").is_match(&w) && !start;
        if acronym || capitalized {
            out.push(w);
        }
    }
    out
}

pub struct Vocabulary {
    file: PathBuf,
    o: OrchRef,
    pub terms: RefCell<Vec<String>>,
    pub corrections: RefCell<Vec<(String, String)>>,
}

impl Vocabulary {
    pub fn new(file: PathBuf, o: OrchRef) -> Self {
        let v = Self { file, o, terms: RefCell::new(vec![]), corrections: RefCell::new(vec![]) };
        v.load();
        v
    }

    pub fn load(&self) {
        let raw = js::read_json(&self.file).unwrap_or(json!({}));
        *self.terms.borrow_mut() = js::arr(&raw["termes"]).iter().filter(|t| js::truthy(t)).map(js::s).collect();
        *self.corrections.borrow_mut() = js::arr(&raw["corrections"])
            .iter()
            .filter(|c| js::truthy(&c["entendu"]) && js::truthy(&c["correct"]))
            .map(|c| (js::gs(c, "entendu"), js::gs(c, "correct")))
            .collect();
    }

    fn save(&self) {
        let corr: Vec<J> = self.corrections.borrow().iter().map(|(e, c)| json!({ "entendu": e, "correct": c })).collect();
        let _ = js::write_json(&self.file, &json!({ "termes": *self.terms.borrow(), "corrections": corr }));
    }

    pub fn add(&self, term: &str) -> Option<String> {
        let t = term.trim();
        let clean = crate::re!(r#"^["«“]|["»”]$"#).replace_all(t, "").trim().to_string();
        if clean.is_empty() || clean.chars().count() > 60 {
            return None;
        }
        if self.terms.borrow().iter().any(|x| x.to_lowercase() == clean.to_lowercase()) {
            return Some(clean);
        }
        self.terms.borrow_mut().push(clean.clone());
        self.save();
        Some(clean)
    }

    pub fn remove(&self, term: &str) -> bool {
        let key = term.trim().to_lowercase();
        let before = self.terms.borrow().len() + self.corrections.borrow().len();
        self.terms.borrow_mut().retain(|t| t.to_lowercase() != key);
        self.corrections.borrow_mut().retain(|(e, c)| e.to_lowercase() != key && c.to_lowercase() != key);
        if self.terms.borrow().len() + self.corrections.borrow().len() == before {
            return false;
        }
        self.save();
        true
    }

    pub fn correct(&self, heard: &str, right: &str) -> Option<J> {
        let entendu = heard.trim().to_string();
        let correct = right.trim().to_string();
        if entendu.is_empty() || correct.is_empty() || entendu.to_lowercase() == correct.to_lowercase() {
            return None;
        }
        self.corrections.borrow_mut().retain(|(e, _)| e.to_lowercase() != entendu.to_lowercase());
        self.corrections.borrow_mut().push((entendu.clone(), correct.clone()));
        if !self.terms.borrow().iter().any(|t| t.to_lowercase() == correct.to_lowercase()) {
            self.terms.borrow_mut().push(correct.clone());
        }
        self.save();
        Some(json!({ "entendu": entendu, "correct": correct }))
    }

    fn memory_names(&self) -> Vec<String> {
        self.o.up().memory.entries.borrow().iter().flat_map(|e| proper_nouns(&js::gs(e, "text"))).collect()
    }

    fn app_names(&self) -> Vec<String> {
        self.o.up().app_index.entries.borrow().iter().filter_map(|e| clean_name(&e.name)).collect()
    }

    fn custom_zone_names(&self) -> Vec<String> {
        self.o.up().zones.list().iter().filter(|z| !DEFAULT_ZONES.contains(&js::gs(z, "id").as_str())).filter_map(|z| clean_name(&js::gs(z, "name"))).collect()
    }

    /// Liste finale, par priorité, sans doublon, bornée pour le prompt Whisper.
    pub fn build(&self) -> J {
        let mut seen = HashSet::new();
        let mut terms: Vec<String> = vec![];
        let mut push = |list: Vec<String>| {
            for t in list {
                let k = t.to_lowercase();
                if seen.contains(&k) || terms.len() >= MAX_TERMS {
                    continue;
                }
                seen.insert(k);
                terms.push(t);
            }
        };
        push(BASE_TERMS.iter().map(|s| s.to_string()).collect());
        push(self.terms.borrow().clone());
        push(self.app_names());
        push(self.memory_names());
        push(self.custom_zone_names());
        let corrections: Vec<J> = self.corrections.borrow().iter().map(|(e, c)| json!({ "entendu": e, "correct": c })).collect();
        json!({ "terms": terms, "corrections": corrections })
    }

    pub fn list_raw(&self) -> J {
        let corr: Vec<J> = self.corrections.borrow().iter().map(|(e, c)| json!({ "entendu": e, "correct": c })).collect();
        json!({ "termes": *self.terms.borrow(), "corrections": corr })
    }
}

#[allow(dead_code)]
fn _v(_: Value) {}
