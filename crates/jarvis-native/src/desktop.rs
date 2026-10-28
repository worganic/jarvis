// Veille du bureau (tools/desktop-watch.cs de la v3) : fenêtres d'application, écrans, applis
// ouvertes sans fenêtre, et actions sur les fenêtres. Processus résident (`jarvis-core desktop-watch`),
// même dialogue en lignes JSON que la v3 sur l'entrée et la sortie standard :
//   sortie : {"t":"state", monitors, windows, fg, background} à chaque changement (≤ 1 / 300 ms)
//            {"t":"result", id, ok, error?, …} réponse à une commande ; {"t":"log", message}
//   entrée : {"id":1,"cmd":"snapshot"} · activate · minimize · maximize · restore · close · move ·
//            center · snap · minimizeAll · uiFind · uiInvoke · uiSetValue · capture · click
// Événements SetWinEventHook : aucune scrutation, CPU quasi nul.

use std::collections::HashSet;
use std::io::{BufRead, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::{Duration, Instant};

use serde_json::{json, Map, Value};
use windows::Win32::Foundation::{HWND, LPARAM, POINT, RECT, WPARAM};
use windows::Win32::Graphics::Dwm::{DwmGetWindowAttribute, DWMWA_CLOAKED, DWMWA_EXTENDED_FRAME_BOUNDS};
use windows::Win32::Graphics::Gdi::{EnumDisplayMonitors, GetMonitorInfoW, MonitorFromRect, HDC, HMONITOR, MONITORINFO, MONITORINFOEXW, MONITOR_DEFAULTTONEAREST};
use windows::Win32::System::RemoteDesktop::ProcessIdToSessionId;
use windows::Win32::System::Threading::{AttachThreadInput, GetCurrentThreadId};
use windows::Win32::UI::Accessibility::{SetWinEventHook, HWINEVENTHOOK};
use windows::Win32::UI::HiDpi::{GetDpiForMonitor, SetProcessDpiAwarenessContext, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, MDT_EFFECTIVE_DPI};
use windows::Win32::UI::Input::KeyboardAndMouse::{keybd_event, mouse_event, SetActiveWindow, SetFocus, KEYEVENTF_KEYUP, MOUSEEVENTF_LEFTDOWN, MOUSEEVENTF_LEFTUP};
use windows::Win32::UI::WindowsAndMessaging::*;

static DIRTY: AtomicBool = AtomicBool::new(true);
static BG_DIRTY: AtomicBool = AtomicBool::new(true);
static OUT: Mutex<()> = Mutex::new(());
static LAST_SENT: Mutex<String> = Mutex::new(String::new());
static BACKGROUND: Mutex<(Vec<Value>, Option<Instant>)> = Mutex::new((Vec::new(), None));

fn self_pid() -> u32 {
    std::process::id()
}

fn send(v: &Value, force: bool) {
    let line = v.to_string();
    let _g = OUT.lock().unwrap();
    let mut last = LAST_SENT.lock().unwrap();
    if !force && line == *last {
        return;
    }
    if v["t"] == "state" {
        *last = line.clone();
    }
    let mut out = std::io::stdout();
    let _ = writeln!(out, "{line}");
    let _ = out.flush();
}

pub fn log(message: &str) {
    send(&json!({ "t": "log", "message": message }), true);
}

pub fn mark_dirty() {
    DIRTY.store(true, Ordering::SeqCst);
}

// --- Écrans ---------------------------------------------------------------------------------------

pub struct Monitor {
    pub handle: HMONITOR,
    pub n: i64,
    pub device: String,
    pub primary: bool,
    pub rect: RECT,
    pub work: RECT,
    pub dpi: u32,
}

unsafe extern "system" fn enum_mon(h: HMONITOR, _hdc: HDC, _r: *mut RECT, data: LPARAM) -> windows_core::BOOL {
    let list = &mut *(data.0 as *mut Vec<Monitor>);
    let mut mi = MONITORINFOEXW::default();
    mi.monitorInfo.cbSize = std::mem::size_of::<MONITORINFOEXW>() as u32;
    if !GetMonitorInfoW(h, &mut mi as *mut MONITORINFOEXW as *mut MONITORINFO).as_bool() {
        return true.into();
    }
    let (mut dx, mut dy) = (96u32, 96u32);
    let _ = GetDpiForMonitor(h, MDT_EFFECTIVE_DPI, &mut dx, &mut dy);
    let end = mi.szDevice.iter().position(|&c| c == 0).unwrap_or(mi.szDevice.len());
    let device = String::from_utf16_lossy(&mi.szDevice[..end]);
    let digits: String = device.chars().filter(|c| c.is_ascii_digit()).collect();
    list.push(Monitor {
        handle: h,
        n: digits.parse().unwrap_or(0),
        device,
        primary: mi.monitorInfo.dwFlags & 1 != 0,
        rect: mi.monitorInfo.rcMonitor,
        work: mi.monitorInfo.rcWork,
        dpi: dx,
    });
    true.into()
}

pub fn monitors() -> Vec<Monitor> {
    let mut list: Vec<Monitor> = vec![];
    unsafe {
        let _ = EnumDisplayMonitors(None, None, Some(enum_mon), LPARAM(&mut list as *mut _ as isize));
    }
    // Numérotation de Windows (DISPLAY1, DISPLAY2…) ; à défaut, de gauche à droite.
    let distinct: HashSet<i64> = list.iter().map(|m| m.n).collect();
    if list.iter().any(|m| m.n == 0) || distinct.len() != list.len() {
        list.sort_by_key(|m| m.rect.left);
        for (i, m) in list.iter_mut().enumerate() {
            m.n = i as i64 + 1;
        }
    }
    list.sort_by_key(|m| m.n);
    list
}

fn rect_obj(r: &RECT) -> Value {
    json!({ "x": r.left, "y": r.top, "w": r.right - r.left, "h": r.bottom - r.top })
}

fn monitor_of<'a>(r: &RECT, mons: &'a [Monitor]) -> Option<&'a Monitor> {
    let hm = unsafe { MonitorFromRect(r, MONITOR_DEFAULTTONEAREST) };
    mons.iter().find(|m| m.handle == hm).or_else(|| mons.iter().find(|m| m.primary)).or_else(|| mons.first())
}

