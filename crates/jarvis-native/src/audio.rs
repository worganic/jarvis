// Périphériques audio de Windows (Core Audio) : remplace tools/audio-device.cs (casque ↔ enceintes)
// et tools/audio-mixer.cs (card AUDIO : volumes, sourdines, périphériques par défaut, crêtes).
// Mêmes réponses JSON que la v3.

use serde_json::{json, Value};
use windows::core::{GUID, HRESULT, PCWSTR};
use windows::Win32::Devices::FunctionDiscovery::PKEY_Device_FriendlyName;
use windows::Win32::Media::Audio::Endpoints::{IAudioEndpointVolume, IAudioMeterInformation};
use windows::core::Interface;
use windows::Win32::Media::Audio::{
    AudioSessionStateExpired, IAudioSessionControl2, IAudioSessionManager2, ISimpleAudioVolume,
    eCapture, eCommunications, eConsole, eMultimedia, eRender, EDataFlow, ERole, IMMDevice, IMMDeviceEnumerator, MMDeviceEnumerator, DEVICE_STATE_ACTIVE,
};
use windows::Win32::System::Com::{CoCreateInstance, CLSCTX_ALL, STGM_READ};

// IPolicyConfig (interface non documentée de Windows) : changer le périphérique par défaut.
#[windows_core::interface("f8679f50-850a-41cf-9c72-430f290290c8")]
unsafe trait IPolicyConfig: windows_core::IUnknown {
    fn GetMixFormat(&self) -> HRESULT;
    fn GetDeviceFormat(&self) -> HRESULT;
    fn ResetDeviceFormat(&self) -> HRESULT;
    fn SetDeviceFormat(&self) -> HRESULT;
    fn GetProcessingPeriod(&self) -> HRESULT;
    fn SetProcessingPeriod(&self) -> HRESULT;
    fn GetShareMode(&self) -> HRESULT;
    fn SetShareMode(&self) -> HRESULT;
    fn GetPropertyValue(&self) -> HRESULT;
    fn SetPropertyValue(&self) -> HRESULT;
    fn SetDefaultEndpoint(&self, device: PCWSTR, role: ERole) -> HRESULT;
    fn SetEndpointVisibility(&self) -> HRESULT;
}

const CLSID_POLICY_CONFIG: GUID = GUID::from_u128(0x870af99c_171d_4f9e_af0d_e63df40c2bc9);

pub struct Endpoint {
    pub id: String,
    pub name: String,
    pub is_default: bool,
}

fn enumerator() -> Result<IMMDeviceEnumerator, String> {
    crate::com_init();
    unsafe { CoCreateInstance(&MMDeviceEnumerator, None, CLSCTX_ALL).map_err(|e| e.message().to_string()) }
}

fn policy() -> Result<IPolicyConfig, String> {
    unsafe { CoCreateInstance(&CLSID_POLICY_CONFIG, None, CLSCTX_ALL).map_err(|e| e.message().to_string()) }
}

fn device_id(d: &IMMDevice) -> String {
    unsafe {
        match d.GetId() {
            Ok(p) => {
                let s = p.to_string().unwrap_or_default();
                windows::Win32::System::Com::CoTaskMemFree(Some(p.0 as *const _));
                s
            }
            Err(_) => String::new(),
        }
    }
}

fn device_name(d: &IMMDevice) -> String {
    unsafe {
        let Ok(store) = d.OpenPropertyStore(STGM_READ) else { return "Inconnu".into() };
        let Ok(pv) = store.GetValue(&PKEY_Device_FriendlyName) else { return "Inconnu".into() };
        let p = pv.Anonymous.Anonymous.Anonymous.pwszVal;
        if p.is_null() {
            "Inconnu".into()
        } else {
            p.to_string().unwrap_or_else(|_| "Inconnu".into())
        }
    }
}

fn default_id(e: &IMMDeviceEnumerator, flow: EDataFlow) -> String {
    unsafe { e.GetDefaultAudioEndpoint(flow, eMultimedia).map(|d| device_id(&d)).unwrap_or_default() }
}

