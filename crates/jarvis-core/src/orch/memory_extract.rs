// Mémoire automatique (idée reprise du projet Cornelius) : les phrases où l'utilisateur parle de lui restent en
// mémoire vive ; après quelques minutes de silence, Claude Haiku (sans outil) propose quoi retenir ou corriger, en
// citant les phrases qui le prouvent. Jarvis vérifie chaque proposition (preuves réelles, aucun secret, souvenir
// existant, échéance valide) avant de l'appliquer, puis oublie les phrases. Aucun oubli automatique : effacer un
// souvenir exige toujours un code. Réglage config.memoryAuto ; journal des analyses dans data/memoire-auto.json.

use std::cell::{Cell, RefCell};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use serde_json::json;

use super::memory::{self, Memory};
use super::{OrchRef, Up};
use crate::js::{self, J};

const MAX_PHRASES: usize = 40;
const MAX_PHRASE_CHARS: usize = 400;
const MAX_CHANGES: usize = 5;
const MAX_MEMORIES_SENT: usize = 120;
const JOURNAL_KEPT: usize = 20;
const CATEGORIES: &[&str] = &["identité", "préférences", "habitudes", "projets", "contacts", "lieu", "situation", "dialogue", "divers"];

const SYSTEM_PROMPT: &str = "Tu tiens la mémoire long terme d'un assistant personnel. Tu reçois les souvenirs actuels de \
l'utilisateur et des phrases qu'il vient de dire, numérotées. Propose seulement des faits durables ou passagers qu'il \
déclare lui-même sur lui : identité, préférences, habitudes, projets, contacts, lieu de vie, façon de lui parler. \
Ne retiens jamais : une question, une commande donnée à l'assistant, une supposition, une citation, un fait sur une \
autre personne qui ne le concerne pas, un mot de passe, un code, un numéro de carte ou tout autre secret, une \
information déjà présente dans les souvenirs. Une correction ne sert que si une phrase contredit un souvenir existant \
(donne son id). Retiens aussi une situation passagère utile pendant quelques jours (congés, déplacement, invité, \
traitement) avec expire_le : le dernier jour où elle reste vraie, au format AAAA-MM-JJ, calculé depuis la date du jour. Formule chaque fait en une phrase courte à la troisième personne \
(« Préfère le thé au café »). Chaque proposition cite les numéros des phrases qui la prouvent. Réponds uniquement \
par un objet JSON, sans texte autour : {\"ajouts\": [{\"texte\": \"…\", \"categorie\": \"…\", \"expire_le\": \"AAAA-MM-JJ ou vide\", \
\"preuves\": [1]}], \"corrections\": [{\"id\": \"…\", \"texte\": \"…\", \"expire_le\": \"…\", \"preuves\": [2]}]}. \
Rien à retenir : {\"ajouts\": [], \"corrections\": []}.";

/// Phrase où l'utilisateur parle de lui (première personne) : les autres ne quittent jamais le PC.
pub fn is_personal(text: &str) -> bool {
    let t = js::fold_lower(text);
    t.chars().count() >= 12
        && crate::re!(r"\b(?:je|j|moi|mon|ma|mes|m|me|nous|notre|nos|on)\b").is_match(&crate::re!(r"[^a-z0-9]+").replace_all(&t, " "))
}

/// Proposition refusée d'office : secret ou donnée sensible.
fn looks_secret(text: &str) -> bool {
    let t = js::fold_lower(text);
    crate::re!(r"mot de passe|password|\bmdp\b|code (?:secret|pin|d acces|d'acces|de (?:carte|validation|deverrouillage))|\biban\b|carte bancaire|\bcvv\b|cryptogramme|identifiant de connexion").is_match(&t)
        || crate::re!(r"\d[\d ]{11,}\d").is_match(&t)
}

fn category(raw: &str) -> String {
    let f = js::fold_lower(raw.trim());
    CATEGORIES.iter().find(|c| js::fold_lower(c) == f).map(|c| c.to_string()).unwrap_or_else(|| "divers".into())
}

/// Mots porteurs de sens (4 lettres et plus) d'un texte replié.
fn words(text: &str) -> Vec<String> {
    let t = js::fold_lower(text);
    crate::re!(r"[^a-z0-9]+").replace_all(&t, " ").split_whitespace().filter(|w| w.chars().count() >= 4).map(String::from).collect()
}

/// Un changement vérifié, prêt à appliquer.
#[derive(Debug, Clone, PartialEq)]
pub enum Change {
    Add { text: String, category: String, expires: Option<String> },
    Update { id: String, before: String, text: String, expires: Option<Option<String>> },
}

