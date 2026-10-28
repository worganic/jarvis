// Lancement de processus (child_process de la v3) : détachés, avec sortie capturée, PowerShell,
// arrêt d'un arbre de processus. Aucune console ne s'ouvre (CREATE_NO_WINDOW).

use std::os::windows::process::CommandExt;
use std::process::Stdio;
use std::time::Duration;

use tokio::io::AsyncWriteExt;

pub const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const DETACHED_PROCESS: u32 = 0x0000_0008;
const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;

/// spawn(…, { detached: true, stdio: 'ignore' }).unref()
pub fn detached(exe: &str, args: &[&str]) -> bool {
    std::process::Command::new(exe)
        .args(args)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW | DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP)
        .spawn()
        .is_ok()
}

/// Lancement détaché avec une console visible (Windows Terminal, scripts de relance…).
pub fn detached_visible(exe: &str, args: &[&str]) -> bool {
    std::process::Command::new(exe).args(args).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).creation_flags(CREATE_NEW_PROCESS_GROUP).spawn().is_ok()
}

pub struct Output {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// Exécute un programme et attend sa fin (stdin facultatif, délai maximal).
pub async fn run(exe: &str, args: &[String], input: Option<&str>, timeout_ms: u64) -> Result<Output, String> {
    let mut cmd = tokio::process::Command::new(exe);
    cmd.args(args)
        .stdin(if input.is_some() { Stdio::piped() } else { Stdio::null() })
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .creation_flags(CREATE_NO_WINDOW);
    let mut child = cmd.spawn().map_err(|e| e.to_string())?;
    if let Some(text) = input {
        if let Some(mut stdin) = child.stdin.take() {
            let data = text.as_bytes().to_vec();
            tokio::task::spawn_local(async move {
                let _ = stdin.write_all(&data).await;
                let _ = stdin.shutdown().await;
            });
        }
    }
    let res = tokio::time::timeout(Duration::from_millis(timeout_ms), child.wait_with_output()).await;
    match res {
        Err(_) => Err(format!("{exe} : délai dépassé")),
        Ok(Err(e)) => Err(e.to_string()),
        Ok(Ok(o)) => Ok(Output {
            code: o.status.code().unwrap_or(-1),
            stdout: decode(&o.stdout),
            stderr: decode(&o.stderr),
        }),
    }
}

/// Texte d'une sortie : UTF-8, sinon page de code OEM/ANSI (outils Windows en français).
pub fn decode(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => bytes.iter().map(|&b| cp850(b)).collect(),
    }
}

// Page de code 850 (console française) : assez pour les accents des messages de Windows.
fn cp850(b: u8) -> char {
    const HIGH: &str = "ÇüéâäàåçêëèïîìÄÅÉæÆôöòûùÿÖÜø£Ø×ƒáíóúñÑªº¿®¬½¼¡«»░▒▓│┤ÁÂÀ©╣║╗╝¢¥┐└┴┬├─┼ãÃ╚╔╩╦╠═╬¤ðÐÊËÈıÍÎÏ┘┌█▄¦Ì▀ÓßÔÒõÕµþÞÚÛÙýÝ¯´\u{ad}±‗¾¶§÷¸°¨·¹³²■\u{a0}";
    if b < 128 {
        b as char
    } else {
        HIGH.chars().nth((b - 128) as usize).unwrap_or('?')
    }
}

/// Script PowerShell passé sur l'entrée standard (runPowerShell de la v3).
pub async fn powershell(script: &str) -> Result<String, String> {
    let args: Vec<String> = ["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-Command", "-"].iter().map(|s| s.to_string()).collect();
    let script = format!("[Console]::OutputEncoding = [System.Text.Encoding]::UTF8\n{script}");
    let o = run("powershell.exe", &args, Some(&script), 60000).await?;
    if o.code == 0 {
        Ok(o.stdout.trim().to_string())
    } else {
        Err(if o.stderr.trim().is_empty() { format!("PowerShell a quitté avec le code {}", o.code) } else { o.stderr.trim().to_string() })
    }
}

/// Commande « shell » (exec de la v3) : cmd.exe /c.
pub async fn exec(command: &str) -> Result<Output, String> {
    run("cmd.exe", &["/d".into(), "/s".into(), "/c".into(), command.into()], None, 30000).await
}

/// Tue un processus et tous ses sous-processus (taskkill /T /F).
pub fn kill_tree(pid: u32) {
    let _ = std::process::Command::new("taskkill")
        .args(["/pid", &pid.to_string(), "/T", "/F"])
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn();
}

/// Cherche un exécutable dans le PATH (et éventuellement des chemins candidats).
pub fn which(names: &[&str], extra: &[std::path::PathBuf]) -> Option<std::path::PathBuf> {
    for p in extra {
        if p.is_file() {
            return Some(p.clone());
        }
    }
    let path = std::env::var("PATH").unwrap_or_default();
    for dir in std::env::split_paths(&path) {
        for n in names {
            let p = dir.join(n);
            if p.is_file() {
                return Some(p);
            }
        }
    }
    None
}

// --- Processus résident dialoguant en lignes (veille du bureau, mixeur audio…) -----------------------

