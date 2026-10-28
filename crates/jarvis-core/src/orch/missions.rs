// Missions vérifiées (orchestrator/missions.js) : une demande en plusieurs étapes, certaines sans IA,
// d'autres non (« ouvre VS Code, puis explique-moi le dernier commit de stats-ia et ensuite baisse le son »).
// Découpage sur « puis », « ensuite », « après ça », « et enfin »… ; étapes dans l'ordre, une à la fois,
// chacune vérifiée ; card « MISSION » cochée ; au premier échec, arrêt et reprise à cette étape
// (« reprends la mission »). Gardée sur disque (data/mission.json).

use std::cell::{Cell, RefCell};
use std::path::PathBuf;
use std::rc::Rc;

use serde_json::{json, Value};
use tokio::sync::oneshot;

use super::{OrchRef, Up};
use crate::js::{self, Timer, J};

const AI_STEP_TIMEOUT: u64 = 10 * 60 * 1000;

/// Découpe sur le texte d'origine (casse et accents gardés), en s'appuyant sur sa version repliée.
pub fn split_steps(raw: &str, fold: &dyn Fn(&str) -> String) -> Vec<String> {
    let words: Vec<&str> = raw.split_whitespace().collect();
    let folded: Vec<String> = words.iter().map(|w| fold(w)).collect();
    let mut steps: Vec<String> = vec![];
    let mut current: Vec<&str> = vec![];
    let mut i = 0;
    while i < words.len() {
        let one = folded[i].as_str();
        let two = if i + 1 < words.len() { format!("{} {}", folded[i], folded[i + 1]) } else { String::new() };
        let three = if i + 2 < words.len() { format!("{two} {}", folded[i + 2]) } else { String::new() };
        let skip = if three == "et pour finir" {
            3
        } else if matches!(two.as_str(), "et puis" | "apres ca" | "apres cela" | "apres quoi" | "et apres" | "et enfin" | "et ensuite" | "pour finir") {
            2
        } else if matches!(one, "puis" | "ensuite") {
            1
        } else {
            0
        };
        if skip > 0 {
            if !current.is_empty() {
                steps.push(current.join(" "));
            }
            current.clear();
            i += skip;
            continue;
        }
        current.push(words[i]);
        i += 1;
    }
    if !current.is_empty() {
        steps.push(current.join(" "));
    }
    steps
        .into_iter()
        .map(|s| {
            let s = crate::re!(r"^[\s,;:.]+|[\s,;:]+$").replace_all(&s, "");
            crate::re!(r"(?i)^(?:et|,)\s+").replace(&s, "").trim().to_string()
        })
        .filter(|s| s.split_whitespace().count() >= 2)
        .collect()
}

pub enum MissionCmd {
    Resume,
    Abandon,
    Status,
}

struct Waiting {
    text: String,
    tx: oneshot::Sender<(bool, String)>,
    timer: Timer,
}

pub struct Missions {
    o: OrchRef,
    file: PathBuf,
    fold: Rc<dyn Fn(&str) -> String>,
    negative: regex::Regex,
    incapable: Rc<dyn Fn(&str) -> Option<String>>,
    log: Rc<dyn Fn(String)>,
    /// { request, steps: [{ text, local, status, note }], startedAt, running }
    pub current: RefCell<Option<J>>,
    generation: Cell<u64>,
    waiting: RefCell<Option<Waiting>>,
    run_token: Cell<u64>,
}

impl Missions {
    pub fn new(
        o: OrchRef,
        file: PathBuf,
        fold: Rc<dyn Fn(&str) -> String>,
        negative: regex::Regex,
        incapable: Rc<dyn Fn(&str) -> Option<String>>,
        log: Rc<dyn Fn(String)>,
    ) -> Self {
        let current = js::read_json(&file).filter(|m| m["steps"].is_array()).map(|mut m| {
            m["running"] = json!(false);
            m
        });
        Self { o, file, fold, negative, incapable, log, current: RefCell::new(current), generation: Cell::new(0), waiting: RefCell::new(None), run_token: Cell::new(0) }
    }

