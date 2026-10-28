// Bureau JARVIS : le HUD devient le fond vivant d'un bureau virtuel de Windows (le n° 2 par défaut).
//
// Pas d'injection dans WorkerW (calque commun à tous les bureaux, sans souris ni clavier) : de vraies
// fenêtres de premier niveau.
//   « desk »       HUD complet sur la zone de travail de l'écran choisi, maintenu TOUT EN BAS de la
//                  pile (WM_WINDOWPOSCHANGING) : les applications s'ouvrent et restent au-dessus ;
//   « desk-bg-N »  fond statique sans icônes sur chacun des autres écrans.
// Le cœur reste toujours visible : dès qu'une application recouvre celui du HUD, ou sur les autres
// bureaux (fond d'écran et icônes de Windows intacts), c'est le widget flottant qui le remplace.
// Petit et discret, il se range à sa place (celle où on l'a posé) ou, si une fenêtre la recouvre,
// dans l'endroit libre le plus proche, en glissant lentement. Un clic sur lui ramène au Bureau
// JARVIS, un double-clic réduit les fenêtres qui cachent le HUD.
//
// Windows ne range pas ces fenêtres dans un bureau virtuel (hors barre des tâches, elles n'ont pas de
// « vue » : IVirtualDesktopManager les laisse sur tous les bureaux). La veille les affiche donc
// elle-même selon le bureau courant, que l'Explorateur écrit dans le registre : notification de
// changement, réaction immédiate. Elle suit aussi le recouvrement : le HUD ne se dessine plus quand
// il est entièrement caché ou sur un autre bureau (jarvis:visibility).

use std::sync::Mutex;
use std::thread;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use tauri::{AppHandle, Emitter, Manager, PhysicalPosition, PhysicalSize, WebviewUrl, WebviewWindow, WebviewWindowBuilder};
use windows::core::{BOOL, GUID, PCWSTR};
use windows::Win32::Foundation::{HANDLE, HWND, LPARAM, LRESULT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED};
use windows::Win32::System::Registry::{
    RegGetValueW, RegNotifyChangeKeyValue, RegOpenKeyExW, HKEY, HKEY_CURRENT_USER, KEY_NOTIFY, KEY_READ, REG_NOTIFY_CHANGE_LAST_SET,
    RRF_RT_REG_BINARY, RRF_RT_REG_SZ,
};
use windows::Win32::System::Threading::{CreateEventW, WaitForSingleObject};
use windows::Win32::UI::Input::KeyboardAndMouse::{
    SendInput, INPUT, INPUT_0, INPUT_KEYBOARD, KEYBDINPUT, KEYBD_EVENT_FLAGS, KEYEVENTF_EXTENDEDKEY, KEYEVENTF_KEYUP, VIRTUAL_KEY, VK_D,
    VK_LCONTROL, VK_LEFT, VK_LWIN, VK_RIGHT,
};
use windows::Win32::UI::Shell::{DefSubclassProc, SetWindowSubclass};
use windows::Win32::UI::WindowsAndMessaging::{
    EnumWindows, GetClassNameW, GetWindowLongPtrW, GetWindowRect, IsIconic, IsWindowVisible, SetWindowPos, ShowWindow,
    SystemParametersInfoW, GWL_EXSTYLE, SPI_GETDESKWALLPAPER, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS,
    GWL_STYLE, HWND_BOTTOM, SWP_NOACTIVATE, SWP_NOMOVE, SWP_NOSIZE, SWP_NOZORDER, SW_HIDE,
    SW_SHOWMINNOACTIVE, SW_SHOWNOACTIVATE, WINDOWPOS, WM_WINDOWPOSCHANGING, WS_EX_LAYERED, WS_EX_NOACTIVATE, WS_EX_TOOLWINDOW,
    WS_EX_TRANSPARENT, WS_MINIMIZEBOX,
};

use crate::{browser_args, create_hud, create_widget, emit_visibility, init_script, widget_window_side, with_profile, State};

// Cœur unique : quand une application recouvre le cœur du HUD, c'est ce cœur-là qui rétrécit et glisse
// jusqu'au widget (et qui revient en grossissant quand la place se libère). Un seul propriétaire à la fois :
//   Off  le HUD dessine le cœur ; le widget est caché
//   Out  trajet HUD → widget (dessiné par la page du widget, dans une fenêtre agrandie le temps du trajet)
//   On   le widget est posé à sa place ; le HUD ne dessine plus son cœur
//   In   trajet widget → HUD
// Le HUD est sous toutes les fenêtres : seul le widget (toujours au-dessus) peut montrer le trajet.
// Passage de relais sans double : la page du widget peint sa 1re image à la taille exacte du cœur du HUD,
// puis prévient (heart_ready) ; le HUD fond alors son cœur (jarvis:heart-owner). À l'arrivée, la page efface
// son canvas (jarvis:heart-settled) AVANT que la fenêtre soit masquée : aucune image rémanente.
const FLIGHT_SPEED: f64 = 1500.0; // px physiques par seconde à l'échelle 1
const FLIGHT_MIN_MS: f64 = 700.0;
const FLIGHT_MAX_MS: f64 = 1300.0;
const DEBOUNCE: Duration = Duration::from_millis(300); // la demande doit durer avant de lancer un trajet

// Out et In (trajet du cœur par le widget) ne sont plus lancés sur le Bureau JARVIS : le HUD déplace son cœur lui-même.
#[derive(Clone, Copy, PartialEq, Default, Debug)]
#[allow(dead_code)]
enum Phase {
    #[default]
    Off,
    Out,
    On,
    In,
}

struct Flight {
    inward: bool,
    park: (i32, i32), // coin haut-gauche de la fenêtre du widget une fois posé (ou à restaurer)
    side: i32,        // côté de la fenêtre du widget posé (px physiques)
    ready: bool,      // la page du widget a peint sa 1re image
    started: Instant,
}

// Déplacement du widget vers un endroit libre : lent (px physiques par seconde à l'échelle 1, durée
// bornée), et seulement quand l'endroit voulu n'a pas changé depuis SETTLE (une fenêtre qu'on
// traîne ne le fait pas errer).
const GLIDE_SPEED: f64 = 260.0;
const GLIDE_MIN_S: f64 = 0.9;
const GLIDE_MAX_S: f64 = 3.5;
const SETTLE: Duration = Duration::from_millis(700);
const FREE_MARGIN: f64 = 10.0; // écart laissé autour de l'orbe (px CSS)
const VD_KEY: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Explorer\\VirtualDesktops";

#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase", default)]
pub struct DeskConfig {
    pub enabled: bool,
    pub desktop: u32, // numéro du bureau virtuel (ordre de la vue des tâches), 2 par défaut
    pub monitor: u32, // 0 : écran principal ; n : n-ième écran de Windows
    pub other_screens: bool,
    pub wallpaper: bool,      // fond d'écran de Windows sous le HUD au lieu du fond noir (sans les icônes)
    pub wallpaper_dim: f64,   // voile de la couleur du thème posé sur ce fond d'écran (0 à 0,9)
}

impl Default for DeskConfig {
    fn default() -> Self {
        Self { enabled: false, desktop: 2, monitor: 0, other_screens: true, wallpaper: false, wallpaper_dim: 0.35 }
    }
}

#[derive(Default)]
pub struct Desk {
    cfg: Mutex<DeskConfig>,
    heart_r: Mutex<f64>, // rayon du cœur rapporté par la page « desk » (px CSS) ; 0 : cœur pas encore fini de grandir
    heart_seen: Mutex<bool>, // la page « desk » a dessiné au moins une image (elle a rapporté un rayon, même 0)
    // Cards visibles du HUD (x, y, largeur, hauteur en px CSS de la page « desk ») : le mini-cœur ne se pose
    // jamais dessus. Elles ne comptent que pour choisir sa place, pas pour savoir si le HUD est recouvert.
    card_rects: Mutex<Vec<[f64; 4]>>,
    pushed_visible: Mutex<Option<(bool, Option<Instant>)>>,
    on_target: Mutex<Option<bool>>, // bureau courant = Bureau JARVIS (dernier état appliqué)
    // Petit cœur du HUD (Bureau JARVIS) : centre en px physiques écran quand une application recouvre le cœur
    // du HUD et qu'il s'est rangé ailleurs, DANS le HUD (donc sous toutes les fenêtres) ; None : cœur au centre.
    mini: Mutex<Option<(i32, i32)>>,
    widget_owned: Mutex<bool>, // widget affiché par le Bureau JARVIS (autres bureaux, cœur recouvert)
    phase: Mutex<Phase>,       // propriétaire du cœur (voir Phase)
    flight: Mutex<Option<Flight>>,
    want: Mutex<Option<(bool, Instant)>>, // trajet demandé (vers le widget ou vers le HUD) et depuis quand
    home: Mutex<Option<(i32, i32)>>, // place choisie du widget (coin haut-gauche, px physiques)
    glide_to: Mutex<Option<(i32, i32)>>, // endroit où le widget glisse
    spot_pending: Mutex<Option<((i32, i32), Instant)>>, // nouvel endroit, en attente de stabilité
    dropped: Mutex<bool>, // le widget vient d'être posé à la main
    accept: Mutex<Option<u64>>, // posé sur une fenêtre : il y reste tant qu'aucune ne bouge
    target: Mutex<Option<u128>>, // bureau virtuel cible (GUID)
    resolve_pending: Mutex<bool>,
    goto_pending: Mutex<bool>,
    reveal_pending: Mutex<bool>,
    layout_sig: Mutex<String>,
    tray_item: Mutex<Option<tauri::menu::CheckMenuItem<tauri::Wry>>>,
}

