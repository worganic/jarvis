// Enregistrement d'une réunion (voice/meeting.py de la v3) : transcription continue sur deux pistes.
//
//   « moi »          : le micro. Aux haut-parleurs, le son nettoyé par l'anti-écho ; au casque, le
//                      son brut, meilleur pour Whisper.
//   « participants » : tout ce que joue le PC (capture « loopback ») — la visio elle-même.
//
// Chaque piste est découpée sur ses silences, puis transcrite par le même Whisper que l'écoute
// (verrou GPU partagé). Le son n'est jamais écrit sur le disque : seules les phrases partent au
// noyau (« meeting_text »). Côté participants, l'empreinte de chaque morceau est comparée aux
// personnes enregistrées, puis aux voix inconnues de la réunion (« Voix inconnue 1 », « 2 »…).

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{Receiver, SyncSender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{json, Value};

use crate::audio::Audio;
use crate::hub::Hub;
use crate::speaker::SpeakerVerifier;
use crate::util::{log, J};

const SR: usize = 16000;
const FRAME: usize = 512;
const MIN_SPEECH: f64 = 0.5;
const END_SILENCE: f64 = 0.7;
const MIN_CHUNK: f64 = 2.0;
const MAX_CHUNK: f64 = 20.0;
const STALL: f64 = 0.9;
const ECHO_WINDOW: f64 = 15.0;
const CLUSTER_SIM: f64 = 0.55;
const CLUSTER_MIN_SECONDS: f64 = 1.2;
const CLUSTER_SAMPLES: usize = 8;
const SUGGESTIONS: usize = 3;

fn now() -> f64 {
    SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs_f64()).unwrap_or(0.0)
}

fn dot(a: &[f32], b: &[f32]) -> f64 {
    a.iter().zip(b).map(|(x, y)| (*x as f64) * (*y as f64)).sum()
}

// --- Voix des participants ----------------------------------------------------------------------

struct Cluster {
    id: String,
    label: String,
    sum: Vec<f32>,
    centroid: Vec<f32>,
    samples: Vec<(f64, Vec<f32>)>,
    person: Option<J>,
    count: usize,
}

struct Voices {
    verifier: Arc<Mutex<SpeakerVerifier>>,
    clusters: Vec<Cluster>,
    seq: usize,
    last: Option<J>,
}

impl Voices {
    fn reset(&mut self) {
        self.clusters.clear();
        self.seq = 0;
        self.last = None;
    }

    fn available(&self) -> bool {
        self.verifier.lock().unwrap().available
    }

    fn who(c: &Cluster) -> J {
        match &c.person {
            Some(p) => json!({ "id": p["id"], "name": p["name"], "known": true, "cluster": c.id }),
            None => json!({ "id": c.id, "name": c.label, "known": false, "cluster": c.id }),
        }
    }

