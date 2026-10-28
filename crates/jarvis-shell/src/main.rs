// Coquille de Jarvis v4 (Tauri 2), reprise de la v3.
//
// Rôle limité aux fenêtres : interface principale (HUD), widget flottant 80 × 80 traversant,
// administration (créée à l'ouverture, DÉTRUITE à la fermeture pour rendre sa mémoire), zone de
// notification, raccourcis globaux, instance unique, démarrage avec Windows.
// Tout le reste (orchestrateur, voix, sécurité, journal) vit dans le noyau Rust (jarvis-core.exe),
// lancé et surveillé ici. Les pages parlent au noyau par une passerelle WebSocket locale dont le
// jeton, tiré à chaque lancement, n'est injecté que dans les fenêtres de cette coquille.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod desk;
mod disk_assets;

use std::io::{BufRead, BufReader, Write};
use std::os::windows::process::CommandExt;
use std::path::PathBuf;
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::menu::{CheckMenuItem, Menu, MenuItem, PredefinedMenuItem};
use tauri::tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent};
use tauri::{AppHandle, Emitter, LogicalPosition, LogicalSize, Manager, WebviewUrl, WebviewWindow, WebviewWindowBuilder, WindowEvent};
use tauri_plugin_autostart::ManagerExt;
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

const CREATE_NO_WINDOW: u32 = 0x0800_0000;
const GATEWAY_PORT: u16 = 8797;
const CORE_RATIO: f64 = 0.34; // rayon du cœur cliquable du widget, en fraction de son côté
// Windows refuse les fenêtres plus étroites que ~136 px : le widget est un carré transparent d'au
// moins cette taille, l'orbe (taille réglée) est dessinée en son centre.
const WIDGET_MIN_WINDOW: f64 = 140.0;
// L'orbe grossit jusqu'à ce facteur quand Jarvis écoute, parle ou travaille (widget.js) : la
// fenêtre lui laisse la place.
const WIDGET_GROW: f64 = 1.6;

fn widget_window_side(size: f64) -> f64 {
    (size * WIDGET_GROW).ceil().max(WIDGET_MIN_WINDOW)
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct WidgetConfig {
    size: f64,
    opacity: f64,
    click_through: bool,
    ghost: bool,
}

impl Default for WidgetConfig {
    fn default() -> Self {
        Self { size: 80.0, opacity: 0.9, click_through: true, ghost: false }
    }
}

struct Shell {
    root: PathBuf,
    token: String,
    port: u16,
    core: Mutex<Option<Child>>,
    core_in: Mutex<Option<ChildStdin>>,
    quitting: Mutex<bool>,
    widget_mode: Mutex<bool>,
    widget_cfg: Mutex<WidgetConfig>,
    widget_interactive: Mutex<bool>,
    /// Ctrl maintenu : le widget est effacé et traversant (compagnon de bureau).
    widget_dodge: Mutex<bool>,
    last_drag: Mutex<Instant>,
    half_screen: Mutex<bool>,
    prev_bounds: Mutex<Option<(LogicalPosition<f64>, LogicalSize<f64>)>>,
    core_failures: Mutex<Vec<Instant>>,
    /// Installation de la mise à jour préparée demandée par le noyau (launch_pending_update).
    apply_update: Mutex<Option<bool>>,
}

type State = Arc<Shell>;

/// Marque-place le temps d'extraire les fichiers intégrés du contexte (voir `disk_assets`).
struct NoAssets;
impl tauri::Assets<tauri::Wry> for NoAssets {
    fn get(&self, _: &tauri::utils::assets::AssetKey) -> Option<std::borrow::Cow<'_, [u8]>> { None }
    fn iter(&self) -> Box<tauri::utils::assets::AssetsIter<'_>> { Box::new(std::iter::empty()) }
    fn csp_hashes(&self, _: &tauri::utils::assets::AssetKey) -> Box<dyn Iterator<Item = tauri::utils::assets::CspHash<'_>> + '_> { Box::new(std::iter::empty()) }
}

fn project_root() -> PathBuf {
    if let Ok(root) = std::env::var("JARVIS_ROOT") {
        return PathBuf::from(root);
    }
    // L'exécutable vit dans <racine>/target/<profil>/jarvis.exe : la racine se déduit de son
    // emplacement, pour survivre au déplacement du dossier (le chemin de compilation est figé).
    let from_exe = std::env::current_exe().ok().and_then(|exe| {
        exe.ancestors()
            // Installé (installer.ps1) : install.json à côté des exécutables, à la place de Cargo.toml.
            .find(|dir| dir.join("ui").join("index.html").is_file() && (dir.join("Cargo.toml").is_file() || dir.join("install.json").is_file()))
            .map(|p| p.to_path_buf())
    });
    // Repli : compilé depuis jarvis-v4/crates/jarvis-shell, la racine du projet est deux niveaux au-dessus.
    from_exe.unwrap_or_else(|| PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("..").join(".."))
}

// Données de l'utilisateur, hors du programme : %LOCALAPPDATA%\Jarvis\donnees une fois prêt (VERSION.json), sinon
// l'ancien <racine>/data tant qu'il n'a pas été recopié ; JARVIS_DATA_DIR l'impose (tests). Même règle que
// jarvis-core/src/paths.rs › data_dir, recalculée à chaque appel.
pub fn data_dir(root: &std::path::Path) -> PathBuf {
    if let Ok(d) = std::env::var("JARVIS_DATA_DIR") {
        return PathBuf::from(d);
    }
    let legacy = root.join("data");
    let Some(new) = std::env::var("LOCALAPPDATA").ok().filter(|s| !s.trim().is_empty()).map(|d| PathBuf::from(d).join("Jarvis").join("donnees")) else {
        return legacy;
    };
    if !new.join("VERSION.json").is_file() && legacy.join("config.json").is_file() {
        return legacy;
    }
    new
}

// Avant tout le reste : recopie de l'ancien data/ au premier lancement, ou import d'une archive préparé dans
// l'administration (« jarvis-core donnees-migrer », Jarvis encore fermé).
fn prepare_data(root: &std::path::Path) {
    if std::env::var("JARVIS_DATA_DIR").is_ok() {
        return;
    }
    let Some(base) = std::env::var("LOCALAPPDATA").ok().filter(|s| !s.trim().is_empty()).map(|d| PathBuf::from(d).join("Jarvis")) else { return };
    let todo = base.join("import-en-attente").is_dir() || (!base.join("donnees").join("VERSION.json").is_file() && root.join("data").join("config.json").is_file());
    if !todo {
        return;
    }
    if let Some(core) = std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join("jarvis-core.exe"))) {
        let _ = Command::new(core).arg("donnees-migrer").current_dir(root).env("JARVIS_ROOT", root).creation_flags(CREATE_NO_WINDOW).status();
    }
}

