// Voix de Jarvis (Speaker de voice/service.py + Voice de voice/engine.py) : une seule file de parole,
// pré-synthèse en parallèle (3 phrases d'avance), cache des phrases courtes, cache disque des phrases
// fréquentes, coupure immédiate (stop), pause quand l'utilisateur parle (rien ne démarre par-dessus lui, et il peut couper la parole),
// volume propre, effets des personnages du cœur, edge-tts et Piper local en secours.

use std::collections::{HashSet, VecDeque};
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::json;

use crate::audio::{Audio, MIC_SR};
use crate::hub::Hub;
use crate::util::{log, J};

const PREFETCH: usize = 3;
const TAIL: f64 = 0.25;
const CACHE_MAX_CHARS: usize = 80;
const CACHE_MAX_ITEMS: usize = 200;
// 10 s : la première phrase passée par le relais Python edge-tts charge le paquet (2 s).
const EDGE_TIMEOUT: Duration = Duration::from_secs(10);
const STALL_SECONDS: f64 = 12.0;

/// Catalogue des voix françaises Edge (clé, id, nom, genre, origine).
pub const VOICES: &[(&str, &str, &str, &str, &str)] = &[
    ("denise", "fr-FR-DeniseNeural", "Denise", "Féminin", "France"),
    ("henri", "fr-FR-HenriNeural", "Henri", "Masculin", "France"),
    ("eloise", "fr-FR-EloiseNeural", "Éloïse", "Féminin", "France"),
    ("sylvie", "fr-CA-SylvieNeural", "Sylvie", "Féminin", "Canada"),
    ("antoine", "fr-CA-AntoineNeural", "Antoine", "Masculin", "Canada"),
    ("charline", "fr-BE-CharlineNeural", "Charline", "Féminin", "Belgique"),
    ("ariane", "fr-CH-ArianeNeural", "Ariane", "Féminin", "Suisse"),
    ("vivienne", "fr-FR-VivienneMultilingualNeural", "Vivienne (multilingue)", "Féminin", "France"),
    ("remy", "fr-FR-RemyMultilingualNeural", "Rémy (multilingue)", "Masculin", "France"),
];

/// (id, nom) d'une voix demandée par clé, id ou nom.
pub fn resolve_voice(query: &str) -> (String, String) {
    if query.is_empty() {
        return (VOICES[0].1.into(), VOICES[0].2.into());
    }
    let q = query.trim().to_lowercase().replace(['é', 'è'], "e");
    for v in VOICES {
        if q == v.0 || q == v.1.to_lowercase() || v.2.to_lowercase().contains(&q) {
            return (v.1.into(), v.2.into());
        }
    }
    (query.into(), query.into())
}

/// Voix de robot : modulation en anneau et filtre en peigne, volume moyen conservé.
fn apply_effect(data: Vec<f32>, sr: usize, effect: &str) -> Vec<f32> {
    let (freq, mix, delay_ms, gain) = match effect {
        "robot" => (70.0f32, 0.35f32, 4.5f32, 0.45f32),
        "robot_deep" => (42.0, 0.3, 6.5, 0.5),
        "robot_light" => (95.0, 0.2, 3.0, 0.3),
        _ => return data,
    };
    if data.is_empty() {
        return data;
    }
    let y: Vec<f32> = data
        .iter()
        .enumerate()
        .map(|(i, x)| x * ((1.0 - mix) + mix * (2.0 * std::f32::consts::PI * freq * i as f32 / sr as f32).sin()))
        .collect();
    let d = ((sr as f32 * delay_ms / 1000.0) as usize).max(1);
    let mut out = y.clone();
    for i in d..out.len() {
        out[i] += gain * y[i - d];
    }
    for i in (2 * d)..out.len() {
        out[i] += gain * gain * y[i - 2 * d];
    }
    let rin = crate::util::rms(&data).max(1e-9);
    let rout = crate::util::rms(&out).max(1e-9);
    out.iter_mut().for_each(|v| *v *= rin / rout);
    let peak = out.iter().fold(0f32, |m, v| m.max(v.abs()));
    if peak > 0.97 {
        out.iter_mut().for_each(|v| *v *= 0.97 / peak);
    }
    out
}

