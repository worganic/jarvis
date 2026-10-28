//! Presse-papier et frappe clavier natifs (remplacent Get-Clipboard / Set-Clipboard / SendKeys de la v3).

use windows::Win32::Foundation::{HANDLE, HGLOBAL, HWND};
use windows::Win32::System::DataExchange::{CloseClipboard, EmptyClipboard, GetClipboardData, IsClipboardFormatAvailable, OpenClipboard, SetClipboardData};
use windows::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalSize, GlobalUnlock, GMEM_MOVEABLE};
use windows::Win32::UI::Input::KeyboardAndMouse::*;
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowThreadProcessId};

const CF_UNICODETEXT: u32 = 13;

fn open_clipboard() -> bool {
    for _ in 0..20 {
        if unsafe { OpenClipboard(Some(HWND::default())) }.is_ok() {
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(15));
    }
    false
}

/// Texte du presse-papier (None s'il n'y a pas de texte).
pub fn clipboard_get() -> Option<String> {
    unsafe {
        if IsClipboardFormatAvailable(CF_UNICODETEXT).is_err() || !open_clipboard() {
            return None;
        }
        let out = (|| {
            let h = GetClipboardData(CF_UNICODETEXT).ok()?;
            let g = HGLOBAL(h.0);
            let p = GlobalLock(g) as *const u16;
            if p.is_null() {
                return None;
            }
            let max = GlobalSize(g) / 2;
            let mut len = 0;
            while len < max && *p.add(len) != 0 {
                len += 1;
            }
            let s = String::from_utf16_lossy(std::slice::from_raw_parts(p, len));
            let _ = GlobalUnlock(g);
            Some(s)
        })();
        let _ = CloseClipboard();
        out
    }
}

/// Remplace le contenu du presse-papier par un texte.
pub fn clipboard_set(text: &str) -> Result<(), String> {
    let w: Vec<u16> = text.encode_utf16().chain(std::iter::once(0)).collect();
    unsafe {
        if !open_clipboard() {
            return Err("presse-papier occupé".into());
        }
        let r = (|| -> Result<(), String> {
            EmptyClipboard().map_err(|e| e.to_string())?;
            let g = GlobalAlloc(GMEM_MOVEABLE, w.len() * 2).map_err(|e| e.to_string())?;
            let p = GlobalLock(g) as *mut u16;
            if p.is_null() {
                return Err("mémoire indisponible".into());
            }
            std::ptr::copy_nonoverlapping(w.as_ptr(), p, w.len());
            let _ = GlobalUnlock(g);
            SetClipboardData(CF_UNICODETEXT, Some(HANDLE(g.0))).map_err(|e| e.to_string())?;
            Ok(())
        })();
        let _ = CloseClipboard();
        r
    }
}

fn key(vk: VIRTUAL_KEY, up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: vk, wScan: 0, dwFlags: if up { KEYEVENTF_KEYUP } else { KEYBD_EVENT_FLAGS(0) }, time: 0, dwExtraInfo: 0 } },
    }
}

fn unicode(c: u16, up: bool) -> INPUT {
    INPUT {
        r#type: INPUT_KEYBOARD,
        Anonymous: INPUT_0 { ki: KEYBDINPUT { wVk: VIRTUAL_KEY(0), wScan: c, dwFlags: if up { KEYEVENTF_UNICODE | KEYEVENTF_KEYUP } else { KEYEVENTF_UNICODE }, time: 0, dwExtraInfo: 0 } },
    }
}

fn send(inputs: &[INPUT]) {
    unsafe {
        SendInput(inputs, std::mem::size_of::<INPUT>() as i32);
    }
}

/// Raccourci : modificateurs maintenus + touche (ex. Ctrl+V).
pub fn chord(mods: &[VIRTUAL_KEY], vk: VIRTUAL_KEY) {
    let mut v = vec![];
    for m in mods {
        v.push(key(*m, false));
    }
    v.push(key(vk, false));
    v.push(key(vk, true));
    for m in mods.iter().rev() {
        v.push(key(*m, true));
    }
    send(&v);
}

pub fn ctrl(letter: char) {
    chord(&[VK_CONTROL], VIRTUAL_KEY(letter.to_ascii_uppercase() as u16));
}

fn named(name: &str) -> Option<VIRTUAL_KEY> {
    Some(match name.to_ascii_uppercase().as_str() {
        "ENTER" | "~" => VK_RETURN,
        "ESC" | "ESCAPE" => VK_ESCAPE,
        "TAB" => VK_TAB,
        "BACKSPACE" | "BS" | "BKSP" => VK_BACK,
        "DELETE" | "DEL" => VK_DELETE,
        "UP" => VK_UP,
        "DOWN" => VK_DOWN,
        "LEFT" => VK_LEFT,
        "RIGHT" => VK_RIGHT,
        "HOME" => VK_HOME,
        "END" => VK_END,
        "PGUP" => VK_PRIOR,
        "PGDN" => VK_NEXT,
        "SPACE" => VK_SPACE,
        "F1" => VK_F1,
        "F2" => VK_F2,
        "F3" => VK_F3,
        "F4" => VK_F4,
        "F5" => VK_F5,
        "F6" => VK_F6,
        "F7" => VK_F7,
        "F8" => VK_F8,
        "F9" => VK_F9,
        "F10" => VK_F10,
        "F11" => VK_F11,
        "F12" => VK_F12,
        _ => return None,
    })
}