    fn save(&self) {
        let res = match &*self.current.borrow() {
            None => {
                let _ = std::fs::remove_file(&self.file);
                Ok(())
            }
            Some(m) => {
                let mut m = m.clone();
                if let Some(o) = m.as_object_mut() {
                    o.remove("running");
                }
                js::write_json(&self.file, &m)
            }
        };
        if let Err(e) = res {
            (self.log)(format!("mission : enregistrement impossible ({e})"));
        }
    }

    fn set_current(&self, m: Option<J>) {
        *self.current.borrow_mut() = m;
        self.generation.set(self.generation.get() + 1);
    }

    fn with<R>(&self, f: impl FnOnce(&mut J) -> R) -> Option<R> {
        self.current.borrow_mut().as_mut().map(f)
    }

    /// Mission si au moins deux étapes, dont une locale et une pour l'IA.
    pub fn plan(&self, text: &str) -> Option<Vec<J>> {
        let parts = split_steps(text, &*self.fold);
        if parts.len() < 2 || parts.len() > 8 {
            return None;
        }
        let o = self.o.up();
        let steps: Vec<J> = parts
            .into_iter()
            .map(|p| {
                let local = o.router.route(&p, false).map(|i| !i.ignored).unwrap_or(false);
                json!({ "text": p, "local": local, "status": "todo" })
            })
            .collect();
        let locals = steps.iter().filter(|s| js::gb(s, "local")).count();
        if locals == 0 || locals == steps.len() {
            return None;
        }
        Some(steps)
    }

    /// Commandes de la mission elle-même (texte replié).
    pub fn command(t: &str) -> Option<MissionCmd> {
        if crate::re!(r"^(?:reprends?|relance|continue|poursuis) (?:la |ma )?mission\b").is_match(t) {
            Some(MissionCmd::Resume)
        } else if crate::re!(r"^(?:abandonne|annule|oublie|arrete) (?:la |ma )?mission\b").is_match(t) {
            Some(MissionCmd::Abandon)
        } else if crate::re!(r"^(?:ou en est|ou en es tu (?:avec|de)|ou j en suis (?:de|avec)|etat de|montre(?: moi)?) (?:la |ma )?mission\b").is_match(t) {
            Some(MissionCmd::Status)
        } else {
            None
        }
    }

    fn say(&self, text: String) {
        self.o.up().emit(json!({ "type": "say", "text": text }));
    }

    pub async fn handle_command(&self, kind: MissionCmd) {
        let Some(m) = self.current.borrow().clone() else {
            self.say("Aucune mission en cours.".into());
            return;
        };
        let steps = js::arr(&m["steps"]);
        match kind {
            MissionCmd::Status => {
                self.card(false);
                let done = steps.iter().filter(|s| js::gs(s, "status") == "done").count();
                let failed = steps.iter().position(|s| js::gs(s, "status") == "failed");
                self.say(match failed {
                    Some(i) => format!("Mission arrêtée à l'étape {} sur {} : {}. Dis « reprends la mission » pour réessayer.", i + 1, steps.len(), js::gs(&steps[i], "text")),
                    None => {
                        let p = if done > 1 { "s" } else { "" };
                        format!("{done} étape{p} faite{p} sur {}.", steps.len())
                    }
                });
            }
            MissionCmd::Abandon => {
                self.cancel("abandon");
                self.set_current(None);
                self.save();
                self.say("Mission abandonnée.".into());
            }
            MissionCmd::Resume => {
                if js::gb(&m, "running") {
                    self.say("La mission est déjà en cours.".into());
                    return;
                }
                self.with(|m| {
                    if let Some(steps) = m["steps"].as_array_mut() {
                        for s in steps {
                            if matches!(js::gs(s, "status").as_str(), "failed" | "running") {
                                s["status"] = json!("todo");
                            }
                        }
                    }
                });
                self.say("Je reprends la mission.".into());
                self.run().await;
            }
        }
    }

