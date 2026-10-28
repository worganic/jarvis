// Bancs d'essai de la reconnaissance vocale (tools/bench-stt.py et tools/bench-speaker.py de la v3),
// lancés depuis l'administration :
//
//   jarvis-voice bench stt --data DOSSIER [--models turbo]
//     Rejoue le corpus enregistré et mesure WER, commandes reconnues (routeur de Jarvis), négatifs
//     retenus (dont « Jarvis ») et latence → data/voice-corpus/report.json
//
//   jarvis-voice bench speaker --data DOSSIER [--voix-synthese]
//     Score de chaque segment contre les empreintes, seuil recommandé par tranche de durée →
//     data/voice-corpus/speaker-report.json

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

use serde_json::{json, Value};
use unicode_normalization::UnicodeNormalization;

use crate::filters;
use crate::util::J;

const SR: usize = 16000;

struct Item {
    id: String,
    text: String,
    config: Option<String>,
    kind: String,
    audio: Vec<f32>,
}

fn arg(args: &[String], name: &str) -> Option<String> {
    args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned()
}

fn load_corpus(data_dir: &Path) -> Vec<Item> {
    let base = data_dir.join("voice-corpus");
    let Some(manifest) = std::fs::read_to_string(base.join("manifest.json")).ok().and_then(|s| serde_json::from_str::<J>(&s).ok()) else { return vec![] };
    let mut out = vec![];
    for it in manifest["items"].as_array().cloned().unwrap_or_default() {
        let Some(file) = it["file"].as_str() else { continue };
        let Some((audio, sr)) = crate::util::read_wav(&base.join(file)) else { continue };
        out.push(Item {
            id: it["id"].as_str().unwrap_or("").into(),
            text: it["text"].as_str().unwrap_or("").into(),
            config: it["config"].as_str().map(String::from),
            kind: it["kind"].as_str().unwrap_or("phrase").into(),
            audio: crate::util::interp_resample(&audio, sr, SR),
        });
    }
    out
}

fn norm(text: &str) -> Vec<String> {
    let t: String = text.nfkc().collect::<String>().to_lowercase().replace('’', "'");
    let t = crate::re!(r"[^\w' -]+").replace_all(&t, " ").replace(['\'', '-'], " ");
    t.split_whitespace().map(String::from).collect()
}

fn wer(r: &str, h: &str) -> f64 {
    let (r, h) = (norm(r), norm(h));
    if r.is_empty() {
        return if h.is_empty() { 0.0 } else { 1.0 };
    }
    let mut d: Vec<usize> = (0..=h.len()).collect();
    for i in 1..=r.len() {
        let mut prev = d[0];
        d[0] = i;
        for j in 1..=h.len() {
            let cur = (d[j] + 1).min(d[j - 1] + 1).min(prev + usize::from(r[i - 1] != h[j - 1]));
            prev = d[j];
            d[j] = cur;
        }
    }
    d[h.len()] as f64 / r.len() as f64
}

fn core_exe() -> PathBuf {
    std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join("jarvis-core.exe"))).unwrap_or_else(|| PathBuf::from("jarvis-core.exe"))
}

/// Intention locale de chaque texte, via le vrai routeur de Jarvis.
fn route(texts: &[String]) -> Vec<Option<String>> {
    if texts.is_empty() {
        return vec![];
    }
    let child = Command::new(core_exe()).args(["bench-helper", "route"]).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn();
    let Ok(mut child) = child else { return vec![None; texts.len()] };
    if let Some(mut si) = child.stdin.take() {
        for t in texts {
            let _ = writeln!(si, "{}", json!(t));
        }
    }
    let out = child.wait_with_output().map(|o| o.stdout).unwrap_or_default();
    serde_json::from_slice::<Vec<Value>>(&out).map(|v| v.into_iter().map(|x| x.as_str().map(String::from)).collect()).unwrap_or_else(|_| vec![None; texts.len()])
}

/// Vocabulaire réellement envoyé par Jarvis à la voix.
fn user_vocabulary() -> (Vec<String>, J) {
    let out = Command::new(core_exe()).args(["bench-helper", "vocab"]).stderr(Stdio::null()).output().map(|o| o.stdout).unwrap_or_default();
    let v: J = serde_json::from_slice(&out).unwrap_or_default();
    let terms = v["terms"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()).unwrap_or_default();
    (terms, v["corrections"].clone())
}