pub type DeskState = std::sync::Arc<Desk>;

fn desk_of(app: &AppHandle) -> DeskState {
    app.state::<DeskState>().inner().clone()
}

pub fn enabled(app: &AppHandle) -> bool {
    desk_of(app).cfg.lock().unwrap().enabled
}

pub fn set_tray_item(app: &AppHandle, item: tauri::menu::CheckMenuItem<tauri::Wry>) {
    *desk_of(app).tray_item.lock().unwrap() = Some(item);
}

// Tests : les bureaux virtuels réels ne sont ni lus ni changés (le Bureau JARVIS est « le courant »).
fn no_virtual_desktops() -> bool {
    std::env::var("JARVIS_DESK_NO_VD").is_ok()
}

fn hwnd_of(w: &WebviewWindow) -> Option<HWND> {
    w.hwnd().ok().map(|h| HWND(h.0 as _))
}

fn rect_of(h: HWND) -> RECT {
    let mut r = RECT::default();
    unsafe {
        let _ = GetWindowRect(h, &mut r);
    }
    r
}

fn show_na(h: HWND) {
    unsafe {
        let _ = ShowWindow(h, SW_SHOWNOACTIVATE);
    }
}

fn hide(h: HWND) {
    unsafe {
        let _ = ShowWindow(h, SW_HIDE);
    }
}

// Réglage lu au lancement dans data/config.json, avant que le noyau ne l'envoie : la fenêtre
// classique ne s'affiche pas un instant pour rien quand le Bureau JARVIS est actif.
pub fn config_at_launch(state: &State) -> DeskConfig {
    let dir = crate::data_dir(&state.root);
    std::fs::read_to_string(dir.join("config.json"))
        .ok()
        .and_then(|t| serde_json::from_str::<Value>(&t).ok())
        .and_then(|v| serde_json::from_value::<DeskConfig>(v["desk"].clone()).ok())
        .unwrap_or_default()
}

// --- Bureaux virtuels (registre de l'Explorateur : ordre et bureau courant) -------------------------

fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}

fn reg_binary(value: &str) -> Vec<u8> {
    let key = wide(VD_KEY);
    let name = wide(value);
    let mut size: u32 = 0;
    unsafe {
        let (pk, pn) = (PCWSTR(key.as_ptr()), PCWSTR(name.as_ptr()));
        if RegGetValueW(HKEY_CURRENT_USER, pk, pn, RRF_RT_REG_BINARY, None, None, Some(&mut size)).is_err() || size == 0 {
            return Vec::new();
        }
        let mut buf = vec![0u8; size as usize];
        if RegGetValueW(HKEY_CURRENT_USER, pk, pn, RRF_RT_REG_BINARY, None, Some(buf.as_mut_ptr() as *mut _), Some(&mut size)).is_err() {
            return Vec::new();
        }
        buf.truncate(size as usize);
        buf
    }
}

fn guid_from_bytes(b: &[u8]) -> u128 {
    GUID {
        data1: u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
        data2: u16::from_le_bytes([b[4], b[5]]),
        data3: u16::from_le_bytes([b[6], b[7]]),
        data4: [b[8], b[9], b[10], b[11], b[12], b[13], b[14], b[15]],
    }
    .to_u128()
}

fn desktop_ids() -> Vec<u128> {
    reg_binary("VirtualDesktopIDs").chunks_exact(16).map(guid_from_bytes).collect()
}

fn current_desktop() -> Option<u128> {
    let b = reg_binary("CurrentVirtualDesktop");
    (b.len() >= 16).then(|| guid_from_bytes(&b[..16]))
}

fn key(vk: VIRTUAL_KEY, up: bool) -> INPUT {
    let mut flags = KEYBD_EVENT_FLAGS(0);
    if up {
        flags |= KEYEVENTF_KEYUP;
    }
    if vk == VK_LEFT || vk == VK_RIGHT {
        flags |= KEYEVENTF_EXTENDEDKEY;
    }
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: vk, wScan: 0, dwFlags: flags, time: 0, dwExtraInfo: 0 } },
    }
}

// Win+Ctrl+<touche> : seul moyen public de créer un bureau ou d'en changer.
fn win_ctrl(vk: VIRTUAL_KEY) {
    let seq = [key(VK_LWIN, false), key(VK_LCONTROL, false), key(vk, false), key(vk, true), key(VK_LCONTROL, true), key(VK_LWIN, true)];
    unsafe {
        SendInput(&seq, std::mem::size_of::<INPUT>() as i32);
    }
}

// Bureau cible : le n-ième de la vue des tâches. S'il manque, Jarvis crée les bureaux manquants
// (Win+Ctrl+D, qui y bascule) puis revient d'autant vers la gauche.
fn resolve_target(n: u32) -> Option<u128> {
    let n = n.clamp(1, 9) as usize;
    let mut ids = desktop_ids();
    if ids.is_empty() {
        return None; // registre illisible : on n'invente rien
    }
    if ids.len() < n {
        let missing = n - ids.len();
        for _ in 0..missing {
            win_ctrl(VK_D);
            thread::sleep(Duration::from_millis(600));
        }
        for _ in 0..missing {
            win_ctrl(VK_LEFT);
            thread::sleep(Duration::from_millis(250));
        }
        ids = desktop_ids();
    }
    ids.get(n - 1).copied()
}

fn switch_to(target: u128) -> bool {
    let ids = desktop_ids();
    let cur = current_desktop().and_then(|c| ids.iter().position(|&i| i == c));
    let to = ids.iter().position(|&i| i == target);
    let (Some(cur), Some(to)) = (cur, to) else { return false };
    let steps = to as i32 - cur as i32;
    for _ in 0..steps.abs() {
        win_ctrl(if steps > 0 { VK_RIGHT } else { VK_LEFT });
        thread::sleep(Duration::from_millis(90));
    }
    true
}

// Attente d'un changement de bureau (valeur du registre écrite par l'Explorateur) ou du délai.
struct DesktopWatch {
    key: HKEY,
    event: HANDLE,
}

impl DesktopWatch {
    fn open() -> Option<Self> {
        let k = wide(VD_KEY);
        let mut key = HKEY::default();
        unsafe {
            if RegOpenKeyExW(HKEY_CURRENT_USER, PCWSTR(k.as_ptr()), None, KEY_READ | KEY_NOTIFY, &mut key).is_err() {
                return None;
            }
            let event = CreateEventW(None, false, false, PCWSTR::null()).ok()?;
            Some(Self { key, event })
        }
    }

    fn wait(&self, ms: u32) {
        unsafe {
            let _ = RegNotifyChangeKeyValue(self.key, false, REG_NOTIFY_CHANGE_LAST_SET, Some(self.event), true);
            let _ = WaitForSingleObject(self.event, ms);
        }
    }
}

// --- Maintien en bas de pile ------------------------------------------------------------------------

unsafe extern "system" fn bottom_proc(hwnd: HWND, msg: u32, wparam: WPARAM, lparam: LPARAM, _id: usize, _data: usize) -> LRESULT {
    if msg == WM_WINDOWPOSCHANGING && lparam.0 != 0 {
        let wp = unsafe { &mut *(lparam.0 as *mut WINDOWPOS) };
        wp.hwndInsertAfter = HWND_BOTTOM;
        wp.flags &= !SWP_NOZORDER;
    }
    unsafe { DefSubclassProc(hwnd, msg, wparam, lparam) }
}

