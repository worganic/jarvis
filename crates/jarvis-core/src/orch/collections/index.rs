// RAG des listes à fiches : un passage par élément, retrouvé par le sens (embeddings) avec repli lexical.
// Les vecteurs sont mis en cache à côté des listes (data/collections/vectors.json), clés = empreinte du
// texte : un élément inchangé n'est jamais recalculé. Un numéro (Lego 75436) ou un code retrouvé à
// l'identique passe avant tout le reste.

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::rc::Rc;

use serde_json::{json, Value};
use sha1_smol::Sha1;

use super::super::embeddings::{cosine, Embedder};
use super::super::memory::{cosine_similarity, normalize, text_to_vector, SEMANTIC_MIN, SEMANTIC_RELATIVE};
use super::schema::fold;
use crate::js::{self, J};

pub struct Passage {
    pub list: String,
    pub element: String,
    pub text: String,
    /// Identifiants repliés (numéro de set, code-barres…).
    pub ids: Vec<String>,
}

#[derive(Clone, Debug)]
pub struct Hit {
    pub list: String,
    pub element: String,
    /// 0 à 1 (1 = identifiant retrouvé à l'identique).
    pub score: f32,
    pub exact: bool,
}

pub struct Index {
    file: PathBuf,
    embedder: RefCell<Option<Rc<Embedder>>>,
    vectors: RefCell<Option<(String, HashMap<String, Vec<f32>>)>>,
    indexing: Cell<bool>,
}

pub fn hash(text: &str) -> String {
    Sha1::from(text).hexdigest()[..16].to_string()
}

impl Index {
    pub fn new(file: PathBuf) -> Self {
        Self { file, embedder: RefCell::new(None), vectors: RefCell::new(None), indexing: Cell::new(false) }
    }

    pub fn attach_embedder(&self, e: Option<Rc<Embedder>>) {
        *self.embedder.borrow_mut() = e;
    }

    fn model(&self) -> String {
        self.embedder.borrow().as_ref().map(|e| e.model.clone()).unwrap_or_default()
    }

    /// Le modèle d'embeddings est-il installé sur le disque (sans le charger) ?
    pub fn model_installed(&self) -> bool {
        self.embedder.borrow().as_ref().map(|e| e.cache_dir.join(e.model.replace('/', std::path::MAIN_SEPARATOR_STR)).join("onnx").exists()).unwrap_or(false)
    }

    fn load(&self) {
        if self.vectors.borrow().is_some() {
            return;
        }
        let model = self.model();
        let mut items = HashMap::new();
        if let Some(raw) = js::read_json(&self.file) {
            if js::gs(&raw, "model") == model {
                if let Some(Value::Object(o)) = raw.get("items") {
                    for (h, v) in o {
                        let vec: Vec<f32> = v.as_array().map(|a| a.iter().map(|x| x.as_f64().unwrap_or(0.0) as f32).collect()).unwrap_or_default();
                        items.insert(h.clone(), vec);
                    }
                }
            }
        }
        *self.vectors.borrow_mut() = Some((model, items));
    }

    fn save(&self) {
        let guard = self.vectors.borrow();
        let Some((model, items)) = guard.as_ref() else { return };
        let mut out = serde_json::Map::new();
        for (h, v) in items {
            let rounded: Vec<f64> = v.iter().map(|x| ((*x as f64) * 1e6).round() / 1e6).collect();
            out.insert(h.clone(), json!(rounded));
        }
        if let Some(dir) = self.file.parent() {
            let _ = std::fs::create_dir_all(dir);
        }
        let _ = std::fs::write(&self.file, serde_json::to_string(&json!({ "model": model, "items": Value::Object(out) })).unwrap_or_default());
    }

    /// Nombre de vecteurs en cache (page d'aide).
    pub fn vector_count(&self) -> usize {
        self.load();
        self.vectors.borrow().as_ref().map(|(_, v)| v.len()).unwrap_or(0)
    }

