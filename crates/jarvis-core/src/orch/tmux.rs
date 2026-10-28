// Pilotage de tmux (orchestrator/tmux.js) : sessions, fenêtres, volets, saisie.
// Launcher « native » (défaut) : psmux, tmux natif Windows ; launcher « wsl » : le vrai tmux dans WSL.
// Toutes les commandes partent sans shell : le texte dicté arrive tel quel. Il voyage en argument de
// set-buffer avec psmux, par l'entrée standard avec WSL.

use std::collections::{HashMap, HashSet, VecDeque};
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::Mutex;

use serde_json::{json, Value};

use crate::js::{self, J};

pub fn defaults() -> J {
    json!({
        "launcher": "native", "distro": "Ubuntu", "tmuxPath": "psmux", "session": "jarvis",
        "claudeCommand": "claude", "defaultDir": "", "enterDelayMs": 250, "showTerminal": true
    })
}

/// Programmes qui reçoivent une consigne en langage naturel : la leur écrire n'exécute rien.
const AGENTS: &[&str] = &["claude", "codex", "gemini", "aider", "agy", "opencode", "cursor-agent", "copilot"];

static RESOLVED: Mutex<Option<HashMap<String, String>>> = Mutex::new(None);

/// Exécutable natif : chemin réglé, sinon PATH puis dossiers de WinGet.
pub fn resolve_native(name: &str) -> String {
    let want = if name.is_empty() { "psmux".to_string() } else { name.to_string() };
    if let Some(f) = RESOLVED.lock().unwrap().as_ref().and_then(|m| m.get(&want).cloned()) {
        return f;
    }
    let mut found = None;
    if Path::new(&want).is_absolute() {
        found = Path::new(&want).exists().then(|| want.clone());
    } else {
        let exe = if want.to_lowercase().ends_with(".exe") { want.clone() } else { format!("{want}.exe") };
        let local = std::env::var("LOCALAPPDATA").unwrap_or_default();
        let mut dirs: Vec<PathBuf> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
        dirs.push(Path::new(&local).join("Microsoft").join("WinGet").join("Links"));
        let pkgs = Path::new(&local).join("Microsoft").join("WinGet").join("Packages");
        if let Ok(rd) = std::fs::read_dir(&pkgs) {
            for e in rd.filter_map(|e| e.ok()) {
                if e.file_name().to_string_lossy().to_lowercase().contains("psmux") {
                    dirs.push(e.path());
                }
            }
        }
        for d in dirs {
            let c = d.join(&exe);
            if c.exists() {
                found = Some(c.to_string_lossy().to_string());
                break;
            }
        }
    }
    match found {
        Some(f) => {
            RESOLVED.lock().unwrap().get_or_insert_with(HashMap::new).insert(want, f.clone());
            f
        }
        None => want,
    }
}

/// « E:\www\A-En cours\jarvis » → « /mnt/e/www/A-En cours/jarvis ».
pub fn to_wsl_path(win: &str) -> String {
    match crate::re!(r"^([a-zA-Z]):[\\/]?(.*)$").captures(win) {
        None => win.replace('\\', "/"),
        Some(m) => {
            let p = format!("/mnt/{}/{}", m[1].to_lowercase(), m[2].replace('\\', "/"));
            p.trim_end_matches('/').to_string()
        }
    }
}

/// Erreur d'une commande tmux : motif (no-distro, no-tmux, no-session, no-window, error).
#[derive(Debug, Clone)]
pub struct TmuxError {
    pub reason: String,
    pub code: i32,
    pub tmux: String,
    pub message: String,
}

impl std::fmt::Display for TmuxError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", if self.tmux.is_empty() { &self.message } else { &self.tmux })
    }
}