fn keep_at_bottom(app: &AppHandle, w: &WebviewWindow) {
    let Some(h) = hwnd_of(w) else { return };
    let raw = h.0 as isize;
    // Le sous-classement se fait sur le fil qui possède la fenêtre (la boucle principale).
    let _ = app.run_on_main_thread(move || unsafe {
        let h = HWND(raw as *mut _);
        let _ = SetWindowSubclass(h, Some(bottom_proc), 0x4A44_534B, 0);
        let _ = SetWindowPos(h, Some(HWND_BOTTOM), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
    });
}

// --- Fenêtres -----------------------------------------------------------------------------------

fn target_monitors(app: &AppHandle, cfg: &DeskConfig) -> (Option<tauri::Monitor>, Vec<tauri::Monitor>) {
    let all = app.available_monitors().unwrap_or_default();
    let primary = app.primary_monitor().ok().flatten();
    let main = if cfg.monitor >= 1 { all.get(cfg.monitor as usize - 1).cloned().or(primary) } else { primary };
    let main_pos = main.as_ref().map(|m| *m.position());
    let others = all.into_iter().filter(|m| Some(*m.position()) != main_pos).collect();
    (main, others)
}

fn layout_signature(app: &AppHandle, cfg: &DeskConfig) -> String {
    let (main, others) = target_monitors(app, cfg);
    let area = |m: &tauri::Monitor| {
        let a = m.work_area();
        format!("{},{},{},{}", a.position.x, a.position.y, a.size.width, a.size.height)
    };
    let mut s = main.as_ref().map(area).unwrap_or_default();
    if cfg.other_screens {
        for m in &others {
            s.push('|');
            s.push_str(&area(m));
        }
    }
    s
}

fn place(w: &WebviewWindow, m: &tauri::Monitor) {
    let a = m.work_area();
    let _ = w.set_position(PhysicalPosition::new(a.position.x, a.position.y));
    let _ = w.set_size(PhysicalSize::new(a.size.width, a.size.height));
}

fn build_backdrop(app: &AppHandle, label: &str, page: &str, role: &str, focusable: bool) -> tauri::Result<WebviewWindow> {
    let state: State = app.state::<State>().inner().clone();
    let b = WebviewWindowBuilder::new(app, label, WebviewUrl::App(page.into()))
        .title("JARVIS · bureau")
        .decorations(false)
        .resizable(false)
        .maximizable(false)
        .minimizable(false)
        .skip_taskbar(true)
        .shadow(false)
        .focused(false)
        .focusable(focusable)
        .visible(false)
        .initialization_script(&init_script(&state, role, label));
    // Fichier déposé sur le cœur du Bureau JARVIS (compagnon) : géré par la page (ui/bridge.js).
    let b = if role == "hud" { b.disable_drag_drop_handler() } else { b };
    let b = with_profile(match browser_args() { Some(a) => b.additional_browser_args(&a), None => b });
    b.build()
}

fn close(app: &AppHandle, label: &str) {
    if let Some(w) = app.get_webview_window(label) {
        let _ = w.destroy();
    }
}

fn bg_labels(app: &AppHandle) -> Vec<String> {
    app.webview_windows().keys().filter(|l| l.starts_with("desk-bg-")).cloned().collect()
}

fn backdrops(app: &AppHandle) -> Vec<HWND> {
    app.webview_windows()
        .into_iter()
        .filter(|(l, _)| l == "desk" || l.starts_with("desk-bg-"))
        .filter_map(|(_, w)| hwnd_of(&w))
        .collect()
}

// Widget flottant (autres bureaux, cœur du HUD recouvert) : affiché sans prendre le focus, clics
// traversants gérés par la veille du curseur de main.rs (widget_mode). `from` : centre d'où il
// apparaît avant de glisser à sa place (le cœur du HUD qu'il remplace).
// Règle de ce module : AUCUN verrou tenu pendant une opération de fenêtre. ShowWindow, SetWindowPos
// ou une création de fenêtre attendent la boucle principale ; si celle-ci attend le même verrou
// (commande desk_state), tout se fige.
fn set_desk_widget(app: &AppHandle, show: bool, from: Option<(i32, i32)>) {
    let desk = desk_of(app);
    let state: State = app.state::<State>().inner().clone();
    {
        let mut owned = desk.widget_owned.lock().unwrap();
        if show == *owned {
            return;
        }
        *owned = show;
    }
    *desk.glide_to.lock().unwrap() = None;
    *desk.spot_pending.lock().unwrap() = None;
    *desk.phase.lock().unwrap() = if show { Phase::On } else { Phase::Off };
    let flight = desk.flight.lock().unwrap().take();
    let w = match app.get_webview_window("widget") {
        Some(w) => Some(w),
        None if show => create_widget(app).ok(),
        None => None,
    };
    *state.widget_mode.lock().unwrap() = show;
    if let Some(h) = w.as_ref().and_then(hwnd_of) {
        // Trajet interrompu : la fenêtre (agrandie le temps du trajet) retrouve sa taille et sa place.
        if let Some(f) = flight {
            unsafe {
                let _ = SetWindowPos(h, None, f.park.0, f.park.1, f.side, f.side, SWP_NOZORDER | SWP_NOACTIVATE);
            }
        }
        if show {
            if desk.home.lock().unwrap().is_none() {
                *desk.home.lock().unwrap() = Some(rect_of(h)).map(|r| (r.left, r.top));
            }
            if let Some((cx, cy)) = from {
                let r = rect_of(h);
                let side = r.right - r.left;
                unsafe {
                    let _ = SetWindowPos(h, None, cx - side / 2, cy - side / 2, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
                }
            }
            show_na(h);
        } else {
            hide(h);
        }
    }
    emit_visibility(app, "widget");
    emit_owner(app, if show { "widget" } else { "hud" });
}

// Journal de débogage du trajet du cœur (fichier temporaire, seulement si JARVIS_DEBUG_COEUR est défini).
fn trace(msg: String) {
    if std::env::var("JARVIS_DEBUG_COEUR").is_err() {
        return;
    }
    use std::io::Write;
    if let Ok(mut f) = std::fs::OpenOptions::new().create(true).append(true).open(std::env::temp_dir().join("jarvis-coeur.log")) {
        let ms = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() % 100000).unwrap_or(0);
        let _ = writeln!(f, "{ms:05} {msg}");
    }
}

fn emit_owner(app: &AppHandle, owner: &str) {
    let _ = app.emit("jarvis:heart-owner", json!({ "label": "desk", "owner": owner }));
}

// Trajet en cours ? (la coquille ne traite alors ni le survol ni le placement du widget.)
pub fn flying(app: &AppHandle) -> bool {
    matches!(*desk_of(app).phase.lock().unwrap(), Phase::Out | Phase::In)
}

fn flight_ms(dist: f64, scale: f64) -> u64 {
    (dist / (FLIGHT_SPEED * scale) * 1000.0).clamp(FLIGHT_MIN_MS, FLIGHT_MAX_MS) as u64
}

// Cards du HUD en coordonnées écran (px physiques) : origine de la fenêtre « desk » + px CSS × échelle.
fn card_covers(app: &AppHandle, desk: &Desk) -> Vec<(isize, RECT)> {
    let rects = desk.card_rects.lock().unwrap().clone();
    if rects.is_empty() {
        return Vec::new();
    }
    let Some(dwin) = app.get_webview_window("desk") else { return Vec::new() };
    let Some(dh) = hwnd_of(&dwin) else { return Vec::new() };
    let origin = rect_of(dh);
    let k = dwin.scale_factor().unwrap_or(1.0);
    cards_to_screen(&rects, (origin.left, origin.top), k)
}

fn cards_to_screen(rects: &[[f64; 4]], origin: (i32, i32), k: f64) -> Vec<(isize, RECT)> {
    rects
        .iter()
        .map(|[x, y, w, h]| {
            let left = origin.0 + (x * k).floor() as i32;
            let top = origin.1 + (y * k).floor() as i32;
            (-1, RECT { left, top, right: left + (w * k).ceil() as i32, bottom: top + (h * k).ceil() as i32 })
        })
        .collect()
}

// Fenêtres d'applications + cards du HUD : tout ce sur quoi le mini-cœur ne doit pas se poser.
fn with_cards(covers: &[(isize, RECT)], cards: &[(isize, RECT)]) -> Vec<(isize, RECT)> {
    covers.iter().chain(cards.iter()).cloned().collect()
}

// 1re image du trajet peinte (ou délai dépassé) : pour un départ, le HUD laisse son cœur au widget.
fn mark_ready(app: &AppHandle, started: Instant) {
    let desk = desk_of(app);
    let outward = {
        let mut fl = desk.flight.lock().unwrap();
        match fl.as_mut() {
            Some(f) if f.started == started && !f.ready => {
                f.ready = true;
                !f.inward
            }
            _ => return,
        }
    };
    if outward {
        emit_owner(app, "widget");
    }
}

// Arrivée du cœur (page du widget, ou délai dépassé). Départ : la fenêtre redevient le petit carré du widget,
// posé à sa place. Retour : le HUD reprend son cœur à pleine opacité, puis la page efface son canvas et
// seulement ensuite la fenêtre est masquée.
fn land(app: &AppHandle) {
    let desk = desk_of(app);
    let phase = *desk.phase.lock().unwrap();
    if !matches!(phase, Phase::Out | Phase::In) {
        return;
    }
    let Some(f) = desk.flight.lock().unwrap().take() else { return };
    trace(format!("[cœur] arrivée (retour={})", f.inward));
    if !f.inward {
        if let Some(h) = app.get_webview_window("widget").as_ref().and_then(hwnd_of) {
            unsafe {
                let _ = SetWindowPos(h, None, f.park.0, f.park.1, f.side, f.side, SWP_NOZORDER | SWP_NOACTIVATE);
            }
        }
        *desk.phase.lock().unwrap() = Phase::On;
        let _ = app.emit("jarvis:heart-settled", json!({ "label": "widget", "dir": "out" }));
        emit_visibility(app, "widget");
        return;
    }
    emit_owner(app, "hud");
    let app2 = app.clone();
    thread::spawn(move || {
        thread::sleep(Duration::from_millis(170)); // le HUD a repris son cœur (fondu de 120 ms)
        let _ = app2.emit("jarvis:heart-settled", json!({ "label": "widget", "dir": "in" })); // la page efface son canvas
        thread::sleep(Duration::from_millis(60));
        let desk = desk_of(&app2);
        let state: State = app2.state::<State>().inner().clone();
        if let Some(h) = app2.get_webview_window("widget").as_ref().and_then(hwnd_of) {
            unsafe {
                let _ = SetWindowPos(h, None, f.park.0, f.park.1, f.side, f.side, SWP_NOZORDER | SWP_NOACTIVATE);
            }
            hide(h);
        }
        *desk.widget_owned.lock().unwrap() = false;
        *state.widget_mode.lock().unwrap() = false;
        *desk.phase.lock().unwrap() = Phase::Off;
        emit_visibility(&app2, "widget");
    });
}

// La demande doit durer DEBOUNCE avant d'agir (une fenêtre qu'on traîne ne fait pas aller-retour le cœur).
fn debounced(desk: &Desk, show: bool) -> bool {
    let mut w = desk.want.lock().unwrap();
    match *w {
        Some((s, since)) if s == show => {
            if since.elapsed() < DEBOUNCE {
                return false;
            }
            *w = None;
            true
        }
        _ => {
            *w = Some((show, Instant::now()));
            false
        }
    }
}

// Centre du petit cœur (px CSS de la page « desk ») pour un centre en px physiques écran.
fn mini_css(center: (i32, i32), origin: (i32, i32), k: f64) -> (f64, f64) {
    ((center.0 - origin.0) as f64 / k, (center.1 - origin.1) as f64 / k)
}

fn emit_spot(app: &AppHandle, mode: &str, css: Option<(f64, f64)>, r: f64, dur_ms: u64) {
    let (x, y) = css.unwrap_or((0.0, 0.0));
    let _ = app.emit("jarvis:heart-spot", json!({ "label": "desk", "mode": mode, "x": x, "y": y, "r": r, "durMs": dur_ms }));
}

// Cœur du HUD recouvert par une application : le HUD, qui est sous TOUTES les fenêtres, déplace lui-même son
// cœur (page « desk », jarvis:heart-spot) vers l'endroit libre le plus proche, en glissant ; il ne passe jamais
// par-dessus une fenêtre. Rien de libre : le cœur reste caché dessous. Libéré, il revient au centre.
fn drive_hud_heart(app: &AppHandle, desk: &Desk, covers: &[(isize, RECT)], rect: RECT, covered: bool) {
    let state: State = app.state::<State>().inner().clone();
    let Some(dwin) = app.get_webview_window("desk") else { return };
    let k = dwin.scale_factor().unwrap_or(1.0);
    let origin = (rect.left, rect.top);
    let hud = ((rect.left + rect.right) / 2, (rect.top + rect.bottom) / 2);
    let size = state.widget_cfg.lock().unwrap().size;
    let r_css = size * if size < 110.0 { 0.3 } else { 0.26 };
    let current = *desk.mini.lock().unwrap();
    if !covered {
        *desk.spot_pending.lock().unwrap() = None;
        if current.is_none() {
            *desk.want.lock().unwrap() = None;
            return;
        }
        if !debounced(desk, false) {
            return;
        }
        let from = current.unwrap_or(hud);
        let dist = (((from.0 - hud.0) as f64).powi(2) + ((from.1 - hud.1) as f64).powi(2)).sqrt();
        *desk.mini.lock().unwrap() = None;
        emit_spot(app, "hud", None, r_css, flight_ms(dist, k));
        return;
    }
    // Même règle de place que le widget : endroit libre le plus proche du cœur, jamais sur une card du HUD.
    let side = (widget_window_side(size) * k).round() as i32;
    let orb = size * crate::WIDGET_GROW * k;
    let pad = (((side as f64 - orb) / 2.0 - FREE_MARGIN * k).round() as i32).max(0);
    let home = (hud.0 - side / 2, hud.1 - side / 2);
    let blocked = with_cards(covers, &card_covers(app, desk));
    let current_tl = current.map(|c| (c.0 - side / 2, c.1 - side / 2));
    let wanted = pick_spot(app, side, pad, home, current_tl, Some(hud), &blocked);
    let wanted_c = (wanted.0 + side / 2, wanted.1 + side / 2);
    let Some(from) = current else {
        if !debounced(desk, true) {
            return;
        }
        let dist = (((wanted_c.0 - hud.0) as f64).powi(2) + ((wanted_c.1 - hud.1) as f64).powi(2)).sqrt();
        *desk.mini.lock().unwrap() = Some(wanted_c);
        *desk.spot_pending.lock().unwrap() = None;
        emit_spot(app, "mini", Some(mini_css(wanted_c, origin, k)), r_css, flight_ms(dist, k));
        return;
    };
    *desk.want.lock().unwrap() = None;
    let mut pending = desk.spot_pending.lock().unwrap();
    if wanted_c == from {
        *pending = None;
        return;
    }
    match *pending {
        Some((p, since)) if p == wanted => {
            if since.elapsed() >= SETTLE {
                *pending = None;
                drop(pending);
                let dist = (((wanted_c.0 - from.0) as f64).powi(2) + ((wanted_c.1 - from.1) as f64).powi(2)).sqrt();
                *desk.mini.lock().unwrap() = Some(wanted_c);
                let secs = (dist / (GLIDE_SPEED * k)).clamp(GLIDE_MIN_S, GLIDE_MAX_S);
                emit_spot(app, "mini", Some(mini_css(wanted_c, origin, k)), r_css, (secs * 1000.0) as u64);
            }
        }
        _ => *pending = Some((wanted, Instant::now())),
    }
}

// Crée, replace ou détruit les fenêtres du Bureau JARVIS d'après la configuration.
fn sync_windows(app: &AppHandle) {
    let desk = desk_of(app);
    let cfg = desk.cfg.lock().unwrap().clone();
    if !cfg.enabled {
        close(app, "desk");
        for l in bg_labels(app) {
            close(app, &l);
        }
        *desk.pushed_visible.lock().unwrap() = None;
        *desk.on_target.lock().unwrap() = None;
        set_desk_widget(app, false, None);
        return;
    }
    let (main, others) = target_monitors(app, &cfg);
    let Some(main) = main else { return };
    let desk_win = match app.get_webview_window("desk") {
        Some(w) => w,
        None => match build_backdrop(app, "desk", "index.html", "hud", true) {
            Ok(w) => {
                keep_at_bottom(app, &w);
                w
            }
            Err(err) => {
                eprintln!("[coquille] bureau JARVIS impossible : {err}");
                return;
            }
        },
    };
    place(&desk_win, &main);
    let wanted: Vec<(String, &tauri::Monitor)> =
        if cfg.other_screens { others.iter().enumerate().map(|(i, m)| (format!("desk-bg-{}", i + 1), m)).collect() } else { Vec::new() };
    for l in bg_labels(app) {
        if !wanted.iter().any(|(w, _)| *w == l) {
            close(app, &l);
        }
    }
    for (label, m) in &wanted {
        let w = match app.get_webview_window(label) {
            Some(w) => w,
            None => match build_backdrop(app, label, "desk-bg.html", "desk-bg", false) {
                Ok(w) => {
                    keep_at_bottom(app, &w);
                    w
                }
                Err(_) => continue,
            },
        };
        place(&w, m);
    }
    *desk.layout_sig.lock().unwrap() = layout_signature(app, &cfg);
    // La veille réapplique l'affichage selon le bureau courant (nouvelles fenêtres encore masquées).
    *desk.on_target.lock().unwrap() = None;
    // Widget créé d'avance (masqué) : sa page écoute déjà le trajet du cœur au premier recouvrement ; sans
    // cela le tout premier départ n'aurait pas d'animation (page pas encore chargée).
    if app.get_webview_window("widget").is_none() && create_widget(app).is_ok() {
        let app2 = app.clone();
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(2500));
            emit_visibility(&app2, "widget"); // masqué : sa page cesse de dessiner
        });
    }
}