    /// Calcule les vecteurs manquants et oublie ceux qui ne servent plus. Retourne le nombre calculé.
    pub async fn ensure(&self, passages: &[Passage]) -> Result<usize, String> {
        let embedder = self.embedder.borrow().clone();
        let Some(embedder) = embedder else { return Ok(0) };
        if !self.model_installed() {
            return Ok(0);
        }
        while self.indexing.get() {
            js::sleep_ms(50).await;
        }
        self.indexing.set(true);
        let res = async {
            self.load();
            let wanted: HashSet<String> = passages.iter().map(|p| hash(&p.text)).collect();
            let mut changed = false;
            {
                let mut guard = self.vectors.borrow_mut();
                let items = &mut guard.as_mut().unwrap().1;
                let before = items.len();
                items.retain(|h, _| wanted.contains(h));
                changed |= items.len() != before;
            }
            let missing: Vec<(String, String)> = {
                let guard = self.vectors.borrow();
                let items = &guard.as_ref().unwrap().1;
                let mut seen = HashSet::new();
                passages.iter().map(|p| (hash(&p.text), p.text.clone())).filter(|(h, _)| !items.contains_key(h) && seen.insert(h.clone())).collect()
            };
            for chunk in missing.chunks(32) {
                let vectors = embedder.embed(chunk.iter().map(|(_, t)| t.clone()).collect(), "passage").await?;
                let mut guard = self.vectors.borrow_mut();
                let items = &mut guard.as_mut().unwrap().1;
                for ((h, _), v) in chunk.iter().zip(vectors) {
                    items.insert(h.clone(), v);
                }
                changed = true;
            }
            if changed {
                self.save();
            }
            Ok(missing.len())
        }
        .await;
        self.indexing.set(false);
        res
    }

    /// Efface le cache (réindexation complète au prochain calcul).
    pub fn reset(&self) {
        *self.vectors.borrow_mut() = Some((self.model(), HashMap::new()));
        let _ = std::fs::remove_file(&self.file);
    }

    /// Mots de la question qui sont exactement un identifiant (numéro de set, code…).
    fn exact_ids(query: &str, passages: &[Passage]) -> HashSet<usize> {
        let q = fold(query);
        let words: Vec<&str> = q.split(' ').filter(|w| !w.is_empty()).collect();
        let padded = format!(" {q} ");
        passages
            .iter()
            .enumerate()
            .filter(|(_, p)| p.ids.iter().any(|id| !id.is_empty() && (words.contains(&id.as_str()) || (id.contains(' ') && padded.contains(&format!(" {id} "))))))
            .map(|(i, _)| i)
            .collect()
    }

    fn lexical(query: &str, passages: &[Passage]) -> Vec<(usize, f64)> {
        let words = normalize(query);
        if words.is_empty() {
            return vec![];
        }
        let qv = text_to_vector(&words.join(" "));
        passages
            .iter()
            .map(|p| {
                let cos = cosine_similarity(&qv, &text_to_vector(&p.text));
                let target = normalize(&p.text);
                let hit = words.iter().filter(|w| target.iter().any(|t| t.starts_with(w.as_str()))).count();
                (cos * 0.6 + (hit as f64 / words.len() as f64) * 0.4, hit)
            })
            .enumerate()
            .map(|(i, (s, hit))| (i, if hit == 0 && s < 0.12 { 0.0 } else { s }))
            .collect()
    }

