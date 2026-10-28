// Surveillance permanente des agents IA (orchestrator/ai-watch.js) : Claude Code (états écrits par les
// hooks dans ~/.claude/ai-state/*.json) et AGY (transcript de ~/.gemini/antigravity-cli/brain/*). Jarvis
// prévient quand un prompt est terminé ou qu'une réponse est attendue, et met à jour la card TERMINAL.

use std::cell::RefCell;
use std::collections::HashMap;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::rc::{Rc, Weak};

use serde_json::{json, Value};

use super::{OrchRef, Up};
use crate::js::{self, Timer, J};

#[derive(Clone, Default)]
struct AgyAnalysis {
    state: String,
    message: String,
    is_question: bool,
    is_done: bool,
    last_prompt_index: i64,
    last_step_index: i64,
}

fn mtime_ms(p: &Path) -> Option<i64> {
    std::fs::metadata(p).ok()?.modified().ok()?.duration_since(std::time::UNIX_EPOCH).ok().map(|d| d.as_millis() as i64)
}

/// Analyse des 128 derniers Ko du transcript d'AGY.
fn analyze_agy_transcript(file: &Path) -> Option<AgyAnalysis> {
    let mut f = std::fs::File::open(file).ok()?;
    let size = f.metadata().ok()?.len();
    if size == 0 {
        return None;
    }
    let n = size.min(131072);
    f.seek(SeekFrom::Start(size - n)).ok()?;
    let mut buf = vec![0u8; n as usize];
    f.read_exact(&mut buf).ok()?;
    let text = String::from_utf8_lossy(&buf);
    let steps: Vec<J> = text.split('\n').filter_map(|l| serde_json::from_str(l.trim()).ok()).collect();
    if steps.is_empty() {
        return None;
    }
    let now = js::now_ms();
    let mut last_prompt_index = -1;
    // id → (description, début, durée)
    let mut tasks: Vec<(String, String, i64, Option<i64>)> = vec![];
    for step in &steps {
        let content = js::gs(step, "content");
        let typ = js::gs(step, "type");
        let source = js::gs(step, "source");
        if typ == "USER_INPUT" || source == "USER_EXPLICIT" {
            last_prompt_index = step["step_index"].as_i64().unwrap_or(-1);
            tasks.clear();
        }
        if typ == "GENERIC" && js::gs(step, "status") == "RUNNING" {
            if let Some(m) = crate::re!(r"(?i)task id:\s*([^\s\n]+)").captures(&content) {
                let desc = crate::re!(r"(?i)Task Description:\s*([^\n]+)").captures(&content).map(|c| c[1].to_string()).unwrap_or_else(|| "Tâche en arrière-plan".into());
                let dur = crate::re!(r"(?i)Timer:\s*(\d+)s").captures(&content).and_then(|c| c[1].parse::<i64>().ok()).map(|s| s * 1000);
                let started = js::parse_date_ms(&js::gs(step, "created_at")).unwrap_or(now);
                let id = m[1].to_string();
                tasks.retain(|t| t.0 != id);
                tasks.push((id, desc, started, dur));
            }
        }
        if source == "SYSTEM" || typ == "SYSTEM_MESSAGE" {
            let tid = crate::re!(r#"(?i)Task id "([^"]+)" finished"#).captures(&content).map(|c| c[1].to_string()).or_else(|| crate::re!(r"(?i)sender=([^\s]+)").captures(&content).map(|c| c[1].to_string()));
            if let Some(t) = tid {
                tasks.retain(|x| x.0 != t);
            }
        }
        if typ == "GENERIC" {
            if let Some(k) = crate::re!(r#"(?i)task "([^"]+)": task is not running"#).captures(&content) {
                tasks.retain(|x| x.0 != k[1]);
            }
        }
    }
    tasks.retain(|t| t.3.map(|d| now - t.2 <= d + 4000).unwrap_or(true));
    let last = steps.last().unwrap();
    let mut a = AgyAnalysis { state: "thinking".into(), last_prompt_index, last_step_index: last["step_index"].as_i64().unwrap_or(-1), ..Default::default() };
    if js::gs(last, "type") == "USER_INPUT" || js::gs(last, "source") == "USER_EXPLICIT" {
        a.message = "Traitement de la demande...".into();
    } else if let Some(t) = tasks.first() {
        a.state = "running".into();
        a.message = if t.1.is_empty() { "Tâche en arrière-plan en cours".into() } else { t.1.clone() };
    } else if js::gs(last, "source") == "MODEL" && js::gs(last, "type") == "PLANNER_RESPONSE" {
        let calls = js::arr(&last["tool_calls"]);
        if let Some(ask) = calls.iter().find(|t| js::gs(t, "name") == "ask_question") {
            a.is_question = true;
            a.state = "waiting".into();
            a.message = js::gs(&ask["args"]["questions"][0], "question");
        } else if !calls.is_empty() {
            a.message = "Exécution d'outils...".into();
        } else if js::gs(last, "status") == "DONE" {
            a.is_done = true;
            a.state = "idle".into();
            a.message = "Exécution achevée".into();
        }
    } else {
        a.message = "Exécution en cours...".into();
    }
    Some(a)
}

#[derive(Clone, Default)]
struct ClaudeSession {
    state: String,
    ts: i64,
    project: String,
    message: String,
    notified_done: bool,
}

#[derive(Clone, Default)]
struct AgySession {
    state: String,
    mtime: i64,
    project: String,
    last_notified_prompt_index: i64,
    last_notified_question_index: i64,
}

/// Agents IA lancés dans un terminal : (PID de chaque Claude Code, nombre d'AGY). Processus `claude.exe` /
/// `agy.exe` (ou Claude Code sous node) dont un ancêtre est un terminal ou un shell, et aucun ancêtre jarvis*
/// (agent lancé par Jarvis).
fn detect_agents() -> (Vec<u32>, usize) {
    let procs = jarvis_native::window::processes();
    let by_pid: HashMap<u32, (u32, String)> = procs.iter().map(|(p, pp, n)| (*p, (*pp, n.to_lowercase()))).collect();
    let host = crate::re!(r"^(?:windowsterminal|openconsole|conhost|cmd|powershell|pwsh|wt|wezterm-gui|alacritty|mintty|tabby|hyper|warp|psmux|tmux)\.exe$");
    let (mut claude, mut agy) = (vec![], 0);
    for (pid, ppid, name) in &procs {
        let lname = name.to_lowercase();
        let is_claude = lname == "claude.exe" || (lname == "node.exe" && crate::re!(r"(?i)claude-code|@anthropic-ai[\\/]claude").is_match(&jarvis_native::window::command_line(*pid)));
        let is_agy = lname == "agy.exe" || lname == "antigravity.exe";
        if !is_claude && !is_agy {
            continue;
        }
        let (mut cur, mut in_host, mut by_jarvis, mut depth) = (*ppid, false, false, 0);
        while cur != 0 && depth < 16 {
            let Some((pp, n)) = by_pid.get(&cur) else { break };
            if n.starts_with("jarvis") {
                by_jarvis = true;
            }
            if host.is_match(n) {
                in_host = true;
            }
            cur = *pp;
            depth += 1;
        }
        if in_host && !by_jarvis {
            if is_claude {
                claude.push(*pid);
            } else {
                agy += 1;
            }
        }
    }
    (claude, agy)
}

pub struct AiWatch {
    o: OrchRef,
    me: RefCell<Weak<AiWatch>>,
    log: Rc<dyn Fn(String)>,
    timer: Timer,
    watcher: RefCell<Option<notify::RecommendedWatcher>>,
    claude_sessions: RefCell<Vec<(String, ClaudeSession)>>,
    agy_sessions: RefCell<HashMap<String, AgySession>>,
    claude_dir: PathBuf,
    agy_brain_dir: PathBuf,
    recent_alerts: RefCell<HashMap<String, i64>>,
    /// Derniers états connus : (claude, agy)
    latest: RefCell<(Option<J>, Option<J>)>,
    /// Agents réellement lancés dans un terminal : (instant du relevé, PID de chaque Claude Code, nombre d'AGY).
    running: RefCell<(i64, Vec<u32>, usize)>,
    /// ~/.claude/sessions : un fichier <pid>.json par Claude Code lancé (session, dossier, nom).
    sessions_dir: PathBuf,
    /// Signature de la card TERMINAL au dernier dessin (pour ne la redessiner que si elle change).
    card_sig: RefCell<String>,
}

impl AiWatch {
    pub fn new(o: OrchRef, log: Rc<dyn Fn(String)>) -> Rc<Self> {
        let home = js::home_dir();
        let me = Rc::new(Self {
            o,
            me: RefCell::new(Weak::new()),
            log,
            timer: Timer::new(),
            watcher: RefCell::new(None),
            claude_sessions: RefCell::new(vec![]),
            agy_sessions: RefCell::new(HashMap::new()),
            claude_dir: home.join(".claude").join("ai-state"),
            agy_brain_dir: home.join(".gemini").join("antigravity-cli").join("brain"),
            recent_alerts: RefCell::new(HashMap::new()),
            latest: RefCell::new((None, None)),
            running: RefCell::new((0, vec![], 0)),
            sessions_dir: home.join(".claude").join("sessions"),
            card_sig: RefCell::new(String::new()),
        });
        *me.me.borrow_mut() = Rc::downgrade(&me);
        me
    }

    pub fn config(&self) -> J {
        let mut d = json!({ "enabled": true, "notifyDone": true, "notifyWaiting": true, "speak": true });
        let c = self.o.up().config.get("aiWatch");
        if c.is_object() {
            js::assign(&mut d, c);
        }
        d
    }

    pub fn start(&self) {
        let _ = std::fs::create_dir_all(&self.claude_dir);
        self.scan_claude_dir(true);
        self.scan_agy_brain(true);
        self.watch_claude_dir();
        let me = self.me.borrow().clone();
        self.timer.every(1500, move || {
            if let Some(m) = me.upgrade() {
                m.scan_claude_dir(false);
                m.scan_agy_brain(false);
                m.purge_old_sessions();
                m.sync_card();
            }
        });
        (self.log)("[ai-watch] surveillance permanente de Claude Code et AGY démarrée".into());
    }

    fn watch_claude_dir(&self) {
        use notify::{RecursiveMode, Watcher};
        let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel::<()>();
        let w = notify::recommended_watcher(move |_: notify::Result<notify::Event>| {
            let _ = tx.send(());
        });
        match w {
            Ok(mut w) => {
                if let Err(e) = w.watch(&self.claude_dir, RecursiveMode::NonRecursive) {
                    (self.log)(format!("[ai-watch] surveillance impossible sur {} : {e}", self.claude_dir.display()));
                    return;
                }
                *self.watcher.borrow_mut() = Some(w);
                let me = self.me.borrow().clone();
                js::spawn(async move {
                    while rx.recv().await.is_some() {
                        let Some(m) = me.upgrade() else { break };
                        m.scan_claude_dir(false);
                    }
                });
            }
            Err(e) => (self.log)(format!("[ai-watch] surveillance impossible sur {} : {e}", self.claude_dir.display())),
        }
    }

    pub fn stop(&self) {
        self.timer.clear();
        self.watcher.borrow_mut().take();
    }

    fn scan_claude_dir(&self, silent: bool) {
        let Ok(rd) = std::fs::read_dir(&self.claude_dir) else { return };
        let now = js::now_ms();
        let live = self.live_statuses();
        let mut events = vec![];
        for e in rd.filter_map(|e| e.ok()) {
            let name = e.file_name().to_string_lossy().to_string();
            if !name.ends_with(".json") {
                continue;
            }
            let Some(data) = js::read_json(&e.path()) else { continue };
            let session_id = name.trim_end_matches(".json").to_string();
            let prev = self.claude_sessions.borrow().iter().find(|(k, _)| *k == session_id).map(|(_, v)| v.clone()).unwrap_or_default();
            let mut state = if js::truthy(&data["state"]) { js::gs(&data, "state") } else { "idle".into() };
            let ts = js::num(&data["ts"]).unwrap_or(0.0) as i64;
            let cwd = js::gs(&data, "cwd");
            let project = if js::truthy(&data["project"]) { js::gs(&data, "project") } else { Path::new(&cwd).file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default() };
            let mut message = js::gs(&data, "message");
            // Statut tenu par Claude Code lui-même : fait foi (longue réflexion ou commande sans hook = toujours en cours ;
            // interruption par Échap = terminé). À défaut, 45 s sans nouvelle alors qu'elle travaillait : terminée.
            let live = live.get(&session_id);
            let busy = state == "thinking" || state == "editing";
            if busy && live.is_some_and(|(s, at)| s == "idle" && *at > ts) {
                state = "idle".into();
                message = "Exécution achevée".into();
            } else if busy && now - ts > 45000 && !live.is_some_and(|(s, _)| s == "busy") {
                state = "idle".into();
                message = "Exécution achevée".into();
            }
            // « Claude is waiting for your input » après la fin d'un tour : il attend le prochain prompt.
            if state == "waiting" && (message.to_lowercase().contains("waiting for your input") || now - ts > 60000) {
                state = "idle".into();
                message = "Exécution achevée".into();
            }
            let mut record = ClaudeSession { state: state.clone(), ts, project: project.clone(), message: message.clone(), notified_done: prev.notified_done };
            let was_busy = prev.state == "thinking" || prev.state == "editing";
            if !silent && (ts > prev.ts || (state == "idle" && was_busy)) {
                if state == "idle" && was_busy {
                    if !record.notified_done {
                        record.notified_done = true;
                        events.push(json!({ "agent": "claude", "event": "Stop", "state": "idle", "sessionId": session_id, "project": project, "cwd": cwd, "message": message }));
                    }
                } else if state == "thinking" || state == "editing" {
                    record.notified_done = false;
                } else if state == "waiting" && prev.state != "waiting" {
                    events.push(json!({ "agent": "claude", "event": "Notification", "state": "waiting", "sessionId": session_id, "project": project, "cwd": cwd, "message": message }));
                }
            }
            let mut list = self.claude_sessions.borrow_mut();
            match list.iter_mut().find(|(k, _)| *k == session_id) {
                Some(x) => x.1 = record,
                None => list.push((session_id, record)),
            }
        }
        for ev in events {
            self.on_event(&ev);
        }
        // Session la plus pertinente pour la card TERMINAL : une active (< 45 s), sinon la plus récente.
        let active = |s: &ClaudeSession| (s.state == "thinking" || s.state == "editing") && now - s.ts <= 45000;
        let mut best: Option<ClaudeSession> = None;
        for (_, s) in self.claude_sessions.borrow().iter() {
            best = match best {
                None => Some(s.clone()),
                Some(b) => {
                    if active(s) && !active(&b) {
                        Some(s.clone())
                    } else if !active(s) && active(&b) {
                        Some(b)
                    } else if s.ts > b.ts {
                        Some(s.clone())
                    } else {
                        Some(b)
                    }
                }
            };
        }
        if let Some(b) = best {
            self.latest.borrow_mut().0 = Some(json!({ "agent": "claude", "state": b.state, "ts": b.ts, "project": b.project, "message": b.message }));
        }
    }

    fn scan_agy_brain(&self, silent: bool) {
        let Ok(rd) = std::fs::read_dir(&self.agy_brain_dir) else { return };
        let mut entries: Vec<(String, PathBuf, i64)> = rd
            .filter_map(|e| e.ok())
            .filter(|e| e.file_type().map(|t| t.is_dir()).unwrap_or(false))
            .map(|e| (e.file_name().to_string_lossy().to_string(), e.path(), mtime_ms(&e.path()).unwrap_or(0)))
            .collect();
        entries.sort_by(|a, b| b.2.cmp(&a.2));
        entries.truncate(5);
        let mut events = vec![];
        for (name, dir, _) in entries {
            let transcript = dir.join(".system_generated").join("logs").join("transcript.jsonl");
            let Some(file_mtime) = mtime_ms(&transcript) else { continue };
            let prev = self.agy_sessions.borrow().get(&name).cloned();
            if prev.as_ref().map(|p| file_mtime <= p.mtime).unwrap_or(false) {
                continue;
            }
            let Some(a) = analyze_agy_transcript(&transcript) else { continue };
            let proj = prev.as_ref().map(|p| p.project.clone()).filter(|p| !p.is_empty()).unwrap_or_else(|| name.chars().take(8).collect());
            let mut rec = AgySession {
                state: a.state.clone(),
                mtime: file_mtime,
                project: proj.clone(),
                last_notified_prompt_index: if silent { a.last_prompt_index } else { prev.as_ref().map(|p| p.last_notified_prompt_index).unwrap_or(-1) },
                last_notified_question_index: if silent { a.last_step_index } else { prev.as_ref().map(|p| p.last_notified_question_index).unwrap_or(-1) },
            };
            let newer = self.latest.borrow().1.as_ref().map(|l| file_mtime >= l["ts"].as_i64().unwrap_or(0)).unwrap_or(true);
            if newer {
                self.latest.borrow_mut().1 = Some(json!({ "agent": "agy", "state": a.state, "ts": file_mtime, "project": proj, "message": a.message }));
            }
            if let (false, Some(prev)) = (silent, prev) {
                if a.is_done && (prev.state == "thinking" || prev.state == "running") {
                    if prev.last_notified_prompt_index != a.last_prompt_index {
                        rec.last_notified_prompt_index = a.last_prompt_index;
                        events.push(json!({ "agent": "agy", "event": "Stop", "state": "idle", "sessionId": name, "project": proj }));
                    }
                } else if a.is_question && prev.state != "waiting" && prev.last_notified_question_index != a.last_step_index {
                    rec.last_notified_question_index = a.last_step_index;
                    events.push(json!({ "agent": "agy", "event": "ask_question", "state": "waiting", "sessionId": name, "project": proj, "message": a.message }));
                }
            }
            self.agy_sessions.borrow_mut().insert(name, rec);
        }
        for ev in events {
            self.on_event(&ev);
        }
    }

    /// Statut de chaque Claude Code lancé, écrit par Claude Code dans ~/.claude/sessions/<pid>.json :
    /// session → (« busy » ou « idle », instant du changement).
    fn live_statuses(&self) -> HashMap<String, (String, i64)> {
        self.running();
        let pids = self.running.borrow().1.clone();
        pids.iter()
            .filter_map(|pid| js::read_json(&self.sessions_dir.join(format!("{pid}.json"))))
            .filter(|i| js::truthy(&i["sessionId"]))
            .map(|i| (js::gs(&i, "sessionId"), (js::gs(&i, "status"), i["statusUpdatedAt"].as_i64().unwrap_or(0))))
            .collect()
    }

    fn purge_old_sessions(&self) {
        let cutoff = js::now_ms() - 30 * 60 * 1000;
        let live = self.live_statuses();
        let old: Vec<String> = self.claude_sessions.borrow().iter().filter(|(k, s)| s.ts != 0 && s.ts < cutoff && !live.contains_key(k)).map(|(k, _)| k.clone()).collect();
        for id in old {
            self.claude_sessions.borrow_mut().retain(|(k, _)| *k != id);
            let _ = std::fs::remove_file(self.claude_dir.join(format!("{id}.json")));
        }
        let alert_cutoff = js::now_ms() - 60000;
        self.recent_alerts.borrow_mut().retain(|_, t| *t >= alert_cutoff);
    }

    /// Agents lancés dans un terminal : (Claude Code, AGY) = nombre de processus interactifs. Relevé au plus
    /// toutes les 1,5 s. Un agent lancé par Jarvis lui-même (démon Claude, tâches) n'en fait pas partie :
    /// seuls comptent ceux qui descendent d'un terminal ou d'un shell et pas d'un processus jarvis*.
    pub fn running(&self) -> (usize, usize) {
        {
            let r = self.running.borrow();
            if js::now_ms() - r.0 < 1500 {
                return (r.1.len(), r.2);
            }
        }
        self.refresh_running()
    }

    fn refresh_running(&self) -> (usize, usize) {
        let (claude, agy) = detect_agents();
        let n = claude.len();
        *self.running.borrow_mut() = (js::now_ms(), claude, agy);
        (n, agy)
    }

    /// Un Claude Code lancé : état de sa session (hooks ai-state, sinon statut de ~/.claude/sessions/<pid>.json).
    /// Renvoie (début de la session, données de la ligne : project, state, ts, message).
    fn claude_view_data(&self, pid: u32) -> (i64, J) {
        let info = js::read_json(&self.sessions_dir.join(format!("{pid}.json"))).unwrap_or(Value::Null);
        let sid = js::gs(&info, "sessionId");
        let rec = if sid.is_empty() { None } else { self.claude_sessions.borrow().iter().find(|(k, _)| *k == sid).map(|(_, s)| s.clone()) };
        // Dossier de lancement (le cwd des hooks suit les « cd » de la session).
        let cwd = js::gs(&info, "cwd");
        let mut project = Path::new(&cwd).file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
        if project.is_empty() {
            project = rec.as_ref().map(|r| r.project.clone()).filter(|p| !p.is_empty()).unwrap_or_else(|| js::gs(&info, "name"));
        }
        let started = info["startedAt"].as_i64().unwrap_or(0);
        let d = match rec {
            Some(r) => json!({ "project": project, "state": r.state, "ts": r.ts, "message": r.message }),
            None => {
                let busy = js::gs(&info, "status") == "busy";
                json!({ "project": project, "state": if busy { "thinking" } else { "" }, "ts": info["statusUpdatedAt"].as_i64().unwrap_or(0), "message": "" })
            }
        };
        (started, d)
    }

    /// Vue de chaque agent lancé : (état : waiting | running | idle | launched, ligne de la card). Une ligne
    /// par Claude Code (un par terminal ou onglet) ; AGY reste regroupé (pas de lien processus ↔ transcript).
    fn agent_views(&self) -> Vec<(&'static str, J)> {
        let (_, na) = self.running();
        let pids = self.running.borrow().1.clone();
        let mut claude: Vec<(i64, u32, J)> = pids.into_iter().map(|pid| {
            let (started, d) = self.claude_view_data(pid);
            (started, pid, d)
        }).collect();
        claude.sort_by_key(|(started, pid, _)| (*started, *pid));
        let o = self.o.up();
        let mut out: Vec<(&'static str, J)> = claude
            .iter()
            .map(|(_, pid, d)| {
                let (state, mut row) = Self::view("Claude Code", 1, d);
                // Même couleur que l'onglet de Windows Terminal où ce Claude Code tourne.
                if let Some(c) = o.terminal_colors.color_of_process(*pid) {
                    row["accent"] = json!(c);
                }
                // Clic sur la card : la fenêtre du terminal passe au premier plan.
                if let Some(h) = o.terminal_colors.window_of_process(*pid) {
                    row["call"] = json!({ "method": "desktopAction", "args": [h, "basculer"] });
                    row["hint"] = json!("Clic : afficher ce terminal · nouveau clic : le réduire");
                }
                (state, row)
            })
            .collect();
        if na > 0 {
            let agy = self.latest.borrow().1.clone().unwrap_or(Value::Null);
            out.push(Self::view("AGY", na, &agy));
        }
        out
    }

    /// Ligne-card d'un agent : « Claude Code · projet », état en badge, dernier message en détail.
    fn view(name: &str, n: usize, d: &J) -> (&'static str, J) {
        let now = js::now_ms();
        let count = if n > 1 { format!(" ×{n}") } else { String::new() };
        let proj = if js::truthy(&d["project"]) { format!(" · {}", js::gs(d, "project")) } else { String::new() };
        let title = format!("{name}{count}{proj}");
        let ts = d["ts"].as_i64().unwrap_or(0);
        let time = if ts != 0 { js::local_of(ts).format("%H:%M:%S").to_string() } else { String::new() };
        let msg = js::gs(d, "message");
        let state = js::gs(d, "state");
        let busy = state == "thinking" || state == "editing" || state == "running";
        // Un état « terminé » ancien n'est pas une information : l'agent attend simplement un prompt.
        let stale = !busy && state != "waiting" && (ts == 0 || now - ts > 10 * 60 * 1000);
        let row = |icon: &str, tone: &str, label: &str, sub: String| {
            let mut r = json!({ "card": true, "icon": icon, "title": title, "state": label, "sub": sub, "meta": time });
            if !tone.is_empty() {
                r["tone"] = json!(tone);
            }
            r
        };
        if state == "waiting" {
            ("waiting", row("?", "danger", "réponse attendue", if msg.is_empty() { format!("{name} attend une confirmation ou une précision") } else { msg }))
        } else if busy {
            let label = if state == "editing" { "modifie des fichiers" } else { "en cours" };
            ("running", row("⏳", "warn", label, if msg.is_empty() { "Prompt en cours de traitement…".to_string() } else { msg }))
        } else if state == "idle" && !stale {
            ("idle", row("✓", "ok", "terminé", if msg.is_empty() { "Exécution achevée".to_string() } else { msg }))
        } else {
            let sub = if ts != 0 { format!("en attente d'un prompt · dernière activité à {time}") } else { "en attente d'un prompt".to_string() };
            let mut r = row("●", "", "lancé", sub);
            r["meta"] = json!("");
            ("launched", r)
        }
    }

    /// Lignes de statut pour la card TERMINAL : uniquement les agents réellement lancés.
    pub fn status_rows(&self) -> Vec<J> {
        let rows: Vec<J> = self.agent_views().into_iter().map(|(_, r)| r).collect();
        if rows.is_empty() {
            return vec![];
        }
        if self.config()["enabled"] == json!(false) {
            let mut v = vec![json!({ "icon": "◈", "title": "Surveillance IA : désactivée", "sub": "Active-la dans Administration ou dis « active la surveillance IA »", "tone": "danger" })];
            v.extend(rows);
            return v;
        }
        rows
    }

    /// Pastille de la card réduite : « ? » si un agent attend une réponse, « ⏳ » s'il travaille.
    pub fn badge(&self) -> (&'static str, &'static str) {
        let views = self.agent_views();
        if views.iter().any(|v| v.0 == "waiting") {
            ("?", "danger")
        } else if views.iter().any(|v| v.0 == "running") {
            ("⏳", "warn")
        } else {
            ("", "")
        }
    }

    /// Redessine la card TERMINAL (ou la retire) quand les agents lancés ou leur état ont changé.
    fn sync_card(&self) {
        let (c, a) = self.refresh_running();
        let rows = self.status_rows().iter().map(|r| r.to_string()).collect::<Vec<_>>().join("");
        let sig = format!("{c}/{a}|{rows}|{}", self.badge().0);
        if *self.card_sig.borrow() == sig {
            return;
        }
        *self.card_sig.borrow_mut() = sig;
        let o = self.o.up();
        o.terminal_voice.update_ai_status();
        o.tmux_voice.refresh_cards();
    }

    /// Événement reçu (endpoint HTTP /ai-event, ou veille autonome).
    pub fn on_event(&self, e: &J) -> J {
        let cfg = self.config();
        if cfg["enabled"] == json!(false) {
            return json!({ "ok": false, "reason": "disabled" });
        }
        let ag = if js::truthy(&e["agent"]) { js::gs(e, "agent").to_lowercase() } else { "claude".into() };
        let is_agy = ag == "agy" || ag == "gemini";
        let label = if is_agy { "AGY" } else { "Claude" };
        let cwd = js::gs(e, "cwd");
        let proj = if js::truthy(&e["project"]) { js::gs(e, "project") } else { Path::new(&cwd).file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default() };
        let proj_label = if proj.is_empty() { String::new() } else { format!(" sur {proj}") };
        let (event, state, message) = (js::gs(e, "event"), js::gs(e, "state"), js::gs(e, "message"));
        let is_done = event == "Stop" || state == "idle";
        let is_waiting = event == "Notification" || state == "waiting" || event == "ask_question";
        let latest = json!({
            "agent": if is_agy { "agy" } else { "claude" },
            "state": if !state.is_empty() { state.clone() } else if is_done { "idle".into() } else if is_waiting { "waiting".into() } else { "thinking".into() },
            "event": if !event.is_empty() { event.clone() } else if is_done { "Stop".into() } else if is_waiting { "Notification".into() } else { "UserPromptSubmit".into() },
            "project": proj, "message": message, "ts": js::now_ms()
        });
        if is_agy {
            self.latest.borrow_mut().1 = Some(latest);
        } else {
            self.latest.borrow_mut().0 = Some(latest);
        }
        let sid = js::gs(e, "sessionId");
        let sess_key = if !sid.is_empty() { sid } else if !proj.is_empty() { format!("{ag}:{proj}") } else { ag.clone() };
        let o = self.o.up();
        let mut sent: Value = Value::Null;
        let mut dedup = false;
        let source = if is_agy { "agy_watch" } else { "claude_watch" };
        let speak = js::truthy(&cfg["speak"]);
        let fresh = |key: String| -> bool {
            let now = js::now_ms();
            let mut alerts = self.recent_alerts.borrow_mut();
            if now - alerts.get(&key).copied().unwrap_or(0) < 5000 {
                false
            } else {
                alerts.insert(key, now);
                true
            }
        };
        if is_done {
            if js::truthy(&cfg["notifyDone"]) {
                if !fresh(format!("done:{sess_key}")) {
                    dedup = true;
                } else {
                    let text = format!("{label} a terminé d'exécuter le prompt{proj_label}.");
                    (self.log)(format!("[ai-watch] {text}"));
                    o.notify(json!({
                        "text": text, "title": format!("{label}{proj_label}"), "source": source, "speak": speak, "card": false, // l'état est dans la card TERMINAL
                        "rows": [{ "icon": "✓", "tone": "ok", "title": text, "sub": if message.is_empty() { "Exécution achevée".to_string() } else { message.clone() } }]
                    }));
                    o.schedules.on_claude_done(if proj.is_empty() { label } else { &proj }, true);
                    sent = json!("done");
                }
            }
        } else if is_waiting && js::truthy(&cfg["notifyWaiting"]) {
            if !fresh(format!("waiting:{sess_key}")) {
                dedup = true;
            } else {
                let clean = message.trim().to_string();
                let detail = if clean.is_empty() { String::new() } else { format!(" : {}", js::slice_chars(&clean, 140)) };
                let text = format!("{label} attend votre réponse{proj_label}{detail}.");
                (self.log)(format!("[ai-watch] {text}"));
                o.notify(json!({
                    "text": text, "title": format!("{label}{proj_label}"), "source": source, "speak": speak, "card": false, // l'état est dans la card TERMINAL
                    "rows": [{ "icon": "?", "tone": "danger", "title": if clean.is_empty() { format!("{label} attend une réponse") } else { clean }, "sub": "Réponse ou confirmation requise" }]
                }));
                sent = json!("waiting");
            }
        }
        o.terminal_voice.update_ai_status();
        json!({ "ok": true, "notified": sent, "deduplicated": dedup })
    }
}