    pub async fn start(&self, text: &str, steps: Vec<J>) {
        self.cancel("remplacée");
        let n = steps.len();
        let names: Vec<J> = steps.iter().map(|s| s["text"].clone()).collect();
        self.set_current(Some(json!({ "request": text, "steps": steps, "startedAt": js::now_ms() })));
        self.save();
        let o = self.o.up();
        o.record("mission", json!({ "demande": text, "etapes": names }));
        o.emit(json!({ "type": "say", "text": format!("Mission en {n} étapes. Je commence."), "ack": true }));
        self.run().await;
    }

    pub async fn run(&self) {
        if self.current.borrow().is_none() {
            return;
        }
        let gen = self.generation.get();
        let token = self.run_token.get() + 1;
        self.run_token.set(token);
        self.with(|m| m["running"] = json!(true));
        self.card(false);
        let count = self.with(|m| js::arr(&m["steps"]).len()).unwrap_or(0);
        let alive = |me: &Self| token == me.run_token.get() && gen == me.generation.get();
        for i in 0..count {
            let Some(step) = self.with(|m| m["steps"][i].clone()) else { return };
            if js::gs(&step, "status") == "done" {
                continue;
            }
            if !alive(self) {
                return; // annulée ou remplacée
            }
            self.with(|m| m["steps"][i]["status"] = json!("running"));
            self.card(false);
            let (ok, note) = if js::gb(&step, "local") { self.run_local(&js::gs(&step, "text")).await } else { self.run_ai(i).await };
            if !alive(self) {
                return;
            }
            self.with(|m| {
                m["steps"][i]["status"] = json!(if ok { "done" } else { "failed" });
                m["steps"][i]["note"] = json!(note);
            });
            self.save();
            self.card(false);
            if !ok {
                self.with(|m| m["running"] = json!(false));
                self.o.up().record("mission_arretee", json!({ "etape": i + 1, "texte": step["text"], "motif": note }));
                self.say(format!(
                    "L'étape {} n'a pas abouti{}. Dis « reprends la mission » pour réessayer, ou « abandonne la mission ».",
                    i + 1,
                    if note.is_empty() { String::new() } else { format!(" : {note}") }
                ));
                return;
            }
        }
        let request = self.with(|m| {
            m["running"] = json!(false);
            m["request"].clone()
        });
        let o = self.o.up();
        o.record("mission_terminee", json!({ "demande": request, "etapes": count }));
        self.say(format!("Mission terminée : {count} étapes sur {count}."));
        self.card(true);
        self.set_current(None);
        self.save();
    }

    async fn run_local(&self, text: &str) -> (bool, String) {
        let o = self.o.up();
        let Some(intent) = o.router.route(text, false).filter(|i| !i.ignored) else { return (false, "je ne sais plus le faire seul".into()) };
        let Some(res) = o.run_intent(intent, text).await else { return (false, "erreur".into()) };
        let say = js::gs(&res, "say");
        let display = js::gs(&res, "display");
        if self.negative.is_match(&say) || crate::re!(r"(?i)^échec$").is_match(&display) {
            return (false, js::slice_chars(if say.is_empty() { &display } else { &say }, 120));
        }
        (true, String::new())
    }

    fn prompt(m: &J, index: usize) -> String {
        let steps = js::arr(&m["steps"]);
        let done: Vec<String> = steps.iter().take(index).enumerate().map(|(i, s)| format!("{}. {} — fait", i + 1, js::gs(s, "text"))).collect();
        let mut out = vec![format!("Mission en {} étapes demandée par l'utilisateur : « {} »", steps.len(), js::gs(m, "request"))];
        if !done.is_empty() {
            out.push(format!("Déjà fait :\n{}", done.join("\n")));
        }
        out.push(format!("Fais maintenant UNIQUEMENT l'étape {} : « {} ». Les étapes suivantes seront faites après.", index + 1, js::gs(&steps[index], "text")));
        out.push("Vérifie que l'étape a réellement abouti avant de répondre, et réponds en une ou deux phrases.".into());
        out.join("\n")
    }