fn precache_phrases() -> HashSet<String> {
    let v: J = serde_json::from_str(include_str!("../../jarvis-core/data/phrases.json")).unwrap_or_default();
    let mut out = HashSet::new();
    if let Some(m) = v.as_object() {
        for (k, list) in m {
            if k.starts_with('_') {
                continue;
            }
            for p in list.as_array().into_iter().flatten() {
                if let Some(s) = p.as_str().map(str::trim).filter(|s| !s.is_empty()) {
                    out.insert(s.to_string());
                }
            }
        }
    }
    out
}

pub struct VoiceSettings {
    pub voice: String,
    pub voice_name: String,
    pub rate: String,
    pub pitch: String,
    pub effect: String,
}

pub struct Voice {
    hub: Hub,
    audio: Arc<Audio>,
    opts: Arc<Mutex<J>>,
    pub settings: Mutex<VoiceSettings>,
    pub speaking: AtomicBool,
    gen: AtomicU64,
    texts: tokio::sync::mpsc::UnboundedSender<(u64, String)>,
    cache: Mutex<VecDeque<(String, Vec<f32>)>>,
    precache: HashSet<String>,
    cache_dir: PathBuf,
    piper: Mutex<Option<sherpa_rs::tts::VitsTts>>,
    pub fallback_active: AtomicBool,
    idle_token: AtomicU64,
    record_path: Option<String>,
    recorded: Mutex<Vec<f32>>,
    /// Segment d'écoute qui a mis la voix en pause (None : pause posée par le noyau).
    hold_segment: Mutex<Option<u64>>,
    /// Runtime tokio : `on_drained` est appelé par le thread audio (hors runtime), où `tokio::spawn` panique.
    rt: tokio::runtime::Handle,
}

impl Voice {
    pub fn start(hub: Hub, audio: Arc<Audio>, opts: Arc<Mutex<J>>) -> Arc<Self> {
        let (voice, voice_name) = resolve_voice(&std::env::var("JARVIS_VOICE").unwrap_or_else(|_| "fr-FR-DeniseNeural".into()));
        let rate = std::env::var("JARVIS_RATE").unwrap_or_else(|_| "+0%".into());
        let (ttx, trx) = tokio::sync::mpsc::unbounded_channel::<(u64, String)>();
        let me = Arc::new(Self {
            hub: hub.clone(),
            audio,
            opts,
            settings: Mutex::new(VoiceSettings { voice, voice_name, rate, pitch: "+0Hz".into(), effect: "none".into() }),
            speaking: AtomicBool::new(false),
            gen: AtomicU64::new(0),
            texts: ttx,
            cache: Mutex::new(VecDeque::new()),
            precache: precache_phrases(),
            cache_dir: crate::util::data_dir().join("voice-cache"),
            piper: Mutex::new(None),
            fallback_active: AtomicBool::new(false),
            idle_token: AtomicU64::new(0),
            record_path: std::env::var("JARVIS_ENGINE_OUTPUT").ok().filter(|s| !s.is_empty()),
            recorded: Mutex::new(vec![]),
            hold_segment: Mutex::new(None),
            rt: tokio::runtime::Handle::current(),
        });
        // Sortie vidée : on laisse au casque le temps de finir la dernière syllabe.
        let w = Arc::downgrade(&me);
        hub.on_drained(Box::new(move || {
            if let Some(v) = w.upgrade() {
                v.on_drained();
            }
        }));
        let (synth_tx, synth_rx) = tokio::sync::mpsc::channel::<(u64, String, tokio::task::JoinHandle<Option<Vec<f32>>>)>(PREFETCH);
        tokio::spawn(Self::synth_loop(me.clone(), trx, synth_tx));
        tokio::spawn(Self::feed_loop(me.clone(), synth_rx));
        tokio::spawn(Self::guards(me.clone()));
        tokio::spawn(Self::edge_probe(me.clone()));
        let w2 = me.clone();
        tokio::spawn(async move { w2.warm_cache().await });
        if me.record_path.is_none() {
            me.audio.ensure_output();
        }
        me
    }

