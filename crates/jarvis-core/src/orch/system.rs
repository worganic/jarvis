// Actions système sous Windows (orchestrator/system.js) : actions nommées, contrôlées et sûres plutôt
// qu'un accès shell libre. Les outils natifs C# de la v3 sont remplacés par jarvis-native.

use std::cell::RefCell;
use std::rc::Weak;
use std::sync::Mutex;

use serde_json::{json, Value};

use super::desktop_state::DesktopState;
use crate::js::{self, J};
use crate::proc;

thread_local! {
    /// Veille du bureau en direct (orchestrator/desktop-state.js), branchée par l'orchestrateur.
    pub static LIVE_DESKTOP: RefCell<Weak<DesktopState>> = RefCell::new(Weak::new());
}

fn live() -> Option<std::rc::Rc<DesktopState>> {
    LIVE_DESKTOP.with(|l| l.borrow().upgrade()).filter(|d| d.ready())
}

/// Navigateurs qu'ouvrir_url accepte de lancer nommément.
const BROWSER_EXES: &[&str] = &["chrome", "firefox", "msedge", "brave", "opera"];

/// Table d'applications connues (évite les homonymies risquées).
pub fn app_registry(name: &str) -> Option<&'static str> {
    Some(match name {
        "photoshop" => "photoshop",
        "chrome" | "navigateur" => "chrome",
        "edge" => "msedge",
        "firefox" => "firefox",
        "spotify" | "musique" => "spotify",
        "code" | "vscode" => "code",
        "notepad" | "bloc-notes" => "notepad",
        "calc" | "calculatrice" => "calc",
        "explorer" | "explorateur" => "explorer",
        "terminal" => "wt",
        "cmd" => "cmd",
        "powershell" => "powershell",
        "word" => "winword",
        "excel" => "excel",
        "paint" => "mspaint",
        _ => return None,
    })
}

pub type R = Result<J, String>;

/// Texte reconnu par l'OCR de Windows sur une image (vide en cas d'échec).
pub async fn extract_ocr(image: &str, lang: &str) -> String {
    let (image, lang) = (image.to_string(), lang.to_string());
    tokio::task::spawn_blocking(move || jarvis_native::ocr::ocr_text(&image, &lang).unwrap_or_default()).await.unwrap_or_default().trim().to_string()
}

pub async fn foreground_window() -> J {
    tokio::task::spawn_blocking(jarvis_native::window::foreground).await.unwrap_or_else(|_| json!({ "process": "", "title": "", "pid": 0 }))
}

fn spotify_launch(args: &str) {
    let raw = crate::re!(r"^--uri=").replace(args, "").trim().to_string();
    proc::detached("cmd.exe", &["/c", "start", "", if raw.is_empty() { "spotify:" } else { &raw }]);
}

pub async fn ouvrir_application(p: &J) -> R {
    let nom = js::gs(p, "nom");
    if nom.is_empty() {
        return Err("Nom de l’application requis".into());
    }
    let app_args = js::gs(p, "arguments");
    let clean = nom.to_lowercase().trim().to_string();
    if clean == "spotify" || clean == "spootify" {
        let _ = proc::exec("taskkill /F /IM SpotifyLauncher.exe").await;
        if jarvis_native::window::is_running("Spotify.exe") {
            if !app_args.is_empty() {
                let raw = crate::re!(r"^--uri=").replace(&app_args, "").trim().to_string();
                if !raw.is_empty() {
                    proc::detached("cmd.exe", &["/c", "start", "", &raw]);
                }
            }
            return Ok(json!({ "ok": true, "message": "Spotify est déjà en cours d’exécution" }));
        }
        let win_apps = std::path::PathBuf::from(js::env("LOCALAPPDATA").unwrap_or_default()).join("Microsoft").join("WindowsApps").join("Spotify.exe");
        if !app_args.is_empty() {
            spotify_launch(&app_args);
        } else if win_apps.exists() {
            proc::detached("explorer.exe", &["shell:AppsFolder\\SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify"]);
        } else {
            proc::detached("cmd.exe", &["/c", "start", "spotify:"]);
        }
        return Ok(json!({ "ok": true, "message": "Application Spotify lancée" }));
    }
    let target = app_registry(&clean).map(String::from).unwrap_or_else(|| nom.clone());
    if !crate::re!(r"^[a-zA-Z0-9._-]+$").is_match(&target) {
        return Err(format!("Nom d’exécutable invalide : {target}"));
    }
    let mut args: Vec<&str> = vec!["/c", "start", "", &target];
    let extra: Vec<String> = if app_args.is_empty() { vec![] } else { app_args.split_whitespace().map(String::from).collect() };
    args.extend(extra.iter().map(|s| s.as_str()));
    proc::detached("cmd.exe", &args);
    Ok(json!({ "ok": true, "message": format!("Application {nom} lancée") }))
}