/// Sous-ensemble de la syntaxe SendKeys : « ^c », « +{TAB} », « %{F4} », « {ENTER} », texte simple.
pub fn send_keys(spec: &str) {
    let chars: Vec<char> = spec.chars().collect();
    let mut i = 0;
    let mut mods: Vec<VIRTUAL_KEY> = vec![];
    while i < chars.len() {
        let c = chars[i];
        match c {
            '^' => {
                mods.push(VK_CONTROL);
                i += 1;
                continue;
            }
            '+' => {
                mods.push(VK_SHIFT);
                i += 1;
                continue;
            }
            '%' => {
                mods.push(VK_MENU);
                i += 1;
                continue;
            }
            '{' => {
                let end = chars[i + 1..].iter().position(|c| *c == '}').map(|p| p + i + 1);
                if let Some(end) = end {
                    let name: String = chars[i + 1..end].iter().collect();
                    if let Some(vk) = named(&name) {
                        chord(&mods, vk);
                    } else {
                        type_text(&name);
                    }
                    mods.clear();
                    i = end + 1;
                    continue;
                }
            }
            '~' => {
                chord(&mods, VK_RETURN);
                mods.clear();
                i += 1;
                continue;
            }
            _ => {}
        }
        if !mods.is_empty() && c.is_ascii_alphanumeric() {
            chord(&mods, VIRTUAL_KEY(c.to_ascii_uppercase() as u16));
        } else {
            type_text(&c.to_string());
        }
        mods.clear();
        i += 1;
    }
}

/// Tape un texte (caractères Unicode, sans passer par le presse-papier).
pub fn type_text(text: &str) {
    let mut v = vec![];
    for u in text.encode_utf16() {
        if u == '\n' as u16 {
            v.push(key(VK_RETURN, false));
            v.push(key(VK_RETURN, true));
        } else if u != '\r' as u16 {
            v.push(unicode(u, false));
            v.push(unicode(u, true));
        }
    }
    send(&v);
}

pub fn foreground_hwnd() -> i64 {
    unsafe { GetForegroundWindow().0 as i64 }
}

/// Nom (minuscules) du programme qui possède la fenêtre `hwnd`, vide si elle n'existe plus.
pub fn owner_name(hwnd: i64) -> String {
    use windows::Win32::Foundation::HWND;
    if hwnd == 0 {
        return String::new();
    }
    let mut pid = 0u32;
    unsafe { GetWindowThreadProcessId(HWND(hwnd as *mut _), Some(&mut pid)) };
    if pid == 0 { String::new() } else { crate::window::process_info(pid).0.to_lowercase() }
}

/// Prépare une saisie dans la fenêtre `hwnd` (un terminal). Ok(true) : elle a dû être ramenée devant ;
/// Ok(false) : elle y était déjà ; Err(programme) : une autre application de l'utilisateur est devant,
/// on n'y touche pas. Ramenée devant seulement si ce qui a pris le focus n'est pas un choix de
/// l'utilisateur : une fenêtre de Jarvis (HUD, widget, bureau), un focus perdu, ou une fenêtre du
/// même programme (menu, boîte de dialogue du terminal).
pub fn ensure_foreground(hwnd: i64) -> Result<bool, String> {
    use windows::Win32::Foundation::HWND;
    let fg = foreground_hwnd();
    if fg == hwnd {
        return Ok(false);
    }
    let pid_of = |h: i64| -> u32 {
        let mut pid = 0u32;
        if h != 0 {
            unsafe { GetWindowThreadProcessId(HWND(h as *mut _), Some(&mut pid)) };
        }
        pid
    };
    let (fg_pid, term_pid) = (pid_of(fg), pid_of(hwnd));
    let fg_name = if fg_pid != 0 { crate::window::process_info(fg_pid).0.to_lowercase() } else { String::new() };
    let ours = fg_name.is_empty() || fg_name.starts_with("jarvis") || fg_name.starts_with("msedgewebview2") || (fg_pid != 0 && fg_pid == term_pid);
    if !ours {
        return Err(fg_name);
    }
    for _ in 0..3 {
        crate::desktop::activate(HWND(hwnd as *mut _));
        std::thread::sleep(std::time::Duration::from_millis(120));
        if foreground_hwnd() == hwnd {
            return Ok(true);
        }
    }
    Err(if fg_name.is_empty() { "aucune fenêtre".into() } else { fg_name })
}

/// Colle un texte (presse-papier + Ctrl+V), puis restaure l'ancien presse-papier.
pub fn paste(text: &str) -> Result<(), String> {
    let old = clipboard_get();
    clipboard_set(text)?;
    std::thread::sleep(std::time::Duration::from_millis(80));
    ctrl('v');
    std::thread::sleep(std::time::Duration::from_millis(350));
    if let Some(old) = old.filter(|o| !o.is_empty()) {
        let _ = clipboard_set(&old);
    }
    Ok(())
}

/// Copie la sélection (ou tout si `whole`) et renvoie le texte.
pub fn copy_selection(whole: bool) -> String {
    let _ = clipboard_set(" ");
    if whole {
        ctrl('a');
        std::thread::sleep(std::time::Duration::from_millis(150));
    }
    ctrl('c');
    std::thread::sleep(std::time::Duration::from_millis(400));
    clipboard_get().unwrap_or_default()
}
