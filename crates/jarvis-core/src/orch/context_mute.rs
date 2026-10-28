// Sourdine contextuelle (orchestrator/context-mute.js) : coupe automatiquement la voix de Jarvis quand
// un jeu, une visioconférence ou un enregistrement est au premier plan — ou tant qu'une réunion Google
// Meet est ouverte — et la rétablit ensuite. Modes : auto, on (forcée), off.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::{Rc, Weak};

use serde_json::{json, Value};

use crate::js::{self, Timer, J};
use crate::proc::LineChild;

struct Rule {
    pattern: &'static str,
    label: &'static str,
    category: &'static str,
    title_only: bool,
}

const RULES: &[Rule] = &[
    Rule { pattern: r"(?i)discord", label: "Discord", category: "visio", title_only: false },
    Rule { pattern: r"(?i)teams", label: "Microsoft Teams", category: "visio", title_only: false },
    Rule { pattern: r"(?i)zoom", label: "Zoom", category: "visio", title_only: false },
    Rule { pattern: r"(?i)webex", label: "Webex", category: "visio", title_only: false },
    Rule { pattern: r"(?i)skype", label: "Skype", category: "visio", title_only: false },
    Rule { pattern: r"(?i)slack", label: "Slack", category: "visio", title_only: false },
    Rule { pattern: r"(?i)google\s*meet|meet\.google\.com", label: "Google Meet", category: "visio", title_only: true },
    Rule { pattern: r"(?i)obs(?:64)?|streamlabs", label: "OBS Studio", category: "enregistrement", title_only: false },
    Rule { pattern: r"(?i)audacity", label: "Audacity", category: "enregistrement", title_only: false },
    Rule { pattern: r"(?i)camtasia", label: "Camtasia", category: "enregistrement", title_only: false },
    Rule { pattern: r"(?i)bandicam", label: "Bandicam", category: "enregistrement", title_only: false },
    Rule { pattern: r"(?i)steam(?:exe)?", label: "Steam", category: "jeu", title_only: false },
    Rule { pattern: r"(?i)epicgames", label: "Epic Games", category: "jeu", title_only: false },
    Rule { pattern: r"(?i)riotclient|valorant|league", label: "Riot Games", category: "jeu", title_only: false },
    Rule { pattern: r"(?i)minecraft", label: "Minecraft", category: "jeu", title_only: false },
    Rule { pattern: r"(?i)cs2|counter-strike", label: "Counter-Strike", category: "jeu", title_only: false },
    Rule { pattern: r"(?i)overwatch", label: "Overwatch", category: "jeu", title_only: false },
    Rule { pattern: r"(?i)gta5", label: "GTA V", category: "jeu", title_only: false },
    Rule { pattern: r"(?i)cyberpunk2077", label: "Cyberpunk 2077", category: "jeu", title_only: false },
    Rule { pattern: r"(?i)fortnite", label: "Fortnite", category: "jeu", title_only: false },
    Rule { pattern: r"(?i)rocketleague", label: "Rocket League", category: "jeu", title_only: false },
    Rule { pattern: r"(?i)unreal|unity", label: "Moteur de jeu", category: "jeu", title_only: false },
];

pub struct ContextMute {
    me: Weak<ContextMute>,
    log: Rc<dyn Fn(String)>,
    emit: Rc<dyn Fn(J)>,
    session_file: Option<PathBuf>,
    meet_probe: Box<dyn Fn() -> Option<J>>,
    meet_enabled: Box<dyn Fn() -> bool>,
    meet_on: Cell<bool>,
    /// Une musique est en lecture (lecteur interne, Spotify…) et le réglage audio.musicSilence est actif : Jarvis se tait.
    music_on: Cell<bool>,
    music_enabled: Cell<bool>,
    last_fg: RefCell<Option<J>>,
    pub mode: RefCell<String>,
    pub is_muted: Cell<bool>,
    pub reason: RefCell<String>,
    active_app: RefCell<Value>,
    timer: Timer,
    watcher: RefCell<Option<Rc<LineChild>>>,
}

impl ContextMute {
    pub fn new(log: Rc<dyn Fn(String)>, emit: Rc<dyn Fn(J)>, session_file: Option<PathBuf>, meet_probe: Box<dyn Fn() -> Option<J>>, meet_enabled: Box<dyn Fn() -> bool>) -> Rc<Self> {
        let c = Rc::new_cyclic(|me| Self {
            me: me.clone(),
            log,
            emit,
            session_file,
            meet_probe,
            meet_enabled,
            meet_on: Cell::new(false),
            music_on: Cell::new(false),
            music_enabled: Cell::new(true),
            last_fg: RefCell::new(None),
            mode: RefCell::new("auto".into()),
            is_muted: Cell::new(false),
            reason: RefCell::new(String::new()),
            active_app: RefCell::new(Value::Null),
            timer: Timer::new(),
            watcher: RefCell::new(None),
        });
        c.load_state();
        c
    }

