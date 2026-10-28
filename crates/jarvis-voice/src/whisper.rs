// Whisper large-v3-turbo (export ONNX, onnxruntime : GPU par DirectML, sinon processeur).
//
// Remplace faster-whisper (CTranslate2) de la v3 avec les mêmes réglages de décodage :
// langue française imposée, recherche en faisceau (beam 2 pour une phrase finale, 1 sinon),
// température 0, amorce (vocabulaire de l'utilisateur, exemples de commandes) et mots prioritaires
// (« hotwords »), pénalité de répétition 1,1, interdiction des 3-grammes répétés, sans horodatages.
// Rend le texte et ce qu'exigent les filtres anti-hallucination : logprob moyen, probabilité de
// silence, taux de compression. Détection de langue (phrase anglaise écartée).

use std::collections::HashMap;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use half::f16;
use ort::session::{Session, SessionInputValue};
use ort::value::{DynValue, Tensor};
use serde_json::json;

use crate::util::{log, J};

pub const SR: usize = 16000;
const N_FFT: usize = 400;
const HOP: usize = 160;
const N_SAMPLES: usize = 30 * SR;
const N_FRAMES: usize = 3000;
const MAX_LENGTH: usize = 448;

/// Résultat d'une transcription.
#[derive(Clone, Debug, Default)]
pub struct Decoded {
    pub text: String,
    pub logprob: f64,
    pub no_speech: f64,
    pub compression: f64,
}

impl Decoded {
    pub fn info(&self) -> J {
        json!({ "logprob": self.logprob, "no_speech": self.no_speech, "compression": self.compression })
    }
}

struct Special {
    sot: u32,
    sot_prev: u32,
    eot: u32,
    transcribe: u32,
    no_timestamps: u32,
    no_speech: Option<u32>,
    lang_fr: u32,
    blank: Option<u32>,
    langs: Vec<(String, u32)>,
    suppress: Vec<u32>,
}

struct Sessions {
    encoder: Session,
    decoder: Session,
    decoder_past: Session,
}

pub struct Whisper {
    /// Sessions ONNX (GPU) : déchargées après une longue inactivité pour rendre la mémoire vidéo.
    ses: Option<Sessions>,
    dir: PathBuf,
    last_use: std::time::Instant,
    tokenizer: tokenizers::Tokenizer,
    sp: Special,
    mels: usize,
    filters: Vec<Vec<f32>>,
    window: Vec<f32>,
    fft: Arc<dyn rustfft::Fft<f32>>,
    fp16: bool,
    pub label: String,
    enc_in: String,
    past_names: Vec<String>,
}

/// Bancs de filtres mel de Whisper (librosa, échelle et normalisation « slaney »).
fn mel_filters(n_mels: usize) -> Vec<Vec<f32>> {
    let sr = SR as f64;
    let hz_to_mel = |f: f64| {
        let (f_sp, min_log_hz, min_log_mel, logstep) = (200.0 / 3.0, 1000.0, 15.0, (6.4f64).ln() / 27.0);
        if f >= min_log_hz {
            min_log_mel + (f / min_log_hz).ln() / logstep
        } else {
            f / f_sp
        }
    };
    let mel_to_hz = |m: f64| {
        let (f_sp, min_log_hz, min_log_mel, logstep) = (200.0 / 3.0, 1000.0, 15.0, (6.4f64).ln() / 27.0);
        if m >= min_log_mel {
            min_log_hz * (logstep * (m - min_log_mel)).exp()
        } else {
            f_sp * m
        }
    };
    let n_freq = 1 + N_FFT / 2;
    let fftfreqs: Vec<f64> = (0..n_freq).map(|i| i as f64 * (sr / 2.0) / (n_freq - 1) as f64).collect();
    let (mmin, mmax) = (hz_to_mel(0.0), hz_to_mel(sr / 2.0));
    let mel_f: Vec<f64> = (0..n_mels + 2).map(|i| mel_to_hz(mmin + (mmax - mmin) * i as f64 / (n_mels + 1) as f64)).collect();
    (0..n_mels)
        .map(|i| {
            let enorm = 2.0 / (mel_f[i + 2] - mel_f[i]);
            fftfreqs
                .iter()
                .map(|f| {
                    let lower = (f - mel_f[i]) / (mel_f[i + 1] - mel_f[i]);
                    let upper = (mel_f[i + 2] - f) / (mel_f[i + 2] - mel_f[i + 1]);
                    (0f64.max(lower.min(upper)) * enorm) as f32
                })
                .collect()
        })
        .collect()
}