fn normalize_level(audio: &[f32], max_gain: f32) -> Vec<f32> {
    let rms = crate::util::rms(audio);
    if rms > 0.004 {
        let gain = (0.06 / rms).min(max_gain);
        if gain > 1.0 {
            return audio.iter().map(|x| (x * gain).clamp(-1.0, 1.0)).collect();
        }
    }
    audio.to_vec()
}

fn transcribe(w: &mut crate::whisper::Whisper, audio: &[f32], variant: &str, beam: usize, terms: &[String], corrections: &J) -> String {
    let compiled = filters::compile_corrections(corrections);
    if variant == "v5" {
        // Transcription d'avant le chantier v6 (amorce en liste, sans correctifs ni filtres).
        let audio = normalize_level(audio, 30.0);
        let list: Vec<String> = terms.iter().cloned().chain(filters::BASE_VOCABULARY.iter().map(|s| s.to_string())).collect();
        let legacy: String = format!("Conversation en français avec Jarvis, l'assistant vocal. Vocabulaire : {}", list.join(", ")).chars().take(520).collect::<String>() + ".";
        let d = w.transcribe(&audio, &legacy, None, 2);
        let mut text = d.text;
        for (re, right) in &compiled {
            text = re.replace_all(&text, fancy_regex::NoExpand(right)).to_string();
        }
        return text;
    }
    let audio = normalize_level(audio, 4.0);
    let d = w.transcribe(&audio, &filters::build_initial_prompt(terms), filters::build_hotwords(terms).as_deref(), beam);
    let (kept, _) = filters::assess(&filters::clean_and_autocorrect(&d.text, &compiled), &d, audio.len(), true, &[], None, None);
    let mut kept = kept.unwrap_or_default();
    if !kept.is_empty() && filters::english_hint(&kept) {
        if let Some((lang, p)) = w.detect_language(&audio) {
            if lang == "en" && p >= 0.5 {
                kept.clear();
            }
        }
    }
    kept
}

fn run_variant(w: &mut crate::whisper::Whisper, corpus: &[Item], variant: &str, beam: usize, label: &str, terms: &[String], corrections: &J) -> J {
    let phrases: Vec<&Item> = corpus.iter().filter(|c| c.kind == "phrase").collect();
    let negatives: Vec<&Item> = corpus.iter().filter(|c| c.kind == "negatif").collect();
    let mut hyps = vec![];
    let mut times = vec![];
    for c in &phrases {
        let t0 = Instant::now();
        hyps.push(transcribe(w, &c.audio, variant, beam, terms, corrections));
        times.push(t0.elapsed().as_secs_f64());
    }
    let neg_texts: Vec<String> = negatives.iter().map(|c| transcribe(w, &c.audio, variant, beam, terms, corrections)).collect();
    let refs: Vec<String> = phrases.iter().map(|c| c.text.clone()).collect();
    let (rk, hk) = (route(&refs), route(&hyps));
    let commands: Vec<(String, Option<String>)> = rk.iter().zip(hk.iter()).filter_map(|(r, h)| r.clone().map(|r| (r, h.clone()))).collect();
    let wers: Vec<f64> = refs.iter().zip(&hyps).map(|(r, h)| wer(r, h)).collect();
    let kept: Vec<&String> = neg_texts.iter().filter(|t| !t.is_empty()).collect();
    let wake = kept.iter().filter(|t| crate::re!(r"(?i)\bjarvis\b").is_match(t)).count();
    let mean = |v: &[f64]| if v.is_empty() { None } else { Some(v.iter().sum::<f64>() / v.len() as f64) };
    let cmd_ok = commands.iter().filter(|(r, h)| Some(r) == h.as_ref()).count();
    let result = json!({
        "label": label,
        "phrases": phrases.len(),
        "wer": mean(&wers).map(|m| (m * 1000.0).round() / 10.0),
        "commandes": if commands.is_empty() { Value::Null } else { json!((1000.0 * cmd_ok as f64 / commands.len() as f64).round() / 10.0) },
        "commandes_total": commands.len(),
        "negatifs": negatives.len(),
        "negatifs_retenus": kept.len(),
        "negatifs_jarvis": wake,
        "latence_ms": mean(&times).map(|m| (m * 1000.0).round() as i64),
        "erreurs": phrases.iter().zip(refs.iter().zip(&hyps)).zip(&wers).filter(|(_, w)| **w > 0.0).take(40)
            .map(|((c, (r, h)), w)| json!({ "id": c.id, "config": c.config, "attendu": r, "entendu": h, "wer": (w * 100.0).round() as i64 })).collect::<Vec<_>>()
    });
    println!(
        "  {label:34} WER {}% · commandes {}% ({}) · négatifs retenus {}/{} dont « Jarvis » {wake} · {} ms",
        result["wer"], result["commandes"], commands.len(), kept.len(), negatives.len(), result["latence_ms"]
    );
    result
}