    fn load_state(&self) {
        if let Some(f) = &self.session_file {
            if let Some(d) = js::read_json(f) {
                let m = js::gs(&d, "contextMuteMode");
                if ["auto", "on", "off"].contains(&m.as_str()) {
                    *self.mode.borrow_mut() = m;
                }
            }
        }
    }

    fn save_state(&self) {
        let Some(f) = &self.session_file else { return };
        let mut d = js::read_json(f).unwrap_or(json!({}));
        d["contextMuteMode"] = json!(*self.mode.borrow());
        let _ = std::fs::write(f, js::pretty(&d));
    }

    pub async fn set_mode(&self, mode: &str) -> Result<J, String> {
        let clean = mode.to_lowercase().trim().to_string();
        if !["auto", "on", "off"].contains(&clean.as_str()) {
            return Err(format!("Mode de sourdine invalide : {mode} (valeurs admises : auto, on, off)"));
        }
        *self.mode.borrow_mut() = clean;
        self.save_state();
        Ok(self.update(None).await)
    }

    pub fn get_state(&self) -> J {
        json!({ "mode": *self.mode.borrow(), "isMuted": self.is_muted.get(), "reason": *self.reason.borrow(), "activeApp": *self.active_app.borrow() })
    }

    fn matches_rule(fg: &J) -> Option<&'static Rule> {
        let proc_ = js::gs(fg, "process");
        let title = js::gs(fg, "title");
        RULES.iter().find(|r| {
            let re = regex::Regex::new(r.pattern).unwrap();
            let target = if r.title_only { &title } else { &proc_ };
            re.is_match(target) || (!r.title_only && re.is_match(&title))
        })
    }

    fn meet_call(&self) -> Option<J> {
        if !(self.meet_enabled)() {
            return None;
        }
        (self.meet_probe)().map(|m| json!({ "title": js::gs(&m, "title") }))
    }

    /// La lecture d'une musique démarre ou s'arrête (événement « media ») : réévalue la sourdine.
    pub fn set_music(&self, playing: bool, enabled: bool) {
        if self.music_on.get() == playing && self.music_enabled.get() == enabled {
            return;
        }
        self.music_on.set(playing);
        self.music_enabled.set(enabled);
        if let Some(me) = self.me.upgrade() {
            js::spawn(async move {
                me.refresh_meet(true).await;
            });
        }
    }

    /// Une page Meet s'ouvre ou se ferme : réévalue la sourdine.
    pub async fn refresh_meet(&self, force: bool) -> J {
        let on = self.meet_call().is_some();
        if !force && on == self.meet_on.get() {
            return self.get_state();
        }
        self.meet_on.set(on);
        let fg = self.last_fg.borrow().clone();
        self.update(Some(fg.unwrap_or(Value::Null))).await
    }

    /// Une visio est en cours (page Meet ouverte, ou application de visio au premier plan), même en « Sourdine forcée » :
    /// l'écoute de Jarvis reste en pause tant qu'elle dure (voice_link › visio_pause).
    fn in_visio(&self) -> bool {
        self.meet_on.get() || self.active_app.borrow()["category"] == "visio"
    }

    pub async fn update(&self, mock_fg: Option<J>) -> J {
        let prev_muted = self.is_muted.get();
        let prev_reason = self.reason.borrow().clone();
        let prev_visio = self.in_visio();
        let mode = self.mode.borrow().clone();
        match mode.as_str() {
            "off" => {
                self.is_muted.set(false);
                *self.reason.borrow_mut() = String::new();
                *self.active_app.borrow_mut() = Value::Null;
            }
            "on" => {
                self.is_muted.set(true);
                *self.reason.borrow_mut() = "Sourdine forcée".into();
                *self.active_app.borrow_mut() = Value::Null;
            }
            _ => {
                let fg = match mock_fg {
                    Some(f) => f,
                    None => super::system::fenetre_active(&json!({})).await.unwrap_or(Value::Null),
                };
                *self.last_fg.borrow_mut() = Some(fg.clone());
                let matched = if fg.is_null() { None } else { Self::matches_rule(&fg) };
                let meet = self.meet_call();
                self.meet_on.set(meet.is_some());
                if let Some(r) = matched {
                    self.is_muted.set(true);
                    *self.reason.borrow_mut() = format!("{} ({})", r.label, r.category);
                    *self.active_app.borrow_mut() = json!({ "process": js::gs(&fg, "process"), "title": js::gs(&fg, "title"), "label": r.label, "category": r.category });
                } else if let Some(m) = meet {
                    self.is_muted.set(true);
                    *self.reason.borrow_mut() = "Google Meet (visio)".into();
                    *self.active_app.borrow_mut() = json!({ "process": "", "title": m["title"], "label": "Google Meet", "category": "visio" });
                } else if self.music_on.get() && self.music_enabled.get() {
                    // Musique en cours : Jarvis ne parle plus (ses réponses restent affichées) jusqu'à l'arrêt de la musique.
                    self.is_muted.set(true);
                    *self.reason.borrow_mut() = "Musique en cours".into();
                    *self.active_app.borrow_mut() = json!({ "process": "", "title": "", "label": "Musique", "category": "musique" });
                } else {
                    self.is_muted.set(false);
                    *self.reason.borrow_mut() = String::new();
                    *self.active_app.borrow_mut() = Value::Null;
                }
            }
        }
        if prev_muted != self.is_muted.get() || prev_reason != *self.reason.borrow() || prev_visio != self.in_visio() {
            (self.log)(format!("sourdine contextuelle : {}", if self.is_muted.get() { format!("ACTIVÉE ({})", self.reason.borrow()) } else { "DÉSACTIVÉE".into() }));
            (self.emit)(json!({ "type": "context_mute", "mode": mode, "isMuted": self.is_muted.get(), "reason": *self.reason.borrow(), "activeApp": *self.active_app.borrow(), "visio": self.in_visio() }));
            if self.is_muted.get() && !prev_muted {
                (self.emit)(json!({ "type": "voice_stop" }));
            }
        }
        self.get_state()
    }

    pub fn start(&self) {
        if self.timer.active() || self.watcher.borrow().is_some() {
            return;
        }
        let me = self.me.clone();
        js::spawn(async move {
            if let Some(s) = me.upgrade() {
                s.update(None).await;
            }
        });
        // Fenêtre au premier plan signalée en direct (SetWinEventHook) : 0 % CPU.
        let exe = std::env::current_exe().unwrap_or_default();
        let me1 = self.me.clone();
        let me2 = self.me.clone();
        match LineChild::spawn(
            &exe,
            &["active-win".into(), "--watch".into()],
            &[],
            move |line| {
                if let (Some(s), Ok(fg)) = (me1.upgrade(), serde_json::from_str::<J>(&line)) {
                    js::spawn(async move {
                        s.update(Some(fg)).await;
                    });
                }
            },
            |_| {},
            move |_| {
                if let Some(s) = me2.upgrade() {
                    s.watcher.borrow_mut().take();
                }
            },
        ) {
            Ok(c) => {
                crate::proc::tie_to_core(c.pid);
                *self.watcher.borrow_mut() = Some(c);
                (self.log)("sourdine contextuelle : hook événementiel natif Win32 actif (0% CPU)".into());
            }
            Err(e) => (self.log)(format!("échec du watcher natif : {e}, repli sur polling")),
        }
        // Relevé de garde (5 s) seulement si le hook natif n'est pas actif.
        let me = self.me.clone();
        self.timer.every(5000, move || {
            if let Some(s) = me.upgrade() {
                if s.watcher.borrow().is_none() {
                    js::spawn(async move {
                        s.update(None).await;
                    });
                }
            }
        });
    }

    pub fn stop(&self) {
        if let Some(w) = self.watcher.borrow_mut().take() {
            w.kill();
        }
        self.timer.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run<F: std::future::Future<Output = ()>>(f: F) {
        let rt = tokio::runtime::Builder::new_current_thread().enable_all().build().unwrap();
        tokio::task::LocalSet::new().block_on(&rt, f);
    }

    fn muet() -> Rc<ContextMute> {
        ContextMute::new(Rc::new(|_| {}), Rc::new(|_| {}), None, Box::new(|| None), Box::new(|| true))
    }

    #[test]
    fn la_musique_en_cours_coupe_la_voix_de_jarvis() {
        run(async {
            let c = muet();
            let bureau = json!({ "process": "explorer", "title": "Dossier" });
            c.update(Some(bureau.clone())).await;
            assert!(!c.is_muted.get(), "rien ne joue : Jarvis parle");
            c.music_on.set(true);
            c.update(Some(bureau.clone())).await;
            assert!(c.is_muted.get(), "musique en cours : Jarvis se tait");
            assert_eq!(*c.reason.borrow(), "Musique en cours");
            c.music_enabled.set(false);
            c.update(Some(bureau.clone())).await;
            assert!(!c.is_muted.get(), "réglage coupé : Jarvis parle");
            c.music_enabled.set(true);
            c.music_on.set(false);
            c.update(Some(bureau)).await;
            assert!(!c.is_muted.get(), "musique arrêtée : Jarvis reparle");
        });
    }

    #[test]
    fn une_visio_garde_la_priorite_sur_la_musique() {
        run(async {
            let c = ContextMute::new(Rc::new(|_| {}), Rc::new(|_| {}), None, Box::new(|| Some(json!({ "title": "Réunion" }))), Box::new(|| true));
            c.music_on.set(true);
            c.update(Some(json!({ "process": "explorer", "title": "Dossier" }))).await;
            assert_eq!(*c.reason.borrow(), "Google Meet (visio)");
        });
    }
}