/// Processus enfant : lignes de sa sortie standard rendues une à une, écriture de lignes sur son entrée.
/// Lie un auxiliaire de longue durée (moteur vocal, veilleurs) au noyau : il est placé dans une tâche
/// Windows (job) que le système ferme avec le noyau, même tué ou planté, en emportant ses processus.
/// Sans cela, un moteur vocal orphelin gardait le port 8798 (voix hors ligne) et jarvis-core.exe
/// verrouillé (relance sans recompilation du noyau) : 21/10/2026. Jamais pour une application
/// que Jarvis ouvre pour l'utilisateur : elle se fermerait avec le noyau.
pub fn tie_to_core(pid: u32) {
    use std::sync::OnceLock;
    use windows::core::PCWSTR;
    use windows::Win32::Foundation::{CloseHandle, HANDLE};
    use windows::Win32::System::JobObjects::{
        AssignProcessToJobObject, CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject,
        JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
    };
    use windows::Win32::System::Threading::{OpenProcess, PROCESS_SET_QUOTA, PROCESS_TERMINATE};
    // Poignée gardée ouverte toute la vie du noyau (jamais fermée : c'est sa fermeture qui arrête la tâche).
    static JOB: OnceLock<Option<isize>> = OnceLock::new();
    let job = JOB.get_or_init(|| unsafe {
        let job = CreateJobObjectW(None, PCWSTR::null()).ok()?;
        let mut info = JOBOBJECT_EXTENDED_LIMIT_INFORMATION::default();
        info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        let size = std::mem::size_of::<JOBOBJECT_EXTENDED_LIMIT_INFORMATION>() as u32;
        SetInformationJobObject(job, JobObjectExtendedLimitInformation, &info as *const _ as *const _, size).ok()?;
        Some(job.0 as isize)
    });
    let (Some(job), true) = (*job, pid > 0) else { return };
    unsafe {
        if let Ok(h) = OpenProcess(PROCESS_SET_QUOTA | PROCESS_TERMINATE, false, pid) {
            let _ = AssignProcessToJobObject(HANDLE(job as _), h);
            let _ = CloseHandle(h);
        }
    }
}

pub struct LineChild {
    pub pid: u32,
    stdin: tokio::sync::Mutex<Option<tokio::process::ChildStdin>>,
    kill: std::cell::RefCell<Option<tokio::sync::oneshot::Sender<()>>>,
}

impl LineChild {
    /// Lance `exe args` ; `on_line` reçoit chaque ligne non vide, `on_exit` le code de sortie.
    pub fn spawn(
        exe: &std::path::Path,
        args: &[String],
        envs: &[(String, String)],
        on_line: impl Fn(String) + 'static,
        on_stderr: impl Fn(String) + 'static,
        on_exit: impl FnOnce(Option<i32>) + 'static,
    ) -> Result<std::rc::Rc<LineChild>, String> {
        Self::spawn_in(None, exe, args, envs, on_line, on_stderr, on_exit)
    }

    /// Comme `spawn`, dans le dossier de travail `cwd`.
    pub fn spawn_in(
        cwd: Option<&std::path::Path>,
        exe: &std::path::Path,
        args: &[String],
        envs: &[(String, String)],
        on_line: impl Fn(String) + 'static,
        on_stderr: impl Fn(String) + 'static,
        on_exit: impl FnOnce(Option<i32>) + 'static,
    ) -> Result<std::rc::Rc<LineChild>, String> {
        use tokio::io::AsyncBufReadExt;
        let mut cmd = tokio::process::Command::new(exe);
        if let Some(d) = cwd {
            cmd.current_dir(d);
        }
        cmd.args(args).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::piped()).kill_on_drop(true).creation_flags(CREATE_NO_WINDOW);
        for (k, v) in envs {
            cmd.env(k, v);
        }
        let mut child = cmd.spawn().map_err(|e| e.to_string())?;
        let pid = child.id().unwrap_or(0);
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let (ktx, krx) = tokio::sync::oneshot::channel::<()>();
        let me = std::rc::Rc::new(LineChild { pid, stdin: tokio::sync::Mutex::new(child.stdin.take()), kill: std::cell::RefCell::new(Some(ktx)) });
        if let Some(err) = stderr {
            tokio::task::spawn_local(async move {
                let mut lines = tokio::io::BufReader::new(err).lines();
                while let Ok(Some(l)) = lines.next_line().await {
                    on_stderr(l);
                }
            });
        }
        tokio::task::spawn_local(async move {
            if let Some(out) = stdout {
                let mut reader = tokio::io::BufReader::new(out);
                let mut buf = Vec::new();
                loop {
                    buf.clear();
                    match reader.read_until(b'\n', &mut buf).await {
                        Ok(0) | Err(_) => break,
                        Ok(_) => {
                            let line = decode(&buf).trim().to_string();
                            if !line.is_empty() {
                                on_line(line);
                            }
                        }
                    }
                }
            }
            let code = tokio::select! {
                st = child.wait() => st.ok().and_then(|s| s.code()),
                _ = krx => { let _ = child.kill().await; None }
            };
            on_exit(code);
        });
        Ok(me)
    }

    pub async fn write_line(&self, line: &str) -> bool {
        let mut guard = self.stdin.lock().await;
        let Some(s) = guard.as_mut() else { return false };
        s.write_all(format!("{line}\n").as_bytes()).await.is_ok() && s.flush().await.is_ok()
    }

    /// Écrit `text` tel quel puis ferme l'entrée standard (demande passée hors ligne de commande).
    pub async fn write_and_close(&self, text: &str) -> bool {
        let mut guard = self.stdin.lock().await;
        let ok = match guard.as_mut() {
            Some(s) => s.write_all(text.as_bytes()).await.is_ok() && s.flush().await.is_ok(),
            None => false,
        };
        guard.take();
        ok
    }

    /// Ferme l'entrée standard (le processus sait qu'il doit s'arrêter).
    pub fn close_stdin(&self) {
        if let Ok(mut g) = self.stdin.try_lock() {
            g.take();
        }
    }

    pub fn kill(&self) {
        if let Some(k) = self.kill.borrow_mut().take() {
            let _ = k.send(());
        }
    }
}
