// Suivi de colis : « suis le colis 6A12345678901 de La Poste ». Aucune clé de transporteur : toutes les 3 heures (et à la
// demande), l'IA lit la page de suivi du transporteur en tâche de fond (recherche web) et Jarvis annonce seulement quand
// le statut change. Un colis livré n'est plus suivi. Données dans data/colis.json (numéros et statuts, rien d'autre).

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use serde_json::{json, Value};

use super::shopping;
use super::OrchRef;
use crate::js::{self, J};

const MAX_PARCELS: usize = 30;
const CHECK_EVERY_MS: u64 = 3 * 3600 * 1000;

fn clean_number(s: &str) -> String {
    s.chars().filter(|c| c.is_ascii_alphanumeric()).collect::<String>().to_uppercase()
}

fn fold(s: &str) -> String {
    crate::re!(r"[^a-z0-9]+").replace_all(&js::fold_lower(s), " ").trim().to_string()
}

pub struct Parcels {
    o: OrchRef,
    file: PathBuf,
    log: Rc<dyn Fn(String)>,
    items: RefCell<Vec<J>>,
    /// Consignes en cours : (texte envoyé à l'IA, id du colis, demandé par l'utilisateur).
    pending: RefCell<Vec<(String, String, bool)>>,
    pub timer: js::Timer,
}

impl Parcels {
    pub fn new(o: OrchRef, file: PathBuf, log: Rc<dyn Fn(String)>) -> Self {
        let items = js::read_json(&file).map(|v| js::arr(&v)).unwrap_or_default();
        Self { o, file, log, items: RefCell::new(items), pending: RefCell::new(vec![]), timer: js::Timer::new() }
    }

    fn save(&self) {
        let _ = js::write_json(Path::new(&self.file), &Value::Array(self.items.borrow().clone()));
    }

    pub fn list(&self) -> Vec<J> {
        self.items.borrow().clone()
    }

    /// Ajoute un colis à suivre (numéro, transporteur facultatif).
    pub fn add(&self, number: &str, carrier: &str) -> Result<J, String> {
        let number = clean_number(number);
        if number.len() < 8 {
            return Err("Ce numéro de suivi est trop court.".into());
        }
        {
            let mut items = self.items.borrow_mut();
            if items.iter().any(|p| js::gs(p, "numero") == number) {
                return Err("Ce colis est déjà suivi.".into());
            }
            if items.len() >= MAX_PARCELS {
                return Err(format!("{MAX_PARCELS} colis au plus."));
            }
            items.push(json!({ "id": format!("c{}", js::now_ms()), "numero": number, "transporteur": js::slice_chars(carrier.trim(), 40), "statut": "", "livre": false, "ajouteLe": js::iso_now() }));
        }
        self.save();
        Ok(self.items.borrow().last().cloned().unwrap_or(Value::Null))
    }

    /// Retire un colis désigné par son numéro (ou la fin du numéro) ou son transporteur.
    pub fn remove(&self, reference: &str) -> Option<J> {
        let r = clean_number(reference);
        let words = fold(reference);
        let found = {
            let mut items = self.items.borrow_mut();
            let i = items.iter().position(|p| (!r.is_empty() && js::gs(p, "numero").ends_with(&r)) || (!words.is_empty() && fold(&js::gs(p, "transporteur")) == words))?;
            items.remove(i)
        };
        self.save();
        Some(found)
    }

    /// Phrase qui résume ce qu'on sait des colis suivis.
    /// « (relevé de 8 h 10) » : dernière lecture réussie du suivi, pour ne pas donner un vieux statut pour frais.
    fn checked_label(p: &J) -> String {
        let at = [js::gs(p, "verifieLe"), js::gs(p, "majLe")].into_iter().find_map(|d| js::parse_date_ms(&d));
        at.map(|ms| format!(" (relevé {})", super::local_skills::reading_label(ms))).unwrap_or_default()
    }

