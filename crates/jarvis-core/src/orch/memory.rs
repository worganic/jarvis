// Mémoire long terme de Jarvis (orchestrator/memory.js) : ce qu'il sait de l'utilisateur.
// Le fichier appartient à l'orchestrateur ; Claude Code n'y touche que par les outils MCP.
// Recherche par le sens (embeddings, embeddings.rs), vecteurs en cache à côté du fichier ; la
// recherche lexicale (trigrammes + thésaurus) reste le repli quand le modèle est indisponible.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;

use serde_json::{json, Value};
use sha1_smol::Sha1;

use super::embeddings::{cosine, Embedder};
use crate::js::{self, J};

pub const MAX_PROMPT_CHARS: usize = 4000;
/// Catégories (repliées) toujours présentes dans le prompt ; les autres se consultent par memoire_lire.
pub const PROMPT_CATEGORIES: &[&str] = &["dialogue", "identite", "lieu"];
pub const SEMANTIC_MIN: f32 = 0.18;
pub const SEMANTIC_RELATIVE: f32 = 0.6;

struct Concept {
    concepts: &'static [&'static str],
    terms: &'static [&'static str],
}

const CONCEPT_MAP: &[Concept] = &[
    Concept { concepts: &["boisson", "boire", "chaud", "chaude", "matin", "deguster"], terms: &["cafe", "the", "tisane", "chocolat", "expresso", "eau", "infusion", "deca", "cappuccino", "latte"] },
    Concept { concepts: &["nourriture", "manger", "repas", "dejeuner", "diner", "faim", "aliment", "plat", "gourmandise"], terms: &["restaurant", "recette", "cuisine", "pizza", "salade", "pain", "viande", "poisson", "legume", "fruit", "pates", "sandwich", "croissant", "croissants", "amande", "amandes", "viennoiserie", "boulangerie"] },
    Concept { concepts: &["sport", "activite", "entrainement", "exercice", "forme", "physique", "bouger"], terms: &["musculation", "course", "running", "jogging", "velo", "cyclisme", "foot", "football", "natation", "piscine", "gym", "fitness", "tennis"] },
    Concept { concepts: &["contact", "coordonnees", "joindre", "ecrire", "appeler", "adresse", "communiquer", "message", "messagerie", "courrier"], terms: &["email", "mail", "telephone", "portable", "numero", "domicile", "bureau", "courriel", "postal", "gmail", "outlook", "boite", "inbox"] },
    Concept { concepts: &["musique", "ecouter", "son", "audio", "morceau", "titre"], terms: &["chanson", "artiste", "album", "playlist", "spotify", "deezer", "youtube", "groupe", "chanteur", "concert"] },
    Concept { concepts: &["travail", "bureau", "projet", "entreprise", "boulot", "profession"], terms: &["code", "developpement", "programmation", "client", "reunion", "mission", "agenda", "rendez-vous", "contrat", "societe", "collegue"] },
    Concept { concepts: &["ordinateur", "machine", "pc", "materiel", "config", "hardware"], terms: &["ecran", "moniteur", "processeur", "cpu", "gpu", "carte", "graphique", "clavier", "souris", "disque", "ssd", "ram", "memoire", "windows"] },
    Concept { concepts: &["voiture", "trajet", "deplacement", "transport", "voyage", "partir"], terms: &["vehicule", "auto", "automobile", "train", "sncf", "gare", "metro", "vol", "avion", "aeroport", "peage", "route", "autoroute"] },
    Concept { concepts: &["famille", "proche", "ami", "entourage", "relation"], terms: &["enfant", "fils", "fille", "femme", "epouse", "mari", "conjoint", "parent", "mere", "pere", "frere", "soeur", "copain", "ami"] },
    Concept { concepts: &["sante", "medical", "soin", "corps", "maladie", "souffrance"], terms: &["medecin", "docteur", "medicament", "allergie", "traitement", "pharmacie", "ordonnance", "hopital", "dentiste", "lunettes"] },
];

/// Mots normalisés (minuscules, sans accents, plus d'une lettre).
pub fn normalize(text: &str) -> Vec<String> {
    let t = js::fold_lower(text);
    let t = crate::re!(r"[^a-z0-9\s]").replace_all(&t, " ");
    t.split_whitespace().filter(|w| w.chars().count() > 1).map(String::from).collect()
}

pub(crate) type Vector = HashMap<String, f64>;

// Trigrammes de caractères et mots entiers surpondérés (poids 3).
pub(crate) fn text_to_vector(text: &str) -> Vector {
    let t = js::fold_lower(text);
    let clean = crate::re!(r"[^a-z0-9]").replace_all(&t, " ").trim().to_string();
    let mut vec = Vector::new();
    if clean.is_empty() {
        return vec;
    }
    let chars: Vec<char> = clean.chars().collect();
    if chars.len() >= 3 {
        for i in 0..=chars.len() - 3 {
            let gram: String = chars[i..i + 3].iter().collect();
            *vec.entry(gram).or_insert(0.0) += 1.0;
        }
    }
    for w in clean.split_whitespace().filter(|w| w.chars().count() > 2) {
        *vec.entry(format!("w_{w}")).or_insert(0.0) += 3.0;
    }
    vec
}

