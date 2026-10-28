// Émotions (orchestrator/emotions.js) : mots et phrases notés de 1 à 10, associés aux 10 niveaux
// d'émotion du cœur (10 extase … 1 désespoir). Une phrase de l'utilisateur est évaluée par
// correspondance directe, sinon par similarité de n-grammes de caractères (seuil 0,42).

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use serde_json::json;

use super::automations::base36;
use crate::js::{self, J};

fn fold(s: &str) -> String {
    let t = js::fold_lower(s);
    let t = crate::re!(r"[^a-z0-9\s]").replace_all(&t, " ");
    crate::re!(r"\s+").replace_all(&t, " ").trim().to_string()
}

/// (score, action, nom, description)
pub fn level(score: i64) -> (i64, &'static str, &'static str, &'static str) {
    match score {
        10 => (10, "extase", "L'extase (ou l'euphorie)", "Joie intense et débordante, sentiment de ravissement absolu qui coupe presque du monde réel."),
        9 => (9, "joie", "La joie", "Émotion vive, chaleureuse et communicative liée à un événement ou un accomplissement direct."),
        8 => (8, "contentement", "Le contentement", "État d'équilibre et de satisfaction paisible, sans excitation mais avec un vrai bien-être intérieur."),
        6 => (6, "melancolie", "La mélancolie", "Tristesse douce et poétique, souvent teintée de nostalgie ou d'un vague à l'âme sans cause dramatique précise."),
        5 => (5, "chagrin", "Le chagrin", "Douleur morale liée à une perte, une déception ou une blessure affective bien identifiable."),
        4 => (4, "abattement", "L'abattement", "Perte d'énergie psychique et physique face à une difficulté, sensation d'être écrasé ou découragé."),
        3 => (3, "detresse", "La détresse", "Sentiment d'angoisse aiguë et d'impuissance face à une situation que l'on ne maîtrise plus."),
        2 => (2, "accablement", "L'accablement", "Épuisement complet sous le poids de la souffrance morale, sentiment de ne plus pouvoir porter le fardeau."),
        1 => (1, "desespoir", "Le désespoir", "Rupture totale de l'espérance, sentiment qu'aucune issue ou amélioration n'est possible."),
        _ => (7, "serenite", "La sérénité", "Calme profond de l'esprit, absence de trouble, neutralité positive exempte d'inquiétude."),
    }
}

pub const DEFAULT_ENTRIES: &[(&str, i64)] = &[
    ("génial", 10), ("ca marche au top", 10), ("c'est incroyable", 10), ("c'est extraordinaire", 10), ("merveilleux", 10),
    ("super", 9), ("merci", 9), ("bravo", 9), ("trop bien", 9), ("excellent", 9), ("merci beaucoup", 9),
    ("tous va bien", 8), ("tout va bien", 8), ("c'est bon", 8), ("impeccable", 8), ("parfaitement satisfait", 8), ("nickel", 8),
    ("tranquille", 7), ("aucun problème", 7), ("calme et posé", 7), ("zen", 7), ("tout est en ordre", 7),
    ("bof", 6), ("comme d'habitude", 6), ("un peu triste", 6), ("un brin nostalgique", 6), ("sans plus", 6),
    ("déçu", 5), ("c'est dommage", 5), ("ça me blesse", 5), ("mauvaise nouvelle", 5), ("peine de coeur", 5),
    ("j'en ai marre", 4), ("ras le bol", 4), ("je suis fatigué", 4), ("découragé", 4), ("trop épuisé", 4),
    ("bouge toi", 3), ("je panique", 3), ("à l'aide", 3), ("au secours", 3), ("je sais plus quoi faire", 3),
    ("tu fait n'importe quoi", 2), ("tu fais n'importe quoi", 2), ("merde", 2), ("trop lourd à porter", 2), ("c'est insupportable", 2), ("j'en peux plus", 2),
    ("tu fais chier", 1), ("c'est foutu", 1), ("aucun espoir", 1), ("irrécupérable", 1), ("tout est perdu", 1), ("foutu d'avance", 1),
];

