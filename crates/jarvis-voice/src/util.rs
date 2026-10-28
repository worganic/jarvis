// Outils communs du moteur vocal : journal (engine.log), JSON, WAV, rééchantillonnage.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::OnceLock;

pub type J = serde_json::Value;

/// Expression régulière compilée une seule fois.
#[macro_export]
macro_rules! re {
    ($pat:expr) => {{
        static RE: std::sync::LazyLock<regex::Regex> = std::sync::LazyLock::new(|| regex::Regex::new($pat).unwrap());
        &*RE
    }};
}

/// Expression régulière avec regards en arrière/avant (fancy-regex), compilée une seule fois.
#[macro_export]
macro_rules! fre {
    ($pat:expr) => {{
        static RE: std::sync::LazyLock<fancy_regex::Regex> = std::sync::LazyLock::new(|| fancy_regex::Regex::new($pat).unwrap());
        &*RE
    }};
}

const LOG_MAX_BYTES: u64 = 2 * 1024 * 1024;
static LOG_COUNT: AtomicU64 = AtomicU64::new(0);
static LOG_PATH: OnceLock<PathBuf> = OnceLock::new();

/// Racine de Jarvis (dossier qui contient ui/ et Cargo.toml).
pub fn root() -> PathBuf {
    if let Ok(r) = std::env::var("JARVIS_ROOT") {
        return PathBuf::from(r);
    }
    std::env::current_exe()
        .ok()
        .and_then(|exe| exe.ancestors().find(|d| d.join("ui").join("index.html").is_file() && (d.join("Cargo.toml").is_file() || d.join("install.json").is_file())).map(|p| p.to_path_buf()))
        .unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join(".."))
}

/// Données de l'utilisateur : JARVIS_DATA_DIR (tests), sinon même règle que jarvis-core/src/paths.rs › data_dir
/// (%LOCALAPPDATA%\Jarvis\donnees une fois prêt, sinon l'ancien <racine>/data tant qu'il n'a pas été recopié).
pub fn data_dir() -> PathBuf {
    if let Ok(d) = std::env::var("JARVIS_DATA_DIR") {
        return PathBuf::from(d);
    }
    let legacy = root().join("data");
    let Some(new) = std::env::var("LOCALAPPDATA").ok().filter(|s| !s.trim().is_empty()).map(|d| PathBuf::from(d).join("Jarvis").join("donnees")) else {
        return legacy;
    };
    if !new.join("VERSION.json").is_file() && legacy.join("config.json").is_file() {
        return legacy;
    }
    new
}

pub fn models_dir() -> PathBuf {
    std::env::var("JARVIS_MODELS_DIR").map(PathBuf::from).unwrap_or_else(|_| data_dir().join("models"))
}

fn log_path() -> &'static PathBuf {
    LOG_PATH.get_or_init(|| std::env::var("JARVIS_ENGINE_LOG").map(PathBuf::from).unwrap_or_else(|_| data_dir().join("logs").join("engine.log")))
}

/// Journal du moteur : sortie standard (relayée par le noyau) et data/logs/engine.log.
pub fn log(message: &str) {
    let mut out = std::io::stdout();
    let _ = writeln!(out, "[moteur] {message}");
    let _ = out.flush();
    let path = log_path();
    let n = LOG_COUNT.fetch_add(1, Ordering::Relaxed);
    if n % 200 == 0 && std::fs::metadata(path).map(|m| m.len() > LOG_MAX_BYTES).unwrap_or(false) {
        let _ = std::fs::rename(path, format!("{}.1", path.display()));
    }
    if let Some(d) = path.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(path) {
        let _ = writeln!(f, "{} {message}", chrono::Local::now().format("%H:%M:%S"));
    }
}

/// JSON indenté d'un espace (json.dump(indent=1) de la v3).
pub fn json_indent1(v: &J) -> String {
    let mut buf = Vec::new();
    let fmt = serde_json::ser::PrettyFormatter::with_indent(b" ");
    let mut ser = serde_json::Serializer::with_formatter(&mut buf, fmt);
    let _ = serde::Serialize::serialize(v, &mut ser);
    String::from_utf8(buf).unwrap_or_default()
}

/// Lecture d'un WAV en mono float32 : (échantillons, fréquence).
pub fn read_wav(path: &Path) -> Option<(Vec<f32>, usize)> {
    let mut r = hound::WavReader::open(path).ok()?;
    let spec = r.spec();
    let ch = spec.channels.max(1) as usize;
    let samples: Vec<f32> = match spec.sample_format {
        hound::SampleFormat::Float => r.samples::<f32>().filter_map(Result::ok).collect(),
        hound::SampleFormat::Int => {
            let scale = (1i64 << (spec.bits_per_sample - 1)) as f32;
            r.samples::<i32>().filter_map(Result::ok).map(|s| s as f32 / scale).collect()
        }
    };
    let mono = if ch == 1 { samples } else { samples.chunks(ch).map(|c| c.iter().sum::<f32>() / ch as f32).collect() };
    Some((mono, spec.sample_rate as usize))
}

/// Écriture d'un WAV 16 bits mono.
pub fn write_wav(path: &Path, audio: &[f32], sr: usize) -> std::io::Result<()> {
    if let Some(d) = path.parent() {
        std::fs::create_dir_all(d)?;
    }
    let spec = hound::WavSpec { channels: 1, sample_rate: sr as u32, bits_per_sample: 16, sample_format: hound::SampleFormat::Int };
    let mut w = hound::WavWriter::create(path, spec).map_err(std::io::Error::other)?;
    for s in audio {
        w.write_sample((s.clamp(-1.0, 1.0) * 32767.0) as i16).map_err(std::io::Error::other)?;
    }
    w.finalize().map_err(std::io::Error::other)
}

/// Rééchantillonnage par interpolation linéaire (np.interp de la v3).
pub fn interp_resample(audio: &[f32], from: usize, to: usize) -> Vec<f32> {
    if from == to || audio.is_empty() {
        return audio.to_vec();
    }
    let n = ((audio.len() as f64) * to as f64 / from as f64).round() as usize;
    if n <= 1 {
        return vec![audio[0]; n];
    }
    let step = (audio.len() - 1) as f64 / (n - 1) as f64;
    (0..n)
        .map(|i| {
            let x = i as f64 * step;
            let i0 = x.floor() as usize;
            let i1 = (i0 + 1).min(audio.len() - 1);
            let t = (x - i0 as f64) as f32;
            audio[i0] * (1.0 - t) + audio[i1] * t
        })
        .collect()
}

pub fn rms(a: &[f32]) -> f32 {
    if a.is_empty() {
        return 0.0;
    }
    (a.iter().map(|x| x * x).sum::<f32>() / a.len() as f32).sqrt()
}

pub fn round3(x: f64) -> f64 {
    (x * 1000.0).round() / 1000.0
}

/// Minuscules sans accents.
pub fn fold(text: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    text.nfd().filter(|c| !unicode_normalization::char::is_combining_mark(*c)).collect::<String>().to_lowercase()
}

/// Environnement onnxruntime partagé par toutes les sessions du moteur.
pub fn ort_env() -> &'static ort::environment::Environment {
    static ENV: OnceLock<ort::environment::Environment> = OnceLock::new();
    ENV.get_or_init(|| ort::init().with_name("jarvis-voice").build().expect("onnxruntime indisponible"))
}