fn random_token() -> String {
    let mut bytes = [0u8; 24];
    getrandom::getrandom(&mut bytes).expect("aléa du système indisponible");
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

fn init_script(state: &Shell, role: &str, label: &str) -> String {
    format!(
        "window.__JARVIS__ = Object.freeze({{ port: {}, token: '{}', role: '{}', label: '{}' }});",
        state.port, state.token, role, label
    )
}

// Arguments supplémentaires de WebView2 (tests : port de débogage). Tauri impose les siens, qui
// écrasent la variable WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS : on les reprend en y ajoutant ceux-ci.
fn test_instance() -> bool {
    std::env::var("JARVIS_TEST_INSTANCE").is_ok()
}

// Profil WebView2 séparé (tests) : jamais celui de l'instance de l'utilisateur.
fn with_profile<'a, R: tauri::Runtime, M: Manager<R>>(b: WebviewWindowBuilder<'a, R, M>) -> WebviewWindowBuilder<'a, R, M> {
    match std::env::var("JARVIS_WEBVIEW_DATA") {
        Ok(dir) => b.data_directory(PathBuf::from(dir)),
        Err(_) => b,
    }
}

fn browser_args() -> Option<String> {
    std::env::var("JARVIS_WEBVIEW_ARGS").ok().map(|extra| {
        format!("--disable-features=msWebOOUI,msPdfOOUI,msSmartScreenProtection {extra}")
    })
}

// --- Noyau (jarvis-core.exe) -----------------------------------------------------------------------------------

// Journal de la coquille (data/logs/coquille.log) : lancement et arrêt du noyau, sortie. Sans console,
// eprintln! ne va nulle part : sans ce fichier, une coquille restée ouverte sans noyau ne laisse aucune trace.
fn utc_stamp() -> String {
    let secs = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs() as i64).unwrap_or(0);
    let (days, rem) = (secs.div_euclid(86400), secs.rem_euclid(86400));
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + if m <= 2 { 1 } else { 0 };
    format!("{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}Z", rem / 3600, rem % 3600 / 60, rem % 60)
}

fn log_shell(state: &Shell, msg: &str) {
    let dir = data_dir(&state.root).join("logs");
    let _ = std::fs::create_dir_all(&dir);
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(dir.join("coquille.log")) {
        let _ = writeln!(f, "[{}] [pid {}] {msg}", utc_stamp(), std::process::id());
    }
}

fn send_core(state: &Shell, msg: Value) {
    if let Some(stdin) = state.core_in.lock().unwrap().as_mut() {
        let _ = writeln!(stdin, "{msg}");
        let _ = stdin.flush();
    }
}

