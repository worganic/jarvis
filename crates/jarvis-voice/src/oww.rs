// Mot d'éveil openWakeWord (modèle entraîné pour « Jarvis » avec wakeword-generator, idée reprise du projet Cornelius).
//
// Même calcul que la bibliothèque Python openWakeWord : tranches de 80 ms (1 280 échantillons) en valeurs 16 bits →
// mel-spectrogramme (÷ 10 + 2, 8 trames par tranche) → fenêtre de 76 trames → empreinte de 96 valeurs → les 16
// dernières empreintes → score du modèle du mot (0 à 1). Les deux modèles de prétraitement (v0.5.1) sont téléchargés une
// fois et vérifiés par empreinte ; le modèle du mot est déposé dans data/models/oww/ (jarvis.onnx par défaut).
// Il complète le détecteur phonétique (wake.rs) : voir `listener.rs` pour la combinaison des deux.

use std::collections::VecDeque;
use std::path::{Path, PathBuf};

use ort::session::{Session, SessionInputValue};
use ort::value::Tensor;

use crate::util::models_dir;

pub const CHUNK: usize = 1280;
const CONTEXT: usize = 160 * 3;
const MEL_BINS: usize = 32;
const MEL_WINDOW: usize = 76;
const MEL_MAX: usize = 970;
const FEATURES: usize = 96;
const FEATURE_WINDOW: usize = 16;
/// Après une détection, rien pendant 2 s (comme Cornelius et le testeur du générateur).
const REFRACTORY_CHUNKS: usize = 25;

pub const PREPROCESS: &[(&str, &str, &str)] = &[
    ("melspectrogram.onnx", "https://github.com/dscripka/openWakeWord/releases/download/v0.5.1/melspectrogram.onnx", "ba2b0e0f8b7b875369a2c89cb13360ff53bac436f2895cced9f479fa65eb176f"),
    ("embedding_model.onnx", "https://github.com/dscripka/openWakeWord/releases/download/v0.5.1/embedding_model.onnx", "70d164290c1d095d1d4ee149bc5e00543250a7316b59f31d056cff7bd3075c1f"),
];

pub fn dir() -> PathBuf {
    models_dir().join("oww")
}

/// Modèle du mot : data/models/oww/jarvis.onnx (ou un autre nom donné par le réglage).
pub fn word_model(name: &str) -> PathBuf {
    dir().join(if name.trim().is_empty() { "jarvis.onnx" } else { name.trim() })
}

pub fn available(name: &str) -> bool {
    word_model(name).is_file() && PREPROCESS.iter().all(|(f, _, _)| dir().join(f).is_file())
}

/// Télécharge les modèles de prétraitement manquants (empreinte SHA-256 vérifiée). Rend le premier problème.
pub async fn ensure_preprocess() -> Result<(), String> {
    use sha2::Digest;
    let d = dir();
    std::fs::create_dir_all(&d).map_err(|e| e.to_string())?;
    for (file, url, sha) in PREPROCESS {
        let path = d.join(file);
        if path.is_file() {
            continue;
        }
        let r = reqwest::Client::new().get(*url).timeout(std::time::Duration::from_secs(60)).send().await.map_err(|e| e.to_string())?;
        if !r.status().is_success() {
            return Err(format!("{file} : HTTP {}", r.status().as_u16()));
        }
        let bytes = r.bytes().await.map_err(|e| e.to_string())?;
        if hex::encode(sha2::Sha256::digest(&bytes)) != *sha {
            return Err(format!("{file} refusé : empreinte inattendue"));
        }
        std::fs::write(&path, &bytes).map_err(|e| e.to_string())?;
    }
    Ok(())
}

/// Combinaison des détecteurs : phonétique seul, openWakeWord seul, l'un ou l'autre, ou les deux d'accord.
#[derive(Clone, Copy, PartialEq, Debug)]
pub enum Mode {
    Phonetic,
    Oww,
    Either,
    Both,
}

impl Mode {
    pub fn parse(s: &str) -> Self {
        match s {
            "oww" => Mode::Oww,
            "union" => Mode::Either,
            "accord" => Mode::Both,
            _ => Mode::Phonetic,
        }
    }
}