/// Vérifie la proposition du modèle : chaque élément doit citer des phrases réelles qui partagent au moins un mot
/// avec lui, ne contenir aucun secret, viser un souvenir existant (corrections) et porter une échéance valide.
/// Retourne les changements retenus et le motif de chaque refus.
pub fn validate(proposal: &J, phrases: &[String], memories: &[J]) -> (Vec<Change>, Vec<String>) {
    let mut kept = vec![];
    let mut rejected = vec![];
    let check = |item: &J| -> Result<(String, Option<String>), String> {
        let text = js::gs(item, "texte").trim().to_string();
        if text.is_empty() || text.chars().count() > 200 {
            return Err("texte vide ou trop long".into());
        }
        if looks_secret(&text) {
            return Err(format!("donnée sensible : {}", js::slice_chars(&text, 40)));
        }
        let proofs: Vec<usize> = js::arr(&item["preuves"]).iter().filter_map(|p| p.as_u64()).map(|p| p as usize).collect();
        if proofs.is_empty() || proofs.iter().any(|p| *p == 0 || *p > phrases.len()) {
            return Err(format!("sans preuve valable : {}", js::slice_chars(&text, 40)));
        }
        let said: Vec<String> = proofs.iter().flat_map(|p| words(&phrases[p - 1])).collect();
        if !words(&text).iter().any(|w| said.iter().any(|s| s.starts_with(&w[..w.len().min(5)]) || w.starts_with(&s[..s.len().min(5)]))) {
            return Err(format!("sans rapport avec les phrases citées : {}", js::slice_chars(&text, 40)));
        }
        let expires = memory::parse_expiry(&js::gs(item, "expire_le")).map_err(|e| format!("{e} : {}", js::slice_chars(&text, 40)))?;
        Ok((text, expires))
    };
    for item in js::arr(&proposal["ajouts"]) {
        match check(&item) {
            Ok((text, expires)) => {
                let known = memories.iter().any(|m| js::fold_lower(&js::gs(m, "text")) == js::fold_lower(&text));
                if known && expires.is_none() {
                    rejected.push(format!("déjà connu : {}", js::slice_chars(&text, 40)));
                } else {
                    kept.push(Change::Add { text, category: category(&js::gs(&item, "categorie")), expires });
                }
            }
            Err(e) => rejected.push(e),
        }
    }
    for item in js::arr(&proposal["corrections"]) {
        let id = js::gs(&item, "id");
        let Some(target) = memories.iter().find(|m| js::gs(m, "id") == id) else {
            rejected.push(format!("correction d'un souvenir inconnu ({id})"));
            continue;
        };
        if js::truthy(&target["pinned"]) {
            rejected.push("correction d'un souvenir épinglé (réglages)".into());
            continue;
        }
        match check(&item) {
            Ok((text, expires)) => {
                let before = js::gs(target, "text");
                let expires = (expires.as_deref().unwrap_or("") != js::gs(target, "expires")).then_some(expires);
                if text == before && expires.is_none() {
                    rejected.push(format!("correction sans changement : {}", js::slice_chars(&text, 40)));
                } else {
                    kept.push(Change::Update { id, before, text, expires });
                }
            }
            Err(e) => rejected.push(e),
        }
    }
    if kept.len() > MAX_CHANGES {
        rejected.push(format!("{} propositions au-delà de {MAX_CHANGES} ignorées", kept.len() - MAX_CHANGES));
        kept.truncate(MAX_CHANGES);
    }
    (kept, rejected)
}

pub struct MemoryExtract {
    o: OrchRef,
    file: PathBuf,
    log: Rc<dyn Fn(String)>,
    /// Phrases de l'utilisateur depuis la dernière analyse (mémoire vive seulement).
    phrases: RefCell<Vec<String>>,
    journal: RefCell<Vec<J>>,
    running: Cell<bool>,
    timer: js::Timer,
}

impl MemoryExtract {
    pub fn new(o: OrchRef, file: PathBuf, log: Rc<dyn Fn(String)>) -> Self {
        let journal = js::read_json(&file).map(|d| js::arr(&d["journal"])).unwrap_or_default();
        Self { o, file, log, phrases: RefCell::new(vec![]), journal: RefCell::new(journal), running: Cell::new(false), timer: js::Timer::new() }
    }

    fn settings(&self) -> (bool, u64) {
        let cfg = self.o.up().config.get("memoryAuto");
        let minutes = js::num(&cfg["idleMinutes"]).unwrap_or(5.0).clamp(1.0, 60.0) as u64;
        (cfg["enabled"] == json!(true), minutes)
    }

