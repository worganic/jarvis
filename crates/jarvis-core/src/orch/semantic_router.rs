// Routeur sémantique local (orchestrator/semantic-router.js) : quand aucune règle ne reconnaît la phrase,
// elle est comparée à des exemples de chaque intention locale avec EmbeddingGemma. Au-dessus du seuil,
// elle est remplacée par la commande canonique, que le routeur à règles sait exécuter. Coût : 0 token.
// Le modèle n'est jamais attendu longtemps : froid, la demande part à l'IA et le chargement se fait en fond.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;

use serde_json::{json, Map, Value};

use super::embeddings::{cosine, Embedder};
use crate::js;

pub const EXAMPLES: &[(&str, &[&str])] = &[
    ("quelle heure est il", &["tu as l'heure", "il est quelle heure maintenant", "donne-moi l'heure exacte", "c'est quelle heure là", "je voudrais savoir l'heure"]),
    ("quelle est la date", &["on est quel jour", "c'est quoi la date aujourd'hui", "nous sommes le combien", "quel jour sommes-nous"]),
    ("mets de la musique", &["j'ai envie d'écouter de la musique", "lance un peu de son", "fais-moi écouter quelque chose", "balance la musique", "joue-moi un morceau", "envoie le son"]),
    ("mets la musique en pause", &["arrête la chanson", "coupe la musique un instant", "stoppe le morceau", "mets le morceau en pause"]),
    ("chanson suivante", &["passe à la suivante", "change de morceau", "saute ce titre", "je n'aime pas cette chanson, mets la suivante", "zappe ce morceau"]),
    ("chanson precedente", &["reviens au morceau d'avant", "remets la chanson d'avant", "rejoue le titre précédent"]),
    ("c est quoi cette chanson", &["qu'est-ce qu'on écoute là", "c'est quoi ce morceau", "qui chante ça", "quel est le titre de cette musique"]),
    ("monte le son", &["plus fort s'il te plaît", "augmente un peu le volume", "je n'entends rien, monte le son", "le son est trop bas"]),
    ("baisse le son", &["moins fort", "c'est trop fort", "diminue le volume", "baisse un peu le volume"]),
    ("lis mes derniers mails", &["est-ce que j'ai reçu quelque chose par mail", "des nouvelles dans ma boîte mail", "quoi de neuf dans mes mails", "consulte ma messagerie"]),
    ("qu est ce que j ai aujourd hui", &["qu'est-ce que j'ai de prévu aujourd'hui", "c'est quoi mon programme du jour", "ai-je des rendez-vous aujourd'hui", "quel est mon emploi du temps"]),
    ("meteo", &["il fait quel temps dehors", "faut-il prendre un parapluie", "est-ce qu'il fait beau dehors", "quelle température fait-il dehors"]),
    ("prends une capture d ecran", &["prends une photo de l'écran", "fais un screenshot", "enregistre ce qu'il y a à l'écran"]),
    ("diagnostic", &["est-ce que tous tes systèmes fonctionnent", "fais un bilan de tes systèmes", "vérifie que tout marche chez toi"]),
    ("qui suis je", &["qu'est-ce que tu connais de moi", "que sais-tu sur moi", "dis-moi ce que tu as retenu sur moi"]),
    ("quelles fenetres sont ouvertes", &["qu'est-ce qui tourne en ce moment sur l'ordinateur", "quels programmes sont lancés"]),
    ("repete", &["tu peux redire ça", "je n'ai pas entendu, recommence", "redis-moi ta dernière phrase"]),
];

pub const THRESHOLD: f32 = 0.83;
const MARGIN: f32 = 0.04;
const WARM_BUDGET_MS: u64 = 900;

fn fold(t: &str) -> String {
    let t = js::fold_lower(t);
    let t = crate::re!(r"['’\-?!,;:.…]").replace_all(&t, " ");
    crate::re!(r"\s+").replace_all(&t, " ").trim().to_string()
}

/// Phrases proches par le sens qui demandent autre chose (l'heure d'un événement, d'un autre lieu).
fn veto(canonical: &str, t: &str) -> bool {
    canonical == "quelle heure est il"
        && (crate::re!(r"\b(?:a|vers|jusqu a|depuis|pour|de|des) quelle heure\b").is_match(t)
            || crate::re!(r"\bheure (?:est il|il est) (?:a|au|aux|en)\b").is_match(t)
            || crate::re!(r"\b(?:lieu|commence|debute|demarre|ouvre|ferme|finit|termine|part|arrive|passe|diffuse|course|match|grand prix|concert|film|train|vol|rendez vous)\b").is_match(t))
}

pub struct SemanticRouter {
    embedder: Rc<Embedder>,
    cache_file: Option<PathBuf>,
    log: Rc<dyn Fn(String)>,
    vectors: RefCell<Option<Vec<(String, Vec<f32>)>>>,
    loading: Cell<bool>,
    signature: String,
    available_cache: Cell<Option<bool>>,
}

