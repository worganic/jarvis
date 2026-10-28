// Superviseur du moteur vocal (core/voice-supervisor.js de la v3) : un seul processus
// (jarvis-voice.exe, qui remplace voice/engine.py), lancé, surveillé et relancé par le noyau.
//
// Relance automatique avec délai croissant (1,5 s → 10 s). Après 5 échecs en 2 minutes, le moteur
// n'est plus relancé automatiquement : Jarvis passe en mode texte et le signale (un redémarrage
// manuel — menu, administration — remet les compteurs à zéro).

use std::cell::{Cell, RefCell};
use std::io::Write;
use std::path::PathBuf;
use std::rc::{Rc, Weak};

use serde_json::json;

use crate::js::{self, Timer, J};
use crate::proc::LineChild;

const MAX_FAILURES: usize = 5;
const FAILURE_WINDOW: i64 = 120000;
const EXIT_PORT_TAKEN: i32 = 3;

pub struct VoiceSupervisor {
    me: RefCell<Weak<VoiceSupervisor>>,
    exe: PathBuf,
    err_file: PathBuf,
    pub port: u16,
    pub token: String,
    get_config: Box<dyn Fn() -> J>,
    get_session: Box<dyn Fn() -> J>,
    log: Rc<dyn Fn(String)>,
    pub on_status: RefCell<Option<Rc<dyn Fn(J)>>>,
    pub child: RefCell<Option<Rc<LineChild>>>,
    pub paused: Cell<bool>,
    stopping: Cell<bool>,
    failures: RefCell<Vec<i64>>,
    restart_timer: Timer,
    pub ready: Cell<bool>,
    pub gave_up: RefCell<Option<String>>,
    started_at: Cell<i64>,
    active: Cell<bool>,
    seq: Cell<u64>,
}

fn random_token() -> String {
    let mut b = [0u8; 24];
    let _ = getrandom::getrandom(&mut b);
    hex::encode(b)
}

impl VoiceSupervisor {
    pub fn new(port: u16, log_dir: PathBuf, get_config: Box<dyn Fn() -> J>, get_session: Box<dyn Fn() -> J>, log: Rc<dyn Fn(String)>) -> Rc<Self> {
        let exe = crate::paths::bin_dir().join("jarvis-voice.exe");
        let s = Rc::new(Self {
            me: RefCell::new(Weak::new()),
            exe,
            err_file: log_dir.join("engine-stderr.log"),
            port,
            token: random_token(),
            get_config,
            get_session,
            log,
            on_status: RefCell::new(None),
            child: RefCell::new(None),
            paused: Cell::new(false),
            stopping: Cell::new(false),
            failures: RefCell::new(vec![]),
            restart_timer: Timer::new(),
            ready: Cell::new(false),
            gave_up: RefCell::new(None),
            started_at: Cell::new(0),
            active: Cell::new(false),
            seq: Cell::new(0),
        });
        *s.me.borrow_mut() = Rc::downgrade(&s);
        s
    }

    fn status_event(&self, st: J) {
        let cb = self.on_status.borrow().clone();
        if let Some(f) = cb {
            f(st);
        }
    }

    pub fn missing(&self) -> Option<String> {
        (!self.exe.is_file()).then(|| format!("moteur vocal absent ({})", self.exe.display()))
    }

    pub fn pid(&self) -> Option<u32> {
        self.child.borrow().as_ref().map(|c| c.pid)
    }