fn log_softmax(logits: &[f32]) -> Vec<f32> {
    let max = logits.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
    let sum: f64 = logits.iter().map(|x| ((*x - max) as f64).exp()).sum();
    let lse = max as f64 + sum.ln();
    logits.iter().map(|x| (*x as f64 - lse) as f32).collect()
}

/// Taux de compression du texte (zlib), comme faster-whisper.
pub fn compression_ratio(text: &str) -> f64 {
    let bytes = text.as_bytes();
    if bytes.is_empty() {
        return 0.0;
    }
    let mut e = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
    let _ = e.write_all(bytes);
    let c = e.finish().unwrap_or_default();
    bytes.len() as f64 / c.len().max(1) as f64
}

#[derive(Clone)]
struct Hyp {
    tokens: Vec<u32>,
    logprob: f64,
}

impl Whisper {
    /// Ouvre les trois sessions du modèle ; rend aussi vrai si elles tournent sur le GPU.
    fn open_sessions(dir: &Path) -> Result<(Sessions, bool), String> {
        let onnx = dir.join("onnx");
        let pick = |base: &str, fp16_first: bool| -> PathBuf {
            let order: &[&str] = if fp16_first { &["_fp16", "", "_int8"] } else { &["", "_fp16", "_int8"] };
            for suffix in order {
                let p = onnx.join(format!("{base}{suffix}.onnx"));
                if p.is_file() {
                    return p;
                }
            }
            onnx.join(format!("{base}.onnx"))
        };
        let dec_gpu = std::env::var("JARVIS_STT_DEC_GPU").is_ok();
        // Première passe (amorce + projection du cache croisé, coûteuse) sur le GPU ; pas suivants (un
        // jeton, cache sur place) sur le processeur, en float32.
        let past_path = match std::env::var("JARVIS_STT_PAST").ok() { Some(v) => onnx.join(v), None => pick("decoder_with_past_model", dec_gpu) };
        let (enc_path, dec_path) = (pick("encoder_model", true), pick("decoder_model", true));
        // Les exports « fp16 » gardent des entrées et sorties en float32 (calculs internes en fp16).
        let gpu = std::env::var("JARVIS_STT_CPU").is_err();
        let open = |p: &Path, try_gpu: bool| -> Result<(Session, bool), String> {
            if try_gpu {
                // DirectML : ni motif mémoire ni exécution parallèle (exigence de l'EP).
                let gpu = (|| -> Result<Session, String> {
                    let e = |x: ort::Error<_>| x.to_string();
                    let b = Session::builder(crate::util::ort_env()).map_err(|x| x.to_string())?;
                    let b = b.with_memory_pattern(false).map_err(e)?;
                    use ort::session::builder::GraphOptimizationLevel as L;
                    let level = match std::env::var("JARVIS_STT_OPT").ok().as_deref() { Some("0") => L::Disable, Some("1") => L::Level1, Some("2") => L::Level2, Some("3") => L::Level3, _ => L::Level3 };
                    let b = b.with_optimization_level(level).map_err(|x| x.to_string())?;
                    let b = b.with_parallel_execution(false).map_err(|x| x.to_string())?;
                    let mut b = b.with_execution_providers([ort::ep::DirectML::default().build().error_on_failure()]).map_err(|x| x.to_string())?;
                    b.commit_from_file(p).map_err(|x| x.to_string())
                })();
                match gpu {
                    Ok(s) => return Ok((s, true)),
                    Err(e) => log(&format!("Whisper : GPU (DirectML) inutilisable pour {} ({e}), repli sur le processeur", p.display())),
                }
            }
            let mut b = Session::builder(crate::util::ort_env()).map_err(|e| e.to_string())?.with_intra_threads(std::env::var("JARVIS_STT_THREADS").ok().and_then(|t| t.parse().ok()).unwrap_or(4)).map_err(|e| e.to_string())?;
            b.commit_from_file(p).map(|s| (s, false)).map_err(|e| format!("{} : {e}", p.display()))
        };
        let tl = std::time::Instant::now();
        let (encoder, on_gpu) = open(&enc_path, gpu)?;
        let t_enc = tl.elapsed();
        // Décodeur (4 couches) sur le processeur : son cache d'attention croisée y reste, au lieu de
        // faire l'aller-retour avec le GPU à chaque jeton.
        let dec_gpu = on_gpu && dec_gpu;
        let (decoder, _) = open(&dec_path, on_gpu)?;
        let (decoder_past, _) = open(&past_path, dec_gpu)?;
        if std::env::var("JARVIS_STT_TIMING").is_ok() {
            log(&format!("Whisper : sessions chargées (encodeur {:.1} s, total {:.1} s)", t_enc.as_secs_f64(), tl.elapsed().as_secs_f64()));
        }
        if std::env::var("JARVIS_STT_TIMING").is_ok() {
            log(&format!("entrées décodeur pas-à-pas : {:?}", decoder_past.inputs().iter().map(|i| i.name().to_string()).collect::<Vec<_>>()));
            log(&format!("sorties décodeur pas-à-pas : {:?}", decoder_past.outputs().iter().map(|i| i.name().to_string()).collect::<Vec<_>>()));
        }
        Ok((Sessions { encoder, decoder, decoder_past }, on_gpu))
    }

