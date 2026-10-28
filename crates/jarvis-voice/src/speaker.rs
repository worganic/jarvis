// Vérification et identification du locuteur (voice/speaker_verifier.py de la v3).
//
// - Empreintes 512-dim du modèle WeSpeaker CAM++ (ONNX local, onnxruntime) : bancs de filtres Kaldi
//   (80, 25 ms / 10 ms, fenêtre de Hamming), moyenne soustraite sur la phrase. Calcul identique à la
//   v3 : les empreintes de data/voiceprint.json restent valables.
// - Identification 1:N par produit scalaire (similarité cosinus) sur l'ensemble des profils.
// - Trois verdicts : voix reconnue (au-dessus du seuil), doute (dans la marge sous le seuil, ou deux
//   profils presque à égalité) et voix inconnue. En mode « reject », seules les voix reconnues
//   comptent ; le HUD propose les autres (« c'est Johann », « nouvelle personne ») et chaque
//   confirmation ajoute la phrase au profil (apprentissage).
// - Chaque profil garde son enrôlement (moyenne par configuration casque / haut-parleurs) et les
//   phrases apprises depuis : l'empreinte est leur moyenne.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::Instant;

use serde_json::{json, Map, Value};
use unicode_normalization::UnicodeNormalization;

use crate::util::{log, J};

pub const SR: usize = 16000;
pub const DEFAULT_THRESHOLD: f64 = 0.5;
pub const FILTER_MODES: &[&str] = &["signal", "reject"];
pub const DEFAULT_FILTER_MODE: &str = "reject";
pub const PROFILE_SCHEMA: i64 = 2;
pub const DEFAULT_DOUBT_MARGIN: f64 = 0.15;
pub const AMBIGUOUS_GAP: f64 = 0.04;
pub const MAX_LEARNED: usize = 40;
pub const MAX_AUTO: usize = 15;
pub const AUTO_LEARN_MARGIN: f64 = 0.12;
pub const AUTO_LEARN_MIN_SECONDS: f64 = 1.5;
pub const AUTO_LEARN_EVERY: f64 = 90.0;
pub const RECENT_TTL: f64 = 900.0;
pub const RECENT_MAX: usize = 40;
pub const SPEAKER_TYPES: &[&str] = &["moi", "femme", "mari", "fille", "fils", "famille", "ami", "invite", "visio", "autre"];
pub const MAX_MEETING_SAMPLES: usize = 8;
pub const MIN_AUDIO_DURATION: f64 = 0.4;
pub const DEFAULT_ENROLL_COUNT: usize = 8;
/// Le suffixe « -rs » marque l'extracteur du port Rust : les empreintes de la v3 (même modèle, calcul
/// des bancs de filtres légèrement différent) ne lui sont pas comparables et sont refaites au démarrage.
pub const EMBEDDER_VERSION: &str = "wespeaker-fbank-cmn-1-rs";

pub fn duration_bucket(seconds: f64) -> &'static str {
    if seconds < 1.2 {
        "court"
    } else if seconds < 2.5 {
        "moyen"
    } else {
        "long"
    }
}

/// Configuration audio déduite du nom de la sortie : casque ou haut-parleurs.
pub fn audio_config(device_name: &str) -> &'static str {
    let name = device_name.to_lowercase();
    if crate::re!(r"casque|headset|headphone|écouteur|ecouteur|hyperx|earbud|buds|airpods").is_match(&name) {
        "casque"
    } else {
        "haut-parleurs"
    }
}

/// Identifiant normalisé à partir d'un prénom ou nom.
pub fn slugify(text: &str) -> String {
    let ascii: String = text.nfkd().filter(|c| c.is_ascii()).collect();
    let clean = crate::re!(r"[^\w\s-]").replace_all(&ascii, "").trim().to_lowercase();
    let s = crate::re!(r"[-\s]+").replace_all(&clean, "-").to_string();
    if s.is_empty() {
        "utilisateur".into()
    } else {
        s
    }
}

