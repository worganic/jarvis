// Mélangeur audio du module Audio : pilote `jarvis-core audio-mixer serve` (jarvis-native › audio::mixer_serve),
// processus durable lancé seulement quand une page HUD affiche la card. Bail : une page visible renouvelle sa
// demande toutes les 10 s ; sans nouvelle pendant 25 s, le processus est fermé.

use std::cell::{Cell, RefCell};
use std::collections::VecDeque;
use std::rc::{Rc, Weak};

use serde_json::{json, Value};
use tokio::sync::oneshot;

use jarvis_core::js::{self, Timer, J};
use jarvis_core::proc::LineChild;

const LEASE_MS: i64 = 25000;
const REPLY_TIMEOUT_MS: u64 = 4000;

pub struct AudioMixer {
    me: Weak<AudioMixer>,
    emit: Rc<dyn Fn(J)>,
    log: Rc<dyn Fn(String)>,
    child: RefCell<Option<Rc<LineChild>>>,
    waiters: RefCell<VecDeque<(u64, oneshot::Sender<J>)>>,
    seq: Cell<u64>,
    lease_until: Cell<i64>,
    meter_until: Cell<i64>,
    meter_on: Cell<bool>,
    lease_timer: Timer,
    last: RefCell<Option<J>>,
}

impl AudioMixer {
    pub fn new(emit: Rc<dyn Fn(J)>, log: Rc<dyn Fn(String)>) -> Rc<Self> {
        Rc::new_cyclic(|me| Self {
            me: me.clone(),
            emit,
            log,
            child: RefCell::new(None),
            waiters: RefCell::new(VecDeque::new()),
            seq: Cell::new(0),
            lease_until: Cell::new(0),
            meter_until: Cell::new(0),
            meter_on: Cell::new(false),
            lease_timer: Timer::new(),
            last: RefCell::new(None),
        })
    }

    fn start(&self) -> bool {
        if self.child.borrow().is_some() {
            return true;
        }
        let exe = std::env::current_exe().unwrap_or_default();
        let (m1, m2) = (self.me.clone(), self.me.clone());
        match LineChild::spawn(
            &exe,
            &["audio-mixer".into(), "serve".into()],
            &[],
            move |line| {
                if let Some(s) = m1.upgrade() {
                    s.on_line(&line);
                }
            },
            |_| {},
            move |_| {
                if let Some(s) = m2.upgrade() {
                    s.child.borrow_mut().take();
                    for (_, w) in s.waiters.borrow_mut().drain(..) {
                        let _ = w.send(json!({ "ok": false, "error": "mélangeur audio arrêté" }));
                    }
                }
            },
        ) {
            Ok(c) => {
                jarvis_core::proc::tie_to_core(c.pid);
                *self.child.borrow_mut() = Some(c);
                true
            }
            Err(e) => {
                (self.log)(format!("[audio] mélangeur indisponible : {e}"));
                false
            }
        }
    }

    pub fn stop(&self) {
        self.lease_timer.clear();
        self.lease_until.set(0);
        self.meter_until.set(0);
        self.meter_on.set(false);
        if let Some(c) = self.child.borrow_mut().take() {
            let c2 = c.clone();
            js::spawn(async move {
                c2.write_line("quit").await;
                c2.close_stdin();
            });
            js::set_timeout(1500, move || c.kill());
        }
    }

    fn on_line(&self, line: &str) {
        let Ok(msg) = serde_json::from_str::<J>(line) else { return };
        if msg["t"] == "peak" {
            (self.emit)(json!({ "type": "audio_peak", "out": msg["out"], "in": msg["in"], "l": msg["l"], "r": msg["r"], "al": msg["al"], "ar": msg["ar"], "af": msg["af"] }));
            return;
        }
        if msg["t"] == "state" {
            *self.last.borrow_mut() = Some(msg.clone());
        }
        if let Some((_, w)) = self.waiters.borrow_mut().pop_front() {
            let _ = w.send(msg.clone());
        }
        if msg["t"] == "state" {
            let mut ev = json!({ "type": "audio_mixer" });
            js::assign(&mut ev, msg);
            (self.emit)(ev);
        }
    }

