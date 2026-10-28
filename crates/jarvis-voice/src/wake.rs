// Mot d'éveil acoustique « Jarvis » (voice/wake.py de la v3).
//
// Détecteur de mots-clés de sherpa-onnx (zipformer zh-en 3M, ~5 Mo, CPU) en unités phonétiques :
// « Jarvis » s'écrit directement en phonèmes, sans entraîner de modèle. Il tourne en continu sur le
// signal nettoyé ; Whisper n'est sollicité que si « Jarvis » a été entendu (ou pendant une
// conversation ouverte). Réglages : data/wake-report.json (banc d'essai), sinon valeurs prudentes.

use std::ffi::CString;
use std::path::PathBuf;

use sherpa_rs::sherpa_rs_sys as sys;

use crate::util::{data_dir, models_dir, J};

const SR: i32 = 16000;
const MODEL: &str = "sherpa-onnx-kws-zipformer-zh-en-3M-2025-12-20";
const NAME: &str = "epoch-13-avg-2-chunk-8-left-64";

/// Prononciations (phonèmes ARPAbet du modèle). « Jarvis » à l'anglaise (JH) et à la française (ZH).
pub fn keyword_set(name: &str) -> &'static [&'static str] {
    match name {
        "anglais" => &["JH AA1 R V AH0 S", "JH AA1 R V IH0 S"],
        "francais" => &["ZH AA1 R V IY1 S", "ZH AA1 R V IH1 S", "JH AA1 R V IY1 S"],
        _ => &["JH AA1 R V AH0 S", "JH AA1 R V IH0 S", "ZH AA1 R V IY1 S", "ZH AA1 R V IH1 S", "JH AA1 R V IY1 S"],
    }
}

fn model_dir() -> PathBuf {
    models_dir().join("kws").join(MODEL)
}

pub fn available() -> bool {
    model_dir().join("tokens.txt").is_file()
}

/// Réglage retenu par le banc d'essai, sinon valeurs par défaut : (jeu, seuil, score).
pub fn calibrated_settings() -> (String, f32, f32) {
    let best: Option<J> = std::fs::read_to_string(data_dir().join("wake-report.json")).ok().and_then(|s| serde_json::from_str::<J>(&s).ok()).map(|v| v["meilleur"].clone());
    if let Some(b) = best {
        let k = b["keywords"].as_str().unwrap_or("");
        if ["anglais", "francais", "mixte"].contains(&k) {
            if let (Some(t), Some(s)) = (b["threshold"].as_f64(), b["score"].as_f64()) {
                return (k.to_string(), t as f32, s as f32);
            }
        }
    }
    ("mixte".into(), 0.25, 1.0)
}

pub struct WakeSpotter {
    spotter: *const sys::SherpaOnnxKeywordSpotter,
    stream: *const sys::SherpaOnnxOnlineStream,
    keywords_file: PathBuf,
    pub keywords: String,
    pub threshold: f32,
}

unsafe impl Send for WakeSpotter {}

impl WakeSpotter {
    pub fn new(keywords: &str, threshold: f32, score: f32) -> Result<Self, String> {
        let dir = model_dir();
        let lines: Vec<String> = keyword_set(keywords).iter().map(|p| format!("{p} @JARVIS")).collect();
        let keywords_file = std::env::temp_dir().join(format!("jarvis-wake-{}.txt", std::process::id()));
        std::fs::write(&keywords_file, lines.join("\n") + "\n").map_err(|e| e.to_string())?;
        let cs = |p: PathBuf| CString::new(p.to_string_lossy().as_bytes()).unwrap_or_default();
        let enc = cs(dir.join(format!("encoder-{NAME}.int8.onnx")));
        let dec = cs(dir.join(format!("decoder-{NAME}.onnx")));
        let joi = cs(dir.join(format!("joiner-{NAME}.int8.onnx")));
        let tokens = cs(dir.join("tokens.txt"));
        let kw = cs(keywords_file.clone());
        let provider = CString::new("cpu").unwrap();
        let spotter = unsafe {
            let mut cfg: sys::SherpaOnnxKeywordSpotterConfig = std::mem::zeroed();
            cfg.feat_config.sample_rate = SR;
            cfg.feat_config.feature_dim = 80;
            cfg.model_config.transducer.encoder = enc.as_ptr();
            cfg.model_config.transducer.decoder = dec.as_ptr();
            cfg.model_config.transducer.joiner = joi.as_ptr();
            cfg.model_config.tokens = tokens.as_ptr();
            cfg.model_config.num_threads = 1;
            cfg.model_config.provider = provider.as_ptr();
            cfg.max_active_paths = 4;
            cfg.keywords_score = score;
            cfg.keywords_threshold = threshold;
            cfg.num_trailing_blanks = 1;
            cfg.keywords_file = kw.as_ptr();
            sys::SherpaOnnxCreateKeywordSpotter(&cfg)
        };
        if spotter.is_null() {
            return Err("détecteur de mots-clés impossible à créer".into());
        }
        let stream = unsafe { sys::SherpaOnnxCreateKeywordStream(spotter) };
        if stream.is_null() {
            unsafe { sys::SherpaOnnxDestroyKeywordSpotter(spotter) };
            return Err("flux du détecteur impossible à créer".into());
        }
        Ok(Self { spotter, stream, keywords_file, keywords: keywords.into(), threshold })
    }

    /// Flux continu (float32, 16 kHz). Rend true si « Jarvis » vient d'être entendu.
    pub fn feed(&mut self, samples: &[f32]) -> bool {
        let mut heard = false;
        unsafe {
            sys::SherpaOnnxOnlineStreamAcceptWaveform(self.stream, SR, samples.as_ptr(), samples.len() as i32);
            while sys::SherpaOnnxIsKeywordStreamReady(self.spotter, self.stream) == 1 {
                sys::SherpaOnnxDecodeKeywordStream(self.spotter, self.stream);
                let r = sys::SherpaOnnxGetKeywordResult(self.spotter, self.stream);
                if !r.is_null() {
                    let kw = (*r).keyword;
                    if !kw.is_null() && *kw != 0 {
                        heard = true;
                        sys::SherpaOnnxResetKeywordStream(self.spotter, self.stream);
                    }
                    sys::SherpaOnnxDestroyKeywordResult(r);
                }
            }
        }
        heard
    }
}

impl Drop for WakeSpotter {
    fn drop(&mut self) {
        unsafe {
            sys::SherpaOnnxDestroyOnlineStream(self.stream);
            sys::SherpaOnnxDestroyKeywordSpotter(self.spotter);
        }
        let _ = std::fs::remove_file(&self.keywords_file);
    }
}