    /// Les passages les plus proches de la question, du plus pertinent au moins pertinent.
    pub async fn search(&self, query: &str, passages: &[Passage], limit: usize) -> Vec<Hit> {
        let query = query.trim();
        if query.is_empty() || passages.is_empty() {
            return vec![];
        }
        let exact = Self::exact_ids(query, passages);
        let lexical: HashMap<usize, f64> = Self::lexical(query, passages).into_iter().collect();
        let mut semantic: HashMap<usize, f32> = HashMap::new();
        let embedder = self.embedder.borrow().clone();
        if let (Some(embedder), true) = (embedder, self.model_installed()) {
            let res: Result<(), String> = async {
                self.ensure(passages).await?;
                let q = embedder.embed(vec![query.to_string()], "query").await?.into_iter().next().unwrap_or_default();
                self.load();
                let guard = self.vectors.borrow();
                let items = &guard.as_ref().unwrap().1;
                for (i, p) in passages.iter().enumerate() {
                    if let Some(v) = items.get(&hash(&p.text)) {
                        semantic.insert(i, cosine(&q, v));
                    }
                }
                Ok(())
            }
            .await;
            if res.is_err() {
                semantic.clear();
            }
        }
        let best = semantic.values().cloned().fold(0f32, f32::max);
        let floor = SEMANTIC_MIN.max(best * SEMANTIC_RELATIVE);
        let mut rows: Vec<(usize, f32, bool)> = vec![];
        for (i, _) in passages.iter().enumerate() {
            let lex = lexical.get(&i).cloned().unwrap_or(0.0);
            let is_exact = exact.contains(&i);
            let score = if is_exact {
                1.0
            } else if !semantic.is_empty() {
                let s = semantic.get(&i).cloned().unwrap_or(0.0);
                if s < floor && lex < 0.35 {
                    continue;
                }
                (s as f64 * 0.75 + lex * 0.25) as f32
            } else {
                if lex < 0.12 {
                    continue;
                }
                lex as f32
            };
            rows.push((i, score, is_exact));
        }
        rows.sort_by(|a, b| b.2.cmp(&a.2).then(b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal)));
        rows.into_iter()
            .take(limit)
            .map(|(i, score, ex)| Hit { list: passages[i].list.clone(), element: passages[i].element.clone(), score: score.min(1.0), exact: ex })
            .collect()
    }

    pub fn stats(&self, passages: usize) -> J {
        json!({ "passages": passages, "vecteurs": self.vector_count(), "modele": self.model(), "modeleInstalle": self.model_installed(),
                "modeleCharge": self.embedder.borrow().as_ref().map(|e| e.active()).unwrap_or(false) })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn passages() -> Vec<Passage> {
        let p = |l: &str, e: &str, t: &str, ids: &[&str]| Passage { list: l.into(), element: e.into(), text: t.into(), ids: ids.iter().map(|s| s.to_string()).collect() };
        vec![
            p("lego", "a", "Set « Le Faucon Millenium » (liste Lego) — 75192 · Star Wars. Possédé. Prix payé : 799,99 €.", &["75192"]),
            p("lego", "b", "Set « Le Faucon Millenium » (liste Lego) — 75436 · Star Wars. Souhaité, pas encore possédé.", &["75436"]),
            p("vinyle", "c", "Disque « Under a Blood Red Sky » (liste Vinyles) — U2 · 1983. Possédé. Acheté où : Fnac Strasbourg.", &[]),
            p("vinyle", "d", "Disque « The Joshua Tree » (liste Vinyles) — U2 · 1987. Souhaité.", &[]),
        ]
    }

    fn run<T>(f: impl std::future::Future<Output = T>) -> T {
        tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap().block_on(f)
    }

    #[test]
    fn un_numero_est_retrouve_a_l_identique() {
        let idx = Index::new(std::env::temp_dir().join("jarvis-test-collections-vectors-a.json"));
        let hits = run(idx.search("est-ce que j'ai le lego 75436", &passages(), 5));
        assert_eq!(hits[0].element, "b");
        assert!(hits[0].exact);
        assert_eq!(hits.iter().filter(|h| h.exact).count(), 1, "le 75192 ne doit pas passer pour exact");
    }

    #[test]
    fn la_recherche_lexicale_trouve_un_titre() {
        let idx = Index::new(std::env::temp_dir().join("jarvis-test-collections-vectors-b.json"));
        let hits = run(idx.search("blood red sky de U2", &passages(), 3));
        assert_eq!(hits[0].element, "c");
        let none = run(idx.search("recette de la tarte aux pommes", &passages(), 3));
        assert!(none.is_empty(), "rien de pertinent : aucun résultat ({none:?})");
    }
}