    pub fn summary(&self) -> String {
        let items = self.items.borrow();
        if items.is_empty() {
            return "Aucun colis n'est suivi.".into();
        }
        items
            .iter()
            .map(|p| {
                let who = if js::gs(p, "transporteur").is_empty() { String::new() } else { format!(" ({})", js::gs(p, "transporteur")) };
                let st = if js::gs(p, "statut").is_empty() { "pas encore de statut".to_string() } else { format!("{}{}", js::gs(p, "statut"), Self::checked_label(p)) };
                format!("Colis {}{who} : {st}{}", js::gs(p, "numero"), if js::gb(p, "livre") { " (livré)" } else { "" })
            })
            .collect::<Vec<_>>()
            .join(". ")
    }

    fn prompt(p: &J, tag: &str) -> String {
        [
            format!("Retrouve où en est le colis n° {}{}.", js::gs(p, "numero"), if js::gs(p, "transporteur").is_empty() { " (transporteur à identifier d'après le format du numéro)".to_string() } else { format!(" du transporteur {}", js::gs(p, "transporteur")) }),
            "Utilise la recherche web (WebSearch puis WebFetch) sur la page officielle de suivi du transporteur.".into(),
            "Ne devine ni n'invente jamais un statut : si tu ne l'as pas lu sur une page consultée, mets une chaîne vide.".into(),
            "Réponds uniquement par un objet JSON valide, sans texte autour : {\"statut\": \"dernier événement de suivi en une phrase courte en français, avec la date si elle est donnée\", \"livre\": true ou false}.".into(),
            format!("[colis-{tag}]"),
        ]
        .join(" ")
    }

    /// Demande à l'IA l'état d'un colis (tâche de fond). `explicit` : l'utilisateur l'a demandé, on annonce toujours le résultat.
    pub fn check(&self, id: &str, explicit: bool) -> bool {
        let Some(p) = self.items.borrow().iter().find(|p| js::gs(p, "id") == id && !js::gb(p, "livre")).cloned() else { return false };
        let Some(o) = self.o.upgrade() else { return false };
        if self.pending.borrow().iter().any(|(_, pid, _)| pid == id) {
            return true;
        }
        let prompt = Self::prompt(&p, &format!("{}-{}", js::now_ms(), js::rand_below(10_000)));
        self.pending.borrow_mut().push((prompt.clone(), id.to_string(), explicit));
        o.enqueue_ai(prompt, json!({ "background": true }));
        true
    }

    /// Vérifie tous les colis non livrés (minuteur et « où en est mon colis »).
    pub fn check_all(&self, explicit: bool) -> usize {
        let ids: Vec<String> = self.items.borrow().iter().filter(|p| !js::gb(p, "livre")).map(|p| js::gs(p, "id")).collect();
        ids.iter().filter(|id| self.check(id, explicit)).count()
    }