// --- Fenêtres -------------------------------------------------------------------------------------

pub fn class_name(h: HWND) -> String {
    let mut buf = [0u16; 256];
    let n = unsafe { GetClassNameW(h, &mut buf) };
    String::from_utf16_lossy(&buf[..n.max(0) as usize])
}

pub fn is_app_window(h: HWND) -> bool {
    unsafe {
        if !IsWindowVisible(h).as_bool() {
            return false;
        }
        if GetWindow(h, GW_OWNER).map(|o| !o.0.is_null()).unwrap_or(false) {
            return false;
        }
        let ex = GetWindowLongPtrW(h, GWL_EXSTYLE) as u32;
        let app = ex & WS_EX_APPWINDOW.0 != 0;
        if ex & WS_EX_TOOLWINDOW.0 != 0 && !app {
            return false;
        }
        if ex & WS_EX_NOACTIVATE.0 != 0 && !app {
            return false;
        }
        let mut cloaked = 0i32;
        if DwmGetWindowAttribute(h, DWMWA_CLOAKED, &mut cloaked as *mut _ as *mut _, 4).is_ok() && cloaked != 0 {
            return false;
        }
        if GetWindowTextLengthW(h) == 0 {
            return false;
        }
        !matches!(class_name(h).as_str(), "Progman" | "WorkerW" | "Shell_TrayWnd" | "Shell_SecondaryTrayWnd")
    }
}

struct ChildPid {
    pid: u32,
    found: u32,
}

unsafe extern "system" fn enum_child(c: HWND, l: LPARAM) -> windows_core::BOOL {
    let st = &mut *(l.0 as *mut ChildPid);
    let mut cp = 0u32;
    GetWindowThreadProcessId(c, Some(&mut cp));
    if cp != st.pid && cp != 0 {
        st.found = cp;
        return false.into();
    }
    true.into()
}

// Application réelle d'une fenêtre UWP (hébergée par ApplicationFrameHost).
fn real_pid(h: HWND, pid: u32, name: &str) -> u32 {
    if !name.eq_ignore_ascii_case("ApplicationFrameHost") {
        return pid;
    }
    let mut st = ChildPid { pid, found: pid };
    unsafe {
        let _ = EnumChildWindows(Some(h), Some(enum_child), LPARAM(&mut st as *mut _ as isize));
    }
    st.found
}

