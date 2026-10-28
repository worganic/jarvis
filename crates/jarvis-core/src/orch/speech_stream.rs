// Découpage au fil de l'eau du texte généré par le modèle en phrases prononçables
// (orchestrator/speech-stream.js). Chaque phrase complète part vers la synthèse vocale dès qu'elle
// est écrite : Jarvis commence à parler pendant que le modèle termine sa réponse.

use std::cell::{Cell, RefCell};

const MIN_SENTENCE: usize = 12; // en dessous, on fusionne avec la phrase suivante (« Oui. », « M. »)
const FIRST_CHUNK: usize = 45; // première phrase : coupure à la virgule dès 45 caractères
const MAX_SPOKEN: usize = 700; // au-delà, la suite reste affichée à l'écran

/// Nettoie un fragment de markdown pour la voix.
pub fn clean_for_speech(text: &str) -> String {
    let t = crate::re!(r"\[([^\]]+)\]\([^)]+\)").replace_all(text, "$1");
    let t = crate::re!(r"https?://\S+").replace_all(&t, "le lien");
    let t = crate::re!(r"(?m)^\s*(?:[-*•]|\d+[.)])\s+").replace_all(&t, "");
    let t = crate::re!(r"[`*#>_|]").replace_all(&t, "");
    crate::re!(r"\s+").replace_all(&t, " ").trim().to_string()
}

fn has_words(text: &str) -> bool {
    crate::re!(r"[A-Za-zÀ-ÿ0-9]").is_match(text)
}

pub struct SpeechStream {
    on_sentence: Box<dyn Fn(String)>,
    raw: RefCell<String>,
    text: RefCell<String>,
    in_code: Cell<bool>,
    spoken_chars: Cell<usize>,
    truncated: Cell<bool>,
    pub saw_delta: Cell<bool>,
    closed: Cell<bool>,
}

impl SpeechStream {
    pub fn new(on_sentence: Box<dyn Fn(String)>) -> Self {
        Self {
            on_sentence,
            raw: RefCell::new(String::new()),
            text: RefCell::new(String::new()),
            in_code: Cell::new(false),
            spoken_chars: Cell::new(0),
            truncated: Cell::new(false),
            saw_delta: Cell::new(false),
            closed: Cell::new(false),
        }
    }

    pub fn push(&self, delta: &str) {
        if self.closed.get() || delta.is_empty() {
            return;
        }
        self.raw.borrow_mut().push_str(delta);
        self.consume_raw();
        self.flush_sentences(false);
    }

    /// Force une fin de phrase (bloc de texte terminé, appel d'outil qui commence…).
    pub fn boundary(&self) {
        self.push("\n");
    }

    /// Fin de la réponse : prononce le reste. Retourne true si quelque chose a été dit.
    pub fn end(&self) -> bool {
        if self.closed.get() {
            return self.spoken_chars.get() > 0;
        }
        if !self.in_code.get() {
            let raw = std::mem::take(&mut *self.raw.borrow_mut());
            self.text.borrow_mut().push_str(&raw);
        }
        self.raw.borrow_mut().clear();
        self.flush_sentences(true);
        self.closed.set(true);
        if self.truncated.get() {
            (self.on_sentence)("La suite est affichée à l’écran.".into());
        }
        self.spoken_chars.get() > 0
    }

    pub fn cancel(&self) {
        self.closed.set(true);
    }

    fn consume_raw(&self) {
        loop {
            let mut raw = self.raw.borrow_mut();
            let fence = raw.find("```");
            if self.in_code.get() {
                match fence {
                    None => {
                        // garde un éventuel début de ``` fermant
                        let keep: String = raw.chars().rev().take(2).collect::<Vec<_>>().into_iter().rev().collect();
                        *raw = keep;
                        return;
                    }
                    Some(f) => {
                        *raw = raw[f + 3..].to_string();
                        self.in_code.set(false);
                        continue;
                    }
                }
            }
            if let Some(f) = fence {
                self.text.borrow_mut().push_str(&format!("{}\n", &raw[..f]));
                *raw = raw[f + 3..].to_string();
                self.in_code.set(true);
                continue;
            }
            // Pas de bloc de code : tout passe, sauf un éventuel début de ``` en fin de flux.
            let tail = crate::re!(r"`{1,2}$").find(&raw).map(|m| m.as_str().to_string()).unwrap_or_default();
            let keep = raw.len() - tail.len();
            self.text.borrow_mut().push_str(&raw[..keep]);
            *raw = tail;
            return;
        }
    }

    fn flush_sentences(&self, final_: bool) {
        loop {
            let text = self.text.borrow().clone();
            let mut cut: Option<usize> = None;
            for m in crate::fre!(r"[.!?…]+(?=\s)|\n").find_iter(&text).flatten() {
                let end = m.end();
                if clean_for_speech(&text[..end]).chars().count() >= MIN_SENTENCE {
                    cut = Some(end);
                    break;
                }
            }
            // Première phrase : on n'attend pas le point, on coupe à la virgule dès 45 caractères.
            if cut.is_none() && self.spoken_chars.get() == 0 && text.chars().count() >= FIRST_CHUNK {
                let start = text.char_indices().nth(20).map(|(i, _)| i).unwrap_or(text.len());
                if let Some(m) = crate::fre!(r"[,;:](?=\s)").find(&text[start..]).ok().flatten() {
                    cut = Some(start + m.start() + 1);
                }
            }
            let Some(c) = cut else { break };
            self.emit(&text[..c]);
            *self.text.borrow_mut() = text[c..].to_string();
        }
        if final_ {
            let rest = std::mem::take(&mut *self.text.borrow_mut());
            self.emit(&rest);
        }
    }

    fn emit(&self, fragment: &str) {
        let sentence = clean_for_speech(fragment);
        if sentence.is_empty() || !has_words(&sentence) || self.truncated.get() {
            return;
        }
        let len = sentence.chars().count();
        if self.spoken_chars.get() + len > MAX_SPOKEN && self.spoken_chars.get() > 0 {
            self.truncated.set(true);
            return;
        }
        self.spoken_chars.set(self.spoken_chars.get() + len);
        (self.on_sentence)(sentence);
    }
}