fn bench_stt(args: &[String]) -> i32 {
    let data = arg(args, "--data").map(PathBuf::from).unwrap_or_else(crate::util::data_dir);
    let (terms, corrections) = user_vocabulary();
    println!("vocabulaire de l'utilisateur : {} termes, {} correction(s)", terms.len(), corrections.as_array().map(|a| a.len()).unwrap_or(0));
    let corpus = load_corpus(&data);
    if corpus.is_empty() {
        println!("corpus vide : enregistre d'abord des phrases (administration → Voix → Corpus).");
        return 1;
    }
    println!(
        "corpus : {} phrases, {} segments négatifs",
        corpus.iter().filter(|c| c.kind == "phrase").count(),
        corpus.iter().filter(|c| c.kind == "negatif").count()
    );
    let models = arg(args, "--models").unwrap_or_else(|| "turbo".into());
    let mut results = vec![];
    for name in models.split(',').map(str::trim).filter(|m| !m.is_empty()) {
        if name != "turbo" {
            println!("{name} : modèle absent, ignoré");
            continue;
        }
        println!("modèle {name} : chargement…");
        let mut w = match crate::whisper::Whisper::load(&crate::util::models_dir().join("whisper-large-v3-turbo-onnx")) {
            Ok(w) => w,
            Err(e) => {
                println!("  chargement impossible : {e}");
                continue;
            }
        };
        let _ = w.warmup();
        for (variant, beam, label) in [("v5", 2usize, "turbo · réglages d'avant (v5)"), ("v6", 2, "turbo · réglages v6 (beam 2)"), ("v6", 5, "turbo · réglages v6 (beam 5)")] {
            results.push(run_variant(&mut w, &corpus, variant, beam, label, &terms, &corrections));
        }
    }
    let mut ranked: Vec<&J> = results.iter().filter(|r| !r["commandes"].is_null()).collect();
    ranked.sort_by(|a, b| {
        let k = |r: &J| (-(r["commandes"].as_f64().unwrap_or(0.0)), r["wer"].as_f64().unwrap_or(1e9), r["negatifs_jarvis"].as_i64().unwrap_or(0), r["latence_ms"].as_i64().unwrap_or(0));
        k(a).partial_cmp(&k(b)).unwrap_or(std::cmp::Ordering::Equal)
    });
    let best = ranked.first().map(|r| r["label"].clone()).unwrap_or(Value::Null);
    let report = json!({ "date": chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string(), "synthetique": false, "resultats": results, "meilleur": best });
    let out = data.join("voice-corpus").join("report.json");
    let _ = std::fs::write(&out, crate::util::json_indent1(&report));
    println!("meilleur réglage : {} · rapport : {}", best, out.display());
    0
}

// --- Calibrage du filtre de locuteur -----------------------------------------------------------

const MARGIN: f64 = 0.03;
const KEEP: f64 = 0.95;

fn recommend(pos: &[f64], neg: &[f64], current: f64) -> J {
    let mut pos = pos.to_vec();
    let mut neg = neg.to_vec();
    pos.sort_by(|a, b| a.partial_cmp(b).unwrap());
    neg.sort_by(|a, b| a.partial_cmp(b).unwrap());
    if pos.is_empty() {
        return json!({ "seuil": current, "motif": "aucune phrase de l'utilisateur dans cette tranche" });
    }
    let floor = pos[((1.0 - KEEP) * (pos.len() - 1) as f64) as usize];
    if neg.is_empty() {
        let thr = current.min(floor);
        return json!({ "seuil": (thr * 1000.0).round() / 1000.0, "motif": "pas de négatif : seuil limité à garder 95 % de tes phrases" });
    }
    let strict = neg[neg.len() - 1] + MARGIN;
    if strict <= floor {
        return json!({ "seuil": (strict * 1000.0).round() / 1000.0, "motif": "tous les négatifs rejetés, ≥ 95 % de tes phrases gardées" });
    }
    // Conflit : point d'égalité des erreurs (faux rejets ≈ fausses acceptations).
    let (mut best, mut best_gap) = (strict, 1e9);
    let mut thr = 0.40;
    while thr < 0.90 {
        let fr = pos.iter().filter(|p| **p < thr).count() as f64 / pos.len() as f64;
        let fa = neg.iter().filter(|n| **n >= thr).count() as f64 / neg.len() as f64;
        if (fr - fa).abs() < best_gap {
            best = thr;
            best_gap = (fr - fa).abs();
        }
        thr += 0.005;
    }
    json!({ "seuil": (best * 1000.0).round() / 1000.0, "motif": "chevauchement : compromis au point d'égalité des erreurs", "conflit": true })
}