    fn voice_key(&self) -> String {
        let s = self.settings.lock().unwrap();
        // Voix sans hauteur ni effet : même clé que la v3 (le cache disque reste valable).
        if s.pitch == "+0Hz" && s.effect == "none" {
            format!("{}|{}", s.voice, s.rate)
        } else {
            format!("{}|{}|{}|{}", s.voice, s.rate, s.pitch, s.effect)
        }
    }

    fn set_speaking(&self, value: bool) {
        if self.speaking.swap(value, Ordering::SeqCst) != value {
            self.hub.emit_state(json!({ "speaking": value }));
        }
    }

    /// Ajoute un texte à la file (phrases enchaînées sans coupure).
    pub fn say(self: &Arc<Self>, text: &str) {
        self.say_with(text, None);
    }

    /// Comme `say`, avec une voix edge-tts imposée pour ce texte seulement (traduction dite dans sa langue) : la voix
    /// des réglages n'est pas modifiée, et il n'y a pas de repli sur la voix locale française.
    pub fn say_with(self: &Arc<Self>, text: &str, voice: Option<&str>) {
        let mut sentences = crate::filters::split_sentences(text);
        if sentences.is_empty() {
            return;
        }
        if let Some(v) = voice.filter(|v| is_voice_id(v)) {
            sentences = sentences.into_iter().map(|s| format!("{VOICE_MARK}{v}{VOICE_END}{s}")).collect();
        }
        log(&format!("parle : « {} »", sentences.join(" ").chars().take(90).collect::<String>()));
        let gen = self.gen.load(Ordering::SeqCst);
        {
            let mut vo = self.audio.voice.lock().unwrap();
            vo.pending += sentences.len();
            vo.progress_at = Some(Instant::now());
            vo.drained_flag = false;
        }
        for s in sentences {
            let _ = self.texts.send((gen, s));
        }
        self.idle_token.fetch_add(1, Ordering::SeqCst);
        // L'utilisateur parle : la phrase attend, Jarvis n'est pas « en train de parler » tant qu'on ne l'entend pas.
        if !self.held() || self.speaking.load(Ordering::SeqCst) {
            self.set_speaking(true);
        }
    }

    /// Vide la file et coupe la voix immédiatement (appelable de tout fil).
    pub fn stop(&self) {
        self.gen.fetch_add(1, Ordering::SeqCst);
        {
            let mut vo = self.audio.voice.lock().unwrap();
            vo.chunks.clear();
            vo.pos = 0;
            vo.pending = 0;
            vo.gen = self.gen.load(Ordering::SeqCst);
        }
        self.set_speaking(false);
        self.hold(false, None);
    }

    /// Pause pendant que l'utilisateur parle (le noyau décide ensuite : stop ou reprise). `segment` : la prise
    /// de parole qui la pose, pour que seule son issue la lève (`release_for`).
    pub fn hold(&self, on: bool, segment: Option<u64>) {
        let waiting = {
            let mut vo = self.audio.voice.lock().unwrap();
            vo.set_held(on);
            if !on {
                vo.progress_at = Some(Instant::now());
            }
            !vo.chunks.is_empty() || vo.pending > 0
        };
        *self.hold_segment.lock().unwrap() = if on { segment } else { None };
        // Phrases arrivées pendant la pause : on les entend maintenant.
        if !on && waiting {
            self.set_speaking(true);
        }
    }