impl SemanticRouter {
    pub fn new(embedder: Rc<Embedder>, cache_file: Option<PathBuf>, log: Rc<dyn Fn(String)>) -> Rc<Self> {
        let mut m = Map::new();
        for (k, v) in EXAMPLES {
            m.insert(k.to_string(), json!(v));
        }
        let sig_src = format!("{}{}", Value::Object(m), embedder.model);
        let signature = sha1_smol::Sha1::from(sig_src).hexdigest()[..16].to_string();
        Rc::new(Self { embedder, cache_file, log, vectors: RefCell::new(None), loading: Cell::new(false), signature, available_cache: Cell::new(None) })
    }

    fn load_cache(&self) -> bool {
        let Some(f) = &self.cache_file else { return false };
        let Some(saved) = js::read_json(f) else { return false };
        if js::gs(&saved, "signature") != self.signature {
            return false;
        }
        let items: Vec<(String, Vec<f32>)> = js::arr(&saved["items"])
            .iter()
            .map(|i| (js::gs(i, "canonical"), i["vector"].as_array().map(|a| a.iter().map(|x| x.as_f64().unwrap_or(0.0) as f32).collect()).unwrap_or_default()))
            .collect();
        *self.vectors.borrow_mut() = Some(items);
        true
    }

    fn save_cache(&self) {
        let (Some(f), Some(v)) = (&self.cache_file, self.vectors.borrow().clone()) else { return };
        let items: Vec<Value> = v.iter().map(|(c, vec)| json!({ "canonical": c, "vector": vec })).collect();
        let _ = std::fs::create_dir_all(f.parent().unwrap_or(f));
        let _ = std::fs::write(f, json!({ "signature": self.signature, "items": items }).to_string());
    }

    /// Vecteurs des exemples : cache disque, sinon calcul (une fois).
    pub async fn prepare(&self) {
        if self.vectors.borrow().is_some() || self.load_cache() || self.loading.get() {
            return;
        }
        self.loading.set(true);
        let items: Vec<(String, String)> = EXAMPLES.iter().flat_map(|(c, ps)| ps.iter().map(move |p| (c.to_string(), p.to_string()))).collect();
        match self.embedder.embed(items.iter().map(|i| i.1.clone()).collect(), "similarity").await {
            Ok(vecs) => {
                *self.vectors.borrow_mut() = Some(items.into_iter().zip(vecs).map(|((c, _), v)| (c, v)).collect());
                self.save_cache();
                (self.log)(format!("[routeur sémantique] {} exemples vectorisés", self.vectors.borrow().as_ref().map(|v| v.len()).unwrap_or(0)));
            }
            Err(e) => (self.log)(format!("[routeur sémantique] indisponible : {e}")),
        }
        self.loading.set(false);
    }

    /// Scores bruts (tests et calibrage) : [(canonique, score)] triés.
    pub async fn scores(&self, text: &str) -> Vec<(String, f32)> {
        self.prepare().await;
        let Some(vectors) = self.vectors.borrow().clone() else { return vec![] };
        let Ok(q) = self.embedder.embed(vec![text.to_string()], "similarity").await else { return vec![] };
        let Some(q) = q.into_iter().next() else { return vec![] };
        let mut best: Vec<(String, f32)> = vec![];
        let mut idx: HashMap<String, usize> = HashMap::new();
        for (c, v) in &vectors {
            let s = cosine(&q, v);
            match idx.get(c) {
                Some(&i) => {
                    if s > best[i].1 {
                        best[i].1 = s;
                    }
                }
                None => {
                    idx.insert(c.clone(), best.len());
                    best.push((c.clone(), s));
                }
            }
        }
        best.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        best
    }

    /// Le modèle est-il déjà sur le disque ? Le routeur ne déclenche jamais de téléchargement.
    fn available(&self) -> bool {
        if let Some(a) = self.available_cache.get() {
            return a;
        }
        let mut p = self.embedder.cache_dir.clone();
        for part in self.embedder.model.split('/') {
            p.push(part);
        }
        let a = p.exists();
        self.available_cache.set(Some(a));
        a
    }

    /// Commande canonique et score, ou None. Ne bloque jamais plus de 900 ms.
    pub async fn match_(self: &Rc<Self>, text: &str) -> Option<(String, f32)> {
        if text.is_empty() || text.encode_utf16().count() > 160 {
            return None;
        }
        if !self.available() && !self.embedder.active() {
            self.available_cache.set(None);
            return None;
        }
        if !self.embedder.active() || self.vectors.borrow().is_none() {
            let me = self.clone();
            js::spawn(async move {
                me.prepare().await;
                let _ = me.embedder.embed(vec!["préchauffage".into()], "similarity").await;
            });
            if !self.embedder.active() {
                return None;
            }
        }
        let ranked = tokio::time::timeout(std::time::Duration::from_millis(WARM_BUDGET_MS), self.scores(text)).await.ok()?;
        let first = ranked.first()?.clone();
        if first.1 < THRESHOLD {
            return None;
        }
        if let Some(second) = ranked.get(1) {
            if first.1 - second.1 < MARGIN {
                return None;
            }
        }
        if veto(&first.0, &fold(text)) {
            return None;
        }
        Some(first)
    }
}
