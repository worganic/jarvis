// Démon CLI persistant pour Claude Code et AGY (orchestrator/cli-daemon.js) : un processus résident
// reçoit les demandes au format stream-json sur son entrée standard (plus de démarrage à froid à chaque
// phrase). La configuration porte une « signature » : si elle change, le démon est relancé dès qu'il est
// libre, en reprenant la conversation en cours.

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::{Rc, Weak};

use serde_json::json;

use crate::js::{self, Timer, J};
use crate::proc::LineChild;

const CRASH_WINDOW: i64 = 5000;
const MAX_CRASHES: u32 = 3;

#[derive(Clone)]
pub struct DaemonConfig {
    pub engine: String,
    pub exe: PathBuf,
    pub args: Vec<String>,
    pub session_args: Vec<String>,
    pub signature: String,
    pub preamble: String,
}

pub struct DaemonTask {
    pub on_event: Box<dyn Fn(&J)>,
    pub on_finish: Box<dyn FnOnce(bool)>,
    pub on_error: Box<dyn FnOnce(String)>,
}

#[derive(Clone, Copy, PartialEq, Debug)]
pub enum DaemonStatus {
    Stopped,
    Starting,
    Idle,
    Busy,
}

pub struct CliDaemon {
    me: RefCell<Weak<CliDaemon>>,
    workspace: PathBuf,
    log: Rc<dyn Fn(String)>,
    on_stderr: Rc<dyn Fn(&str, &str)>,
    child: RefCell<Option<Rc<LineChild>>>,
    child_seq: Cell<u64>,
    pub config: RefCell<Option<DaemonConfig>>,
    pub status: Cell<DaemonStatus>,
    task: RefCell<Option<DaemonTask>>,
    needs_preamble: Cell<bool>,
    warm: Cell<bool>,
    started_at: Cell<i64>,
    crashes: Cell<u32>,
    pub disabled: Cell<bool>,
    stopping: Cell<bool>,
    respawn: Timer,
}

impl CliDaemon {
    pub fn new(workspace: PathBuf, log: Rc<dyn Fn(String)>, on_stderr: Rc<dyn Fn(&str, &str)>) -> Rc<Self> {
        let d = Rc::new(Self {
            me: RefCell::new(Weak::new()),
            workspace,
            log,
            on_stderr,
            child: RefCell::new(None),
            child_seq: Cell::new(0),
            config: RefCell::new(None),
            status: Cell::new(DaemonStatus::Stopped),
            task: RefCell::new(None),
            needs_preamble: Cell::new(false),
            warm: Cell::new(false),
            started_at: Cell::new(0),
            crashes: Cell::new(0),
            disabled: Cell::new(false),
            stopping: Cell::new(false),
            respawn: Timer::new(),
        });
        *d.me.borrow_mut() = Rc::downgrade(&d);
        d
    }

    pub fn engine(&self) -> Option<String> {
        self.config.borrow().as_ref().map(|c| c.engine.clone())
    }

    pub fn signature(&self) -> Option<String> {
        self.config.borrow().as_ref().map(|c| c.signature.clone())
    }

    /// Prêt à recevoir une demande avec cette configuration précise.
    pub fn can_serve(&self, signature: &str) -> bool {
        self.child.borrow().is_some()
            && !self.disabled.get()
            && self.task.borrow().is_none()
            && matches!(self.status.get(), DaemonStatus::Idle | DaemonStatus::Starting)
            && self.signature().as_deref() == Some(signature)
    }

    pub fn child(&self) -> Option<Rc<LineChild>> {
        self.child.borrow().clone()
    }

    pub fn busy(&self) -> bool {
        self.task.borrow().is_some()
    }

    /// Démarre le démon ou le relance si la configuration a changé (seulement s'il est libre).
    pub fn ensure(&self, config: DaemonConfig) {
        if self.disabled.get() || self.stopping.get() {
            return;
        }
        if self.child.borrow().is_some() && self.signature().as_deref() == Some(config.signature.as_str()) {
            return;
        }
        if self.task.borrow().is_some() {
            return;
        }
        if self.child.borrow().is_some() {
            self.kill();
        }
        self.spawn_with(config);
    }

    fn spawn_with(&self, config: DaemonConfig) {
        self.status.set(DaemonStatus::Starting);
        self.needs_preamble.set(!config.preamble.is_empty());
        self.warm.set(false);
        self.started_at.set(js::now_ms());
        let mut args = config.args.clone();
        args.extend(config.session_args.iter().cloned());
        let seq = self.child_seq.get() + 1;
        self.child_seq.set(seq);
        *self.config.borrow_mut() = Some(config.clone());
        let me1 = self.me.borrow().clone();
        let me2 = self.me.borrow().clone();
        let me3 = self.me.borrow().clone();
        let engine = config.engine.clone();
        let engine2 = config.engine.clone();
        let env = vec![("JARVIS_PERSISTENT_DAEMON".to_string(), "1".to_string())];
        let spawned = LineChild::spawn_in(
            Some(&self.workspace),
            &config.exe,
            &args,
            &env,
            move |line| {
                if let Some(d) = me1.upgrade() {
                    if d.child_seq.get() == seq {
                        if let Ok(evt) = serde_json::from_str::<J>(&line) {
                            d.handle_event(&evt);
                        }
                    }
                }
            },
            move |chunk| {
                if let Some(d) = me2.upgrade() {
                    (d.on_stderr)(&engine, &chunk);
                    if crate::re!(r"(?i)error|erreur").is_match(&chunk) {
                        (d.log)(format!("[démon {engine} stderr] {}", js::slice_chars(chunk.trim(), 200)));
                    }
                }
            },
            move |code| {
                if let Some(d) = me3.upgrade() {
                    if d.child_seq.get() == seq {
                        d.on_exit(code);
                    }
                }
            },
        );
        match spawned {
            Ok(c) => {
                (self.log)(format!("[démon {engine2}] préchauffage (PID {})…", c.pid));
                *self.child.borrow_mut() = Some(c);
            }
            Err(e) => {
                (self.log)(format!("[démon {engine2}] erreur de lancement : {e}"));
                self.on_exit(None);
            }
        }
    }