fn start_core(app: &AppHandle) {
    let state: State = app.state::<State>().inner().clone();
    // Le noyau est livré à côté de la coquille (même dossier target/<profil>/).
    let core = std::env::var("JARVIS_CORE").map(PathBuf::from).unwrap_or_else(|_| {
        std::env::current_exe().ok().and_then(|e| e.parent().map(|d| d.join("jarvis-core.exe"))).unwrap_or_else(|| PathBuf::from("jarvis-core.exe"))
    });
    let spawned = Command::new(&core)
        .current_dir(&state.root)
        .env("JARVIS_ROOT", &state.root)
        .env("JARVIS_GATEWAY_TOKEN", &state.token)
        .env("JARVIS_GATEWAY_PORT", state.port.to_string())
        .env("JARVIS_SHELL_PID", std::process::id().to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .creation_flags(CREATE_NO_WINDOW)
        .spawn();
    let mut child = match spawned {
        Ok(c) => c,
        Err(err) => {
            eprintln!("[coquille] noyau impossible à lancer : {err} ({})", core.display());
            log_shell(&state, &format!("noyau impossible à lancer : {err} ({} est-il compilé ?)", core.display()));
            return;
        }
    };
    log_shell(&state, &format!("noyau lancé (pid {})", child.id()));
    let stdout = child.stdout.take();
    *state.core_in.lock().unwrap() = child.stdin.take();
    *state.core.lock().unwrap() = Some(child);

    let app2 = app.clone();
    thread::spawn(move || {
        if let Some(out) = stdout {
            for line in BufReader::new(out).lines().map_while(Result::ok) {
                if let Some(json) = line.strip_prefix("@@shell ") {
                    if let Ok(msg) = serde_json::from_str::<Value>(json) {
                        handle_core_command(&app2, msg);
                    }
                }
            }
        }
        // Le noyau s'est arrêté : relance, sauf à la fermeture de Jarvis.
        let state: State = app2.state::<State>().inner().clone();
        if *state.quitting.lock().unwrap() {
            return;
        }
        *state.core_in.lock().unwrap() = None;
        let status = state.core.lock().unwrap().take().and_then(|mut c| c.wait().ok());
        log_shell(&state, &format!("noyau arrêté ({})", status.map_or("statut inconnu".to_string(), |s| s.to_string())));
        let now = Instant::now();
        let recent = {
            let mut f = state.core_failures.lock().unwrap();
            f.retain(|t| now.duration_since(*t) < Duration::from_secs(120));
            f.push(now);
            f.len()
        };
        if recent >= 5 {
            eprintln!("[coquille] le noyau s'est arrêté 5 fois en 2 minutes : pas de nouvelle relance");
            // Une coquille sans noyau ne sert à rien (ni voix, ni STOP, ni commandes) : elle se ferme au lieu de rester
            // ouverte, muette, sur le bureau.
            log_shell(&state, "le noyau s'est arrêté 5 fois en 2 minutes : fermeture de la coquille");
            quit(&app2);
            return;
        }
        thread::sleep(Duration::from_millis(1500 * recent as u64));
        start_core(&app2);
    });
}

fn handle_core_command(app: &AppHandle, msg: Value) {
    let state: State = app.state::<State>().inner().clone();
    match msg["cmd"].as_str().unwrap_or("") {
        "ready" => {
            if let Some(cfg) = msg.get("widget").and_then(|w| serde_json::from_value::<WidgetConfig>(w.clone()).ok()) {
                *state.widget_cfg.lock().unwrap() = cfg;
            }
            // Musique locale de data/music, lue par le protocole « asset » (en plus du dossier Musique).
            let _ = app.asset_protocol_scope().allow_directory(data_dir(&state.root).join("music"), true);
            if let Some(cfg) = msg.get("desk").and_then(|d| serde_json::from_value::<desk::DeskConfig>(d.clone()).ok()) {
                desk::apply_config(app, cfg);
            }
        }
        "desk_config" => {
            if let Some(cfg) = msg.get("desk").and_then(|d| serde_json::from_value::<desk::DeskConfig>(d.clone()).ok()) {
                desk::apply_config(app, cfg);
            }
        }
        "desk_goto" => desk::goto(app),
        "open_admin" => open_admin(app, msg["section"].as_str().map(String::from)),
        "widget_mode" => set_widget_mode(app, msg["on"].as_bool().unwrap_or(false)),
        "widget_config" => {
            if let Some(cfg) = msg.get("widget").and_then(|w| serde_json::from_value::<WidgetConfig>(w.clone()).ok()) {
                apply_widget_config(app, cfg);
            }
        }
        "autostart" => {
            let al = app.autolaunch();
            let _ = if msg["enable"].as_bool().unwrap_or(false) { al.enable() } else { al.disable() };
        }
        "quit" => {
            // « update » : le noyau a demandé d'installer la version préparée (updateApply), Jarvis relancé ensuite.
            if msg["update"].as_bool() == Some(true) {
                *app.state::<State>().inner().apply_update.lock().unwrap() = Some(true);
            }
            quit(app)
        }
        "fatal" => {
            eprintln!("[coquille] noyau : {}", msg["reason"].as_str().unwrap_or("erreur fatale"));
            quit(app);
        }
        _ => {}
    }
}

fn quit(app: &AppHandle) {
    let state: State = app.state::<State>().inner().clone();
    *state.quitting.lock().unwrap() = true;
    log_shell(&state, "arrêt demandé");
    send_core(&state, json!({ "cmd": "quit" }));
    let app2 = app.clone();
    thread::spawn(move || {
        // Le noyau arrête proprement la voix et l'orchestrateur ; au-delà de 3 s, on tranche.
        let deadline = Instant::now() + Duration::from_secs(3);
        loop {
            let done = match state.core.lock().unwrap().as_mut() {
                Some(c) => c.try_wait().map(|s| s.is_some()).unwrap_or(true),
                None => true,
            };
            if done || Instant::now() > deadline {
                break;
            }
            thread::sleep(Duration::from_millis(100));
        }
        if let Some(mut c) = state.core.lock().unwrap().take() {
            let _ = c.kill();
        }
        log_shell(&state, "fermeture de la coquille");
        launch_pending_update(&state);
        // Filet : si la sortie propre se bloque (fenêtre, zone de notification…), la coquille se ferme quand même.
        // Le 30/09/2026 un jarvis.exe est resté ouvert plus de 10 minutes après « Quitter » (relance en attente).
        let state2 = state.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_secs(5));
            log_shell(&state2, "sortie propre bloquée : fermeture forcée");
            std::process::exit(0);
        });
        app2.exit(0);
    });
}

