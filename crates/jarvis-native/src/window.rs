// Fenêtre au premier plan et informations de processus (tools/active-win.cs de la v3).

use std::collections::HashMap;
use std::sync::Mutex;

use serde_json::{json, Value};
use windows::core::PWSTR;
use windows::Win32::Foundation::{CloseHandle, HWND};
use windows::Win32::System::Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION};
use windows::Win32::UI::WindowsAndMessaging::{GetForegroundWindow, GetWindowTextLengthW, GetWindowTextW, GetWindowThreadProcessId};

/// Titre d'une fenêtre.
pub fn window_text(h: HWND) -> String {
    unsafe {
        let len = GetWindowTextLengthW(h);
        if len <= 0 {
            return String::new();
        }
        let mut buf = vec![0u16; (len as usize + 1).min(1024)];
        let n = GetWindowTextW(h, &mut buf);
        String::from_utf16_lossy(&buf[..n.max(0) as usize])
    }
}

/// Chemin complet de l'exécutable d'un processus (vide si inaccessible).
pub fn process_path(pid: u32) -> String {
    unsafe {
        let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return String::new() };
        let mut buf = vec![0u16; 1024];
        let mut size = buf.len() as u32;
        let ok = QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut size).is_ok();
        let _ = CloseHandle(h);
        if ok {
            String::from_utf16_lossy(&buf[..size as usize])
        } else {
            String::new()
        }
    }
}

/// Nom du processus sans extension, d'après son chemin.
pub fn name_of_path(path: &str) -> String {
    std::path::Path::new(path).file_stem().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default()
}

static PROC_CACHE: Mutex<Option<HashMap<u32, (String, String)>>> = Mutex::new(None);

/// (nom, chemin) d'un processus, mis en cache (comme la v3 : vidé au-delà de 2000 entrées).
pub fn process_info(pid: u32) -> (String, String) {
    {
        let cache = PROC_CACHE.lock().unwrap();
        if let Some(hit) = cache.as_ref().and_then(|c| c.get(&pid)) {
            return hit.clone();
        }
    }
    let path = process_path(pid);
    let name = if path.is_empty() { process_name_fallback(pid) } else { name_of_path(&path) };
    let mut cache = PROC_CACHE.lock().unwrap();
    let c = cache.get_or_insert_with(HashMap::new);
    if c.len() > 2000 {
        c.clear();
    }
    c.insert(pid, (name.clone(), path.clone()));
    (name, path)
}

/// Nom d'un processus protégé (sans chemin lisible) par l'instantané des processus.
pub fn process_name_fallback(pid: u32) -> String {
    use windows::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS};
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return String::new() };
        let mut e = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut out = String::new();
        if Process32FirstW(snap, &mut e).is_ok() {
            loop {
                if e.th32ProcessID == pid {
                    let end = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(e.szExeFile.len());
                    out = name_of_path(&String::from_utf16_lossy(&e.szExeFile[..end]));
                    break;
                }
                if Process32NextW(snap, &mut e).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snap);
        out
    }
}

/// Tous les processus : (pid, pid parent, nom de l'exécutable).
pub fn processes() -> Vec<(u32, u32, String)> {
    use windows::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS};
    let mut out = vec![];
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return out };
        let mut e = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        if Process32FirstW(snap, &mut e).is_ok() {
            loop {
                let end = e.szExeFile.iter().position(|&c| c == 0).unwrap_or(e.szExeFile.len());
                out.push((e.th32ProcessID, e.th32ParentProcessID, String::from_utf16_lossy(&e.szExeFile[..end])));
                if Process32NextW(snap, &mut e).is_err() {
                    break;
                }
            }
        }
        let _ = CloseHandle(snap);
    }
    out
}

/// Un processus de ce nom (« Spotify.exe ») tourne-t-il ?
pub fn is_running(exe: &str) -> bool {
    let want = exe.to_lowercase();
    processes().iter().any(|(_, _, n)| n.to_lowercase() == want)
}