pub fn reason_of(enoent: bool, text: &str) -> &'static str {
    if enoent {
        return "no-tmux";
    }
    if crate::re!(r"(?i)DISTRO_NOT_FOUND|aucune distribution|no distribution").is_match(text) {
        "no-distro"
    } else if crate::re!(r"(?i)execvpe?\(tmux\)|tmux: (?:not found|command not found)|No such file or directory").is_match(text) {
        "no-tmux"
    } else if crate::re!(r"(?i)no server running|can't find session|no current (?:session|client)|error connecting").is_match(text) {
        "no-session"
    } else if crate::re!(r"(?i)can't find (?:window|pane)|window not found|pane not found").is_match(text) {
        "no-window"
    } else {
        "error"
    }
}

fn no_session(e: &TmuxError) -> bool {
    e.reason == "no-session" || (e.code == 1 && e.tmux.is_empty())
}

pub struct Tmux {
    get_config: Box<dyn Fn() -> J>,
    log: Rc<dyn Fn(String)>,
}

impl Tmux {
    pub fn new(get_config: Box<dyn Fn() -> J>, log: Rc<dyn Fn(String)>) -> Self {
        Self { get_config, log }
    }

    pub fn cfg(&self) -> J {
        let mut d = defaults();
        js::assign(&mut d, (self.get_config)());
        d
    }

    pub fn session(&self) -> String {
        let s = js::gs(&self.cfg(), "session");
        if s.is_empty() { "jarvis".into() } else { s }
    }

    fn native(&self) -> bool {
        js::gs(&self.cfg(), "launcher") == "native"
    }

    /// Dossier Windows → dossier tel que tmux le voit.
    pub fn dir(&self, win: &str) -> String {
        if win.is_empty() {
            String::new()
        } else if self.native() {
            win.to_string()
        } else {
            to_wsl_path(win)
        }
    }