pub(crate) fn cosine_similarity(a: &Vector, b: &Vector) -> f64 {
    if a.is_empty() || b.is_empty() {
        return 0.0;
    }
    let mut dot = 0.0;
    let mut na = 0.0;
    for (k, va) in a {
        na += va * va;
        if let Some(vb) = b.get(k) {
            dot += va * vb;
        }
    }
    let nb: f64 = b.values().map(|v| v * v).sum();
    if na == 0.0 || nb == 0.0 {
        return 0.0;
    }
    dot / (na.sqrt() * nb.sqrt())
}

fn expand_query(words: &[String]) -> Vec<String> {
    let mut out: Vec<String> = vec![];
    let mut seen = HashSet::new();
    let mut push = |w: &str, out: &mut Vec<String>| {
        if seen.insert(w.to_string()) {
            out.push(w.to_string());
        }
    };
    for w in words {
        push(w, &mut out);
    }
    for w in words {
        for g in CONCEPT_MAP {
            if g.concepts.contains(&w.as_str()) {
                for t in g.terms {
                    push(t, &mut out);
                }
            } else if g.terms.contains(&w.as_str()) {
                for c in g.concepts {
                    push(c, &mut out);
                }
            }
        }
    }
    out
}

/// Date d'expiration d'un souvenir temporaire : dernier jour où il reste vrai (« AAAA-MM-JJ »).
/// Accepte aussi « JJ/MM/AAAA ». Vide → souvenir durable. Une date passée est refusée.
pub fn parse_expiry(raw: &str) -> Result<Option<String>, String> {
    let t = raw.trim();
    if t.is_empty() {
        return Ok(None);
    }
    let date = chrono::NaiveDate::parse_from_str(t, "%Y-%m-%d")
        .or_else(|_| chrono::NaiveDate::parse_from_str(t, "%d/%m/%Y"))
        .map_err(|_| format!("date d'expiration illisible : {t} (attendu AAAA-MM-JJ)"))?;
    if date < chrono::Local::now().date_naive() {
        return Err(format!("date d'expiration déjà passée : {t}"));
    }
    Ok(Some(date.format("%Y-%m-%d").to_string()))
}

/// Souvenir temporaire arrivé à échéance : son dernier jour est passé.
fn is_expired(e: &J, today: &str) -> bool {
    let exp = js::gs(e, "expires");
    !exp.is_empty() && exp.as_str() < today
}

/// « 3 novembre » (année ajoutée si ce n'est pas l'année en cours).
pub fn expiry_label(exp: &str) -> String {
    use chrono::Datelike;
    const MOIS: [&str; 12] = ["janvier", "février", "mars", "avril", "mai", "juin", "juillet", "août", "septembre", "octobre", "novembre", "décembre"];
    let Ok(d) = chrono::NaiveDate::parse_from_str(exp, "%Y-%m-%d") else { return exp.to_string() };
    let day = if d.day() == 1 { "1er".to_string() } else { d.day().to_string() };
    if d.year() == chrono::Local::now().year() {
        format!("{day} {}", MOIS[d.month0() as usize])
    } else {
        format!("{day} {} {}", MOIS[d.month0() as usize], d.year())
    }
}

struct Lexical {
    score: f64,
    exact_matches: usize,
}

struct EmbItem {
    hash: String,
    v: Vec<f32>,
}

pub struct Memory {
    pub file: PathBuf,
    pub entries: RefCell<Vec<J>>,
    vectors: RefCell<HashMap<String, Vector>>,
    vector_file: PathBuf,
    embedder: RefCell<Option<Rc<Embedder>>>,
    embeddings: RefCell<Option<(String, HashMap<String, EmbItem>)>>,
    indexing: Cell<bool>,
    pub on_semantic_error: RefCell<Option<Rc<dyn Fn(String)>>>,
}

impl Memory {
    pub fn new(file: PathBuf) -> Self {
        let vector_file = file.parent().map(|d| d.join("memory-vectors.json")).unwrap_or_else(|| PathBuf::from("memory-vectors.json"));
        let m = Self {
            file,
            entries: RefCell::new(vec![]),
            vectors: RefCell::new(HashMap::new()),
            vector_file,
            embedder: RefCell::new(None),
            embeddings: RefCell::new(None),
            indexing: Cell::new(false),
            on_semantic_error: RefCell::new(None),
        };
        m.load();
        m
    }