// Mise à jour d'un Jarvis installé (crates/jarvis-core/src/update.rs) : une version compilée attend dans
// %LOCALAPPDATA%\Jarvis\build\pret.json. Installée à la fermeture, Jarvis fermé : sur demande (bouton « Installer et
// redémarrer », Jarvis relancé ensuite), ou en mode « auto » (réglage update.mode) à toute fermeture, sans relance.
fn launch_pending_update(state: &Shell) {
    let Some(base) = std::env::var("LOCALAPPDATA").ok().filter(|s| !s.trim().is_empty()).map(|d| PathBuf::from(d).join("Jarvis")) else { return };
    let script = state.root.join("installer.ps1");
    if !base.join("build").join("pret.json").is_file() || !state.root.join("install.json").is_file() || !script.is_file() {
        return;
    }
    let asked = state.apply_update.lock().unwrap().unwrap_or(false);
    let auto = std::fs::read_to_string(data_dir(&state.root).join("config.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .is_some_and(|c| c["update"]["mode"] == "auto");
    if !asked && !auto {
        return;
    }
    // Copie hors du dossier du programme, que le script remplace.
    let tmp = std::env::temp_dir().join("jarvis-installer.ps1");
    if std::fs::copy(&script, &tmp).is_err() {
        return;
    }
    let mut args = vec!["-NoProfile".to_string(), "-ExecutionPolicy".into(), "Bypass".into(), "-File".into(), tmp.to_string_lossy().to_string(), "-Appliquer".into()];
    if !asked {
        args.extend(["-SansRelancer".to_string(), "-Silencieux".into()]);
    }
    log_shell(state, &format!("installation de la mise à jour préparée ({})", if asked { "demandée" } else { "mode auto" }));
    let _ = Command::new("powershell.exe").args(&args).spawn();
}

// --- Fenêtres -----------------------------------------------------------------------------------

fn emit_visibility(app: &AppHandle, label: &str) {
    if let Some(w) = app.get_webview_window(label) {
        let visible = w.is_visible().unwrap_or(false) && !w.is_minimized().unwrap_or(false);
        let _ = app.emit("jarvis:visibility", json!({ "label": label, "visible": visible }));
    }
}

fn create_hud(app: &AppHandle, visible: bool) -> tauri::Result<WebviewWindow> {
    let state: State = app.state::<State>().inner().clone();
    let win = WebviewWindowBuilder::new(app, "hud", WebviewUrl::App("index.html".into()))
        .title("JARVIS")
        .inner_size(1600.0, 900.0)
        .min_inner_size(900.0, 560.0)
        .decorations(false)
        .center()
        .visible(visible)
        .disable_drag_drop_handler() // fichier déposé sur le cœur : géré par la page (ui/bridge.js)
        .initialization_script(&init_script(&state, "hud", "hud"));
    let win = with_profile(match browser_args() { Some(a) => win.additional_browser_args(&a), None => win }).build()?;
    let app2 = app.clone();
    win.on_window_event(move |event| match event {
        // Fermer la fenêtre principale la masque : Jarvis reste à l'écoute (zone de notification).
        WindowEvent::CloseRequested { api, .. } => {
            api.prevent_close();
            if let Some(w) = app2.get_webview_window("hud") {
                let _ = w.hide();
            }
            emit_visibility(&app2, "hud");
        }
        WindowEvent::Resized(_) | WindowEvent::Focused(_) => emit_visibility(&app2, "hud"),
        _ => {}
    });
    Ok(win)
}

fn show_hud(app: &AppHandle) {
    // Bureau JARVIS actif : l'interface est le bureau virtuel, plus la fenêtre classique.
    if desk::enabled(app) {
        desk::goto(app);
        return;
    }
    if let Some(w) = app.get_webview_window("hud") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
    }
    emit_visibility(app, "hud");
}

fn toggle_hud(app: &AppHandle) {
    if desk::enabled(app) {
        desk::goto(app);
        return;
    }
    let state: State = app.state::<State>().inner().clone();
    if *state.widget_mode.lock().unwrap() {
        set_widget_mode(app, false);
        return;
    }
    if let Some(w) = app.get_webview_window("hud") {
        if w.is_visible().unwrap_or(false) && w.is_focused().unwrap_or(false) {
            let _ = w.hide();
            emit_visibility(app, "hud");
        } else {
            show_hud(app);
        }
    }
}

fn open_admin(app: &AppHandle, section: Option<String>) {
    let select = section.map(|s| format!("window.JarvisAdmin && window.JarvisAdmin.selectSection({})", json!(s)));
    if let Some(w) = app.get_webview_window("admin") {
        let _ = w.unminimize();
        let _ = w.show();
        let _ = w.set_focus();
        if let Some(js) = select {
            let _ = w.eval(&js);
        }
        return;
    }
    let state: State = app.state::<State>().inner().clone();
    let built = WebviewWindowBuilder::new(app, "admin", WebviewUrl::App("admin.html".into()))
        .title("Jarvis · Administration")
        .inner_size(1280.0, 860.0)
        .min_inner_size(900.0, 560.0)
        .decorations(false)
        .center()
        .initialization_script(&init_script(&state, "admin", "admin"));
    let built = with_profile(match browser_args() { Some(a) => built.additional_browser_args(&a), None => built }).build();
    if let (Ok(w), Some(js)) = (built, select) {
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(1200)); // page chargée et reliée au noyau
            let _ = w.eval(&js);
        });
    }
}

// --- Widget flottant ----------------------------------------------------------------------------

fn widget_state_file(state: &Shell) -> PathBuf {
    data_dir(&state.root).join("widget.json")
}

