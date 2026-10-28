// Embeddings sémantiques locaux (orchestrator/embeddings.js + embed-worker.js) : modèle multilingue
// ONNX (EmbeddingGemma 300m quantifié q8), exécuté hors ligne dans un fil dédié, démarré à la demande
// et arrêté après 5 minutes d'inactivité (la mémoire du modèle est alors rendue).
//
// Même calcul que transformers.js en v3 (pipeline « feature-extraction », pooling moyen sur
// last_hidden_state, normalisation L2) : les vecteurs déjà calculés par la v3 restent valables.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc;

use tokio::sync::oneshot;

use crate::js::{self, Timer};

pub const DEFAULT_MODEL: &str = "onnx-community/embeddinggemma-300m-ONNX";
const IDLE_MS: u64 = 5 * 60 * 1000;
const REQUEST_TIMEOUT_MS: u64 = 180 * 1000;

struct Prefix {
    query: &'static str,
    passage: &'static str,
    similarity: Option<&'static str>,
}

fn prefix_for(model: &str) -> Prefix {
    let m = model.to_lowercase();
    if m.contains("e5") {
        Prefix { query: "query: ", passage: "passage: ", similarity: None }
    } else if m.contains("embeddinggemma") {
        Prefix { query: "task: search result | query: ", passage: "title: none | text: ", similarity: Some("task: sentence similarity | query: ") }
    } else {
        Prefix { query: "", passage: "", similarity: None }
    }
}

type Reply = oneshot::Sender<Result<Vec<Vec<f32>>, String>>;

enum Msg {
    Embed(Vec<String>, Reply),
    Stop,
}

pub struct Embedder {
    pub cache_dir: PathBuf,
    pub model: String,
    log: Rc<dyn Fn(String)>,
    worker: RefCell<Option<mpsc::Sender<Msg>>>,
    pending: std::cell::Cell<usize>,
    idle: Timer,
    me: RefCell<std::rc::Weak<Embedder>>,
}

impl Embedder {
    pub fn new(cache_dir: PathBuf, log: Rc<dyn Fn(String)>) -> Rc<Self> {
        let model = js::env("JARVIS_EMBED_MODEL").unwrap_or_else(|| DEFAULT_MODEL.into());
        let e = Rc::new(Self {
            cache_dir,
            model,
            log,
            worker: RefCell::new(None),
            pending: std::cell::Cell::new(0),
            idle: Timer::new(),
            me: RefCell::new(std::rc::Weak::new()),
        });
        *e.me.borrow_mut() = Rc::downgrade(&e);
        e
    }

    pub fn active(&self) -> bool {
        self.worker.borrow().is_some()
    }

    /// kind : « query » (la question) ou « passage » (un souvenir indexé).
    pub async fn embed(&self, texts: Vec<String>, kind: &str) -> Result<Vec<Vec<f32>>, String> {
        let p = prefix_for(&self.model);
        let pre = match kind {
            "query" => p.query,
            "passage" => p.passage,
            // « similarity » : préfixe propre au modèle s'il en a un ; type inconnu : aucun préfixe (comme la v3).
            "similarity" => p.similarity.unwrap_or(""),
            _ => "",
        };
        let list: Vec<String> = texts.into_iter().map(|t| format!("{pre}{t}")).collect();
        if list.is_empty() {
            return Ok(vec![]);
        }
        self.ensure_worker();
        self.idle.clear();
        let (tx, rx) = oneshot::channel();
        let sent = self.worker.borrow().as_ref().map(|w| w.send(Msg::Embed(list, tx)).is_ok()).unwrap_or(false);
        if !sent {
            *self.worker.borrow_mut() = None;
            return Err("thread des embeddings arrêté".into());
        }
        self.pending.set(self.pending.get() + 1);
        let res = tokio::time::timeout(std::time::Duration::from_millis(REQUEST_TIMEOUT_MS), rx).await;
        self.pending.set(self.pending.get().saturating_sub(1));
        self.arm_idle();
        match res {
            Err(_) => Err("délai dépassé pour le calcul des embeddings".into()),
            Ok(Err(_)) => {
                *self.worker.borrow_mut() = None;
                Err("thread des embeddings arrêté".into())
            }
            Ok(Ok(r)) => r,
        }
    }

    fn ensure_worker(&self) {
        if self.worker.borrow().is_some() {
            return;
        }
        let (tx, rx) = mpsc::channel::<Msg>();
        let dir = self.cache_dir.join(self.model.replace('/', std::path::MAIN_SEPARATOR_STR));
        (self.log)(format!("[mémoire sémantique] démarrage du modèle {}", self.model));
        std::thread::Builder::new()
            .name("embeddings".into())
            .spawn(move || worker_main(dir, rx))
            .ok();
        *self.worker.borrow_mut() = Some(tx);
    }