unsafe extern "system" fn enum_top(h: HWND, l: LPARAM) -> windows_core::BOOL {
    let list = &mut *(l.0 as *mut Vec<HWND>);
    list.push(h);
    true.into()
}

/// Fenêtres de premier niveau, dans l'ordre Z (de la plus haute à la plus basse).
pub fn top_windows() -> Vec<HWND> {
    let mut list: Vec<HWND> = vec![];
    unsafe {
        let _ = EnumWindows(Some(enum_top), LPARAM(&mut list as *mut _ as isize));
    }
    list
}

fn placement(h: HWND) -> WINDOWPLACEMENT {
    let mut wp = WINDOWPLACEMENT { length: std::mem::size_of::<WINDOWPLACEMENT>() as u32, ..Default::default() };
    unsafe {
        let _ = GetWindowPlacement(h, &mut wp);
    }
    wp
}

pub fn snapshot() -> Value {
    let mons = monitors();
    let mut windows: Vec<Value> = vec![];
    let mut fg = unsafe { GetForegroundWindow() };
    let mut z = 0i64;
    for h in top_windows() {
        if !is_app_window(h) {
            continue;
        }
        let mut pid = 0u32;
        unsafe { GetWindowThreadProcessId(h, Some(&mut pid)) };
        if pid == self_pid() {
            continue;
        }
        let mut info = crate::window::process_info(pid);
        let real = real_pid(h, pid, &info.0);
        if real != pid {
            info = crate::window::process_info(real);
        }
        let (min, max) = unsafe { (IsIconic(h).as_bool(), IsZoomed(h).as_bool()) };
        let wp = placement(h);
        let mut r = RECT::default();
        if min {
            r = wp.rcNormalPosition;
        } else {
            unsafe {
                let _ = GetWindowRect(h, &mut r);
            }
        }
        let mon = monitor_of(&r, &mons);
        if fg.0.is_null() && !min && z == 0 {
            fg = h;
        }
        windows.push(json!({
            "hwnd": h.0 as i64, "pid": real, "process": info.0, "path": info.1,
            "title": crate::window::window_text(h), "cls": class_name(h),
            "x": r.left, "y": r.top, "w": r.right - r.left, "h": r.bottom - r.top,
            "monitor": mon.map(|m| m.n).unwrap_or(0), "min": min, "max": max, "fg": h == fg, "z": z
        }));
        z += 1;
    }
    let bg = BACKGROUND.lock().unwrap().0.clone();
    let pids: HashSet<i64> = windows.iter().map(|w| w["pid"].as_i64().unwrap_or(0)).collect();
    let names: HashSet<String> = windows.iter().map(|w| w["process"].as_str().unwrap_or("").to_lowercase()).collect();
    let background: Vec<Value> = bg
        .into_iter()
        .filter(|b| !pids.contains(&b["pid"].as_i64().unwrap_or(0)) && !names.contains(&b["process"].as_str().unwrap_or("").to_lowercase()))
        .collect();
    json!({
        "t": "state",
        "monitors": mons.iter().map(|m| json!({
            "n": m.n, "device": m.device, "primary": m.primary, "scale": ((m.dpi as f64 / 96.0) * 100.0).round() / 100.0,
            "rect": rect_obj(&m.rect), "work": rect_obj(&m.work)
        })).collect::<Vec<_>>(),
        "windows": windows,
        "fg": fg.0 as i64,
        "background": background
    })
}

// Applications de la session ouvertes sans fenêtre visible (hors Windows lui-même).
fn refresh_background() {
    let mut list = vec![];
    let mut seen = HashSet::new();
    let mut mine = 0u32;
    unsafe {
        let _ = ProcessIdToSessionId(self_pid(), &mut mine);
    }
    let windir = std::env::var("WINDIR").unwrap_or_else(|_| "C:\\Windows".into()).to_lowercase();
    for (pid, _, _) in crate::window::processes() {
        let mut session = 0u32;
        if unsafe { ProcessIdToSessionId(pid, &mut session) }.is_err() || session != mine {
            continue;
        }
        let info = crate::window::process_info(pid);
        if info.1.is_empty() || info.1.to_lowercase().starts_with(&windir) {
            continue;
        }
        if !seen.insert(info.0.to_lowercase()) {
            continue;
        }
        list.push(json!({ "pid": pid, "process": info.0, "path": info.1 }));
    }
    *BACKGROUND.lock().unwrap() = (list, Some(Instant::now()));
}