pub fn apply_config(app: &AppHandle, cfg: DeskConfig) {
    let desk = desk_of(app);
    let changed_desktop = desk.cfg.lock().unwrap().desktop != cfg.desktop || desk.target.lock().unwrap().is_none();
    *desk.cfg.lock().unwrap() = cfg.clone();
    if changed_desktop {
        *desk.resolve_pending.lock().unwrap() = true;
    }
    if let Some(item) = desk.tray_item.lock().unwrap().as_ref() {
        let _ = item.set_checked(cfg.enabled);
    }
    let _ = app.emit("jarvis:wallpaper", json!({})); // les pages du bureau relisent le fond (desk_wallpaper)
    let app2 = app.clone();
    // Création de fenêtres hors de la boucle principale (voir la note des commandes dans main.rs).
    thread::spawn(move || {
        // Bureau JARVIS actif : la fenêtre classique est DÉTRUITE (sa page rend 200 à 300 Mo) ;
        // inactif : elle revient.
        if cfg.enabled {
            close(&app2, "hud");
        } else if app2.get_webview_window("hud").is_none() {
            if let Err(err) = create_hud(&app2, true) {
                eprintln!("[coquille] fenêtre principale impossible : {err}");
            }
        }
        sync_windows(&app2);
    });
}

pub fn goto(app: &AppHandle) {
    *desk_of(app).goto_pending.lock().unwrap() = true;
}