fn endpoints(e: &IMMDeviceEnumerator, flow: EDataFlow, default: &str) -> Vec<Endpoint> {
    let mut out = vec![];
    unsafe {
        let Ok(col) = e.EnumAudioEndpoints(flow, DEVICE_STATE_ACTIVE) else { return out };
        let n = col.GetCount().unwrap_or(0);
        for i in 0..n {
            if let Ok(d) = col.Item(i) {
                let id = device_id(&d);
                out.push(Endpoint { is_default: id == default, name: device_name(&d), id });
            }
        }
    }
    out
}

fn set_default(p: &IPolicyConfig, id: &str) {
    if id.is_empty() {
        return;
    }
    let w = crate::wide(id);
    unsafe {
        for role in [eConsole, eMultimedia, eCommunications] {
            let _ = p.SetDefaultEndpoint(PCWSTR(w.as_ptr()), role);
        }
    }
}

fn looks_like_headset(name: &str) -> bool {
    let l = name.to_lowercase();
    ["casque", "headset", "hyperx", "headphone", "écouteurs", "ecouteurs", "wireless", "bluetooth", "cloud"].iter().any(|k| l.contains(k))
}

fn looks_like_pc(name: &str) -> bool {
    let l = name.to_lowercase();
    ["realtek", "haut-parleurs", "speakers", "microphone interne", "built-in", "high definition audio"].iter().any(|k| l.contains(k))
}

/// audio-device.exe status | toggle | set <casque|pc> : casque ↔ enceintes du PC.
pub fn audio_device(cmd: &str, target: &str) -> Value {
    let run = || -> Result<Value, String> {
        let e = enumerator()?;
        let p = policy()?;
        let read = |e: &IMMDeviceEnumerator| {
            let r = endpoints(e, eRender, &default_id(e, eRender));
            let c = endpoints(e, eCapture, &default_id(e, eCapture));
            (r, c)
        };
        let (renders, captures) = read(&e);
        let headset_r = renders.iter().find(|r| looks_like_headset(&r.name)).map(|x| x.id.clone());
        let headset_c = captures.iter().find(|c| looks_like_headset(&c.name)).map(|x| x.id.clone());
        let pc_r = renders.iter().find(|r| !looks_like_headset(&r.name) && looks_like_pc(&r.name)).or_else(|| renders.iter().find(|r| !looks_like_headset(&r.name))).map(|x| x.id.clone());
        let pc_c = captures.iter().find(|c| !looks_like_headset(&c.name) && looks_like_pc(&c.name)).or_else(|| captures.iter().find(|c| !looks_like_headset(&c.name))).map(|x| x.id.clone());
        let cur_headset = renders.iter().find(|r| r.is_default).map(|r| looks_like_headset(&r.name)).unwrap_or(false);
        let to_pc = |p: &IPolicyConfig| {
            if let Some(id) = &pc_r {
                set_default(p, id);
            }
            if let Some(id) = &pc_c {
                set_default(p, id);
            }
        };
        let to_headset = |p: &IPolicyConfig| {
            if let Some(id) = &headset_r {
                set_default(p, id);
            }
            if let Some(id) = &headset_c {
                set_default(p, id);
            }
        };
        let cmd = cmd.to_lowercase();
        if ["toggle", "switch", "basculer"].contains(&cmd.as_str()) {
            if cur_headset {
                to_pc(&p)
            } else {
                to_headset(&p)
            }
        } else if cmd == "set" || cmd == "mode" {
            let mode = if target.is_empty() { "casque".to_string() } else { target.to_lowercase() };
            if ["pc", "ordinateur", "enceinte", "enceintes", "speakers"].contains(&mode.as_str()) {
                to_pc(&p)
            } else {
                to_headset(&p)
            }
        }
        // Relecture après un basculement éventuel.
        let (renders, captures) = read(&e);
        let cur_r = renders.iter().find(|r| r.is_default);
        let cur_c = captures.iter().find(|c| c.is_default);
        let headset = cur_r.map(|r| looks_like_headset(&r.name)).unwrap_or(false);
        let mode = if headset { "casque" } else { "pc" };
        let hr_name = renders.iter().find(|r| looks_like_headset(&r.name)).map(|r| r.name.clone());
        let hc_name = captures.iter().find(|c| looks_like_headset(&c.name)).map(|c| c.name.clone());
        let pr_name = renders.iter().find(|r| !looks_like_headset(&r.name) && looks_like_pc(&r.name)).or_else(|| renders.iter().find(|r| !looks_like_headset(&r.name))).map(|r| r.name.clone());
        let pc_name = captures.iter().find(|c| !looks_like_headset(&c.name) && looks_like_pc(&c.name)).or_else(|| captures.iter().find(|c| !looks_like_headset(&c.name))).map(|c| c.name.clone());
        let clean = |s: &str| s.replace('\r', "").replace('\n', " ");
        Ok(json!({
            "ok": true,
            "mode": mode,
            "label": if headset { "CASQUE" } else { "ENCEINTES / PC" },
            "render": clean(&cur_r.map(|r| r.name.clone()).unwrap_or_else(|| "Inconnu".into())),
            "capture": clean(&cur_c.map(|c| c.name.clone()).unwrap_or_else(|| "Inconnu".into())),
            "headsetAvailable": hr_name.is_some() || hc_name.is_some(),
            "headsetName": clean(&hr_name.or(hc_name).unwrap_or_else(|| "Casque non détecté".into())),
            "pcName": clean(&pr_name.or(pc_name).unwrap_or_else(|| "Haut-parleurs PC".into()))
        }))
    };
    run().unwrap_or_else(|e| json!({ "ok": false, "error": e }))
}