    /// (locuteur, événement « meeting_voice » d'une nouvelle voix inconnue).
    fn assign(&mut self, audio: &[f32]) -> (Option<J>, Option<J>) {
        if !self.available() {
            return (None, None);
        }
        let duration = audio.len() as f64 / SR as f64;
        let (emb, ranked, threshold) = {
            let mut v = self.verifier.lock().unwrap();
            let Some(emb) = v.compute_embedding(audio, SR) else { return (self.last.clone(), None) };
            let ranked: Vec<(J, f64)> = v.rank(&emb).into_iter().map(|(i, s)| {
                let p = &v.speakers[i];
                (json!({ "id": p.id, "name": p.name, "type": p.typ.clone().unwrap_or_else(|| "autre".into()) }), s)
            }).collect();
            let t = v.threshold_for(duration);
            (emb, ranked, t)
        };
        if let Some((spk, sim)) = ranked.first().filter(|(_, s)| *s >= threshold) {
            let who = json!({ "id": spk["id"], "name": spk["name"], "known": true, "similarity": (sim * 1000.0).round() / 1000.0 });
            self.last = Some(who.clone());
            return (Some(who), None);
        }
        let mut best: Option<usize> = None;
        let mut best_sim = -1.0;
        for (i, c) in self.clusters.iter().enumerate() {
            let s = dot(&emb, &c.centroid);
            if s > best_sim {
                best = Some(i);
                best_sim = s;
            }
        }
        let mut event = None;
        let idx = match best {
            Some(i) if best_sim >= CLUSTER_SIM => i,
            _ if duration < CLUSTER_MIN_SECONDS && self.last.is_some() => return (self.last.clone(), None),
            _ => {
                self.seq += 1;
                let id = format!("u{}", self.seq);
                let label = format!("Voix inconnue {}", self.seq);
                event = Some(json!({
                    "t": "meeting_voice", "id": id, "label": label,
                    "suggest": ranked.iter().take(SUGGESTIONS).map(|(s, sim)| json!({ "id": s["id"], "name": s["name"], "type": s["type"], "similarity": (sim * 1000.0).round() / 1000.0 })).collect::<Vec<_>>()
                }));
                self.clusters.push(Cluster { id, label, sum: vec![0.0; emb.len()], centroid: emb.clone(), samples: vec![], person: None, count: 0 });
                self.clusters.len() - 1
            }
        };
        let c = &mut self.clusters[idx];
        c.sum.iter_mut().zip(&emb).for_each(|(a, b)| *a += b);
        let n: f32 = c.sum.iter().map(|x| x * x).sum::<f32>().sqrt();
        if n > 1e-6 {
            c.centroid = c.sum.iter().map(|x| x / n).collect();
        }
        c.count += 1;
        c.samples.push((duration, emb));
        c.samples.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));
        c.samples.truncate(CLUSTER_SAMPLES);
        let who = Self::who(c);
        self.last = Some(who.clone());
        (Some(who), event)
    }

    /// Voix inconnue nommée : profil appris, puis les voix inconnues qui lui ressemblent la rejoignent.
    fn name(&mut self, cid: &str, name: &str, speaker_type: &str) -> J {
        if !self.available() {
            return json!({ "ok": false, "error": "empreintes_indisponibles" });
        }
        let Some(idx) = self.clusters.iter().position(|c| c.id == cid) else { return json!({ "ok": false, "error": "voix_inconnue_introuvable" }) };
        let embs: Vec<Vec<f32>> = self.clusters[idx].samples.iter().map(|(_, e)| e.clone()).collect();
        let res = self.verifier.lock().unwrap().learn_embeddings(name, &embs, if speaker_type.is_empty() { "visio" } else { speaker_type }, "visio", "reunion");
        if !res["ok"].as_bool().unwrap_or(false) {
            return res;
        }
        let person = res["speaker"].clone();
        self.clusters[idx].person = Some(person.clone());
        let threshold = self.verifier.lock().unwrap().threshold_for(3.0);
        let mut merged = vec![];
        for i in 0..self.clusters.len() {
            if i == idx || self.clusters[i].person.is_some() {
                continue;
            }
            let v = self.verifier.lock().unwrap();
            let m = v.rank(&self.clusters[i].centroid).into_iter().find(|(j, _)| v.speakers[*j].id == person["id"].as_str().unwrap_or("")).map(|(_, s)| s).unwrap_or(-1.0);
            drop(v);
            if m >= threshold {
                self.clusters[i].person = Some(person.clone());
                merged.push(self.clusters[i].id.clone());
            }
        }
        let cluster_id = self.clusters[idx].id.clone();
        if let Some(last) = &self.last {
            let lc = last["cluster"].as_str().unwrap_or("").to_string();
            if lc == cluster_id || merged.contains(&lc) {
                self.last = Some(Self::who(&self.clusters[idx]));
            }
        }
        let mut out = res;
        out["id"] = json!(cluster_id);
        out["merged"] = json!(merged);
        out
    }
}

// --- Découpage d'une piste ---------------------------------------------------------------------