    pub fn load(&self) {
        match js::read_json(&self.file) {
            Some(Value::Array(a)) => {
                *self.entries.borrow_mut() = a;
                self.rebuild_vectors();
                self.purge_expired();
            }
            _ => {
                self.entries.borrow_mut().clear();
                self.vectors.borrow_mut().clear();
            }
        }
    }

    fn corpus(e: &J) -> String {
        format!("{} {}", js::gs(e, "category"), js::gs(e, "text"))
    }

    fn rebuild_vectors(&self) {
        let mut v = self.vectors.borrow_mut();
        v.clear();
        for e in self.entries.borrow().iter() {
            v.insert(js::gs(e, "id"), text_to_vector(&Self::corpus(e)));
        }
    }

    pub fn save(&self) {
        let _ = js::write_json(&self.file, &Value::Array(self.entries.borrow().clone()));
    }

    pub fn len(&self) -> usize {
        self.entries.borrow().len()
    }

    pub fn list(&self) -> Vec<J> {
        self.purge_expired();
        self.entries.borrow().clone()
    }

    /// Retire les souvenirs temporaires échus. Retourne ceux qui viennent d'être oubliés.
    pub fn purge_expired(&self) -> Vec<J> {
        let today = chrono::Local::now().format("%Y-%m-%d").to_string();
        if !self.entries.borrow().iter().any(|e| is_expired(e, &today)) {
            return vec![];
        }
        let all = std::mem::take(&mut *self.entries.borrow_mut());
        let (gone, kept): (Vec<J>, Vec<J>) = all.into_iter().partition(|e| is_expired(e, &today));
        *self.entries.borrow_mut() = kept;
        {
            let mut v = self.vectors.borrow_mut();
            for e in &gone {
                v.remove(&js::gs(e, "id"));
            }
        }
        self.save();
        gone
    }

    /// Fixe (AAAA-MM-JJ, déjà validée) ou retire (None) la date d'expiration d'un souvenir.
    pub fn set_expiry(&self, id: &str, expires: Option<&str>) -> Option<J> {
        let updated = {
            let mut entries = self.entries.borrow_mut();
            let entry = entries.iter_mut().find(|e| js::gs(e, "id") == id)?;
            let obj = entry.as_object_mut()?;
            match expires {
                Some(d) => obj.insert("expires".into(), json!(d)),
                None => obj.remove("expires"),
            };
            entry.clone()
        };
        self.save();
        Some(updated)
    }

    /// Efface intégralement la mémoire locale et le cache vectoriel.
    pub fn clear(&self) {
        self.entries.borrow_mut().clear();
        self.vectors.borrow_mut().clear();
        *self.embeddings.borrow_mut() = None;
        self.save();
        let _ = std::fs::remove_file(&self.vector_file);
    }

    /// Les souvenirs épinglés (profil de dialogue) ne comptent pas : ils viennent des réglages.
    pub fn is_empty(&self) -> bool {
        !self.entries.borrow().iter().any(|e| !js::truthy(js::get(e, "pinned")))
    }

    fn new_id() -> String {
        format!("m{}{}", to_base36(js::now_ms() as u64), js::rand_below(1000))
    }

    pub fn add(&self, text: &str, category: &str) -> Option<J> {
        self.add_until(text, category, None)
    }

    /// Ajout avec une date d'expiration facultative (validée par `parse_expiry`).
    pub fn add_until(&self, text: &str, category: &str, expires: Option<&str>) -> Option<J> {
        self.purge_expired();
        let clean = text.trim();
        if clean.is_empty() {
            return None;
        }
        let existing = self.entries.borrow().iter().find(|e| js::gs(e, "text").to_lowercase() == clean.to_lowercase()).cloned();
        if let Some(existing) = existing {
            // Le même fait redit avec une autre échéance : la nouvelle date remplace l'ancienne.
            if let Some(d) = expires.filter(|d| js::gs(&existing, "expires") != *d) {
                return self.set_expiry(&js::gs(&existing, "id"), Some(d));
            }
            return Some(existing);
        }
        let cat = if category.trim().is_empty() { "divers".to_string() } else { category.trim().to_lowercase() };
        let mut entry = json!({ "id": Self::new_id(), "category": cat, "text": clean, "createdAt": js::iso_now() });
        if let Some(d) = expires {
            entry["expires"] = json!(d);
        }
        self.vectors.borrow_mut().insert(js::gs(&entry, "id"), text_to_vector(&Self::corpus(&entry)));
        self.entries.borrow_mut().push(entry.clone());
        self.save();
        Some(entry)
    }

    pub fn forget(&self, reference: &str) -> Option<J> {
        let key = reference.trim().to_lowercase();
        if key.is_empty() {
            return None;
        }
        let removed = {
            let mut entries = self.entries.borrow_mut();
            let idx = entries.iter().position(|e| js::gs(e, "id").to_lowercase() == key || js::gs(e, "text").to_lowercase() == key)?;
            entries.remove(idx)
        };
        self.vectors.borrow_mut().remove(&js::gs(&removed, "id"));
        self.save();
        Some(removed)
    }