// --- Mélangeur (card AUDIO) ------------------------------------------------------------------------

pub struct DefaultEp {
    pub id: String,
    pub name: String,
    pub volume: Option<IAudioEndpointVolume>,
    pub meter: Option<IAudioMeterInformation>,
}

fn default_endpoint(e: &IMMDeviceEnumerator, flow: EDataFlow) -> DefaultEp {
    unsafe {
        let Ok(d) = e.GetDefaultAudioEndpoint(flow, eMultimedia) else {
            return DefaultEp { id: String::new(), name: String::new(), volume: None, meter: None };
        };
        DefaultEp {
            id: device_id(&d),
            name: device_name(&d),
            volume: d.Activate::<IAudioEndpointVolume>(CLSCTX_ALL, None).ok(),
            meter: d.Activate::<IAudioMeterInformation>(CLSCTX_ALL, None).ok(),
        }
    }
}

fn esc(s: &str) -> String {
    s.chars().map(|c| if (c as u32) < 32 { ' ' } else { c }).collect()
}

/// Mélangeur résident : un objet par fil (COM), commandes de audio-mixer.exe.
pub struct Mixer {
    e: IMMDeviceEnumerator,
    p: IPolicyConfig,
    pub out_ep: DefaultEp,
    pub in_ep: DefaultEp,
    /// Application suivie par la card musique (« spotify », « jarvis »…) et ses sessions audio (relues toutes les 2 s).
    app: String,
    app_sessions: Vec<(ISimpleAudioVolume, IAudioMeterInformation)>,
    app_at: Option<std::time::Instant>,
}

/// Nom de processus attendu pour une application du lecteur multimédia de Windows : « Spotify.exe »,
/// « SpotifyAB.SpotifyMusic_zpdnekdrzrea0!Spotify », « C:…chrome.exe » → « spotify », « chrome ».
pub fn app_token(app: &str) -> String {
    let a = app.trim().to_lowercase();
    let a = a.rsplit('!').next().unwrap_or(&a);
    let a = a.rsplit(['\\', '/']).next().unwrap_or(a);
    a.trim_end_matches(".exe").trim().to_string()
}