const SYNTH_VOICES: &[&str] = &[
    "fr-FR-HenriNeural", "fr-FR-DeniseNeural", "fr-FR-EloiseNeural", "fr-FR-RemyMultilingualNeural",
    "fr-FR-VivienneMultilingualNeural", "fr-CA-AntoineNeural", "fr-CA-SylvieNeural", "fr-CA-JeanNeural",
    "fr-BE-CharlineNeural", "fr-BE-GerardNeural", "fr-CH-ArianeNeural", "fr-CH-FabriceNeural",
];
const SYNTH_LINES: &[&str] = &[
    "Jarvis, quelle heure est-il ?", "Mets de la musique.", "Ouvre mes mails.", "Merci.",
    "Bonsoir à tous et bienvenue dans ce journal.", "On se retrouve après la publicité.",
    "Il fera beau demain sur une grande partie du pays.", "Regardez bien ce détail, c'est important.",
];

/// Voix de synthèse (edge-tts) comme « autres voix » quand aucune série négative n'existe.
fn synthetic_negatives() -> Vec<Item> {
    let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().expect("runtime");
    let cache = std::env::temp_dir().join("jarvis-voix-synthese");
    let _ = std::fs::create_dir_all(&cache);
    let mut out = vec![];
    for v in SYNTH_VOICES {
        for (i, line) in SYNTH_LINES.iter().enumerate() {
            let path = cache.join(format!("{v}-{i}.wav"));
            let audio = match crate::util::read_wav(&path) {
                Some((a, sr)) => crate::util::interp_resample(&a, sr, SR),
                None => match rt.block_on(crate::edge::synth(line, v, "+0%", "+0Hz")) {
                    Ok((a, sr)) => {
                        let _ = crate::util::write_wav(&path, &a, sr as usize);
                        crate::util::interp_resample(&a, sr as usize, SR)
                    }
                    Err(_) => continue,
                },
            };
            out.push(Item { id: String::new(), text: String::new(), config: None, kind: "negatif".into(), audio });
        }
    }
    out
}