/// Le succès n'est annoncé qu'une fois le processus réellement disparu.
pub async fn fermer_application(p: &J) -> R {
    let nom = js::gs(p, "nom");
    if nom.is_empty() {
        return Err("Nom de l’application requis".into());
    }
    let clean = crate::re!(r"(?i)\.exe$").replace(nom.trim(), "").to_string();
    if !crate::re!(r"^[\w .+-]{1,60}$").is_match(&clean) {
        return Ok(json!({ "ok": false, "error": format!("nom d’application invalide : {nom}") }));
    }
    let exe = format!("{clean}.exe");
    if !jarvis_native::window::is_running(&exe) {
        return Ok(json!({ "ok": false, "error": format!("{clean} n’est pas ouvert") }));
    }
    let _ = proc::exec(&format!("taskkill /IM \"{exe}\" /T")).await;
    for _ in 0..8 {
        js::sleep_ms(250).await;
        if !jarvis_native::window::is_running(&exe) {
            return Ok(json!({ "ok": true, "message": format!("Application {clean} fermée") }));
        }
    }
    Ok(json!({ "ok": false, "error": format!("{clean} ne s’est pas fermé (il attend peut-être une confirmation à l’écran)") }))
}

pub async fn lister_fenetres(_p: &J) -> R {
    if let Some(live) = live() {
        return Ok(Value::Array(
            live.windows()
                .iter()
                .map(|w| json!({ "Id": w["pid"], "ProcessName": w["process"], "Application": w["app"], "MainWindowTitle": w["title"], "Ecran": w["monitor"], "Etat": w["state"] }))
                .collect(),
        ));
    }
    let out = proc::powershell("$procs = Get-Process | Where-Object { $_.MainWindowTitle -ne '' } | Select-Object Id, ProcessName, MainWindowTitle\n$procs | ConvertTo-Json -Compress").await?;
    if out.is_empty() {
        return Ok(json!([]));
    }
    Ok(match serde_json::from_str::<J>(&out) {
        Ok(Value::Array(a)) => Value::Array(a),
        Ok(v) => json!([v]),
        Err(_) => json!([]),
    })
}

