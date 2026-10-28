// Écoute (Listener de voice/engine.py) : VAD Silero, mot d'éveil acoustique, garde « Jarvis »,
// fin de phrase adaptative, interruption par la parole (Jarvis en pause), transcriptions partielles et
// finales, empreinte vocale, « Jarvis » seul en attente de la suite.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, RecvTimeoutError, SyncSender};
use std::sync::Condvar;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::audio::{Audio, AudioCmd, BLOCK, SR};
use crate::corpus::CorpusRecorder;
use crate::filters;
use crate::hub::Hub;
use crate::speaker::SpeakerVerifier;
use crate::tts::Voice;
use crate::util::{log, J};
use crate::whisper::Whisper;

const PARTIAL_PERIOD: f64 = 0.8;
const PARTIAL_WINDOW: usize = 8 * SR;
const SPEAKER_WINDOW: usize = 10 * SR;
const WAKE_HOLD: f64 = 1.6;
/// Parole qui ne s'adresse pas à Jarvis : ses annonces n'attendent pas plus longtemps.
const HOLD_MAX: f64 = 10.0;
const FULL_CHECK: f64 = 4.0;

fn clock() -> f64 {
    static START: std::sync::OnceLock<Instant> = std::sync::OnceLock::new();
    START.get_or_init(Instant::now).elapsed().as_secs_f64()
}

enum Job {
    Prefix { audio: Vec<f32>, segment: u64, raw: bool },
    Partial { audio: Vec<f32>, raw: bool, segment: u64, at: f64, spk: Vec<f32> },
    Final { audio: Vec<f32>, speaker: Option<J>, wake: bool, segment: u64, raw: bool },
}

pub struct Shared {
    pub muted: AtomicBool,
    pub mode: Mutex<String>,
    pub gate_until: Mutex<f64>,
    wake_at: Mutex<f64>,
    pub segment: AtomicU64,
    prefix: Mutex<HashMap<u64, bool>>,
    prompt: Mutex<String>,
    hotwords: Mutex<Option<String>>,
    corrections: Mutex<Vec<(fancy_regex::Regex, String)>>,
    recent_said: Mutex<VecDeque<(f64, String)>>,
    noise_floor: Mutex<f32>,
    busy: AtomicBool,
    held: Mutex<Option<(f64, String, Option<J>, Option<J>)>>,
    partial: Mutex<Option<(u64, String, f64)>>,
}

pub struct Listener {
    hub: Hub,
    audio: Arc<Audio>,
    voice: Arc<Voice>,
    pub whisper: Arc<Mutex<Whisper>>,
    verifier: Arc<Mutex<SpeakerVerifier>>,
    opts: Arc<Mutex<J>>,
    corpus: Arc<CorpusRecorder>,
    pub s: Shared,
    jobs: Mutex<VecDeque<Job>>,
    jobs_cv: Condvar,
    who_jobs: SyncSender<(u64, Vec<f32>)>,
    pub debug: Option<Arc<crate::debug::AudioDebug>>,
    raw_stt: bool,
}