    /// Lève la pause si elle vient encore de cette prise de parole (une plus récente la garde).
    pub fn release_for(&self, segment: u64) {
        if self.held() && *self.hold_segment.lock().unwrap() == Some(segment) {
            self.hold(false, None);
        }
    }

    pub fn held(&self) -> bool {
        self.audio.voice.lock().unwrap().held
    }

    pub fn set_gain(&self, level: f64) {
        self.audio.voice.lock().unwrap().gain = level.clamp(0.0, 1.0) as f32;
    }

    pub fn set_voice(self: &Arc<Self>, voice: Option<&str>, rate: Option<&str>, pitch: Option<&str>, effect: Option<&str>) -> bool {
        let mut changed = false;
        {
            let mut s = self.settings.lock().unwrap();
            if let Some(p) = pitch {
                let p = if p.trim().is_empty() { "+0Hz".to_string() } else { p.trim().to_string() };
                if crate::re!(r"^[+-]\d{1,3}Hz$").is_match(&p) && p != s.pitch {
                    s.pitch = p;
                    changed = true;
                }
            }
            if let Some(e) = effect {
                let e = if ["none", "robot", "robot_deep", "robot_light"].contains(&e) { e } else { "none" };
                if e != s.effect {
                    s.effect = e.into();
                    changed = true;
                }
            }
            if let Some(v) = voice.filter(|v| !v.is_empty()) {
                let (id, name) = resolve_voice(v);
                if id != s.voice {
                    s.voice = id;
                    s.voice_name = name;
                    changed = true;
                }
            }
            if let Some(r) = rate.filter(|r| !r.trim().is_empty()) {
                let mut r = r.trim().to_string();
                if !r.ends_with('%') {
                    if r.starts_with(['+', '-']) {
                        r.push('%');
                    } else if r.chars().all(|c| c.is_ascii_digit()) {
                        r = format!("+{r}%");
                    }
                }
                if r != s.rate {
                    s.rate = r;
                    changed = true;
                }
            }
            if changed {
                log(&format!("voix mise à jour : {} ({}) · vitesse={} · hauteur={} · effet={}", s.voice_name, s.voice, s.rate, s.pitch, s.effect));
                self.hub.emit_state(json!({ "voice": s.voice, "voice_name": s.voice_name, "rate": s.rate, "pitch": s.pitch, "effect": s.effect }));
            }
        }
        if changed {
            let me = self.clone();
            // Handle explicite : peut être appelé depuis un fil hors runtime (audio), où tokio::spawn panique.
            self.rt.spawn(async move { me.warm_cache().await });
        }
        changed
    }

    fn on_drained(self: &Arc<Self>) {
        if !self.speaking.load(Ordering::SeqCst) {
            return;
        }
        let token = self.idle_token.load(Ordering::SeqCst);
        let me = self.clone();
        self.rt.spawn(async move {
            tokio::time::sleep(Duration::from_secs_f64(TAIL)).await;
            let drained = {
                let vo = me.audio.voice.lock().unwrap();
                vo.chunks.is_empty() && vo.pending == 0
            };
            if drained && me.idle_token.load(Ordering::SeqCst) == token {
                me.set_speaking(false);
            }
        });
    }

    // --- Synthèse ---------------------------------------------------------------------------------

    async fn synth_loop(me: Arc<Self>, mut texts: tokio::sync::mpsc::UnboundedReceiver<(u64, String)>, ready: tokio::sync::mpsc::Sender<(u64, String, tokio::task::JoinHandle<Option<Vec<f32>>>)>) {
        while let Some((gen, sentence)) = texts.recv().await {
            if gen != me.gen.load(Ordering::SeqCst) {
                continue;
            }
            let m2 = me.clone();
            let s2 = sentence.clone();
            let task = tokio::spawn(async move { m2.audio_for(&s2).await });
            // File bornée : PREFETCH synthèses en vol.
            if ready.send((gen, sentence, task)).await.is_err() {
                break;
            }
        }
    }

