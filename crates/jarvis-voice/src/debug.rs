// Enregistrement de diagnostic (option « debugAudio », AudioDebug de voice/engine.py) : chaque phrase
// captée est gardée dans data/voice-debug en trois WAV 16 kHz (traite, brut, pc) ; index.jsonl note
// la phrase, le son donné à Whisper et la décision. 300 phrases au plus ; écriture dans un fil à part.

use std::collections::VecDeque;
use std::io::Write;
use std::path::PathBuf;
use std::sync::mpsc::SyncSender;
use std::sync::Mutex;
use std::time::Instant;

use serde_json::json;

use crate::audio::Decimator;
use crate::util::{log, J};

const KEEP: usize = 300;
const SR: usize = 16000;

type Job = (Option<String>, Option<Vec<f32>>, Option<Vec<f32>>, Option<Vec<f32>>, J);

pub struct AudioDebug {
    pub folder: PathBuf,
    refbuf: Mutex<(VecDeque<(Instant, Vec<f32>)>, Decimator)>,
    jobs: SyncSender<Job>,
}

impl AudioDebug {
    pub fn new(folder: PathBuf) -> Self {
        let _ = std::fs::create_dir_all(&folder);
        let (tx, rx) = std::sync::mpsc::sync_channel::<Job>(50);
        let f2 = folder.clone();
        std::thread::Builder::new()
            .name("diagnostic-audio".into())
            .spawn(move || {
                let index = f2.join("index.jsonl");
                while let Ok((name, audio, raw, pc, info)) = rx.recv() {
                    if let Some(name) = &name {
                        for (tag, data) in [("traite", &audio), ("brut", &raw), ("pc", &pc)] {
                            if let Some(d) = data.as_ref().filter(|d| !d.is_empty()) {
                                let _ = crate::util::write_wav(&f2.join(format!("{name}-{tag}.wav")), d, SR);
                            }
                        }
                        prune(&f2);
                    }
                    if std::fs::metadata(&index).map(|m| m.len() > 2 * 1024 * 1024).unwrap_or(false) {
                        let _ = std::fs::rename(&index, f2.join("index.jsonl.1"));
                    }
                    let mut line = json!({ "t": chrono::Local::now().format("%H:%M:%S").to_string() });
                    if let (Some(o), Some(i)) = (line.as_object_mut(), info.as_object()) {
                        for (k, v) in i {
                            o.insert(k.clone(), v.clone());
                        }
                    }
                    match std::fs::OpenOptions::new().create(true).append(true).open(&index) {
                        Ok(mut f) => {
                            let _ = writeln!(f, "{line}");
                        }
                        Err(e) => log(&format!("diagnostic audio : écriture impossible ({e})")),
                    }
                }
            })
            .ok();
        Self { folder, refbuf: Mutex::new((VecDeque::new(), Decimator::default())), jobs: tx }
    }

    /// Son du PC (48 kHz mono) : gardé en 16 kHz pour la fenêtre de chaque phrase.
    pub fn add_ref(&self, mono48: &[f32]) {
        let mut r = self.refbuf.lock().unwrap();
        let s = r.1.process(mono48);
        r.0.push_back((Instant::now(), s));
        while r.0.len() > 4000 {
            r.0.pop_front();
        }
    }

    fn ref_window(&self, seconds: f64) -> Option<Vec<f32>> {
        let r = self.refbuf.lock().unwrap();
        let since = std::time::Duration::from_secs_f64(seconds + 0.1);
        let chunks: Vec<f32> = r.0.iter().filter(|(t, _)| t.elapsed() <= since).flat_map(|(_, s)| s.iter().copied()).collect();
        (!chunks.is_empty()).then_some(chunks)
    }

    pub fn save(&self, segment: u64, audio: &[f32], raw: Option<&[f32]>, info: J) {
        let pc = self.ref_window(audio.len() as f64 / SR as f64);
        let name = format!("{}-{segment:04}", chrono::Local::now().format("%Y%m%d-%H%M%S"));
        let mut i = json!({ "segment": segment, "fichier": name });
        if let (Some(o), Some(src)) = (i.as_object_mut(), info.as_object()) {
            for (k, v) in src {
                o.insert(k.clone(), v.clone());
            }
        }
        let _ = self.jobs.try_send((Some(name), Some(audio.to_vec()), raw.map(|r| r.to_vec()), pc, i));
    }

    pub fn note(&self, segment: u64, fields: J) {
        let mut i = json!({ "segment": segment });
        if let (Some(o), Some(src)) = (i.as_object_mut(), fields.as_object()) {
            for (k, v) in src {
                o.insert(k.clone(), v.clone());
            }
        }
        let _ = self.jobs.try_send((None, None, None, None, i));
    }
}

fn prune(folder: &PathBuf) {
    let mut stems: Vec<String> = std::fs::read_dir(folder)
        .map(|rd| rd.flatten().filter_map(|e| e.file_name().to_str().and_then(|n| n.strip_suffix("-traite.wav")).map(String::from)).collect())
        .unwrap_or_default();
    stems.sort();
    if stems.len() > KEEP {
        for stem in &stems[..stems.len() - KEEP] {
            for tag in ["traite", "brut", "pc"] {
                let _ = std::fs::remove_file(folder.join(format!("{stem}-{tag}.wav")));
            }
        }
    }
}