impl Listener {
    #[allow(clippy::too_many_arguments)]
    pub fn start(
        hub: Hub,
        audio: Arc<Audio>,
        voice: Arc<Voice>,
        whisper: Arc<Mutex<Whisper>>,
        verifier: Arc<Mutex<SpeakerVerifier>>,
        opts: Arc<Mutex<J>>,
        corpus: Arc<CorpusRecorder>,
        blocks: Receiver<Vec<f32>>,
        debug: Option<Arc<crate::debug::AudioDebug>>,
    ) -> Arc<Self> {
        let (wtx, wrx) = std::sync::mpsc::sync_channel::<(u64, Vec<f32>)>(1);
        let raw_stt = opts.lock().unwrap()["sttRawAudio"] != J::Bool(false);
        let me = Arc::new(Self {
            hub,
            audio,
            voice,
            whisper,
            verifier,
            opts,
            corpus,
            s: Shared {
                muted: AtomicBool::new(false),
                mode: Mutex::new("wakeword".into()),
                gate_until: Mutex::new(0.0),
                wake_at: Mutex::new(-1e9),
                segment: AtomicU64::new(0),
                prefix: Mutex::new(HashMap::new()),
                prompt: Mutex::new(filters::build_initial_prompt(&[])),
                hotwords: Mutex::new(None),
                corrections: Mutex::new(vec![]),
                recent_said: Mutex::new(VecDeque::new()),
                noise_floor: Mutex::new(0.003),
                busy: AtomicBool::new(false),
                held: Mutex::new(None),
                partial: Mutex::new(None),
            },
            jobs: Mutex::new(VecDeque::new()),
            jobs_cv: Condvar::new(),
            who_jobs: wtx,
            debug,
            raw_stt,
        });
        let w = me.clone();
        std::thread::Builder::new().name("transcription".into()).spawn(move || w.worker()).ok();
        let w2 = me.clone();
        std::thread::Builder::new()
            .name("empreinte".into())
            .spawn(move || {
                while let Ok((segment, audio)) = wrx.recv() {
                    if let Some(who) = w2.who(&audio) {
                        if segment == w2.s.segment.load(Ordering::SeqCst) {
                            w2.hub.emit(json!({ "t": "who", "segment": segment, "who": who }));
                        }
                    }
                }
            })
            .ok();
        let w3 = me.clone();
        std::thread::Builder::new()
            .name("ecoute".into())
            .spawn(move || {
                let mut blocks = blocks;
                loop {
                    let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| w3.run(&mut blocks)));
                    if r.is_err() {
                        log("erreur du thread d'écoute : reprise…");
                        w3.hub.emit_state(json!({ "error": "écoute interrompue, reprise…", "capturing": false }));
                        std::thread::sleep(Duration::from_secs(1));
                    } else {
                        break;
                    }
                }
            })
            .ok();
        me
    }

    // --- Réglages venus du noyau ---------------------------------------------------------------

    pub fn set_vocabulary(&self, terms: &J, corrections: &J) {
        let t: Vec<String> = terms.as_array().map(|a| a.iter().filter_map(|v| v.as_str().map(String::from)).collect()).unwrap_or_default();
        *self.s.prompt.lock().unwrap() = filters::build_initial_prompt(&t);
        *self.s.hotwords.lock().unwrap() = filters::build_hotwords(&t);
        let c = filters::compile_corrections(corrections);
        let n = c.len();
        *self.s.corrections.lock().unwrap() = c;
        log(&format!("vocabulaire : {} termes, {n} correction(s)", t.len()));
    }

    pub fn gate_open(&self) -> bool {
        *self.s.mode.lock().unwrap() != "wakeword" || clock() < *self.s.gate_until.lock().unwrap()
    }

    pub fn set_gate(&self, open: bool, seconds: f64) {
        *self.s.gate_until.lock().unwrap() = if open { clock() + seconds } else { 0.0 };
    }

    pub fn remember_said(&self, text: &str) {
        let mut r = self.s.recent_said.lock().unwrap();
        r.push_back((clock(), text.to_string()));
        while r.len() > 12 {
            r.pop_front();
        }
    }

    pub fn recent_phrases(&self, seconds: f64) -> Vec<String> {
        let now = clock();
        self.s.recent_said.lock().unwrap().iter().filter(|(t, _)| now - t < seconds).map(|(_, s)| s.clone()).collect()
    }

    fn opt_f(&self, k: &str, d: f64) -> f64 {
        self.opts.lock().unwrap()[k].as_f64().unwrap_or(d)
    }

    fn opt_b(&self, k: &str, d: bool) -> bool {
        self.opts.lock().unwrap()[k].as_bool().unwrap_or(d)
    }

    fn opt_s(&self, k: &str, d: &str) -> String {
        self.opts.lock().unwrap()[k].as_str().unwrap_or(d).to_string()
    }

    // --- Boucle d'écoute -----------------------------------------------------------------------

    fn run(self: &Arc<Self>, blocks: &mut Receiver<Vec<f32>>) {
        let models = crate::util::models_dir();
        let mut vad = crate::vad::Vad::load(&models.join("vad").join("silero_vad_v6.onnx"))
            .map_err(|e| log(&format!("Silero VAD indisponible ({e}), repli sur détection d'énergie RMS classique")))
            .ok();
        if vad.is_some() {
            log("Silero VAD v6 neuronal (ONNX) initialisé avec succès");
        }
        // Combinaison des détecteurs (réglage wakeEngine) : phonétique (sherpa-onnx), openWakeWord, ou les deux.
        let mut mode = crate::oww::Mode::parse(&self.opt_s("wakeEngine", "phonetique"));
        let mut oww = None;
        if self.opt_b("acousticWake", true) && mode != crate::oww::Mode::Phonetic {
            let name = self.opt_s("wakeModel", "jarvis.onnx");
            let threshold = self.opt_f("wakeThreshold", 0.5) as f32;
            match crate::oww::OpenWakeWord::new(&name, threshold) {
                Ok(w) => {
                    log(&format!("mot d'éveil openWakeWord actif ({name}, seuil {threshold}, mode {mode:?})"));
                    oww = Some(w);
                }
                Err(e) => {
                    log(&format!("mot d'éveil openWakeWord indisponible ({e}) : détecteur phonétique seul"));
                    mode = crate::oww::Mode::Phonetic;
                }
            }
        }
        let (mut phonetic_at, mut oww_at): (Option<f64>, Option<f64>) = (None, None);
        let mut spotter = None;
        if self.opt_b("acousticWake", true) && mode != crate::oww::Mode::Oww {
            if crate::wake::available() {
                let (k, t, sc) = crate::wake::calibrated_settings();
                match crate::wake::WakeSpotter::new(&k, t, sc) {
                    Ok(w) => {
                        log(&format!("mot d'éveil acoustique actif ({k}, seuil {t})"));
                        spotter = Some(w);
                    }
                    Err(e) => log(&format!("mot d'éveil acoustique indisponible ({e}) : garde textuelle seule")),
                }
            } else {
                log("mot d'éveil acoustique : modèle absent (data/models/kws), garde textuelle seule");
            }
        }
        let shared = self.audio.shared.clone();
        let mut consumed: u64 = 0;
        let mut pre: VecDeque<Vec<f32>> = VecDeque::new();
        let pre_max = (0.35 * SR as f64 / BLOCK as f64) as usize;
        let mut buf: Vec<Vec<f32>> = vec![];
        let mut capturing = false;
        let (mut speech, mut silence, mut loud_blocks, mut barge) = (0usize, 0usize, 0usize, 0i64);
        let mut wake_in_segment = false;
        let mut during_speech = false;
        // Pause sans suite (le noyau n'a rien dit) : reprise d'elle-même à cette heure.
        let mut hold_release = 0.0f64;
        let mut capture_at = 0.0f64;
        let mut prefix_sent = false;
        let (mut last_partial, mut last_level) = (0.0f64, 0.0f64);
        let mut level = 0.0f32;
        let mut last_voiced_at = 0.0f64;
        let mut forced_partial = false;
        let grace_blocks = (0.7 * SR as f64 / BLOCK as f64) as usize;
        loop {
            let block = match blocks.recv_timeout(Duration::from_secs(3)) {
                Ok(b) => b,
                Err(RecvTimeoutError::Timeout) => {
                    if shared.file_input.is_none() && !shared.suspended.load(Ordering::SeqCst) {
                        log("aucun son du micro depuis 3 s : réouverture");
                        self.audio.send(AudioCmd::Reopen);
                    }
                    continue;
                }
                Err(RecvTimeoutError::Disconnected) => return,
            };
            consumed += 1;
            let max_blocks = (self.opt_f("maxSegment", 25.0) * SR as f64 / BLOCK as f64) as usize;
            // Délais relus à chaque bloc : un réglage de l'administration s'applique tout de suite.
            let end_blocks = ((self.opt_f("minEndpointingDelay", 0.45) * SR as f64 / BLOCK as f64) as usize).max(4);
            let long_blocks = ((self.opt_f("maxEndpointingDelay", 2.5) * SR as f64 / BLOCK as f64) as usize).max(end_blocks);
            let barge_blocks = ((self.opt_f("minInterruptionDuration", 0.25) * SR as f64 / BLOCK as f64) as i64).max(3);
            let rms = crate::util::rms(&block);
            level += ((rms / 0.08).powf(0.6).min(1.0) - level) * 0.5;
            let now = clock();
            if now - last_level > 0.05 {
                last_level = now;
                self.hub.emit(json!({ "t": "level", "v": (level as f64 * 1000.0).round() / 1000.0 }));
            }
            if self.s.muted.load(Ordering::SeqCst) {
                continue;
            }
            let prob = vad.as_mut().map(|v| v.prob(&block)).unwrap_or(0.0);
            let floor = *self.s.noise_floor.lock().unwrap();
            let voiced = if vad.is_some() { prob >= 0.5 } else { rms > (floor * 3.0).max(0.004) };
            let speaking = self.voice.speaking.load(Ordering::SeqCst);
            // Pas pendant que Jarvis parle : le résidu d'écho ferait monter le plancher de bruit.
            if !capturing && !voiced && rms > 1e-5 && !speaking {
                let mut f = self.s.noise_floor.lock().unwrap();
                *f = (*f + (rms - *f) * 0.01).clamp(0.0005, 0.05);
            }

            // Détecteur acoustique : en continu, et pendant que Jarvis parle si sa voix ne peut pas
            // revenir dans le micro (anti-écho actif, ou casque).
            let mut wake_heard = false;
            let spot_during_speech = shared.aec.load(Ordering::Relaxed) || shared.config() == "casque";
            if !speaking || spot_during_speech {
                if spotter.as_mut().map(|sp| sp.feed(&block)).unwrap_or(false) {
                    phonetic_at = Some(now);
                }
                if oww.as_mut().map(|w| w.feed(&block)).unwrap_or(false) {
                    oww_at = Some(now);
                }
                if crate::oww::decide(mode, now, phonetic_at, oww_at) {
                    let source = if oww_at == Some(now) && phonetic_at == Some(now) { "acoustique+openwakeword" } else if oww_at == Some(now) { "openwakeword" } else { "acoustique" };
                    (phonetic_at, oww_at) = (None, None);
                    wake_heard = true;
                    *self.s.wake_at.lock().unwrap() = now;
                    wake_in_segment = wake_in_segment || capturing;
                    self.hub.emit(json!({ "t": "wake", "source": source }));
                }
            }

            // Pause restée sans suite : Jarvis reprend.
            if !capturing && hold_release > 0.0 && now > hold_release {
                if self.voice.held() {
                    self.voice.hold(false, None);
                }
                hold_release = 0.0;
            }

            // Jarvis parle et personne ne lui parle encore : seule une vraie voix humaine, maintenue,
            // ouvre l'écoute. Jarvis se MET EN PAUSE (plus d'écho dans le micro) ; le noyau décide ensuite :
            // vraie demande → il abandonne et répond, sinon il reprend.
            if speaking && !capturing {
                let loud = prob >= 0.7 && rms > (floor * 4.0).max(0.01);
                if self.opt_b("allowInterruptions", true) && (loud || wake_heard || barge > 0) {
                    barge = if wake_heard { barge_blocks } else if loud { barge + 1 } else { barge - 1 };
                    if barge >= barge_blocks {
                        let why = if wake_heard { "« Jarvis » entendu".to_string() } else { format!("vad {prob:.2}") };
                        log(&format!("l'utilisateur parle pendant la réponse ({why}) : Jarvis en pause, écoute"));
                        let seg = self.s.segment.fetch_add(1, Ordering::SeqCst) + 1;
                        self.voice.hold(true, Some(seg));
                        self.hub.emit(json!({ "t": "interrupted" }));
                        barge = 0;
                        hold_release = 0.0;
                        capture_at = now;
                        capturing = true;
                        buf = pre.drain(..).collect();
                        buf.push(block);
                        speech = 0;
                        silence = 0;
                        loud_blocks = barge_blocks as usize;
                        wake_in_segment = true; // on répond à Jarvis : pas besoin de redire « Jarvis »
                        during_speech = true;
                        prefix_sent = true;
                        self.hub.emit_state(json!({ "capturing": true }));
                        continue;
                    }
                } else {
                    barge = 0;
                }
                pre.push_back(block);
                while pre.len() > pre_max {
                    pre.pop_front();
                }
                continue;
            }
            barge = 0;

            if !capturing {
                pre.push_back(block);
                while pre.len() > pre_max {
                    pre.pop_front();
                }
                speech = if voiced { speech + 1 } else { 0 };
                if speech >= 2 {
                    capturing = true;
                    buf = pre.drain(..).collect();
                    silence = 0;
                    loud_blocks = speech;
                    wake_in_segment = now - *self.s.wake_at.lock().unwrap() < 1.5;
                    let seg = self.s.segment.fetch_add(1, Ordering::SeqCst) + 1;
                    // L'utilisateur parle : rien de nouveau ne démarre par-dessus lui.
                    self.voice.hold(true, Some(seg));
                    hold_release = 0.0;
                    capture_at = now;
                    prefix_sent = false;
                    speech = 0;
                    self.hub.emit_state(json!({ "capturing": true }));
                }
                continue;
            }

            buf.push(block);
            silence = if voiced { 0 } else { silence + 1 };
            if voiced {
                last_voiced_at = now;
                forced_partial = false;
                loud_blocks += 1;
            }
            if now - *self.s.wake_at.lock().unwrap() < 1.0 {
                wake_in_segment = true;
            }
            let segment = self.s.segment.load(Ordering::SeqCst);
            let mode_wake = *self.s.mode.lock().unwrap() == "wakeword";
            // Hors conversation : dès que le début de la phrase est là, on y cherche « Jarvis ».
            let head_len = (self.opt_f("wakePrefixSeconds", 1.6) * SR as f64) as usize;
            if !prefix_sent && !wake_in_segment && mode_wake && !self.gate_open() && buf.len() * BLOCK >= head_len {
                prefix_sent = true;
                let proc: Vec<f32> = buf.concat();
                let (stt, stt_raw, _) = self.stt_audio(&proc, consumed, buf.len());
                self.submit(Job::Prefix { audio: stt[..head_len.min(stt.len())].to_vec(), segment, raw: stt_raw });
            }
            // Fin de phrase : silence de « minEndpointingDelay »… sauf si la phrase semble inachevée.
            let listening = self.gate_open() || wake_in_segment || during_speech;
            // Longue parole qui ne s'adresse pas à Jarvis (télévision, conversation dans la pièce) : il ne
            // retient pas ses annonces au-delà de HOLD_MAX.
            if !listening && now - capture_at > HOLD_MAX {
                self.voice.release_for(segment);
            }
            let mut done = buf.len() >= max_blocks;
            if !done && silence >= end_blocks {
                let p = self.s.partial.lock().unwrap().clone().filter(|p| p.0 == segment);
                let fresh = p.as_ref().map(|p| p.2 >= last_voiced_at).unwrap_or(false);
                if !listening {
                    done = true;
                } else if !fresh && silence < end_blocks + grace_blocks {
                    if !forced_partial && !self.s.busy.load(Ordering::SeqCst) && self.jobs_empty() {
                        forced_partial = true;
                        let proc: Vec<f32> = buf.concat();
                        let (stt, stt_raw, _) = self.stt_audio(&proc, consumed, buf.len());
                        let a = stt[stt.len().saturating_sub(PARTIAL_WINDOW)..].to_vec();
                        let spk = proc[proc.len().saturating_sub(SPEAKER_WINDOW)..].to_vec();
                        self.submit(Job::Partial { audio: a, raw: stt_raw, segment, at: now, spk });
                    }
                } else if fresh && silence < long_blocks && filters::looks_unfinished(&p.as_ref().unwrap().1) {
                    if silence == end_blocks || (silence - end_blocks) % (SR / BLOCK) == 0 {
                        self.hub.emit(json!({ "t": "waiting_more", "text": p.unwrap().1 }));
                    }
                } else {
                    done = true;
                }
            }
            if done {
                let audio: Vec<f32> = buf.concat();
                let (stt, stt_raw, raw_audio) = self.stt_audio(&audio, consumed, buf.len());
                buf.clear();
                capturing = false;
                silence = 0;
                self.hub.emit_state(json!({ "capturing": false }));
                let sent = audio.len() >= (0.35 * SR as f64) as usize && loud_blocks >= 3;
                if sent {
                    self.segment(&audio, wake_in_segment, segment, stt, stt_raw, raw_audio);
                }
                // Trop court : Jarvis reprend tout de suite ; sinon il attend la décision du noyau (5 s au plus).
                if !sent {
                    self.voice.release_for(segment);
                } else if self.voice.held() {
                    hold_release = now + 5.0;
                }
                during_speech = false;
                wake_in_segment = false;
            } else if now - last_partial > PARTIAL_PERIOD && (self.gate_open() || wake_in_segment || during_speech) {
                last_partial = now;
                if !self.s.busy.load(Ordering::SeqCst) && self.jobs_empty() {
                    let proc: Vec<f32> = buf.concat();
                    let (stt, stt_raw, _) = self.stt_audio(&proc, consumed, buf.len());
                    let a = stt[stt.len().saturating_sub(PARTIAL_WINDOW)..].to_vec();
                    if a.len() >= (0.35 * SR as f64) as usize {
                        let spk = proc[proc.len().saturating_sub(SPEAKER_WINDOW)..].to_vec();
                        self.submit(Job::Partial { audio: a, raw: stt_raw, segment, at: now, spk });
                    }
                }
            }
        }
    }

    // --- Son donné à Whisper -------------------------------------------------------------------
    // Sans écho possible, Whisper reçoit le son brut (le traitement WebRTC le fait halluciner) ;
    // la détection de parole garde le son traité.

    fn echo_free(&self, nblocks: usize) -> bool {
        let shared = &self.audio.shared;
        if !shared.aec.load(Ordering::Relaxed) {
            return true;
        }
        if self.voice.speaking.load(Ordering::SeqCst) {
            return false;
        }
        shared.since_echo() > nblocks as f64 * BLOCK as f64 / SR as f64 + 0.5
    }

    /// (son pour Whisper, est-ce le brut ?, son brut ou None) des nblocks derniers blocs lus.
    fn stt_audio(&self, processed: &[f32], consumed: u64, nblocks: usize) -> (Vec<f32>, bool, Option<Vec<f32>>) {
        let raw = self.audio.shared.raw_blocks(consumed.saturating_sub(nblocks as u64), nblocks).filter(|r| r.len() == processed.len());
        if let Some(r) = &raw {
            if self.raw_stt && self.echo_free(nblocks) {
                return (r.clone(), true, raw.clone());
            }
        }
        (processed.to_vec(), false, raw)
    }

    /// File de 4 travaux au plus (les plus récents passent à la trappe quand elle est pleine).
    fn submit(&self, job: Job) {
        let mut q = self.jobs.lock().unwrap();
        if q.len() < 4 {
            q.push_back(job);
            self.jobs_cv.notify_one();
        }
    }

    fn jobs_empty(&self) -> bool {
        self.jobs.lock().unwrap().is_empty()
    }

    fn note(&self, segment: u64, fields: J) {
        if let Some(d) = &self.debug {
            d.note(segment, fields);
        }
    }

    /// audio : son traité (empreinte vocale, corpus) ; stt : son donné à Whisper.
    fn segment(&self, audio: &[f32], wake_heard: bool, segment: u64, stt: Vec<f32>, stt_raw: bool, raw_audio: Option<Vec<f32>>) {
        let shared = &self.audio.shared;
        if let Some(d) = &self.debug {
            d.save(
                segment,
                audio,
                raw_audio.as_deref(),
                json!({
                    "duree": ((audio.len() as f64 / SR as f64) * 100.0).round() / 100.0, "config": shared.config(),
                    "whisper": if stt_raw { "brut" } else { "traité" }, "anti_echo": shared.aec.load(Ordering::Relaxed),
                    "son_pc": shared.reference.load(Ordering::Relaxed), "garde_ouverte": self.gate_open(),
                    "jarvis_detecte": wake_heard, "micro": *shared.input_name.lock().unwrap()
                }),
            );
        }
        if self.corpus.active() {
            self.corpus.save(audio);
            self.voice.release_for(segment);
            return;
        }
        let mut speaker = None;
        {
            let mut v = self.verifier.lock().unwrap();
            if v.available {
                v.config = shared.config();
                if v.is_enrolling {
                    let mut res = v.enroll_sample(audio);
                    let mut ev = json!({ "t": "speaker_enroll_progress" });
                    if let (Some(e), Some(r)) = (ev.as_object_mut(), res.as_object_mut()) {
                        for (k, val) in r.iter() {
                            e.insert(k.clone(), val.clone());
                        }
                    }
                    self.hub.emit(ev);
                    if res["completed"].as_bool().unwrap_or(false) {
                        let st = v.get_status();
                        self.hub.emit(json!({ "t": "speaker_enrolled", "status": st, "speaker": res["speaker"] }));
                        self.hub.emit_state(json!({ "speaker": st }));
                    }
                    self.voice.release_for(segment);
                    return;
                }
                // Mode « reject » : une voix douteuse ou inconnue est quand même transcrite ; le noyau
                // ne l'exécute pas tant que personne ne l'a confirmée.
                speaker = Some(v.verify(audio));
            }
        }
        // Une phrase finale est prioritaire sur les partielles en attente.
        let wake = wake_heard || self.gate_open();
        let mut q = self.jobs.lock().unwrap();
        q.retain(|j| matches!(j, Job::Prefix { .. }));
        q.push_back(Job::Final { audio: stt, speaker, wake, segment, raw: stt_raw });
        self.jobs_cv.notify_one();
    }

    // --- Transcription -------------------------------------------------------------------------

    fn normalize(&self, audio: &[f32], raw: bool) -> Vec<f32> {
        // Le plancher de bruit est mesuré sur le son traité : il ne s'applique pas au son brut.
        let rms = crate::util::rms(audio);
        let floor = *self.s.noise_floor.lock().unwrap();
        let threshold = if raw { 0.002 } else { (floor * 2.5).max(0.004) };
        if rms > threshold {
            let gain = (0.06 / rms).min(4.0);
            if gain > 1.0 {
                return audio.iter().map(|x| (x * gain).clamp(-1.0, 1.0)).collect();
            }
        }
        audio.to_vec()
    }

    fn worker(self: Arc<Self>) {
        loop {
            let job = {
                let mut q = self.jobs.lock().unwrap();
                while q.is_empty() {
                    q = self.jobs_cv.wait(q).unwrap();
                }
                q.pop_front().unwrap()
            };
            self.s.busy.store(true, Ordering::SeqCst);
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| match job {
                Job::Prefix { audio, segment, raw } => self.prefix(&audio, segment, raw),
                Job::Partial { audio, raw, segment, at, spk } => {
                    let text = self.transcribe(&audio, false, None, None, raw).unwrap_or_default();
                    *self.s.partial.lock().unwrap() = Some((segment, text.clone(), at));
                    if !text.is_empty() {
                        self.hub.emit(json!({ "t": "partial", "text": text }));
                        // Qui parle : seule la demande la plus récente compte.
                        let _ = self.who_jobs.try_send((segment, spk));
                    }
                }
                Job::Final { audio, speaker, wake, segment, raw } => self.final_(&audio, speaker, wake, segment, raw),
            }));
            if r.is_err() {
                log("erreur de transcription (reprise)");
            }
            self.s.busy.store(false, Ordering::SeqCst);
        }
    }

    fn transcribe(&self, audio: &[f32], final_: bool, speaker: Option<&J>, beam: Option<usize>, raw: bool) -> Option<String> {
        let audio = self.normalize(audio, raw);
        let accepted = speaker.map(|s| s["decision"] == "accepted").unwrap_or(false);
        let score = if accepted { speaker.and_then(|s| s["similarity"].as_f64()) } else { None };
        let threshold = speaker.and_then(|s| s["threshold"].as_f64());
        let beam = beam.unwrap_or(if final_ { self.opt_f("sttBeam", 2.0) as usize } else { 1 });
        let prompt = self.s.prompt.lock().unwrap().clone();
        let hot = self.s.hotwords.lock().unwrap().clone();
        let mut w = self.whisper.lock().unwrap();
        let d = w.transcribe(&audio, &prompt, hot.as_deref(), beam);
        let corrected = filters::clean_and_autocorrect(&d.text, &self.s.corrections.lock().unwrap());
        let recent = if final_ { self.recent_phrases(12.0) } else { vec![] };
        let (mut text, mut reason) = filters::assess(&corrected, &d, audio.len(), final_, &recent, score, threshold);
        if final_ {
            if let Some(t) = &text {
                if filters::english_hint(t) {
                    if let Some((lang, p)) = w.detect_language(&audio) {
                        if lang == "en" && p >= 0.5 {
                            text = None;
                            reason = "phrase en anglais".into();
                        }
                    }
                }
            }
        }
        if final_ && text.is_none() && !d.text.is_empty() {
            log(&format!("rejet ({reason}) : « {} »", d.text.chars().take(90).collect::<String>()));
        }
        text
    }

    /// Morceau d'une réunion : même Whisper (verrou partagé), mêmes filtres anti-hallucination.
    pub fn meeting_transcribe(&self, audio: &[f32]) -> Option<String> {
        let audio = self.normalize(audio, true);
        let prompt = self.s.prompt.lock().unwrap().clone();
        let hot = self.s.hotwords.lock().unwrap().clone();
        let d = self.whisper.lock().unwrap().transcribe(&audio, &prompt, hot.as_deref(), self.opt_f("sttBeam", 2.0) as usize);
        let corrected = filters::clean_and_autocorrect(&d.text, &self.s.corrections.lock().unwrap());
        let (text, reason) = filters::assess(&corrected, &d, audio.len(), true, &[], None, None);
        if text.is_none() && !d.text.is_empty() {
            log(&format!("réunion : rejet ({reason}) : « {} »", d.text.chars().take(90).collect::<String>()));
        }
        text
    }

    fn has_wake(&self, head: &[f32], raw: bool) -> (bool, String) {
        let audio = self.normalize(head, raw);
        let prompt = self.s.prompt.lock().unwrap().clone();
        let hot = self.s.hotwords.lock().unwrap().clone();
        let d = self.whisper.lock().unwrap().transcribe(&audio, &prompt, hot.as_deref(), 1);
        let text = filters::fix_command_start(&filters::clean_and_autocorrect(&d.text, &self.s.corrections.lock().unwrap()));
        (filters::wake_word(&text), d.text)
    }

    fn prefix(&self, head: &[f32], segment: u64, raw: bool) {
        let (found, heard) = self.has_wake(head, raw);
        self.note(segment, json!({ "etape": "début (1,6 s)", "jarvis": found, "entendu": heard.chars().take(80).collect::<String>() }));
        let mut p = self.s.prefix.lock().unwrap();
        p.insert(segment, found);
        if p.len() > 50 {
            let mut keys: Vec<u64> = p.keys().copied().collect();
            keys.sort();
            for k in &keys[..keys.len() - 20] {
                p.remove(k);
            }
        }
        drop(p);
        if found {
            *self.s.wake_at.lock().unwrap() = clock();
            self.hub.emit(json!({ "t": "wake", "source": "texte" }));
        }
    }

    fn final_(self: &Arc<Self>, audio: &[f32], speaker: Option<J>, wake_flag: bool, seg: u64, raw: bool) {
        let known = self.s.prefix.lock().unwrap().remove(&seg);
        let mut wake = wake_flag || known == Some(true);
        let mut text: Option<String> = None;
        if !wake && *self.s.mode.lock().unwrap() == "wakeword" {
            // Garde « Jarvis » : hors conversation, seul le début de la phrase est transcrit. Pas de
            // « Jarvis » en tête : la phrase est ignorée (télévision, conversation dans la pièce…).
            let mut heard = String::new();
            if known.is_none() {
                let n = ((self.opt_f("wakePrefixSeconds", 1.6) * SR as f64) as usize).min(audio.len());
                let (w, h) = self.has_wake(&audio[..n], raw);
                wake = w;
                heard = h;
            }
            if !wake && audio.len() as f64 <= FULL_CHECK * SR as f64 {
                // Seconde chance pour une phrase courte : le début décodé en beam 1 coupe parfois « Jarvis ».
                text = self.transcribe(audio, true, speaker.as_ref(), None, raw);
                wake = text.as_deref().map(filters::wake_word).unwrap_or(false);
            }
            if !wake {
                let shown: String = text.clone().filter(|t| !t.is_empty()).unwrap_or(heard).chars().take(60).collect();
                if shown.is_empty() {
                    log(&format!("ignorée (pas de « Jarvis » au début) · {:.1} s", audio.len() as f64 / SR as f64));
                } else {
                    log(&format!("ignorée (pas de « Jarvis » au début) : « {shown} »"));
                }
                self.note(seg, json!({ "decision": "ignorée : pas de « Jarvis » au début", "entendu": shown }));
                self.hub.emit(json!({ "t": "ignored", "reason": "sans mot d'éveil", "preview": shown }));
                self.voice.release_for(seg);
                return;
            }
        }
        let text = match text {
            Some(t) => Some(t),
            None => self.transcribe(audio, true, speaker.as_ref(), None, raw),
        };
        let Some(mut text) = text.filter(|t| !t.is_empty()) else {
            self.note(seg, json!({ "decision": "rejetée par les filtres (voir engine.log)" }));
            self.voice.release_for(seg);
            return;
        };
        let mut who = speaker.as_ref().map(|s| s["speaker"].clone()).filter(|v| !v.is_null());
        let mut check = Self::check(speaker.as_ref());
        // « Jarvis » seul suivi d'une pause : on attend la suite pour n'en faire qu'une demande.
        if filters::wake_only(&text) {
            self.note(seg, json!({ "decision": "« Jarvis » seul : attend la suite", "texte": text }));
            *self.s.held.lock().unwrap() = Some((clock(), text, who, check));
            {
                let mut g = self.s.gate_until.lock().unwrap();
                *g = g.max(clock() + 6.0);
            }
            self.schedule_release(WAKE_HOLD + 0.2);
            return;
        }
        if let Some((t0, htext, hwho, hcheck)) = self.s.held.lock().unwrap().take() {
            if clock() - t0 < WAKE_HOLD + 4.0 {
                text = format!("{htext} {text}");
                who = who.or(hwho);
                check = check.or(hcheck);
            }
        }
        let mut msg = json!({ "t": "final", "text": text, "config": self.audio.shared.config() });
        if let Some(w) = &who {
            msg["speaker"] = w.clone();
        } else if speaker.as_ref().map(|s| s["decision"] == "unknown").unwrap_or(false) {
            msg["speakerUnknown"] = json!(true);
        }
        if let Some(c) = &check {
            msg["speakerCheck"] = c.clone();
        }
        let verdict = match check.as_ref().and_then(|c| c["decision"].as_str()) {
            Some("doubt") => " (voix douteuse)",
            Some("rejected") => " (voix non reconnue)",
            _ => "",
        };
        let name = who.as_ref().and_then(|w| w["name"].as_str()).map(|n| format!(" ({n})")).unwrap_or_default();
        log(&format!("phrase : « {text} »{name}{verdict}"));
        self.note(seg, json!({ "decision": "transmise", "texte": text }));
        self.hub.emit(msg);
    }

    /// Verdict de l'empreinte joint à la phrase finale (HUD : couleur, boutons de confirmation).
    fn check(speaker: Option<&J>) -> Option<J> {
        let s = speaker?;
        let d = s["decision"].as_str()?;
        if ["bypass", "disabled"].contains(&d) {
            return None;
        }
        let mut out = serde_json::Map::new();
        for k in ["decision", "state", "similarity", "threshold", "candidate", "second", "sample"] {
            if !s[k].is_null() {
                out.insert(k.into(), s[k].clone());
            }
        }
        Some(Value::Object(out))
    }

    /// Qui parle, pendant la phrase (transcriptions partielles) : verdict sans filtrage.
    fn who(&self, audio: &[f32]) -> Option<J> {
        let mut v = self.verifier.lock().unwrap();
        if audio.is_empty() || !v.available || v.is_enrolling {
            return None;
        }
        v.config = self.audio.shared.config();
        let (ident, _) = v.identify(audio, SR)?;
        let spk = &ident["speaker"];
        Some(json!({ "state": ident["state"], "similarity": ident["similarity"], "name": spk["name"], "id": spk["id"] }))
    }

    fn schedule_release(self: &Arc<Self>, seconds: f64) {
        let me = Arc::downgrade(self);
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs_f64(seconds));
            if let Some(m) = me.upgrade() {
                m.release_hold();
            }
        });
    }

    fn release_hold(self: &Arc<Self>) {
        let held = self.s.held.lock().unwrap().clone();
        let Some((t0, text, who, check)) = held else { return };
        if clock() - t0 < WAKE_HOLD {
            return;
        }
        if self.hub.state_flag("capturing") {
            // L'utilisateur parle encore : la suite arrive.
            self.schedule_release(0.3);
            return;
        }
        *self.s.held.lock().unwrap() = None;
        let mut msg = json!({ "t": "final", "text": text, "config": self.audio.shared.config() });
        if let Some(w) = who {
            msg["speaker"] = w;
        }
        if let Some(c) = check {
            msg["speakerCheck"] = c;
        }
        self.hub.emit(msg);
    }
}
