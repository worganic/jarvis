// Relance complète de Jarvis après une modification de son code (tools/relaunch.js de la v3), lancée
// par le noyau dans sa propre console :
//   1. attend la fermeture de jarvis.exe (on ne peut pas remplacer un exécutable en cours) ;
//   2. recompile (cargo build --release : noyau, moteur vocal, coquille qui intègre ui/) ;
//   3. rouvre Jarvis — l'ancienne version si la compilation échoue, avec le motif pour le noyau.
//
// L'écran est redessiné sur place (logo, étapes, barre de progression lue dans la sortie de cargo, nouveautés
// de l'historique) dans une fenêtre compacte (COLS × ROWS). Si la sortie n'est pas une console (redirection),
// il se contente d'un journal ligne à ligne.
//
//   jarvis-core relaunch --pid <pid de jarvis.exe> [--build]
//   jarvis-core relaunch --apercu     (démonstration de l'écran : ne ferme, ne compile, ne rouvre rien)
//
// Il tourne depuis une copie temporaire de jarvis-core.exe : cargo peut ainsi remplacer l'original.

use std::io::{IsTerminal, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::time::{Duration, Instant};

use serde_json::json;

const ESC: &str = "\x1b[";
/// Taille de la fenêtre de mise à jour (caractères × lignes) : l'écran est dessiné pour y tenir.
const COLS: usize = 86;
const ROWS: usize = 32;
const CYAN: [u8; 3] = [56, 225, 255];
const GREEN: [u8; 3] = [53, 255, 158];

fn rgb(c: [u8; 3], s: &str) -> String {
    format!("{ESC}38;2;{};{};{}m{s}{ESC}0m", c[0], c[1], c[2])
}
fn cyan(s: &str) -> String {
    rgb(CYAN, s)
}
fn green(s: &str) -> String {
    rgb(GREEN, s)
}
fn red(s: &str) -> String {
    rgb([255, 77, 94], s)
}
fn amber(s: &str) -> String {
    rgb([255, 196, 64], s)
}
fn white(s: &str) -> String {
    rgb([230, 250, 255], s)
}
fn dim(s: &str) -> String {
    rgb([90, 130, 150], s)
}
fn bold(s: &str) -> String {
    format!("{ESC}1m{s}{ESC}22m")
}
fn mix(a: [u8; 3], b: [u8; 3], t: f64) -> [u8; 3] {
    let f = |i: usize| (a[i] as f64 + (b[i] as f64 - a[i] as f64) * t).round() as u8;
    [f(0), f(1), f(2)]
}

const LOGO: &[&str] = &[
    "     ██╗ █████╗ ██████╗ ██╗   ██╗██╗███████╗",
    "     ██║██╔══██╗██╔══██╗██║   ██║██║██╔════╝",
    "     ██║███████║██████╔╝██║   ██║██║███████╗",
    "██   ██║██╔══██║██╔══██╗╚██╗ ██╔╝██║╚════██║",
    "╚█████╔╝██║  ██║██║  ██║ ╚████╔╝ ██║███████║",
    " ╚════╝ ╚═╝  ╚═╝╚═╝  ╚═╝  ╚═══╝  ╚═╝╚══════╝",
];

/// Logo en dégradé cyan → bleu, parcouru par un reflet lumineux.
fn logo_lines(now_ms: u128) -> Vec<String> {
    let width = LOGO[0].chars().count() as f64;
    let shine = ((now_ms as f64 / 22.0) % (width + 40.0)) - 20.0;
    LOGO.iter()
        .map(|line| {
            line.chars()
                .enumerate()
                .map(|(x, ch)| {
                    if ch == ' ' {
                        return " ".to_string();
                    }
                    let mut c = mix(CYAN, [70, 120, 255], x as f64 / width);
                    let glow = (1.0 - (x as f64 - shine).abs() / 6.0).max(0.0);
                    if glow > 0.0 {
                        c = mix(c, [255, 255, 255], glow * 0.8);
                    }
                    if "╚╝═║╔╗".contains(ch) {
                        c = mix(c, [0, 20, 40], 0.45); // l'ombre des lettres
                    }
                    rgb(c, &ch.to_string())
                })
                .collect()
        })
        .collect()
}

const SPIN: &[&str] = &["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];

fn duration(ms: u128) -> String {
    let s = (ms as f64 / 1000.0).round() as u64;
    if s < 60 {
        format!("{s} s")
    } else {
        format!("{} min {:02}", s / 60, s % 60)
    }
}

/// Barre de progression : dégradé cyan → vert ; sans total connu, un reflet va et vient.
fn bar(p: Option<f64>, width: usize, now_ms: u128) -> String {
    let mut out = String::new();
    match p {
        None => {
            let pos = (((now_ms as f64 / 400.0).sin() + 1.0) / 2.0 * (width - 6) as f64).floor() as usize;
            for i in 0..width {
                out.push_str(&if i >= pos && i < pos + 6 { cyan("█") } else { dim("░") });
            }
        }
        Some(p) => {
            let filled = (p.clamp(0.0, 1.0) * width as f64).round() as usize;
            for i in 0..width {
                out.push_str(&if i < filled { rgb(mix(CYAN, GREEN, i as f64 / width as f64), "█") } else { dim("░") });
            }
        }
    }
    out
}

#[derive(Clone, Copy, PartialEq)]
enum St {
    Wait,
    Run,
    Ok,
    Fail,
    Skip,
}

struct Step {
    id: &'static str,
    label: &'static str,
    state: St,
    t0: Option<Instant>,
    t1: Option<Instant>,
    note: String,
}

#[derive(Default)]
struct Cargo {
    done: u64,
    total: u64,
    current: String,
    log: Vec<String>,
    errors: Vec<String>,
}

struct Screen {
    tty: bool,
    started: Instant,
    steps: Vec<Step>,
    cargo: Arc<Mutex<Cargo>>,
    footer: (String, fn(&str) -> String),
    /// Version installée par cette mise à jour (première entrée de ui/changelog.json), vide si inconnue.
    version: String,
    /// Entrées de l'historique plus récentes que la version qui tournait : { version, title, changes }.
    news: Vec<serde_json::Value>,
    since: String,
}

/// Écriture à l'écran par un fil à part. Si le terminal cesse de lire (fenêtre Windows Terminal figée), seul ce
/// fil reste bloqué : la mise à jour continue (fermeture, compilation, réouverture). `frame` : image complète, qui
/// remplace celle encore en attente ; sinon le texte s'ajoute à la suite (titre, curseur).
fn paint(text: String, frame: bool) {
    type Slot = Arc<(Mutex<(Option<String>, String)>, Condvar)>;
    static SLOT: OnceLock<Slot> = OnceLock::new();
    let slot = SLOT.get_or_init(|| {
        let slot: Slot = Arc::new((Mutex::new((None, String::new())), Condvar::new()));
        let mine = slot.clone();
        std::thread::spawn(move || loop {
            let next = {
                let (m, cv) = &*mine;
                let mut g = m.lock().unwrap();
                while g.0.is_none() && g.1.is_empty() {
                    g = cv.wait(g).unwrap();
                }
                // Ajouts d'abord (titre, écran vierge, curseur), puis la dernière image.
                let tail = std::mem::take(&mut g.1);
                format!("{tail}{}", g.0.take().unwrap_or_default())
            };
            let mut out = std::io::stdout();
            let _ = out.write_all(next.as_bytes());
            let _ = out.flush();
        });
        slot
    });
    let (m, cv) = &**slot;
    {
        let mut g = m.lock().unwrap();
        if frame {
            // Une image complète rend inutile ce qui attendait encore (elle redessine tout l'écran).
            g.0 = Some(text);
        } else {
            g.1.push_str(&text);
        }
    }
    cv.notify_one();
}

/// Texte ramené à `n` caractères (« … » s'il est coupé).
fn clip(s: &str, n: usize) -> String {
    if s.chars().count() <= n {
        return s.to_string();
    }
    format!("{}…", s.chars().take(n.saturating_sub(1)).collect::<String>().trim_end())
}

/// Nouveautés en `budget` lignes au plus : chaque version (numéro, titre), puis le début de ses changements
/// tant qu'il reste de la place ; la dernière ligne annonce les versions qui n'ont pas tenu.
fn news_lines(news: &[serde_json::Value], since: &str, budget: usize) -> Vec<String> {
    use crate::js;
    if news.is_empty() || budget < 3 {
        return vec![];
    }
    let mut out = vec![format!("   {}  {}", cyan(&bold("NOUVEAUTÉS")), dim(&format!("depuis la v{since}")))];
    for (i, v) in news.iter().enumerate() {
        let rest = news.len() - i;
        // Place pour cette version, plus une ligne « … » si d'autres suivent.
        if out.len() + 1 + usize::from(rest > 1) > budget {
            out.push(format!("   {}", dim(&format!("… et {rest} autre(s) version(s) : historique dans le HUD"))));
            break;
        }
        let head = format!("v{}", js::gs(v, "version"));
        out.push(format!("   {}  {}", green(&head), white(&clip(&js::gs(v, "title"), COLS - 8 - head.len()))));
        for c in js::arr(&v["changes"]) {
            if out.len() + 1 + usize::from(rest > 1) > budget {
                break;
            }
            out.push(format!("     {} {}", dim("·"), dim(&clip(&js::s(&c), COLS - 8))));
        }
    }
    out
}

impl Screen {
    fn step(&mut self, id: &str) -> &mut Step {
        self.steps.iter_mut().find(|s| s.id == id).unwrap()
    }

    fn begin(&mut self, id: &str) {
        let tty = self.tty;
        let s = self.step(id);
        s.state = St::Run;
        s.t0 = Some(Instant::now());
        if !tty {
            println!("… {}", s.label);
        }
        self.render();
    }

    fn end(&mut self, id: &str, ok: bool, note: &str) {
        let tty = self.tty;
        let s = self.step(id);
        s.state = if ok { St::Ok } else { St::Fail };
        s.t1 = Some(Instant::now());
        s.note = note.to_string();
        if !tty {
            println!("{} {}{}", if ok { "✔" } else { "✖" }, s.label, if note.is_empty() { String::new() } else { format!(" — {note}") });
        }
        self.render();
    }

    fn render(&self) {
        if !self.tty {
            return;
        }
        let now = self.started.elapsed().as_millis();
        let cols = COLS;
        let bar_w = 40usize.min(cols.saturating_sub(34)).max(16);
        let mut lines = vec![String::new()];
        lines.extend(logo_lines(now).into_iter().map(|l| format!("   {l}")));
        lines.push(String::new());
        let version = if self.version.is_empty() { String::new() } else { format!("   {}", green(&bold(&format!("v{}", self.version)))) };
        lines.push(format!("   {}{version}   {}", cyan(&bold("M I S E   À   J O U R")), dim(&chrono::Local::now().format("%H:%M:%S").to_string())));
        lines.push(String::new());
        let cargo = self.cargo.lock().unwrap();
        for s in &self.steps {
            let icon = match s.state {
                St::Ok => green("✔"),
                St::Fail => red("✖"),
                St::Run => cyan(SPIN[(now / 90) as usize % SPIN.len()]),
                St::Skip => dim("–"),
                St::Wait => dim("○"),
            };
            let label = match s.state {
                St::Wait | St::Skip => dim(s.label),
                St::Fail => red(s.label),
                _ => white(s.label),
            };
            let time = s.t0.map(|t0| dim(&duration(s.t1.unwrap_or_else(Instant::now).duration_since(t0).as_millis()))).unwrap_or_default();
            let pad = " ".repeat(36usize.saturating_sub(s.label.chars().count()).max(2));
            let note = if s.note.is_empty() { String::new() } else if s.state == St::Fail { format!("  {}", red(&s.note)) } else { format!("  {}", dim(&s.note)) };
            lines.push(format!("   {icon}  {label}{pad}{time}{note}"));
            if s.id == "build" && s.state == St::Run {
                let p = (cargo.total > 0).then(|| cargo.done as f64 / cargo.total as f64);
                let count = match p {
                    Some(p) => format!("{}/{}  {}", cargo.done, cargo.total, white(&format!("{} %", (p * 100.0).round()))),
                    None => dim("préparation…"),
                };
                lines.push(format!("      {}  {count}", bar(p, bar_w, now)));
                if !cargo.current.is_empty() {
                    let cur: String = format!("▸ {}", cargo.current).chars().take(cols - 8).collect();
                    lines.push(format!("      {}", dim(&cur)));
                }
            }
        }
        if !cargo.errors.is_empty() {
            lines.push(String::new());
            lines.push(format!("   {}", red(&bold("Erreurs de compilation"))));
            let start = cargo.errors.len().saturating_sub(10);
            for e in &cargo.errors[start..] {
                let e: String = e.chars().take(cols - 6).collect();
                lines.push(format!("   {} {e}", red("│")));
            }
        }
        if cargo.errors.is_empty() {
            // Nouveautés dans la place restante (5 lignes gardées pour le pied).
            let news = news_lines(&self.news, &self.since, ROWS.saturating_sub(lines.len() + 5));
            if !news.is_empty() {
                lines.push(String::new());
                lines.extend(news);
            }
        }
        lines.push(String::new());
        lines.push(format!("   {}", dim(&"─".repeat(56))));
        lines.push(format!("   {}", (self.footer.1)(&self.footer.0)));
        lines.push(String::new());
        let body = lines.iter().map(|l| format!("{l}{ESC}K")).collect::<Vec<_>>().join("\n");
        paint(format!("{ESC}H{body}{ESC}J"), true);
    }
}

/// Le processus `pid` tourne-t-il encore ?
fn alive(pid: u32) -> bool {
    jarvis_native::window::processes().iter().any(|(p, _, _)| *p == pid)
}

fn sleep_rendering(screen: &Screen, ms: u64) {
    let until = Instant::now() + Duration::from_millis(ms);
    while Instant::now() < until {
        screen.render();
        std::thread::sleep(Duration::from_millis(90));
    }
}

/// cargo n'affiche sa progression que dans une console : on la force (CARGO_TERM_PROGRESS_WHEN) et on
/// lit « Building [===>  ] 452/453: jarvis(bin) » et « Compiling tauri v2… » dans sa sortie.
fn run_cargo(root: &Path, screen: &Screen) -> (Option<i32>, Option<String>) {
    let child = Command::new("cargo")
        .args(["build", "--release"])
        .current_dir(root)
        .env("CARGO_TERM_PROGRESS_WHEN", "always")
        .env("CARGO_TERM_PROGRESS_WIDTH", "100")
        .env("CARGO_TERM_COLOR", "never")
        // Seules les crates touchées sont recompilées (le reste vient du cache incrémental).
        .env("CARGO_INCREMENTAL", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(e) => return (None, Some(e.to_string())),
    };
    let tty = screen.tty;
    let reader = |stream: Box<dyn Read + Send>, cargo: Arc<Mutex<Cargo>>| {
        std::thread::spawn(move || {
            let mut in_error = false;
            let mut buf = std::io::BufReader::new(stream);
            let mut raw = Vec::new();
            loop {
                raw.clear();
                // Les lignes de progression se terminent par \r : découpe sur \r comme sur \n.
                let mut byte = [0u8; 1];
                let mut eof = true;
                while let Ok(1) = buf.read(&mut byte) {
                    eof = false;
                    if byte[0] == b'\n' || byte[0] == b'\r' {
                        break;
                    }
                    raw.push(byte[0]);
                }
                if eof && raw.is_empty() {
                    break;
                }
                let line = String::from_utf8_lossy(&raw).trim_end().to_string();
                if line.trim().is_empty() {
                    continue;
                }
                let mut c = cargo.lock().unwrap();
                if let Some(m) = crate::re!(r"Building \[.*?\]\s+(\d+)/(\d+)(?::\s*(.*))?").captures(&line) {
                    c.done = m[1].parse().unwrap_or(0);
                    c.total = m[2].parse().unwrap_or(0);
                    if let Some(cur) = m.get(3) {
                        c.current = cur.as_str().trim().to_string();
                    }
                    continue;
                }
                let comp = crate::re!(r"^\s*Compiling (\S+) v(\S+)").captures(&line).map(|m| format!("{} {}", &m[1], &m[2]));
                if let Some(cur) = &comp {
                    c.current = cur.clone();
                }
                if line.trim().starts_with("error") {
                    in_error = true;
                } else if crate::re!(r"^\s*(Compiling|Finished|warning)").is_match(&line) {
                    in_error = false;
                }
                if in_error {
                    c.errors.push(line.clone());
                }
                c.log.push(line.clone());
                if !tty && (comp.is_some() || in_error || line.contains("Finished")) {
                    println!("  {}", line.trim());
                }
            }
        })
    };
    let h1 = reader(Box::new(child.stdout.take().unwrap()), screen.cargo.clone());
    let h2 = reader(Box::new(child.stderr.take().unwrap()), screen.cargo.clone());
    let status = loop {
        screen.render();
        match child.try_wait() {
            Ok(Some(st)) => break Some(st),
            Ok(None) => std::thread::sleep(Duration::from_millis(90)),
            Err(_) => break None,
        }
    };
    let _ = h1.join();
    let _ = h2.join();
    (status.and_then(|s| s.code()), None)
}

fn enable_vt() {
    use windows::Win32::System::Console::{GetConsoleMode, GetStdHandle, SetConsoleMode, CONSOLE_MODE, ENABLE_VIRTUAL_TERMINAL_PROCESSING, STD_OUTPUT_HANDLE};
    unsafe {
        if let Ok(h) = GetStdHandle(STD_OUTPUT_HANDLE) {
            let mut mode = CONSOLE_MODE::default();
            if GetConsoleMode(h, &mut mode).is_ok() {
                let _ = SetConsoleMode(h, mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING);
            }
        }
    }
}

/// Fichiers dont une erreur ouvrirait un droit qu'on voulait fermer : politique d'accord, zones, sauvegarde.
const SENSITIVE: &[&str] = &[
    "crates/jarvis-core/src/orch/policy.rs",
    "crates/jarvis-core/src/orch/zones.rs",
    "crates/jarvis-core/src/orch/security.rs",
    "crates/jarvis-core/src/orch/core_approvals.rs",
    "crates/jarvis-core/src/backup.rs",
    // Un changement de version (nouvelle fonction) déclenche le test qui exige la mise à jour de l'aide.
    "Cargo.toml",
];

/// Exécutables de target/release que cargo remplace à la recompilation.
const OUTPUTS: &[&str] = &["jarvis-core.exe", "jarvis.exe", "jarvis-voice.exe"];

/// Un exécutable encore lancé ailleurs — serveurs MCP de Jarvis (« jarvis-core mcp … ») ouverts par un autre agent
/// IA, antivirus — ne peut pas être supprimé, et cargo échoue (« Accès refusé », os error 5). Windows permet en
/// revanche de le renommer : il est mis de côté (.old-…), ses processus continuent dessus, et une copie libre reprend
/// sa place pour que cargo puisse la remplacer. Les fichiers mis de côté sont effacés dès qu'ils ne servent plus.
/// Renvoie le nombre d'exécutables mis de côté.
fn free_outputs(root: &Path) -> usize {
    let dir = root.join("target").join("release");
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for e in rd.flatten() {
            if e.file_name().to_string_lossy().contains(".exe.old-") {
                let _ = std::fs::remove_file(e.path()); // encore utilisé : il reste, effacé une prochaine fois
            }
        }
    }
    let stamp = chrono::Local::now().format("%Y%m%d%H%M%S").to_string();
    let mut freed = 0;
    for name in OUTPUTS {
        let path = dir.join(name);
        // Ouverture en écriture refusée : le fichier est un exécutable en cours d'utilisation.
        if !path.is_file() || std::fs::OpenOptions::new().write(true).open(&path).is_ok() {
            continue;
        }
        let aside = dir.join(format!("{name}.old-{stamp}"));
        if std::fs::rename(&path, &aside).is_ok() && std::fs::copy(&aside, &path).is_ok() {
            freed += 1;
        }
    }
    freed
}

/// Un de ces fichiers est plus récent que le dernier jarvis-core.exe compilé.
fn sensitive_changed(root: &Path) -> bool {
    let built = std::fs::metadata(root.join("target").join("release").join("jarvis-core.exe")).and_then(|m| m.modified()).ok();
    SENSITIVE.iter().any(|f| {
        let t = std::fs::metadata(root.join(f)).and_then(|m| m.modified()).ok();
        match (t, built) {
            (Some(t), Some(b)) => t > b,
            (Some(_), None) => true,
            _ => false,
        }
    })
}

/// `cargo test` du noyau : (réussi, dernières lignes en cas d'échec).
/// Rafraîchit l'écran pendant l'exécution pour ne pas figer la fenêtre sur « Fermeture de Jarvis ».
fn run_tests(root: &Path, screen: &Screen) -> (bool, Vec<String>) {
    let child = Command::new("cargo")
        .args(["test", "--release", "-p", "jarvis-core", "--lib", "--quiet"])
        .current_dir(root)
        .env("CARGO_INCREMENTAL", "1")
        .env("CARGO_TERM_COLOR", "never")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn();
    let mut child = match child {
        Ok(c) => c,
        Err(e) => return (false, vec![format!("cargo introuvable ({e})")]),
    };
    let read_all = |mut stream: Box<dyn Read + Send>| {
        std::thread::spawn(move || {
            let mut s = String::new();
            let _ = stream.read_to_string(&mut s);
            s
        })
    };
    let h_out = read_all(Box::new(child.stdout.take().unwrap()));
    let h_err = read_all(Box::new(child.stderr.take().unwrap()));
    let ok = loop {
        screen.render();
        match child.try_wait() {
            Ok(Some(st)) => break st.success(),
            Ok(None) => std::thread::sleep(Duration::from_millis(90)),
            Err(_) => break false,
        }
    };
    let out = h_out.join().unwrap_or_default();
    let err = h_err.join().unwrap_or_default();
    if ok {
        (true, vec![])
    } else {
        let text = format!("{out}\n{err}");
        let lines: Vec<String> = text.lines().filter(|l| l.contains("FAILED") || l.contains("panicked") || l.starts_with("error")).take(8).map(String::from).collect();
        (false, lines)
    }
}

/// `jarvis-core relaunch …`
pub fn main(args: &[String]) -> i32 {
    let flag = |name: &str| args.iter().any(|a| a == name);
    let value = |name: &str| args.iter().position(|a| a == name).and_then(|i| args.get(i + 1)).cloned();
    let preview = flag("--apercu");
    let pid: Option<u32> = if preview { None } else { value("--pid").and_then(|p| p.parse().ok()) };
    let build = preview || flag("--build");
    let root = value("--root").map(PathBuf::from).unwrap_or_else(crate::paths::root);
    let exe = root.join("target").join("release").join("jarvis.exe");
    let result_file = crate::paths::data_dir().join("runtime").join("relaunch.json");

    let tty = std::io::stdout().is_terminal();
    if tty {
        enable_vt();
        // Console classique : fenêtre ramenée à la taille de l'écran (Windows Terminal la reçoit au lancement).
        if std::env::var_os("WT_SESSION").is_none() {
            let _ = Command::new("cmd").args(["/c", &format!("mode con: cols={COLS} lines={ROWS}")]).status();
        }
        paint(format!("\x1b]0;Jarvis — mise à jour\x07{ESC}?25l{ESC}2J"), false); // titre, curseur masqué, écran vierge
    } else {
        println!("Jarvis — mise à jour\n");
    }
    let mut steps = vec![Step { id: "close", label: "Fermeture de Jarvis", state: if pid.is_some() || preview { St::Wait } else { St::Skip }, t0: None, t1: None, note: String::new() }];
    // Sécurité touchée : les tests du noyau passent avant la recompilation (échec = version précédente conservée).
    let check = build && !preview && sensitive_changed(&root);
    if check {
        steps.push(Step { id: "tests", label: "Tests de sécurité", state: St::Wait, t0: None, t1: None, note: String::new() });
    }
    if build {
        steps.push(Step { id: "build", label: "Recompilation de Jarvis", state: St::Wait, t0: None, t1: None, note: String::new() });
    }
    steps.push(Step { id: "open", label: "Réouverture de Jarvis", state: St::Wait, t0: None, t1: None, note: String::new() });
    let mut screen = Screen {
        tty,
        started: Instant::now(),
        steps,
        cargo: Arc::new(Mutex::new(Cargo::default())),
        footer: ("Ne ferme pas cette fenêtre : Jarvis se rouvre tout seul.".into(), dim),
        version: crate::changelog::current_in(&root).map(|v| v.version).unwrap_or_default(),
        news: vec![],
        since: env!("CARGO_PKG_VERSION").to_string(),
    };
    // Cet exécutable est une copie de l'ancien noyau : sa version est celle d'avant la mise à jour.
    // L'aperçu montre les deux dernières versions.
    if preview {
        let all = crate::changelog::newer_in(&root, "0");
        screen.since = all.get(2).map(|v| crate::js::gs(v, "version")).unwrap_or_default();
        screen.news = all.into_iter().take(2).collect();
    } else {
        screen.news = crate::changelog::newer_in(&root, &screen.since);
    }
    let finish = |screen: &Screen, wait_ms: u64| {
        sleep_rendering(screen, wait_ms);
        if screen.tty {
            paint(format!("{ESC}?25h"), false);
            std::thread::sleep(Duration::from_millis(150)); // le temps de l'écrire, sans attendre un terminal figé
        }
    };

    if preview {
        screen.begin("close");
        sleep_rendering(&screen, 1200);
        screen.end("close", true, "");
        screen.begin("build");
        sleep_rendering(&screen, 800);
        let crates = ["serde", "tokio", "tauri-utils", "wry", "tao", "tauri-runtime", "tauri", "jarvis(bin)"];
        screen.cargo.lock().unwrap().total = 453;
        for i in 380..=453u64 {
            {
                let mut c = screen.cargo.lock().unwrap();
                c.done = i;
                c.current = crates[(((i - 380) as f64 / 74.0) * crates.len() as f64).floor().min((crates.len() - 1) as f64) as usize].to_string();
            }
            sleep_rendering(&screen, if i == 453 { 2500 } else { 40 });
        }
        screen.end("build", true, "");
        screen.begin("open");
        sleep_rendering(&screen, 900);
        screen.end("open", true, "");
        screen.footer = ("✔ Aperçu terminé : rien n’a été fermé, compilé ni rouvert.".into(), green);
        finish(&screen, 4000);
        return 0;
    }

    if let Some(pid) = pid {
        screen.begin("close");
        let deadline = Instant::now() + Duration::from_secs(30);
        while alive(pid) && Instant::now() < deadline {
            sleep_rendering(&screen, 200);
        }
        if alive(pid) {
            screen.end("close", false, "toujours ouvert après 30 s");
            screen.footer = ("Mise à jour abandonnée : ferme Jarvis puis relance-la.".into(), red);
            finish(&screen, 8000);
            return 1;
        }
        // Le noyau et le moteur vocal s'arrêtent avec la coquille : quelques instants de plus.
        sleep_rendering(&screen, 1500);
        screen.end("close", true, "");
    }

    let mut result = json!({ "at": crate::js::iso_now(), "build": null });
    let mut tests_failed = false;
    if check {
        screen.begin("tests");
        let t0 = Instant::now();
        let (ok, lines) = run_tests(&root, &screen);
        if ok {
            screen.end("tests", true, "");
        } else {
            tests_failed = true;
            screen.end("tests", false, "échec : la version précédente est conservée");
            screen.cargo.lock().unwrap().errors.extend(lines.clone());
            result["build"] = json!({ "ok": false, "code": null, "seconds": t0.elapsed().as_secs(), "error": "tests de sécurité en échec", "tests": lines });
        }
    }
    if build && !tests_failed {
        screen.begin("build");
        let t0 = Instant::now();
        let freed = free_outputs(&root);
        if freed > 0 {
            screen.step("build").note = format!("{freed} exécutable(s) encore utilisé(s) mis de côté");
        }
        let (code, error) = run_cargo(&root, &screen);
        let ok = code == Some(0);
        result["build"] = json!({ "ok": ok, "code": code, "seconds": t0.elapsed().as_secs(), "error": error });
        if ok {
            {
                let mut c = screen.cargo.lock().unwrap();
                if c.total > 0 {
                    c.done = c.total;
                }
            }
            // Cargo ne refait pas un exécutable dont rien n'a changé (la coquille, quand seul le noyau bouge) : il
            // garde sa vieille date, et la surveillance du code (code_watch, « la plus ancienne date fait foi »)
            // le croirait périmé à chaque démarrage. La compilation réussie les date donc tous de maintenant.
            let now = std::time::SystemTime::now();
            for name in OUTPUTS {
                if let Ok(f) = std::fs::OpenOptions::new().write(true).open(root.join("target").join("release").join(name)) {
                    let _ = f.set_modified(now);
                }
            }
            screen.end("build", true, "");
        } else {
            let note = match &error {
                Some(e) => format!("cargo introuvable ({e})"),
                None => format!("échec (code {})", code.map(|c| c.to_string()).unwrap_or_else(|| "?".into())),
            };
            screen.end("build", false, &note);
            let mut c = screen.cargo.lock().unwrap();
            if c.errors.is_empty() {
                let start = c.log.len().saturating_sub(8);
                let tail = c.log[start..].to_vec();
                c.errors.extend(tail);
            }
        }
    }
    if let Some(dir) = result_file.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let _ = std::fs::write(&result_file, serde_json::to_string_pretty(&result).unwrap_or_default());

    screen.begin("open");
    if !exe.is_file() {
        screen.end("open", false, "jarvis.exe introuvable");
        screen.footer = (format!("{} introuvable : impossible de rouvrir Jarvis.", exe.display()), red);
        finish(&screen, 15000);
        return 1;
    }
    {
        use std::os::windows::process::CommandExt;
        // Processus détaché : la console de mise à jour peut se fermer sans l'emporter.
        let _ = Command::new(&exe).current_dir(&root).creation_flags(0x0000_0008 | 0x0000_0200).spawn();
    }
    sleep_rendering(&screen, 600);
    screen.end("open", true, "");
    let failed = result["build"]["ok"] == json!(false);
    screen.footer = if failed {
        (if tests_failed { "Les tests de sécurité ont échoué : Jarvis redémarre avec la version précédente, sans recompilation." } else { "La compilation a échoué : Jarvis redémarre avec la version précédente." }.into(), amber)
    } else {
        ("✔ Jarvis est à jour et de retour. Cette fenêtre se ferme.".into(), green)
    };
    finish(&screen, if failed { 15000 } else { 1800 });
    0
}

/// Lance la relance dans sa propre console, depuis une copie de jarvis-core.exe (cargo pourra
/// remplacer l'original) : une fenêtre Windows Terminal à la taille de l'écran (COLS × ROWS), sinon une
/// console classique (PowerShell Start-Process), que l'écran redimensionne lui-même.
pub fn spawn_console(shell_pid: u32) -> Result<(), String> {
    let me = std::env::current_exe().map_err(|e| e.to_string())?;
    let copy = std::env::temp_dir().join(format!("jarvis-relaunch-{}.exe", std::process::id()));
    std::fs::copy(&me, &copy).map_err(|e| format!("copie de l'exécutable impossible ({e})"))?;
    let root = crate::paths::root();
    {
        use std::os::windows::process::CommandExt;
        let (root_s, pid_s) = (root.display().to_string(), shell_pid.to_string());
        let wt = Command::new("wt.exe")
            .args(["-w", "new", "--size", &format!("{COLS},{ROWS}"), "new-tab", "--title", "Jarvis — mise à jour", "--suppressApplicationTitle", "-d", &root_s])
            .arg(&copy)
            .args(["relaunch", "--pid", &pid_s, "--build", "--root", &root_s])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .creation_flags(0x0800_0000)
            .spawn();
        if wt.is_ok() {
            return Ok(());
        }
    }
    let q = |s: &str| format!("'{}'", s.replace('\'', "''"));
    let cmd = format!(
        "Start-Process -FilePath {} -ArgumentList @('relaunch', '--pid', '{shell_pid}', '--build', '--root', {}) -WorkingDirectory {}",
        q(&copy.display().to_string()),
        q(&format!("\"{}\"", root.display())),
        q(&root.display().to_string())
    );
    let mut c = Command::new("powershell.exe");
    c.args(["-NoProfile", "-NonInteractive", "-Command", &cmd]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null());
    {
        use std::os::windows::process::CommandExt;
        c.creation_flags(0x0800_0000);
    }
    let st = c.status().map_err(|e| e.to_string())?;
    if st.success() {
        Ok(())
    } else {
        Err(format!("PowerShell a répondu {}", st.code().unwrap_or(-1)))
    }
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nouveautes_tiennent_dans_la_place() {
        let v = |n: &str| json!({ "version": n, "title": "Titre", "changes": ["un changement très long ".repeat(10), "deux".to_string()] });
        let news = vec![v("4.3.0"), v("4.2.0"), v("4.1.0")];
        for budget in 3..12 {
            let lines = news_lines(&news, "4.0.0", budget);
            assert!(lines.len() <= budget, "budget {budget} : {} lignes", lines.len());
        }
        let few = news_lines(&news, "4.0.0", 5);
        assert!(few.last().unwrap().contains("autre(s) version(s)"), "les versions qui n'ont pas tenu sont annoncées");
        assert_eq!(news_lines(&news, "4.0.0", 20).len(), 10, "tout tient : en-tête + 3 × (titre + 2 changements)");
        assert!(news_lines(&news, "4.0.0", 2).is_empty());
        assert_eq!(clip("abcdef", 4).chars().count(), 4);
    }
}