// --- Actions --------------------------------------------------------------------------------------

fn hwnd_of(cmd: &Value) -> Result<HWND, String> {
    let v = cmd.get("hwnd").ok_or("fenêtre non précisée")?;
    let n = v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)).or_else(|| v.as_str().and_then(|s| s.parse().ok())).ok_or("fenêtre non précisée")?;
    let h = HWND(n as *mut _);
    if !unsafe { IsWindow(Some(h)).as_bool() } {
        return Err("cette fenêtre n'existe plus".into());
    }
    Ok(h)
}

fn ok(ok: bool, error: Option<&str>) -> Map<String, Value> {
    let mut m = Map::new();
    m.insert("ok".into(), json!(ok));
    if let Some(e) = error {
        m.insert("error".into(), json!(e));
    }
    m
}

/// Premier plan absolu : la fenêtre passe devant toutes les autres et reçoit le focus.
pub fn activate(h: HWND) -> bool {
    unsafe {
        if h.0.is_null() || !IsWindow(Some(h)).as_bool() {
            return false;
        }
        let fg = GetForegroundWindow();
        let fg_thread = if fg.0.is_null() { 0 } else { GetWindowThreadProcessId(fg, None) };
        let cur = GetCurrentThreadId();
        let target = GetWindowThreadProcessId(h, None);
        let _ = SystemParametersInfoW(SPI_SETFOREGROUNDLOCKTIMEOUT, 0, None, SYSTEM_PARAMETERS_INFO_UPDATE_FLAGS(0));
        let _ = AllowSetForegroundWindow(u32::MAX);
        let _ = LockSetForegroundWindow(LSFW_UNLOCK);
        let att_fg = fg_thread != 0 && fg_thread != cur && AttachThreadInput(cur, fg_thread, true).as_bool();
        let att_t = target != 0 && cur != target && AttachThreadInput(cur, target, true).as_bool();
        keybd_event(0x12, 0x38, Default::default(), 0);
        keybd_event(0x12, 0x38, KEYEVENTF_KEYUP, 0);
        if IsIconic(h).as_bool() {
            let _ = ShowWindow(h, SW_RESTORE);
        } else if !IsZoomed(h).as_bool() {
            let _ = ShowWindow(h, SW_SHOW);
        }
        let flags = SWP_NOSIZE | SWP_NOMOVE | SWP_SHOWWINDOW;
        let _ = SetWindowPos(h, Some(HWND_TOPMOST), 0, 0, 0, 0, flags);
        std::thread::sleep(Duration::from_millis(20));
        let _ = SetWindowPos(h, Some(HWND_NOTOPMOST), 0, 0, 0, 0, flags);
        let _ = SetWindowPos(h, Some(HWND_TOP), 0, 0, 0, 0, flags);
        let _ = BringWindowToTop(h);
        let ok = SetForegroundWindow(h).as_bool();
        let _ = SetActiveWindow(h);
        let _ = SetFocus(Some(h));
        SwitchToThisWindow(h, true);
        if att_t {
            let _ = AttachThreadInput(cur, target, false);
        }
        if att_fg {
            let _ = AttachThreadInput(cur, fg_thread, false);
        }
        mark_dirty();
        std::thread::sleep(Duration::from_millis(60));
        ok || GetForegroundWindow() == h || (!IsIconic(h).as_bool() && IsWindowVisible(h).as_bool())
    }
}