    /// Commande tmux brute.
    pub async fn run(&self, args: &[&str], input: Option<&str>, timeout_ms: u64) -> Result<String, TmuxError> {
        let cfg = self.cfg();
        let clean: Vec<String> = args.iter().filter(|a| !a.is_empty()).map(|a| a.to_string()).collect();
        let (file, argv) = if js::gs(&cfg, "launcher") == "native" {
            (resolve_native(&js::gs(&cfg, "tmuxPath")), clean)
        } else {
            let mut v = vec![];
            let distro = js::gs(&cfg, "distro");
            if !distro.is_empty() {
                v.push("-d".to_string());
                v.push(distro);
            }
            v.push("-e".into());
            v.push("tmux".into());
            v.extend(clean);
            ("wsl.exe".to_string(), v)
        };
        let mut cmd = tokio::process::Command::new(&file);
        cmd.args(&argv).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).kill_on_drop(true);
        #[cfg(windows)]
        cmd.creation_flags(0x0800_0000);
        let mut child = match cmd.spawn() {
            Ok(c) => c,
            Err(e) => {
                let enoent = e.kind() == std::io::ErrorKind::NotFound;
                return Err(TmuxError { reason: reason_of(enoent, &e.to_string()).into(), code: -1, tmux: String::new(), message: e.to_string() });
            }
        };
        if let Some(mut stdin) = child.stdin.take() {
            use tokio::io::AsyncWriteExt;
            let _ = stdin.write_all(input.unwrap_or("").as_bytes()).await;
            drop(stdin);
        }
        let out = match tokio::time::timeout(std::time::Duration::from_millis(timeout_ms), child.wait_with_output()).await {
            Err(_) => return Err(TmuxError { reason: "error".into(), code: -1, tmux: String::new(), message: "délai dépassé".into() }),
            Ok(Err(e)) => return Err(TmuxError { reason: "error".into(), code: -1, tmux: String::new(), message: e.to_string() }),
            Ok(Ok(o)) => o,
        };
        let stdout = String::from_utf8_lossy(&out.stdout).replace('\0', "");
        if !out.status.success() {
            let stderr = String::from_utf8_lossy(&out.stderr).replace('\0', "");
            let tmux = if stderr.trim().is_empty() { stdout.trim().to_string() } else { stderr.trim().to_string() };
            let code = out.status.code().unwrap_or(-1);
            return Err(TmuxError { reason: reason_of(false, &tmux).into(), code, tmux, message: format!("code {code}") });
        }
        Ok(stdout)
    }

    async fn r(&self, args: &[&str]) -> Result<String, TmuxError> {
        self.run(args, None, 10000).await
    }

    pub async fn version(&self) -> Result<String, TmuxError> {
        Ok(self.r(&["-V"]).await?.trim().to_string())
    }

    pub async fn has_session(&self) -> Result<bool, TmuxError> {
        match self.r(&["has-session", "-t", &format!("={}", self.session())]).await {
            Ok(_) => Ok(true),
            Err(e) if no_session(&e) => Ok(false),
            Err(e) => Err(e),
        }
    }

    /// Crée la session (détachée) si besoin. Retourne la fenêtre initiale si elle vient d'être créée.
    pub async fn ensure_session(&self, win_dir: &str) -> Result<Option<String>, TmuxError> {
        if self.has_session().await? {
            return Ok(None);
        }
        let d = if win_dir.is_empty() { js::gs(&self.cfg(), "defaultDir") } else { win_dir.to_string() };
        let dir = self.dir(&d);
        let s = self.session();
        let mut args = vec!["new-session", "-d", "-P", "-F", "#{session_name}:#{window_index}", "-s", &s, "-x", "200", "-y", "50"];
        if !dir.is_empty() {
            args.push("-c");
            args.push(&dir);
        }
        Ok(Some(self.r(&args).await?.trim().to_string()))
    }

    pub async fn attached(&self, session: Option<&str>) -> bool {
        let s = session.map(String::from).unwrap_or_else(|| self.session());
        self.r(&["list-clients", "-t", &format!("={s}")]).await.map(|o| !o.trim().is_empty()).unwrap_or(false)
    }

    /// Onglet Windows Terminal attaché à la session ; repli : le client tmux seul.
    pub fn open_terminal(&self, session: Option<&str>) -> bool {
        let cfg = self.cfg();
        let s = session.map(String::from).unwrap_or_else(|| self.session());
        let title = format!("tmux · {s}");
        let attach: Vec<String> = if js::gs(&cfg, "launcher") == "native" {
            vec![resolve_native(&js::gs(&cfg, "tmuxPath")), "attach-session".into(), "-t".into(), s.clone()]
        } else {
            let mut v = vec!["wsl.exe".to_string()];
            let distro = js::gs(&cfg, "distro");
            if !distro.is_empty() {
                v.push("-d".into());
                v.push(distro);
            }
            v.extend(["-e", "tmux", "new-session", "-A", "-s"].iter().map(|x| x.to_string()));
            v.push(s.clone());
            v
        };
        let mut wt: Vec<String> = vec!["-w".into(), "jarvis".into(), "new-tab".into(), "--title".into(), title, "--suppressApplicationTitle".into()];
        wt.extend(attach.iter().cloned());
        crate::proc::detached_visible("wt.exe", &wt.iter().map(|x| x.as_str()).collect::<Vec<_>>()) || crate::proc::detached_visible(&attach[0], &attach[1..].iter().map(|x| x.as_str()).collect::<Vec<_>>())
    }

    /// Fenêtres de la session : [{ index, name, active, command, title, path, target }].
    pub async fn windows(&self) -> Result<Vec<J>, TmuxError> {
        let s = self.session();
        let out = self
            .r(&["list-windows", "-t", &format!("={s}"), "-F", "#{window_index}\t#{window_name}\t#{window_active}\t#{pane_current_command}\t#{pane_title}\t#{pane_current_path}"])
            .await?;
        Ok(out
            .split('\n')
            .filter(|l| !l.is_empty())
            .map(|line| {
                let p: Vec<&str> = line.split('\t').collect();
                let g = |i: usize| p.get(i).copied().unwrap_or("").trim_end_matches('\r').to_string();
                let index: i64 = g(0).parse().unwrap_or(0);
                json!({ "index": index, "name": g(1), "active": g(2) == "1", "command": g(3), "title": g(4), "path": g(5), "target": format!("{s}:{index}") })
            })
            .collect())
    }

    /// Fenêtre désignée à la voix : numéro, nom (« claude »), ou programme qui y tourne.
    pub async fn find_window(&self, spoken: &str) -> Result<Option<J>, TmuxError> {
        let list = self.windows().await?;
        let s = spoken.trim().to_lowercase();
        if s.is_empty() {
            return Ok(list.into_iter().find(|w| js::gb(w, "active")));
        }
        if s.bytes().all(|b| b.is_ascii_digit()) {
            let n: i64 = s.parse().unwrap_or(-1);
            return Ok(list.into_iter().find(|w| w["index"].as_i64() == Some(n)));
        }
        let name = |w: &J| js::gs(w, "name").to_lowercase();
        Ok(list
            .iter()
            .find(|w| name(w) == s)
            .or_else(|| list.iter().find(|w| name(w).starts_with(&s) || js::gs(w, "command").to_lowercase() == s))
            .or_else(|| if s == "claude" { list.iter().find(|w| is_claude(w)) } else { None })
            .cloned())
    }

    /// psmux : la commande part avec la fenêtre, dans un PowerShell qui reste ouvert après elle.
    pub fn shell_command(&self, command: &str) -> Option<Vec<String>> {
        if command.is_empty() || !self.native() {
            return None;
        }
        let shell = {
            let s = js::gs(&self.cfg(), "shell");
            if !s.is_empty() {
                s
            } else {
                let pwsh = resolve_native("pwsh");
                if Path::new(&pwsh).is_absolute() { pwsh } else { "powershell.exe".into() }
            }
        };
        Some(vec![shell, "-NoLogo".into(), "-NoExit".into(), "-Command".into(), command.to_string()])
    }

    /// Nouvelle fenêtre avec un shell qui exécute la commande.
    pub async fn new_window(&self, name: &str, win_dir: &str, command: &str) -> Result<String, TmuxError> {
        let d = if win_dir.is_empty() { js::gs(&self.cfg(), "defaultDir") } else { win_dir.to_string() };
        let dir = self.dir(&d);
        let direct = self.shell_command(command);
        let s = self.session();
        if let Some(direct) = &direct {
            if !self.has_session().await? {
                let mut args: Vec<&str> = vec!["new-session", "-d", "-P", "-F", "#{session_name}:#{window_index}", "-s", &s, "-x", "200", "-y", "50"];
                if !dir.is_empty() {
                    args.push("-c");
                    args.push(&dir);
                }
                args.extend(direct.iter().map(|x| x.as_str()));
                let target = self.r(&args).await?.trim().to_string();
                if !name.is_empty() {
                    self.r(&["rename-window", "-t", &target, name]).await?;
                }
                return Ok(target);
            }
        }
        let fresh = if direct.is_some() { None } else { self.ensure_session(win_dir).await? };
        let target = if let Some(fresh) = fresh {
            if !name.is_empty() {
                self.r(&["rename-window", "-t", &fresh, name]).await?;
            }
            fresh
        } else {
            let t = format!("{s}:");
            let mut args: Vec<&str> = vec!["new-window", "-P", "-F", "#{session_name}:#{window_index}", "-t", &t];
            if !name.is_empty() {
                args.push("-n");
                args.push(name);
            }
            if !dir.is_empty() {
                args.push("-c");
                args.push(&dir);
            }
            if let Some(d) = &direct {
                args.extend(d.iter().map(|x| x.as_str()));
            }
            self.r(&args).await?.trim().to_string()
        };
        if !command.is_empty() && direct.is_none() {
            js::sleep_ms(400).await;
            self.send_line(&target, command, true).await?;
        }
        Ok(target)
    }

    /// Texte + Entrée (envoyée séparément, après un court délai).
    pub async fn send_line(&self, target: &str, text: &str, enter: bool) -> Result<(), TmuxError> {
        if self.native() {
            let session = target.split(':').next().filter(|s| !s.is_empty()).map(String::from).unwrap_or_else(|| self.session());
            self.r(&["set-buffer", "-t", &session, "-b", "jarvis-voix", "--", text]).await?;
            self.r(&["paste-buffer", "-d", "-b", "jarvis-voix", "-t", target]).await?;
        } else {
            self.run(&["load-buffer", "-b", "jarvis-voix", "-", ";", "paste-buffer", "-d", "-b", "jarvis-voix", "-t", target], Some(text), 10000).await?;
        }
        if !enter {
            return Ok(());
        }
        let delay = js::gn_or(&self.cfg(), "enterDelayMs", 0.0).max(0.0) as u64;
        js::sleep_ms(delay).await;
        self.r(&["send-keys", "-t", target, "Enter"]).await?;
        Ok(())
    }

    pub async fn send_keys(&self, target: &str, keys: &[&str]) -> Result<(), TmuxError> {
        let mut args = vec!["send-keys", "-t", target];
        args.extend_from_slice(keys);
        self.r(&args).await.map(|_| ())
    }

    /// Dernières lignes affichées dans le volet (lignes repliées recollées).
    pub async fn capture(&self, target: &str, lines: i64) -> Result<String, TmuxError> {
        let start = format!("-{}", lines.max(1));
        Ok(self.r(&["capture-pane", "-p", "-J", "-t", target, "-S", &start]).await?.trim_end().to_string())
    }

    /// Écran visible du volet seulement.
    pub async fn capture_screen(&self, target: &str) -> Result<String, TmuxError> {
        Ok(self.r(&["capture-pane", "-p", "-J", "-t", target]).await?.trim_end().to_string())
    }

    pub async fn pane_info(&self, target: &str) -> Result<J, TmuxError> {
        let out = self.r(&["display-message", "-p", "-t", target, "#{pane_current_command}\t#{pane_title}\t#{window_name}"]).await?;
        let p: Vec<&str> = out.trim().split('\t').collect();
        Ok(json!({ "command": p.first().unwrap_or(&""), "title": p.get(1).unwrap_or(&""), "name": p.get(2).unwrap_or(&"") }))
    }

    pub async fn select(&self, target: &str) -> Result<(), TmuxError> {
        self.r(&["select-window", "-t", target]).await.map(|_| ())
    }

    pub async fn select_relative(&self, next: bool) -> Result<(), TmuxError> {
        let t = format!("{}:{}", self.session(), if next { "+" } else { "-" });
        self.r(&["select-window", "-t", &t]).await.map(|_| ())
    }

    pub async fn split(&self, target: &str, side: bool) -> Result<(), TmuxError> {
        self.r(&["split-window", if side { "-h" } else { "-v" }, "-t", target, "-c", "#{pane_current_path}"]).await.map(|_| ())
    }

    pub async fn kill_window(&self, target: &str) -> Result<(), TmuxError> {
        self.r(&["kill-window", "-t", target]).await.map(|_| ())
    }

    pub async fn kill_pane(&self, target: &str) -> Result<(), TmuxError> {
        self.r(&["kill-pane", "-t", target]).await.map(|_| ())
    }

    pub async fn kill_session(&self) -> Result<(), TmuxError> {
        self.r(&["kill-session", "-t", &format!("={}", self.session())]).await.map(|_| ())
    }

    /// Tous les volets de toutes les sessions. Aucun serveur tmux : liste vide.
    pub async fn list_panes(&self) -> Result<Vec<J>, TmuxError> {
        let out = match self
            .r(&["list-panes", "-a", "-F", "#{session_name}\t#{window_index}\t#{pane_index}\t#{window_name}\t#{pane_current_command}\t#{pane_title}\t#{pane_current_path}\t#{pane_pid}\t#{pane_active}\t#{window_active}"])
            .await
        {
            Ok(o) => o,
            Err(e) if no_session(&e) => return Ok(vec![]),
            Err(e) => return Err(e),
        };
        Ok(out
            .split('\n')
            .filter(|l| !l.is_empty())
            .map(|line| {
                let p: Vec<&str> = line.split('\t').collect();
                let g = |i: usize| p.get(i).copied().unwrap_or("").trim_end_matches('\r').to_string();
                let (session, window, pane) = (g(0), g(1).parse::<i64>().unwrap_or(0), g(2).parse::<i64>().unwrap_or(0));
                json!({
                    "target": format!("{session}:{window}.{pane}"), "session": session, "window": window, "pane": pane,
                    "windowName": g(3), "command": g(4), "title": g(5), "path": g(6), "pid": g(7).parse::<i64>().unwrap_or(0),
                    "active": g(8) == "1" && g(9) == "1"
                })
            })
            .collect())
    }

    pub fn log(&self, m: String) {
        (self.log)(m)
    }
}