    async fn feed_loop(me: Arc<Self>, mut ready: tokio::sync::mpsc::Receiver<(u64, String, tokio::task::JoinHandle<Option<Vec<f32>>>)>) {
        while let Some((gen, sentence, task)) = ready.recv().await {
            let data = match task.await {
                Ok(d) => d,
                Err(e) => {
                    log(&format!("TTS échouée pour « {} » : {e}", sentence.chars().take(30).collect::<String>()));
                    None
                }
            };
            let stale = {
                let mut vo = me.audio.voice.lock().unwrap();
                let stale = gen != me.gen.load(Ordering::SeqCst);
                if !stale {
                    vo.pending = vo.pending.saturating_sub(1);
                    if let Some(d) = data.filter(|d| !d.is_empty()) {
                        vo.chunks.push_back(d);
                        vo.drained_flag = false;
                    }
                }
                stale
            };
            if stale {
                continue;
            }
            if let Some(path) = me.record_path.clone() {
                // Test : pas de haut-parleur, la file est vidée dans un fichier.
                me.drain_to_file(&path);
                continue;
            }
            if !me.audio.ensure_output() {
                me.stop();
            } else {
                let drained = {
                    let vo = me.audio.voice.lock().unwrap();
                    vo.chunks.is_empty() && vo.pending == 0
                };
                if drained {
                    // Dernière phrase en échec : la sortie ne verra plus de nouvel audio.
                    me.on_drained();
                }
            }
        }
    }

    fn drain_to_file(self: &Arc<Self>, path: &str) {
        let pending = {
            let mut vo = self.audio.voice.lock().unwrap();
            let mut rec = self.recorded.lock().unwrap();
            while let Some(c) = vo.chunks.pop_front() {
                rec.extend(c);
            }
            vo.pos = 0;
            vo.progress_at = Some(Instant::now());
            vo.pending
        };
        if pending > 0 {
            return;
        }
        if path != "null" {
            let rec = self.recorded.lock().unwrap();
            if !rec.is_empty() {
                let _ = crate::util::write_wav(std::path::Path::new(path), &rec, MIC_SR);
            }
        }
        self.on_drained();
    }

    /// Garde-fous : parole bloquée (aucun son depuis 12 s) et sortie audio perdue.
    async fn guards(me: Arc<Self>) {
        loop {
            tokio::time::sleep(Duration::from_secs(1)).await;
            if !me.speaking.load(Ordering::SeqCst) {
                continue;
            }
            let stalled = me.audio.voice.lock().unwrap().progress_at.map(|t| t.elapsed().as_secs_f64() > STALL_SECONDS).unwrap_or(false);
            if stalled {
                let (n, p) = {
                    let vo = me.audio.voice.lock().unwrap();
                    (vo.chunks.len(), vo.pending)
                };
                log(&format!("parole bloquée depuis {STALL_SECONDS:.0} s ({n} bloc(s) prêts, {p} en synthèse) : file vidée, écoute rétablie"));
                me.stop();
                continue;
            }
            if me.record_path.is_none() && !me.audio.ensure_output() {
                me.stop();
            }
        }
    }

    fn cache_get(&self, key: &str) -> Option<Vec<f32>> {
        let mut c = self.cache.lock().unwrap();
        let i = c.iter().position(|(k, _)| k == key)?;
        let item = c.remove(i)?;
        let data = item.1.clone();
        c.push_back(item);
        Some(data)
    }

    fn remember(&self, key: String, data: Vec<f32>) {
        let mut c = self.cache.lock().unwrap();
        c.retain(|(k, _)| *k != key);
        c.push_back((key, data));
        while c.len() > CACHE_MAX_ITEMS {
            c.pop_front();
        }
    }

    fn disk_path(&self, key: &str) -> PathBuf {
        let digest = sha1_smol::Sha1::from(key.as_bytes()).digest().to_string();
        self.cache_dir.join(format!("{}.wav", &digest[..20]))
    }