fn initial_widget_position(app: &AppHandle, state: &Shell, size: f64) -> (f64, f64) {
    if let Ok(text) = std::fs::read_to_string(widget_state_file(state)) {
        if let Ok(v) = serde_json::from_str::<Value>(&text) {
            if let (Some(x), Some(y)) = (v["x"].as_f64(), v["y"].as_f64()) {
                return (x, y);
            }
        }
    }
    if let Ok(Some(m)) = app.primary_monitor() {
        let scale = m.scale_factor();
        let area = m.work_area();
        let x = area.position.x as f64 / scale + area.size.width as f64 / scale - size - 16.0;
        let y = area.position.y as f64 / scale + area.size.height as f64 / scale - size - 16.0;
        return (x, y);
    }
    (100.0, 100.0)
}

fn save_widget_position(app: &AppHandle) {
    let state: State = app.state::<State>().inner().clone();
    if let Some(w) = app.get_webview_window("widget") {
        if let Ok(pos) = w.outer_position() {
            let scale = w.scale_factor().unwrap_or(1.0);
            let p = pos.to_logical::<f64>(scale);
            let _ = std::fs::write(widget_state_file(&state), json!({ "x": p.x, "y": p.y }).to_string());
        }
    }
    desk::widget_dropped(app);
}

fn create_widget(app: &AppHandle) -> tauri::Result<WebviewWindow> {
    let state: State = app.state::<State>().inner().clone();
    let size = widget_window_side(state.widget_cfg.lock().unwrap().size);
    let (x, y) = initial_widget_position(app, &state, size);
    let win = WebviewWindowBuilder::new(app, "widget", WebviewUrl::App("widget.html".into()))
        .title("JARVIS · widget")
        .inner_size(size, size)
        .position(x, y)
        .transparent(true)
        .decorations(false)
        .resizable(false)
        .maximizable(false)
        .minimizable(false)
        .skip_taskbar(true)
        .always_on_top(true)
        .shadow(false)
        .focused(false) // ne vole jamais le focus au jeu ou au logiciel en cours
        .visible(false)
        // Compagnon de bureau : un fichier déposé sur le cœur le nourrit. Le dépôt est laissé à WebView2 et à la
        // page (ui/bridge.js), qui n'en garde que l'extension (le fichier n'est ni lu, ni copié, ni modifié).
        .disable_drag_drop_handler()
        .initialization_script(&init_script(&state, "widget", "widget"));
    let win = with_profile(match browser_args() { Some(a) => win.additional_browser_args(&a), None => win }).build()?;
    let _ = win.set_ignore_cursor_events(true);
    *state.widget_interactive.lock().unwrap() = false;
    Ok(win)
}

fn set_widget_mode(app: &AppHandle, on: bool) {
    // Bureau JARVIS actif : le widget est géré par desk.rs (autres bureaux) ; « interface complète »
    // ramène au Bureau JARVIS.
    if desk::enabled(app) {
        if !on {
            desk::goto(app);
        }
        return;
    }
    let state: State = app.state::<State>().inner().clone();
    *state.widget_mode.lock().unwrap() = on;
    if on {
        let w = match app.get_webview_window("widget") {
            Some(w) => w,
            None => match create_widget(app) {
                Ok(w) => w,
                Err(err) => {
                    eprintln!("[coquille] widget impossible : {err}");
                    return;
                }
            },
        };
        let _ = w.show();
        if let Some(h) = app.get_webview_window("hud") {
            let _ = h.hide();
        }
    } else {
        if let Some(w) = app.get_webview_window("widget") {
            let _ = w.hide();
        }
        show_hud(app);
    }
    emit_visibility(app, "widget");
    emit_visibility(app, "hud");
}

fn apply_widget_config(app: &AppHandle, cfg: WidgetConfig) {
    let state: State = app.state::<State>().inner().clone();
    let old = std::mem::replace(&mut *state.widget_cfg.lock().unwrap(), cfg.clone());
    if let Some(w) = app.get_webview_window("widget") {
        let (old_side, new_side) = (widget_window_side(old.size), widget_window_side(cfg.size));
        if (old_side - new_side).abs() > 0.5 {
            // Taille changée : on garde le centre du widget à sa place.
            if let Ok(pos) = w.outer_position() {
                let scale = w.scale_factor().unwrap_or(1.0);
                let p = pos.to_logical::<f64>(scale);
                let cx = p.x + old_side / 2.0;
                let cy = p.y + old_side / 2.0;
                let _ = w.set_size(LogicalSize::new(new_side, new_side));
                let _ = w.set_position(LogicalPosition::new(cx - new_side / 2.0, cy - new_side / 2.0));
                save_widget_position(app);
            }
        }
        if cfg.ghost {
            let _ = w.set_ignore_cursor_events(true);
        }
    }
}

// Clics traversants : Tauri ne transmet pas le survol à une fenêtre qui ignore la souris (Electron
// le faisait). La coquille suit donc elle-même le curseur : au-dessus du cœur de l'orbe, le widget
// reçoit les clics ; ailleurs, ils traversent vers la fenêtre de derrière.
fn ctrl_down() -> bool {
    use windows::Win32::UI::Input::KeyboardAndMouse::{GetAsyncKeyState, VK_CONTROL};
    unsafe { GetAsyncKeyState(VK_CONTROL.0 as i32) < 0 }
}