fn move_to_monitor(h: HWND, n: i64) -> Map<String, Value> {
    let mons = monitors();
    let Some(target) = mons.iter().find(|m| m.n == n) else { return ok(false, Some(&format!("écran {n} introuvable"))) };
    unsafe {
        let (was_max, was_min) = (IsZoomed(h).as_bool(), IsIconic(h).as_bool());
        let wp = placement(h);
        let src = monitor_of(&wp.rcNormalPosition, &mons).unwrap_or(target);
        if src.n == target.n && !was_min {
            activate(h);
            return ok(true, None);
        }
        let (r, sw, tw) = (wp.rcNormalPosition, src.work, target.work);
        let (rw, rh) = (r.right - r.left, r.bottom - r.top);
        let w = rw.min(tw.right - tw.left);
        let hh = rh.min(tw.bottom - tw.top);
        let fx = if sw.right - sw.left - rw > 0 { (r.left - sw.left) as f64 / (sw.right - sw.left - rw) as f64 } else { 0.0 }.clamp(0.0, 1.0);
        let fy = if sw.bottom - sw.top - rh > 0 { (r.top - sw.top) as f64 / (sw.bottom - sw.top - rh) as f64 } else { 0.0 }.clamp(0.0, 1.0);
        let x = tw.left + (fx * ((tw.right - tw.left) - w).max(0) as f64).round() as i32;
        let y = tw.top + (fy * ((tw.bottom - tw.top) - hh).max(0) as f64).round() as i32;
        if was_max || was_min {
            let _ = ShowWindow(h, SW_RESTORE);
        }
        let _ = SetWindowPos(h, None, x, y, w, hh, SWP_NOZORDER | SWP_SHOWWINDOW);
        if was_max {
            let _ = ShowWindow(h, SW_MAXIMIZE);
        }
    }
    activate(h);
    mark_dirty();
    ok(true, None)
}

/// Remet une fenêtre à une place enregistrée (emplacements de la card Bureau) : position et taille en
/// coordonnées d'écran, puis agrandie si elle l'était (sur l'écran de cette place).
fn place(h: HWND, (x, y, w, hh): (i32, i32, i32, i32), max: bool, focus: bool) -> Map<String, Value> {
    if w <= 0 || hh <= 0 {
        return ok(false, Some("place enregistrée invalide"));
    }
    unsafe {
        if IsZoomed(h).as_bool() || IsIconic(h).as_bool() {
            let _ = ShowWindow(h, SW_RESTORE);
        }
        let _ = SetWindowPos(h, None, x, y, w, hh, SWP_NOZORDER | SWP_NOACTIVATE | SWP_SHOWWINDOW);
        if max {
            let _ = ShowWindow(h, SW_MAXIMIZE);
        }
    }
    if focus {
        activate(h);
    }
    mark_dirty();
    ok(true, None)
}

fn center(h: HWND, n: i64) -> Map<String, Value> {
    let mons = monitors();
    let Some(target) = mons.iter().find(|m| m.n == n) else { return ok(false, Some(&format!("écran {n} introuvable"))) };
    unsafe {
        if IsZoomed(h).as_bool() || IsIconic(h).as_bool() {
            let _ = ShowWindow(h, SW_RESTORE);
        }
        let wp = placement(h);
        let (r, tw) = (wp.rcNormalPosition, target.work);
        let (aw, ah) = (tw.right - tw.left, tw.bottom - tw.top);
        let mut w = r.right - r.left;
        let mut hh = r.bottom - r.top;
        if w <= 0 || w > aw {
            w = (aw as f64 * 0.8) as i32;
        }
        if hh <= 0 || hh > ah {
            hh = (ah as f64 * 0.8) as i32;
        }
        let _ = SetWindowPos(h, None, tw.left + (aw - w) / 2, tw.top + (ah - hh) / 2, w, hh, SWP_NOZORDER | SWP_SHOWWINDOW);
    }
    activate(h);
    mark_dirty();
    ok(true, None)
}

fn snap(h: HWND, side: &str) -> Map<String, Value> {
    let mons = monitors();
    unsafe {
        let mut r = RECT::default();
        let _ = GetWindowRect(h, &mut r);
        let Some(mon) = monitor_of(&r, &mons) else { return ok(false, Some("écran introuvable")) };
        let w = mon.work;
        let half = (w.right - w.left) / 2;
        if IsZoomed(h).as_bool() || IsIconic(h).as_bool() {
            let _ = ShowWindow(h, SW_RESTORE);
        }
        let x = if side == "right" { w.left + half } else { w.left };
        let _ = SetWindowPos(h, None, x, w.top, half, w.bottom - w.top, SWP_NOZORDER);
    }
    mark_dirty();
    ok(true, None)
}