    /// Fin d'une tâche de fond : true si c'était l'un de nos contrôles.
    pub fn done(&self, text: &str, answer: &str, ok: bool) -> bool {
        let (id, explicit) = {
            let mut pending = self.pending.borrow_mut();
            let Some(i) = pending.iter().position(|(p, _, _)| p == text) else { return false };
            let (_, id, explicit) = pending.remove(i);
            (id, explicit)
        };
        let Some(o) = self.o.upgrade() else { return true };
        let data = if ok { shopping::read_offers_data(Path::new(""), answer) } else { None };
        let status = data.as_ref().map(|d| js::slice_chars(js::gs(d, "statut").trim(), 200)).unwrap_or_default();
        let delivered = data.as_ref().map(|d| js::gb(d, "livre")).unwrap_or(false);
        let (number, previous) = {
            let items = self.items.borrow();
            match items.iter().find(|p| js::gs(p, "id") == id) {
                Some(p) => (js::gs(p, "numero"), js::gs(p, "statut")),
                None => return true, // colis retiré entre-temps
            }
        };
        if status.is_empty() {
            if explicit {
                // Suivi illisible : le dernier statut connu, daté, plutôt que « rien ».
                let known = self.items.borrow().iter().find(|p| js::gs(p, "id") == id).filter(|_| !previous.is_empty()).map(Self::checked_label);
                let text = match known {
                    Some(when) => format!("Je n'arrive pas à relire le suivi du colis {number}. Dernier statut connu{when} : {previous}."),
                    None => format!("Je n'ai pas trouvé de statut fiable pour le colis {number}."),
                };
                o.emit(json!({ "type": "say", "text": text }));
            }
            return true;
        }
        let changed = status != previous;
        if let Some(p) = self.items.borrow_mut().iter_mut().find(|p| js::gs(p, "id") == id) {
            p["verifieLe"] = json!(js::iso_now());
            if changed || delivered {
                p["statut"] = json!(status);
                p["livre"] = json!(delivered);
                p["majLe"] = json!(js::iso_now());
            }
        }
        self.save();
        if changed || delivered {
            o.record("colis_statut", json!({ "colis": number, "livre": delivered }));
        }
        if changed || explicit {
            let prefix = if delivered { "Ton colis est livré" } else { "Ton colis" };
            let text = format!("{prefix} {number} : {status}.");
            if explicit {
                o.emit(json!({ "type": "say", "text": text }));
            } else {
                // Contrôle de fond : annonce spontanée, soumise aux heures calmes et à la sourdine.
                o.notify(json!({ "text": text, "title": "Colis", "source": "colis", "card": false }));
            }
        }
        (self.log)(format!("Colis {number} : {} ", if changed { "statut mis à jour" } else { "inchangé" }));
        true
    }

    /// Minuteur : contrôle périodique des colis non livrés.
    pub fn start(&self) {
        let w = self.o.clone();
        self.timer.every(CHECK_EVERY_MS, move || {
            if let Some(o) = w.upgrade() {
                o.parcels.check_all(false);
            }
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parcels() -> (Parcels, PathBuf) {
        let file = std::env::temp_dir().join(format!("jarvis-test-colis-{}-{}.json", js::now_ms(), js::rand_below(100000)));
        (Parcels::new(std::rc::Weak::new(), file.clone(), Rc::new(|_| {})), file)
    }

    #[test]
    fn ajout_doublon_et_retrait() {
        let (p, file) = parcels();
        assert!(p.add("6a 1234 5678 901", "La Poste").is_ok());
        assert_eq!(js::gs(&p.list()[0], "numero"), "6A12345678901");
        assert!(p.add("6A12345678901", "").is_err(), "doublon");
        assert!(p.add("123", "").is_err(), "trop court");
        assert!(p.summary().contains("pas encore de statut"));
        assert!(p.remove("78901").is_some(), "retrouvé par la fin du numéro");
        assert!(p.list().is_empty());
        let _ = std::fs::remove_file(file);
    }

    #[test]
    fn consigne_sans_invention() {
        let (p, file) = parcels();
        let c = p.add("CB123456789FR", "Chronopost").unwrap();
        let prompt = Parcels::prompt(&c, "t");
        assert!(prompt.contains("CB123456789FR") && prompt.contains("Chronopost") && prompt.contains("n'invente jamais"));
        let _ = std::fs::remove_file(file);
    }

    #[test]
    fn un_statut_connu_est_date() {
        let (p, file) = parcels();
        p.add("6A12345678901", "La Poste").unwrap();
        if let Some(c) = p.items.borrow_mut().first_mut() {
            c["statut"] = json!("En cours d'acheminement");
            c["verifieLe"] = json!(js::iso_now());
        }
        assert!(p.summary().contains("En cours d'acheminement (relevé de "), "{}", p.summary());
        let _ = std::fs::remove_file(file);
    }
}