    /// Retire le souvenir épinglé `pin` (liste supprimée…). Vrai s'il existait.
    pub fn forget_pinned(&self, pin: &str) -> bool {
        let id = self.entries.borrow().iter().find(|e| js::gs(e, "pinned") == pin).map(|e| js::gs(e, "id"));
        id.is_some_and(|id| self.forget(&id).is_some())
    }

    /// Modification d'un souvenir (administration) : texte et/ou catégorie.
    pub fn update(&self, id: &str, text: Option<&str>, category: Option<&str>) -> Option<J> {
        let updated = {
            let mut entries = self.entries.borrow_mut();
            let entry = entries.iter_mut().find(|e| js::gs(e, "id") == id)?;
            if let Some(t) = text {
                let clean = t.trim();
                if clean.is_empty() {
                    return None;
                }
                entry["text"] = json!(clean);
            }
            if let Some(c) = category {
                let c = c.trim().to_lowercase();
                entry["category"] = json!(if c.is_empty() { "divers".to_string() } else { c });
            }
            entry["updatedAt"] = json!(js::iso_now());
            entry.clone()
        };
        self.vectors.borrow_mut().insert(id.to_string(), text_to_vector(&Self::corpus(&updated)));
        self.save();
        Some(updated)
    }

    /// Souvenir « épinglé » unique, réécrit à chaque changement (profil de dialogue, projets).
    pub fn upsert_pinned(&self, pin: &str, text: &str, category: &str) -> Option<J> {
        let clean = text.trim();
        let existing = self.entries.borrow().iter().find(|e| js::gs(e, "pinned") == pin).cloned();
        if clean.is_empty() {
            if let Some(e) = existing {
                self.forget(&js::gs(&e, "id"));
            }
            return None;
        }
        if let Some(e) = existing {
            if js::gs(&e, "text") == clean && js::gs(&e, "category") == category {
                return Some(e);
            }
            return self.update(&js::gs(&e, "id"), Some(clean), Some(category));
        }
        let cat = if category.is_empty() { "divers".to_string() } else { category.to_lowercase() };
        let entry = json!({ "id": Self::new_id(), "category": cat, "text": clean, "pinned": pin, "createdAt": js::iso_now() });
        self.vectors.borrow_mut().insert(js::gs(&entry, "id"), text_to_vector(&Self::corpus(&entry)));
        self.entries.borrow_mut().push(entry.clone());
        self.save();
        Some(entry)
    }

    // Score lexical (repli sans modèle) : trigrammes, thésaurus et recouvrement exact.
    fn lexical_scores(&self, query: &str) -> Option<Vec<(J, Lexical)>> {
        let words = normalize(query);
        if words.is_empty() {
            return None;
        }
        let expanded = expand_query(&words).join(" ");
        let qv = text_to_vector(&expanded);
        let vectors = self.vectors.borrow();
        Some(
            self.entries
                .borrow()
                .iter()
                .map(|entry| {
                    let ev = vectors.get(&js::gs(entry, "id")).cloned().unwrap_or_else(|| text_to_vector(&Self::corpus(entry)));
                    let cos = cosine_similarity(&qv, &ev);
                    let target = normalize(&Self::corpus(entry));
                    let exact = words.iter().filter(|w| target.iter().any(|t| t.starts_with(w.as_str()))).count();
                    let exact_score = exact as f64 / (words.len().max(1)) as f64;
                    (entry.clone(), Lexical { score: cos * 0.6 + exact_score * 0.4, exact_matches: exact })
                })
                .collect(),
        )
    }

    /// Recherche lexicale synchrone.
    pub fn search(&self, query: &str, limit: usize) -> Vec<J> {
        self.purge_expired();
        let Some(mut scored) = self.lexical_scores(query) else {
            let e = self.entries.borrow();
            let skip = e.len().saturating_sub(limit);
            return e.iter().skip(skip).cloned().collect();
        };
        scored.retain(|(_, l)| l.score > 0.08 || l.exact_matches > 0);
        scored.sort_by(|a, b| b.1.score.partial_cmp(&a.1.score).unwrap_or(std::cmp::Ordering::Equal));
        scored.into_iter().take(limit).map(|(e, _)| e).collect()
    }

    // --- Mémoire sémantique par embeddings -------------------------------------------------------

    pub fn attach_embedder(&self, embedder: Option<Rc<Embedder>>) {
        *self.embedder.borrow_mut() = embedder;
    }

    fn passage_text(e: &J) -> String {
        format!("{} : {}", js::gs(e, "category"), js::gs(e, "text"))
    }