    /// Recharge les sessions si elles ont été déchargées.
    fn ensure(&mut self) -> Result<(), String> {
        self.last_use = std::time::Instant::now();
        if self.ses.is_none() {
            let t = std::time::Instant::now();
            let (ses, _) = Self::open_sessions(&self.dir)?;
            self.ses = Some(ses);
            log(&format!("Whisper : rechargé en {:.1} s", t.elapsed().as_secs_f64()));
        }
        Ok(())
    }

    /// Libère la mémoire vidéo si Whisper n'a pas servi depuis `idle`. Rend vrai s'il vient d'être déchargé.
    pub fn unload_if_idle(&mut self, idle: std::time::Duration) -> bool {
        if self.ses.is_some() && self.last_use.elapsed() >= idle {
            self.ses = None;
            return true;
        }
        false
    }

    /// Dossier du modèle : data/models/whisper-large-v3-turbo-onnx (onnx/*.onnx, tokenizer.json…).
    pub fn load(dir: &Path) -> Result<Self, String> {
        let tokenizer = tokenizers::Tokenizer::from_file(dir.join("tokenizer.json")).map_err(|e| format!("tokenizer : {e}"))?;
        let gen: J = std::fs::read_to_string(dir.join("generation_config.json")).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        let cfg: J = std::fs::read_to_string(dir.join("config.json")).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default();
        let mels = cfg["num_mel_bins"].as_u64().unwrap_or(128) as usize;
        let id = |t: &str| tokenizer.token_to_id(t);
        let need = |t: &str| id(t).ok_or_else(|| format!("jeton {t} absent"));
        let mut langs: Vec<(String, u32)> = gen["lang_to_id"]
            .as_object()
            .map(|m| m.iter().filter_map(|(k, v)| v.as_u64().map(|i| (k.trim_start_matches("<|").trim_end_matches("|>").to_string(), i as u32))).collect())
            .unwrap_or_default();
        langs.sort_by_key(|(_, i)| *i);
        let sp = Special {
            sot: need("<|startoftranscript|>")?,
            sot_prev: need("<|startofprev|>")?,
            eot: need("<|endoftext|>")?,
            transcribe: need("<|transcribe|>")?,
            no_timestamps: need("<|notimestamps|>")?,
            no_speech: id("<|nospeech|>").or_else(|| id("<|nocaptions|>")),
            lang_fr: need("<|fr|>")?,
            blank: tokenizer.encode(" ", false).ok().and_then(|e| e.get_ids().first().copied()),
            langs,
            suppress: gen["suppress_tokens"].as_array().map(|a| a.iter().filter_map(|v| v.as_u64().map(|x| x as u32)).collect()).unwrap_or_default(),
        };
        let (ses, on_gpu) = Self::open_sessions(dir)?;
        // Les exports « fp16 » gardent des entrées et sorties en float32 (calculs internes en fp16).
        let fp16 = std::env::var("JARVIS_STT_FP16_INPUT").is_ok();
        let enc_in = ses.encoder.inputs().first().map(|i| i.name().to_string()).unwrap_or_else(|| "input_features".into());
        let past_names: Vec<String> = ses.decoder_past.inputs().iter().map(|i| i.name().to_string()).filter(|n| n.starts_with("past_key_values")).collect();
        let window: Vec<f32> = (0..N_FFT).map(|n| (0.5 - 0.5 * (2.0 * std::f64::consts::PI * n as f64 / N_FFT as f64).cos()) as f32).collect();
        let label = format!("turbo · {}", if on_gpu { "gpu/directml" } else { "cpu" });
        Ok(Self {
            ses: Some(ses),
            dir: dir.to_path_buf(),
            last_use: std::time::Instant::now(),
            tokenizer,
            sp,
            mels,
            filters: mel_filters(mels),
            window,
            fft: rustfft::FftPlanner::new().plan_fft_forward(N_FFT),
            fp16,
            label,
            enc_in,
            past_names,
        })
    }

