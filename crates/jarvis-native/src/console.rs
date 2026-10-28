// Onglets de terminal (Windows Terminal) : fenêtre hôte d'un onglet et couleur de l'onglet.
//
// Chaque onglet est une pseudo-console ; Windows Terminal rattache sa fenêtre (GetConsoleWindow) à la
// fenêtre qui l'affiche, et suit l'onglet quand on le déplace d'une fenêtre à l'autre. La couleur passe par
// la séquence DECAC « ESC [ 2 ; fg ; bg , | » écrite dans la console de l'onglet : bg est un index de la
// palette du profil (Campbell : 1 rouge, 2 vert, 3 jaune, 4 bleu…). Une couleur choisie à la main dans
// Windows Terminal (ou --tabColor) reste prioritaire.
//
// Se rattacher à une console touche tout le processus : à lancer dans un processus à part
// (`jarvis-core console-tabs`), jamais dans le noyau.

use serde_json::{json, Value};
use windows::core::{w, PCWSTR};
use windows::Win32::Foundation::{CloseHandle, GENERIC_READ, GENERIC_WRITE, HANDLE};
use windows::Win32::Storage::FileSystem::{CreateFileW, FILE_FLAGS_AND_ATTRIBUTES, FILE_SHARE_READ, FILE_SHARE_WRITE, OPEN_EXISTING};
use windows::Win32::Foundation::HWND;
use windows::Win32::System::Console::{
    AttachConsole, FreeConsole, GetConsoleMode, GetConsoleTitleW, GetConsoleWindow, GetStdHandle, SetConsoleMode, SetStdHandle, WriteConsoleW, CONSOLE_MODE,
    ENABLE_VIRTUAL_TERMINAL_PROCESSING, STD_ERROR_HANDLE, STD_OUTPUT_HANDLE,
};
use windows::Win32::UI::WindowsAndMessaging::{GetWindow, GW_OWNER};

/// Se rattache à la console du processus `pid` : fenêtre hôte de l'onglet (0 si inconnue), titre de la
/// console (= nom de l'onglet) et, si `color` est donné, colore l'onglet. None si le processus n'a pas de console.
fn tab(pid: u32, color: Option<u8>) -> Option<(i64, String, bool)> {
    unsafe {
        let _ = FreeConsole();
        AttachConsole(pid).ok()?;
        let cw = GetConsoleWindow();
        let host = if cw.0.is_null() { 0 } else { GetWindow(cw, GW_OWNER).map(|h| h.0 as i64).unwrap_or(0) };
        let mut buf = [0u16; 512];
        let n = GetConsoleTitleW(&mut buf) as usize;
        let title = String::from_utf16_lossy(&buf[..n.min(buf.len())]);
        let mut written = false;
        if let Some(c) = color {
            if let Ok(out) = CreateFileW(PCWSTR(w!("CONOUT$").as_ptr()), (GENERIC_READ | GENERIC_WRITE).0, FILE_SHARE_READ | FILE_SHARE_WRITE, None, OPEN_EXISTING, FILE_FLAGS_AND_ATTRIBUTES(0), None) {
                let mut mode = CONSOLE_MODE(0);
                let _ = GetConsoleMode(out, &mut mode);
                let _ = SetConsoleMode(out, mode | ENABLE_VIRTUAL_TERMINAL_PROCESSING);
                let seq: Vec<u16> = format!("\x1b[2;15;{c},|").encode_utf16().collect();
                written = WriteConsoleW(out, &seq, None, None).is_ok();
                let _ = SetConsoleMode(out, mode);
                let _ = CloseHandle(out);
            }
        }
        let _ = FreeConsole();
        Some((host, title, written))
    }
}

/// Palette Campbell (couleurs que Jarvis pose dans les onglets).
const CAMPBELL: &[[u8; 3]] = &[
    [0xC5, 0x0F, 0x1F], [0x13, 0xA1, 0x0E], [0xC1, 0x9C, 0x00], [0x00, 0x37, 0xDA], [0x88, 0x17, 0x98], [0x3A, 0x96, 0xDD],
    [0xE7, 0x48, 0x56], [0x16, 0xC6, 0x0C], [0xF9, 0xF1, 0xA5], [0x3B, 0x78, 0xFF], [0xB4, 0x00, 0x9E], [0x61, 0xD6, 0xD6],
];

/// Onglet inactif : Windows Terminal le peint à ~31 % de sa couleur sur le fond de la barre (#2C2C2C, thème
/// sombre). On retire ce mélange, puis on se cale sur la palette Campbell si la couleur en est proche.
fn unblend(c: [u8; 3]) -> [u8; 3] {
    const A: f64 = 0.311;
    const BG: f64 = 44.0;
    let raw = c.map(|v| ((v as f64 - BG * (1.0 - A)) / A).round().clamp(0.0, 255.0) as u8);
    let dist = |p: &[u8; 3]| p.iter().zip(raw.iter()).map(|(a, b)| (*a as i32 - *b as i32).abs()).sum::<i32>();
    CAMPBELL.iter().min_by_key(|p| dist(p)).filter(|p| dist(p) < 60).copied().unwrap_or(raw)
}