    /// Options du moteur : réglages « engine » (et « livekit » de la v1, repris tels quels).
    pub fn engine_options(&self) -> J {
        let v = (self.get_config)();
        let pick = |k: &str, d: J| -> J {
            if !v["engine"][k].is_null() {
                v["engine"][k].clone()
            } else if !v["livekit"][k].is_null() {
                v["livekit"][k].clone()
            } else {
                d
            }
        };
        let mid = js::num(&pick("minInterruptionDuration", json!(0.25))).unwrap_or(0.25).min(0.6);
        json!({
            "echoCancellation": pick("echoCancellation", json!(true)),
            "noiseSuppression": pick("noiseSuppression", json!(true)),
            "autoGainControl": pick("autoGainControl", json!(true)),
            "systemAudioReference": pick("systemAudioReference", json!(true)),
            "micDevice": pick("micDevice", json!("")),
            "outDevice": pick("outDevice", json!("")),
            "minEndpointingDelay": pick("minEndpointingDelay", json!(0.45)),
            "maxEndpointingDelay": pick("maxEndpointingDelay", json!(2.5)),
            "allowInterruptions": pick("allowInterruptions", json!(true)),
            "minInterruptionDuration": mid,
            "sttModel": pick("sttModel", json!("turbo")),
            "sttBeam": pick("sttBeam", json!(2)),
            "acousticWake": pick("acousticWake", json!(true)),
            "wakeEngine": pick("wakeEngine", json!("phonetique")),
            "wakeModel": pick("wakeModel", json!("jarvis.onnx")),
            "wakeThreshold": pick("wakeThreshold", json!(0.5)),
            "ttsFallback": pick("ttsFallback", json!(true)),
            "debugAudio": pick("debugAudio", json!(false)),
            "sttRawAudio": pick("sttRawAudio", json!(true))
        })
    }

    pub fn start(&self) {
        if js::env("JARVIS_NO_VOICE").is_some() || self.paused.get() || self.child.borrow().is_some() {
            return;
        }
        if let Some(missing) = self.missing() {
            (self.log)(format!("[voix] moteur vocal non démarré : {missing}"));
            self.status_event(json!({ "state": "unavailable", "reason": missing }));
            return;
        }
        self.stopping.set(false);
        self.ready.set(false);
        self.started_at.set(js::now_ms());
        self.active.set(true);
        let sess = (self.get_session)();
        let mut envs = vec![
            ("JARVIS_ENGINE_PORT".to_string(), self.port.to_string()),
            ("JARVIS_ENGINE_TOKEN".to_string(), self.token.clone()),
            ("JARVIS_ENGINE_OPTIONS".to_string(), self.engine_options().to_string()),
        ];
        // Reconnaissance vocale sur le processeur : rend toute la mémoire vidéo au bureau et aux navigateurs.
        if (self.get_config)()["sttCpu"] == json!(true) {
            envs.push(("JARVIS_STT_CPU".into(), "1".into()));
        }
        let voice = js::gs(&sess, "voice");
        if !voice.is_empty() {
            envs.push(("JARVIS_VOICE".into(), voice));
        }
        let rate = js::gs(&sess, "voiceRate");
        if !rate.is_empty() {
            envs.push(("JARVIS_RATE".into(), rate));
        }
        let seq = self.seq.get() + 1;
        self.seq.set(seq);
        let (w1, w2, w3) = (self.me.borrow().clone(), self.me.borrow().clone(), self.me.borrow().clone());
        let spawned = LineChild::spawn_in(
            Some(&crate::paths::root()),
            &self.exe,
            &[],
            &envs,
            move |line| {
                let Some(s) = w1.upgrade() else { return };
                (s.log)(format!("[voix] {line}"));
                if line.contains("moteur vocal prêt") {
                    s.ready.set(true);
                    s.status_event(json!({ "state": "ready" }));
                }
            },
            move |line| {
                let Some(s) = w2.upgrade() else { return };
                // Sortie d'erreur complète, gardée pour le diagnostic.
                if std::fs::metadata(&s.err_file).map(|m| m.len() > 1024 * 1024).unwrap_or(false) {
                    let _ = std::fs::rename(&s.err_file, s.err_file.with_extension("log.1"));
                }
                if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(&s.err_file) {
                    let _ = writeln!(f, "[{}] {line}", js::iso_now());
                }
                if crate::re!(r"Error|panicked|Exception|erreur").is_match(&line) {
                    (s.log)(format!("[voix] {line}"));
                    crate::logger::logger().error("voix:moteur", &js::slice_chars(&line, 600));
                }
            },
            move |code| {
                if let Some(s) = w3.upgrade() {
                    s.on_exit(seq, code.unwrap_or(-1), None);
                }
            },
        );
        match spawned {
            Ok(child) => {
                crate::proc::tie_to_core(child.pid);
                *self.child.borrow_mut() = Some(child);
                self.status_event(json!({ "state": "starting" }));
            }
            Err(e) => {
                self.status_event(json!({ "state": "starting" }));
                self.on_exit(seq, -1, Some(e));
            }
        }
    }