    // --- Spectrogramme log-mel ------------------------------------------------------------------------

    /// Entrée de l'encodeur : [mels][3000] pour une fenêtre de 30 s (complétée de silence).
    fn log_mel(&self, audio: &[f32]) -> Vec<f32> {
        let mut x = vec![0f32; N_SAMPLES];
        let n = audio.len().min(N_SAMPLES);
        x[..n].copy_from_slice(&audio[..n]);
        // STFT centrée, bords réfléchis (torch.stft center=True), la dernière trame est écartée.
        let pad = N_FFT / 2;
        let mut padded = Vec::with_capacity(N_SAMPLES + 2 * pad);
        for i in (1..=pad).rev() {
            padded.push(x[i]);
        }
        padded.extend_from_slice(&x);
        for i in 0..pad {
            padded.push(x[N_SAMPLES - 2 - i]);
        }
        let n_freq = N_FFT / 2 + 1;
        let mut mel = vec![0f32; self.mels * N_FRAMES];
        let mut buf = vec![rustfft::num_complex::Complex::new(0f32, 0.0); N_FFT];
        let mut power = vec![0f32; n_freq];
        for t in 0..N_FRAMES {
            let start = t * HOP;
            for k in 0..N_FFT {
                buf[k] = rustfft::num_complex::Complex::new(padded[start + k] * self.window[k], 0.0);
            }
            self.fft.process(&mut buf);
            for k in 0..n_freq {
                power[k] = buf[k].norm_sqr();
            }
            for (m, f) in self.filters.iter().enumerate() {
                let e: f32 = f.iter().zip(&power).map(|(a, b)| a * b).sum();
                mel[m * N_FRAMES + t] = e.max(1e-10).log10();
            }
        }
        let max = mel.iter().cloned().fold(f32::NEG_INFINITY, f32::max);
        for v in mel.iter_mut() {
            *v = (v.max(max - 8.0) + 4.0) / 4.0;
        }
        mel
    }

    fn encode(&mut self, audio: &[f32]) -> Result<DynValue, String> {
        let mel = self.log_mel(audio);
        let shape = [1usize, self.mels, N_FRAMES];
        let input: SessionInputValue = if self.fp16 {
            Tensor::from_array((shape, mel.iter().map(|v| f16::from_f32(*v)).collect::<Vec<_>>())).map_err(|e| e.to_string())?.into()
        } else {
            Tensor::from_array((shape, mel)).map_err(|e| e.to_string())?.into()
        };
        let inputs: Vec<(std::borrow::Cow<str>, SessionInputValue)> = vec![(self.enc_in.clone().into(), input)];
        let mut out = self.ses.as_mut().ok_or("modèle déchargé")?.encoder.run(inputs).map_err(|e| e.to_string())?;
        let key = out.keys().next().map(|k| k.to_string()).ok_or("encodeur muet")?;
        out.remove(&key).ok_or_else(|| "sortie de l'encodeur absente".to_string())
    }

    // --- Décodage -------------------------------------------------------------------------------------

    fn tokens_of(&self, text: &str) -> Vec<u32> {
        self.tokenizer.encode(text, false).map(|e| e.get_ids().to_vec()).unwrap_or_default()
    }