    fn entry_hash(e: &J) -> String {
        let mut h = Sha1::new();
        h.update(Self::passage_text(e).as_bytes());
        h.digest().to_string()[..16].to_string()
    }

    fn load_embeddings(&self) {
        if self.embeddings.borrow().is_some() {
            return;
        }
        let model = self.embedder.borrow().as_ref().map(|e| e.model.clone()).unwrap_or_default();
        let mut items = HashMap::new();
        if let Some(raw) = js::read_json(&self.vector_file) {
            if js::gs(&raw, "model") == model {
                if let Some(Value::Object(o)) = raw.get("items") {
                    for (id, item) in o {
                        let v: Vec<f32> = item["v"].as_array().map(|a| a.iter().map(|x| x.as_f64().unwrap_or(0.0) as f32).collect()).unwrap_or_default();
                        items.insert(id.clone(), EmbItem { hash: js::gs(item, "hash"), v });
                    }
                }
            }
        }
        *self.embeddings.borrow_mut() = Some((model, items));
    }

    fn save_embeddings(&self) {
        let emb = self.embeddings.borrow();
        let Some((model, items)) = emb.as_ref() else { return };
        let mut out = serde_json::Map::new();
        for (id, item) in items {
            let v: Vec<f64> = item.v.iter().map(|x| ((*x as f64) * 1e6).round() / 1e6).collect();
            out.insert(id.clone(), json!({ "hash": item.hash, "v": v }));
        }
        if let Some(dir) = self.vector_file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(&self.vector_file, serde_json::to_string(&json!({ "model": model, "items": Value::Object(out) })).unwrap_or_default());
    }

    /// Calcule les vecteurs manquants (souvenirs nouveaux ou modifiés). Retourne leur nombre.
    pub async fn index(&self) -> Result<usize, String> {
        let embedder = self.embedder.borrow().clone();
        let Some(embedder) = embedder else { return Ok(0) };
        if self.entries.borrow().is_empty() {
            return Ok(0);
        }
        if self.indexing.get() {
            while self.indexing.get() {
                js::sleep_ms(50).await;
            }
            return Ok(0);
        }
        self.indexing.set(true);
        let res = async {
            self.load_embeddings();
            let ids: HashSet<String> = self.entries.borrow().iter().map(|e| js::gs(e, "id")).collect();
            let mut changed = false;
            {
                let mut emb = self.embeddings.borrow_mut();
                let items = &mut emb.as_mut().unwrap().1;
                let before = items.len();
                items.retain(|id, _| ids.contains(id));
                changed |= items.len() != before;
            }
            let missing: Vec<J> = {
                let emb = self.embeddings.borrow();
                let items = &emb.as_ref().unwrap().1;
                self.entries.borrow().iter().filter(|e| items.get(&js::gs(e, "id")).map(|i| i.hash != Self::entry_hash(e)).unwrap_or(true)).cloned().collect()
            };
            if !missing.is_empty() {
                let vectors = embedder.embed(missing.iter().map(Self::passage_text).collect(), "passage").await?;
                let mut emb = self.embeddings.borrow_mut();
                let items = &mut emb.as_mut().unwrap().1;
                for (entry, v) in missing.iter().zip(vectors) {
                    items.insert(js::gs(entry, "id"), EmbItem { hash: Self::entry_hash(entry), v });
                }
                changed = true;
            }
            if changed {
                self.save_embeddings();
            }
            Ok(missing.len())
        }
        .await;
        self.indexing.set(false);
        res
    }

    /// Recherche par le sens : 75 % similarité sémantique + 25 % score lexical ; repli lexical sans modèle.
    pub async fn search_semantic(&self, query: &str, limit: usize) -> Vec<J> {
        self.purge_expired();
        let text = query.trim().to_string();
        let embedder = self.embedder.borrow().clone();
        let Some(embedder) = embedder else { return self.search(&text, limit) };
        if text.is_empty() || self.entries.borrow().is_empty() {
            return self.search(&text, limit);
        }
        let res: Result<Vec<J>, String> = async {
            self.index().await?;
            let q = embedder.embed(vec![text.clone()], "query").await?.into_iter().next().unwrap_or_default();
            self.load_embeddings();
            let lexical: HashMap<String, Lexical> = self.lexical_scores(&text).unwrap_or_default().into_iter().map(|(e, l)| (js::gs(&e, "id"), l)).collect();
            let emb = self.embeddings.borrow();
            let items = &emb.as_ref().unwrap().1;
            let mut rows: Vec<(J, f32, usize, f64)> = self
                .entries
                .borrow()
                .iter()
                .filter_map(|e| {
                    let id = js::gs(e, "id");
                    let item = items.get(&id)?;
                    let semantic = cosine(&q, &item.v);
                    let (ls, lx) = lexical.get(&id).map(|l| (l.score, l.exact_matches)).unwrap_or((0.0, 0));
                    Some((e.clone(), semantic, lx, semantic as f64 * 0.75 + ls * 0.25))
                })
                .collect();
            let best = rows.iter().fold(0f32, |m, r| m.max(r.1));
            let floor = SEMANTIC_MIN.max(best * SEMANTIC_RELATIVE);
            rows.retain(|r| r.1 >= floor || r.2 > 0);
            rows.sort_by(|a, b| b.3.partial_cmp(&a.3).unwrap_or(std::cmp::Ordering::Equal));
            Ok(rows.into_iter().take(limit).map(|r| r.0).collect())
        }
        .await;
        match res {
            Ok(v) => v,
            Err(err) => {
                if let Some(cb) = self.on_semantic_error.borrow().clone() {
                    cb(err);
                }
                self.search(&text, limit)
            }
        }
    }