// --- Recouvrement ---------------------------------------------------------------------------------

struct Scan {
    stop_at: isize,
    ignore: Vec<isize>,
    covers: Vec<(isize, RECT)>,
}

fn class_name(h: HWND) -> String {
    let mut buf = [0u16; 64];
    let n = unsafe { GetClassNameW(h, &mut buf) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

// Fenêtres qui cachent vraiment le HUD : visibles, non réduites, du bureau courant, et pas des calques
// invisibles posés sur tout l'écran (NVIDIA GeForce Overlay, barres de jeu : WS_EX_NOACTIVATE,
// fenêtres outils superposées, clics traversants).
unsafe extern "system" fn scan_proc(h: HWND, lparam: LPARAM) -> BOOL {
    let scan = unsafe { &mut *(lparam.0 as *mut Scan) };
    let id = h.0 as isize;
    if id == scan.stop_at {
        return BOOL(0); // tout ce qui suit est sous le HUD
    }
    if scan.ignore.contains(&id) {
        return BOOL(1);
    }
    unsafe {
        if !IsWindowVisible(h).as_bool() || IsIconic(h).as_bool() {
            return BOOL(1);
        }
        let ex = GetWindowLongPtrW(h, GWL_EXSTYLE);
        let has = |flag: u32| ex & flag as isize != 0;
        if has(WS_EX_TRANSPARENT.0) || has(WS_EX_NOACTIVATE.0) || (has(WS_EX_TOOLWINDOW.0) && has(WS_EX_LAYERED.0)) {
            return BOOL(1);
        }
        let mut cloaked: u32 = 0;
        if DwmGetWindowAttribute(h, DWMWA_CLOAKED, &mut cloaked as *mut u32 as *mut _, 4).is_ok() && cloaked != 0 {
            return BOOL(1); // autre bureau virtuel, application en veille
        }
        let mut r = RECT::default();
        if GetWindowRect(h, &mut r).is_err() || r.right - r.left < 32 || r.bottom - r.top < 32 {
            return BOOL(1);
        }
        // « #32768 » : menus contextuels (dont celui du widget : il ne doit pas le faire fuir).
        if matches!(class_name(h).as_str(), "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd" | "#32768") {
            return BOOL(1);
        }
        scan.covers.push((id, r));
    }
    BOOL(1)
}

fn windows_above(stop_at: isize, ignore: Vec<isize>) -> Vec<(isize, RECT)> {
    let mut scan = Scan { stop_at, ignore, covers: Vec::new() };
    unsafe {
        let _ = EnumWindows(Some(scan_proc), LPARAM(&mut scan as *mut Scan as isize));
    }
    scan.covers
}

fn contains(r: &RECT, x: i32, y: i32) -> bool {
    x >= r.left && x < r.right && y >= r.top && y < r.bottom
}

fn ignored_hwnds(app: &AppHandle) -> Vec<isize> {
    app.webview_windows()
        .into_iter()
        .filter(|(l, _)| l == "widget" || l.starts_with("desk-bg-"))
        .filter_map(|(_, w)| hwnd_of(&w).map(|h| h.0 as isize))
        .collect()
}

fn push_visibility(app: &AppHandle, desk: &Desk, visible: bool) {
    let mut last = desk.pushed_visible.lock().unwrap();
    // Changement, ou rappel toutes les 3 s (une page rechargée repart « visible »).
    let due = match *last {
        Some((v, at)) => v != visible || at.map_or(true, |t| t.elapsed() > Duration::from_secs(3)),
        None => true,
    };
    if due {
        *last = Some((visible, Some(Instant::now())));
        let _ = app.emit("jarvis:visibility", json!({ "label": "desk", "visible": visible }));
    }
}

// --- Place du widget : endroit libre le plus proche, atteint en glissant --------------------------

// Zone de travail (px physiques) de l'écran qui contient le point.
fn work_area_at(app: &AppHandle, x: i32, y: i32) -> Option<RECT> {
    let all = app.available_monitors().unwrap_or_default();
    let inside = |m: &tauri::Monitor| {
        let (p, s) = (m.position(), m.size());
        x >= p.x && x < p.x + s.width as i32 && y >= p.y && y < p.y + s.height as i32
    };
    let m = all.iter().find(|m| inside(m)).cloned().or_else(|| app.primary_monitor().ok().flatten())?;
    let a = m.work_area();
    Some(RECT { left: a.position.x, top: a.position.y, right: a.position.x + a.size.width as i32, bottom: a.position.y + a.size.height as i32 })
}

fn covers_signature(covers: &[(isize, RECT)]) -> u64 {
    use std::hash::{Hash, Hasher};
    let mut h = std::collections::hash_map::DefaultHasher::new();
    for (id, r) in covers {
        (id, r.left, r.top, r.right, r.bottom).hash(&mut h);
    }
    h.finish()
}

fn axis(from: i32, to: i32, step: usize) -> Vec<i32> {
    let mut v: Vec<i32> = (from..=to).step_by(step).collect();
    if v.last() != Some(&to) {
        v.push(to);
    }
    v
}

// Coin haut-gauche de la fenêtre (côté `side`) dont l'orbe (fenêtre moins `pad` de chaque côté)
// ne touche aucune fenêtre : la place choisie si elle est libre, sinon l'endroit actuel s'il l'est
// resté (pas de va-et-vient), sinon le plus proche de la place choisie. Rien de libre (application
// en plein écran) : la place choisie.
fn free_spot(area: RECT, side: i32, pad: i32, home: (i32, i32), keep: Option<(i32, i32)>, covers: &[(isize, RECT)]) -> (i32, i32) {
    let (x0, x1) = (area.left - pad, area.right - side + pad);
    let (y0, y1) = (area.top - pad, area.bottom - side + pad);
    if x1 < x0 || y1 < y0 {
        return home;
    }
    let clamp = |(x, y): (i32, i32)| (x.clamp(x0, x1), y.clamp(y0, y1));
    let free = |(x, y): (i32, i32)| {
        let f = RECT { left: x + pad, top: y + pad, right: x + side - pad, bottom: y + side - pad };
        !covers.iter().any(|(_, r)| r.left < f.right && r.right > f.left && r.top < f.bottom && r.bottom > f.top)
    };
    let home = clamp(home);
    if free(home) {
        return home;
    }
    if let Some(k) = keep.map(clamp).filter(|&k| free(k)) {
        return k;
    }
    let step = (side / 4).max(12) as usize;
    let dist = |(x, y): (i32, i32)| (x - home.0) as i64 * (x - home.0) as i64 + (y - home.1) as i64 * (y - home.1) as i64;
    let mut best: Option<(i64, (i32, i32))> = None;
    for y in axis(y0, y1, step) {
        for x in axis(x0, x1, step) {
            let d = dist((x, y));
            if best.map_or(true, |(b, _)| d < b) && free((x, y)) {
                best = Some((d, (x, y)));
            }
        }
    }
    best.map(|(_, p)| p).unwrap_or(home)
}

// Le carré d'orbe de la fenêtre posée en (x, y) ne touche-t-il aucune fenêtre ?
fn spot_free(covers: &[(isize, RECT)], side: i32, pad: i32, (x, y): (i32, i32)) -> bool {
    let f = RECT { left: x + pad, top: y + pad, right: x + side - pad, bottom: y + side - pad };
    !covers.iter().any(|(_, r)| r.left < f.right && r.right > f.left && r.top < f.bottom && r.bottom > f.top)
}

// Où le widget doit se poser : dans un endroit SANS APPLICATION, le plus proche du cœur, sur l'écran du HUD
// si possible (il n'est pas obligé d'aller sur l'autre écran).
//   1. il est déjà dans un endroit libre : il n'en bouge pas ;
//   2. sa place habituelle (`home`) si elle est libre ET sur l'écran du HUD ;
//   3. sinon l'endroit libre le plus proche du centre du HUD, sur l'écran du HUD ;
//   4. rien de libre sur cet écran (applications partout) : comme avant, autour de sa place habituelle.
// `hud` : centre du HUD (None sur un autre bureau virtuel : règle d'avant, autour de `home`).
fn pick_spot(app: &AppHandle, side: i32, pad: i32, home: (i32, i32), current: Option<(i32, i32)>, hud: Option<(i32, i32)>, covers: &[(isize, RECT)]) -> (i32, i32) {
    let around_home = |keep: Option<(i32, i32)>| match work_area_at(app, home.0 + side / 2, home.1 + side / 2) {
        Some(a) => free_spot(a, side, pad, home, keep, covers),
        None => home,
    };
    let Some(hud) = hud else { return around_home(current) };
    if let Some(c) = current.filter(|&c| spot_free(covers, side, pad, c)) {
        return c;
    }
    if let Some(area) = work_area_at(app, hud.0, hud.1) {
        let (hx, hy) = (home.0 + side / 2, home.1 + side / 2);
        let home_here = hx >= area.left && hx < area.right && hy >= area.top && hy < area.bottom;
        let anchor = if home_here { home } else { (hud.0 - side / 2, hud.1 - side / 2) };
        let p = free_spot(area, side, pad, anchor, None, covers);
        if spot_free(covers, side, pad, p) {
            return p;
        }
    }
    around_home(None)
}

fn widget_held(state: &State) -> bool {
    *state.widget_interactive.lock().unwrap() || state.last_drag.lock().unwrap().elapsed() < Duration::from_millis(400)
}

// Choisit où le widget doit être (appelé par la veille, 4 fois par seconde).
fn steer_widget(app: &AppHandle, desk: &Desk, covers: &[(isize, RECT)], hud: Option<(i32, i32)>) {
    let state: State = app.state::<State>().inner().clone();
    if widget_held(&state) || flying(app) {
        return; // survolé ou déplacé à la main (il ne se dérobe pas sous la souris), ou cœur en vol
    }
    let Some(w) = app.get_webview_window("widget") else { return };
    let Some(h) = hwnd_of(&w) else { return };
    let wr = rect_of(h);
    let side = wr.right - wr.left;
    let scale = w.scale_factor().unwrap_or(1.0);
    let Some(home) = *desk.home.lock().unwrap() else { return };
    let sig = covers_signature(covers);
    if std::mem::take(&mut *desk.dropped.lock().unwrap()) {
        *desk.accept.lock().unwrap() = Some(sig);
    }
    let accepted = {
        let mut accept = desk.accept.lock().unwrap();
        if *accept != Some(sig) {
            *accept = None;
        }
        accept.is_some()
    };
    if work_area_at(app, home.0 + side / 2, home.1 + side / 2).is_none() {
        return;
    }
    // Empreinte : l'orbe à sa taille maximale (elle grossit quand Jarvis intervient), plus une marge,
    // pas le carré transparent de la fenêtre.
    let orb = state.widget_cfg.lock().unwrap().size * crate::WIDGET_GROW * scale;
    let pad = (((side as f64 - orb) / 2.0 - FREE_MARGIN * scale).round() as i32).max(0);
    let current = *desk.glide_to.lock().unwrap();
    // Cards du HUD (seulement sur le Bureau JARVIS, où le HUD est affiché) : le mini-cœur ne reste jamais dessus,
    // même posé à la main ; une card qui s'ouvre sous lui le fait glisser vers une zone vide.
    let cards = if hud.is_some() { card_covers(app, desk) } else { Vec::new() };
    let blocked = with_cards(covers, &cards);
    let accepted = accepted && spot_free(&cards, side, pad, home);
    let wanted = if accepted { home } else { pick_spot(app, side, pad, home, current, hud, &blocked) };

    let mut pending = desk.spot_pending.lock().unwrap();
    if current.is_none() || current == Some(wanted) {
        *pending = None;
        *desk.glide_to.lock().unwrap() = Some(wanted);
        return;
    }
    match *pending {
        Some((p, since)) if p == wanted => {
            if since.elapsed() >= SETTLE {
                *pending = None;
                *desk.glide_to.lock().unwrap() = Some(wanted);
            }
        }
        _ => *pending = Some((wanted, Instant::now())),
    }
}

struct Glide {
    from: (f64, f64),
    to: (i32, i32),
    start: Instant,
    secs: f64,
}

// Animation du widget vers glide_to (~60 images par seconde, lente et adoucie).
fn start_glide(app: AppHandle) {
    thread::spawn(move || {
        let mut glide: Option<Glide> = None;
        loop {
            thread::sleep(Duration::from_millis(16));
            let state: State = app.state::<State>().inner().clone();
            if *state.quitting.lock().unwrap() {
                return;
            }
            let desk = desk_of(&app);
            let owned = *desk.widget_owned.lock().unwrap();
            let target = *desk.glide_to.lock().unwrap();
            let (true, Some(to), false) = (owned && !flying(&app), target, widget_held(&state)) else {
                glide = None;
                thread::sleep(Duration::from_millis(100));
                continue;
            };
            let Some(h) = app.get_webview_window("widget").as_ref().and_then(hwnd_of) else { continue };
            let r = rect_of(h);
            let cur = (r.left, r.top);
            let done = glide.as_ref().map_or(true, |g| g.to != to || g.start.elapsed().as_secs_f64() >= g.secs);
            if done && cur == to {
                glide = None;
                thread::sleep(Duration::from_millis(100));
                continue;
            }
            // Nouvel endroit, ou fenêtre déplacée d'ailleurs (apparition au centre du HUD) : on repart d'ici.
            if done {
                let scale = app.get_webview_window("widget").and_then(|w| w.scale_factor().ok()).unwrap_or(1.0);
                let dist = (((to.0 - cur.0) as f64).powi(2) + ((to.1 - cur.1) as f64).powi(2)).sqrt();
                let secs = (dist / (GLIDE_SPEED * scale)).clamp(GLIDE_MIN_S, GLIDE_MAX_S);
                glide = Some(Glide { from: (cur.0 as f64, cur.1 as f64), to, start: Instant::now(), secs });
            }
            let Some(g) = glide.as_ref() else { continue };
            let p = (g.start.elapsed().as_secs_f64() / g.secs).min(1.0);
            let e = if p < 0.5 { 4.0 * p * p * p } else { 1.0 - (-2.0 * p + 2.0).powi(3) / 2.0 };
            let x = (g.from.0 + (to.0 as f64 - g.from.0) * e).round() as i32;
            let y = (g.from.1 + (to.1 as f64 - g.from.1) * e).round() as i32;
            if (x, y) != cur {
                unsafe {
                    let _ = SetWindowPos(h, None, x, y, 0, 0, SWP_NOSIZE | SWP_NOZORDER | SWP_NOACTIVATE);
                }
            }
        }
    });
}

// Widget posé à la main (ou recentré après un changement de taille) : c'est sa nouvelle place.
pub fn widget_dropped(app: &AppHandle) {
    let desk = desk_of(app);
    let Some(h) = app.get_webview_window("widget").as_ref().and_then(hwnd_of) else { return };
    let r = rect_of(h);
    *desk.home.lock().unwrap() = Some((r.left, r.top));
    *desk.glide_to.lock().unwrap() = Some((r.left, r.top));
    *desk.spot_pending.lock().unwrap() = None;
    *desk.dropped.lock().unwrap() = true;
}

// --- Veille -----------------------------------------------------------------------------------------

pub fn start_watch(app: AppHandle) {
    start_glide(app.clone());
    thread::spawn(move || {
        let watch = if no_virtual_desktops() { None } else { DesktopWatch::open() };
        let mut last_layout_check = Instant::now();
        loop {
            match watch.as_ref() {
                Some(w) => w.wait(250),
                None => thread::sleep(Duration::from_millis(250)),
            }
            let state: State = app.state::<State>().inner().clone();
            if *state.quitting.lock().unwrap() {
                return;
            }
            let desk = desk_of(&app);
            let cfg = desk.cfg.lock().unwrap().clone();
            if !cfg.enabled {
                thread::sleep(Duration::from_millis(250));
                continue;
            }

            if std::mem::take(&mut *desk.resolve_pending.lock().unwrap()) && !no_virtual_desktops() {
                *desk.target.lock().unwrap() = resolve_target(cfg.desktop);
            }
            let target = *desk.target.lock().unwrap();

            if std::mem::take(&mut *desk.goto_pending.lock().unwrap()) {
                let switched = target.map_or(false, switch_to);
                if !switched {
                    if let Some(w) = app.get_webview_window("desk") {
                        let _ = w.set_focus();
                    }
                }
            }

            // Bureau affiché : le Bureau JARVIS, ou un autre (widget seul).
            let on_target = no_virtual_desktops() || target.is_none() || current_desktop() == target;
            let changed = *desk.on_target.lock().unwrap() != Some(on_target);
            if changed {
                *desk.on_target.lock().unwrap() = Some(on_target);
                for h in backdrops(&app) {
                    if on_target {
                        show_na(h);
                        unsafe {
                            let _ = SetWindowPos(h, Some(HWND_BOTTOM), 0, 0, 0, 0, SWP_NOMOVE | SWP_NOSIZE | SWP_NOACTIVATE);
                        }
                    } else {
                        hide(h);
                    }
                }
            }

            let Some(desk_win) = app.get_webview_window("desk") else { continue };
            let Some(dh) = hwnd_of(&desk_win) else { continue };
            let rect = rect_of(dh);
            if !on_target {
                push_visibility(&app, &desk, false);
                // Un trajet en cours ne se termine pas sur un autre bureau : le widget s'y pose tout de suite.
                if flying(&app) {
                    interrupt_flight(&app);
                }
                set_desk_widget(&app, true, None);
                steer_widget(&app, &desk, &windows_above(0, ignored_hwnds(&app)), None);
                continue;
            }

            // « Afficher le bureau » (Win+D) ou réduction : le fond revient aussitôt.
            unsafe {
                if IsIconic(dh).as_bool() || !IsWindowVisible(dh).as_bool() {
                    show_na(dh);
                }
            }

            let covers = windows_above(dh.0 as isize, ignored_hwnds(&app));

            if std::mem::take(&mut *desk.reveal_pending.lock().unwrap()) {
                for (h, r) in &covers {
                    let overlaps = r.left < rect.right && r.right > rect.left && r.top < rect.bottom && r.bottom > rect.top;
                    let h = HWND(*h as *mut _);
                    let minimizable = unsafe { GetWindowLongPtrW(h, GWL_STYLE) } & WS_MINIMIZEBOX.0 as isize != 0;
                    if overlaps && minimizable {
                        unsafe {
                            let _ = ShowWindow(h, SW_SHOWMINNOACTIVE);
                        }
                    }
                }
                continue;
            }

            let (cx, cy) = ((rect.left + rect.right) / 2, (rect.top + rect.bottom) / 2);
            let covered = |x: i32, y: i32| covers.iter().any(|(_, r)| contains(r, x, y));
            let heart_covered = covered(cx, cy);
            let inset = 24;
            let fully = heart_covered
                && covered(rect.left + inset, rect.top + inset)
                && covered(rect.right - inset, rect.top + inset)
                && covered(rect.left + inset, rect.bottom - inset)
                && covered(rect.right - inset, rect.bottom - inset);
            push_visibility(&app, &desk, !fully);

            // Cœur du HUD recouvert (une fois la page dessinée) : le widget prend le relais, parti
            // du centre du HUD.
            // Page dessinée (elle a rapporté un rayon, même 0) : sans rayon connu, le widget apparaît directement.
            let has_heart = *desk.heart_seen.lock().unwrap();
            // Sur le Bureau JARVIS, plus de widget au-dessus des applications : le HUD déplace son propre cœur.
            if flying(&app) {
                interrupt_flight(&app);
            }
            set_desk_widget(&app, false, None); // sans effet si déjà caché
            drive_hud_heart(&app, &desk, &covers, rect, heart_covered && has_heart);

            // Écran branché, débranché, résolution ou barre des tâches modifiées.
            if last_layout_check.elapsed() > Duration::from_secs(2) {
                last_layout_check = Instant::now();
                if layout_signature(&app, &cfg) != *desk.layout_sig.lock().unwrap() {
                    let app2 = app.clone();
                    thread::spawn(move || sync_windows(&app2));
                }
            }
        }
    });
}

// --- Commandes appelées par les pages --------------------------------------------------------------
// Asynchrones : elles ne tournent jamais sur la boucle principale (voir set_desk_widget).

// Cards visibles du HUD (px CSS de la page « desk »), rapportées par la page quand elles changent.
#[tauri::command]
pub async fn desk_cards(app: AppHandle, rects: Vec<[f64; 4]>) {
    let clean: Vec<[f64; 4]> = rects
        .into_iter()
        .filter(|r| r.iter().all(|v| v.is_finite()) && r[2] > 0.0 && r[3] > 0.0)
        .take(64)
        .collect();
    *desk_of(&app).card_rects.lock().unwrap() = clean;
}

#[tauri::command]
pub async fn desk_heart(app: AppHandle, r: f64) {
    if r.is_finite() && r >= 0.0 {
        *desk_of(&app).heart_r.lock().unwrap() = r; // 0 : cœur encore en train de grandir, pas de trajet possible
        *desk_of(&app).heart_seen.lock().unwrap() = true;
    }
}

// 1re image du trajet peinte par la page du widget : le HUD peut laisser son cœur.
#[tauri::command]
pub async fn heart_ready(app: AppHandle) {
    trace("[cœur] heart_ready reçu de la page".to_string());
    let started = desk_of(&app).flight.lock().unwrap().as_ref().map(|f| f.started);
    if let Some(s) = started {
        mark_ready(&app, s);
    }
}

// Le cœur est arrivé (fin de l'animation de la page du widget).
#[tauri::command]
pub async fn heart_landed(app: AppHandle) {
    trace("[cœur] heart_landed reçu de la page".to_string());
    land(&app);
}

// Trajet interrompu (changement de bureau) : le widget reste ou se pose tout de suite, sans animation.
fn interrupt_flight(app: &AppHandle) {
    let desk = desk_of(app);
    let Some(f) = desk.flight.lock().unwrap().take() else { return };
    if let Some(h) = app.get_webview_window("widget").as_ref().and_then(hwnd_of) {
        unsafe {
            let _ = SetWindowPos(h, None, f.park.0, f.park.1, f.side, f.side, SWP_NOZORDER | SWP_NOACTIVATE);
        }
    }
    *desk.phase.lock().unwrap() = Phase::On;
    let _ = app.emit("jarvis:heart-settled", json!({ "label": "widget", "dir": "cancel" }));
    emit_owner(app, "widget");
}

#[tauri::command]
pub async fn desk_goto(app: AppHandle) {
    goto(&app);
}

// Bouton « Mode application » du Bureau JARVIS : retour à la fenêtre classique. Le noyau enregistre
// le réglage puis renvoie desk_config (fenêtres du bureau détruites, fenêtre classique recréée).
#[tauri::command]
pub async fn desk_leave(app: AppHandle) {
    leave(&app);
}

// Bouton « Mode bureau » de la fenêtre classique : active le Bureau JARVIS et y conduit.
#[tauri::command]
pub async fn desk_enter(app: AppHandle) {
    enter(&app);
}

pub fn enter(app: &AppHandle) {
    let state: State = app.state::<State>().inner().clone();
    crate::send_core(&state, json!({ "cmd": "desk_set", "enabled": true }));
    goto(app); // traité par la veille une fois le bureau cible trouvé
}

pub fn leave(app: &AppHandle) {
    let state: State = app.state::<State>().inner().clone();
    crate::send_core(&state, json!({ "cmd": "desk_set", "enabled": false }));
}

// Double-clic sur le widget : réduit les fenêtres qui recouvrent le HUD (après être revenu au
// Bureau JARVIS si besoin : la veille ne le fait que sur celui-ci).
#[tauri::command]
pub async fn desk_reveal(app: AppHandle) {
    let desk = desk_of(&app);
    if *desk.on_target.lock().unwrap() != Some(true) {
        goto(&app);
    }
    *desk.reveal_pending.lock().unwrap() = true;
}

// Petit cœur rangé dans le HUD : { x, y, r } en px CSS de la page « desk » (page rechargée), sinon null.
fn mini_state(app: &AppHandle, desk: &Desk) -> Value {
    let Some(c) = *desk.mini.lock().unwrap() else { return Value::Null };
    let Some(dwin) = app.get_webview_window("desk") else { return Value::Null };
    let Some(dh) = hwnd_of(&dwin) else { return Value::Null };
    let r = rect_of(dh);
    let state: State = app.state::<State>().inner().clone();
    let size = state.widget_cfg.lock().unwrap().size;
    let (x, y) = mini_css(c, (r.left, r.top), dwin.scale_factor().unwrap_or(1.0));
    json!({ "x": x, "y": y, "r": size * if size < 110.0 { 0.3 } else { 0.26 } })
}

#[tauri::command]
pub async fn desk_state(app: AppHandle) -> Value {
    let desk = desk_of(&app);
    let visible = desk.pushed_visible.lock().unwrap().map(|(v, _)| v).unwrap_or(false);
    let on_desk = desk.on_target.lock().unwrap().unwrap_or(false);
    let widget = *desk.widget_owned.lock().unwrap();
    json!({
        "enabled": desk.cfg.lock().unwrap().enabled,
        "visible": visible,
        "onDesk": on_desk,
        "coeur": on_desk && widget, // cœur du HUD recouvert : le widget le remplace
        "owner": match *desk.phase.lock().unwrap() { Phase::Off => "hud", Phase::Out => "vers-widget", Phase::On => "widget", Phase::In => "vers-hud" },
        "widget": widget,
        "widgetAt": *desk.glide_to.lock().unwrap(),
        "mini": mini_state(&app, &desk)
    })
}


// --- Fond d'écran de Windows sous le HUD (réglage « wallpaper ») ---------------------------------------
// Les fenêtres du bureau restent opaques (sinon les icônes de l'Explorateur, juste dessous, se verraient) :
// c'est l'image elle-même qui est relue et posée par les pages sous leurs effets (ui/wallpaper.js).

const WALLPAPER_MAX: u64 = 48 * 1024 * 1024;

fn reg_sz(key: &str, value: &str) -> Option<String> {
    let (k, n) = (wide(key), wide(value));
    let mut buf = [0u16; 260];
    let mut size = (buf.len() * 2) as u32;
    unsafe {
        RegGetValueW(HKEY_CURRENT_USER, PCWSTR(k.as_ptr()), PCWSTR(n.as_ptr()), RRF_RT_REG_SZ, None, Some(buf.as_mut_ptr() as *mut _), Some(&mut size))
            .ok()
            .ok()?;
    }
    let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    Some(String::from_utf16_lossy(&buf[..len]).trim().to_string())
}

// Image affichée par Windows : le fichier choisi, sinon la copie que Windows en garde (diaporama,
// Windows à la une, fichier d'origine déplacé).
fn wallpaper_file() -> Option<std::path::PathBuf> {
    let mut buf = [0u16; 1024];
    let ok = unsafe {
        SystemParametersInfoW(SPI_GETDESKWALLPAPER, buf.len() as u32, Some(buf.as_mut_ptr() as *mut _), SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0)).is_ok()
    };
    if ok {
        let len = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
        let p = std::path::PathBuf::from(String::from_utf16_lossy(&buf[..len]));
        if len > 0 && p.is_file() {
            return Some(p);
        }
    }
    let p = std::path::PathBuf::from(std::env::var("APPDATA").ok()?).join("Microsoft\\Windows\\Themes\\TranscodedWallpaper");
    p.is_file().then_some(p)
}

// Ajustement choisi dans Paramètres › Personnalisation (WallpaperStyle, TileWallpaper) en équivalent CSS.
fn wallpaper_fit() -> &'static str {
    match reg_sz("Control Panel\\Desktop", "WallpaperStyle").as_deref() {
        Some("6") => "contain",
        Some("2") => "stretch",
        Some("0") if reg_sz("Control Panel\\Desktop", "TileWallpaper").as_deref() == Some("1") => "tile",
        Some("0") => "center",
        _ => "cover", // 10 (remplir), 22 (étendre) et inconnu
    }
}