    /// Jetons d'amorce : [sot_prev] + mots prioritaires + amorce, puis la séquence de départ.
    fn prompt(&self, initial_prompt: &str, hotwords: Option<&str>, lang: u32) -> (Vec<u32>, usize) {
        let mut p = vec![];
        let previous = if initial_prompt.trim().is_empty() { vec![] } else { self.tokens_of(&format!(" {}", initial_prompt.trim())) };
        let hot = hotwords.filter(|h| !h.trim().is_empty());
        if !previous.is_empty() || hot.is_some() {
            p.push(self.sp.sot_prev);
            if let Some(h) = hot {
                let mut ht = self.tokens_of(&format!(" {}", h.trim()));
                if ht.len() >= MAX_LENGTH / 2 {
                    ht.truncate(MAX_LENGTH / 2 - 1);
                }
                p.extend(ht);
            }
            if !previous.is_empty() {
                let keep = MAX_LENGTH / 2 - 1;
                let start = previous.len().saturating_sub(keep);
                p.extend_from_slice(&previous[start..]);
            }
        }
        let sot_index = p.len();
        p.extend([self.sp.sot, lang, self.sp.transcribe, self.sp.no_timestamps]);
        (p, sot_index)
    }

    fn logits_rows(out: &ort::session::SessionOutputs, rows_last_only: bool) -> Result<(Vec<usize>, Vec<f32>), String> {
        let v = &out["logits"];
        let (shape, data): (Vec<usize>, Vec<f32>) = if let Ok((s, d)) = v.try_extract_tensor::<f16>() {
            (s.iter().map(|x| *x as usize).collect(), d.iter().map(|x| x.to_f32()).collect())
        } else {
            let (s, d) = v.try_extract_tensor::<f32>().map_err(|e| e.to_string())?;
            (s.iter().map(|x| *x as usize).collect(), d.to_vec())
        };
        let _ = rows_last_only;
        Ok((shape, data))
    }

    /// Valeurs « present.* » → entrées « past_key_values.* » de l'étape suivante.
    fn take_presents(out: &mut ort::session::SessionOutputs, names: &[String], keep_encoder: Option<&HashMap<String, DynValue>>) -> HashMap<String, DynValue> {
        let mut map = HashMap::new();
        for n in names {
            let present = n.replacen("past_key_values", "present", 1);
            if let Some(v) = out.remove(&present) {
                map.insert(n.clone(), v);
            }
        }
        let _ = keep_encoder;
        map
    }

    fn suppress(&self, logits: &mut [f32], first: bool) {
        let vocab = logits.len();
        for &t in &self.sp.suppress {
            if (t as usize) < vocab {
                logits[t as usize] = f32::NEG_INFINITY;
            }
        }
        // Jetons spéciaux (langues, tâches, horodatages…) : jamais dans le texte. Fin de texte permise.
        for t in (self.sp.eot as usize + 1)..vocab {
            logits[t] = f32::NEG_INFINITY;
        }
        if first {
            if let Some(b) = self.sp.blank {
                logits[b as usize] = f32::NEG_INFINITY;
            }
            logits[self.sp.eot as usize] = f32::NEG_INFINITY;
        }
    }

    fn penalize(logits: &mut [f32], generated: &[u32], repetition_penalty: f32, no_repeat: usize) {
        if repetition_penalty != 1.0 {
            let mut seen = std::collections::HashSet::new();
            for &t in generated {
                if seen.insert(t) {
                    let l = &mut logits[t as usize];
                    *l = if *l < 0.0 { *l * repetition_penalty } else { *l / repetition_penalty };
                }
            }
        }
        if no_repeat > 0 && generated.len() + 1 >= no_repeat {
            let prefix = &generated[generated.len() + 1 - no_repeat..];
            for i in 0..=(generated.len() - (no_repeat - 1)) {
                if i + no_repeat > generated.len() {
                    break;
                }
                if generated[i..i + no_repeat - 1] == *prefix {
                    logits[generated[i + no_repeat - 1] as usize] = f32::NEG_INFINITY;
                }
            }
        }
    }