fn watch_widget_cursor(app: AppHandle) {
    thread::spawn(move || loop {
        thread::sleep(Duration::from_millis(40));
        let state: State = app.state::<State>().inner().clone();
        if *state.quitting.lock().unwrap() {
            return;
        }
        if !*state.widget_mode.lock().unwrap() {
            if !desk::enabled(&app) {
                thread::sleep(Duration::from_millis(400));
            }
            continue;
        }
        let Some(w) = app.get_webview_window("widget") else { continue };
        let cfg = state.widget_cfg.lock().unwrap().clone();
        let dragging = state.last_drag.lock().unwrap().elapsed() < Duration::from_millis(400);
        // Ctrl maintenu (et pas de glisser en cours) : le widget s'efface, les clics traversent.
        let dodge = !dragging && ctrl_down();
        if dodge != *state.widget_dodge.lock().unwrap() {
            *state.widget_dodge.lock().unwrap() = dodge;
            let _ = w.emit("jarvis:widget-dodge", dodge);
        }
        let want = if cfg.ghost || dodge || desk::flying(&app) {
            false // en vol, la fenêtre est agrandie : aucun clic ne la concerne
        } else if !cfg.click_through || dragging {
            true
        } else {
            match (w.cursor_position(), w.outer_position(), w.outer_size()) {
                (Ok(c), Ok(p), Ok(s)) => {
                    let cx = p.x as f64 + s.width as f64 / 2.0;
                    let cy = p.y as f64 + s.height as f64 / 2.0;
                    // Rayon d'après la taille de l'orbe (pixels physiques), pas celle de la fenêtre.
                    let r = CORE_RATIO * cfg.size * w.scale_factor().unwrap_or(1.0);
                    let _ = s;
                    (c.x - cx).powi(2) + (c.y - cy).powi(2) <= r * r
                }
                _ => false,
            }
        };
        let mut cur = state.widget_interactive.lock().unwrap();
        if *cur != want {
            *cur = want;
            let _ = w.set_ignore_cursor_events(!want);
            let _ = w.emit("jarvis:widget-hover", want);
        }
    });
}

// --- Menus (zone de notification, clic droit du widget) -------------------------------------------

fn on_menu(app: &AppHandle, id: &str) {
    let state: State = app.state::<State>().inner().clone();
    match id {
        "toggle" => toggle_hud(app),
        "full" => set_widget_mode(app, false),
        "widget" => {
            let on = !*state.widget_mode.lock().unwrap();
            set_widget_mode(app, on);
        }
        "admin" => open_admin(app, None),
        "admin_widget" => open_admin(app, Some("widget".into())),
        "ghost" => send_core(&state, json!({ "cmd": "widget_ghost" })),
        "desk" => send_core(&state, json!({ "cmd": "desk_toggle" })),
        "desk_goto" => desk::goto(app),
        "app_mode" => desk::leave(app),
        "desk_mode" => desk::enter(app),
        "voice" => send_core(&state, json!({ "cmd": "restart_voice" })),
        "autostart" => {
            let al = app.autolaunch();
            let enabled = al.is_enabled().unwrap_or(false);
            let _ = if enabled { al.disable() } else { al.enable() };
        }
        "quit" => quit(app),
        _ => {}
    }
}

fn build_tray(app: &AppHandle) -> tauri::Result<()> {
    let autostart = app.autolaunch().is_enabled().unwrap_or(false);
    let desk_item = CheckMenuItem::with_id(app, "desk", "Bureau JARVIS (bureau virtuel 2)", true, desk::enabled(app), None::<&str>)?;
    desk::set_tray_item(app, desk_item.clone());
    let menu = Menu::with_items(
        app,
        &[
            &MenuItem::with_id(app, "toggle", "Afficher / masquer Jarvis (Ctrl+Maj+J)", true, None::<&str>)?,
            &MenuItem::with_id(app, "desk_goto", "Aller au Bureau JARVIS (Ctrl+Maj+B)", true, None::<&str>)?,
            &PredefinedMenuItem::separator(app)?,
            &desk_item,
            &CheckMenuItem::with_id(app, "autostart", "Démarrer avec Windows", true, autostart, None::<&str>)?,
            &MenuItem::with_id(app, "voice", "Redémarrer la voix", true, None::<&str>)?,
            &MenuItem::with_id(app, "widget", "Mode widget flottant (Ctrl+Maj+W)", true, None::<&str>)?,
            &MenuItem::with_id(app, "admin", "Configuration… (F10)", true, None::<&str>)?,
            &PredefinedMenuItem::separator(app)?,
            &MenuItem::with_id(app, "quit", "Quitter Jarvis", true, None::<&str>)?,
        ],
    )?;
    let mut builder = TrayIconBuilder::with_id("jarvis").tooltip("Jarvis — assistant personnel vocal").menu(&menu).show_menu_on_left_click(false);
    if let Some(icon) = app.default_window_icon() {
        builder = builder.icon(icon.clone());
    }
    builder
        .on_menu_event(|app, event| on_menu(app, event.id.as_ref()))
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click { button: MouseButton::Left, button_state: MouseButtonState::Up, .. } = event {
                toggle_hud(tray.app_handle());
            }
        })
        .build(app)?;
    Ok(())
}

// --- Commandes appelées par les pages --------------------------------------------------------------

// Sous Windows, une commande SYNCHRONE qui crée une fenêtre se bloque elle-même (elle tourne dans
// la boucle principale, dont la création de fenêtre a besoin) : ces commandes sont asynchrones.

fn state_of(app: &AppHandle) -> State {
    app.state::<State>().inner().clone()
}

#[tauri::command]
async fn toggle_widget(app: AppHandle) {
    let on = !*state_of(&app).widget_mode.lock().unwrap();
    set_widget_mode(&app, on);
}