    /// Chiffres de la mémoire pour la page d'aide : jamais le texte des souvenirs.
    pub fn stats(&self) -> J {
        let entries = self.entries.borrow();
        let mut categories: std::collections::BTreeMap<String, usize> = std::collections::BTreeMap::new();
        for e in entries.iter() {
            *categories.entry(js::gs(e, "category")).or_insert(0) += 1;
        }
        let pinned = entries.iter().filter(|e| !js::gs(e, "pinned").is_empty()).count();
        let temporary = entries.iter().filter(|e| !js::gs(e, "expires").is_empty()).count();
        let total = entries.len();
        drop(entries);
        self.load_embeddings();
        let emb = self.embeddings.borrow();
        let (vectors, dim) = emb.as_ref().map(|(_, items)| (items.len(), items.values().next().map(|i| i.v.len()).unwrap_or(0))).unwrap_or((0, 0));
        json!({
            "souvenirs": total, "epingles": pinned, "temporaires": temporary, "categories": categories, "vecteurs": vectors, "dimension": dim,
            "indexe": vectors >= total && total > 0, "fichier": self.file.file_name().map(|f| f.to_string_lossy().to_string()),
            "fichierVecteurs": self.vector_file.file_name().map(|f| f.to_string_lossy().to_string())
        })
    }

    /// Texte d'un souvenir, suivi de son échéance s'il est temporaire.
    pub fn text_with_expiry(e: &J) -> String {
        let exp = js::gs(e, "expires");
        if exp.is_empty() {
            js::gs(e, "text")
        } else {
            format!("{} (jusqu'au {})", js::gs(e, "text"), expiry_label(&exp))
        }
    }

    /// Souvenir toujours donné dans le prompt : préférences de dialogue, identité, lieu de vie, et
    /// toute situation temporaire en cours. Le reste se consulte à la demande (memoire_lire), comme
    /// dans Cornelius : prompt plus court et plus stable d'une session à l'autre.
    fn always_in_prompt(e: &J) -> bool {
        let cat = js::fold_lower(&js::gs(e, "category"));
        PROMPT_CATEGORIES.contains(&cat.as_str()) || !js::gs(e, "expires").is_empty()
    }

    /// Bloc injecté dans le prompt système : l'essentiel de ce que Jarvis sait, et ce qu'il peut consulter.
    pub fn prompt_block(&self) -> String {
        self.purge_expired();
        let entries = self.entries.borrow();
        let visible: Vec<&J> = entries.iter().filter(|e| js::gs(e, "pinned") != "profile").collect();
        if visible.is_empty() {
            return "Tu ne sais encore rien de personnel sur lui.".into();
        }
        let (essential, on_demand): (Vec<&J>, Vec<&J>) = visible.into_iter().partition(|e| Self::always_in_prompt(e));
        // Les préférences de dialogue passent en tête : elles ne doivent jamais être coupées.
        let mut groups: Vec<(String, Vec<String>)> = vec![("dialogue".into(), vec![])];
        for e in &essential {
            let cat = js::gs(e, "category");
            let text = Self::text_with_expiry(e);
            match groups.iter_mut().find(|(c, _)| *c == cat) {
                Some(g) => g.1.push(text),
                None => groups.push((cat, vec![text])),
            }
        }
        if groups[0].1.is_empty() {
            groups.remove(0);
        }
        let mut block = String::new();
        let mut shown = 0;
        for (cat, items) in &groups {
            let section = format!("{} : {}\n", cat, items.join(" ; "));
            if block.chars().count() + section.chars().count() > MAX_PROMPT_CHARS {
                break;
            }
            block.push_str(&section);
            shown += items.len();
        }
        let cut = essential.len() - shown;
        let mut counts: Vec<(String, usize)> = vec![];
        for e in on_demand.iter() {
            let cat = js::gs(e, "category");
            match counts.iter_mut().find(|(c, _)| *c == cat) {
                Some(c) => c.1 += 1,
                None => counts.push((cat, 1)),
            }
        }
        if !counts.is_empty() || cut > 0 {
            let total = on_demand.len() + cut;
            let detail = counts.iter().map(|(c, n)| format!("{c} {n}")).collect::<Vec<_>>().join(", ");
            block.push_str(&format!(
                "Autres souvenirs à consulter ({total}{}) : appelle memoire_lire avec le sujet de la demande avant de répondre à une question qui dépend de ses goûts, habitudes, contacts ou projets.\n",
                if detail.is_empty() { String::new() } else { format!(" : {detail}") }
            ));
        }
        block.trim().to_string()
    }