pub async fn activer_fenetre(p: &J) -> R {
    let titre = js::gs(p, "titre");
    if titre.is_empty() {
        return Err("Titre de fenêtre requis".into());
    }
    let safe = crate::re!(r#"["'\\]"#).replace_all(&titre, "").into_owned();
    let out = proc::powershell(&format!("$wscript = New-Object -ComObject WScript.Shell\n$ok = $wscript.AppActivate(\"{safe}\")\nif ($ok) {{ \"ACTIVATED\" }} else {{ \"NOT_FOUND\" }}")).await?;
    if out.contains("ACTIVATED") {
        Ok(json!({ "ok": true, "message": format!("Fenêtre \"{titre}\" activée au premier plan") }))
    } else {
        Ok(json!({ "ok": false, "message": format!("Aucune fenêtre trouvée correspondant à \"{titre}\"") }))
    }
}

pub async fn ouvrir_url(p: &J) -> R {
    let url = js::gs(p, "url");
    if url.is_empty() {
        return Err("URL requise".into());
    }
    let trimmed = url.trim();
    if !crate::re!(r"(?i)^https?://").is_match(trimmed) {
        return Err("L’URL doit commencer par http:// ou https://".into());
    }
    let nav = js::gs(p, "navigateur");
    let browser = BROWSER_EXES.contains(&nav.as_str()).then_some(nav);
    // « & | < > ^ " » couperaient la commande de cmd.exe : une telle adresse est confiée à l'explorateur, sans shell.
    if trimmed.chars().any(|c| "&|<>^\"".contains(c)) {
        if browser.is_some() {
            return Err("Cette adresse contient des caractères spéciaux : elle ne peut pas être ouverte dans un navigateur précis.".into());
        }
        proc::detached("explorer.exe", &[trimmed]);
        return Ok(json!({ "ok": true, "message": format!("URL ouverte dans le navigateur : {trimmed}") }));
    }
    let mut args = vec!["/c", "start", ""];
    if let Some(b) = &browser {
        args.push(b);
    }
    args.push(trimmed);
    proc::detached("cmd.exe", &args);
    Ok(json!({ "ok": true, "message": format!("URL ouverte dans {} : {trimmed}", browser.as_deref().unwrap_or("le navigateur")) }))
}

pub async fn ouvrir_parametres(p: &J) -> R {
    let uri = format!("ms-settings:{}", js::gs(p, "page").trim());
    if !crate::re!(r"(?i)^ms-settings:[a-z0-9-]*$").is_match(&uri) {
        return Err(format!("Page de paramètres invalide : {}", js::gs(p, "page")));
    }
    proc::detached("cmd.exe", &["/c", "start", "", &uri]);
    Ok(json!({ "ok": true, "message": format!("Paramètres ouverts : {uri}") }))
}

/// Extinction / redémarrage différés de 30 s (annulables), ou annulation.
pub async fn alimentation(p: &J) -> R {
    let action = js::gs(p, "action");
    let flags: &[&str] = match action.as_str() {
        "eteindre" => &["/s", "/t", "30"],
        "redemarrer" => &["/r", "/t", "30"],
        "annuler" => &["/a"],
        _ => return Err(format!("Action d’alimentation inconnue : {action}")),
    };
    let o = proc::run("shutdown.exe", &flags.iter().map(|s| s.to_string()).collect::<Vec<_>>(), None, 15000).await?;
    if o.code != 0 && action != "annuler" {
        return Err(format!("shutdown a répondu {}", o.code));
    }
    let msg = match action.as_str() {
        "eteindre" => "Extinction dans 30 secondes",
        "redemarrer" => "Redémarrage dans 30 secondes",
        _ => "Extinction annulée",
    };
    Ok(json!({ "ok": true, "message": msg }))
}

pub async fn vider_corbeille(_p: &J) -> R {
    proc::powershell("Clear-RecycleBin -Force -ErrorAction SilentlyContinue; \"OK\"").await?;
    Ok(json!({ "ok": true, "message": "Corbeille vidée" }))
}

pub async fn ouvrir_dossier(p: &J) -> R {
    let chemin = js::gs(p, "chemin");
    if chemin.is_empty() {
        return Err("Chemin de dossier requis".into());
    }
    let resolved = super::zones::resolve(&chemin);
    if !std::path::Path::new(&resolved).exists() {
        return Err(format!("Le dossier n’existe pas : {resolved}"));
    }
    proc::detached("explorer.exe", &[&resolved]);
    Ok(json!({ "ok": true, "message": format!("Dossier ouvert : {resolved}") }))
}

pub async fn presse_papier_lire(_p: &J) -> R {
    let text = proc::powershell("Get-Clipboard -Raw").await?;
    Ok(json!({ "texte": text }))
}

pub async fn presse_papier_ecrire(p: &J) -> R {
    use base64::Engine;
    let b64 = base64::engine::general_purpose::STANDARD.encode(js::gs(p, "texte").as_bytes());
    proc::powershell(&format!("$b = [System.Convert]::FromBase64String('{b64}')\n$s = [System.Text.Encoding]::UTF8.GetString($b)\nSet-Clipboard -Value $s")).await?;
    Ok(Value::Null)
}

/// Capture d'écran en PNG (avec OCR facultatif).
pub async fn capture_ecran(p: &J) -> R {
    let workspace = js::gs(p, "workspace");
    let base = if workspace.is_empty() { crate::paths::workspace() } else { std::path::PathBuf::from(&workspace) }.join("captures");
    let _ = std::fs::create_dir_all(&base);
    let nom = js::gs(p, "nom_fichier");
    let filename = format!(
        "{}.png",
        if nom.is_empty() { format!("capture_{}", js::now_ms()) } else { crate::re!(r"[^a-zA-Z0-9._-]").replace_all(&nom, "_").into_owned() }
    );
    let target = base.join(filename);
    let t2 = target.clone();
    let shot = tokio::task::spawn_blocking(move || jarvis_native::screenshot::screenshot(Some(&t2), "png", 90, 0)).await.map_err(|e| e.to_string())?;
    let resolution = if js::gb(&shot, "ok") { format!("{}x{}", js::gs(&shot, "width"), js::gs(&shot, "height")) } else { return Err(js::gs(&shot, "error")) };
    let chemin = target.to_string_lossy().into_owned();
    let ocr = if js::gb(p, "avec_ocr") { extract_ocr(&chemin, "fr-FR").await } else { String::new() };
    let mut res = json!({
        "ok": true, "chemin": chemin, "resolution": resolution,
        "message": format!("Capture d’écran enregistrée : {chemin}{}", if ocr.is_empty() { String::new() } else { format!("\n\nTexte extrait par OCR :\n{ocr}") })
    });
    if !ocr.is_empty() {
        res["texte_ocr"] = json!(ocr);
    }
    Ok(res)
}

/// Lecture de l'écran par OCR local (0 token).
pub async fn lire_ecran_ocr(p: &J) -> R {
    let cap = capture_ecran(&json!({ "workspace": p.get("workspace").cloned().unwrap_or(Value::Null), "avec_ocr": true })).await?;
    let raw = js::gs(&cap, "texte_ocr");
    if raw.trim().is_empty() {
        return Ok(json!({ "ok": true, "chemin": cap["chemin"], "texte": "", "resume": "Aucun texte lisible détecté à l'écran.", "message": "Aucun texte lisible détecté sur l'écran actuellement." }));
    }
    let mut summary = raw.clone();
    if js::gb(p, "filtrer_erreur") {
        let lines: Vec<&str> = raw.lines().map(|l| l.trim()).filter(|l| !l.is_empty()).collect();
        let err: Vec<&str> = lines
            .iter()
            .copied()
            .filter(|l| crate::re!(r"(?i)\b(?:error|erreur|exception|warning|failed|échec|echec|fatal|crash|cannot|denied|introuvable|invalide)\b").is_match(l))
            .collect();
        summary = if !err.is_empty() { err.iter().take(4).copied().collect::<Vec<_>>().join(". ") } else { lines.iter().take(3).copied().collect::<Vec<_>>().join(". ") };
    }
    Ok(json!({ "ok": true, "chemin": cap["chemin"], "texte": raw, "resume": summary, "message": format!("Texte extrait de l'écran :\n{raw}") }))
}

pub async fn dessiner(p: &J) -> R {
    let ws = js::gs(p, "workspace");
    super::drawing::draw(&js::gs(p, "sujet"), if ws.is_empty() { None } else { Some(std::path::Path::new(&ws)) }, true)
}

/// Touches média du volume (comme WScript.SendKeys de la v3).
fn volume_keys(key: u8, times: usize) {
    use windows::Win32::UI::Input::KeyboardAndMouse::{keybd_event, KEYEVENTF_KEYUP};
    for _ in 0..times {
        unsafe {
            keybd_event(key, 0, Default::default(), 0);
            keybd_event(key, 0, KEYEVENTF_KEYUP, 0);
        }
    }
}

pub async fn volume(p: &J) -> R {
    const MUTE: u8 = 0xAD;
    const DOWN: u8 = 0xAE;
    const UP: u8 = 0xAF;
    let action = js::gs(p, "action");
    let out = match action.as_str() {
        "mute" => {
            volume_keys(MUTE, 1);
            "MUTE_TOGGLED".to_string()
        }
        "up" => {
            volume_keys(UP, 5);
            "VOLUME_UP".to_string()
        }
        "down" => {
            volume_keys(DOWN, 5);
            "VOLUME_DOWN".to_string()
        }
        _ if !p.get("niveau").map(|v| v.is_null()).unwrap_or(true) => {
            let pct = js::num(&p["niveau"]).unwrap_or(0.0).clamp(0.0, 100.0);
            volume_keys(DOWN, 50);
            volume_keys(UP, (pct / 2.0).round() as usize);
            format!("SET_VOLUME_{}", js::num_str(pct))
        }
        _ => return Err("Préciser un niveau (0 à 100) ou une action (up, down, mute)".into()),
    };
    Ok(json!({ "ok": true, "message": format!("Volume ajusté : {out}") }))
}

/// Luminosité actuelle (en %), ou None si l'écran ne la pilote pas.
pub async fn luminosite_actuelle() -> Option<f64> {
    let out = proc::powershell("try { (Get-WmiObject -Namespace root/WMI -Class WmiMonitorBrightness).CurrentBrightness | Select-Object -First 1 } catch { \"\" }").await.ok()?;
    out.trim().parse().ok()
}

pub async fn luminosite(p: &J) -> R {
    let delta = js::num(&p["ecart"]).filter(|d| d.is_finite() && !p["ecart"].is_null());
    let relative = delta.is_some() && p.get("pourcentage").map(|v| v.is_null()).unwrap_or(true);
    let target = if relative {
        format!("[Math]::Max(10, [Math]::Min(100, $cur + ({})))", delta.unwrap().round())
    } else {
        js::num_str(js::num(&p["pourcentage"]).unwrap_or(0.0).clamp(10.0, 100.0))
    };
    let out = proc::powershell(&format!(
        "try {{\n  $cur = (Get-WmiObject -Namespace root/WMI -Class WmiMonitorBrightness).CurrentBrightness | Select-Object -First 1\n  $val = {target}\n  (Get-WmiObject -Namespace root/WMI -Class WmiMonitorBrightnessMethods).WmiSetBrightness(1, $val)\n  \"BRIGHTNESS_$val\"\n}} catch {{\n  \"BRIGHTNESS_UNSUPPORTED\"\n}}"
    ))
    .await?;
    if out.contains("BRIGHTNESS_UNSUPPORTED") {
        return Ok(json!({ "ok": false, "message": "La luminosité matérielle n’est pas gérée sur cet écran." }));
    }
    let val: Option<i64> = crate::re!(r"BRIGHTNESS_(\d+)").captures(&out).and_then(|c| c[1].parse().ok());
    Ok(json!({ "ok": true, "valeur": val, "message": format!("Luminosité réglée à {}%", val.map(|v| v.to_string()).unwrap_or_else(|| "NaN".into())) }))
}

pub async fn verrouiller(_p: &J) -> R {
    proc::detached("rundll32.exe", &["user32.dll,LockWorkStation"]);
    Ok(json!({ "ok": true, "message": "Session Windows verrouillée" }))
}

pub async fn veille(_p: &J) -> R {
    proc::detached("rundll32.exe", &["powrprof.dll,SetSuspendState", "0,1,0"]);
    Ok(json!({ "ok": true, "message": "Mise en veille déclenchée" }))
}

pub async fn fenetre_active(_p: &J) -> R {
    if let Some(l) = live() {
        if let Some(fg) = l.foreground() {
            return Ok(fg);
        }
    }
    Ok(foreground_window().await)
}

pub async fn telemetrie(_p: &J) -> R {
    Ok(telemetry())
}

// --- Périphérique audio (casque ↔ enceintes) -----------------------------------------------------

pub async fn get_audio_device() -> J {
    tokio::task::spawn_blocking(|| jarvis_native::audio::audio_device("status", "")).await.unwrap_or_else(|e| json!({ "ok": false, "error": e.to_string() }))
}

pub async fn toggle_audio_device() -> J {
    tokio::task::spawn_blocking(|| jarvis_native::audio::audio_device("toggle", "")).await.unwrap_or_else(|e| json!({ "ok": false, "error": e.to_string() }))
}

pub async fn set_audio_device(mode: &str) -> J {
    let m = if mode.is_empty() { "casque".to_string() } else { mode.to_string() };
    tokio::task::spawn_blocking(move || jarvis_native::audio::audio_device("set", &m)).await.unwrap_or_else(|e| json!({ "ok": false, "error": e.to_string() }))
}

/// Action système par son nom (System[action] de la v3). None : action inconnue.
pub async fn call(action: &str, p: &J) -> Option<R> {
    Some(match action {
        "ouvrir_application" => ouvrir_application(p).await,
        "fermer_application" => fermer_application(p).await,
        "lister_fenetres" => lister_fenetres(p).await,
        "activer_fenetre" => activer_fenetre(p).await,
        "ouvrir_url" => ouvrir_url(p).await,
        "ouvrir_parametres" => ouvrir_parametres(p).await,
        "alimentation" => alimentation(p).await,
        "vider_corbeille" => vider_corbeille(p).await,
        "ouvrir_dossier" => ouvrir_dossier(p).await,
        "presse_papier_lire" => presse_papier_lire(p).await,
        "presse_papier_ecrire" => presse_papier_ecrire(p).await,
        "capture_ecran" => capture_ecran(p).await,
        "lire_ecran_ocr" => lire_ecran_ocr(p).await,
        "dessiner" => dessiner(p).await,
        "volume" => volume(p).await,
        "luminosite" => luminosite(p).await,
        "verrouiller" => verrouiller(p).await,
        "veille" => veille(p).await,
        "fenetre_active" => fenetre_active(p).await,
        "telemetrie" => telemetrie(p).await,
        "getAudioDevice" => Ok(get_audio_device().await),
        "toggleAudioDevice" => Ok(toggle_audio_device().await),
        "setAudioDevice" => Ok(set_audio_device(&js::gs(p, "mode")).await),
        _ => return None,
    })
}

pub fn has(action: &str) -> bool {
    matches!(
        action,
        "ouvrir_application" | "fermer_application" | "lister_fenetres" | "activer_fenetre" | "ouvrir_url" | "ouvrir_parametres" | "alimentation"
            | "vider_corbeille" | "ouvrir_dossier" | "presse_papier_lire" | "presse_papier_ecrire" | "capture_ecran" | "lire_ecran_ocr" | "dessiner"
            | "volume" | "luminosite" | "verrouiller" | "veille" | "fenetre_active" | "telemetrie" | "getAudioDevice" | "toggleAudioDevice" | "setAudioDevice"
    )
}

// --- Télémétrie CPU / RAM ------------------------------------------------------------------------

static PREV_CPU: Mutex<Option<(u64, u64)>> = Mutex::new(None);

fn ft(f: windows::Win32::Foundation::FILETIME) -> u64 {
    ((f.dwHighDateTime as u64) << 32) | f.dwLowDateTime as u64
}

/// Charge CPU globale et occupation mémoire (getSystemTelemetry de la v3).
pub fn telemetry() -> J {
    use windows::Win32::Foundation::FILETIME;
    use windows::Win32::System::ProcessStatus::{GetProcessMemoryInfo, PROCESS_MEMORY_COUNTERS};
    use windows::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    use windows::Win32::System::Threading::{GetCurrentProcess, GetSystemTimes};
    let mut mem = MEMORYSTATUSEX { dwLength: std::mem::size_of::<MEMORYSTATUSEX>() as u32, ..Default::default() };
    unsafe {
        let _ = GlobalMemoryStatusEx(&mut mem);
    }
    let total = mem.ullTotalPhys as f64;
    let used = (mem.ullTotalPhys - mem.ullAvailPhys) as f64;
    let ram_pct = if total > 0.0 { (used / total * 100.0).round() } else { 0.0 };
    let gb = 1024f64.powi(3);
    let (mut idle, mut kernel, mut user) = (FILETIME::default(), FILETIME::default(), FILETIME::default());
    unsafe {
        let _ = GetSystemTimes(Some(&mut idle), Some(&mut kernel), Some(&mut user));
    }
    let (idle, total_t) = (ft(idle), ft(kernel) + ft(user)); // le temps noyau inclut l'inactivité
    let mut cpu = 0f64;
    {
        let mut prev = PREV_CPU.lock().unwrap();
        if let Some((pi, pt)) = *prev {
            let (di, dt) = (idle.saturating_sub(pi), total_t.saturating_sub(pt));
            if dt > 0 {
                cpu = ((1.0 - di as f64 / dt as f64) * 100.0).round().clamp(0.0, 100.0);
            }
        }
        *prev = Some((idle, total_t));
    }
    let mut pmc = PROCESS_MEMORY_COUNTERS { cb: std::mem::size_of::<PROCESS_MEMORY_COUNTERS>() as u32, ..Default::default() };
    let proc_mb = unsafe {
        if GetProcessMemoryInfo(GetCurrentProcess(), &mut pmc, pmc.cb).is_ok() {
            (pmc.WorkingSetSize as f64 / 1048576.0).round()
        } else {
            0.0
        }
    };
    json!({
        "cpu": cpu as i64,
        "ramPct": ram_pct as i64,
        "ramUsedGb": format!("{:.1}", used / gb),
        "ramTotalGb": format!("{:.1}", total / gb),
        "procMb": proc_mb as i64,
        "isOverloaded": cpu >= 90.0 || ram_pct >= 90.0
    })
}