    async fn run_ai(&self, index: usize) -> (bool, String) {
        let Some(text) = self.with(|m| Self::prompt(m, index)) else { return (false, "erreur".into()) };
        let (tx, rx) = oneshot::channel();
        let timer = Timer::new();
        let o = self.o.clone();
        timer.set(AI_STEP_TIMEOUT, move || {
            if let Some(w) = o.up().missions.waiting.borrow_mut().take() {
                let _ = w.tx.send((false, "trop long".into()));
            }
        });
        *self.waiting.borrow_mut() = Some(Waiting { text: text.clone(), tx, timer });
        self.o.up().enqueue_ai(text, json!({}));
        rx.await.unwrap_or((false, "arret".into()))
    }

    /// Fin d'une tâche IA (orchestrateur › complete_task) : est-ce l'étape attendue ?
    pub fn observe(&self, text: &str, ok: bool, answer: &str) -> bool {
        let w = {
            let mut slot = self.waiting.borrow_mut();
            match &*slot {
                Some(w) if w.text == text => slot.take().unwrap(),
                _ => return false,
            }
        };
        w.timer.clear();
        let cannot = if ok { (self.incapable)(answer) } else { None };
        let _ = w.tx.send(if ok && cannot.is_none() {
            (true, String::new())
        } else {
            (false, if cannot.is_some() { "l’IA dit ne pas pouvoir le faire".into() } else { "échec de l’IA".into() })
        });
        true
    }

    /// Arrêt général ou mission remplacée : l'étape IA en attente est lâchée.
    pub fn cancel(&self, reason: &str) {
        self.run_token.set(self.run_token.get() + 1);
        if let Some(w) = self.waiting.borrow_mut().take() {
            w.timer.clear();
            let _ = w.tx.send((false, reason.to_string()));
        }
        let was_running = self.current.borrow().as_ref().map(|m| js::gb(m, "running")).unwrap_or(false);
        if was_running {
            self.with(|m| {
                m["running"] = json!(false);
                if let Some(steps) = m["steps"].as_array_mut() {
                    for s in steps.iter_mut().filter(|s| js::gs(s, "status") == "running") {
                        s["status"] = json!("failed");
                        s["note"] = json!(reason);
                    }
                }
            });
            self.save();
            self.card(false);
        }
    }

    pub fn card(&self, finished: bool) {
        let Some(m) = self.current.borrow().clone() else { return };
        let steps = js::arr(&m["steps"]);
        let done = steps.iter().filter(|s| js::gs(s, "status") == "done").count();
        let rows: Vec<J> = steps
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let st = js::gs(s, "status");
                let icon = match st.as_str() {
                    "done" => "✓",
                    "running" => "…",
                    "failed" => "✕",
                    _ => "○",
                };
                let sub = match st.as_str() {
                    "failed" => {
                        let n = js::gs(s, "note");
                        if n.is_empty() { "échec".into() } else { n }
                    }
                    "running" => "en cours…".into(),
                    _ if js::gb(s, "local") => "sans IA".into(),
                    _ => "IA".to_string(),
                };
                let mut row = json!({ "icon": icon, "title": format!("{}. {}", i + 1, js::gs(s, "text")), "sub": sub });
                match st.as_str() {
                    "done" => row["tone"] = json!("ok"),
                    "failed" => {
                        row["tone"] = json!("warn");
                        row["say"] = json!("reprends la mission");
                        row["hint"] = json!("Reprendre");
                    }
                    _ => {}
                }
                row
            })
            .collect();
        self.o.up().show_widget(
            "tool",
            json!({
                "title": "Mission", "tool": "mission", "count": steps.len(), "rows": rows,
                "footer": if finished { "Mission terminée".to_string() } else { format!("{done} / {}", steps.len()) },
                "ttl": if finished { 20000 } else { 600000 }
            }),
        );
    }
}

#[allow(unused)]
fn _v(_: Value) {}
