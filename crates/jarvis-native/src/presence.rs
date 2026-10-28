// Présence et état du poste (compagnon de bureau) : temps depuis la dernière action au clavier ou à la souris
// (jamais les touches elles-mêmes : pas de crochet clavier), batterie, nombre d'éléments dans la corbeille.

use windows::core::PCWSTR;
use windows::Win32::System::Power::{GetSystemPowerStatus, SYSTEM_POWER_STATUS};
use windows::Win32::System::SystemInformation::GetTickCount;
use windows::Win32::UI::Input::KeyboardAndMouse::{GetLastInputInfo, LASTINPUTINFO};
use windows::Win32::UI::Shell::{SHQueryRecycleBinW, SHQUERYRBINFO};

/// Secondes écoulées depuis la dernière action de l'utilisateur (clavier, souris, toucher).
pub fn idle_seconds() -> f64 {
    let mut info = LASTINPUTINFO { cbSize: std::mem::size_of::<LASTINPUTINFO>() as u32, dwTime: 0 };
    unsafe {
        if !GetLastInputInfo(&mut info).as_bool() {
            return 0.0;
        }
        GetTickCount().wrapping_sub(info.dwTime) as f64 / 1000.0
    }
}

/// (sur batterie, pourcentage) ; None sur un PC fixe ou si Windows ne le sait pas.
pub fn battery() -> Option<(bool, u8)> {
    let mut s = SYSTEM_POWER_STATUS::default();
    unsafe { GetSystemPowerStatus(&mut s).ok()? };
    // 128 : pas de batterie ; 255 : inconnu.
    if s.BatteryFlag & 128 != 0 || s.BatteryLifePercent == 255 {
        return None;
    }
    Some((s.ACLineStatus == 0, s.BatteryLifePercent))
}

/// Éléments dans la corbeille (tous les lecteurs).
pub fn recycle_bin_count() -> Option<i64> {
    let mut info = SHQUERYRBINFO { cbSize: std::mem::size_of::<SHQUERYRBINFO>() as u32, ..Default::default() };
    unsafe { SHQueryRecycleBinW(PCWSTR::null(), &mut info).ok()? };
    Some(info.i64NumItems)
}