// Couleur unie de Windows (« R G B »), sous l'image ou à sa place.
fn wallpaper_color() -> String {
    let rgb: Vec<u8> = reg_sz("Control Panel\\Colors", "Background")
        .unwrap_or_default()
        .split_whitespace()
        .filter_map(|v| v.parse().ok())
        .collect();
    match rgb[..] {
        [r, g, b] => format!("rgb({r}, {g}, {b})"),
        _ => "rgb(0, 0, 0)".to_string(),
    }
}

fn image_mime(b: &[u8]) -> &'static str {
    match b {
        [0x89, b'P', b'N', b'G', ..] => "image/png",
        [b'B', b'M', ..] => "image/bmp",
        [b'G', b'I', b'F', ..] => "image/gif",
        [b'R', b'I', b'F', b'F', _, _, _, _, b'W', b'E', b'B', b'P', ..] => "image/webp",
        _ => "image/jpeg",
    }
}

fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(data.len().div_ceil(3) * 4);
    for c in data.chunks(3) {
        let n = (c[0] as u32) << 16 | (*c.get(1).unwrap_or(&0) as u32) << 8 | *c.get(2).unwrap_or(&0) as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if c.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if c.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

// { on, sig, fit, color, dim, url? } : l'image (data: URL) n'est renvoyée que si elle a changé depuis `sig`.
#[tauri::command]
pub async fn desk_wallpaper(app: AppHandle, sig: Option<String>) -> Value {
    let cfg = desk_of(&app).cfg.lock().unwrap().clone();
    if !cfg.enabled || !cfg.wallpaper {
        return json!({ "on": false });
    }
    let mut res = json!({ "on": true, "fit": wallpaper_fit(), "color": wallpaper_color(), "dim": cfg.wallpaper_dim.clamp(0.0, 0.9), "sig": "" });
    let Some(file) = wallpaper_file() else { return res };
    let Ok(meta) = std::fs::metadata(&file) else { return res };
    let mtime = meta.modified().ok().and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok()).map(|d| d.as_millis()).unwrap_or(0);
    let new_sig = format!("{}|{}|{}", file.display(), meta.len(), mtime);
    res["sig"] = json!(new_sig);
    if sig.as_deref() == Some(new_sig.as_str()) || meta.len() > WALLPAPER_MAX {
        return res;
    }
    if let Ok(bytes) = std::fs::read(&file) {
        res["url"] = json!(format!("data:{};base64,{}", image_mime(&bytes), base64(&bytes)));
    }
    res
}