type ChunkFn = Arc<dyn Fn(&'static str, Vec<f32>, f64) + Send + Sync>;

struct Track {
    name: &'static str,
    on_chunk: ChunkFn,
    min_level: f32,
    floor: f32,
    pending: Vec<f32>,
    frames: Vec<f32>,
    voiced: usize,
    silence: usize,
    started_at: f64,
    last_at: f64,
}

impl Track {
    fn new(name: &'static str, on_chunk: ChunkFn, min_level: f32) -> Self {
        Self { name, on_chunk, min_level, floor: min_level / 2.0, pending: vec![], frames: vec![], voiced: 0, silence: 0, started_at: 0.0, last_at: 0.0 }
    }

    fn reset(&mut self) {
        self.frames.clear();
        self.voiced = 0;
        self.silence = 0;
        self.started_at = 0.0;
        self.last_at = 0.0;
    }

    fn feed(&mut self, samples: &[f32]) {
        self.pending.extend_from_slice(samples);
        let n = self.pending.len() / FRAME;
        let buf: Vec<f32> = self.pending.drain(..n * FRAME).collect();
        for f in buf.chunks(FRAME) {
            self.frame(f);
        }
    }

    fn frame(&mut self, frame: &[f32]) {
        let t = now();
        let rms = crate::util::rms(frame);
        let voiced = rms > (self.floor * 3.0).max(self.min_level);
        if self.frames.is_empty() && !voiced {
            self.floor += (rms.min(0.05) - self.floor) * 0.02;
            return;
        }
        if self.frames.is_empty() {
            self.started_at = t - FRAME as f64 / SR as f64;
        }
        self.frames.extend_from_slice(frame);
        self.last_at = t;
        if voiced {
            self.voiced += 1;
            self.silence = 0;
        } else {
            self.silence += 1;
        }
        let length = self.frames.len() as f64 / SR as f64;
        let sil = self.silence as f64 * FRAME as f64 / SR as f64;
        if length >= MAX_CHUNK || (length >= MIN_CHUNK && sil >= END_SILENCE) {
            self.flush();
        } else if (self.voiced as f64 * FRAME as f64 / SR as f64) < 0.15 && sil >= END_SILENCE {
            self.reset(); // claquement, souffle : abandonné
        }
    }

    fn flush(&mut self) {
        if !self.frames.is_empty() && self.voiced as f64 * FRAME as f64 / SR as f64 >= MIN_SPEECH {
            (self.on_chunk)(self.name, std::mem::take(&mut self.frames), self.started_at);
        }
        self.reset();
    }

    /// Flux interrompu (la sortie du PC n'envoie plus rien quand rien ne joue) : clôture.
    fn tick(&mut self) {
        if !self.frames.is_empty() && now() - self.last_at > STALL {
            self.flush();
        }
    }
}

// --- Enregistreur ------------------------------------------------------------------------------

enum Job {
    Chunk(&'static str, Vec<f32>, f64),
    Done,
}

pub type Transcribe = Arc<dyn Fn(&[f32]) -> Option<String> + Send + Sync>;
pub type RecentSaid = Arc<dyn Fn() -> Vec<String> + Send + Sync>;

pub struct MeetingRecorder {
    hub: Hub,
    audio: Arc<Audio>,
    transcribe: Transcribe,
    recent_said: RecentSaid,
    voices: Mutex<Voices>,
    pub active: AtomicBool,
    jobs: SyncSender<Job>,
    pc_lines: Mutex<Vec<(f64, String)>>,
    pc_queued: Mutex<Vec<f64>>,
    tracks: Mutex<Vec<Track>>,
    pc_device: Mutex<Option<String>>,
    started_at: Mutex<f64>,
    stats: Mutex<(u64, u64, u64)>,
}

impl MeetingRecorder {
    pub fn new(hub: Hub, audio: Arc<Audio>, transcribe: Transcribe, recent_said: RecentSaid, verifier: Arc<Mutex<SpeakerVerifier>>) -> Arc<Self> {
        let (tx, rx) = std::sync::mpsc::sync_channel::<Job>(240);
        let me = Arc::new(Self {
            hub,
            audio,
            transcribe,
            recent_said,
            voices: Mutex::new(Voices { verifier, clusters: vec![], seq: 0, last: None }),
            active: AtomicBool::new(false),
            jobs: tx,
            pc_lines: Mutex::new(vec![]),
            pc_queued: Mutex::new(vec![]),
            tracks: Mutex::new(vec![]),
            pc_device: Mutex::new(None),
            started_at: Mutex::new(0.0),
            stats: Mutex::new((0, 0, 0)),
        });
        let w = me.clone();
        std::thread::Builder::new().name("reunion-transcription".into()).spawn(move || w.worker(rx)).ok();
        let w2 = me.clone();
        std::thread::Builder::new()
            .name("reunion-veille".into())
            .spawn(move || loop {
                std::thread::sleep(Duration::from_millis(300));
                if w2.active.load(Ordering::SeqCst) {
                    for t in w2.tracks.lock().unwrap().iter_mut() {
                        t.tick();
                    }
                }
            })
            .ok();
        me
    }

    fn stats_json(&self) -> J {
        let s = *self.stats.lock().unwrap();
        json!({ "moi": s.0, "participants": s.1, "ecartees": s.2 })
    }

    pub fn state(&self) -> J {
        let on = self.active.load(Ordering::SeqCst);
        json!({
            "t": "meeting_state", "on": on,
            "config": self.audio.shared.config(),
            "micro": *self.audio.shared.input_name.lock().unwrap(),
            "pc": *self.pc_device.lock().unwrap(),
            "aec": self.audio.shared.aec.load(Ordering::Relaxed),
            "since": if on { json!(*self.started_at.lock().unwrap()) } else { Value::Null },
            "stats": self.stats_json(),
            "voices": self.voices.lock().unwrap().available()
        })
    }

    pub fn start(self: &Arc<Self>) -> J {
        if self.active.load(Ordering::SeqCst) {
            return self.state();
        }
        *self.stats.lock().unwrap() = (0, 0, 0);
        self.pc_lines.lock().unwrap().clear();
        self.pc_queued.lock().unwrap().clear();
        self.voices.lock().unwrap().reset();
        let w = Arc::downgrade(self);
        let on_chunk: ChunkFn = Arc::new(move |source, audio, started_at| {
            if let Some(m) = w.upgrade() {
                if source == "participants" {
                    m.pc_queued.lock().unwrap().push(started_at);
                }
                if m.jobs.try_send(Job::Chunk(source, audio, started_at)).is_err() {
                    log("réunion : transcription en retard, un morceau est perdu");
                }
            }
        });
        *self.tracks.lock().unwrap() = vec![
            Track::new("moi", on_chunk.clone(), 0.004),
            // Son numérique du PC : pas de bruit de fond, seuil plus bas.
            Track::new("participants", on_chunk, 0.002),
        ];
        // Micro : dérivation du son 16 kHz de la capture.
        let w2 = Arc::downgrade(self);
        let shared = self.audio.shared.clone();
        self.audio.shared.taps.lock().unwrap().push((
            1,
            Box::new(move |processed: &[f32], raw: Option<&[f32]>| {
                let Some(m) = w2.upgrade() else { return };
                if !m.active.load(Ordering::SeqCst) {
                    return;
                }
                let use_processed = shared.aec.load(Ordering::Relaxed) || raw.is_none();
                let mut tracks = m.tracks.lock().unwrap();
                if let Some(t) = tracks.iter_mut().find(|t| t.name == "moi") {
                    t.feed(if use_processed { processed } else { raw.unwrap() });
                }
            }),
        ));
        // Son du PC : capture « loopback » de la sortie par défaut.
        let mut pc_error = None;
        *self.pc_device.lock().unwrap() = None;
        if self.audio.shared.file_input.is_some() {
            pc_error = Some("entrée fichier (tests)".to_string());
        } else {
            let w3 = Arc::downgrade(self);
            match self.audio.loopback(Some(Box::new(move |s16: &[f32]| {
                if let Some(m) = w3.upgrade() {
                    let mut tracks = m.tracks.lock().unwrap();
                    if let Some(t) = tracks.iter_mut().find(|t| t.name == "participants") {
                        t.feed(s16);
                    }
                }
            }))) {
                Ok(name) => *self.pc_device.lock().unwrap() = Some(name),
                Err(e) => pc_error = Some(e),
            }
        }
        self.active.store(true, Ordering::SeqCst);
        *self.started_at.lock().unwrap() = now();
        let mut state = self.state();
        if let Some(e) = &pc_error {
            state["pc_error"] = json!(e);
        }
        log(&format!(
            "réunion : enregistrement démarré · micro « {} » ({}, {}) · PC « {} »",
            self.audio.shared.input_name.lock().unwrap(),
            self.audio.shared.config(),
            if self.audio.shared.aec.load(Ordering::Relaxed) { "son nettoyé par l’anti-écho" } else { "son brut" },
            self.pc_device.lock().unwrap().clone().unwrap_or_else(|| format!("indisponible : {}", pc_error.clone().unwrap_or_default()))
        ));
        self.hub.emit(state.clone());
        state
    }

    pub fn stop(&self) -> J {
        if !self.active.swap(false, Ordering::SeqCst) {
            return self.state();
        }
        self.audio.shared.taps.lock().unwrap().retain(|(id, _)| *id != 1);
        let _ = self.audio.loopback(None);
        *self.pc_device.lock().unwrap() = None;
        for t in self.tracks.lock().unwrap().iter_mut() {
            t.flush();
        }
        let s = *self.stats.lock().unwrap();
        log(&format!("réunion : enregistrement arrêté ({} phrases du micro, {} du PC, {} écartées comme écho)", s.0, s.1, s.2));
        // Les derniers morceaux sont encore transcrits : « meeting_done » part une fois la file vide.
        let _ = self.jobs.try_send(Job::Done);
        let state = self.state();
        self.hub.emit(state.clone());
        state
    }

    /// Card de réunion : « Voix inconnue 2, c'est Marie ».
    pub fn name_voice(&self, cid: &str, name: &str, speaker_type: &str) -> J {
        let res = self.voices.lock().unwrap().name(cid, name, speaker_type);
        if res["ok"].as_bool().unwrap_or(false) {
            self.hub.emit(json!({ "t": "meeting_voice_named", "id": res["id"], "speaker": res["speaker"], "merged": res["merged"], "created": res["created"].as_bool().unwrap_or(false) }));
        } else {
            self.hub.emit(json!({ "t": "meeting_voice_named", "id": cid, "ok": false, "error": res["error"], "message": res["message"] }));
        }
        res
    }

    /// La piste du PC a-t-elle encore du son antérieur à `end` à transcrire ?
    fn pc_busy_before(&self, end: f64) -> bool {
        if let Some(t) = self.tracks.lock().unwrap().iter().find(|t| t.name == "participants") {
            if !t.frames.is_empty() && t.started_at < end {
                return true;
            }
        }
        self.pc_queued.lock().unwrap().iter().any(|s| *s < end)
    }

    fn worker(&self, rx: Receiver<Job>) {
        // Un morceau du micro attend que la piste du PC couvrant la même période soit transcrite :
        // s'il n'en est que l'écho, il est alors reconnu et écarté.
        let mut held: VecDeque<(Vec<f32>, f64)> = VecDeque::new();
        loop {
            match rx.recv_timeout(Duration::from_millis(300)) {
                Ok(Job::Done) => {
                    while let Some((a, s)) = held.pop_front() {
                        self.handle("moi", &a, s);
                    }
                    self.hub.emit(json!({ "t": "meeting_done", "stats": self.stats_json() }));
                }
                Ok(Job::Chunk(source, audio, started_at)) => {
                    if source == "moi" && self.pc_busy_before(started_at + audio.len() as f64 / SR as f64) {
                        held.push_back((audio, started_at));
                    } else {
                        self.handle(source, &audio, started_at);
                    }
                }
                Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
                Err(_) => return,
            }
            let t = now();
            let mut keep = VecDeque::new();
            while let Some((a, s)) = held.pop_front() {
                if !self.pc_busy_before(s + a.len() as f64 / SR as f64) || t - s > 45.0 {
                    self.handle("moi", &a, s);
                } else {
                    keep.push_back((a, s));
                }
            }
            held = keep;
        }
    }

    fn handle(&self, source: &'static str, audio: &[f32], started_at: f64) {
        let text = (self.transcribe)(audio);
        if source == "participants" {
            let mut q = self.pc_queued.lock().unwrap();
            if let Some(i) = q.iter().position(|s| *s == started_at) {
                q.remove(i);
            }
        }
        let Some(text) = text else { return };
        let t = now();
        self.pc_lines.lock().unwrap().retain(|(at, _)| t - at < ECHO_WINDOW);
        let said = (self.recent_said)();
        if !said.is_empty() && crate::filters::similar_to(&text, &said) {
            self.stats.lock().unwrap().2 += 1; // la voix de Jarvis elle-même, jouée par le PC
            return;
        }
        // Écho du PC dans le micro (les phrases très courtes sont gardées).
        if source == "moi" {
            let pc: Vec<String> = self.pc_lines.lock().unwrap().iter().map(|(_, s)| s.clone()).collect();
            if !pc.is_empty() && crate::filters::norm_words(&text).len() >= 3 && crate::filters::similar_to(&text, &[pc.join(" ")]) {
                self.stats.lock().unwrap().2 += 1;
                return;
            }
        }
        let mut msg = json!({ "t": "meeting_text", "source": source, "text": text, "at": started_at });
        if source == "participants" {
            self.pc_lines.lock().unwrap().push((t, text.clone()));
            let (who, event) = self.voices.lock().unwrap().assign(audio);
            if let Some(e) = event {
                self.hub.emit(e);
            }
            if let Some(w) = who {
                msg["speaker"] = w;
            }
            self.stats.lock().unwrap().1 += 1;
        } else {
            self.stats.lock().unwrap().0 += 1;
        }
        self.hub.emit(msg);
    }
}