fn minimize_all(monitor: i64) -> Map<String, Value> {
    let mons = monitors();
    let mut count = 0;
    for h in top_windows() {
        unsafe {
            if !is_app_window(h) || IsIconic(h).as_bool() {
                continue;
            }
            let mut pid = 0u32;
            GetWindowThreadProcessId(h, Some(&mut pid));
            if pid == self_pid() {
                continue;
            }
            let mut r = RECT::default();
            let _ = GetWindowRect(h, &mut r);
            let mon = monitor_of(&r, &mons);
            if monitor != 0 && mon.map(|m| m.n != monitor).unwrap_or(true) {
                continue;
            }
            let _ = ShowWindowAsync(h, SW_SHOWMINNOACTIVE);
            count += 1;
        }
    }
    mark_dirty();
    let mut m = ok(true, None);
    m.insert("count".into(), json!(count));
    m
}

/// Clic gauche aux coordonnées de l'écran.
pub fn click_at(x: i32, y: i32) -> Map<String, Value> {
    unsafe {
        let _ = SetCursorPos(x, y);
        std::thread::sleep(Duration::from_millis(40));
        mouse_event(MOUSEEVENTF_LEFTDOWN, 0, 0, 0, 0);
        mouse_event(MOUSEEVENTF_LEFTUP, 0, 0, 0, 0);
    }
    mark_dirty();
    ok(true, None)
}

// Capture de la partie visible d'une fenêtre (sans l'ombre), en vrais pixels.
fn capture(h: HWND, file: &str) -> Map<String, Value> {
    let mut r = RECT::default();
    unsafe {
        if DwmGetWindowAttribute(h, DWMWA_EXTENDED_FRAME_BOUNDS, &mut r as *mut _ as *mut _, std::mem::size_of::<RECT>() as u32).is_err() {
            let _ = GetWindowRect(h, &mut r);
        }
    }
    let (w, hh) = (r.right - r.left, r.bottom - r.top);
    if w <= 0 || hh <= 0 {
        return ok(false, Some("fenêtre invisible"));
    }
    match crate::screenshot::capture_rect(r.left, r.top, w, hh).and_then(|img| crate::screenshot::save_image(&img, std::path::Path::new(file), "png", 90)) {
        Ok(()) => {
            let mut m = ok(true, None);
            m.insert("x".into(), json!(r.left));
            m.insert("y".into(), json!(r.top));
            m.insert("w".into(), json!(w));
            m.insert("h".into(), json!(hh));
            m.insert("file".into(), json!(file));
            m
        }
        Err(e) => ok(false, Some(&e)),
    }
}

fn int(cmd: &Value, key: &str, def: i64) -> i64 {
    match cmd.get(key) {
        Some(v) => v.as_i64().or_else(|| v.as_f64().map(|f| f as i64)).or_else(|| v.as_str().and_then(|s| s.trim().parse().ok())).unwrap_or(def),
        None => def,
    }
}

fn string(cmd: &Value, key: &str) -> String {
    match cmd.get(key) {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Null) | None => String::new(),
        Some(v) => v.to_string(),
    }
}