/// Processus de l'application (d'après le nom de l'exécutable). « jarvis » : les processus WebView2 lancés par la
/// coquille de Jarvis (le HUD, où joue le lecteur interne), repérés par leur ascendance.
fn app_pids(token: &str) -> std::collections::HashSet<u32> {
    let procs = crate::window::processes();
    let mut out = std::collections::HashSet::new();
    if token == "jarvis" {
        let shell: u32 = std::env::var("JARVIS_SHELL_PID").ok().and_then(|p| p.parse().ok()).unwrap_or(0);
        if shell == 0 {
            return out;
        }
        let parent: std::collections::HashMap<u32, u32> = procs.iter().map(|(p, pp, _)| (*p, *pp)).collect();
        // Seulement WebView2 (le HUD, où joue le lecteur) : la voix de Jarvis (jarvis-voice.exe) descend aussi de la coquille.
        for (pid, _, name) in &procs {
            if !name.to_lowercase().starts_with("msedgewebview2") {
                continue;
            }
            let (mut cur, mut depth) = (*pid, 0);
            while depth < 6 {
                if cur == shell {
                    out.insert(*pid);
                    break;
                }
                match parent.get(&cur) {
                    Some(&pp) if pp != 0 && pp != cur => cur = pp,
                    _ => break,
                }
                depth += 1;
            }
        }
        return out;
    }
    if token.len() < 3 {
        return out;
    }
    for (pid, _, name) in &procs {
        let stem = name.to_lowercase();
        let stem = stem.trim_end_matches(".exe");
        if stem == token || (stem.len() >= 4 && (stem.contains(token) || token.contains(stem))) {
            out.insert(*pid);
        }
    }
    out
}

impl Mixer {
    pub fn new() -> Result<Self, String> {
        let e = enumerator()?;
        let p = policy()?;
        let out_ep = default_endpoint(&e, eRender);
        let in_ep = default_endpoint(&e, eCapture);
        Ok(Self { e, p, out_ep, in_ep, app: String::new(), app_sessions: vec![], app_at: None })
    }

    pub fn refresh(&mut self) {
        self.out_ep = default_endpoint(&self.e, eRender);
        self.in_ep = default_endpoint(&self.e, eCapture);
    }

    fn ep_json(ep: &DefaultEp) -> Value {
        unsafe {
            let v = ep.volume.as_ref().and_then(|v| v.GetMasterVolumeLevelScalar().ok());
            let m = ep.volume.as_ref().and_then(|v| v.GetMute().ok()).map(|b| b.as_bool()).unwrap_or(false);
            json!({
                "id": esc(&ep.id), "name": esc(&ep.name), "available": v.is_some(),
                "volume": v.map(|x| (x * 100.0).round() as i64).unwrap_or(0), "muted": m
            })
        }
    }

    fn devices(&self, flow: EDataFlow, default: &str) -> Value {
        Value::Array(endpoints(&self.e, flow, default).into_iter().map(|d| json!({ "id": esc(&d.id), "name": esc(&d.name), "default": d.id == default })).collect())
    }

    pub fn state(&mut self) -> Value {
        self.refresh();
        json!({
            "t": "state", "ok": true,
            "out": Self::ep_json(&self.out_ep), "in": Self::ep_json(&self.in_ep),
            "outputs": self.devices(eRender, &self.out_ep.id.clone()),
            "inputs": self.devices(eCapture, &self.in_ep.id.clone())
        })
    }

    pub fn error(message: &str) -> Value {
        json!({ "t": "error", "ok": false, "error": esc(message) })
    }