    /// Journal des dernières analyses (administration) : chiffres et souvenirs retenus, jamais les phrases.
    pub fn journal(&self) -> Vec<J> {
        self.journal.borrow().clone()
    }

    pub fn pending(&self) -> usize {
        self.phrases.borrow().len()
    }

    /// Désactivation : les phrases en attente sont oubliées sans être analysées.
    pub fn disable(&self) {
        self.timer.clear();
        self.phrases.borrow_mut().clear();
    }

    /// Phrase de l'utilisateur (voix reconnue ou clavier) : gardée si elle parle de lui, puis analyse après le silence.
    pub fn note(&self, text: &str) {
        let (enabled, minutes) = self.settings();
        if !enabled || !is_personal(text) {
            return;
        }
        {
            let mut p = self.phrases.borrow_mut();
            p.push(js::slice_chars(text.trim(), MAX_PHRASE_CHARS));
            if p.len() > MAX_PHRASES {
                p.remove(0);
            }
        }
        let w = self.o.clone();
        self.timer.set(minutes * 60_000, move || {
            if let Some(o) = w.upgrade() {
                js::spawn(async move {
                    o.memory_extract.analyze().await;
                });
            }
        });
    }

    fn push_journal(&self, entry: J) {
        {
            let mut j = self.journal.borrow_mut();
            j.insert(0, entry);
            j.truncate(JOURNAL_KEPT);
        }
        let _ = js::write_json(&self.file, &json!({ "journal": self.journal.borrow().clone() }));
    }

