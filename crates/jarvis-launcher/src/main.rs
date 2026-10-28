// Lanceur de Jarvis v4 : « Jarvis v4.exe », posé à la racine du projet.
//
// Il démarre target/release/jarvis.exe (la coquille, qui lance le noyau puis le moteur vocal) avec
// la racine du projet comme dossier courant. Si Jarvis v4 tourne déjà, la coquille (instance
// unique) ramène simplement le HUD au premier plan. Si un autre Jarvis (la v3) occupe les ports,
// le lanceur prévient au lieu de démarrer une v4 qui ne pourrait pas ouvrir sa passerelle.

#![windows_subsystem = "windows"]

use std::net::{SocketAddr, TcpStream};
use std::os::windows::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Duration;

use windows::core::{HSTRING, PWSTR};
use windows::Win32::Foundation::CloseHandle;
use windows::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Process32FirstW, Process32NextW, PROCESSENTRY32W, TH32CS_SNAPPROCESS};
use windows::Win32::System::Threading::{OpenProcess, QueryFullProcessImageNameW, PROCESS_NAME_WIN32, PROCESS_QUERY_LIMITED_INFORMATION};
use windows::Win32::UI::WindowsAndMessaging::{MessageBoxW, MB_ICONERROR, MB_ICONWARNING, MB_OK};

const GATEWAY_PORT: u16 = 8797;
const CREATE_NO_WINDOW: u32 = 0x0800_0000;

fn message(text: &str, warning: bool) {
    let icon = if warning { MB_ICONWARNING } else { MB_ICONERROR };
    unsafe { MessageBoxW(None, &HSTRING::from(text), &HSTRING::from("Jarvis v4"), MB_OK | icon) };
}

/// Racine du projet : le dossier qui contient ui/index.html et Cargo.toml, en partant de l'exécutable.
fn project_root() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    exe.ancestors().find(|d| d.join("ui").join("index.html").is_file() && d.join("Cargo.toml").is_file()).map(Path::to_path_buf)
}

/// Chemins complets des processus jarvis.exe en cours.
fn running_shells() -> Vec<PathBuf> {
    let mut found = vec![];
    unsafe {
        let Ok(snap) = CreateToolhelp32Snapshot(TH32CS_SNAPPROCESS, 0) else { return found };
        let mut entry = PROCESSENTRY32W { dwSize: std::mem::size_of::<PROCESSENTRY32W>() as u32, ..Default::default() };
        let mut ok = Process32FirstW(snap, &mut entry).is_ok();
        while ok {
            let len = entry.szExeFile.iter().position(|&c| c == 0).unwrap_or(entry.szExeFile.len());
            let name = String::from_utf16_lossy(&entry.szExeFile[..len]);
            if name.eq_ignore_ascii_case("jarvis.exe") {
                if let Ok(h) = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, false, entry.th32ProcessID) {
                    let mut buf = [0u16; 1024];
                    let mut size = buf.len() as u32;
                    if QueryFullProcessImageNameW(h, PROCESS_NAME_WIN32, PWSTR(buf.as_mut_ptr()), &mut size).is_ok() {
                        found.push(PathBuf::from(String::from_utf16_lossy(&buf[..size as usize])));
                    }
                    let _ = CloseHandle(h);
                }
            }
            ok = Process32NextW(snap, &mut entry).is_ok();
        }
        let _ = CloseHandle(snap);
    }
    found
}

fn same_file(a: &Path, b: &Path) -> bool {
    match (a.canonicalize(), b.canonicalize()) {
        (Ok(a), Ok(b)) => a == b,
        _ => a.to_string_lossy().eq_ignore_ascii_case(&b.to_string_lossy()),
    }
}

fn port_busy(port: u16) -> bool {
    TcpStream::connect_timeout(&SocketAddr::from(([127, 0, 0, 1], port)), Duration::from_millis(300)).is_ok()
}

fn main() {
    let Some(root) = project_root() else {
        message("Dossier de Jarvis v4 introuvable : « Jarvis v4.exe » doit rester à la racine du projet (à côté du dossier ui et de Cargo.toml).", false);
        return;
    };
    let shell = root.join("target").join("release").join("jarvis.exe");
    if !shell.is_file() || !root.join("target").join("release").join("jarvis-core.exe").is_file() {
        message(
            &format!("Jarvis v4 n'est pas compilé.\n\nDans {} :\n    cargo build --release", root.display()),
            false,
        );
        return;
    }
    let shells = running_shells();
    let v4_running = shells.iter().any(|p| same_file(p, &shell));
    if !v4_running && (port_busy(GATEWAY_PORT) || !shells.is_empty()) {
        message(
            "Un autre Jarvis (probablement la v3) est déjà lancé et occupe les ports 8797 à 8799.\n\nFermez-le (icône de la zone de notification → Quitter), puis relancez Jarvis v4.",
            true,
        );
        return;
    }
    if let Err(err) = Command::new(&shell).current_dir(&root).env("JARVIS_ROOT", &root).creation_flags(CREATE_NO_WINDOW).spawn() {
        message(&format!("Impossible de lancer {} :\n{err}", shell.display()), false);
    }
}