fn now_utc() -> String {
    chrono::Utc::now().format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

fn norm(v: &[f32]) -> Vec<f32> {
    let n: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    if n > 1e-6 {
        v.iter().map(|x| x / n).collect()
    } else {
        v.to_vec()
    }
}

fn dot(a: &[f32], b: &[f32]) -> f64 {
    a.iter().zip(b).map(|(x, y)| (*x as f64) * (*y as f64)).sum()
}

/// Vecteur 512 normalisé, ou None.
fn vec_of(raw: &J) -> Option<Vec<f32>> {
    let a = raw.as_array()?;
    if a.len() != 512 {
        return None;
    }
    let v: Vec<f32> = a.iter().map(|x| x.as_f64().unwrap_or(0.0) as f32).collect();
    let n: f32 = v.iter().map(|x| x * x).sum::<f32>().sqrt();
    (n > 1e-6).then(|| v.iter().map(|x| x / n).collect())
}

fn dump(v: &[f32]) -> J {
    J::Array(v.iter().map(|x| json!(((*x as f64) * 1e5).round() / 1e5)).collect())
}

fn r3(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

// --- Empreinte WeSpeaker ---------------------------------------------------------------------------

pub struct WespeakerEmbedder {
    session: ort::session::Session,
    input: String,
    pub sample_rate: usize,
    scale: f64,
    pub dim: usize,
    window: Vec<f64>,
    banks: Vec<Vec<f64>>,
    fft: Arc<dyn rustfft::Fft<f64>>,
}

const FRAME: usize = 400;
const SHIFT: usize = 160;
const NFFT: usize = 512;
const MELS: usize = 80;

impl WespeakerEmbedder {
    pub fn new(model_path: &Path, threads: usize) -> Result<Self, String> {
        let session = ort::session::Session::builder(crate::util::ort_env())
            .map_err(|e| e.to_string())?
            .with_intra_threads(threads)
            .map_err(|e| e.to_string())?
            .with_inter_threads(1)
            .map_err(|e| e.to_string())?
            .commit_from_file(model_path)
            .map_err(|e| e.to_string())?;
        let meta = |k: &str| session.metadata().ok().and_then(|m| m.custom(k));
        let sample_rate = meta("sample_rate").and_then(|s| s.parse().ok()).unwrap_or(SR);
        // WeSpeaker s'entraîne sur des échantillons à l'échelle 16 bits (normalize_samples = 0).
        let scale = if meta("normalize_samples").as_deref() == Some("1") { 1.0 } else { 32768.0 };
        let dim = meta("output_dim").and_then(|s| s.parse().ok()).unwrap_or(512);
        let input = session.inputs().first().map(|i| i.name().to_string()).ok_or("modèle sans entrée")?;
        let window = (0..FRAME).map(|n| 0.54 - 0.46 * (2.0 * std::f64::consts::PI * n as f64 / (FRAME - 1) as f64).cos()).collect();
        let banks = Self::mel_banks(sample_rate as f64, 20.0);
        let fft = rustfft::FftPlanner::new().plan_fft_forward(NFFT);
        Ok(Self { session, input, sample_rate, scale, dim, window, banks, fft })
    }

    fn mel_banks(sr: f64, low: f64) -> Vec<Vec<f64>> {
        let high = sr / 2.0;
        let mel = |f: f64| 1127.0 * (1.0 + f / 700.0).ln();
        let (lo, hi) = (mel(low), mel(high));
        let delta = (hi - lo) / (MELS + 1) as f64;
        let bins = NFFT / 2;
        let mels: Vec<f64> = (0..bins).map(|k| mel(k as f64 * sr / NFFT as f64)).collect();
        (0..MELS)
            .map(|m| {
                let (left, center, right) = (lo + m as f64 * delta, lo + (m + 1) as f64 * delta, lo + (m + 2) as f64 * delta);
                let mut row = vec![0.0; bins + 1];
                for (k, mv) in mels.iter().enumerate() {
                    row[k] = 0f64.max(((mv - left) / (center - left)).min((right - mv) / (right - center)));
                }
                row
            })
            .collect()
    }

    /// Bancs de filtres log, moyenne soustraite sur la phrase : [trames][80].
    pub fn features(&self, wave: &[f32]) -> Option<Vec<[f32; MELS]>> {
        if wave.len() < FRAME {
            return None;
        }
        let x: Vec<f64> = wave.iter().map(|v| *v as f64 * self.scale).collect();
        let count = 1 + (x.len() - FRAME) / SHIFT;
        let eps = f32::EPSILON as f64;
        let mut feats: Vec<[f64; MELS]> = Vec::with_capacity(count);
        let mut buf = vec![rustfft::num_complex::Complex::new(0.0f64, 0.0); NFFT];
        for i in 0..count {
            let f = &x[i * SHIFT..i * SHIFT + FRAME];
            let mean = f.iter().sum::<f64>() / FRAME as f64;
            let fr: Vec<f64> = f.iter().map(|v| v - mean).collect();
            for (n, b) in buf.iter_mut().enumerate() {
                let v = if n < FRAME {
                    let pre = if n == 0 { fr[0] - 0.97 * fr[0] } else { fr[n] - 0.97 * fr[n - 1] };
                    pre * self.window[n]
                } else {
                    0.0
                };
                *b = rustfft::num_complex::Complex::new(v, 0.0);
            }
            self.fft.process(&mut buf);
            let power: Vec<f64> = buf[..=NFFT / 2].iter().map(|c| c.norm_sqr()).collect();
            let mut row = [0f64; MELS];
            for (m, bank) in self.banks.iter().enumerate() {
                let e: f64 = bank.iter().zip(&power).map(|(w, p)| w * p).sum();
                row[m] = e.max(eps).ln();
            }
            feats.push(row);
        }
        let mut means = [0f64; MELS];
        for r in &feats {
            for m in 0..MELS {
                means[m] += r[m];
            }
        }
        for m in means.iter_mut() {
            *m /= feats.len() as f64;
        }
        Some(
            feats
                .iter()
                .map(|r| {
                    let mut o = [0f32; MELS];
                    for m in 0..MELS {
                        o[m] = (r[m] - means[m]) as f32;
                    }
                    o
                })
                .collect(),
        )
    }

    pub fn embed(&mut self, wave: &[f32]) -> Option<Vec<f32>> {
        let feats = self.features(wave)?;
        if feats.len() < 10 {
            return None;
        }
        let t = feats.len();
        let flat: Vec<f32> = feats.iter().flat_map(|r| r.iter().copied()).collect();
        let tensor = ort::value::Tensor::from_array(([1usize, t, MELS], flat)).ok()?;
        let outputs = self.session.run(ort::inputs![self.input.as_str() => tensor]).ok()?;
        let (_, data) = outputs[0].try_extract_tensor::<f32>().ok()?;
        Some(data.to_vec())
    }
}

// --- Profils ---------------------------------------------------------------------------------------

#[derive(Clone)]
pub struct Learned {
    pub e: Vec<f32>,
    pub c: String,
    pub at: J,
    pub src: String,
    pub sid: Option<String>,
}

#[derive(Clone)]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub typ: Option<String>,
    pub base: Vec<(String, Vec<f32>, usize)>,
    pub learned: Vec<Learned>,
    pub samples_count: i64,
    pub enrolled_at: J,
    pub is_primary: bool,
    pub variants: Vec<(String, Vec<f32>)>,
    pub embedding: Vec<f32>,
}

impl Profile {
    fn who(&self, similarity: Option<f64>) -> J {
        let mut info = json!({ "id": self.id, "name": self.name, "type": self.typ.clone().unwrap_or_else(|| "autre".into()) });
        if let Some(s) = similarity {
            info["similarity"] = json!(r3(s));
        }
        info
    }

    /// Empreinte : enrôlement (par configuration) + phrases apprises, moyennés. Chaque configuration
    /// (casque, haut-parleurs) pèse autant dans l'empreinte générale.
    fn rebuild(&mut self) -> bool {
        let mut sums: Vec<(String, Vec<f32>)> = vec![];
        let mut add = |cfg: &str, v: &[f32], w: f32| match sums.iter_mut().find(|(c, _)| c == cfg) {
            Some((_, s)) => s.iter_mut().zip(v).for_each(|(a, b)| *a += b * w),
            None => sums.push((cfg.to_string(), v.iter().map(|x| x * w).collect())),
        };
        for (cfg, v, n) in &self.base {
            add(cfg, v, (*n).max(1) as f32);
        }
        for item in &self.learned {
            add(if item.c.is_empty() { "*" } else { &item.c }, &item.e, 1.0);
        }
        if sums.is_empty() {
            return false;
        }
        let normed: Vec<(String, Vec<f32>)> = sums.into_iter().map(|(c, v)| (c, norm(&v))).collect();
        self.variants = normed.iter().filter(|(c, _)| c != "*").cloned().collect();
        let mut mean = vec![0f32; normed[0].1.len()];
        for (_, v) in &normed {
            mean.iter_mut().zip(v).for_each(|(a, b)| *a += b);
        }
        let n = normed.len() as f32;
        mean.iter_mut().for_each(|a| *a /= n);
        self.embedding = norm(&mean);
        true
    }
}

struct Recent {
    e: Vec<f32>,
    c: String,
    at: Instant,
}

pub struct SpeakerVerifier {
    pub model_path: PathBuf,
    pub profile_path: PathBuf,
    corpus_dir: PathBuf,
    extractor: Option<WespeakerEmbedder>,
    pub available: bool,
    pub enrolled: bool,
    pub enabled: bool,
    pub threshold: f64,
    pub mode: String,
    pub thresholds: Vec<(String, f64)>,
    pub config: String,
    pub enroll_config: Option<String>,
    pub speakers: Vec<Profile>,
    enrolled_at: J,
    samples_count: i64,
    pub doubt_margin: f64,
    pub learning: bool,
    pub auto_learn: bool,
    last_auto: Option<Instant>,
    recent: VecDeque<(String, Recent)>,
    recent_seq: u64,
    pub is_enrolling: bool,
    pub enroll_target_name: String,
    pub enroll_target_id: String,
    enroll_samples: Vec<Vec<f32>>,
    pub enroll_target_count: usize,
    enroll_type: Option<String>,
}

impl SpeakerVerifier {
    pub fn new(model_path: PathBuf, profile_path: PathBuf, corpus_dir: PathBuf) -> Self {
        let mut v = Self {
            model_path,
            profile_path,
            corpus_dir,
            extractor: None,
            available: false,
            enrolled: false,
            enabled: true,
            threshold: DEFAULT_THRESHOLD,
            mode: DEFAULT_FILTER_MODE.into(),
            thresholds: vec![],
            config: "haut-parleurs".into(),
            enroll_config: None,
            speakers: vec![],
            enrolled_at: Value::Null,
            samples_count: 0,
            doubt_margin: DEFAULT_DOUBT_MARGIN,
            learning: true,
            auto_learn: true,
            last_auto: None,
            recent: VecDeque::new(),
            recent_seq: 0,
            is_enrolling: false,
            enroll_target_name: "Johann".into(),
            enroll_target_id: "johann".into(),
            enroll_samples: vec![],
            enroll_target_count: DEFAULT_ENROLL_COUNT,
            enroll_type: None,
        };
        v.init_extractor();
        v.load_profile();
        v
    }

    fn init_extractor(&mut self) {
        if !self.model_path.exists() {
            log(&format!("modèle absent : {}", self.model_path.display()));
            return;
        }
        match WespeakerEmbedder::new(&self.model_path, 2) {
            Ok(e) => {
                log(&format!("extracteur d'empreinte prêt (modèle: CAM++, bancs de filtres normalisés, dim: {})", e.dim));
                self.extractor = Some(e);
                self.available = true;
            }
            Err(e) => {
                log(&format!("impossible d'initialiser l'extracteur d'empreinte : {e}"));
                self.available = false;
            }
        }
    }

    fn speaker(&self, id: &str) -> Option<&Profile> {
        self.speakers.iter().find(|s| s.id == id)
    }

    fn speaker_mut(&mut self, id: &str) -> Option<&mut Profile> {
        self.speakers.iter_mut().find(|s| s.id == id)
    }

    fn primary(&self) -> Option<&Profile> {
        self.speakers.iter().find(|s| s.is_primary)
    }

    fn set_reference(&mut self) {
        if let Some(p) = self.primary().cloned() {
            self.enrolled_at = p.enrolled_at.clone();
            self.samples_count = p.samples_count;
        }
    }

    pub fn load_profile(&mut self) {
        self.speakers.clear();
        let Some(data) = std::fs::read_to_string(&self.profile_path).ok().and_then(|s| serde_json::from_str::<J>(&s).ok()) else {
            self.enrolled = false;
            return;
        };
        let mut migrated = false;
        self.enabled = data.get("enabled").map(|v| v.as_bool().unwrap_or(true)).unwrap_or(true);
        self.threshold = data["threshold"].as_f64().unwrap_or(DEFAULT_THRESHOLD);
        let mode = data["mode"].as_str().unwrap_or("");
        self.mode = if FILTER_MODES.contains(&mode) { mode.into() } else { DEFAULT_FILTER_MODE.into() };
        if data["schema"].as_i64().unwrap_or(1) < PROFILE_SCHEMA {
            // Profil antérieur au 28/09/2026 : passage unique au filtrage strict.
            self.mode = "reject".into();
            migrated = true;
        }
        self.doubt_margin = data["doubtMargin"].as_f64().unwrap_or(DEFAULT_DOUBT_MARGIN).clamp(0.0, 0.3);
        self.auto_learn = data.get("autoLearn").map(|v| v.as_bool().unwrap_or(true)).unwrap_or(true);
        self.learning = data.get("learning").map(|v| v.as_bool().unwrap_or(true)).unwrap_or(true);
        self.thresholds = data["thresholds"]
            .as_object()
            .map(|m| {
                m.iter()
                    .filter(|(k, v)| ["court", "moyen", "long"].contains(&k.as_str()) && v.is_number())
                    .map(|(k, v)| (k.clone(), v.as_f64().unwrap_or(0.5).clamp(0.40, 0.90)))
                    .collect()
            })
            .unwrap_or_default();

        if let Some(raw) = data["speakers"].as_array().filter(|a| !a.is_empty()) {
            for spk in raw {
                let general = vec_of(&spk["embedding"]);
                let id = spk["id"].as_str().map(String::from).unwrap_or_else(|| slugify(spk["name"].as_str().unwrap_or("Utilisateur")));
                let samples = spk["samplesCount"].as_i64().unwrap_or(3);
                let mut base = vec![];
                if let Some(b) = spk["base"].as_object() {
                    for (cfg, item) in b {
                        if let Some(v) = vec_of(&item["embedding"]) {
                            base.push((cfg.clone(), v, item["n"].as_u64().unwrap_or(1) as usize));
                        }
                    }
                } else {
                    // Profil antérieur : chaque variante est la moyenne de l'enrôlement.
                    if let Some(vs) = spk["variants"].as_object() {
                        for (cfg, ve) in vs {
                            if let Some(v) = vec_of(ve) {
                                base.push((cfg.clone(), v, samples.max(1) as usize));
                            }
                        }
                    }
                    if base.is_empty() {
                        if let Some(g) = general.clone() {
                            base.push(("*".into(), g, samples.max(1) as usize));
                        }
                    }
                }
                let learned = spk["learned"]
                    .as_array()
                    .map(|l| {
                        l.iter()
                            .filter_map(|item| {
                                vec_of(&item["embedding"]).map(|e| Learned {
                                    e,
                                    c: item["config"].as_str().unwrap_or("*").to_string(),
                                    at: item["at"].clone(),
                                    src: item["source"].as_str().unwrap_or("confirme").to_string(),
                                    sid: None,
                                })
                            })
                            .collect()
                    })
                    .unwrap_or_default();
                let typ = spk["type"].as_str().filter(|t| SPEAKER_TYPES.contains(t)).map(String::from);
                let mut p = Profile {
                    name: spk["name"].as_str().map(String::from).unwrap_or_else(|| capitalize(&id)),
                    id,
                    typ,
                    base,
                    learned,
                    samples_count: samples,
                    enrolled_at: spk["enrolledAt"].clone(),
                    is_primary: spk["isPrimary"].as_bool().unwrap_or(false),
                    variants: vec![],
                    embedding: vec![],
                };
                if p.rebuild() {
                    self.speakers.push(p);
                }
            }
        }

        let stale = self.available && data["embedder"].as_str() != Some(EMBEDDER_VERSION);
        if stale && !self.speakers.is_empty() {
            // Sauvegarde de l'empreinte d'origine (une seule fois) avant de la refaire.
            let backup = self.profile_path.with_file_name("voiceprint.avant-reembed.json");
            if !backup.exists() && std::fs::copy(&self.profile_path, &backup).is_ok() {
                log(&format!("ancienne empreinte sauvegardée : {}", backup.display()));
            }
            self.reembed(&data);
            migrated = true;
        }

        // Migration depuis l'ancien format mono-locuteur (même extracteur seulement).
        if self.speakers.is_empty() && !stale {
            if let Some(v) = vec_of(&data["embedding"]) {
                let name = data["name"].as_str().unwrap_or("Johann").to_string();
                let samples = data["samplesCount"].as_i64().unwrap_or(3);
                let mut p = Profile {
                    id: slugify(&name),
                    name,
                    typ: Some("moi".into()),
                    base: vec![("*".into(), v, samples.max(1) as usize)],
                    learned: vec![],
                    samples_count: samples,
                    enrolled_at: data["enrolledAt"].clone(),
                    is_primary: true,
                    variants: vec![],
                    embedding: vec![],
                };
                p.rebuild();
                self.speakers.push(p);
            }
        }

        if !self.speakers.is_empty() {
            if !self.speakers.iter().any(|s| s.is_primary) {
                self.speakers[0].is_primary = true;
            }
            for s in self.speakers.iter_mut() {
                if s.typ.is_none() {
                    s.typ = Some(if s.is_primary { "moi".into() } else { "autre".into() });
                }
            }
            self.set_reference();
            self.enrolled = true;
            let names = self.speakers.iter().map(|s| s.name.clone()).collect::<Vec<_>>().join(", ");
            log(&format!(
                "profils locuteurs chargés : {} profil(s) [{names}], seuil: {:.2}, mode: {}, marge de doute: {:.2}, actif: {}",
                self.speakers.len(),
                self.threshold,
                self.mode,
                self.doubt_margin,
                if self.enabled { "True" } else { "False" }
            ));
            if migrated {
                log("profil mis à jour (schéma 2) : seules les voix reconnues sont désormais prises en compte");
                self.save_profile();
            }
            return;
        }
        self.enrolled = false;
    }

    /// Empreintes d'un ancien extracteur : le profil principal est refait à partir du corpus.
    fn reembed(&mut self, data: &J) {
        let corpus = self.corpus_embeddings();
        let mut kept = vec![];
        for mut spk in std::mem::take(&mut self.speakers) {
            spk.learned.clear();
            spk.base.clear();
            if spk.is_primary && !corpus.is_empty() {
                spk.base = corpus
                    .iter()
                    .map(|(cfg, v)| {
                        let mut m = vec![0f32; v[0].len()];
                        for e in v {
                            m.iter_mut().zip(e).for_each(|(a, b)| *a += b);
                        }
                        let n = v.len() as f32;
                        m.iter_mut().for_each(|a| *a /= n);
                        (cfg.clone(), norm(&m), v.len())
                    })
                    .collect();
                spk.samples_count = corpus.iter().map(|(_, v)| v.len() as i64).sum();
            }
            if spk.rebuild() {
                log(&format!("empreinte de {} refaite avec le nouvel extracteur ({} phrases du corpus)", spk.name, spk.samples_count));
                kept.push(spk);
            } else {
                log(&format!("empreinte de {} à réenregistrer (ancien extracteur, incompatible)", spk.name));
            }
        }
        self.speakers = kept;
        self.thresholds.clear();
        if data["threshold"].as_f64().map(|t| t > 0.6).unwrap_or(false) {
            self.threshold = DEFAULT_THRESHOLD;
        }
    }

    /// Empreintes des phrases du corpus de l'utilisateur, par configuration audio.
    fn corpus_embeddings(&mut self) -> Vec<(String, Vec<Vec<f32>>)> {
        let Some(items) = std::fs::read_to_string(self.corpus_dir.join("manifest.json")).ok().and_then(|s| serde_json::from_str::<J>(&s).ok()) else {
            return vec![];
        };
        let mut out: Vec<(String, Vec<Vec<f32>>)> = vec![];
        for item in items["items"].as_array().cloned().unwrap_or_default() {
            if item["kind"] != "phrase" || !item["file"].is_string() {
                continue;
            }
            let Some((audio, sr)) = crate::util::read_wav(&self.corpus_dir.join(item["file"].as_str().unwrap())) else { continue };
            if let Some(emb) = self.compute_embedding(&audio, sr) {
                let cfg = item["config"].as_str().unwrap_or("*").to_string();
                match out.iter_mut().find(|(c, _)| *c == cfg) {
                    Some((_, v)) => v.push(emb),
                    None => out.push((cfg, vec![emb])),
                }
            }
        }
        out
    }

    pub fn save_profile(&mut self) {
        if let Some(parent) = self.profile_path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if self.primary().is_none() {
            // Un contact de visio n'est jamais le profil principal (celui de l'utilisateur).
            if let Some(p) = self.speakers.iter_mut().find(|s| s.typ.as_deref() != Some("visio")) {
                p.is_primary = true;
            }
        }
        let primary = self.primary().cloned();
        let list: Vec<J> = self
            .speakers
            .iter()
            .map(|s| {
                let mut variants = Map::new();
                for (c, v) in &s.variants {
                    variants.insert(c.clone(), dump(v));
                }
                let mut base = Map::new();
                for (c, v, n) in &s.base {
                    base.insert(c.clone(), json!({ "n": n, "embedding": dump(v) }));
                }
                json!({
                    "id": s.id,
                    "name": s.name,
                    "type": s.typ.clone().unwrap_or_else(|| "autre".into()),
                    "isPrimary": s.is_primary,
                    "samplesCount": s.samples_count,
                    "learnedCount": s.learned.len(),
                    "enrolledAt": s.enrolled_at,
                    "embedding": dump(&s.embedding),
                    "variants": variants,
                    "base": base,
                    "learned": s.learned.iter().map(|l| json!({ "embedding": dump(&l.e), "config": if l.c.is_empty() { "*" } else { &l.c }, "at": l.at, "source": l.src })).collect::<Vec<_>>()
                })
            })
            .collect();
        let mut thresholds = Map::new();
        for (k, v) in &self.thresholds {
            thresholds.insert(k.clone(), json!(r3(*v)));
        }
        let payload = json!({
            "schema": PROFILE_SCHEMA,
            "enrolled": !self.speakers.is_empty(),
            "enabled": self.enabled,
            "threshold": r3(self.threshold),
            "mode": self.mode,
            "doubtMargin": r3(self.doubt_margin),
            "autoLearn": self.auto_learn,
            "learning": self.learning,
            "thresholds": thresholds,
            "speakersCount": self.speakers.len(),
            "speakers": list,
            // Rétrocompatibilité mono-locuteur
            "enrolledAt": primary.as_ref().map(|p| p.enrolled_at.clone()).unwrap_or(Value::Null),
            "samplesCount": primary.as_ref().map(|p| p.samples_count).unwrap_or(0),
            "embedding": primary.as_ref().map(|p| dump(&p.embedding)).unwrap_or(Value::Null),
            "model": "wespeaker_en_voxceleb_CAM++",
            "embedder": EMBEDDER_VERSION
        });
        let tmp = PathBuf::from(format!("{}.tmp", self.profile_path.display()));
        let text = crate::util::json_indent1(&payload);
        if std::fs::write(&tmp, text).is_ok() {
            let _ = std::fs::rename(&tmp, &self.profile_path);
        }
        log(&format!("profil sauvegardé dans {} ({} locuteur(s))", self.profile_path.display(), self.speakers.len()));
    }

    /// Vecteur 512 normalisé L2 pour un signal float32.
    pub fn compute_embedding(&mut self, audio: &[f32], sample_rate: usize) -> Option<Vec<f32>> {
        let ex = self.extractor.as_mut()?;
        if (audio.len() as f64) < MIN_AUDIO_DURATION * sample_rate as f64 {
            return None;
        }
        let peak = audio.iter().fold(0f32, |m, x| m.max(x.abs()));
        let mut a: Vec<f32> = if peak > 1.5 { audio.iter().map(|x| x / 32768.0).collect() } else { audio.to_vec() };
        if sample_rate != ex.sample_rate {
            a = crate::util::interp_resample(&a, sample_rate, ex.sample_rate);
        }
        let raw = ex.embed(&a)?;
        let n: f32 = raw.iter().map(|x| x * x).sum::<f32>().sqrt();
        (n >= 1e-6).then(|| raw.iter().map(|x| x / n).collect())
    }

    // --- Identification : trois verdicts ------------------------------------------------------------

    /// Profils classés du plus au moins ressemblant (meilleure similarité entre l'empreinte générale
    /// et les variantes).
    pub fn rank(&self, emb: &[f32]) -> Vec<(usize, f64)> {
        let mut out: Vec<(usize, f64)> = self
            .speakers
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let best = std::iter::once(&s.embedding).chain(s.variants.iter().map(|(_, v)| v)).map(|r| dot(emb, r)).fold(f64::MIN, f64::max);
                (i, best)
            })
            .collect();
        out.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal));
        out
    }

    pub fn state_for(&self, best: f64, second: f64, threshold: f64) -> &'static str {
        if best >= threshold {
            if second >= threshold - self.doubt_margin / 2.0 && best - second < AMBIGUOUS_GAP {
                return "doubt";
            }
            return "known";
        }
        if best >= threshold - self.doubt_margin {
            "doubt"
        } else {
            "unknown"
        }
    }

    pub fn threshold_for(&self, duration: f64) -> f64 {
        let b = duration_bucket(duration);
        self.thresholds.iter().find(|(k, _)| k == b).map(|(_, v)| *v).unwrap_or(self.threshold)
    }

    /// Qui parle ? Verdict sans filtrage ni journal. None : pas d'empreinte, filtrage coupé, module
    /// absent ou segment trop court. Rend aussi l'empreinte calculée.
    pub fn identify(&mut self, audio: &[f32], sample_rate: usize) -> Option<(J, Vec<f32>)> {
        if !self.available || !self.enabled || self.speakers.is_empty() {
            return None;
        }
        let emb = self.compute_embedding(audio, sample_rate)?;
        let ranked = self.rank(&emb);
        let (bi, sim) = *ranked.first()?;
        let duration = audio.len() as f64 / sample_rate as f64;
        let threshold = self.threshold_for(duration);
        let second = ranked.get(1).copied();
        let state = self.state_for(sim, second.map(|s| s.1).unwrap_or(-1.0), threshold);
        Some((
            json!({
                "state": state,
                "similarity": r3(sim.max(0.0)),
                "threshold": r3(threshold),
                "duration": (duration * 100.0).round() / 100.0,
                "speaker": self.speakers[bi].who(Some(sim)),
                "second": second.map(|(i, s)| self.speakers[i].who(Some(s))).unwrap_or(Value::Null)
            }),
            emb,
        ))
    }

    /// Identifie quel locuteur parle parmi les profils (1:N) : matched, similarity, decision
    /// (accepted | doubt | rejected | unknown | bypass | disabled), state, reason, speaker,
    /// candidate, second, sample.
    pub fn verify(&mut self, audio: &[f32]) -> J {
        if !self.available {
            return json!({ "matched": true, "similarity": 1.0, "decision": "bypass", "reason": "module_indisponible", "speaker": null });
        }
        if !self.enabled {
            return json!({ "matched": true, "similarity": 1.0, "decision": "disabled", "reason": "filtrage_desactive", "speaker": null });
        }
        if self.speakers.is_empty() {
            return json!({ "matched": true, "similarity": 1.0, "decision": "bypass", "reason": "aucune_empreinte", "speaker": null });
        }
        let Some((ident, emb)) = self.identify(audio, SR) else {
            // Segment trop court : bypass pour ne pas bloquer les ordres ultra-brefs.
            return json!({ "matched": true, "similarity": 0.0, "decision": "bypass", "reason": "audio_court", "speaker": null });
        };
        let state = ident["state"].as_str().unwrap_or("unknown").to_string();
        let sim = ident["similarity"].as_f64().unwrap_or(0.0);
        let threshold = ident["threshold"].as_f64().unwrap_or(self.threshold);
        let duration = ident["duration"].as_f64().unwrap_or(0.0);
        let sample = self.remember(emb, duration);
        let detail = format!("{duration:.1} s, {}, {}", duration_bucket(duration), self.config);
        let candidate = ident["speaker"].clone();
        let cname = candidate["name"].as_str().unwrap_or("").to_string();
        let (matched, decision, speaker_info, reason);
        if self.mode == "signal" {
            if sim >= threshold {
                matched = true;
                decision = "accepted";
                speaker_info = candidate.clone();
                reason = format!("voix reconnue : {cname} ({sim:.2} >= seuil {threshold:.2})");
                log(&format!("LOCUTEUR IDENTIFIÉ : {cname} (score: {sim:.2} · {detail})"));
            } else {
                matched = true;
                decision = "unknown";
                speaker_info = Value::Null;
                reason = format!("voix non reconnue (meilleure similarité {sim:.2} < seuil {threshold:.2})");
                log(&format!("VOIX SIGNALÉE : {reason} · {detail}"));
            }
        } else if state == "known" && candidate["type"] == "visio" {
            // Voix apprise en réunion : reconnue, mais ne commande pas Jarvis.
            matched = false;
            decision = "rejected";
            speaker_info = Value::Null;
            reason = format!("voix d'un contact de visio : {cname} ({sim:.2}), pas de commande");
            log(&format!("VOIX REJETÉE : {reason} · {detail}"));
        } else if state == "known" {
            matched = true;
            decision = "accepted";
            speaker_info = candidate.clone();
            reason = format!("voix reconnue : {cname} ({sim:.2} >= seuil {threshold:.2})");
            log(&format!("LOCUTEUR IDENTIFIÉ : {cname} (score: {sim:.2} · {detail})"));
            let id = candidate["id"].as_str().unwrap_or("").to_string();
            self.auto_learn_sample(&id, &sample, sim, threshold, duration);
        } else if state == "doubt" {
            matched = false;
            decision = "doubt";
            speaker_info = Value::Null;
            let second = &ident["second"];
            reason = if sim >= threshold && second.is_object() {
                format!(
                    "voix hésitante entre {cname} et {} ({sim:.2} / {:.2})",
                    second["name"].as_str().unwrap_or(""),
                    second["similarity"].as_f64().unwrap_or(0.0)
                )
            } else {
                format!("voix douteuse : {cname} ? ({sim:.2}, seuil {threshold:.2})")
            };
            log(&format!("VOIX DOUTEUSE : {reason} · {detail}"));
        } else {
            matched = false;
            decision = "rejected";
            speaker_info = Value::Null;
            reason = format!("voix non reconnue (meilleure similarité {sim:.2} < seuil {threshold:.2})");
            log(&format!("VOIX REJETÉE : {reason} · {detail}"));
        }
        json!({
            "matched": matched, "similarity": sim, "threshold": threshold, "duration": duration,
            "decision": decision, "state": state, "reason": reason, "speaker": speaker_info,
            "candidate": candidate, "second": ident["second"], "sample": sample
        })
    }

    // --- Apprentissage ------------------------------------------------------------------------------

    /// Garde l'empreinte d'une phrase (15 min) : l'utilisateur peut dire qui parlait.
    pub fn remember(&mut self, emb: Vec<f32>, _duration: f64) -> String {
        let now = Instant::now();
        self.recent.retain(|(_, r)| now.duration_since(r.at).as_secs_f64() <= RECENT_TTL);
        while self.recent.len() >= RECENT_MAX {
            self.recent.pop_front();
        }
        self.recent_seq += 1;
        let sid = format!("v{}", self.recent_seq);
        self.recent.push_back((sid.clone(), Recent { e: emb, c: self.config.clone(), at: now }));
        sid
    }

    fn recent(&self, sid: &str) -> Option<&Recent> {
        self.recent.iter().find(|(k, _)| k == sid).map(|(_, r)| r)
    }

    /// Ajoute une phrase au profil (les plus anciennes automatiques partent d'abord).
    fn add_learned(spk: &mut Profile, e: Vec<f32>, config: &str, source: &str, sample: Option<String>) {
        spk.learned.push(Learned { e, c: if config.is_empty() { "*".into() } else { config.into() }, at: json!(now_utc()), src: source.into(), sid: sample });
        let autos: Vec<usize> = spk.learned.iter().enumerate().filter(|(_, l)| l.src == "auto").map(|(i, _)| i).collect();
        if autos.len() > MAX_AUTO {
            spk.learned.remove(autos[0]);
        }
        while spk.learned.len() > MAX_LEARNED {
            let i = spk.learned.iter().position(|l| l.src == "auto").unwrap_or(0);
            spk.learned.remove(i);
        }
        spk.rebuild();
    }

    /// Phrase reconnue avec une grande assurance : apprise d'elle-même (micro et pièce réels).
    fn auto_learn_sample(&mut self, id: &str, sample: &str, sim: f64, threshold: f64, duration: f64) {
        if !self.learning || !self.auto_learn || sim < threshold + AUTO_LEARN_MARGIN || duration < AUTO_LEARN_MIN_SECONDS {
            return;
        }
        if self.last_auto.map(|t| t.elapsed().as_secs_f64() < AUTO_LEARN_EVERY).unwrap_or(false) {
            return;
        }
        let Some((e, c)) = self.recent(sample).map(|r| (r.e.clone(), r.c.clone())) else { return };
        let Some(spk) = self.speaker_mut(id) else { return };
        Self::add_learned(spk, e, &c, "auto", Some(sample.to_string()));
        let name = spk.name.clone();
        self.last_auto = Some(Instant::now());
        log(&format!("apprentissage automatique : phrase ajoutée au profil de {name} ({sim:.2})"));
        self.save_profile();
    }

    /// L'utilisateur confirme qui parlait : la phrase rejoint ce profil.
    pub fn learn(&mut self, sample: &str, speaker_id: &str, source: &str) -> J {
        let id = slugify(speaker_id);
        if self.speaker(&id).is_none() {
            return json!({ "ok": false, "error": "locuteur_introuvable" });
        }
        let Some((e, c)) = self.recent(sample).map(|r| (r.e.clone(), r.c.clone())) else {
            return json!({ "ok": false, "error": "phrase_expiree", "message": "Cette phrase est trop ancienne pour être apprise." });
        };
        let spk = self.speaker(&id).unwrap();
        if spk.learned.iter().any(|l| l.sid.as_deref() == Some(sample)) {
            return json!({ "ok": true, "already": true, "speaker": spk.who(None), "learned": spk.learned.len() });
        }
        // Une phrase attribuée par erreur à quelqu'un d'autre lui est retirée.
        for other in self.speakers.iter_mut().filter(|s| s.id != id) {
            if other.learned.iter().any(|l| l.sid.as_deref() == Some(sample)) {
                other.learned.retain(|l| l.sid.as_deref() != Some(sample));
                other.rebuild();
            }
        }
        let spk = self.speaker_mut(&id).unwrap();
        Self::add_learned(spk, e, &c, source, Some(sample.to_string()));
        let (count, who, name) = (spk.learned.len(), spk.who(None), spk.name.clone());
        log(&format!("phrase apprise pour {name} ({source}, {count} phrase(s) apprise(s))"));
        self.save_profile();
        json!({ "ok": true, "speaker": who, "learned": count, "status": self.get_status() })
    }

    /// Annule l'apprentissage d'une phrase (mauvais bouton).
    pub fn unlearn(&mut self, sample: &str) -> J {
        let mut found = None;
        for (i, spk) in self.speakers.iter_mut().enumerate() {
            let before = spk.learned.len();
            spk.learned.retain(|l| l.sid.as_deref() != Some(sample));
            if spk.learned.len() != before {
                found = Some((i, spk.rebuild()));
                break;
            }
        }
        match found {
            None => return json!({ "ok": false, "error": "phrase_inconnue" }),
            // Profil créé depuis cette seule phrase : il n'a plus rien, il disparaît.
            Some((i, false)) => {
                self.speakers.remove(i);
            }
            _ => {}
        }
        self.save_profile();
        json!({ "ok": true, "status": self.get_status() })
    }

    /// Nouvelle personne (HUD) : un profil naît de cette phrase. Un prénom déjà enregistré complète ce
    /// profil au lieu d'en créer un second.
    pub fn create_from_sample(&mut self, sample: &str, name: &str, speaker_type: &str) -> J {
        let clean: String = crate::re!(r"\s+").replace_all(name, " ").trim().chars().take(40).collect();
        if clean.is_empty() {
            return json!({ "ok": false, "error": "nom_invalide" });
        }
        let kind = if SPEAKER_TYPES.contains(&speaker_type) { speaker_type } else { "autre" };
        let id = slugify(&clean);
        let Some((e, c)) = self.recent(sample).map(|r| (r.e.clone(), r.c.clone())) else {
            return json!({ "ok": false, "error": "phrase_expiree", "message": "Cette phrase est trop ancienne pour être apprise." });
        };
        if let Some(existing) = self.speaker_mut(&id) {
            if kind != "autre" {
                existing.typ = Some(kind.into());
            } else if existing.typ.is_none() {
                existing.typ = Some(kind.into());
            }
            let mut res = self.learn(sample, &id, "confirme");
            res["created"] = json!(false);
            return res;
        }
        for other in self.speakers.iter_mut() {
            if other.learned.iter().any(|l| l.sid.as_deref() == Some(sample)) {
                other.learned.retain(|l| l.sid.as_deref() != Some(sample));
                other.rebuild();
            }
        }
        let mut spk = Profile {
            id: id.clone(),
            name: clean.clone(),
            typ: Some(kind.into()),
            base: vec![],
            learned: vec![],
            samples_count: 0,
            enrolled_at: json!(now_utc()),
            is_primary: self.speakers.is_empty(),
            variants: vec![],
            embedding: vec![],
        };
        Self::add_learned(&mut spk, e, &c, "creation", Some(sample.to_string()));
        let who = spk.who(None);
        self.speakers.push(spk);
        self.enrolled = true;
        log(&format!("nouvelle personne : {clean} ({kind}), profil créé depuis une phrase"));
        self.save_profile();
        json!({ "ok": true, "created": true, "speaker": who, "learned": 1, "status": self.get_status() })
    }

    /// Voix nommée dans la card de réunion : ses passages créent ou complètent le profil.
    pub fn learn_embeddings(&mut self, name: &str, embeddings: &[Vec<f32>], speaker_type: &str, config: &str, source: &str) -> J {
        let clean: String = crate::re!(r"\s+").replace_all(name, " ").trim().chars().take(40).collect();
        if clean.is_empty() {
            return json!({ "ok": false, "error": "nom_invalide" });
        }
        let vecs: Vec<Vec<f32>> = embeddings.iter().filter(|v| v.len() == 512).map(|v| norm(v)).take(MAX_MEETING_SAMPLES).collect();
        if vecs.is_empty() {
            return json!({ "ok": false, "error": "aucune_empreinte", "message": "Pas assez de voix pour l'apprendre." });
        }
        let kind = if SPEAKER_TYPES.contains(&speaker_type) { speaker_type } else { "visio" };
        let id = slugify(&clean);
        let created = self.speaker(&id).is_none();
        if created {
            let first = self.speakers.is_empty();
            self.speakers.push(Profile {
                id: id.clone(),
                name: clean.clone(),
                typ: Some(kind.into()),
                base: vec![],
                learned: vec![],
                samples_count: 0,
                enrolled_at: json!(now_utc()),
                is_primary: first && kind == "moi",
                variants: vec![],
                embedding: vec![],
            });
            self.enrolled = true;
        }
        let n = vecs.len();
        let spk = self.speaker_mut(&id).unwrap();
        for v in vecs {
            Self::add_learned(spk, v, config, source, None);
        }
        let who = spk.who(None);
        log(&format!("réunion : {n} passage(s) appris pour {clean}{}", if created { " (nouveau profil)" } else { "" }));
        self.save_profile();
        json!({ "ok": true, "created": created, "speaker": who, "learned": n, "status": self.get_status() })
    }

    pub fn set_type(&mut self, speaker_id: &str, speaker_type: &str) -> J {
        let id = slugify(speaker_id);
        if !SPEAKER_TYPES.contains(&speaker_type) || self.speaker(&id).is_none() {
            return json!({ "ok": false, "error": "locuteur_ou_type_invalide" });
        }
        self.speaker_mut(&id).unwrap().typ = Some(speaker_type.into());
        self.save_profile();
        json!({ "ok": true, "status": self.get_status() })
    }

    pub fn set_options(&mut self, doubt_margin: Option<f64>, auto_learn: Option<bool>, learning: Option<bool>) -> J {
        if let Some(d) = doubt_margin {
            self.doubt_margin = d.clamp(0.0, 0.3);
        }
        if let Some(a) = auto_learn {
            self.auto_learn = a;
        }
        if let Some(l) = learning {
            self.learning = l;
        }
        if self.enrolled {
            self.save_profile();
        }
        self.get_status()
    }

    // --- Enrôlement ---------------------------------------------------------------------------------

    pub fn start_enrollment(&mut self, name: &str, target_count: usize, speaker_type: Option<&str>) -> J {
        let clean = if name.trim().is_empty() { "Utilisateur".to_string() } else { name.trim().to_string() };
        self.enroll_target_id = slugify(&clean);
        self.enroll_target_name = clean.clone();
        self.enroll_target_count = target_count.clamp(3, 12);
        self.enroll_config = Some(self.config.clone());
        self.enroll_type = speaker_type.filter(|t| SPEAKER_TYPES.contains(t)).map(String::from);
        self.is_enrolling = true;
        self.enroll_samples.clear();
        log(&format!(
            "démarrage enrôlement pour « {clean} » (id: {}, {} répliques, {})",
            self.enroll_target_id, self.enroll_target_count, self.config
        ));
        self.get_status()
    }

    pub fn cancel_enrollment(&mut self) -> J {
        self.is_enrolling = false;
        self.enroll_samples.clear();
        log("enrôlement annulé");
        self.get_status()
    }

    pub fn enroll_sample(&mut self, audio: &[f32]) -> J {
        if !self.is_enrolling {
            return json!({ "ok": false, "error": "enrolement_non_demarre" });
        }
        let (target, tname) = (self.enroll_target_count, self.enroll_target_name.clone());
        if audio.len() < SR / 2 {
            return json!({
                "ok": false, "error": "trop_court", "message": "Parlez un peu plus longtemps pour bien capter votre voix.",
                "current": self.enroll_samples.len(), "target": target, "targetName": tname
            });
        }
        let Some(emb) = self.compute_embedding(audio, SR) else {
            return json!({
                "ok": false, "error": "echec_extraction", "message": "Signal audio insuffisant, réessayez.",
                "current": self.enroll_samples.len(), "target": target, "targetName": tname
            });
        };
        self.enroll_samples.push(emb);
        let current = self.enroll_samples.len();
        log(&format!("échantillon {current}/{target} capté pour {tname}"));
        if current < target {
            return json!({
                "ok": true, "completed": false,
                "message": format!("Échantillon {current}/{target} enregistré pour {tname}. Continuez à parler."),
                "current": current, "target": target, "targetName": tname
            });
        }
        // Enrôlement complet : moyenne vectorielle et normalisation L2.
        let mut mean = vec![0f32; self.enroll_samples[0].len()];
        for e in &self.enroll_samples {
            mean.iter_mut().zip(e).for_each(|(a, b)| *a += b);
        }
        mean.iter_mut().for_each(|a| *a /= current as f32);
        let final_emb = norm(&mean);
        let id = self.enroll_target_id.clone();
        let is_first = self.speakers.is_empty();
        let existing = self.speaker(&id).cloned();
        let is_primary = existing.as_ref().map(|e| e.is_primary).unwrap_or(is_first);
        let cfg = self.enroll_config.clone().unwrap_or_else(|| self.config.clone());
        let mut base: Vec<(String, Vec<f32>, usize)> = existing.as_ref().map(|e| e.base.clone()).unwrap_or_default();
        base.retain(|(c, _, _)| *c != cfg);
        base.push((cfg, final_emb, current));
        let typ = self.enroll_type.clone().or_else(|| existing.as_ref().and_then(|e| e.typ.clone())).unwrap_or_else(|| if is_primary { "moi".into() } else { "autre".into() });
        let mut spk = Profile {
            id: id.clone(),
            name: tname.clone(),
            typ: Some(typ),
            base,
            learned: existing.as_ref().map(|e| e.learned.clone()).unwrap_or_default(),
            samples_count: current as i64,
            enrolled_at: json!(now_utc()),
            is_primary,
            variants: vec![],
            embedding: vec![],
        };
        spk.rebuild();
        match self.speakers.iter().position(|s| s.id == id) {
            Some(i) => self.speakers[i] = spk,
            None => self.speakers.push(spk),
        }
        self.enrolled = true;
        self.enabled = true;
        if is_primary {
            self.set_reference();
        }
        self.is_enrolling = false;
        self.enroll_samples.clear();
        self.save_profile();
        log(&format!("enrôlement terminé avec succès pour {tname} !"));
        json!({
            "ok": true, "completed": true,
            "message": format!("Empreinte vocale de {tname} enregistrée avec succès !"),
            "current": current, "target": target,
            "speaker": { "id": id, "name": tname },
            "status": self.get_status()
        })
    }

    pub fn remove_speaker(&mut self, speaker_id: &str) -> J {
        let id = slugify(speaker_id);
        let Some(i) = self.speakers.iter().position(|s| s.id == id) else { return json!({ "ok": false, "error": "locuteur_introuvable" }) };
        let removed = self.speakers.remove(i);
        log(&format!("locuteur supprimé : {} ({id})", removed.name));
        if self.speakers.is_empty() {
            self.enrolled = false;
            self.enrolled_at = Value::Null;
            self.samples_count = 0;
        } else if removed.is_primary {
            self.speakers[0].is_primary = true;
            self.set_reference();
        }
        self.save_profile();
        json!({ "ok": true, "status": self.get_status() })
    }

    pub fn rename_speaker(&mut self, speaker_id: &J, new_name: Option<&str>) -> J {
        let (sid, name) = if speaker_id.is_object() {
            (speaker_id["id"].as_str().unwrap_or("").to_string(), new_name.map(String::from).or_else(|| speaker_id["name"].as_str().map(String::from)))
        } else {
            (speaker_id.as_str().unwrap_or("").to_string(), new_name.map(String::from))
        };
        let id = slugify(&sid);
        let clean = name.unwrap_or_default().trim().to_string();
        if clean.is_empty() {
            return json!({ "ok": false, "error": "nom_invalide" });
        }
        let Some(spk) = self.speaker_mut(&id) else { return json!({ "ok": false, "error": "locuteur_introuvable" }) };
        spk.name = clean.clone();
        self.save_profile();
        log(&format!("locuteur renommé : {id} -> {clean}"));
        json!({ "ok": true, "status": self.get_status() })
    }

    pub fn set_primary(&mut self, speaker_id: &str) -> J {
        let id = slugify(speaker_id);
        if self.speaker(&id).is_none() {
            return json!({ "ok": false, "error": "locuteur_introuvable" });
        }
        for s in self.speakers.iter_mut() {
            s.is_primary = s.id == id;
        }
        self.set_reference();
        self.save_profile();
        json!({ "ok": true, "status": self.get_status() })
    }

    pub fn set_enabled(&mut self, enabled: bool) -> J {
        self.enabled = enabled;
        if self.enrolled {
            self.save_profile();
        }
        self.get_status()
    }

    pub fn set_mode(&mut self, mode: &str) -> J {
        if FILTER_MODES.contains(&mode) {
            self.mode = mode.into();
            if self.enrolled {
                self.save_profile();
            }
        }
        self.get_status()
    }

    pub fn set_threshold(&mut self, threshold: f64) -> J {
        self.threshold = r3(threshold.clamp(0.40, 0.90));
        if self.enrolled {
            self.save_profile();
        }
        self.get_status()
    }

    pub fn set_thresholds(&mut self, t: &J) {
        self.thresholds = t
            .as_object()
            .map(|m| m.iter().filter(|(k, _)| ["court", "moyen", "long"].contains(&k.as_str())).filter_map(|(k, v)| v.as_f64().map(|f| (k.clone(), f.clamp(0.40, 0.90)))).collect())
            .unwrap_or_default();
        if self.enrolled {
            self.save_profile();
        }
    }

    pub fn reset(&mut self) -> J {
        self.speakers.clear();
        self.enrolled = false;
        self.enrolled_at = Value::Null;
        self.samples_count = 0;
        self.is_enrolling = false;
        self.enroll_samples.clear();
        self.recent.clear();
        let _ = std::fs::remove_file(&self.profile_path);
        log("toutes les empreintes vocales ont été réinitialisées");
        self.get_status()
    }

    pub fn get_status(&self) -> J {
        let primary = self.primary();
        let mut thresholds = Map::new();
        for (k, v) in &self.thresholds {
            thresholds.insert(k.clone(), json!(v));
        }
        json!({
            "available": self.available,
            "enrolled": !self.speakers.is_empty(),
            "enabled": self.enabled,
            "mode": self.mode,
            "threshold": self.threshold,
            "thresholds": thresholds,
            "doubtMargin": self.doubt_margin,
            "autoLearn": self.auto_learn,
            "learning": self.learning,
            "config": self.config,
            "enrollConfig": if self.is_enrolling { json!(self.enroll_config) } else { Value::Null },
            "enrolledAt": primary.map(|p| p.enrolled_at.clone()).unwrap_or_else(|| self.enrolled_at.clone()),
            "samplesCount": primary.map(|p| p.samples_count).unwrap_or(self.samples_count),
            "isEnrolling": self.is_enrolling,
            "enrolledSamples": if self.is_enrolling { self.enroll_samples.len() } else { 0 },
            "targetSamples": if self.is_enrolling { self.enroll_target_count } else { 0 },
            "targetName": if self.is_enrolling { self.enroll_target_name.clone() } else { String::new() },
            "targetId": if self.is_enrolling { self.enroll_target_id.clone() } else { String::new() },
            "speakersCount": self.speakers.len(),
            "speakers": self.speakers.iter().map(|s| {
                let mut configs: Vec<String> = s.variants.iter().map(|(c, _)| c.clone()).collect();
                configs.sort();
                json!({
                    "id": s.id, "name": s.name, "samplesCount": s.samples_count, "enrolledAt": s.enrolled_at,
                    "isPrimary": s.is_primary, "type": s.typ.clone().unwrap_or_else(|| "autre".into()),
                    "learned": s.learned.len(), "configs": configs
                })
            }).collect::<Vec<_>>(),
            "model": "wespeaker_en_voxceleb_CAM++"
        })
    }
}

fn capitalize(s: &str) -> String {
    let mut c = s.chars();
    match c.next() {
        Some(f) => f.to_uppercase().collect::<String>() + c.as_str(),
        None => String::new(),
    }
}