/// Fenêtre au premier plan : { process, title, pid }.
pub fn foreground() -> Value {
    unsafe {
        let h = GetForegroundWindow();
        if h.0.is_null() {
            return json!({ "process": "", "title": "", "pid": 0 });
        }
        let mut pid = 0u32;
        GetWindowThreadProcessId(h, Some(&mut pid));
        let title = window_text(h).replace('\r', "").replace('\n', " ");
        let proc_name = if pid > 0 { process_info(pid).0 } else { String::new() };
        json!({ "process": proc_name, "title": title, "pid": pid })
    }
}

unsafe extern "system" fn on_foreground(_h: windows::Win32::UI::Accessibility::HWINEVENTHOOK, ev: u32, hwnd: HWND, _o: i32, _c: i32, _t: u32, _m: u32) {
    use std::io::Write;
    if ev == 0x0003 && !hwnd.0.is_null() {
        let mut out = std::io::stdout();
        // Noyau parti (tuyau fermé) : on s'arrête. Un veilleur orphelin gardait jarvis-core.exe verrouillé
        // et la relance ne pouvait plus le recompiler (21/10/2026).
        if writeln!(out, "{}", foreground()).and_then(|_| out.flush()).is_err() {
            std::process::exit(0);
        }
    }
}

/// `active-win --watch` : une ligne JSON à chaque changement de fenêtre au premier plan.
pub fn watch_foreground() -> i32 {
    use std::io::Write;
    use windows::Win32::UI::Accessibility::SetWinEventHook;
    use windows::Win32::UI::WindowsAndMessaging::{GetMessageW, MSG};
    let mut out = std::io::stdout();
    let _ = writeln!(out, "{}", foreground());
    let _ = out.flush();
    unsafe {
        let _ = SetWinEventHook(0x0003, 0x0003, None, Some(on_foreground), 0, 0, 0);
        let mut msg = MSG::default();
        while GetMessageW(&mut msg, None, 0, 0).as_bool() {}
    }
    0
}

/// Ligne de commande d'un processus (NtQueryInformationProcess, classe 60), ou chaîne vide.
pub fn command_line(pid: u32) -> String {
    use windows::Wdk::System::Threading::{NtQueryInformationProcess, PROCESSINFOCLASS};
    use windows::Win32::System::Threading::{OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION};
    unsafe {
        let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid) else { return String::new() };
        let mut len = 0u32;
        let _ = NtQueryInformationProcess(h, PROCESSINFOCLASS(60), std::ptr::null_mut(), 0, &mut len);
        let mut out = String::new();
        if len > 0 && len < 1 << 20 {
            let mut buf = vec![0u8; len as usize];
            if NtQueryInformationProcess(h, PROCESSINFOCLASS(60), buf.as_mut_ptr() as *mut _, len, &mut len).is_ok() {
                // UNICODE_STRING { Length: u16, MaximumLength: u16, Buffer: *u16 } suivi des caractères.
                let us = &*(buf.as_ptr() as *const windows::Win32::Foundation::UNICODE_STRING);
                if !us.Buffer.is_null() {
                    out = String::from_utf16_lossy(std::slice::from_raw_parts(us.Buffer.0, us.Length as usize / 2));
                }
            }
        }
        let _ = CloseHandle(h);
        out
    }
}

/// Mémoire (working set, octets) et temps CPU cumulé (noyau + utilisateur, en unités de 100 ns).
pub fn process_stats(pid: u32) -> Option<(u64, u64)> {
    use windows::Win32::Foundation::FILETIME;
    use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    use windows::Win32::System::Threading::{GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION, PROCESS_VM_READ};
    unsafe {
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | PROCESS_VM_READ, false, pid)
            .or_else(|_| OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, pid))
            .ok()?;
        let mut mem = PROCESS_MEMORY_COUNTERS { cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32, ..Default::default() };
        let ws = if GetProcessMemoryInfo(h, &mut mem, mem.cb).is_ok() { mem.WorkingSetSize as u64 } else { 0 };
        let (mut c, mut e, mut k, mut u) = (FILETIME::default(), FILETIME::default(), FILETIME::default(), FILETIME::default());
        let cpu = if GetProcessTimes(h, &mut c, &mut e, &mut k, &mut u).is_ok() {
            let t = |f: FILETIME| ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64;
            t(k) + t(u)
        } else {
            0
        };
        let _ = CloseHandle(h);
        Some((ws, cpu))
    }
}
