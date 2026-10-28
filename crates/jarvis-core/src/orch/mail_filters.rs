// Exclusions des annonces de nouveaux mails (orchestrator/mail-filters.js) : « n'annonce plus les mails
// de news@amazon.fr ». Chaque exclusion est une règle (data/mails-exclus.json) ET un souvenir de la
// mémoire (catégorie « mails ») ; la mémoire fait foi. Rien n'est jamais modifié dans la boîte mail.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use serde_json::{json, Value};

use super::{OrchRef, Up};
use crate::js::{self, J};

fn fold(s: &str) -> String {
    let t = js::fold_lower(s);
    crate::re!(r"\s+").replace_all(&t, " ").trim().to_string()
}

pub fn address_of(from: &str) -> String {
    crate::re!(r"<([^>]+)>")
        .captures(from)
        .or_else(|| crate::re!(r#"([^\s<>"]+@[^\s<>"]+)"#).captures(from))
        .map(|c| c[1].trim().to_lowercase())
        .unwrap_or_default()
}

pub fn name_of(from: &str) -> String {
    let n = crate::re!(r"<.*>").replace(from, "").replace('"', "").trim().to_string();
    if n.is_empty() {
        address_of(from)
    } else {
        n
    }
}

pub fn domain_of(address: &str) -> String {
    address.split('@').nth(1).unwrap_or("").to_lowercase()
}

/// Valeur dite ou cliquée → règle : adresse, domaine ou nom d'expéditeur.
pub fn rule_for(value: &str) -> Option<J> {
    let v = crate::re!(r#"^[«"“]\s*|\s*[»"”]$"#).replace_all(value.trim(), "");
    let v = crate::re!(r"[.!?]+$").replace(&v, "").trim().to_string();
    if v.is_empty() {
        return None;
    }
    let address = address_of(&v);
    if !address.is_empty() {
        let n = name_of(&v);
        return Some(json!({ "type": "from", "value": address, "label": if n != address { format!("{n} ({address})") } else { address.clone() } }));
    }
    let d = v.to_lowercase();
    let d = d.trim_start_matches('@');
    let domain = crate::re!(r"^(?:tout le domaine|le domaine|domaine)\s+").replace(d, "").to_string();
    if crate::re!(r"^[a-z0-9-]+(?:\.[a-z0-9-]+)+$").is_match(&domain) {
        return Some(json!({ "type": "domain", "value": domain, "label": format!("tout le domaine {domain}") }));
    }
    Some(json!({ "type": "sender", "value": fold(&v), "label": v }))
}

pub struct MailFilters {
    file: PathBuf,
    o: OrchRef,
    on_memory_change: Box<dyn Fn(&str, &J)>,
    log: Rc<dyn Fn(String)>,
    rules: RefCell<Vec<J>>,
}

impl MailFilters {
    pub fn new(file: PathBuf, o: OrchRef, on_memory_change: Box<dyn Fn(&str, &J)>, log: Rc<dyn Fn(String)>) -> Self {
        let rules = js::read_json(&file).map(|v| js::arr(&v).into_iter().filter(|r| js::truthy(&r["type"]) && js::truthy(&r["value"])).collect()).unwrap_or_default();
        Self { file, o, on_memory_change, log, rules: RefCell::new(rules) }
    }

    fn save(&self) {
        if let Err(e) = js::write_json(&self.file, &Value::Array(self.rules.borrow().clone())) {
            (self.log)(format!("exclusions de mails : {e}"));
        }
    }

    /// Règles actives : celles dont le souvenir a été effacé de la mémoire sont retirées.
    pub fn list(&self) -> Vec<J> {
        let ids: std::collections::HashSet<String> = self.o.up().memory.entries.borrow().iter().map(|e| js::gs(e, "id")).collect();
        let before = self.rules.borrow().len();
        self.rules.borrow_mut().retain(|r| !js::truthy(&r["memoryId"]) || ids.contains(&js::gs(r, "memoryId")));
        if self.rules.borrow().len() != before {
            self.save();
        }
        self.rules.borrow().clone()
    }

    fn memory_text(rule: &J) -> String {
        let label = js::gs(rule, "label");
        let what = match js::gs(rule, "type").as_str() {
            "keyword" => format!("les nouveaux mails dont l'objet contient « {label} »"),
            "domain" => format!("les nouveaux mails venant de {label}"),
            _ => format!("les nouveaux mails de {label}"),
        };
        format!("Mails : ne pas m'annoncer {what} — je ne veux pas être prévenu de ces mails (exclusion des annonces de nouveaux mails).")
    }

    /// { type, value, label } ou une valeur libre → { ok, rule, existed }.
    pub fn add(&self, input: &J) -> J {
        let rule = if let Some(s) = input.as_str() { rule_for(s) } else { Some(input.clone()) };
        let Some(rule) = rule.filter(|r| js::truthy(&r["value"])) else { return json!({ "ok": false }) };
        if let Some(existing) = self.list().into_iter().find(|r| r["type"] == rule["type"] && r["value"] == rule["value"]) {
            return json!({ "ok": true, "rule": existing, "existed": true });
        }
        let entry = self.o.up().memory.add(&Self::memory_text(&rule), "mails");
        let label = if js::truthy(&rule["label"]) { rule["label"].clone() } else { rule["value"].clone() };
        let saved = json!({ "type": rule["type"], "value": rule["value"], "label": label, "memoryId": entry.as_ref().map(|e| e["id"].clone()), "at": js::iso_now() });
        self.rules.borrow_mut().push(saved.clone());
        self.save();
        if let Some(e) = entry {
            (self.on_memory_change)("add", &e);
        }
        json!({ "ok": true, "rule": saved, "existed": false })
    }

    /// Retire les règles qui correspondent à ce qui est dit.
    pub fn remove(&self, spoken: &str) -> Vec<J> {
        let s = crate::re!(r#"^[«"“]\s*|\s*[»"”]$"#).replace_all(spoken, "");
        let want = fold(&crate::re!(r"[.!?]+$").replace(&s, ""));
        if want.is_empty() {
            return vec![];
        }
        let hit = |r: &J| {
            let v = js::gs(r, "value");
            let l = js::gs(r, "label");
            fold(&v) == want
                || fold(&l) == want
                || fold(&l).contains(&want)
                || fold(&v).contains(&want)
                || (js::gs(r, "type") == "from" && (domain_of(&v) == want || fold(v.split('@').next().unwrap_or("")) == want))
        };
        let removed: Vec<J> = self.list().into_iter().filter(|r| hit(r)).collect();
        if removed.is_empty() {
            return vec![];
        }
        self.rules.borrow_mut().retain(|r| !removed.contains(r));
        self.save();
        for r in &removed {
            if js::truthy(&r["memoryId"]) {
                if let Some(e) = self.o.up().memory.forget(&js::gs(r, "memoryId")) {
                    (self.on_memory_change)("forget", &e);
                }
            }
        }
        removed
    }

    /// Règle qui exclut ce mail.
    pub fn match_(&self, mail: &J) -> Option<J> {
        let from = js::gs(mail, "from");
        let address = address_of(&from);
        let domain = domain_of(&address);
        let name = fold(&name_of(&from));
        let subject = fold(&js::gs(mail, "subject"));
        self.list().into_iter().find(|r| {
            let v = js::gs(r, "value");
            match js::gs(r, "type").as_str() {
                "from" => !address.is_empty() && address == v,
                "domain" => !domain.is_empty() && (domain == v || domain.ends_with(&format!(".{v}"))),
                "sender" => (!name.is_empty() && name.contains(&v)) || (!address.is_empty() && address.contains(&crate::re!(r"\s+").replace_all(&v, "").into_owned())),
                "keyword" => subject.contains(&v),
                _ => false,
            }
        })
    }
}