    async fn request(&self, line: &str) -> J {
        if !self.start() {
            return json!({ "ok": false, "error": "mélangeur audio indisponible" });
        }
        let id = self.seq.get() + 1;
        self.seq.set(id);
        let (tx, rx) = oneshot::channel();
        self.waiters.borrow_mut().push_back((id, tx));
        let child = self.child.borrow().clone();
        if let Some(c) = child {
            c.write_line(line).await;
        }
        match tokio::time::timeout(std::time::Duration::from_millis(REPLY_TIMEOUT_MS), rx).await {
            Ok(Ok(v)) => v,
            _ => {
                self.waiters.borrow_mut().retain(|(i, _)| *i != id);
                json!({ "ok": false, "error": "le mélangeur audio ne répond pas" })
            }
        }
    }

    fn send(&self, line: &str) {
        let child = self.child.borrow().clone();
        if let Some(c) = child {
            let l = line.to_string();
            js::spawn(async move {
                c.write_line(&l).await;
            });
        }
    }

    /// Page visible (on) : suivi des changements ; meter : niveaux de crête en direct.
    pub async fn watch(&self, on: bool, meter: bool) -> J {
        if !on {
            self.meter_until.set(0);
            self.set_meter(false);
            return self.last.borrow().clone().unwrap_or(json!({ "ok": true }));
        }
        let now = js::now_ms();
        self.lease_until.set(now + LEASE_MS);
        self.meter_until.set(if meter { now + LEASE_MS } else { 0 });
        let started = self.start();
        if !started {
            return json!({ "ok": false, "error": "mélangeur audio indisponible" });
        }
        let fut = self.request("state");
        self.send("watch 1");
        self.set_meter(meter);
        self.arm_lease();
        fut.await
    }

    fn set_meter(&self, on: bool) {
        if on == self.meter_on.get() {
            return;
        }
        self.meter_on.set(on);
        self.send(&format!("meter {}", if on { 1 } else { 0 }));
    }

    fn arm_lease(&self) {
        let me = self.me.clone();
        self.lease_timer.set(2000, move || {
            let Some(s) = me.upgrade() else { return };
            let now = js::now_ms();
            if now >= s.lease_until.get() {
                s.stop();
                return;
            }
            if now >= s.meter_until.get() {
                s.set_meter(false);
            }
            s.arm_lease();
        });
    }

    /// Card musique : suit l'application qui joue (son volume et ses niveaux à elle, pas ceux de Windows entier).
    /// Le mélangeur l'oublie s'il s'arrête (fin du bail) : la card la renvoie à chaque renouvellement.
    pub async fn app(&self, name: &str) -> J {
        let clean: String = name.chars().filter(|c| c.is_ascii_alphanumeric() || " ._!-".contains(*c)).take(120).collect();
        // La réponse « app » ne se mélange pas à l'état : elle a sa propre forme ({ t: "app", found, volume }).
        self.request(&format!("app {clean}")).await
    }

    /// Volume propre de l'application suivie (mélangeur de volume de Windows), en %.
    pub async fn app_volume(&self, value: &J) -> J {
        let pct = js::num(value).unwrap_or(-1.0);
        if !(0.0..=100.0).contains(&pct) {
            return json!({ "ok": false, "error": "volume invalide" });
        }
        self.request(&format!("appvol {}", pct.round())).await
    }

    pub async fn set_volume(&self, kind: &str, value: &J) -> J {
        let pct = js::num(value).unwrap_or(0.0).clamp(0.0, 100.0).round() as i64;
        self.request(&format!("vol {} {pct}", if kind == "in" { "in" } else { "out" })).await
    }

    /// on : true / false / « toggle ».
    pub async fn set_mute(&self, kind: &str, on: &J) -> J {
        let v = if on == &json!("toggle") { "toggle".to_string() } else if js::truthy(on) { "1".into() } else { "0".into() };
        self.request(&format!("mute {} {v}", if kind == "in" { "in" } else { "out" })).await
    }

    /// Identifiant Windows d'un périphérique : « {0.0.0.00000000}.{guid} ».
    pub async fn set_default(&self, id: &str) -> J {
        let clean = id.trim();
        if !jarvis_core::re!(r"(?i)^\{[\w.-]+\}\.\{[0-9a-f-]{36}\}$").is_match(clean) {
            return json!({ "ok": false, "error": "périphérique invalide" });
        }
        self.request(&format!("default {clean}")).await
    }
}

#[allow(dead_code)]
fn _v(_: Value) {}
