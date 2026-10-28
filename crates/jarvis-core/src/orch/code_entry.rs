// Saisie du code de validation (orchestrator/code-entry.js) chiffre par chiffre : clic sur le clavier
// du HUD, touches du clavier ou chiffres dictés. Chaque chiffre est vérifié et renvoyé à l'interface
// (« code_entry ») : vert s'il est bon, rouge sinon. Au 4e bon chiffre, l'action est validée.
// À la voix, il n'est accepté que si l'empreinte vocale reconnaît l'utilisateur (quand elle est active).

use serde_json::json;

use super::{OrchRef, Up};
use crate::js::{self, J};

fn unit(w: &str) -> Option<u32> {
    Some(match w {
        "zero" => 0,
        "un" | "une" => 1,
        "deux" => 2,
        "trois" => 3,
        "quatre" => 4,
        "cinq" => 5,
        "six" => 6,
        "sept" => 7,
        "huit" => 8,
        "neuf" => 9,
        "dix" => 10,
        "onze" => 11,
        "douze" => 12,
        "treize" => 13,
        "quatorze" => 14,
        "quinze" => 15,
        "seize" => 16,
        _ => return None,
    })
}

fn ten(w: &str) -> Option<u32> {
    Some(match w {
        "vingt" | "vingts" => 20,
        "trente" => 30,
        "quarante" => 40,
        "cinquante" => 50,
        "soixante" => 60,
        _ => return None,
    })
}

const FILLER: &[&str] = &["le", "la", "code", "c", "est", "et", "voici", "voila", "alors", "euh", "heu", "jarvis", "mon", "valide", "validation", "chiffre", "chiffres", "numero", "je", "tape", "saisis", "puis", "ensuite"];

/// « 4 8 2 1 », « quarante-huit vingt et un », « zéro cinq… » → « 4821 » ; None si autre chose.
pub fn spoken_digits(text: &str) -> Option<String> {
    let f = js::fold_lower(text);
    let f = crate::re!(r"[^a-z0-9]+").replace_all(&f, " ");
    let tokens: Vec<&str> = f.trim().split(' ').filter(|w| !w.is_empty() && !FILLER.contains(w)).collect();
    if tokens.is_empty() {
        return None;
    }
    let at = |i: usize| tokens.get(i).copied().unwrap_or("");
    let mut out = String::new();
    let mut i = 0;
    while i < tokens.len() {
        let w = tokens[i];
        if w.bytes().all(|b| b.is_ascii_digit()) {
            out.push_str(w);
            i += 1;
            continue;
        }
        let mut n = None;
        if w == "quatre" && matches!(at(i + 1), "vingt" | "vingts") {
            n = Some(80);
            i += 1;
        } else if let Some(t) = ten(w) {
            n = Some(t);
        }
        if let Some(mut n) = n {
            if let Some(u) = unit(at(i + 1)) {
                let max = if n == 60 || n == 80 { 16 } else { 9 };
                if u <= max && u > 0 {
                    n += u;
                    i += 1;
                    if n == 70 || n == 90 {
                        if let Some(u2) = unit(at(i + 1)).filter(|u| *u <= 9 && *u > 0) {
                            n += u2;
                            i += 1;
                        }
                    }
                }
            }
            out.push_str(&n.to_string());
            i += 1;
            continue;
        }
        if let Some(u) = unit(w) {
            if u == 10 && matches!(at(i + 1), "sept" | "huit" | "neuf") {
                out.push_str(&(10 + unit(at(i + 1)).unwrap()).to_string());
                i += 2;
                continue;
            }
            out.push_str(&u.to_string());
            i += 1;
            continue;
        }
        return None;
    }
    if out.is_empty() { None } else { Some(out) }
}

pub struct CodeEntry {
    o: OrchRef,
}

impl CodeEntry {
    pub fn new(o: OrchRef) -> Self {
        Self { o }
    }

    /// Validation par code en attente (la plus récente si plusieurs) : (taskId, code, typed).
    fn current(&self, task_id: Option<&str>) -> Option<(String, String, String)> {
        let o = self.o.up();
        let pending = o.pending.borrow();
        let found = match task_id {
            Some(id) => pending.iter().find(|(k, e)| k == id && js::truthy(&e["code"])),
            None => pending.iter().rev().find(|(_, e)| js::truthy(&e["code"])),
        };
        found.map(|(k, e)| (k.clone(), js::gs(e, "code"), js::gs(e, "typed")))
    }

    fn set_typed(&self, id: &str, typed: &str) {
        let o = self.o.up();
        let mut pending = o.pending.borrow_mut();
        if let Some((_, e)) = pending.iter_mut().find(|(k, _)| k == id) {
            e["typed"] = json!(typed);
        }
    }

    /// Un code est-il attendu ?
    pub fn has_current(&self) -> bool {
        self.current(None).is_some()
    }

    /// Chiffres saisis (un ou plusieurs) → { ok, filled, done } ; un chiffre faux arrête la saisie.
    pub fn digits(&self, digits: &str, task_id: Option<&str>, source: &str) -> J {
        let Some((id, code, mut typed)) = self.current(task_id) else { return json!({ "ok": false, "error": "aucun code attendu" }) };
        let o = self.o.up();
        let code: Vec<char> = code.chars().collect();
        for d in digits.chars().filter(|c| c.is_ascii_digit()) {
            let index = typed.chars().count();
            let good = code.get(index) == Some(&d);
            if good {
                typed.push(d);
                self.set_typed(&id, &typed);
            }
            let filled = typed.chars().count();
            o.emit(json!({ "type": "code_entry", "id": id, "index": index, "digit": d.to_string(), "ok": good, "filled": filled, "source": source }));
            if !good {
                return json!({ "ok": true, "filled": filled, "wrong": true });
            }
            if filled == code.len() {
                o.approve(&id);
                o.emit(json!({ "type": "say", "text": "Code validé." }));
                return json!({ "ok": true, "filled": filled, "done": true });
            }
        }
        json!({ "ok": true, "filled": typed.chars().count() })
    }

    /// Effacer le dernier chiffre (touche ⌫ ou Retour arrière).
    pub fn back(&self, task_id: Option<&str>) -> J {
        let Some((id, _, mut typed)) = self.current(task_id) else { return json!({ "ok": false }) };
        typed.pop();
        self.set_typed(&id, &typed);
        let n = typed.chars().count();
        self.o.up().emit(json!({ "type": "code_entry", "id": id, "index": n, "digit": null, "ok": null, "filled": n, "source": "clic" }));
        json!({ "ok": true, "filled": n })
    }

    /// Phrase dite pendant qu'un code est attendu : chiffres → saisie ; sinon None (phrase ordinaire).
    pub fn voice(&self, text: &str, meta: &J) -> Option<J> {
        self.current(None)?;
        let digits = spoken_digits(text).filter(|d| d.len() <= 4)?;
        let o = self.o.up();
        let spk = o.get_speaker_status();
        let recognized = js::truthy(&meta["speaker"]["name"]) && !js::gb(meta, "speakerUnknown");
        if js::gb(&spk, "enabled") && !js::arr(&spk["speakers"]).is_empty() && !recognized {
            o.record("validation_vocale_refusee", json!({ "motif": "voix non reconnue" }));
            return Some(json!({ "refused": true }));
        }
        Some(self.digits(&digits, None, "voix"))
    }
}