fn bench_speaker(args: &[String]) -> i32 {
    let data = arg(args, "--data").map(PathBuf::from).unwrap_or_else(crate::util::data_dir);
    let synth = args.iter().any(|a| a == "--voix-synthese");
    let mut corpus = load_corpus(&data);
    if !corpus.iter().any(|c| c.kind == "phrase") {
        println!("Corpus vide : lis d'abord des phrases (bouton « Lire » de chaque phrase, ci-dessous).");
        return 2;
    }
    if synth {
        let s = synthetic_negatives();
        println!("{} segments de voix de synthèse ajoutés comme « autres voix » ({} voix)", s.len(), SYNTH_VOICES.len());
        corpus.extend(s);
    }
    if !corpus.iter().any(|c| c.kind == "negatif") {
        println!("Pas encore de série négative : les seuils seront calculés sur tes phrases seules. Lance aussi « Série négative » avec une vidéo pour un calibrage complet.");
    }
    let mut v = crate::speaker::SpeakerVerifier::new(
        crate::util::models_dir().join("speaker").join("wespeaker_en_voxceleb_CAM++.onnx"),
        data.join("voiceprint.json"),
        data.join("voice-corpus"),
    );
    if !v.available || v.speakers.is_empty() {
        println!("aucune empreinte vocale enregistrée : enrôle d'abord ta voix (administration → Voix).");
        return 1;
    }
    let mut buckets: Vec<(&str, Vec<f64>, Vec<f64>)> = vec![("court", vec![], vec![]), ("moyen", vec![], vec![]), ("long", vec![], vec![])];
    // --refaire : simule une empreinte refaite avec l'extracteur actuel (centrale du corpus par
    // configuration audio, la phrase évaluée mise de côté) au lieu d'utiliser l'empreinte enregistrée.
    let refaire = args.iter().any(|a| a == "--refaire");
    let embeddings: Vec<Option<Vec<f32>>> = if refaire { corpus.iter().map(|c| v.compute_embedding(&c.audio, SR)).collect() } else { vec![] };
    let centroid = |skip: usize, cfg: Option<&str>| -> Option<Vec<f32>> {
        let mut acc: Vec<f32> = vec![];
        let mut n = 0;
        for (i, c) in corpus.iter().enumerate() {
            if i == skip || c.kind != "phrase" || (cfg.is_some() && c.config.as_deref() != cfg) {
                continue;
            }
            let Some(e) = &embeddings[i] else { continue };
            if acc.is_empty() {
                acc = vec![0.0; e.len()];
            }
            acc.iter_mut().zip(e).for_each(|(a, b)| *a += b);
            n += 1;
        }
        let norm = acc.iter().map(|x| x * x).sum::<f32>().sqrt();
        (n > 0 && norm > 1e-6).then(|| acc.iter().map(|x| x / norm).collect())
    };
    for (idx, c) in corpus.iter().enumerate() {
        let (emb, score) = if refaire {
            let Some(emb) = embeddings[idx].clone() else { continue };
            let mut best = f64::MIN;
            for cfg in ["casque", "haut-parleurs"] {
                if let Some(ce) = centroid(idx, Some(cfg)) {
                    best = best.max(emb.iter().zip(&ce).map(|(a, b)| (*a as f64) * (*b as f64)).sum());
                }
            }
            if best == f64::MIN {
                continue;
            }
            (emb, best)
        } else {
            let Some(emb) = v.compute_embedding(&c.audio, SR) else { continue };
            let Some((_, score)) = v.rank(&emb).first().copied() else { continue };
            (emb, score)
        };
        let _ = &emb;
        let b = crate::speaker::duration_bucket(c.audio.len() as f64 / SR as f64);
        let s = (score * 1000.0).round() / 1000.0;
        if let Some(bk) = buckets.iter_mut().find(|(n, _, _)| *n == b) {
            if c.kind == "negatif" {
                bk.2.push(s);
            } else {
                bk.1.push(s);
            }
        }
    }
    let threshold = v.threshold;
    let mut tranches = serde_json::Map::new();
    let mut seuils = serde_json::Map::new();
    for (b, pos, neg) in &buckets {
        let mut rec = recommend(pos, neg, threshold);
        let seuil = rec["seuil"].as_f64().unwrap_or(threshold);
        let count = |l: &[f64], f: &dyn Fn(f64) -> bool| l.iter().filter(|x| f(**x)).count();
        let (cur_fr, cur_fa) = (count(pos, &|p| p < threshold), count(neg, &|n| n >= threshold));
        let (new_fr, new_fa) = (count(pos, &|p| p < seuil), count(neg, &|n| n >= seuil));
        let mut sp = pos.clone();
        sp.sort_by(|a, b| a.partial_cmp(b).unwrap());
        let mut sn = neg.clone();
        sn.sort_by(|a, b| a.partial_cmp(b).unwrap());
        rec["phrases"] = json!(pos.len());
        rec["negatifs"] = json!(neg.len());
        rec["actuel"] = json!({ "rejets_a_tort": cur_fr, "acceptes_a_tort": cur_fa });
        rec["recommande"] = json!({ "rejets_a_tort": new_fr, "acceptes_a_tort": new_fa });
        rec["scores_phrases"] = json!(sp);
        rec["scores_negatifs"] = json!(sn);
        println!(
            "{b:6} : {} phrases, {} négatifs · seuil {threshold:.2} → {cur_fr} rejets à tort, {cur_fa} acceptés à tort · recommandé {seuil:.3} → {new_fr} / {new_fa} ({})",
            pos.len(),
            neg.len(),
            rec["motif"].as_str().unwrap_or("")
        );
        if !pos.is_empty() {
            seuils.insert(b.to_string(), json!(seuil));
        }
        tranches.insert(b.to_string(), rec);
    }
    let report = json!({
        "date": chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string(),
        "seuil_actuel": threshold, "tranches": tranches, "voix_synthese": synth,
        "embedder": crate::speaker::EMBEDDER_VERSION, "seuils": seuils
    });
    // La simulation (--refaire) ne remplace pas le rapport de l'empreinte enregistrée.
    let out = data.join("voice-corpus").join(if refaire { "speaker-report-simulation.json" } else { "speaker-report.json" });
    let _ = std::fs::write(&out, crate::util::json_indent1(&report));
    println!("rapport : {}", out.display());
    0
}

pub fn main(args: &[String]) -> i32 {
    match args.first().map(String::as_str) {
        Some("stt") => bench_stt(&args[1..]),
        Some("speaker") => bench_speaker(&args[1..]),
        _ => {
            eprintln!("usage : jarvis-voice bench stt|speaker --data DOSSIER");
            1
        }
    }
}