    /// Prénom ou nom de l'utilisateur mémorisé.
    pub fn get_user_name(&self) -> Option<String> {
        let entries = self.entries.borrow();
        let re1 = crate::re!(r"(?i)(?:(?:l'utilisateur|je)\s+(?:s'appelle|m'appelle|me prénomme|me nomme)|(?:mon\s+)?(?:prénom|prenom)\s+(?:est|c'est))\s+([A-ZÀ-ÖØ-ß][a-zà-öø-ÿ\-]+)");
        let re2 = crate::re!(r"(?i)(?:mon\s+nom\s+(?:est|c'est))\s+([A-ZÀ-ÖØ-ß][a-zà-öø-ÿ\-]+)");
        for e in entries.iter().rev() {
            let txt = js::gs(e, "text");
            if let Some(m) = re1.captures(&txt) {
                return Some(m[1].trim().to_string());
            }
            if let Some(m) = re2.captures(&txt) {
                return Some(m[1].trim().to_string());
            }
        }
        const STOP: &[&str] = &[
            "mon", "ma", "mes", "ton", "ta", "tes", "son", "sa", "ses", "le", "la", "les", "un", "une", "des", "je", "tu", "il", "elle", "on",
            "nous", "vous", "ils", "elles", "utilisateur", "user", "monsieur", "madame", "anniversaire", "date", "naissance", "ne", "né", "nee", "née",
        ];
        let label = crate::re!(r"(?i)(?:prénom|prenom|nom)\s*:\s*([A-ZÀ-ÖØ-ß][a-zà-öø-ÿ\-]+)");
        let word = crate::re!(r"\b([A-ZÀ-ÖØ-ß][a-zà-öø-ÿ\-]+)\b");
        for e in entries.iter().rev() {
            let cat = js::gs(e, "category").to_lowercase();
            if cat == "identite" || cat == "identité" {
                let txt = js::gs(e, "text");
                if let Some(m) = label.captures(&txt) {
                    return Some(m[1].trim().to_string());
                }
                for m in word.find_iter(&txt) {
                    if !STOP.contains(&m.as_str().to_lowercase().as_str()) {
                        return Some(m.as_str().trim().to_string());
                    }
                }
            }
        }
        None
    }

    /// Date d'anniversaire mémorisée : (jour, mois 0-11, année).
    pub fn get_birthday(&self) -> Option<(u32, u32, Option<i32>)> {
        let month = |m: &str| -> Option<u32> {
            Some(match m {
                "janvier" => 0,
                "fevrier" | "février" => 1,
                "mars" => 2,
                "avril" => 3,
                "mai" => 4,
                "juin" => 5,
                "juillet" => 6,
                "aout" | "août" => 7,
                "septembre" => 8,
                "octobre" => 9,
                "novembre" => 10,
                "decembre" | "décembre" => 11,
                _ => return None,
            })
        };
        for e in self.entries.borrow().iter().rev() {
            let txt = js::gs(e, "text").to_lowercase();
            if crate::re!(r"anniversaire|date de naissance|ne le|nee le|né le|née le").is_match(&txt) {
                if let Some(m) = crate::re!(r"(?i)(\d{1,2})\s*(?:er)?\s+([a-zéûè]+)(?:\s+(\d{4}))?").captures(&txt) {
                    if let Some(mo) = month(&m[2]) {
                        return Some((m[1].parse().unwrap_or(1), mo, m.get(3).and_then(|y| y.as_str().parse().ok())));
                    }
                }
                if let Some(m) = crate::re!(r"(\d{1,2})[/\-](\d{1,2})(?:[/\-](\d{2,4}))?").captures(&txt) {
                    let d: u32 = m[1].parse().unwrap_or(0);
                    let mo: i32 = m[2].parse::<i32>().unwrap_or(0) - 1;
                    if (1..=31).contains(&d) && (0..=11).contains(&mo) {
                        return Some((d, mo as u32, m.get(3).and_then(|y| y.as_str().parse().ok())));
                    }
                }
            }
        }
        None
    }

    pub fn is_birthday_today(&self) -> bool {
        use chrono::Datelike;
        let now = chrono::Local::now();
        match self.get_birthday() {
            Some((d, m, _)) => now.day() == d && now.month0() == m,
            None => false,
        }
    }