/// Les deux détecteurs doivent avoir entendu le mot à moins de 1,5 s d'écart (mode « accord »).
pub const AGREEMENT_S: f64 = 1.5;

/// Décision d'éveil d'après les derniers instants (en secondes) où chaque détecteur a entendu le mot.
pub fn decide(mode: Mode, now: f64, phonetic_at: Option<f64>, oww_at: Option<f64>) -> bool {
    let recent = |t: Option<f64>| t.map(|t| now - t <= AGREEMENT_S).unwrap_or(false);
    let fresh = |t: Option<f64>| t.map(|t| (now - t).abs() < 1e-9).unwrap_or(false);
    match mode {
        Mode::Phonetic => fresh(phonetic_at),
        Mode::Oww => fresh(oww_at),
        Mode::Either => fresh(phonetic_at) || fresh(oww_at),
        Mode::Both => (fresh(phonetic_at) && recent(oww_at)) || (fresh(oww_at) && recent(phonetic_at)),
    }
}

fn session(path: &Path) -> Result<Session, String> {
    Session::builder(crate::util::ort_env())
        .map_err(|e| e.to_string())?
        .with_intra_threads(1)
        .map_err(|e| e.to_string())?
        .with_inter_threads(1)
        .map_err(|e| e.to_string())?
        .commit_from_file(path)
        .map_err(|e| format!("{} : {e}", path.display()))
}

pub struct OpenWakeWord {
    mel: Session,
    embed: Session,
    word: Session,
    word_input: String,
    pub threshold: f32,
    /// Échantillons en attente d'une tranche complète.
    pending: Vec<f32>,
    /// Dernière tranche et son contexte (1 280 + 480 échantillons).
    raw: VecDeque<f32>,
    mels: VecDeque<[f32; MEL_BINS]>,
    features: VecDeque<Vec<f32>>,
    refractory: usize,
    pub last_score: f32,
}

impl OpenWakeWord {
    pub fn new(name: &str, threshold: f32) -> Result<Self, String> {
        let d = dir();
        let word = session(&word_model(name))?;
        let word_input = word.inputs().first().map(|i| i.name().to_string()).ok_or("modèle du mot sans entrée")?;
        Ok(Self {
            mel: session(&d.join("melspectrogram.onnx"))?,
            embed: session(&d.join("embedding_model.onnx"))?,
            word,
            word_input,
            threshold,
            pending: Vec::with_capacity(CHUNK * 2),
            raw: VecDeque::with_capacity(CHUNK + CONTEXT),
            mels: (0..MEL_WINDOW).map(|_| [1.0; MEL_BINS]).collect(),
            features: VecDeque::new(),
            refractory: 0,
            last_score: 0.0,
        })
    }

    /// Flux continu (float32 entre -1 et 1, 16 kHz). Rend true si le mot vient d'être entendu.
    pub fn feed(&mut self, samples: &[f32]) -> bool {
        self.pending.extend_from_slice(samples);
        let mut heard = false;
        while self.pending.len() >= CHUNK {
            let chunk: Vec<f32> = self.pending.drain(..CHUNK).collect();
            if let Some(score) = self.step(&chunk) {
                self.last_score = score;
                if self.refractory > 0 {
                    self.refractory -= 1;
                } else if score >= self.threshold {
                    heard = true;
                    self.refractory = REFRACTORY_CHUNKS;
                }
            }
        }
        heard
    }