    async fn audio_for(self: &Arc<Self>, sentence: &str) -> Option<Vec<f32>> {
        let (forced, sentence) = split_forced_voice(sentence);
        let key = match forced {
            Some(v) => format!("{v}|+0%|{sentence}"),
            None => format!("{}|{sentence}", self.voice_key()),
        };
        let effect = if forced.is_some() { "none".to_string() } else { self.settings.lock().unwrap().effect.clone() };
        if let Some(c) = self.cache_get(&key) {
            return Some(c);
        }
        let disk = (forced.is_none() && self.precache.contains(sentence)).then(|| self.disk_path(&key));
        if let Some(d) = disk.as_ref().filter(|d| d.is_file()) {
            if let Some((data, sr)) = crate::util::read_wav(d) {
                let data = crate::util::interp_resample(&data, sr, MIC_SR);
                self.remember(key, data.clone());
                return Some(data);
            }
        }
        let (data, sr) = match self.synth(sentence, forced).await {
            Ok(r) => r,
            Err(e) => {
                log(&format!("TTS échouée pour « {} » : {e}", sentence.chars().take(30).collect::<String>()));
                return None;
            }
        };
        let data = apply_effect(crate::util::interp_resample(&data, sr as usize, MIC_SR), MIC_SR, &effect);
        if let Some(d) = disk {
            if let Err(e) = crate::util::write_wav(&d, &data, MIC_SR) {
                log(&format!("cache audio non écrit : {e}"));
            }
        }
        if sentence.chars().count() <= CACHE_MAX_CHARS {
            self.remember(key, data.clone());
        }
        Some(data)
    }

    async fn synth(self: &Arc<Self>, sentence: &str, forced: Option<&str>) -> Result<(Vec<f32>, u32), String> {
        let (voice, rate, pitch) = match forced {
            Some(v) => (v.to_string(), "+0%".to_string(), "+0Hz".to_string()),
            None => {
                let s = self.settings.lock().unwrap();
                (s.voice.clone(), s.rate.clone(), s.pitch.clone())
            }
        };
        let edge = async {
            if std::env::var("JARVIS_EDGE_DISABLED").is_ok() {
                return Err("edge-tts désactivé (test)".to_string());
            }
            match tokio::time::timeout(EDGE_TIMEOUT, crate::edge::synth(sentence, &voice, &rate, &pitch)).await {
                Ok(r) => r,
                Err(_) => Err("délai dépassé".into()),
            }
        };
        match edge.await {
            Ok(r) => Ok(r),
            Err(e) => {
                // Voix imposée (autre langue) : la voix locale est française, mieux vaut se taire que lire de travers.
                if forced.is_some() || self.opts.lock().unwrap()["ttsFallback"] == J::Bool(false) {
                    return Err(e);
                }
                if !self.fallback_active.swap(true, Ordering::SeqCst) {
                    log(&format!("edge-tts indisponible ({}) : voix locale Piper", e.chars().take(80).collect::<String>()));
                    self.hub.emit_state(json!({ "tts": "piper" }));
                }
                let me = self.clone();
                let s = sentence.to_string();
                tokio::task::spawn_blocking(move || me.piper_synth(&s)).await.map_err(|e| e.to_string())?
            }
        }
    }