// --- Tests : placement du mini-cœur (fonctions pures, sans fenêtre) ------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_du_fond_d_ecran() {
        assert_eq!(base64(b""), "");
        assert_eq!(base64(b"M"), "TQ==");
        assert_eq!(base64(b"Ma"), "TWE=");
        assert_eq!(base64(b"Man"), "TWFu");
        assert_eq!(image_mime(&[0x89, b'P', b'N', b'G']), "image/png");
        assert_eq!(image_mime(&[0xFF, 0xD8, 0xFF]), "image/jpeg");
    }

    fn r(left: i32, top: i32, right: i32, bottom: i32) -> RECT {
        RECT { left, top, right, bottom }
    }

    #[test]
    fn cards_converties_en_coordonnees_ecran() {
        // HUD en (1000, 200), échelle 1,5 : une card de 100×40 px CSS en (20, 10)
        let c = cards_to_screen(&[[20.0, 10.0, 100.0, 40.0]], (1000, 200), 1.5);
        assert_eq!(c.len(), 1);
        let (_, rc) = c[0];
        assert_eq!((rc.left, rc.top, rc.right, rc.bottom), (1030, 215, 1180, 275));
    }

    #[test]
    fn une_card_sous_le_mini_coeur_le_fait_partir() {
        let area = r(0, 0, 1920, 1080);
        let (side, pad) = (120, 20);
        let here = (900, 480);
        // Aucune application : la place actuelle est libre.
        assert!(spot_free(&[], side, pad, here));
        // Une card s'ouvre dessous : la place n'est plus libre, et la nouvelle ne touche pas la card.
        let card = vec![(-1isize, r(850, 450, 1150, 650))];
        assert!(!spot_free(&card, side, pad, here));
        let p = free_spot(area, side, pad, here, Some(here), &card);
        assert_ne!(p, here);
        assert!(spot_free(&card, side, pad, p), "nouvelle place {p:?} encore sur la card");
    }

    #[test]
    fn applications_et_cards_sont_evitees_ensemble() {
        let area = r(0, 0, 1920, 1080);
        let (side, pad) = (120, 20);
        let home = (100, 100);
        let apps = vec![(1isize, r(0, 0, 400, 1080))]; // une application occupe la gauche
        let cards = vec![(-1isize, r(400, 0, 700, 400))]; // une card juste à côté
        let all = with_cards(&apps, &cards);
        let p = free_spot(area, side, pad, home, None, &all);
        assert!(spot_free(&apps, side, pad, p) && spot_free(&cards, side, pad, p), "place {p:?} sur une application ou une card");
    }
}
