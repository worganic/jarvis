#![allow(non_snake_case, non_upper_case_globals)]
//! API Windows natives de Jarvis v4 : remplace les outils C# de la v3 (tools/*.cs).
//!
//! - `window` : fenêtre au premier plan, informations de processus (active-win.cs) ;
//! - `screenshot` : capture d'écran GDI, rognage autour du curseur (screenshot.cs) ;
//! - `ocr` : reconnaissance de texte Windows.Media.Ocr (ocr.ps1, ocr-words.ps1) ;
//! - `audio` : périphériques audio par défaut, volumes, sourdines, crêtes (audio-device.cs, audio-mixer.cs) ;
//! - `desktop` : veille du bureau (fenêtres, écrans, applis en fond) et actions sur les fenêtres (desktop-watch.cs) ;
//! - `uia` : UI Automation, éléments d'une fenêtre, clic par le nom, saisie (desktop-watch.cs).
//! - `input` : presse-papier et frappe clavier (Get-Clipboard, SendKeys).

pub mod audio;
pub mod console;
pub mod desktop;
pub mod input;
pub mod media;
pub mod ocr;
pub mod presence;
pub mod screenshot;
pub mod uia;
pub mod window;

/// Chaîne UTF-16 terminée par zéro.
pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

/// Initialise COM sur le fil courant (multithread), sans erreur si c'est déjà fait.
pub fn com_init() {
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(None, windows::Win32::System::Com::COINIT_MULTITHREADED);
    }
}
