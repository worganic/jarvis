// Corpus vocal de l'utilisateur (voice/corpus.py de la v3).
//
// Enregistre, à la demande explicite de l'utilisateur (administration → Voix → Corpus), les segments
// de parole tels que Whisper les recevrait. Ils servent uniquement à mesurer et régler la
// reconnaissance, restent sur ce PC (data/voice-corpus) et ne sont jamais transmis à Jarvis.
//   - phrase  : l'utilisateur lit une phrase affichée ; les segments captés jusqu'à 1,5 s de
//               silence forment un seul fichier ;
//   - negatif : pendant N secondes, tout ce qui est capté est gardé, un fichier par segment.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{json, Value};

use crate::util::{data_dir, log, J};

const SR: usize = 16000;
const PHRASE_GAP: f64 = 1.5;
const PHRASE_TIMEOUT: f64 = 20.0;

fn corpus_dir() -> PathBuf {
    std::env::var("JARVIS_VOICE_CORPUS").map(PathBuf::from).unwrap_or_else(|_| data_dir().join("voice-corpus"))
}

fn write_json(path: &PathBuf, data: &J) {
    if let Some(d) = path.parent() {
        let _ = std::fs::create_dir_all(d);
    }
    let tmp = path.with_extension("json.tmp");
    if std::fs::write(&tmp, crate::util::json_indent1(data)).is_ok() {
        let _ = std::fs::rename(&tmp, path);
    }
}

fn load_manifest() -> J {
    std::fs::read_to_string(corpus_dir().join("manifest.json"))
        .ok()
        .and_then(|s| serde_json::from_str::<J>(&s).ok())
        .filter(|v| v["items"].is_array())
        .unwrap_or_else(|| json!({ "items": [] }))
}

struct State {
    mode: Option<&'static str>,
    item: J,
    started: Instant,
    last: Option<Instant>,
    until: Instant,
    audio: Vec<Vec<f32>>,
    count: usize,
}

pub struct CorpusRecorder {
    st: Mutex<State>,
}