    /// Une tranche de 80 ms : nouvelles trames de mel, nouvelle empreinte, score (dès 16 empreintes réelles).
    fn step(&mut self, chunk: &[f32]) -> Option<f32> {
        for s in chunk {
            self.raw.push_back((s.clamp(-1.0, 1.0) * 32767.0).round());
        }
        while self.raw.len() > CHUNK + CONTEXT {
            self.raw.pop_front();
        }
        let input: Vec<f32> = self.raw.iter().copied().collect();
        let n = input.len();
        let x: SessionInputValue = Tensor::from_array(([1usize, n], input)).ok()?.into();
        let out = self.mel.run(vec![(std::borrow::Cow::Borrowed("input"), x)]).ok()?;
        let (_, spec) = out[0].try_extract_tensor::<f32>().ok()?;
        for frame in spec.chunks_exact(MEL_BINS) {
            let mut f = [0f32; MEL_BINS];
            for (dst, v) in f.iter_mut().zip(frame) {
                *dst = v / 10.0 + 2.0;
            }
            self.mels.push_back(f);
        }
        drop(out);
        while self.mels.len() > MEL_MAX {
            self.mels.pop_front();
        }
        if self.mels.len() < MEL_WINDOW {
            return None;
        }
        let window: Vec<f32> = self.mels.iter().skip(self.mels.len() - MEL_WINDOW).flat_map(|f| f.iter().copied()).collect();
        let x: SessionInputValue = Tensor::from_array(([1usize, MEL_WINDOW, MEL_BINS, 1], window)).ok()?.into();
        let out = self.embed.run(vec![(std::borrow::Cow::Borrowed("input_1"), x)]).ok()?;
        let (_, emb) = out[0].try_extract_tensor::<f32>().ok()?;
        self.features.push_back(emb.iter().take(FEATURES).copied().collect());
        drop(out);
        while self.features.len() > FEATURE_WINDOW {
            self.features.pop_front();
        }
        if self.features.len() < FEATURE_WINDOW {
            return None;
        }
        let feats: Vec<f32> = self.features.iter().flat_map(|f| f.iter().copied()).collect();
        let x: SessionInputValue = Tensor::from_array(([1usize, FEATURE_WINDOW, FEATURES], feats)).ok()?.into();
        let out = self.word.run(vec![(std::borrow::Cow::Owned(self.word_input.clone()), x)]).ok()?;
        let (_, score) = out[0].try_extract_tensor::<f32>().ok()?;
        score.first().copied()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn combinaison_des_detecteurs() {
        let t = 100.0;
        assert!(decide(Mode::Phonetic, t, Some(t), None));
        assert!(!decide(Mode::Phonetic, t, None, Some(t)));
        assert!(decide(Mode::Oww, t, None, Some(t)));
        assert!(decide(Mode::Either, t, None, Some(t)));
        assert!(!decide(Mode::Both, t, None, Some(t)), "un seul détecteur ne suffit pas");
        assert!(decide(Mode::Both, t, Some(t - 1.0), Some(t)), "accord à 1 s");
        assert!(!decide(Mode::Both, t, Some(t - 3.0), Some(t)), "trop éloignés");
        assert!(!decide(Mode::Either, t, Some(t - 0.5), None), "déjà signalé");
        assert_eq!(Mode::parse("accord"), Mode::Both);
        assert_eq!(Mode::parse("n'importe quoi"), Mode::Phonetic);
    }

    /// Essai réel : JARVIS_TEST_OWW=<dossier avec melspectrogram.onnx, embedding_model.onnx et le modèle du mot>,
    /// ORT_DYLIB_PATH vers onnxruntime.dll. Le silence ne doit jamais déclencher.
    #[test]
    #[ignore]
    fn le_silence_ne_declenche_pas() {
        let d = std::env::var("JARVIS_TEST_OWW").expect("JARVIS_TEST_OWW");
        std::env::set_var("JARVIS_MODELS_DIR", Path::new(&d).parent().unwrap());
        let name = std::env::var("JARVIS_TEST_OWW_MODEL").unwrap_or_else(|_| "jarvis.onnx".into());
        let mut w = OpenWakeWord::new(&name, 0.5).unwrap();
        let silence = vec![0f32; 16000 * 4];
        assert!(!w.feed(&silence));
        assert!(w.last_score < 0.5, "score {}", w.last_score);
        // Phrase enregistrée (WAV 16 kHz mono 16 bits) contenant le mot : elle doit déclencher.
        if let Ok(wav) = std::env::var("JARVIS_TEST_OWW_WAV") {
            let samples: Vec<f32> = hound::WavReader::open(&wav).unwrap().samples::<i16>().map(|s| s.unwrap() as f32 / 32768.0).collect();
            let (mut heard, mut best) = (false, 0f32);
            for block in samples.chunks(512).chain(std::iter::once(&silence[..16000])) {
                heard |= w.feed(block);
                best = best.max(w.last_score);
            }
            eprintln!("score maximal : {best:.3}");
            assert!(heard, "mot non détecté (score maximal {best:.3})");
        }
    }
}