pub fn execute(cmd: &Value) -> Result<Map<String, Value>, String> {
    let name = string(cmd, "cmd");
    Ok(match name.as_str() {
        "snapshot" => {
            if BACKGROUND.lock().unwrap().1.is_none() {
                refresh_background();
            }
            let s = snapshot();
            LAST_SENT.lock().unwrap().clear();
            mark_dirty();
            let mut m = ok(true, None);
            m.insert("state".into(), s);
            m
        }
        "activate" => ok(activate(hwnd_of(cmd)?), None),
        "minimize" => {
            unsafe {
                let _ = ShowWindowAsync(hwnd_of(cmd)?, SW_MINIMIZE);
            }
            mark_dirty();
            ok(true, None)
        }
        "maximize" => {
            let h = hwnd_of(cmd)?;
            unsafe {
                if IsIconic(h).as_bool() {
                    let _ = ShowWindow(h, SW_RESTORE);
                }
                let _ = ShowWindow(h, SW_MAXIMIZE);
            }
            activate(h);
            mark_dirty();
            ok(true, None)
        }
        "restore" => {
            let h = hwnd_of(cmd)?;
            unsafe {
                let _ = ShowWindow(h, SW_RESTORE);
            }
            activate(h);
            mark_dirty();
            ok(true, None)
        }
        "close" => {
            unsafe {
                let _ = PostMessageW(Some(hwnd_of(cmd)?), WM_CLOSE, WPARAM(0), LPARAM(0));
            }
            mark_dirty();
            ok(true, None)
        }
        "move" => move_to_monitor(hwnd_of(cmd)?, int(cmd, "monitor", 0)),
        "center" => center(hwnd_of(cmd)?, int(cmd, "monitor", 0)),
        "snap" => snap(hwnd_of(cmd)?, &string(cmd, "side")),
        "place" => place(hwnd_of(cmd)?, (int(cmd, "x", 0) as i32, int(cmd, "y", 0) as i32, int(cmd, "w", 0) as i32, int(cmd, "h", 0) as i32), cmd.get("max").and_then(|v| v.as_bool()).unwrap_or(false), cmd.get("activate").and_then(|v| v.as_bool()).unwrap_or(false)),
        "minimizeAll" => minimize_all(int(cmd, "monitor", 0)),
        "uiFind" => crate::uia::ui_find(hwnd_of(cmd)?, int(cmd, "limit", 150) as usize),
        "uiInvoke" => crate::uia::ui_invoke(hwnd_of(cmd)?, &string(cmd, "name"), int(cmd, "index", -1)),
        "uiSetValue" => crate::uia::ui_set_value(hwnd_of(cmd)?, &string(cmd, "field"), &string(cmd, "value")),
        "capture" => capture(hwnd_of(cmd)?, &string(cmd, "file")),
        "click" => click_at(int(cmd, "x", 0) as i32, int(cmd, "y", 0) as i32),
        _ => ok(false, Some(&format!("commande inconnue : {name}"))),
    })
}

unsafe extern "system" fn on_win_event(_hook: HWINEVENTHOOK, ev: u32, hwnd: HWND, id_object: i32, id_child: i32, _thread: u32, _time: u32) {
    if ev == 0x0003 {
        mark_dirty();
        return;
    }
    if id_object != 0 || id_child != 0 || hwnd.0.is_null() {
        return;
    }
    if GetAncestor(hwnd, GA_ROOT) != hwnd {
        return;
    }
    mark_dirty();
    if ev == 0x8000 || ev == 0x8001 {
        BG_DIRTY.store(true, Ordering::SeqCst);
    }
}

/// Boucle résidente (`jarvis-core desktop-watch`).
pub fn serve() -> i32 {
    unsafe {
        let _ = SetProcessDpiAwarenessContext(DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2);
    }
    // Commandes : un fil dédié ; fin de l'entrée = fin du noyau → arrêt.
    std::thread::Builder::new()
        .name("commandes".into())
        .spawn(|| {
            crate::com_init();
            let stdin = std::io::stdin();
            for line in stdin.lock().lines().map_while(Result::ok) {
                if line.trim().is_empty() {
                    continue;
                }
                let (id, res) = match serde_json::from_str::<Value>(&line) {
                    Ok(cmd) => {
                        let id = cmd.get("id").cloned().unwrap_or(Value::Null);
                        (id, execute(&cmd))
                    }
                    Err(e) => (Value::Null, Err(e.to_string())),
                };
                let mut m = match res {
                    Ok(m) => m,
                    Err(e) => ok(false, Some(&e)),
                };
                m.insert("t".into(), json!("result"));
                m.insert("id".into(), id);
                send(&Value::Object(m), true);
            }
            std::process::exit(0);
        })
        .ok();
    // Émission regroupée : au plus un instantané toutes les 300 ms, seulement s'il a changé.
    std::thread::Builder::new()
        .name("emission".into())
        .spawn(|| loop {
            std::thread::sleep(Duration::from_millis(300));
            let stale = BACKGROUND.lock().unwrap().1.map(|t| t.elapsed() > Duration::from_secs(8)).unwrap_or(true);
            if BG_DIRTY.swap(false, Ordering::SeqCst) || stale {
                mark_dirty();
                refresh_background();
            }
            if !DIRTY.swap(false, Ordering::SeqCst) {
                continue;
            }
            send(&snapshot(), false);
        })
        .ok();
    unsafe {
        for (a, b) in [(0x0003u32, 0x0003u32), (0x0016, 0x0017), (0x8000, 0x8003), (0x800B, 0x800C)] {
            let _ = SetWinEventHook(a, b, None, Some(on_win_event), 0, 0, 0);
        }
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {}
    }
    0
}

#[allow(dead_code)]
fn _unused(_: POINT) {}