    /// Une commande (« state », « vol out 40 », « mute in toggle », « default <id> ») ; None : rien à répondre.
    pub fn run(&mut self, line: &str) -> Option<Value> {
        let line = line.trim();
        let a: Vec<&str> = line.splitn(3, ' ').filter(|s| !s.is_empty()).collect();
        let cmd = a.first()?.to_lowercase();
        let ctx = GUID::zeroed();
        match cmd.as_str() {
            "state" => Some(self.state()),
            "vol" | "mute" => {
                if a.len() < 3 {
                    return Some(Self::error(&format!("usage : {cmd} out|in valeur")));
                }
                self.refresh();
                let input = a[1] == "in";
                let ep = if input { &self.in_ep } else { &self.out_ep };
                let Some(vol) = ep.volume.clone() else {
                    return Some(Self::error(&format!("aucun périphérique {}", if input { "d'entrée" } else { "de sortie" })));
                };
                unsafe {
                    if cmd == "vol" {
                        let Ok(pct) = a[2].parse::<f64>() else { return Some(Self::error("volume invalide")) };
                        let _ = vol.SetMasterVolumeLevelScalar((pct.clamp(0.0, 100.0) / 100.0) as f32, &ctx);
                    } else {
                        let cur = vol.GetMute().map(|b| b.as_bool()).unwrap_or(false);
                        let on = if a[2] == "toggle" { !cur } else { a[2] == "1" || a[2] == "true" };
                        let _ = vol.SetMute(on, &ctx);
                    }
                }
                Some(self.state())
            }
            // Card musique : « app <nom> » suit une application (vide : aucune), « appvol <pct> » règle son volume.
            "app" => {
                self.app = if a.len() > 1 { app_token(&line[line.find(' ').unwrap_or(0) + 1..]) } else { String::new() };
                self.app_refresh(true);
                Some(self.app_json())
            }
            "appvol" => {
                let Some(pct) = a.get(1).and_then(|x| x.parse::<f32>().ok()) else { return Some(Self::error("usage : appvol <0-100>")) };
                self.app_refresh(true);
                let ctx = GUID::zeroed();
                for (v, _) in &self.app_sessions {
                    unsafe {
                        let _ = v.SetMasterVolume(pct.clamp(0.0, 100.0) / 100.0, &ctx);
                    }
                }
                Some(self.app_json())
            }
            "default" => {
                if a.len() < 2 {
                    return Some(Self::error("usage : default <id>"));
                }
                let id = line[line.find(' ').unwrap_or(0) + 1..].trim().to_string();
                let w = crate::wide(&id);
                if unsafe { self.e.GetDevice(PCWSTR(w.as_ptr())) }.is_err() {
                    return Some(Self::error("périphérique introuvable"));
                }
                set_default(&self.p, &id);
                Some(self.state())
            }
            _ => Some(Self::error(&format!("commande inconnue : {cmd}"))),
        }
    }

    /// Crêtes gauche et droite de la sortie (une seule voie : la même valeur des deux côtés).
    pub fn out_channels(&self) -> (f32, f32) {
        let Some(m) = self.out_ep.meter.as_ref() else { return (0.0, 0.0) };
        unsafe {
            let n = m.GetMeteringChannelCount().unwrap_or(0) as usize;
            if n < 2 {
                let v = m.GetPeakValue().unwrap_or(0.0);
                return (v, v);
            }
            let mut v = vec![0.0f32; n];
            if m.GetChannelsPeakValues(&mut v).is_err() {
                return (0.0, 0.0);
            }
            (v[0], v[1])
        }
    }

    /// Sessions audio de l'application suivie, sur la sortie par défaut (relues au plus toutes les 2 s).
    fn app_refresh(&mut self, force: bool) {
        if self.app.is_empty() {
            self.app_sessions.clear();
            return;
        }
        if !force && self.app_at.map(|t| t.elapsed() < std::time::Duration::from_secs(2)).unwrap_or(false) {
            return;
        }
        self.app_at = Some(std::time::Instant::now());
        self.app_sessions.clear();
        let pids = app_pids(&self.app);
        if pids.is_empty() {
            return;
        }
        unsafe {
            let Ok(d) = self.e.GetDefaultAudioEndpoint(eRender, eMultimedia) else { return };
            let Ok(mgr) = d.Activate::<IAudioSessionManager2>(CLSCTX_ALL, None) else { return };
            let Ok(list) = mgr.GetSessionEnumerator() else { return };
            let n = list.GetCount().unwrap_or(0);
            for i in 0..n {
                let Ok(ctl) = list.GetSession(i) else { continue };
                if ctl.GetState().map(|s| s == AudioSessionStateExpired).unwrap_or(true) {
                    continue;
                }
                let Ok(ctl2) = ctl.cast::<IAudioSessionControl2>() else { continue };
                let Ok(pid) = ctl2.GetProcessId() else { continue };
                if !pids.contains(&pid) {
                    continue;
                }
                if let (Ok(v), Ok(m)) = (ctl.cast::<ISimpleAudioVolume>(), ctl.cast::<IAudioMeterInformation>()) {
                    self.app_sessions.push((v, m));
                }
            }
        }
    }