    /// Ville de résidence mémorisée.
    pub fn get_city(&self) -> Option<String> {
        for e in self.entries.borrow().iter().rev() {
            let txt = js::gs(e, "text");
            let m = crate::re!(r"(?i)(?:habite|vit|reside|réside)\s+à\s+([A-ZÀ-ÖØ-ß][a-zà-öø-ÿ\-]+)")
                .captures(&txt)
                .or_else(|| crate::re!(r"(?i)ville\s*:\s*([A-ZÀ-ÖØ-ß][a-zà-öø-ÿ\-]+)").captures(&txt))
                .or_else(|| crate::re!(r"(?i)(?:domicile|résidence|adresse)\s*:\s*([A-ZÀ-ÖØ-ß][a-zà-öø-ÿ\-]+)").captures(&txt));
            if let Some(m) = m {
                return Some(m[1].trim().to_string());
            }
        }
        None
    }
}

pub fn to_base36(mut n: u64) -> String {
    const D: &[u8] = b"0123456789abcdefghijklmnopqrstuvwxyz";
    if n == 0 {
        return "0".into();
    }
    let mut out = vec![];
    while n > 0 {
        out.push(D[(n % 36) as usize]);
        n /= 36;
    }
    out.reverse();
    String::from_utf8(out).unwrap()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn temp_memory(name: &str) -> Memory {
        let dir = std::env::temp_dir().join(format!("jarvis-mem-{name}-{}", js::now_ms()));
        let _ = std::fs::create_dir_all(&dir);
        Memory::new(dir.join("memory.json"))
    }

    fn day(offset: i64) -> chrono::NaiveDate {
        chrono::Local::now().date_naive() + chrono::Duration::days(offset)
    }

    fn iso(offset: i64) -> String {
        day(offset).format("%Y-%m-%d").to_string()
    }

    #[test]
    fn une_date_d_expiration_se_lit_et_refuse_le_passe() {
        assert_eq!(parse_expiry("").unwrap(), None);
        assert_eq!(parse_expiry(&iso(3)).unwrap(), Some(iso(3)));
        assert_eq!(parse_expiry(&day(3).format("%d/%m/%Y").to_string()).unwrap(), Some(iso(3)));
        assert!(parse_expiry(&iso(-1)).is_err());
        assert!(parse_expiry("demain").is_err());
    }

    #[test]
    fn un_souvenir_temporaire_reste_vrai_jusqu_au_soir_de_son_echeance() {
        let m = temp_memory("echeance");
        m.add_until("En congé", "habitudes", Some(&iso(0)));
        assert_eq!(m.list().len(), 1);
        assert!(m.prompt_block().contains("En congé (jusqu'au"));
    }

    #[test]
    fn un_souvenir_echu_est_oublie() {
        let m = temp_memory("echu");
        m.add("Aime le café", "préférences");
        let e = m.add_until("En déplacement à Lyon", "lieu", Some(&iso(2))).unwrap();
        // Échéance dépassée, écrite directement : parse_expiry refuse une date passée.
        m.set_expiry(&js::gs(&e, "id"), Some(&iso(-1)));
        assert_eq!(m.purge_expired().len(), 1);
        assert_eq!(m.list().len(), 1);
        assert!(!m.prompt_block().contains("Lyon"));
        assert!(m.search("Lyon", 5).iter().all(|e| !js::gs(e, "text").contains("Lyon")));
    }

    #[test]
    fn le_prompt_ne_garde_que_l_essentiel_et_renvoie_a_memoire_lire() {
        let m = temp_memory("essentiel");
        m.add("Prénom : Johann", "identité");
        m.add("Préfère des réponses courtes", "dialogue");
        m.add("Aime le café serré", "préférences");
        m.add("Fait du vélo le dimanche", "habitudes");
        m.add_until("En congé", "habitudes", Some(&iso(4)));
        let block = m.prompt_block();
        assert!(block.contains("Johann") && block.contains("réponses courtes") && block.contains("En congé (jusqu'au"), "{block}");
        assert!(!block.contains("café") && !block.contains("vélo"), "{block}");
        assert!(block.contains("Autres souvenirs à consulter (2 : préférences 1, habitudes 1)") && block.contains("memoire_lire"), "{block}");
    }

    #[test]
    fn redire_un_fait_change_son_echeance() {
        let m = temp_memory("redire");
        m.add_until("En congé", "habitudes", Some(&iso(2)));
        let e = m.add_until("en congé", "habitudes", Some(&iso(5))).unwrap();
        assert_eq!(js::gs(&e, "expires"), iso(5));
        assert_eq!(m.list().len(), 1);
        m.set_expiry(&js::gs(&e, "id"), None);
        assert!(js::gs(&m.list()[0], "expires").is_empty());
    }
}
