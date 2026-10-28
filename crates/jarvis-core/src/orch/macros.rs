// Macros personnelles (orchestrator/macros.js) : « quand je dis mode travail, ouvre VS Code, lance
// Chrome et mets de la musique ». Une phrase déclenche une suite d'actions ; les étapes reconnues
// localement s'exécutent sans IA, les autres partent à l'IA. Stockage : data/macros.json.

use std::cell::RefCell;
use std::path::PathBuf;

use serde_json::{json, Value};

use crate::js::{self, J};

pub fn fold(text: &str) -> String {
    let t = js::fold_lower(text);
    let t = crate::re!(r"['’\-?!,;:.…]").replace_all(&t, " ");
    crate::re!(r"\s+").replace_all(&t, " ").trim().to_string()
}

/// Découpe « ouvre VS Code, lance Chrome et mets de la musique » en étapes.
pub fn split_steps(text: &str) -> Vec<String> {
    let re = crate::fre!(r"(?i)\s*(?:,|;|\bpuis\b|\bensuite\b|\bet ensuite\b|\bet puis\b|\bet\b(?=\s+(?:ouvre|lance|mets|joue|affiche|monte|baisse|coupe|passe|ferme|verrouille|cherche|lis|dis|active|desactive|désactive|règle|regle|prends|fais)\b))\s*");
    let mut out = vec![];
    let mut last = 0;
    for m in re.find_iter(text).flatten() {
        out.push(text[last..m.start()].to_string());
        last = m.end();
    }
    out.push(text[last..].to_string());
    out.into_iter()
        .map(|s| crate::re!(r"[.!?]+$").replace(s.trim(), "").to_string())
        .filter(|s| !s.is_empty())
        .take(8)
        .collect()
}

pub struct Macros {
    file: PathBuf,
    pub items: RefCell<Vec<J>>,
}

impl Macros {
    pub fn new(file: PathBuf) -> Self {
        let m = Self { file, items: RefCell::new(vec![]) };
        m.load();
        m
    }

    fn load(&self) {
        let items = js::read_json(&self.file).and_then(|s| s.get("macros").and_then(|m| m.as_array().cloned())).unwrap_or_default();
        *self.items.borrow_mut() = items;
    }

    fn save(&self) {
        let _ = js::write_json(&self.file, &json!({ "macros": Value::Array(self.items.borrow().clone()) }));
    }

    pub fn add(&self, trigger: &str, steps: Vec<String>) -> Option<J> {
        let key = fold(trigger);
        if key.is_empty() || steps.is_empty() {
            return None;
        }
        let macro_ = json!({ "name": trigger.trim(), "key": key, "steps": steps, "createdAt": js::iso_now() });
        {
            let mut items = self.items.borrow_mut();
            items.retain(|m| js::gs(m, "key") != key);
            items.push(macro_.clone());
        }
        self.save();
        Some(macro_)
    }

    pub fn remove(&self, trigger: &str) -> usize {
        let key = fold(trigger);
        let before = self.items.borrow().len();
        self.items.borrow_mut().retain(|m| {
            let k = js::gs(m, "key");
            k != key && !k.contains(&key)
        });
        let after = self.items.borrow().len();
        if after != before {
            self.save();
        }
        before - after
    }

    /// La phrase entière doit être le déclencheur (à la politesse près).
    pub fn match_(&self, text: &str) -> Option<J> {
        let key = fold(text);
        let key = crate::re!(r"^(?:jarvis |active |lance |passe en |mode )").replace(&key, "").into_owned();
        let key = crate::re!(r"\s+(?:s il te plait|stp|jarvis)$").replace(&key, "").into_owned();
        self.items
            .borrow()
            .iter()
            .find(|m| {
                let k = js::gs(m, "key");
                k == key || crate::re!(r"^mode ").replace(&k, "") == key
            })
            .cloned()
    }

    pub fn list(&self) -> Vec<J> {
        self.items.borrow().clone()
    }
}