    fn kv_tensor_copy(v: &DynValue, rows: &[usize]) -> Result<DynValue, String> {
        if let Ok((shape, data)) = v.try_extract_tensor::<f16>() {
            let shape: Vec<usize> = shape.iter().map(|x| *x as usize).collect();
            let per: usize = shape[1..].iter().product();
            let mut out = Vec::with_capacity(per * rows.len());
            for &r in rows {
                out.extend_from_slice(&data[r * per..(r + 1) * per]);
            }
            let mut s = shape.clone();
            s[0] = rows.len();
            return Tensor::from_array((s, out)).map(|t| t.into_dyn()).map_err(|e| e.to_string());
        }
        let (shape, data) = v.try_extract_tensor::<f32>().map_err(|e| e.to_string())?;
        let shape: Vec<usize> = shape.iter().map(|x| *x as usize).collect();
        let per: usize = shape[1..].iter().product();
        let mut out = Vec::with_capacity(per * rows.len());
        for &r in rows {
            out.extend_from_slice(&data[r * per..(r + 1) * per]);
        }
        let mut s = shape.clone();
        s[0] = rows.len();
        Tensor::from_array((s, out)).map(|t| t.into_dyn()).map_err(|e| e.to_string())
    }

    /// Décodage d'une fenêtre (≤ 30 s). beam : taille du faisceau (1 = glouton).
    fn decode_window(&mut self, audio: &[f32], initial_prompt: &str, hotwords: Option<&str>, beam: usize) -> Result<Decoded, String> {
        let t0 = std::time::Instant::now();
        let enc = self.encode(audio)?;
        let t_enc = t0.elapsed();
        let (prompt, sot_index) = self.prompt(initial_prompt, hotwords, self.sp.lang_fr);
        let beam = beam.max(1);

        // Étape 0 : toute l'amorce, lot de 1.
        let ids: Vec<i64> = prompt.iter().map(|t| *t as i64).collect();
        let n = ids.len();
        let ids_t: SessionInputValue = Tensor::from_array(([1usize, n], ids)).map_err(|e| e.to_string())?.into();
        let mut out = self
            .ses
            .as_mut()
            .ok_or("modèle déchargé")?
            .decoder
            .run(vec![(std::borrow::Cow::Borrowed("input_ids"), ids_t), (std::borrow::Cow::Borrowed("encoder_hidden_states"), SessionInputValue::from(&enc))])
            .map_err(|e| e.to_string())?;
        let (shape, logits) = Self::logits_rows(&out, false)?;
        let vocab = shape[2];
        // Probabilité de silence : au jeton de départ (sot), comme faster-whisper.
        let no_speech = match self.sp.no_speech {
            Some(ns) => {
                let row = &logits[sot_index * vocab..(sot_index + 1) * vocab];
                (log_softmax(row)[ns as usize] as f64).exp()
            }
            None => 0.0,
        };
        let t_first = t0.elapsed() - t_enc;
        let mut first = logits[(n - 1) * vocab..n * vocab].to_vec();
        let names = self.past_names.clone();
        let mut kv = Self::take_presents(&mut out, &names, None);
        drop(out);

        let max_new = MAX_LENGTH / 2;
        let mut finished: Vec<Hyp> = vec![];
        // Faisceau : hypothèses vivantes, une ligne de cache chacune.
        self.suppress(&mut first, true);
        Self::penalize(&mut first, &[], 1.1, 3);
        let lp = log_softmax(&first);
        let mut order: Vec<usize> = (0..vocab).collect();
        order.sort_by(|a, b| lp[*b].partial_cmp(&lp[*a]).unwrap_or(std::cmp::Ordering::Equal));
        let mut alive: Vec<Hyp> = vec![];
        let mut done_first = false;
        for (rank, &t) in order.iter().take(beam * 2).enumerate() {
            let h = Hyp { tokens: vec![t as u32], logprob: lp[t] as f64 };
            if t as u32 == self.sp.eot {
                done_first |= rank == 0;
                finished.push(h);
            } else if alive.len() < beam {
                alive.push(h);
            }
            if alive.len() >= beam {
                break;
            }
        }
        if done_first {
            alive.clear();
        }
        if !alive.is_empty() {
            // Cache étendu au nombre d'hypothèses (lignes identiques).
            let rows = vec![0usize; alive.len()];
            for v in kv.values_mut() {
                *v = Self::kv_tensor_copy(v, &rows)?;
            }
        }
        let mut step = 1;
        let mut run_ms = 0f64;
        while !alive.is_empty() && step < max_new {
            let b = alive.len();
            let ids: Vec<i64> = alive.iter().map(|h| *h.tokens.last().unwrap() as i64).collect();
            let ids_t: SessionInputValue = Tensor::from_array(([b, 1usize], ids)).map_err(|e| e.to_string())?.into();
            let mut inputs: Vec<(std::borrow::Cow<'_, str>, SessionInputValue<'_>)> = vec![("input_ids".into(), ids_t)];
            let has_enc_input = self.ses.as_ref().ok_or("modèle déchargé")?.decoder_past.inputs().iter().any(|i| i.name() == "encoder_hidden_states");
            let enc_rows;
            if has_enc_input {
                enc_rows = Self::kv_tensor_copy(&enc, &vec![0usize; b])?;
                inputs.push(("encoder_hidden_states".into(), SessionInputValue::from(&enc_rows)));
            }
            for n in &names {
                if let Some(v) = kv.get(n) {
                    inputs.push((n.clone().into(), SessionInputValue::from(v)));
                }
            }
            let tr = std::time::Instant::now();
            let mut out = self.ses.as_mut().ok_or("modèle déchargé")?.decoder_past.run(inputs).map_err(|e| e.to_string())?;
            run_ms += tr.elapsed().as_secs_f64() * 1000.0;
            let (_, logits) = Self::logits_rows(&out, true)?;
            // Le cache de l'encodeur n'est pas renvoyé par le décodeur « with past » : il est gardé.
            let mut fresh = Self::take_presents(&mut out, &names, None);
            drop(out);
            for (k, v) in kv.drain() {
                fresh.entry(k).or_insert(v);
            }
            kv = fresh;

            let mut cands: Vec<(usize, u32, f64)> = vec![];
            for (i, h) in alive.iter().enumerate() {
                let mut row = logits[i * vocab..(i + 1) * vocab].to_vec();
                self.suppress(&mut row, false);
                Self::penalize(&mut row, &h.tokens, 1.1, 3);
                let lp = log_softmax(&row);
                let mut idx: Vec<usize> = (0..vocab).collect();
                idx.select_nth_unstable_by(beam * 2, |a, b| lp[*b].partial_cmp(&lp[*a]).unwrap_or(std::cmp::Ordering::Equal));
                for &t in idx.iter().take(beam * 2) {
                    if lp[t].is_finite() {
                        cands.push((i, t as u32, h.logprob + lp[t] as f64));
                    }
                }
            }
            cands.sort_by(|a, b| b.2.partial_cmp(&a.2).unwrap_or(std::cmp::Ordering::Equal));
            // Comme CTranslate2 : la recherche s'arrête quand la meilleure candidate est la fin de texte ;
            // les fins de texte moins bien classées sont gardées, le faisceau continue sans elles.
            let mut next: Vec<Hyp> = vec![];
            let mut parents: Vec<usize> = vec![];
            let mut top_finished = false;
            for (rank, (parent, tok, score)) in cands.into_iter().enumerate() {
                let mut tokens = alive[parent].tokens.clone();
                tokens.push(tok);
                let h = Hyp { tokens, logprob: score };
                if tok == self.sp.eot {
                    if rank == 0 {
                        top_finished = true;
                    }
                    finished.push(h);
                } else if next.len() < beam {
                    next.push(h);
                    parents.push(parent);
                }
                if next.len() >= beam {
                    break;
                }
            }
            if top_finished {
                break;
            }
            if next.is_empty() {
                break;
            }
            // Cache réordonné selon les hypothèses retenues (copie seulement si nécessaire).
            let identity = parents.len() == alive.len() && parents.iter().enumerate().all(|(i, p)| i == *p);
            if !identity {
                for (k, v) in kv.iter_mut() {
                    if k.contains(".decoder.") || !k.contains(".encoder.") {
                        *v = Self::kv_tensor_copy(v, &parents)?;
                    } else if parents.len() != alive.len() {
                        *v = Self::kv_tensor_copy(v, &vec![0usize; parents.len()])?;
                    }
                }
            }
            alive = next;
            step += 1;
        }
        if finished.is_empty() || (step >= max_new && !alive.is_empty()) {
            finished.extend(alive);
        }
        // Meilleure hypothèse : logprob normalisé par la longueur (length_penalty = 1).
        let best = finished
            .into_iter()
            .max_by(|a, b| (a.logprob / a.tokens.len() as f64).partial_cmp(&(b.logprob / b.tokens.len() as f64)).unwrap_or(std::cmp::Ordering::Equal))
            .unwrap_or(Hyp { tokens: vec![], logprob: 0.0 });
        let text_tokens: Vec<u32> = best.tokens.iter().copied().filter(|t| *t != self.sp.eot).collect();
        let text = self.tokenizer.decode(&text_tokens, true).unwrap_or_default().trim().to_string();
        let avg = best.logprob / (text_tokens.len() as f64 + 1.0);
        if std::env::var("JARVIS_STT_TIMING").is_ok() {
            log(&format!("Whisper : encodeur {} ms, amorce {} ms ({} jetons), pas {} ms dont onnx {:.0} ms ({} jetons)", t_enc.as_millis(), t_first.as_millis(), n, (t0.elapsed() - t_enc - t_first).as_millis(), run_ms, text_tokens.len()));
        }
        Ok(Decoded { compression: compression_ratio(&text), text, logprob: avg, no_speech })
    }