    fn on_exit(&self, code: Option<i32>) {
        let task = self.task.borrow_mut().take();
        let quick = js::now_ms() - self.started_at.get() < CRASH_WINDOW;
        let engine = self.engine().unwrap_or_default();
        *self.child.borrow_mut() = None;
        self.status.set(DaemonStatus::Stopped);
        if let Some(t) = task {
            (t.on_error)("processus résident interrompu".into());
        }
        if self.stopping.get() {
            return;
        }
        self.crashes.set(if quick { self.crashes.get() + 1 } else { 0 });
        let code = code.map(|c| c.to_string()).unwrap_or_else(|| "null".into());
        if self.crashes.get() >= MAX_CRASHES {
            self.disabled.set(true);
            (self.log)(format!("[démon {engine}] arrêts répétés (code {code}) : démon désactivé, lancement à la demande"));
            return;
        }
        (self.log)(format!("[démon {engine}] arrêté (code {code}), relance dans 2 s"));
        let me = self.me.borrow().clone();
        self.respawn.set(2000, move || {
            if let Some(d) = me.upgrade() {
                let cfg = d.config.borrow().clone();
                if d.child.borrow().is_none() && !d.stopping.get() {
                    if let Some(c) = cfg {
                        d.spawn_with(c);
                    }
                }
            }
        });
    }

    /// Envoie une demande au démon ; false s'il ne peut pas la prendre.
    pub fn submit(&self, text: &str, task: DaemonTask) -> bool {
        let Some(sig) = self.signature() else { return false };
        if !self.can_serve(&sig) {
            return false;
        }
        let cfg = self.config.borrow().clone().unwrap();
        let content = if self.needs_preamble.get() && !cfg.preamble.is_empty() { format!("{}\n\nDemande de l'utilisateur : {text}", cfg.preamble) } else { text.to_string() };
        let payload = if cfg.engine == "agy" { json!({ "event": "user", "message": { "content": content } }) } else { json!({ "type": "user", "message": { "role": "user", "content": content } }) };
        let Some(child) = self.child.borrow().clone() else { return false };
        let line = payload.to_string();
        let log = self.log.clone();
        let engine = cfg.engine.clone();
        js::spawn(async move {
            if !child.write_line(&line).await {
                log(format!("[démon {engine}] écriture impossible"));
            }
        });
        self.needs_preamble.set(false);
        self.status.set(DaemonStatus::Busy);
        *self.task.borrow_mut() = Some(task);
        true
    }

    fn handle_event(&self, evt: &J) {
        let is_init = (js::gs(evt, "type") == "system" && js::gs(evt, "subtype") == "init") || js::gs(evt, "event") == "init";
        if is_init && !self.warm.get() {
            self.warm.set(true);
            if self.status.get() == DaemonStatus::Starting {
                self.status.set(DaemonStatus::Idle);
            }
            (self.log)(format!("[démon {}] opérationnel en {} ms", self.engine().unwrap_or_default(), js::now_ms() - self.started_at.get()));
        }
        if self.task.borrow().is_none() {
            return;
        }
        if let Some(t) = self.task.borrow().as_ref() {
            (t.on_event)(evt);
        }
        let is_result = js::gs(evt, "type") == "result" || js::gs(evt, "event") == "result";
        if is_result {
            let is_error = if js::gs(evt, "type") == "result" { js::gb(evt, "is_error") } else { js::gs(&evt["result"], "status") == "ERROR" };
            let task = self.task.borrow_mut().take();
            self.status.set(DaemonStatus::Idle);
            self.crashes.set(0);
            if let Some(t) = task {
                (t.on_finish)(is_error);
            }
        }
    }

    /// Annulation par l'utilisateur : la tâche est abandonnée sans rappel, le processus relancé.
    pub fn abort(&self) {
        *self.task.borrow_mut() = None;
        self.kill();
        let cfg = self.config.borrow().clone();
        if let Some(c) = cfg {
            if !self.stopping.get() && !self.disabled.get() {
                self.spawn_with(c);
            }
        }
    }

    pub fn kill(&self) {
        // Le processus tué ne doit plus rien signaler (numéro de lancement changé).
        self.child_seq.set(self.child_seq.get() + 1);
        let child = self.child.borrow_mut().take();
        self.status.set(DaemonStatus::Stopped);
        if let Some(c) = child {
            c.close_stdin();
            crate::proc::kill_tree(c.pid);
            c.kill();
        }
    }

    pub fn stop(&self) {
        self.stopping.set(true);
        self.respawn.clear();
        *self.task.borrow_mut() = None;
        self.kill();
    }
}
