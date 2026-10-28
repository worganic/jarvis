// Contrôles média de Windows (SMTC) : remplace voice/media_control.py de la v3. Lit et pilote la
// session média active — Spotify en priorité, sinon navigateur ou lecteur : titre, artiste, album,
// pochette, position, état ; lecture, pause, précédent, suivant.
//   jarvis-core media-control status | play|pause|pauseall|toggle|next|previous|stop | watch
// En mode watch : une ligne JSON à chaque changement ; commandes lues sur l'entrée standard.

use std::io::{BufRead, Write};
use std::sync::mpsc;
use std::time::Duration;

use base64::Engine;
use serde_json::{json, Map, Value};
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession as Session, GlobalSystemMediaTransportControlsSessionManager as Manager,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus as Status,
};
use windows::Storage::Streams::DataReader;

const THUMB_MAX: u64 = 600_000;
const BROWSERS: &[&str] = &["chrome", "msedge", "edge", "firefox", "opera", "brave"];

fn status_name(s: Status) -> &'static str {
    match s {
        Status::Closed => "closed",
        Status::Opened => "opened",
        Status::Changing => "changing",
        Status::Stopped => "stopped",
        Status::Playing => "playing",
        Status::Paused => "paused",
        _ => "unknown",
    }
}

fn app_id(s: &Session) -> String {
    s.SourceAppUserModelId().map(|h| h.to_string()).unwrap_or_default().to_lowercase()
}

pub struct Media {
    manager: Manager,
    app_filter: String,
    thumb_key: Option<(String, String, String)>,
    thumb: Option<String>,
}

impl Media {
    pub fn new() -> Result<Self, String> {
        crate::com_init();
        let manager = Manager::RequestAsync().and_then(|op| op.get()).map_err(|e| e.message().to_string())?;
        let app_filter = std::env::var("JARVIS_MEDIA_APP").unwrap_or_default().to_lowercase();
        Ok(Self { manager, app_filter, thumb_key: None, thumb: None })
    }

    fn sessions(&self) -> Vec<Session> {
        let mut out = vec![];
        if let Ok(list) = self.manager.GetSessions() {
            for s in list {
                if self.app_filter.is_empty() || app_id(&s).contains(&self.app_filter) {
                    out.push(s);
                }
            }
        }
        out
    }

    /// Session à piloter, sans jamais réveiller un navigateur (YouTube en pause) pour « lecture ».
    fn pick(&self, target: Option<&str>, cmd: Option<&str>) -> Option<Session> {
        let sessions = self.sessions();
        if let Some(t) = target {
            let t = t.to_lowercase();
            return sessions.into_iter().find(|s| app_id(s).contains(&t));
        }
        let spotify = sessions.iter().find(|s| app_id(s).contains("spotify")).cloned();
        if let Some(sp) = &spotify {
            if sp.GetPlaybackInfo().and_then(|i| i.PlaybackStatus()).map(|st| st == Status::Playing).unwrap_or(false) {
                return spotify;
            }
        }
        let play_like = matches!(cmd, Some("play") | Some("toggle"));
        let mut current = self.manager.GetCurrentSession().ok();
        if let Some(c) = &current {
            let ca = app_id(c);
            if !self.app_filter.is_empty() && !ca.contains(&self.app_filter) {
                current = None;
            } else if play_like && BROWSERS.iter().any(|b| ca.contains(b)) {
                return spotify;
            }
        }
        if spotify.is_some() {
            return spotify;
        }
        if play_like {
            return sessions.into_iter().find(|s| {
                let a = app_id(s);
                !BROWSERS.iter().any(|b| a.contains(b))
            });
        }
        current.or_else(|| sessions.into_iter().next())
    }

    /// Met en pause toutes les sessions qui jouent, quelle que soit leur source (Spotify, YouTube dans un navigateur, lecteur…).
    fn pause_all(&self) -> bool {
        let mut any = false;
        for s in self.sessions() {
            let playing = s.GetPlaybackInfo().and_then(|i| i.PlaybackStatus()).map(|st| st == Status::Playing).unwrap_or(false);
            if playing && s.TryPauseAsync().and_then(|o| o.get()).unwrap_or(false) {
                any = true;
            }
        }
        any
    }

    pub fn command(&self, cmd: &str, target: Option<&str>) -> bool {
        if cmd == "pauseall" {
            return self.pause_all();
        }
        let Some(s) = self.pick(target, Some(cmd)) else { return false };
        let op = match cmd {
            "play" => s.TryPlayAsync(),
            "pause" => s.TryPauseAsync(),
            "toggle" => s.TryTogglePlayPauseAsync(),
            "next" => s.TrySkipNextAsync(),
            "previous" => s.TrySkipPreviousAsync(),
            "stop" => s.TryStopAsync(),
            _ => return false,
        };
        op.and_then(|o| o.get()).unwrap_or(false)
    }

    fn read_thumb(props: &windows::Media::Control::GlobalSystemMediaTransportControlsSessionMediaProperties) -> Option<String> {
        let r = props.Thumbnail().ok()?;
        let stream = r.OpenReadAsync().ok()?.get().ok()?;
        let size = stream.Size().ok()?;
        if size == 0 || size > THUMB_MAX {
            return None;
        }
        let reader = DataReader::CreateDataReader(&stream.GetInputStreamAt(0).ok()?).ok()?;
        reader.LoadAsync(size as u32).ok()?.get().ok()?;
        let mut buf = vec![0u8; size as usize];
        reader.ReadBytes(&mut buf).ok()?;
        let mime = if buf.starts_with(b"\x89PNG") { "image/png" } else { "image/jpeg" };
        Some(format!("data:{mime};base64,{}", base64::engine::general_purpose::STANDARD.encode(&buf)))
    }