/// N-grammes de caractères (trigrammes) + mots de plus de 2 lettres (poids 3).
fn ngrams(s: &str) -> HashMap<String, f64> {
    let s = fold(s);
    let mut v = HashMap::new();
    let chars: Vec<char> = s.chars().collect();
    if chars.len() >= 3 {
        for i in 0..=chars.len() - 3 {
            *v.entry(chars[i..i + 3].iter().collect::<String>()).or_insert(0.0) += 1.0;
        }
    }
    for w in s.split_whitespace().filter(|w| w.chars().count() > 2) {
        *v.entry(format!("w_{w}")).or_insert(0.0) += 3.0;
    }
    v
}

fn cosine_sim(a: &HashMap<String, f64>, b: &HashMap<String, f64>) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let na: f64 = a.values().map(|v| v * v).sum();
    let nb: f64 = b.values().map(|v| v * v).sum();
    let dot: f64 = a.iter().filter_map(|(k, va)| b.get(k).map(|vb| va * vb)).sum();
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    dot / (na.sqrt() * nb.sqrt())
}

fn clamp_score(v: &J) -> i64 {
    let n = js::num(v).filter(|n| *n != 0.0 && n.is_finite()).unwrap_or(7.0);
    (n.round() as i64).clamp(1, 10)
}

pub struct Emotions {
    file: PathBuf,
    log: Rc<dyn Fn(String)>,
    entries: RefCell<Vec<J>>,
    vectors: RefCell<HashMap<String, HashMap<String, f64>>>,
}

impl Emotions {
    pub fn new(file: PathBuf, log: Rc<dyn Fn(String)>) -> Self {
        let me = Self { file, log, entries: RefCell::new(vec![]), vectors: RefCell::new(HashMap::new()) };
        me.load();
        me
    }

    pub fn load(&self) {
        if let Some(raw) = js::read_json(&self.file).and_then(|r| r.as_array().cloned()).filter(|a| !a.is_empty()) {
            let entries: Vec<J> = raw
                .iter()
                .enumerate()
                .map(|(i, e)| {
                    json!({
                        "id": if js::truthy(&e["id"]) { js::gs(e, "id") } else { format!("em_{}", i + 1) },
                        "text": js::gs(e, "text").trim(),
                        "score": clamp_score(&e["score"]),
                        "createdAt": if js::truthy(&e["createdAt"]) { js::gs(e, "createdAt") } else { js::iso_now() }
                    })
                })
                .filter(|e| js::truthy(&e["text"]))
                .collect();
            *self.entries.borrow_mut() = entries;
            self.rebuild_vectors();
            return;
        }
        self.reset_defaults();
    }

    pub fn reset_defaults(&self) {
        let now = js::iso_now();
        *self.entries.borrow_mut() = DEFAULT_ENTRIES.iter().enumerate().map(|(i, (t, s))| json!({ "id": format!("em_{}", i + 1), "text": t, "score": s, "createdAt": now })).collect();
        self.save();
        self.rebuild_vectors();
    }

    fn rebuild_vectors(&self) {
        let mut v = self.vectors.borrow_mut();
        v.clear();
        for e in self.entries.borrow().iter() {
            v.insert(js::gs(e, "id"), ngrams(&js::gs(e, "text")));
        }
    }

    fn save(&self) {
        if let Err(e) = js::write_json(&self.file, &json!(*self.entries.borrow())) {
            (self.log)(format!("[emotions] Erreur écriture {} : {e}", self.file.display()));
        }
    }

    pub fn list(&self) -> Vec<J> {
        self.entries
            .borrow()
            .iter()
            .map(|e| {
                let (score, action, name, description) = level(e["score"].as_i64().unwrap_or(7));
                let mut o = e.clone();
                js::assign(&mut o, json!({ "level": score, "action": action, "levelName": name, "description": description }));
                o
            })
            .collect()
    }