    fn on_exit(&self, seq: u64, code: i32, reason: Option<String>) {
        if seq != self.seq.get() {
            return;
        }
        self.child.borrow_mut().take();
        self.ready.set(false);
        self.active.set(false);
        if self.stopping.get() {
            return;
        }
        if code == EXIT_PORT_TAKEN {
            (self.log)(format!("[voix] port {} déjà pris : un autre moteur vocal tourne", self.port));
        }
        let now = js::now_ms();
        let attempt = {
            let mut f = self.failures.borrow_mut();
            f.retain(|t| now - t < FAILURE_WINDOW);
            f.push(now);
            f.len()
        };
        if attempt >= MAX_FAILURES {
            let why = format!(
                "moteur vocal arrêté {MAX_FAILURES} fois en 2 minutes (code {code}{})",
                reason.map(|r| format!(", {r}")).unwrap_or_default()
            );
            (self.log)(format!("[voix] {why} : mode texte"));
            crate::logger::logger().error("voix:moteur", &why);
            *self.gave_up.borrow_mut() = Some(why.clone());
            self.status_event(json!({ "state": "failed", "reason": why }));
            return;
        }
        let delay = (1500.0 * 1.6f64.powi(attempt as i32 - 1)).min(10000.0);
        (self.log)(format!("[voix] moteur arrêté (code {code}) : relance dans {} ms (tentative {attempt})", delay.round()));
        self.status_event(json!({ "state": "restarting", "attempt": attempt, "delay": delay }));
        let w = self.me.borrow().clone();
        self.restart_timer.set(delay as u64, move || {
            if let Some(s) = w.upgrade() {
                s.start();
            }
        });
    }

    pub fn stop(&self) {
        self.stopping.set(true);
        self.restart_timer.clear();
        self.seq.set(self.seq.get() + 1);
        if let Some(c) = self.child.borrow_mut().take() {
            c.kill();
        }
        self.ready.set(false);
        self.active.set(false);
    }

    pub fn restart(&self) {
        self.stop();
        self.failures.borrow_mut().clear();
        *self.gave_up.borrow_mut() = None;
        let w = self.me.borrow().clone();
        js::set_timeout(600, move || {
            if let Some(s) = w.upgrade() {
                s.start();
            }
        });
    }

    /// Optimisation › écoute orale coupée : le moteur est arrêté (Whisper libère la mémoire vidéo).
    pub fn set_paused(&self, paused: bool) {
        if paused == self.paused.get() {
            return;
        }
        self.paused.set(paused);
        if paused {
            self.stop();
            self.status_event(json!({ "state": "paused" }));
            (self.log)("[voix] écoute orale désactivée (optimisation) : moteur vocal arrêté".into());
        } else {
            (self.log)("[voix] écoute orale réactivée".into());
            self.failures.borrow_mut().clear();
            *self.gave_up.borrow_mut() = None;
            self.start();
        }
    }

    /// Lu par l'administration.
    pub fn status(&self) -> J {
        json!({
            "backend": if self.active.get() { json!("engine") } else { J::Null },
            "ready": self.ready.get(),
            "paused": self.paused.get(),
            "fallback": *self.gave_up.borrow(),
            "pid": self.pid(),
            "missing": self.missing()
        })
    }
}