    pub fn describe(&mut self, with_thumb: bool, target: Option<&str>) -> Value {
        let Some(s) = self.pick(target, None) else { return json!({ "ok": true, "active": false }) };
        let mut out = Map::new();
        out.insert("ok".into(), json!(true));
        out.insert("active".into(), json!(true));
        out.insert("app".into(), json!(s.SourceAppUserModelId().map(|h| h.to_string()).unwrap_or_default()));
        if let Ok(props) = s.TryGetMediaPropertiesAsync().and_then(|o| o.get()) {
            let title = props.Title().map(|h| h.to_string()).unwrap_or_default();
            let artist = props.Artist().map(|h| h.to_string()).unwrap_or_default();
            let album = props.AlbumTitle().map(|h| h.to_string()).unwrap_or_default();
            out.insert("title".into(), json!(title));
            out.insert("artist".into(), json!(artist));
            out.insert("album".into(), json!(album));
            if with_thumb && props.Thumbnail().is_ok() {
                let key = (title, artist, album);
                if self.thumb_key.as_ref() != Some(&key) {
                    self.thumb_key = Some(key);
                    self.thumb = Self::read_thumb(&props);
                }
                if let Some(t) = &self.thumb {
                    out.insert("thumbnail".into(), json!(t));
                }
            }
        }
        if let Ok(info) = s.GetPlaybackInfo() {
            if let Ok(st) = info.PlaybackStatus() {
                out.insert("status".into(), json!(status_name(st)));
            }
            if let Ok(c) = info.Controls() {
                out.insert("canPlay".into(), json!(c.IsPlayEnabled().unwrap_or(false)));
                out.insert("canPause".into(), json!(c.IsPauseEnabled().unwrap_or(false)));
                out.insert("canNext".into(), json!(c.IsNextEnabled().unwrap_or(false)));
                out.insert("canPrevious".into(), json!(c.IsPreviousEnabled().unwrap_or(false)));
            }
        }
        if let Ok(t) = s.GetTimelineProperties() {
            // TimeSpan : unités de 100 ns ; DateTime : depuis 1601.
            if let (Ok(pos), Ok(start), Ok(end)) = (t.Position(), t.StartTime(), t.EndTime()) {
                out.insert("position".into(), json!(pos.Duration / 10_000));
                out.insert("duration".into(), json!((end.Duration - start.Duration) / 10_000));
            }
            if let Ok(upd) = t.LastUpdatedTime() {
                out.insert("updatedAt".into(), json!(upd.UniversalTime / 10_000 - 11_644_473_600_000));
            }
        }
        Value::Object(out)
    }
}

fn emit(v: &Value) {
    let mut o = std::io::stdout();
    let _ = writeln!(o, "{v}");
    let _ = o.flush();
}

fn now_ms() -> i64 {
    std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_millis() as i64).unwrap_or(0)
}

/// Point d'entrée de `jarvis-core media-control <mode> [application]`.
pub fn main(mode: &str, target: Option<&str>) -> i32 {
    let mut media = match Media::new() {
        Ok(m) => m,
        Err(e) => {
            emit(&json!({ "ok": false, "error": e }));
            return 1;
        }
    };
    match mode {
        "status" => emit(&media.describe(true, target)),
        "watch" => watch(&mut media),
        cmd => {
            let done = media.command(cmd, target);
            std::thread::sleep(Duration::from_millis(150));
            let mut st = media.describe(false, target);
            st["done"] = json!(done);
            emit(&st);
        }
    }
    0
}

/// Relevé toutes les 700 ms ; seules les nouveautés sont émises. Commandes sur l'entrée standard.
fn watch(media: &mut Media) {
    let (tx, rx) = mpsc::channel::<String>();
    std::thread::spawn(move || {
        for line in std::io::stdin().lock().lines().map_while(Result::ok) {
            if tx.send(line.trim().to_lowercase()).is_err() {
                return;
            }
        }
        let _ = tx.send("quit".into());
    });
    let mut last: Option<Value> = None;
    let mut last_target: Option<String> = None;
    loop {
        let raw = match rx.recv_timeout(Duration::from_millis(700)) {
            Ok(l) => Some(l),
            Err(mpsc::RecvTimeoutError::Timeout) => None,
            Err(mpsc::RecvTimeoutError::Disconnected) => return,
        };
        if raw.as_deref() == Some("quit") {
            return;
        }
        let mut cmd: Option<String> = None;
        if let Some(r) = raw.filter(|r| !r.is_empty()) {
            let (c, t) = match r.split_once(':') {
                Some((c, t)) => (c.to_string(), Some(t.to_string())),
                None => match r.split_once(char::is_whitespace) {
                    Some((c, t)) => (c.to_string(), Some(t.trim().to_string())),
                    None => (r.clone(), None),
                },
            };
            if let Some(t) = t.filter(|t| !t.is_empty()) {
                last_target = Some(t);
            }
            cmd = Some(c);
        }
        match cmd.as_deref() {
            Some("status") => last = None,
            Some(c) => {
                media.command(c, last_target.as_deref());
                std::thread::sleep(Duration::from_millis(150));
            }
            None => {}
        }
        let mut state = media.describe(true, last_target.as_deref());
        let mut key = state.clone();
        if let Some(o) = key.as_object_mut() {
            o.remove("position");
            o.remove("updatedAt");
        }
        if Some(&key) != last.as_ref() || cmd.is_some() {
            last = Some(key);
            state["at"] = json!(now_ms());
            emit(&state);
        }
    }
}