    /// Transcription. Au-delà de 30 s (réunions), l'audio est découpé en fenêtres de 30 s.
    pub fn transcribe(&mut self, audio: &[f32], initial_prompt: &str, hotwords: Option<&str>, beam: usize) -> Decoded {
        if let Err(e) = self.ensure() {
            log(&format!("Whisper : rechargement impossible ({e})"));
            return Decoded { text: String::new(), logprob: 0.0, no_speech: 1.0, compression: 0.0 };
        }
        let mut parts = vec![];
        let mut logprobs = vec![];
        let mut no_speech = 0f64;
        let mut compression = 0f64;
        let mut start = 0;
        while start < audio.len().max(1) {
            let end = (start + N_SAMPLES).min(audio.len());
            match self.decode_window(&audio[start..end], initial_prompt, hotwords, beam) {
                Ok(d) => {
                    if !d.text.is_empty() {
                        parts.push(d.text.clone());
                    }
                    logprobs.push(d.logprob);
                    no_speech = no_speech.max(d.no_speech);
                    compression = compression.max(d.compression);
                }
                Err(e) => log(&format!("Whisper : échec du décodage ({e})")),
            }
            if end >= audio.len() {
                break;
            }
            start = end;
        }
        Decoded {
            text: parts.join(" ").trim().to_string(),
            logprob: if logprobs.is_empty() { 0.0 } else { logprobs.iter().sum::<f64>() / logprobs.len() as f64 },
            no_speech,
            compression,
        }
    }