    pub fn add(&self, text: &str, score: &J) -> J {
        let clean = text.trim();
        if clean.is_empty() {
            return json!({ "ok": false, "error": "Texte vide" });
        }
        let s = clamp_score(score);
        let existing = self.entries.borrow().iter().position(|e| fold(&js::gs(e, "text")) == fold(clean));
        if let Some(i) = existing {
            let entry = {
                let mut list = self.entries.borrow_mut();
                list[i]["score"] = json!(s);
                list[i]["updatedAt"] = json!(js::iso_now());
                list[i].clone()
            };
            self.save();
            self.rebuild_vectors();
            return json!({ "ok": true, "entry": entry, "updated": true });
        }
        let entry = json!({ "id": format!("em_{}_{}", base36(js::now_ms() as u64), js::rand_below(1000)), "text": clean, "score": s, "createdAt": js::iso_now() });
        self.entries.borrow_mut().insert(0, entry.clone());
        self.vectors.borrow_mut().insert(js::gs(&entry, "id"), ngrams(clean));
        self.save();
        json!({ "ok": true, "entry": entry })
    }

    pub fn update(&self, id: &str, text: Option<&str>, score: Option<&J>) -> J {
        let entry = {
            let mut list = self.entries.borrow_mut();
            let Some(e) = list.iter_mut().find(|e| js::gs(e, "id") == id) else { return json!({ "ok": false, "error": "Entrée introuvable" }) };
            if let Some(t) = text {
                let clean = t.trim();
                if clean.is_empty() {
                    return json!({ "ok": false, "error": "Texte vide" });
                }
                e["text"] = json!(clean);
            }
            if let Some(s) = score {
                e["score"] = json!(clamp_score(s));
            }
            e["updatedAt"] = json!(js::iso_now());
            e.clone()
        };
        self.vectors.borrow_mut().insert(id.to_string(), ngrams(&js::gs(&entry, "text")));
        self.save();
        json!({ "ok": true, "entry": entry })
    }

    pub fn delete(&self, id: &str) -> J {
        let idx = self.entries.borrow().iter().position(|e| js::gs(e, "id") == id);
        let Some(i) = idx else { return json!({ "ok": false, "error": "Entrée introuvable" }) };
        let removed = self.entries.borrow_mut().remove(i);
        self.vectors.borrow_mut().remove(id);
        self.save();
        json!({ "ok": true, "entry": removed })
    }

    /// Évaluation lexicale puis par similarité de l'émotion d'un message.
    pub fn evaluate(&self, text: &str) -> Option<J> {
        let clean = fold(text.trim());
        if clean.is_empty() {
            return None;
        }
        let entries = self.entries.borrow();
        let mut best: Option<&J> = None;
        let mut longest = 0;
        let padded = format!(" {clean} ");
        for e in entries.iter() {
            let ec = fold(&js::gs(e, "text"));
            if ec.is_empty() {
                continue;
            }
            if clean == ec {
                best = Some(e);
                break;
            }
            if padded.contains(&format!(" {ec} ")) && ec.len() > longest {
                longest = ec.len();
                best = Some(e);
            }
        }
        let result = |e: &J, kind: &str, sim: Option<f64>| {
            let (score, action, name, description) = level(e["score"].as_i64().unwrap_or(7));
            let mut r = json!({ "matched": true, "matchType": kind, "entryText": e["text"], "score": score, "action": action, "name": name, "description": description });
            if let Some(s) = sim {
                r["similarity"] = json!((s * 100.0).round() / 100.0);
            }
            r
        };
        if let Some(e) = best {
            return Some(result(e, "direct", None));
        }
        let input = ngrams(&clean);
        let vectors = self.vectors.borrow();
        let mut best_sim = 0.0;
        let mut best_entry = None;
        for e in entries.iter() {
            let Some(v) = vectors.get(&js::gs(e, "id")) else { continue };
            let s = cosine_sim(&input, v);
            if s > best_sim {
                best_sim = s;
                best_entry = Some(e);
            }
        }
        best_entry.filter(|_| best_sim >= 0.42).map(|e| result(e, "semantic", Some(best_sim)))
    }
}