    /// Crêtes gauche et droite de l'application suivie (max sur ses sessions), et session trouvée ou non.
    pub fn app_channels(&mut self) -> Option<(f32, f32, bool)> {
        if self.app.is_empty() {
            return None;
        }
        self.app_refresh(false);
        let (mut l, mut r) = (0.0f32, 0.0f32);
        for (_, m) in &self.app_sessions {
            unsafe {
                let n = m.GetMeteringChannelCount().unwrap_or(0) as usize;
                if n >= 2 {
                    let mut v = vec![0.0f32; n];
                    if m.GetChannelsPeakValues(&mut v).is_ok() {
                        l = l.max(v[0]);
                        r = r.max(v[1]);
                    }
                } else if let Ok(v) = m.GetPeakValue() {
                    l = l.max(v);
                    r = r.max(v);
                }
            }
        }
        Some((l, r, !self.app_sessions.is_empty()))
    }

    /// Volume propre de l'application suivie (celui du mélangeur de volume de Windows), en %.
    fn app_json(&self) -> Value {
        let v = self.app_sessions.first().and_then(|(v, _)| unsafe { v.GetMasterVolume().ok() });
        json!({ "t": "app", "ok": true, "app": esc(&self.app), "found": !self.app_sessions.is_empty(), "volume": v.map(|x| (x * 100.0).round() as i64) })
    }

    pub fn peaks(&self) -> (f32, f32) {
        let peak = |ep: &DefaultEp| unsafe { ep.meter.as_ref().and_then(|m| m.GetPeakValue().ok()).unwrap_or(0.0) };
        (peak(&self.out_ep), peak(&self.in_ep))
    }
}

/// audio-mixer.exe serve : boucle résidente sur l'entrée et la sortie standard (mêmes lignes JSON
/// que la v3). « meter 1 » ajoute les crêtes toutes les 100 ms, « watch 1 » renvoie l'état quand il
/// change ailleurs.
pub fn mixer_serve() -> i32 {
    use std::io::{BufRead, Write};
    use std::sync::mpsc;
    use std::time::{Duration, Instant};
    let mut mixer = match Mixer::new() {
        Ok(m) => m,
        Err(e) => {
            println!("{}", Mixer::error(&e));
            return 1;
        }
    };
    let (tx, rx) = mpsc::channel::<String>();
    std::thread::spawn(move || {
        let stdin = std::io::stdin();
        for line in stdin.lock().lines().map_while(Result::ok) {
            if tx.send(line).is_err() {
                break;
            }
        }
    });
    let mut out = std::io::stdout();
    let mut last_state = String::new();
    let mut next_watch = Instant::now();
    let (mut meter_on, mut watch_on) = (false, false);
    mixer.refresh();
    loop {
        match rx.recv_timeout(Duration::from_millis(100)) {
            Ok(line) => {
                let t = line.trim();
                if t == "quit" {
                    break;
                }
                let parts: Vec<&str> = t.split_whitespace().collect();
                match parts.first().copied() {
                    Some("meter") => meter_on = parts.get(1) == Some(&"1"),
                    Some("watch") => watch_on = parts.get(1) == Some(&"1"),
                    _ => {
                        if let Some(res) = mixer.run(t) {
                            let s = res.to_string();
                            if res["t"] == "state" {
                                last_state = s.clone();
                            }
                            let _ = writeln!(out, "{s}");
                            let _ = out.flush();
                        }
                    }
                }
                continue;
            }
            Err(mpsc::RecvTimeoutError::Disconnected) => break,
            Err(mpsc::RecvTimeoutError::Timeout) => {}
        }
        if meter_on {
            let (o, i) = mixer.peaks();
            let (l, r) = mixer.out_channels();
            // Application suivie par la card musique : ses crêtes à elle (al, ar) et session trouvée (af).
            let app = mixer.app_channels().map(|(al, ar, f)| format!(",\"al\":{al:.3},\"ar\":{ar:.3},\"af\":{f}")).unwrap_or_default();
            let _ = writeln!(out, "{{\"t\":\"peak\",\"out\":{o:.3},\"in\":{i:.3},\"l\":{l:.3},\"r\":{r:.3}{app}}}");
            let _ = out.flush();
        }
        if watch_on && Instant::now() >= next_watch {
            next_watch = Instant::now() + Duration::from_millis(1200);
            let s = mixer.state().to_string();
            if s != last_state {
                last_state = s.clone();
                let _ = writeln!(out, "{s}");
                let _ = out.flush();
            }
        }
    }
    0
}