#[tauri::command]
async fn set_widget(app: AppHandle, on: bool) {
    set_widget_mode(&app, on);
}

fn half_screen_toggle(app: &AppHandle) -> bool {
    let state = state_of(app);
    let Some(w) = app.get_webview_window("hud") else { return false };
    let scale = w.scale_factor().unwrap_or(1.0);
    let mut half = state.half_screen.lock().unwrap();
    if *half {
        if let Some((p, s)) = state.prev_bounds.lock().unwrap().take() {
            let _ = w.set_size(s);
            let _ = w.set_position(p);
        }
        *half = false;
    } else {
        if let (Ok(p), Ok(s)) = (w.outer_position(), w.inner_size()) {
            *state.prev_bounds.lock().unwrap() = Some((p.to_logical(scale), s.to_logical(scale)));
        }
        if let Ok(Some(m)) = w.current_monitor() {
            let area = m.work_area();
            let ms = m.scale_factor();
            let wa = area.size.width as f64 / ms;
            let ha = area.size.height as f64 / ms;
            let _ = w.set_size(LogicalSize::new(wa / 2.0, ha));
            let _ = w.set_position(LogicalPosition::new(area.position.x as f64 / ms + wa / 2.0, area.position.y as f64 / ms));
        }
        *half = true;
    }
    *half
}

#[tauri::command]
async fn toggle_half_screen(app: AppHandle) -> Value {
    json!({ "isHalfScreen": half_screen_toggle(&app) })
}

#[tauri::command]
async fn set_window_mode(app: AppHandle, mode: String) -> Value {
    let state = state_of(&app);
    let half = *state.half_screen.lock().unwrap();
    match mode.as_str() {
        "half" => {
            if !half {
                half_screen_toggle(&app);
            }
            json!({ "mode": "half" })
        }
        "widget" => {
            set_widget_mode(&app, true);
            json!({ "mode": "widget" })
        }
        _ => {
            if half {
                half_screen_toggle(&app);
            }
            if *state.widget_mode.lock().unwrap() {
                set_widget_mode(&app, false);
            }
            json!({ "mode": "normal" })
        }
    }
}

#[tauri::command]
fn get_window_state(app: AppHandle, state: tauri::State<State>) -> Value {
    let maximized = app.get_webview_window("hud").and_then(|w| w.is_maximized().ok()).unwrap_or(false);
    json!({
        "isHalfScreen": *state.half_screen.lock().unwrap(),
        "isWidgetMode": *state.widget_mode.lock().unwrap(),
        "isMaximized": maximized
    })
}

#[tauri::command]
async fn open_admin_cmd(app: AppHandle, section: Option<String>) {
    open_admin(&app, section);
}

#[tauri::command]
async fn admin_window(app: AppHandle, action: String) {
    if let Some(w) = app.get_webview_window("admin") {
        // Fermer DÉTRUIT la fenêtre : sa page WebView2 rend sa mémoire (≈ 200 Mo en v1).
        let _ = if action == "minimize" { w.minimize() } else { w.destroy() };
    }
}

#[tauri::command]
fn widget_hover(state: tauri::State<State>, on: bool) {
    if on {
        *state.last_drag.lock().unwrap() = Instant::now();
    }
}

#[tauri::command]
fn widget_move(app: AppHandle, state: tauri::State<State>, x: f64, y: f64) {
    *state.last_drag.lock().unwrap() = Instant::now();
    if let Some(w) = app.get_webview_window("widget") {
        let _ = w.set_position(LogicalPosition::new(x, y));
    }
}

/// Promenade du compagnon de bureau : déplace le widget sans le rendre cliquable ni enregistrer sa place.
#[tauri::command]
fn widget_walk(app: AppHandle, x: f64, y: f64) {
    if let Some(w) = app.get_webview_window("widget") {
        let _ = w.set_position(LogicalPosition::new(x, y));
    }
}

#[tauri::command]
fn widget_moved(app: AppHandle) {
    save_widget_position(&app);
}

#[tauri::command]
fn widget_position(app: AppHandle) -> Option<(f64, f64)> {
    let w = app.get_webview_window("widget")?;
    let scale = w.scale_factor().unwrap_or(1.0);
    let p = w.outer_position().ok()?.to_logical::<f64>(scale);
    Some((p.x, p.y))
}

#[tauri::command]
async fn widget_menu(app: AppHandle, window: WebviewWindow) -> Result<(), String> {
    // Le menu s'ouvre sur la fenêtre appelante (le widget).
    let w = window;
    let ghost = state_of(&app).widget_cfg.lock().unwrap().ghost;
    let build = || -> tauri::Result<Menu<tauri::Wry>> {
        Menu::with_items(
            &app,
            &[
                &MenuItem::with_id(&app, "full", "Interface complète", true, None::<&str>)?,
                &if desk::enabled(&app) {
                    MenuItem::with_id(&app, "app_mode", "Mode application classique", true, None::<&str>)?
                } else {
                    MenuItem::with_id(&app, "desk_mode", "Mode bureau (Bureau JARVIS)", true, None::<&str>)?
                },
                &MenuItem::with_id(&app, "admin_widget", "Configuration…", true, None::<&str>)?,
                &PredefinedMenuItem::separator(&app)?,
                &CheckMenuItem::with_id(&app, "ghost", "Mode fantôme (Ctrl+Maj+G)", true, ghost, None::<&str>)?,
                &PredefinedMenuItem::separator(&app)?,
                &MenuItem::with_id(&app, "quit", "Quitter Jarvis", true, None::<&str>)?,
            ],
        )
    };
    let menu = build().map_err(|e| e.to_string())?;
    w.popup_menu(&menu).map_err(|e| e.to_string())
}