/// Couleur réellement affichée par chaque onglet d'une fenêtre : celle de Jarvis, ou celle choisie à la main
/// dans Windows Terminal (prioritaire). Image de la fenêtre (même cachée) lue sur une grille de points de
/// l'onglet ; la couleur majoritaire l'emporte (le texte et l'icône sont minoritaires). Renvoie (pid, couleur,
/// exacte) : exacte pour l'onglet sélectionné, estimée pour un onglet inactif (peint atténué). None : onglet
/// sans couleur (gris du thème), fenêtre réduite ou onglet introuvable.
fn tab_colors(host: i64, titles: &[(u32, String)]) -> Vec<(u32, Option<String>, bool)> {
    let h = HWND(host as *mut _);
    let (tabs, shot) = crate::screenshot::dpi_aware(|| (crate::uia::tab_items(h), crate::screenshot::capture_window(h)));
    let Ok((img, ox, oy)) = shot else { return titles.iter().map(|(p, _)| (*p, None, false)).collect() };
    let norm = |s: &str| crate::uia::norm(s);
    titles
        .iter()
        .map(|(pid, title)| {
            // Onglet de ce processus : même titre ; une fenêtre à un seul onglet n'a pas besoin du titre.
            let t = tabs.iter().find(|t| !title.is_empty() && norm(&t.0) == norm(title)).or(if tabs.len() == 1 { tabs.first() } else { None });
            let selected = t.map(|t| t.5).unwrap_or(false);
            let color = t.and_then(|&(_, x, y, w, hh, selected)| {
                let mut count: std::collections::HashMap<[u8; 3], usize> = std::collections::HashMap::new();
                for fx in [0.03, 0.06, 0.5, 0.75, 0.92] {
                    for fy in [0.25, 0.75] {
                        let px = (x - ox as i64) as f64 + w as f64 * fx;
                        let py = (y - oy as i64) as f64 + hh as f64 * fy;
                        if px < 0.0 || py < 0.0 || px >= img.width() as f64 || py >= img.height() as f64 {
                            continue;
                        }
                        let p = img.get_pixel(px as u32, py as u32);
                        *count.entry([p[0], p[1], p[2]]).or_default() += 1;
                    }
                }
                let (c, n) = count.into_iter().max_by_key(|(_, n)| *n)?;
                let c = if selected { c } else { unblend(c) };
                let (mx, mn) = (c.iter().max().copied().unwrap_or(0) as i32, c.iter().min().copied().unwrap_or(0) as i32);
                // Gris du thème (onglet sans couleur) : écarté.
                if n < 3 || mx - mn < 30 {
                    return None;
                }
                Some(format!("#{:02X}{:02X}{:02X}", c[0], c[1], c[2]))
            });
            (*pid, color, selected)
        })
        .collect()
}

/// `jarvis-core console-tabs <pid>[:<couleur>]…` : une ligne JSON [{ pid, hwnd, title, colored, shown }] sur la
/// sortie (shown : couleur réellement affichée par l'onglet, « #RRGGBB » ou null ; exact : lue sur l'onglet
/// sélectionné, sinon estimée).
pub fn main(args: &[String]) -> i32 {
    // La sortie standard est un tube vers le noyau : la garder, AttachConsole pourrait la remplacer par la
    // console de l'onglet (le JSON s'afficherait alors dans le terminal de l'utilisateur).
    let (out, err): (HANDLE, HANDLE) = unsafe { (GetStdHandle(STD_OUTPUT_HANDLE).unwrap_or_default(), GetStdHandle(STD_ERROR_HANDLE).unwrap_or_default()) };
    let mut found: Vec<(u32, i64, String, bool)> = vec![];
    for a in args {
        let (p, c) = match a.split_once(':') {
            Some((p, c)) => (p, c.parse::<u8>().ok()),
            None => (a.as_str(), None),
        };
        let Ok(pid) = p.parse::<u32>() else { continue };
        if let Some((hwnd, title, colored)) = tab(pid, c) {
            found.push((pid, hwnd, title, colored));
        }
    }
    // Laisser Windows Terminal redessiner les onglets qui viennent d'être colorés avant de les lire.
    if found.iter().any(|f| f.3) {
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    let mut shown: std::collections::HashMap<u32, (Option<String>, bool)> = std::collections::HashMap::new();
    let hosts: std::collections::BTreeSet<i64> = found.iter().map(|f| f.1).filter(|h| *h != 0).collect();
    for host in hosts {
        let titles: Vec<(u32, String)> = found.iter().filter(|f| f.1 == host).map(|f| (f.0, f.2.clone())).collect();
        shown.extend(tab_colors(host, &titles).into_iter().map(|(p, c, exact)| (p, (c, exact))));
    }
    let res: Vec<Value> = found
        .iter()
        .map(|(pid, hwnd, title, colored)| {
            let (color, exact) = shown.get(pid).cloned().unwrap_or((None, false));
            json!({ "pid": pid, "hwnd": hwnd, "title": title, "colored": colored, "shown": color, "exact": exact })
        })
        .collect();
    unsafe {
        let _ = SetStdHandle(STD_OUTPUT_HANDLE, out);
        let _ = SetStdHandle(STD_ERROR_HANDLE, err);
    }
    println!("{}", Value::Array(res));
    0
}