// --- Agents qui tournent dans un volet --------------------------------------------------------------

#[derive(Clone)]
pub struct Proc {
    pub pid: u32,
    pub ppid: u32,
    pub name: String,
}

/// Table des processus (natif, instantané). La ligne de commande n'est lue que pour node.exe.
pub fn process_table() -> HashMap<u32, Proc> {
    jarvis_native::window::processes().into_iter().map(|(pid, ppid, name)| (pid, Proc { pid, ppid, name })).collect()
}

fn agent_name(exe: &str) -> Option<&'static str> {
    Some(match exe {
        "claude" => "Claude Code",
        "codex" => "Codex",
        "gemini" => "Gemini",
        "aider" => "Aider",
        "agy" => "AGY",
        "opencode" => "OpenCode",
        _ => return None,
    })
}

/// Agent (Claude Code…) parmi les descendants du processus `root` : (nom, pid).
pub fn find_agent_process(root: u32, table: &HashMap<u32, Proc>) -> Option<(&'static str, u32)> {
    if root == 0 || table.is_empty() {
        return None;
    }
    let mut children: HashMap<u32, Vec<&Proc>> = HashMap::new();
    for p in table.values() {
        children.entry(p.ppid).or_default().push(p);
    }
    let mut queue = VecDeque::from([(root, 0)]);
    let mut seen = HashSet::new();
    while let Some((pid, depth)) = queue.pop_front() {
        if !seen.insert(pid) || depth > 6 {
            continue;
        }
        for c in children.get(&pid).cloned().unwrap_or_default() {
            let lower = c.name.to_lowercase();
            let exe = lower.trim_end_matches(".exe");
            if let Some(n) = agent_name(exe) {
                return Some((n, c.pid));
            }
            if exe == "node" && crate::re!(r#"(?i)claude-code|@anthropic-ai[\\/]claude|[\\/]claude(?:\.js|\.cmd)?["\s]"#).is_match(&jarvis_native::window::command_line(c.pid)) {
                return Some(("Claude Code", c.pid));
            }
            queue.push_back((c.pid, depth + 1));
        }
    }
    None
}

/// Le processus existe-t-il encore ?
pub fn process_alive(pid: u32) -> bool {
    pid != 0 && jarvis_native::window::processes().iter().any(|(p, _, _)| *p == pid)
}

pub fn is_claude(w: &J) -> bool {
    let cmd = js::gs(w, "command").to_lowercase();
    let cmd = cmd.trim_end_matches(".exe");
    crate::re!(r"(?i)^claude").is_match(&js::gs(w, "name")) || cmd == "claude" || (cmd == "node" && crate::re!(r"(?i)claude").is_match(&js::gs(w, "title")))
}

/// Un volet où tourne un agent reçoit des consignes ; ailleurs, taper = exécuter.
pub fn is_agent(info: &J) -> bool {
    let cmd = js::gs(info, "command").to_lowercase();
    let cmd = cmd.trim_end_matches(".exe");
    AGENTS.contains(&cmd) || (cmd == "node" && crate::re!(r"(?i)claude|codex|gemini").is_match(&js::gs(info, "title")))
}

#[allow(unused)]
fn _v(_: Value) {}