    fn arm_idle(&self) {
        self.idle.clear();
        if self.pending.get() > 0 || self.worker.borrow().is_none() {
            return;
        }
        let me = self.me.borrow().clone();
        self.idle.set(IDLE_MS, move || {
            if let Some(e) = me.upgrade() {
                e.stop();
            }
        });
    }

    pub fn stop(&self) {
        self.idle.clear();
        if let Some(w) = self.worker.borrow_mut().take() {
            let _ = w.send(Msg::Stop);
        }
    }
}

/// Produit scalaire (vecteurs déjà normalisés par le modèle).
pub fn cosine(a: &[f32], b: &[f32]) -> f32 {
    a.iter().zip(b.iter()).map(|(x, y)| x * y).sum()
}

#[cfg(feature = "semantic")]
fn worker_main(dir: PathBuf, rx: mpsc::Receiver<Msg>) {
    let mut model: Option<Result<Model, String>> = None;
    while let Ok(msg) = rx.recv() {
        match msg {
            Msg::Stop => break,
            Msg::Embed(texts, reply) => {
                if model.is_none() {
                    model = Some(Model::load(&dir));
                }
                let res = match model.as_mut().unwrap() {
                    Ok(m) => texts.iter().map(|t| m.embed(t)).collect::<Result<Vec<_>, _>>(),
                    Err(e) => {
                        let e = e.clone();
                        model = None; // nouvel essai à la prochaine demande
                        Err(e)
                    }
                };
                let _ = reply.send(res);
            }
        }
    }
}

#[cfg(not(feature = "semantic"))]
fn worker_main(_dir: PathBuf, rx: mpsc::Receiver<Msg>) {
    while let Ok(msg) = rx.recv() {
        match msg {
            Msg::Stop => break,
            Msg::Embed(_, reply) => {
                let _ = reply.send(Err("mémoire sémantique non compilée".into()));
            }
        }
    }
}

#[cfg(feature = "semantic")]
struct Model {
    session: ort::session::Session,
    tokenizer: tokenizers::Tokenizer,
    _env: ort::environment::Environment,
}

#[cfg(feature = "semantic")]
impl Model {
    fn load(dir: &std::path::Path) -> Result<Self, String> {
        let onnx = dir.join("onnx").join("model_quantized.onnx");
        if !onnx.exists() {
            return Err(format!("modèle d'embeddings absent ({})", onnx.display()));
        }
        let tokenizer = tokenizers::Tokenizer::from_file(dir.join("tokenizer.json")).map_err(|e| e.to_string())?;
        let env = ort::init().with_name("jarvis-embeddings").build().map_err(|e| e.to_string())?;
        let mut builder = ort::session::Session::builder(&env).map_err(|e| e.to_string())?;
        builder = builder.with_intra_threads(2).map_err(|e| e.to_string())?;
        let session = builder.commit_from_file(&onnx).map_err(|e| e.to_string())?;
        Ok(Self { session, tokenizer, _env: env })
    }

    fn embed(&mut self, text: &str) -> Result<Vec<f32>, String> {
        let enc = self.tokenizer.encode(text, true).map_err(|e| e.to_string())?;
        let ids: Vec<i64> = enc.get_ids().iter().map(|&i| i as i64).collect();
        let mask: Vec<i64> = enc.get_attention_mask().iter().map(|&m| m as i64).collect();
        let n = ids.len();
        let ids_t = ort::value::Tensor::from_array(([1usize, n], ids)).map_err(|e| e.to_string())?;
        let mask_t = ort::value::Tensor::from_array(([1usize, n], mask.clone())).map_err(|e| e.to_string())?;
        let outputs = self.session.run(ort::inputs!["input_ids" => ids_t, "attention_mask" => mask_t]).map_err(|e| e.to_string())?;
        let (shape, data) = outputs["last_hidden_state"].try_extract_tensor::<f32>().map_err(|e| e.to_string())?;
        let dim = *shape.last().unwrap_or(&0) as usize;
        let mut v = vec![0f32; dim];
        let mut count = 0f32;
        for t in 0..n {
            if mask[t] == 0 {
                continue;
            }
            count += 1.0;
            for d in 0..dim {
                v[d] += data[t * dim + d];
            }
        }
        if count > 0.0 {
            for x in v.iter_mut() {
                *x /= count;
            }
        }
        let norm = v.iter().map(|x| x * x).sum::<f32>().sqrt().max(1e-12);
        for x in v.iter_mut() {
            *x /= norm;
        }
        Ok(v)
    }
}