impl CorpusRecorder {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            st: Mutex::new(State { mode: None, item: Value::Null, started: Instant::now(), last: None, until: Instant::now(), audio: vec![], count: 0 }),
        })
    }

    fn status(&self, st: &State, state: &str, extra: J) {
        let mut data = json!({ "mode": st.mode, "state": state, "item": st.item, "at": chrono::Utc::now().timestamp_millis() as f64 / 1000.0 });
        if let (Some(d), Some(e)) = (data.as_object_mut(), extra.as_object()) {
            for (k, v) in e {
                d.insert(k.clone(), v.clone());
            }
        }
        write_json(&corpus_dir().join("status.json"), &data);
    }

    fn arm(self: &Arc<Self>, seconds: f64) {
        let me = Arc::downgrade(self);
        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs_f64(seconds));
            if let Some(m) = me.upgrade() {
                m.active();
            }
        });
    }

    pub fn start_phrase(self: &Arc<Self>, id: &str, text: &str, config: &str) {
        {
            let mut st = self.st.lock().unwrap();
            st.mode = Some("phrase");
            st.item = json!({ "id": id, "text": text, "config": config, "kind": "phrase" });
            st.started = Instant::now();
            st.last = None;
            st.audio.clear();
            self.status(&st, "ecoute", json!({}));
        }
        log(&format!("corpus : lecture attendue pour « {} » ({config})", text.chars().take(60).collect::<String>()));
        // Sans aucune parole, rien n'appellerait active() : un minuteur clôt l'attente.
        self.arm(PHRASE_TIMEOUT + 0.2);
    }

    pub fn start_negative(self: &Arc<Self>, seconds: f64, config: &str) {
        let secs = seconds.clamp(10.0, 300.0);
        {
            let mut st = self.st.lock().unwrap();
            st.mode = Some("negatif");
            st.item = json!({ "config": config, "kind": "negatif" });
            st.started = Instant::now();
            st.until = st.started + Duration::from_secs_f64(secs);
            st.count = 0;
            self.status(&st, "ecoute", json!({}));
        }
        log(&format!("corpus : série négative pendant {secs:.0} s ({config})"));
        self.arm(secs + 0.2);
    }

    pub fn stop(&self) {
        let mut st = self.st.lock().unwrap();
        st.mode = None;
        st.item = Value::Null;
        st.audio.clear();
        self.status(&st, "arret", json!({}));
    }

    /// Le segment courant doit-il être gardé pour le corpus (et donc écarté de Jarvis) ?
    pub fn active(&self) -> bool {
        let mut st = self.st.lock().unwrap();
        match st.mode {
            Some("phrase") => {
                if st.audio.is_empty() && st.started.elapsed().as_secs_f64() > PHRASE_TIMEOUT {
                    st.mode = None;
                    self.status(&st, "expire", json!({}));
                    return false;
                }
                if !st.audio.is_empty() && st.last.map(|l| l.elapsed().as_secs_f64() > PHRASE_GAP).unwrap_or(false) {
                    self.finish_phrase(&mut st);
                    return false;
                }
                true
            }
            Some("negatif") => {
                if Instant::now() > st.until {
                    st.mode = None;
                    let c = st.count;
                    self.status(&st, "termine", json!({ "segments": c }));
                    log(&format!("corpus : série négative terminée, {c} segment(s)"));
                    return false;
                }
                true
            }
            _ => false,
        }
    }

    pub fn save(self: &Arc<Self>, audio: &[f32]) {
        let mut st = self.st.lock().unwrap();
        match st.mode {
            Some("phrase") => {
                st.audio.push(audio.to_vec());
                st.last = Some(Instant::now());
                let dur = st.audio.iter().map(|a| a.len()).sum::<usize>() as f64 / SR as f64;
                self.status(&st, "capte", json!({ "duree": (dur * 100.0).round() / 100.0 }));
                drop(st);
                // Clôture différée : si aucun autre segment n'arrive, la phrase est écrite.
                let me = Arc::downgrade(self);
                std::thread::spawn(move || {
                    std::thread::sleep(Duration::from_secs_f64(PHRASE_GAP + 0.1));
                    if let Some(m) = me.upgrade() {
                        let mut st = m.st.lock().unwrap();
                        if st.mode == Some("phrase") && !st.audio.is_empty() && st.last.map(|l| l.elapsed().as_secs_f64() >= PHRASE_GAP).unwrap_or(false) {
                            m.finish_phrase(&mut st);
                        }
                    }
                });
            }
            Some("negatif") => {
                st.count += 1;
                let mut item = st.item.clone();
                item["id"] = json!(format!("neg-{}", chrono::Utc::now().timestamp_millis()));
                item["text"] = json!("");
                self.write(&item, audio);
                let c = st.count;
                self.status(&st, "capte", json!({ "segments": c }));
            }
            _ => {}
        }
    }

    fn finish_phrase(&self, st: &mut State) {
        let gap = vec![0f32; SR / 5];
        let mut audio = vec![];
        for (i, a) in st.audio.iter().enumerate() {
            if i > 0 {
                audio.extend_from_slice(&gap);
            }
            audio.extend_from_slice(a);
        }
        let item = st.item.clone();
        self.write(&item, &audio);
        st.mode = None;
        st.audio.clear();
        self.status(st, "enregistre", json!({ "id": item["id"] }));
    }

    fn write(&self, item: &J, audio: &[f32]) {
        let dir = corpus_dir();
        let kind = item["kind"].as_str().unwrap_or("phrase");
        let sub = if kind == "negatif" { "negatif".to_string() } else { item["config"].as_str().unwrap_or("autre").to_string() };
        let rel = format!("{sub}/{}.wav", item["id"].as_str().unwrap_or("x"));
        let _ = crate::util::write_wav(&dir.join(&rel), audio, SR);
        let mut manifest = load_manifest();
        let items = manifest["items"].as_array_mut().unwrap();
        // Une phrase relue remplace la précédente prise de la même configuration.
        if kind == "phrase" {
            items.retain(|i| !(i["id"] == item["id"] && i["config"] == item["config"]));
        }
        let mut entry = item.clone();
        entry["file"] = json!(rel);
        entry["duration"] = json!(((audio.len() as f64 / SR as f64) * 100.0).round() / 100.0);
        entry["rms"] = json!(((crate::util::rms(audio) as f64) * 10000.0).round() / 10000.0);
        entry["at"] = json!(chrono::Local::now().format("%Y-%m-%dT%H:%M:%S").to_string());
        items.push(entry);
        write_json(&dir.join("manifest.json"), &manifest);
    }
}