    /// Langue la plus probable : (code, probabilité).
    pub fn detect_language(&mut self, audio: &[f32]) -> Option<(String, f64)> {
        self.ensure().ok()?;
        let enc = self.encode(audio).ok()?;
        let ids_t: SessionInputValue = Tensor::from_array(([1usize, 1usize], vec![self.sp.sot as i64])).ok()?.into();
        let out = self.ses.as_mut()?.decoder.run(vec![(std::borrow::Cow::Borrowed("input_ids"), ids_t), (std::borrow::Cow::Borrowed("encoder_hidden_states"), SessionInputValue::from(&enc))]).ok()?;
        let (shape, logits) = Self::logits_rows(&out, true).ok()?;
        let vocab = shape[2];
        let row = &logits[..vocab];
        let langs = &self.sp.langs;
        if langs.is_empty() {
            return None;
        }
        let vals: Vec<f32> = langs.iter().map(|(_, i)| row[*i as usize]).collect();
        let lp = log_softmax(&vals);
        let (bi, bp) = lp.iter().enumerate().max_by(|a, b| a.1.partial_cmp(b.1).unwrap_or(std::cmp::Ordering::Equal))?;
        Some((langs[bi].0.clone(), (*bp as f64).exp()))
    }

    /// Préchauffage (première inférence, détection d'une panne du GPU).
    pub fn warmup(&mut self) -> Result<(), String> {
        self.ensure()?;
        self.decode_window(&vec![0f32; SR], "", None, 1).map(|_| ())
    }
}
