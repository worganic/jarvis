// Musique et médias (orchestrator/media.js) : pont vers le contrôle média de Windows
// (`jarvis-core media-control watch`, jarvis-native::media). Le processus n'est lancé qu'à la demande
// (widget ouvert, commande de musique) et s'arrête après une minute d'inactivité.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};

use serde_json::{json, Value};
use tokio::sync::oneshot;

use crate::js::{self, Timer, J};
use crate::proc::LineChild;

const IDLE_STOP_MS: u64 = 60000;
pub const COMMANDS: &[&str] = &["play", "pause", "pauseall", "toggle", "next", "previous", "stop", "status"];

pub struct MediaBridge {
    me: Weak<MediaBridge>,
    log: Rc<dyn Fn(String)>,
    on_state: Rc<dyn Fn(J)>,
    child: RefCell<Option<Rc<LineChild>>>,
    pub state: RefCell<J>,
    waiters: RefCell<Vec<oneshot::Sender<J>>>,
    keep_alive: Cell<bool>,
    idle: Timer,
}

impl MediaBridge {
    pub fn new(log: Rc<dyn Fn(String)>, on_state: Rc<dyn Fn(J)>) -> Rc<Self> {
        Rc::new_cyclic(|me| Self {
            me: me.clone(),
            log,
            on_state,
            child: RefCell::new(None),
            state: RefCell::new(json!({ "ok": true, "active": false })),
            waiters: RefCell::new(vec![]),
            keep_alive: Cell::new(false),
            idle: Timer::new(),
        })
    }

    pub fn available(&self) -> bool {
        true
    }

    fn ensure(&self) -> bool {
        if self.child.borrow().is_some() {
            return true;
        }
        let exe = std::env::current_exe().unwrap_or_default();
        let (m1, m2, m3) = (self.me.clone(), self.me.clone(), self.me.clone());
        let child = LineChild::spawn(
            &exe,
            &["media-control".into(), "watch".into()],
            &[],
            move |line| {
                if let (Some(s), Ok(v)) = (m1.upgrade(), serde_json::from_str::<J>(&line)) {
                    s.update(v);
                }
            },
            move |err| {
                if let Some(s) = m2.upgrade() {
                    (s.log)(format!("[médias] {}", js::slice_chars(err.trim(), 300)));
                }
            },
            move |_| {
                if let Some(s) = m3.upgrade() {
                    s.child.borrow_mut().take();
                    let st = s.state.borrow().clone();
                    for w in s.waiters.borrow_mut().drain(..) {
                        let _ = w.send(st.clone());
                    }
                }
            },
        );
        match child {
            Ok(c) => {
                crate::proc::tie_to_core(c.pid);
                *self.child.borrow_mut() = Some(c);
                self.touch();
                true
            }
            Err(e) => {
                (self.log)(format!("[médias] lancement impossible : {e}"));
                false
            }
        }
    }

    fn update(&self, state: J) {
        *self.state.borrow_mut() = state.clone();
        for w in self.waiters.borrow_mut().drain(..) {
            let _ = w.send(state.clone());
        }
        (self.on_state)(state);
    }

    pub fn touch(&self) {
        let me = self.me.clone();
        self.idle.set(IDLE_STOP_MS, move || {
            if let Some(s) = me.upgrade() {
                if !s.keep_alive.get() {
                    s.stop();
                }
            }
        });
    }

    pub fn set_keep_alive(&self, on: bool) {
        self.keep_alive.set(on);
        if on {
            self.ensure();
        }
        self.touch();
    }

    /// Envoie une commande et attend l'état suivant (au plus 2,5 s).
    pub async fn command(&self, cmd: &str, app: Option<&str>) -> J {
        let clean = cmd.to_lowercase();
        let app = app.map(|a| a.to_lowercase().trim().to_string()).filter(|a| !a.is_empty());
        if !COMMANDS.contains(&clean.as_str()) {
            return json!({ "ok": false, "error": format!("commande inconnue : {cmd}") });
        }
        if !self.ensure() {
            return json!({ "ok": false, "error": "contrôle des médias indisponible" });
        }
        self.touch();
        let (tx, rx) = oneshot::channel();
        self.waiters.borrow_mut().push(tx);
        let child = self.child.borrow().clone();
        let payload = match &app {
            Some(a) => format!("{clean}:{a}"),
            None => clean,
        };
        let written = match child {
            Some(c) => c.write_line(&payload).await,
            None => false,
        };
        if !written {
            return json!({ "ok": false, "error": "contrôle des médias indisponible" });
        }
        match tokio::time::timeout(std::time::Duration::from_millis(2500), rx).await {
            Ok(Ok(s)) => s,
            _ => self.state.borrow().clone(),
        }
    }

    pub async fn status(&self, app: Option<&str>) -> J {
        self.command("status", app).await
    }

    pub fn stop(&self) {
        self.idle.clear();
        if let Some(c) = self.child.borrow_mut().take() {
            let c2 = c.clone();
            js::spawn(async move {
                c2.write_line("quit").await;
            });
            js::set_timeout(500, move || c.kill());
        }
    }
}

/// Phrase courte pour « qu'est-ce qu'on écoute ? ».
pub fn describe_track(state: &J) -> Option<String> {
    if !js::gb(state, "active") || !js::truthy(&state["title"]) {
        return None;
    }
    let artist = js::gs(state, "artist");
    let who = if artist.is_empty() { String::new() } else { format!(" de {artist}") };
    let verb = if js::gs(state, "status") == "playing" { "C'est" } else { "En pause :" };
    Some(format!("{verb} {}{who}.", js::gs(state, "title")))
}

#[allow(dead_code)]
fn _v(_: Value) {}