    /// Analyse les phrases en attente (minuterie de silence, ou « Analyser maintenant » dans l'administration).
    pub async fn analyze(&self) -> J {
        if self.running.get() {
            return json!({ "ok": false, "error": "analyse déjà en cours" });
        }
        let phrases = std::mem::take(&mut *self.phrases.borrow_mut());
        self.timer.clear();
        if phrases.is_empty() {
            return json!({ "ok": true, "statut": "rien à analyser" });
        }
        self.running.set(true);
        let o = self.o.up();
        let memories: Vec<J> = o.memory.list().into_iter().filter(|m| js::gs(m, "pinned").is_empty()).collect();
        let sent: Vec<J> = memories
            .iter()
            .rev()
            .take(MAX_MEMORIES_SENT)
            .map(|m| json!({ "id": m["id"], "categorie": m["category"], "texte": m["text"], "expire_le": m["expires"] }))
            .collect();
        let numbered: Vec<String> = phrases.iter().enumerate().map(|(i, p)| format!("{}. {p}", i + 1)).collect();
        let input = format!(
            "Aujourd'hui : {}.\n\nSouvenirs actuels :\n{}\n\nPhrases de l'utilisateur :\n{}",
            chrono::Local::now().format("%Y-%m-%d"),
            serde_json::to_string(&sent).unwrap_or_default(),
            numbered.join("\n")
        );
        let answer = o.quick_haiku(SYSTEM_PROMPT, &input, 90).await;
        self.running.set(false);
        let mut entry = json!({ "at": js::iso_now(), "phrases": phrases.len() });
        let proposal = match &answer {
            Ok(a) => super::shopping::read_offers_data(Path::new(""), a),
            Err(_) => None,
        };
        let Some(proposal) = proposal else {
            let why = answer.err().unwrap_or_else(|| "réponse illisible".into());
            entry["statut"] = json!("impossible");
            entry["motif"] = json!(why);
            (self.log)(format!("mémoire automatique : analyse impossible ({why})"));
            self.push_journal(entry.clone());
            return json!({ "ok": false, "entry": entry });
        };
        let (changes, rejected) = validate(&proposal, &phrases, &memories);
        let mut retained = vec![];
        for change in &changes {
            match change {
                Change::Add { text, category, expires } => {
                    if let Some(e) = o.memory.add_until(text, category, expires.as_deref()) {
                        o.record("memoire_ajout", json!({ "categorie": e["category"], "texte": e["text"], "expire": e["expires"], "origine": "automatique" }));
                        retained.push(json!({ "type": "ajout", "texte": Memory::text_with_expiry(&e), "categorie": e["category"] }));
                    }
                }
                Change::Update { id, before, text, expires } => {
                    if let Some(mut e) = o.memory.update(id, Some(text), None) {
                        if let Some(exp) = expires {
                            e = o.memory.set_expiry(id, exp.as_deref()).unwrap_or(e);
                        }
                        o.record("memoire_modification", json!({ "avant": before, "apres": e["text"], "categorie": e["category"], "origine": "automatique" }));
                        retained.push(json!({ "type": "correction", "texte": Memory::text_with_expiry(&e), "avant": before }));
                    }
                }
            }
        }
        if !retained.is_empty() {
            o.publish_memory();
            o.refresh_daemon();
            let o2 = o.clone();
            js::spawn(async move {
                let _ = o2.memory.index().await;
            });
        }
        entry["statut"] = json!(if retained.is_empty() { "rien à retenir" } else { "analysée" });
        entry["retenus"] = json!(retained);
        entry["ecartes"] = json!(rejected);
        (self.log)(format!("mémoire automatique : {} phrase(s), {} retenu(s), {} écarté(s)", phrases.len(), changes.len(), rejected.len()));
        self.push_journal(entry.clone());
        json!({ "ok": true, "entry": entry })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn phrases() -> Vec<String> {
        vec!["Je préfère le thé au café le matin".into(), "Mets de la musique".into(), "Je suis en congé jusqu'à vendredi".into()]
    }

    #[test]
    fn seules_les_phrases_a_la_premiere_personne_sont_gardees() {
        assert!(is_personal("Je préfère le thé au café"));
        assert!(is_personal("Mon frère s'appelle Paul"));
        assert!(!is_personal("Mets de la musique"));
        assert!(!is_personal("Quelle heure est-il ?"));
        assert!(!is_personal("je"));
    }

    #[test]
    fn une_proposition_prouvee_est_retenue() {
        let p = json!({ "ajouts": [{ "texte": "Préfère le thé au café", "categorie": "Préférences", "preuves": [1] }], "corrections": [] });
        let (kept, rejected) = validate(&p, &phrases(), &[]);
        assert_eq!(kept, vec![Change::Add { text: "Préfère le thé au café".into(), category: "préférences".into(), expires: None }]);
        assert!(rejected.is_empty());
    }

    #[test]
    fn sans_preuve_ou_hors_sujet_ou_secret_c_est_ecarte() {
        let p = json!({ "ajouts": [
            { "texte": "Aime le jazz", "preuves": [] },
            { "texte": "Aime le jazz", "preuves": [9] },
            { "texte": "Joue au tennis le samedi", "preuves": [2] },
            { "texte": "Son mot de passe est soleil", "preuves": [1] }
        ] });
        let (kept, rejected) = validate(&p, &phrases(), &[]);
        assert!(kept.is_empty());
        assert_eq!(rejected.len(), 4, "{rejected:?}");
    }

    #[test]
    fn une_situation_passagere_garde_son_echeance_et_une_date_passee_est_refusee() {
        let later = (chrono::Local::now().date_naive() + chrono::Duration::days(3)).format("%Y-%m-%d").to_string();
        let p = json!({ "ajouts": [
            { "texte": "En congé", "categorie": "habitudes", "expire_le": later, "preuves": [3] },
            { "texte": "En congé jusqu'à hier", "expire_le": "2001-01-01", "preuves": [3] }
        ] });
        let (kept, rejected) = validate(&p, &phrases(), &[]);
        assert_eq!(kept, vec![Change::Add { text: "En congé".into(), category: "habitudes".into(), expires: Some(later) }]);
        assert_eq!(rejected.len(), 1);
    }

    #[test]
    fn une_correction_vise_un_souvenir_existant_non_epingle() {
        let memories = vec![
            json!({ "id": "m1", "category": "préférences", "text": "Préfère le café" }),
            json!({ "id": "m2", "category": "dialogue", "text": "Profil", "pinned": "profile" }),
        ];
        let p = json!({ "corrections": [
            { "id": "m1", "texte": "Préfère le thé au café", "preuves": [1] },
            { "id": "m2", "texte": "Préfère le thé", "preuves": [1] },
            { "id": "m9", "texte": "Préfère le thé", "preuves": [1] }
        ] });
        let (kept, rejected) = validate(&p, &phrases(), &memories);
        assert_eq!(kept, vec![Change::Update { id: "m1".into(), before: "Préfère le café".into(), text: "Préfère le thé au café".into(), expires: None }]);
        assert_eq!(rejected.len(), 2);
    }

    #[test]
    fn un_fait_deja_connu_n_est_pas_redouble() {
        let memories = vec![json!({ "id": "m1", "category": "préférences", "text": "Préfère le thé au café" })];
        let p = json!({ "ajouts": [{ "texte": "préfère le thé au café", "preuves": [1] }] });
        let (kept, rejected) = validate(&p, &phrases(), &memories);
        assert!(kept.is_empty());
        assert_eq!(rejected.len(), 1);
    }
}