    fn piper_synth(&self, sentence: &str) -> Result<(Vec<f32>, u32), String> {
        let mut p = self.piper.lock().unwrap();
        if p.is_none() {
            let d = crate::util::models_dir().join("tts").join("vits-piper-fr_FR-siwis-medium");
            let s = |x: PathBuf| x.to_string_lossy().to_string();
            let cfg = sherpa_rs::tts::VitsTtsConfig {
                model: s(d.join("fr_FR-siwis-medium.onnx")),
                tokens: s(d.join("tokens.txt")),
                data_dir: s(d.join("espeak-ng-data")),
                length_scale: 1.0,
                noise_scale: 0.667,
                noise_scale_w: 0.8,
                silence_scale: 0.2,
                onnx_config: sherpa_rs::OnnxConfig { provider: "cpu".into(), debug: false, num_threads: 2 },
                tts_config: sherpa_rs::tts::CommonTtsConfig { max_num_sentences: 1, ..Default::default() },
                ..Default::default()
            };
            *p = Some(sherpa_rs::tts::VitsTts::new(cfg));
            log("voix locale Piper chargée (fr_FR siwis)");
        }
        let audio = p.as_mut().unwrap().create(sentence, 0, 1.0).map_err(|e| e.to_string())?;
        Ok((audio.samples, audio.sample_rate))
    }

    /// Toutes les 60 s en mode secours : edge-tts est-il revenu ?
    async fn edge_probe(me: Arc<Self>) {
        loop {
            tokio::time::sleep(Duration::from_secs(60)).await;
            if !me.fallback_active.load(Ordering::SeqCst) {
                continue;
            }
            let (voice, rate, pitch) = {
                let s = me.settings.lock().unwrap();
                (s.voice.clone(), s.rate.clone(), s.pitch.clone())
            };
            if let Ok(Ok(_)) = tokio::time::timeout(EDGE_TIMEOUT, crate::edge::synth("test", &voice, &rate, &pitch)).await {
                me.fallback_active.store(false, Ordering::SeqCst);
                me.hub.emit_state(json!({ "tts": "edge" }));
                log("edge-tts de nouveau disponible");
            }
        }
    }

    /// Prépare en arrière-plan les phrases fréquentes (accusés de réception, confirmations).
    async fn warm_cache(self: &Arc<Self>) {
        let key = self.voice_key();
        let mut list: Vec<String> = self.precache.iter().cloned().collect();
        list.sort();
        for sentence in list {
            if key != self.voice_key() {
                return; // la voix a changé : une nouvelle préparation est lancée
            }
            if self.fallback_active.load(Ordering::SeqCst) && !self.disk_path(&format!("{key}|{sentence}")).is_file() {
                return; // pas de cache disque en voix de secours
            }
            if self.audio_for(&sentence).await.is_none() {
                return;
            }
        }
    }
}

/// Marqueurs d'une phrase à dire avec une voix imposée : « \u{1}voix\u{2}texte ».
const VOICE_MARK: char = '\u{1}';
const VOICE_END: char = '\u{2}';

/// Identifiant de voix edge-tts (ex. en-GB-SoniaNeural) : jamais autre chose que lettres et tirets.
pub fn is_voice_id(v: &str) -> bool {
    crate::re!(r"^[a-z]{2,3}-[A-Z]{2,4}-[A-Za-z]{3,30}Neural$").is_match(v)
}

fn split_forced_voice(sentence: &str) -> (Option<&str>, &str) {
    if let Some(rest) = sentence.strip_prefix(VOICE_MARK) {
        if let Some((voice, text)) = rest.split_once(VOICE_END) {
            if is_voice_id(voice) {
                return (Some(voice), text);
            }
        }
    }
    (None, sentence)
}

#[cfg(test)]
mod forced_voice_tests {
    use super::*;

    #[test]
    fn voix_imposee_reconnue_et_retiree() {
        assert_eq!(split_forced_voice("\u{1}en-GB-SoniaNeural\u{2}Hello there"), (Some("en-GB-SoniaNeural"), "Hello there"));
        assert_eq!(split_forced_voice("Bonjour"), (None, "Bonjour"));
        // Une voix qui n'a pas la forme d'un identifiant edge-tts n'est jamais utilisée.
        assert_eq!(split_forced_voice("\u{1}../../x\u{2}Hello"), (None, "\u{1}../../x\u{2}Hello"));
        assert!(is_voice_id("fr-FR-DeniseNeural"));
        assert!(!is_voice_id("Denise"));
    }
}