#[tauri::command]
async fn quit_app(app: AppHandle) {
    quit(&app);
}

#[tauri::command]
fn set_autostart(app: AppHandle, enable: bool) -> bool {
    let al = app.autolaunch();
    let _ = if enable { al.enable() } else { al.disable() };
    al.is_enabled().unwrap_or(false)
}

#[tauri::command]
fn get_autostart(app: AppHandle) -> bool {
    app.autolaunch().is_enabled().unwrap_or(false)
}

// --- Programme principal ------------------------------------------------------------------------

fn main() {
    let port = std::env::var("JARVIS_GATEWAY_PORT").ok().and_then(|p| p.parse().ok()).unwrap_or(GATEWAY_PORT);
    let root = project_root();
    prepare_data(&root);
    let state: State = Arc::new(Shell {
        root,
        token: random_token(),
        port,
        core: Mutex::new(None),
        core_in: Mutex::new(None),
        quitting: Mutex::new(false),
        widget_mode: Mutex::new(false),
        widget_cfg: Mutex::new(WidgetConfig::default()),
        widget_interactive: Mutex::new(false),
        widget_dodge: Mutex::new(false),
        last_drag: Mutex::new(Instant::now() - Duration::from_secs(60)),
        half_screen: Mutex::new(false),
        prev_bounds: Mutex::new(None),
        core_failures: Mutex::new(Vec::new()),
        apply_update: Mutex::new(None),
    });

    let shortcuts = [
        Shortcut::new(Some(Modifiers::CONTROL | Modifiers::SHIFT), Code::KeyJ),
        Shortcut::new(Some(Modifiers::CONTROL | Modifiers::SHIFT), Code::KeyW),
        Shortcut::new(Some(Modifiers::CONTROL | Modifiers::SHIFT), Code::KeyG),
        Shortcut::new(Some(Modifiers::CONTROL | Modifiers::SHIFT), Code::KeyB),
    ];

    let mut builder = tauri::Builder::default();
    if !test_instance() {
        // Instance unique : relancer Jarvis ramène simplement la fenêtre existante.
        builder = builder.plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| show_hud(app)));
    }
    builder
        .plugin(tauri_plugin_autostart::init(tauri_plugin_autostart::MacosLauncher::LaunchAgent, None))
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(move |app, shortcut, event| {
                    if event.state() != ShortcutState::Pressed {
                        return;
                    }
                    let state: State = app.state::<State>().inner().clone();
                    match shortcut.key {
                        Code::KeyJ => toggle_hud(app),
                        Code::KeyW => {
                            let on = !*state.widget_mode.lock().unwrap();
                            set_widget_mode(app, on);
                        }
                        // Mode fantôme : seul moyen d'en sortir, puisque plus rien ne réagit à la souris.
                        Code::KeyG => send_core(&state, json!({ "cmd": "widget_ghost" })),
                        // Bureau JARVIS : y aller s'il est actif, sinon l'activer.
                        Code::KeyB => {
                            if desk::enabled(app) {
                                desk::goto(app);
                            } else {
                                send_core(&state, json!({ "cmd": "desk_toggle" }));
                            }
                        }
                        _ => {}
                    }
                })
                .build(),
        )
        .manage(state)
        .manage(desk::DeskState::default())
        .on_menu_event(|app, event| on_menu(app, event.id.as_ref()))
        .invoke_handler(tauri::generate_handler![
            toggle_widget,
            set_widget,
            toggle_half_screen,
            set_window_mode,
            get_window_state,
            open_admin_cmd,
            admin_window,
            widget_hover,
            widget_move,
            widget_walk,
            widget_moved,
            widget_position,
            widget_menu,
            quit_app,
            set_autostart,
            get_autostart,
            desk::desk_heart,
            desk::desk_cards,
            desk::heart_ready,
            desk::heart_landed,
            desk::desk_goto,
            desk::desk_leave,
            desk::desk_enter,
            desk::desk_reveal,
            desk::desk_state,
            desk::desk_wallpaper
        ])
        .setup(move |app| {
            let handle = app.handle().clone();
            start_core(&handle);
            // Bureau JARVIS actif : pas de fenêtre classique (desk.rs crée celle du bureau).
            let desk_cfg = desk::config_at_launch(&handle.state::<State>().inner().clone());
            if desk_cfg.enabled {
                desk::apply_config(&handle, desk_cfg);
            } else {
                create_hud(&handle, true)?;
            }
            if !test_instance() {
                build_tray(&handle)?;
                for s in shortcuts {
                    if let Err(err) = app.global_shortcut().register(s) {
                        eprintln!("[coquille] raccourci indisponible : {err}");
                    }
                }
            }
            desk::start_watch(handle.clone());
            watch_widget_cursor(handle);
            Ok(())
        })
        .build({
            // ui/ lue sur le disque : une modification de l'interface ne demande plus de recompiler.
            let mut context = tauri::generate_context!();
            let embedded = context.set_assets(Box::new(NoAssets));
            let assets: Box<dyn tauri::Assets<tauri::Wry>> = match disk_assets::DiskAssets::new(&project_root(), embedded) {
                Ok(disk) => Box::new(disk),
                Err(embedded) => embedded,
            };
            context.set_assets(assets);
            context
        })
        .expect("coquille Jarvis impossible à construire")
        .run(|_app, _event| {});
}
