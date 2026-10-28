// Liaison noyau ↔ moteur vocal (core/voice-link.js de la v3).
//
// Le mot d'éveil, la fenêtre de conversation et le filtre d'écho vivent dans le noyau : la voix
// fonctionne quel que soit l'état des fenêtres, et l'interface ne fait qu'afficher (événements
// voice_*). Hors conversation, le moteur ne transcrit que les phrases qui commencent par « Jarvis ».

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet, VecDeque};
use std::rc::{Rc, Weak};

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use tokio::sync::mpsc;
use tokio_tungstenite::tungstenite::Message;

use crate::js::{self, Timer, J};
use crate::orch::Orchestrator;
use crate::voice_supervisor::VoiceSupervisor;

const WAKE: &str = r"(?:dis\s+jarvis|hey\s+jarvis|ok\s+jarvis|bonjour\s+jarvis|salut\s+jarvis|jarvis|jervis|jarvice)";
const WAKE_VARIANTS: &[&str] = &["jarvi", "jarviss", "jarvisse", "jarvise", "charvis", "jarvice", "jervis", "garvis", "djarvis", "jarviz", "jarvys"];
/// Conversation ouverte sans redire « Jarvis » : prolongée à chaque phrase et à chaque réponse.
const CONVERSATION_MS: i64 = 40000;
const ECHO_WINDOW: i64 = 15000;
const STOP_HOLD_MS: i64 = 4000;
/// Voix non reconnue confirmée dans ce délai : la phrase est aussi exécutée.
const SPEAKER_RUN_MS: i64 = 45000;
const SPEAKER_KEEP_MS: i64 = 10 * 60000;
const SPEAKER_TYPES: &[&str] = &["moi", "femme", "mari", "fille", "fils", "famille", "ami", "invite", "visio", "autre"];
const MAX_FREE_WORDS: usize = 45;

fn wake_prefix() -> &'static fancy_regex::Regex {
    crate::fre!(&format!(r"(?i)^\s*{WAKE}\b[\s,.!:]*"))
}

fn contains_wake(t: &str) -> bool {
    js::ftest(crate::fre!(&format!(r"(?i)\b{WAKE}\b")), t)
}

fn end_conversation(t: &str) -> bool {
    js::ftest(crate::fre!(r"(?i)^(?:merci[\s,.!]*)?(?:c['’]est tout|ce sera tout|ça sera tout|ca sera tout|fin de (?:la )?conversation|au revoir|à plus tard|a plus tard|bonne nuit)\b"), t)
}

/// Pendant que Jarvis parle : simple acquiescement, pas une demande.
fn backchannel(t: &str) -> bool {
    crate::re!(r"(?i)^(?:oui|ouais|ouai|mouais|ok|okay|d['’ ]?accord|dac|hm+|mh+|hum+|euh+|ah+|oh+|ha+|je vois|super|parfait|top|cool|bien|très bien|tres bien|c['’]est ça|c['’]est ca|exactement|merci|vas[- ]y|continue)$").is_match(t)
}

/// Arrêt immédiat de tout : phrase courte seulement (« arrête de lire le fichier » reste une demande).
fn stop_now(t: &str) -> bool {
    crate::re!(r"(?i)^(?:stop(?:pe)?|arr[eê]te(?:[- ]toi)?|arr[eê]te tout|stop tout|tais[- ]toi|silence|chut+|ça suffit|ca suffit|annule(?: tout)?|laisse tomber|stop stop)$").is_match(t)
}

const VOCATIVE: &str = r"(?:ch[ée]ri(?:e)?|mon (?:c(?:œ|oe)ur|amour|ch[ée]ri|grand|petit|fils|pote|vieux)|ma (?:ch[ée]rie|puce|belle|grande|fille|biche)|maman|papa|mamie|papi|mamy|papy|b[ée]b[ée]|les (?:enfants|gars|filles|amis|copains)|ma poule)";

fn to_someone(t: &str) -> bool {
    js::ftest(crate::fre!(&format!(r"(?i)^{VOCATIVE}\b[\s,]|[\s,]{VOCATIVE}\s*[?!.…]*$")), t)
}

/// Écoute libre (conversation ouverte, sans « Jarvis ») : phrase manifestement adressée à quelqu'un
/// d'autre, écartée. Rend le motif, ou None. Dire « Jarvis » passe toujours outre.
pub fn not_for_jarvis(text: &str, has_wake: bool, expects_reply: bool) -> Option<&'static str> {
    let t = text.trim();
    let bare = bare_command(t);
    if expects_reply {
        return None;
    }
    if has_wake {
        let about = js::ftest(crate::fre!(r"(?i)(?:^|[\s'’])(?:à|a|de|du|que|avec|sur|le|ton|mon|ce|demande)\s+(?:jarvis|jervis|jarvice)\b"), t);
        let called = js::ftest(crate::fre!(r"(?i)[,!]\s*(?:jarvis|jervis|jarvice)\b"), t);
        return (!js::ftest(wake_prefix(), t) && about && !called).then_some("parle de Jarvis");
    }
    if crate::re!(r"(?i)^(?:allô|allo|all[ôo]\s+oui)\b").is_match(t) {
        return Some("téléphone");
    }
    if to_someone(&bare) || to_someone(t) {
        return Some("adressé à quelqu’un");
    }
    if crate::re!(r"(?i)^(?:(?:ha|hé|hi|ah|héhé|hihi|mdr|lol)[\s,.!]*)+$").is_match(&bare) {
        return Some("rire");
    }
    if backchannel(&bare) && !crate::re!(r"(?i)^merci$").is_match(&bare) {
        return Some("acquiescement");
    }
    if bare.split_whitespace().count() > MAX_FREE_WORDS {
        return Some("trop long pour une demande");
    }
    None
}

/// « Jarvis, stop ! » → « stop » (mot d'éveil et ponctuation retirés).
pub fn bare_command(text: &str) -> String {
    let t = wake_prefix().replace(text, "").to_string();
    let t = crate::re!(r"[\s,.!?…]+$").replace(&t, "").to_string();
    let t = crate::re!(r"[,.!?]").replace_all(&t, " ").to_string();
    let t = crate::re!(r"\s+").replace_all(&t, " ").to_string();
    crate::re!(r"(?i)\s*jarvis$").replace(&t, "").trim().to_string()
}

fn edit_distance(a: &str, b: &str) -> usize {
    let (a, b): (Vec<char>, Vec<char>) = (a.chars().collect(), b.chars().collect());
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for i in 1..=a.len() {
        let mut diag = row[0];
        row[0] = i;
        for j in 1..=b.len() {
            let tmp = row[j];
            row[j] = (row[j] + 1).min(row[j - 1] + 1).min(diag + usize::from(a[i - 1] != b[j - 1]));
            diag = tmp;
        }
    }
    row[b.len()]
}

/// Variantes mal transcrites (« Charvis », « J'arvis », « Jarvisse »…) en tête de phrase seulement.
pub fn normalize_wake(text: &str) -> String {
    let Some(m) = crate::re!(r"(?is)^\s*((?:(?:dis|hey|ok|okay|bonjour|salut)\s+)?)([a-zà-ÿ'’]+)(.*)$").captures(text) else { return text.to_string() };
    let raw = m[2].to_lowercase();
    if raw == "jarvis" {
        return text.to_string();
    }
    let word = raw.replace(['\'', '’'], "");
    let n = word.chars().count();
    // « Ok Google », « hey Google » : l'appel est mal entendu (Whisper connaît mieux ces mots) ; seul « Google » seul reste une
    // vraie recherche. Pas de « Google » sans « ok », « hey », « dis »…
    let google_call = !m[1].trim().is_empty() && (word == "google" || word == "gougle");
    let close = word == "jarvis" || WAKE_VARIANTS.contains(&word.as_str()) || google_call || ((5..=8).contains(&n) && edit_distance(&word, "jarvis") <= 1);
    if close {
        format!("{}Jarvis{}", &m[1], &m[3])
    } else {
        text.to_string()
    }
}

fn speech_words(text: &str) -> Vec<String> {
    let f = js::fold_lower(text);
    crate::re!(r"[^a-z0-9\s]").replace_all(&f, " ").split_whitespace().filter(|w| w.len() > 1).map(String::from).collect()
}

#[derive(Clone)]
struct TaskState {
    status: String,
    background: bool,
    conversational: bool,
}

struct SpeakerPending {
    text: String,
    meta: J,
    at: i64,
}

pub struct VoiceLink {
    me: RefCell<Weak<VoiceLink>>,
    sup: Rc<VoiceSupervisor>,
    orch: Rc<Orchestrator>,
    to_ui: Rc<dyn Fn(J)>,
    log: Rc<dyn Fn(String)>,
    tx: RefCell<Option<mpsc::UnboundedSender<Message>>>,
    pub authed: Cell<bool>,
    conn_seq: Cell<u64>,
    retry: Timer,
    pub state: RefCell<J>,
    listening_mode: RefCell<String>,
    vocabulary: RefCell<Option<J>>,
    voice_setting: RefCell<Option<J>>,
    recent_speech: RefCell<VecDeque<(HashSet<String>, i64)>>,
    conversation_until: Cell<i64>,
    conversation_closed_at: Cell<i64>,
    pending: RefCell<HashMap<String, TaskState>>,
    ptt_active: Cell<bool>,
    ptt_released_at: Cell<i64>,
    pub engine_metrics: RefCell<Option<J>>,
    pub audio_mode: RefCell<Option<String>>,
    music_playing: Cell<bool>,
    stop_handled_until: Cell<i64>,
    last_question_at: Cell<i64>,
    note_listening: Cell<bool>,
    terminal_listening: Cell<bool>,
    relay_listening: Cell<bool>,
    meeting_on: Cell<bool>,
    pub mic_muted: Cell<bool>,
    /// Écoute mise en pause automatiquement pendant une visio (Google Meet) : remise en route à la fin de la réunion.
    pub visio_paused: Cell<bool>,
    /// Dernière phrase « voix non reconnue » annoncée à voix haute (ms) : une fois toutes les 30 s au plus.
    unrecognized_hint_at: Cell<i64>,
    speaker_pending: RefCell<Vec<(String, SpeakerPending)>>,
    speaker_seq: Cell<u64>,
    learning: RefCell<HashMap<String, (J, String)>>,
    pub jarvis_volume: Cell<i64>,
}

impl VoiceLink {
    pub fn new(sup: Rc<VoiceSupervisor>, orch: Rc<Orchestrator>, to_ui: Rc<dyn Fn(J)>, log: Rc<dyn Fn(String)>) -> Rc<Self> {
        let mode = orch.status.d.borrow().listening_mode.clone();
        let vol = js::gn(&orch.config.get("audio"), "jarvisVolume").filter(|v| v.is_finite()).map(|v| v as i64).unwrap_or(100);
        let l = Rc::new(Self {
            me: RefCell::new(Weak::new()),
            sup,
            orch,
            to_ui,
            log,
            tx: RefCell::new(None),
            authed: Cell::new(false),
            conn_seq: Cell::new(0),
            retry: Timer::new(),
            state: RefCell::new(json!({ "ready": false, "capturing": false, "speaking": false, "muted": false })),
            listening_mode: RefCell::new(if mode.is_empty() { "wakeword".into() } else { mode }),
            vocabulary: RefCell::new(None),
            voice_setting: RefCell::new(None),
            recent_speech: RefCell::new(VecDeque::new()),
            conversation_until: Cell::new(0),
            conversation_closed_at: Cell::new(-1_000_000_000),
            pending: RefCell::new(HashMap::new()),
            ptt_active: Cell::new(false),
            ptt_released_at: Cell::new(0),
            engine_metrics: RefCell::new(None),
            audio_mode: RefCell::new(None),
            music_playing: Cell::new(false),
            stop_handled_until: Cell::new(0),
            last_question_at: Cell::new(0),
            note_listening: Cell::new(false),
            terminal_listening: Cell::new(false),
            relay_listening: Cell::new(false),
            meeting_on: Cell::new(false),
            mic_muted: Cell::new(false),
            visio_paused: Cell::new(false),
            unrecognized_hint_at: Cell::new(0),
            speaker_pending: RefCell::new(vec![]),
            speaker_seq: Cell::new(0),
            learning: RefCell::new(HashMap::new()),
            jarvis_volume: Cell::new(vol),
        });
        *l.me.borrow_mut() = Rc::downgrade(&l);
        l
    }

    fn ui(&self, ev: J) {
        (self.to_ui)(ev);
    }

    fn state_event(&self, connected: bool) -> J {
        let mut ev = json!({ "type": "voice_state" });
        js::assign(&mut ev, self.state.borrow().clone());
        ev["connected"] = json!(connected);
        ev
    }

    /// État pour l'interface (voiceState) et le diagnostic.
    pub fn snapshot(&self) -> J {
        self.state_event(self.authed.get())
    }

    pub fn is_ready(&self) -> bool {
        self.authed.get() && js::gb(&self.state.borrow(), "ready")
    }

    pub fn set_jarvis_volume(&self, pct: f64) -> i64 {
        let v = (if pct.is_finite() { pct } else { 0.0 }).clamp(0.0, 100.0).round() as i64;
        self.jarvis_volume.set(v);
        self.send(json!({ "t": "volume", "level": v as f64 / 100.0 }));
        v
    }

    /// Pendant une visio (Google Meet), l'écoute de Jarvis est mise en pause : plus de transcription (Whisper) en tâche de
    /// fond, qui charge le processeur et le micro en même temps que la visio. Remise en route à la fin, sauf si le micro
    /// a été coupé ou rétabli à la main entre-temps. Réglage : visio.pauseListening (administration).
    fn visio_pause(&self, on: bool) {
        let enabled = self.orch.config.get("visio")["pauseListening"] != json!(false);
        if on && enabled && !self.mic_muted.get() && !self.visio_paused.get() {
            self.visio_paused.set(true);
            self.mic_muted.set(true);
            self.send(json!({ "t": "mute" }));
            (self.log)("[voix] visio en cours : écoute mise en pause (visio.pauseListening)".into());
        } else if !on && self.visio_paused.get() {
            self.visio_paused.set(false);
            self.mic_muted.set(false);
            self.send(json!({ "t": "unmute" }));
            (self.log)("[voix] visio terminée : écoute rétablie".into());
        }
    }

    pub fn set_mic_muted(&self, on: bool) -> J {
        self.visio_paused.set(false); // le micro est repris en main : plus de remise en route automatique
        self.mic_muted.set(on);
        self.send(json!({ "t": if on { "mute" } else { "unmute" } }));
        (self.log)(format!("[voix] micro {} depuis le HUD", if on { "coupé" } else { "rétabli" }));
        json!({ "ok": true, "muted": on })
    }

    // --- Connexion au moteur ----------------------------------------------------------------

    pub fn connect(self: &Rc<Self>) {
        self.retry.clear();
        if self.sup.child.borrow().is_none() {
            let w = Rc::downgrade(self);
            self.retry.set(1500, move || {
                if let Some(l) = w.upgrade() {
                    l.connect();
                }
            });
            return;
        }
        let seq = self.conn_seq.get() + 1;
        self.conn_seq.set(seq);
        self.authed.set(false);
        let me = Rc::downgrade(self);
        let url = format!("ws://127.0.0.1:{}", self.sup.port);
        let token = self.sup.token.clone();
        js::spawn(async move {
            let conn = tokio_tungstenite::connect_async(url).await;
            let Some(l) = me.upgrade() else { return };
            let ws = match conn {
                Ok((ws, _)) => ws,
                Err(_) => {
                    l.lost(seq);
                    return;
                }
            };
            let (mut sink, mut source) = ws.split();
            let (tx, mut rx) = mpsc::unbounded_channel::<Message>();
            *l.tx.borrow_mut() = Some(tx);
            let _ = sink.send(Message::Text(json!({ "t": "auth", "token": token }).to_string().into())).await;
            let writer = js::spawn(async move {
                while let Some(m) = rx.recv().await {
                    if sink.send(m).await.is_err() {
                        break;
                    }
                }
            });
            drop(l);
            while let Some(Ok(msg)) = source.next().await {
                let text = match msg {
                    Message::Text(t) => t.to_string(),
                    Message::Close(_) => break,
                    _ => continue,
                };
                let Some(l) = me.upgrade() else { break };
                if l.conn_seq.get() != seq {
                    break;
                }
                let Ok(v) = serde_json::from_str::<Value>(&text) else { continue };
                if !l.authed.get() {
                    l.authed.set(true);
                    l.on_connected();
                }
                l.on_engine(&v);
            }
            writer.abort();
            if let Some(l) = me.upgrade() {
                l.lost(seq);
            }
        });
    }

    fn lost(self: &Rc<Self>, seq: u64) {
        if self.conn_seq.get() != seq {
            return;
        }
        self.tx.borrow_mut().take();
        self.authed.set(false);
        {
            let mut st = self.state.borrow_mut();
            st["ready"] = json!(false);
            st["capturing"] = json!(false);
            st["speaking"] = json!(false);
        }
        self.ui(self.state_event(false));
        let w = Rc::downgrade(self);
        self.retry.set(1500, move || {
            if let Some(l) = w.upgrade() {
                l.connect();
            }
        });
    }

    pub fn send(&self, msg: J) {
        if !self.authed.get() {
            return;
        }
        if let Some(tx) = self.tx.borrow().as_ref() {
            let _ = tx.send(Message::Text(msg.to_string().into()));
        }
    }

    fn send_spread(&self, t: &str, extra: &J) {
        let mut m = json!({ "t": t });
        js::assign(&mut m, extra.clone());
        self.send(m);
    }

    /// Le moteur a pu redémarrer : on lui renvoie tout ce qu'il doit savoir.
    fn on_connected(&self) {
        (self.log)("[voix] noyau relié au moteur vocal".into());
        if let Some(v) = self.vocabulary.borrow().clone() {
            self.send_spread("vocab", &v);
        }
        if let Some(v) = self.voice_setting.borrow().clone() {
            let mut m = v.clone();
            m["sample"] = json!(false);
            self.send_spread("voice", &m);
        }
        let mode = if *self.listening_mode.borrow() == "wakeword" { "wakeword" } else { "continuous" };
        self.send(json!({ "t": "mode", "mode": mode }));
        if self.music_playing.get() {
            self.send(json!({ "t": "pc_meter", "on": true }));
        }
        if self.meeting_on.get() {
            self.send(json!({ "t": "meeting", "on": true })); // moteur relancé en pleine réunion
        }
        if self.mic_muted.get() {
            self.send(json!({ "t": "mute" }));
        }
        if self.jarvis_volume.get() != 100 {
            self.send(json!({ "t": "volume", "level": self.jarvis_volume.get() as f64 / 100.0 }));
        }
        self.sync_gate();
    }

    // --- Événements du moteur -----------------------------------------------------------------

    fn on_engine(self: &Rc<Self>, msg: &J) {
        let t = js::gs(msg, "t");
        match t.as_str() {
            "level" => self.ui(json!({ "type": "level", "value": msg["v"] })),
            "pc_level" => self.ui(json!({ "type": "pc_level", "v": msg["v"] })),
            "state" => {
                let was_speaking = js::gb(&self.state.borrow(), "speaking");
                {
                    let mut st = self.state.borrow_mut();
                    js::assign(&mut st, msg.clone());
                    if let Some(o) = st.as_object_mut() {
                        o.remove("t");
                    }
                }
                self.ui(self.state_event(true));
                // Fin de la parole de Jarvis : on laisse le temps de répondre sans redire « Jarvis ».
                if was_speaking && !js::gb(msg, "speaking") && js::now_ms() - self.conversation_closed_at.get() > 8000 {
                    self.open_conversation(CONVERSATION_MS);
                }
            }
            "who" => self.ui(json!({ "type": "voice_who", "who": if js::truthy(&msg["who"]) { msg["who"].clone() } else { Value::Null } })),
            "partial" => {
                let text = js::gs(msg, "text");
                self.ui(json!({ "type": "voice_partial", "text": text }));
                self.speculate(&text);
            }
            "final" => {
                let meta = json!({
                    "speaker": if js::truthy(&msg["speaker"]) { msg["speaker"].clone() } else { Value::Null },
                    "speakerUnknown": js::truthy(&msg["speakerUnknown"]),
                    "speakerCheck": if js::truthy(&msg["speakerCheck"]) { msg["speakerCheck"].clone() } else { Value::Null },
                    "config": msg["config"]
                });
                self.on_final(&js::gs(msg, "text"), &meta);
            }
            "speaker_learned" => self.on_speaker_learned(msg),
            "wake" => self.ui(json!({ "type": "voice_wake", "source": msg["source"] })),
            "ignored" => self.ui(json!({ "type": "voice_ignored", "preview": msg["preview"] })),
            "interrupted" => self.ui(json!({ "type": "voice_interrupted" })),
            "metrics" => *self.engine_metrics.borrow_mut() = Some(json!({ "mb": msg["mb"], "cpu": msg["cpu"], "at": js::now_ms() })),
            "speaker_enrolled" => {
                let status = if msg["status"].is_object() { msg["status"].clone() } else { json!({}) };
                let speaker = msg["speaker"].clone();
                self.orch.record("enrolement_locuteur_termine", json!({ "status": status, "speaker": speaker, "origine": "voix" }));
                self.orch.publish_status();
                self.ui(json!({ "type": "speaker_enrolled", "status": status, "speaker": speaker }));
                let name = Some(js::gs(&speaker, "name")).filter(|n| !n.is_empty()).unwrap_or_else(|| "votre voix".into());
                self.say_local(&format!("Empreinte vocale de {name} enregistrée avec succès !"));
            }
            "speaker_reset" => {
                self.ui(json!({ "type": "speaker_reset_done", "status": msg["status"] }));
                self.say_local("Empreintes vocales réinitialisées.");
            }
            // Phrase qui semble inachevée : le moteur attend la suite.
            "waiting_more" => self.ui(json!({ "type": "voice_waiting", "text": msg["text"] })),
            // Réunion enregistrée : card de réunion et archive (meetings.rs).
            "meeting_state" | "meeting_text" | "meeting_done" | "meeting_voice" | "meeting_voice_named" => self.orch.meetings.on_engine(msg),
            other => {
                if other.starts_with("speaker_") {
                    let mut ev = msg.clone();
                    ev["type"] = json!(other);
                    self.ui(ev);
                }
            }
        }
    }

    fn is_echo(&self, text: &str) -> bool {
        let words = speech_words(text);
        if words.len() < 2 {
            return false;
        }
        let now = js::now_ms();
        let mut spoken = HashSet::new();
        for (w, at) in self.recent_speech.borrow().iter() {
            if now - at < ECHO_WINDOW {
                spoken.extend(w.iter().cloned());
            }
        }
        if spoken.is_empty() {
            return false;
        }
        words.iter().filter(|w| spoken.contains(*w)).count() as f64 / words.len() as f64 >= 0.7
    }

    fn remember_speech(&self, text: &str) {
        if crate::re!(r"\?\s*$").is_match(text) {
            self.last_question_at.set(js::now_ms()); // « oui » y répondra
        }
        let mut r = self.recent_speech.borrow_mut();
        r.push_back((speech_words(text).into_iter().collect(), js::now_ms()));
        while r.len() > 12 {
            r.pop_front();
        }
    }

    /// Arrêt demandé à la voix : une seule fois par phrase (partielle puis finale).
    fn stop_from_voice(&self) {
        if js::now_ms() < self.stop_handled_until.get() {
            return;
        }
        self.stop_handled_until.set(js::now_ms() + STOP_HOLD_MS);
        self.send(json!({ "t": "stop" }));
        self.orch.stop_all("voix");
        // Jarvis se tait et laisse la parole : la suite peut se dire sans redire « Jarvis ».
        self.open_conversation(CONVERSATION_MS);
    }

    fn speculate(&self, text: &str) {
        let raw = text.trim();
        // « Stop » : pas besoin d'attendre la fin de la phrase.
        if stop_now(&bare_command(&normalize_wake(raw))) && !self.is_echo(raw) {
            self.stop_from_voice();
            return;
        }
        if raw.chars().count() < 4 || self.is_echo(raw) {
            return;
        }
        let cleaned = wake_prefix().replace(&normalize_wake(raw), "").trim().to_string();
        if cleaned.chars().count() >= 4 {
            self.orch.on_speculative_utterance(&cleaned, &json!({ "source": "voice" }));
        }
    }

    /// Ce qui est dit part dans un terminal (au premier plan, ou relais vers Claude) : sans « Jarvis ».
    fn writing_to_terminal(&self) -> bool {
        (self.terminal_listening.get() || self.relay_listening.get()) && !self.meeting_on.get()
    }

    fn in_conversation(&self) -> bool {
        let now = js::now_ms();
        if *self.listening_mode.borrow() != "wakeword" {
            return false;
        }
        if self.note_listening.get() || self.writing_to_terminal() {
            return true;
        }
        // En réunion, la conversation libre est coupée : on parle aux participants, pas à Jarvis.
        if self.meeting_on.get() {
            return false;
        }
        if now - self.conversation_closed_at.get() < 3000 {
            return false;
        }
        if now < self.conversation_until.get() {
            return true;
        }
        if js::gb(&self.state.borrow(), "speaking") {
            return now - self.conversation_closed_at.get() > 8000;
        }
        self.pending.borrow().values().any(|t| !t.background && t.conversational)
    }

    fn awaiting_reply(&self) -> bool {
        self.pending.borrow().values().any(|t| t.status == "awaiting" || t.status == "waiting_reply")
    }

    fn open_conversation(&self, ms: i64) {
        if *self.listening_mode.borrow() != "wakeword" || (self.meeting_on.get() && !self.note_listening.get()) {
            return;
        }
        self.conversation_until.set(self.conversation_until.get().max(js::now_ms() + ms));
        self.sync_gate();
    }

    fn close_conversation(&self) {
        self.conversation_until.set(0);
        self.conversation_closed_at.set(js::now_ms());
        self.sync_gate();
    }

    /// Garde du moteur : ouverte pendant une conversation ou une attente de réponse / d'accord.
    fn sync_gate(&self) {
        let now = js::now_ms();
        let wakeword = *self.listening_mode.borrow() == "wakeword";
        let mut seconds = ((self.conversation_until.get() - now) as f64 / 1000.0).max(0.0);
        if self.meeting_on.get() && !self.note_listening.get() {
            seconds = 0.0;
        }
        if self.awaiting_reply() {
            seconds = seconds.max(60.0);
        }
        if (self.note_listening.get() || self.writing_to_terminal()) && wakeword {
            seconds = seconds.max(6.0 * 3600.0);
        }
        if seconds > 0.0 {
            self.send(json!({ "t": "gate", "open": true, "seconds": seconds.ceil() as i64 }));
        } else {
            self.send(json!({ "t": "gate", "open": false }));
        }
    }

    fn final_event(shown: &J, extra: J) -> J {
        let mut ev = json!({ "type": "voice_final" });
        js::assign(&mut ev, shown.clone());
        js::assign(&mut ev, extra);
        ev
    }

    /// Phrase finale du moteur. Rend true si elle est transmise à l'orchestrateur. Le moteur a mis Jarvis en pause
    /// pendant qu'on parlait : demande transmise → « stop » (il abandonne et répond), sinon il reprend.
    fn on_final(self: &Rc<Self>, raw_text: &str, meta: &J) -> bool {
        let sent = self.decide_final(raw_text, meta);
        if !sent {
            self.send(json!({ "t": "unduck" }));
        }
        sent
    }

    fn decide_final(self: &Rc<Self>, raw_text: &str, meta: &J) -> bool {
        let text = normalize_wake(raw_text.trim());
        if text.is_empty() {
            return false;
        }
        let echo = self.is_echo(&text);
        let has_wake = contains_wake(&text);
        let bare = bare_command(&text);
        // Empreinte vocale (mode strict) : « doubt » ou « rejected » = voix non reconnue.
        let check = meta["speakerCheck"].clone();
        let decision = js::gs(&check, "decision");
        let mut foreign = decision == "doubt" || decision == "rejected";
        // Voix presque reconnue + mot d'éveil : la demande passe quand même. La similarité est à moins de `nearMiss`
        // (12 points par défaut, réglable dans l'administration, 0 = jamais) sous le seuil, le profil le plus proche est
        // une vraie personne (pas un contact de visio) et aucun autre profil ne s'en approche. Les actions sensibles
        // gardent leur code de validation : une voix imitée ne peut rien faire de grave.
        let mut near_miss: Option<J> = None;
        if foreign && has_wake {
            let tol = self.orch.get_speaker_status()["nearMiss"].as_f64().unwrap_or(0.12);
            let sim = check["similarity"].as_f64().unwrap_or(0.0);
            let thr = check["threshold"].as_f64().unwrap_or(0.5);
            let cand = &check["candidate"];
            let rival = check["second"]["similarity"].as_f64().unwrap_or(0.0);
            if tol > 0.0 && cand.is_object() && js::gs(cand, "type") != "visio" && sim >= thr - tol && rival < sim - 0.1 {
                near_miss = Some(json!({ "id": cand["id"], "name": cand["name"], "similarity": sim, "nearMiss": true }));
                foreign = false;
                self.orch.record("voix_proche_acceptee", json!({ "texte": js::slice_chars(&text, 300), "similarite": sim, "seuil": thr, "personne": cand["name"] }));
            }
        }
        let mut shown = json!({ "text": text, "speaker": meta["speaker"], "speakerUnknown": meta["speakerUnknown"] });
        if js::truthy(&check) {
            shown["speakerCheck"] = check.clone();
        }

        // « Stop » : tout s'arrête, rien n'est demandé. Pas pour une voix inconnue (la télévision qui
        // dit « stop ») ; une voix douteuse, sans doute la tienne, oui.
        if !echo && stop_now(&bare) && decision != "rejected" {
            self.ui(Self::final_event(&shown, json!({ "accepted": true, "echo": false })));
            self.stop_from_voice();
            return false;
        }

        // Jarvis parlait (en pause) : son propre écho ou un simple « oui » ne l'arrête pas.
        // Sauf s'il attend justement une réponse ou un accord : alors « oui » est la réponse.
        let help_recent = self.orch.command_help.suggestion.borrow().as_ref().map(|(_, at)| js::now_ms() - at < 45000).unwrap_or(false);
        let expects_yes = self.awaiting_reply() || help_recent;
        let speaking = js::gb(&self.state.borrow(), "speaking");
        if speaking && (echo || (backchannel(&bare) && !expects_yes)) {
            self.ui(Self::final_event(&shown, json!({ "accepted": false, "echo": echo })));
            if !echo {
                self.open_conversation(CONVERSATION_MS);
            }
            return false;
        }

        let mode = self.listening_mode.borrow().clone();
        let mut accepted = !echo;
        if accepted && mode == "wakeword" && !has_wake && !self.awaiting_reply() && !self.in_conversation() {
            accepted = false;
        }
        if accepted && mode == "ptt" && !self.ptt_active.get() && js::now_ms() - self.ptt_released_at.get() > 1500 {
            accepted = false;
        }
        // Écoute libre : ce qui s'adresse visiblement à quelqu'un d'autre n'est pas une demande.
        let mut not_mine = None;
        if accepted && mode == "wakeword" && !self.note_listening.get() && !self.writing_to_terminal() {
            let replying = expects_yes
                || self.orch.pending_ai_confirm.borrow().is_some()
                || self.orch.waiting_reply.borrow().is_some()
                || js::now_ms() - self.last_question_at.get() < 20000;
            not_mine = not_for_jarvis(&text, has_wake, replying);
            if let Some(why) = not_mine {
                accepted = false;
                self.orch.record("demande_ignoree", json!({ "texte": js::slice_chars(&text, 300), "motif": format!("écoute libre : {why}") }));
            }
        }
        // Demande valable… d'une voix non reconnue : rien n'est exécuté. Terminal au premier plan (dictée
        // active) : une voix douteuse écrit quand même ; une voix inconnue aussi, mais n'envoie jamais Entrée.
        let terminal_doubt = accepted && foreign && self.writing_to_terminal();
        if accepted && foreign && !terminal_doubt {
            let st = self.orch.get_speaker_status();
            let mut extra = json!({ "accepted": false, "echo": false });
            if st["learning"] != json!(false) {
                extra["pendingId"] = json!(self.hold_for_speaker(&text, meta));
                extra["people"] = self.speaker_people(&check, &st);
                // On le dit une fois (30 s) : la phrase rouge reste cliquable (« c'est moi »), sinon on la répète.
                if has_wake && !speaking && js::now_ms() - self.unrecognized_hint_at.get() > 30_000 {
                    self.unrecognized_hint_at.set(js::now_ms());
                    self.say_local("Je n'ai pas reconnu ta voix. Touche la phrase en rouge pour confirmer, ou répète-la.");
                }
            }
            self.ui(Self::final_event(&shown, extra));
            self.orch.record(
                "voix_non_reconnue",
                json!({
                    "texte": js::slice_chars(&text, 300),
                    "verdict": if decision == "doubt" { "doute" } else { "inconnue" },
                    "similarite": check["similarity"],
                    "proche": if js::truthy(&check["candidate"]) { check["candidate"]["name"].clone() } else { Value::Null }
                }),
            );
            return false;
        }
        let mut extra = json!({ "accepted": accepted, "echo": echo });
        if let Some(why) = not_mine {
            extra["reason"] = json!(why);
        }
        self.ui(Self::final_event(&shown, extra));
        if !accepted {
            return false;
        }

        if mode == "wakeword" {
            if !has_wake && !self.note_listening.get() && !self.writing_to_terminal() && end_conversation(&text) {
                self.close_conversation();
                self.say_local("Très bien.");
                return false;
            }
            self.open_conversation(CONVERSATION_MS);
        }
        let mut cleaned = wake_prefix().replace(&text, "").trim().to_string();
        if cleaned.is_empty() {
            if !has_wake {
                return false;
            }
            cleaned = "Es-tu là ?".into();
        }
        // Une nouvelle demande remplace ce que Jarvis était en train de dire.
        self.send(json!({ "t": "stop" }));
        let mut m = json!({ "source": "voice", "wake": has_wake });
        if let Some(sp) = near_miss {
            m["speaker"] = sp; // voix presque reconnue : exécutée au nom de la personne la plus proche
            m["speakerNearMiss"] = json!(true);
        } else if js::truthy(&meta["speaker"]) {
            m["speaker"] = meta["speaker"].clone();
        }
        if terminal_doubt {
            m["speakerDoubt"] = json!(true);
            if decision == "rejected" {
                m["speakerRejected"] = json!(true);
            }
        }
        let orch = self.orch.clone();
        js::spawn(async move { orch.submit(cleaned, m).await });
        true
    }

    // --- Voix non reconnues : confirmation depuis le HUD ----------------------------------------

    fn hold_for_speaker(&self, text: &str, meta: &J) -> String {
        let now = js::now_ms();
        let mut p = self.speaker_pending.borrow_mut();
        p.retain(|(_, x)| now - x.at <= SPEAKER_KEEP_MS);
        while p.len() >= 12 {
            p.remove(0);
        }
        let id = format!("p{}", self.speaker_seq.get() + 1);
        self.speaker_seq.set(self.speaker_seq.get() + 1);
        p.push((id.clone(), SpeakerPending { text: text.to_string(), meta: meta.clone(), at: now }));
        id
    }

    /// Personnes enregistrées, pour les boutons : la plus probable d'abord, puis le profil principal.
    fn speaker_people(&self, check: &J, status: &J) -> J {
        let first = js::gs(&check["candidate"], "id");
        let second = js::gs(&check["second"], "id");
        let rank = |s: &J| {
            let id = js::gs(s, "id");
            if !first.is_empty() && id == first {
                0
            } else if !second.is_empty() && id == second {
                1
            } else if js::gb(s, "isPrimary") {
                2
            } else {
                3
            }
        };
        let mut list = js::arr(&status["speakers"]);
        list.sort_by_key(|s| rank(s));
        J::Array(
            list.iter()
                .take(6)
                .map(|s| {
                    let typ = Some(js::gs(s, "type")).filter(|t| !t.is_empty()).unwrap_or_else(|| if js::gb(s, "isPrimary") { "moi".into() } else { "autre".into() });
                    json!({ "id": s["id"], "name": s["name"], "type": typ })
                })
                .collect(),
        )
    }

    /// « C'est Johann » (who.id) ou « nouvelle personne » (who.create, who.name, who.type).
    pub fn confirm_speaker(self: &Rc<Self>, pending_id: &str, who: &J) -> J {
        let found = self.speaker_pending.borrow().iter().position(|(id, _)| id == pending_id);
        let Some(idx) = found else { return json!({ "ok": false, "error": "Phrase expirée : répète-la." }) };
        let (check, text, meta, at) = {
            let p = &self.speaker_pending.borrow()[idx].1;
            (p.meta["speakerCheck"].clone(), p.text.clone(), p.meta.clone(), p.at)
        };
        let sample = js::gs(&check, "sample");
        if sample.is_empty() {
            return json!({ "ok": false, "error": "Empreinte de cette phrase indisponible." });
        }
        if !self.authed.get() {
            return json!({ "ok": false, "error": "Moteur vocal indisponible." });
        }
        let person;
        if js::gb(who, "create") {
            let name = crate::re!(r"\s+").replace_all(&js::gs(who, "name"), " ").trim().to_string();
            let name = js::slice_chars(&name, 40);
            if !crate::re!(r"^\p{L}[\p{L}\p{M}' .-]*$").is_match(&name) {
                return json!({ "ok": false, "error": "Prénom invalide." });
            }
            let t = js::gs(who, "type");
            let typ = if SPEAKER_TYPES.contains(&t.as_str()) { t } else { "autre".into() };
            self.send(json!({ "t": "speaker_create", "sample": sample, "name": name, "type": typ }));
            person = json!({ "id": null, "name": name, "type": typ, "created": true });
        } else {
            let st = self.orch.get_speaker_status();
            let wid = js::gs(who, "id");
            let Some(found) = js::arr(&st["speakers"]).into_iter().find(|s| js::gs(s, "id") == wid) else {
                return json!({ "ok": false, "error": "Personne inconnue." });
            };
            self.send(json!({ "t": "speaker_learn", "sample": sample, "id": found["id"] }));
            let typ = Some(js::gs(&found, "type")).filter(|t| !t.is_empty()).unwrap_or_else(|| "autre".into());
            person = json!({ "id": found["id"], "name": found["name"], "type": typ });
        }
        self.speaker_pending.borrow_mut().remove(idx);
        self.ui(json!({ "type": "speaker_pending_done", "pendingId": pending_id }));
        self.learning.borrow_mut().insert(sample, (person.clone(), text.clone()));
        let mut executed = false;
        if js::now_ms() - at < SPEAKER_RUN_MS {
            // Même parcours qu'une phrase reconnue d'emblée, au nom de cette personne.
            let speaker = json!({ "id": person["id"], "name": person["name"], "similarity": check["similarity"], "confirmed": true });
            let mut c = check.clone();
            c["decision"] = json!("accepted");
            c["state"] = json!("known");
            c["confirmed"] = json!(true);
            let mut m = meta.clone();
            m["speaker"] = speaker;
            m["speakerUnknown"] = json!(false);
            m["speakerCheck"] = c;
            executed = self.on_final(&text, &m);
        }
        json!({ "ok": true, "executed": executed, "person": person })
    }

    pub fn dismiss_speaker(&self, pending_id: &str) -> J {
        let found = self.speaker_pending.borrow().iter().position(|(id, _)| id == pending_id);
        let Some(idx) = found else { return json!({ "ok": false }) };
        let (_, p) = self.speaker_pending.borrow_mut().remove(idx);
        self.orch.record("parole_parasite_ignoree", json!({ "pendingId": pending_id, "texte": js::slice_chars(&p.text, 200) }));
        self.ui(json!({ "type": "speaker_pending_done", "pendingId": pending_id }));
        json!({ "ok": true })
    }

    /// Réponse du moteur : phrase apprise (ou nouveau profil créé).
    fn on_speaker_learned(&self, msg: &J) {
        let sample = js::gs(msg, "sample");
        let ctx = self.learning.borrow_mut().remove(&sample);
        let spk = if js::truthy(&msg["speaker"]) {
            msg["speaker"].clone()
        } else {
            ctx.as_ref().map(|(p, _)| p.clone()).unwrap_or_else(|| json!({}))
        };
        let ok = js::truthy(&msg["ok"]);
        if ok {
            let mut rec = json!({ "id": spk["id"], "nom": spk["name"], "lien": spk["type"], "appris": msg["learned"] });
            if let Some((_, text)) = &ctx {
                rec["texte"] = json!(js::slice_chars(text, 200));
            }
            self.orch.record(if js::truthy(&msg["created"]) { "nouveau_locuteur" } else { "apprentissage_locuteur" }, rec);
            self.orch.publish_status();
        }
        self.ui(json!({
            "type": "speaker_learned", "ok": ok, "created": js::truthy(&msg["created"]), "speaker": spk,
            "learned": js::gn(msg, "learned").unwrap_or(0.0),
            "error": if js::truthy(&msg["error"]) { msg["error"].clone() } else { Value::Null },
            "message": if js::truthy(&msg["message"]) { msg["message"].clone() } else { Value::Null }
        }));
    }

    /// Parole locale du noyau (hors orchestrateur) : écrite dans l'interface et prononcée.
    pub fn say_local(&self, text: &str) {
        let cm = &self.orch.context_mute;
        let muted = cm.is_muted.get();
        let mut ev = json!({ "type": "say", "text": text, "local": true });
        if muted {
            ev["muted"] = json!(true);
            ev["muteReason"] = json!(*cm.reason.borrow());
        }
        self.ui(ev);
        if muted {
            return; // sourdine : écrit, pas dit
        }
        let spoken = self.spoken(text);
        self.remember_speech(&spoken);
        self.orch.pronunciation.remember(text, &spoken, &self.voice_id(), "");
        self.send(json!({ "t": "say", "text": spoken }));
    }

    /// Texte prononcé : l'écran garde l'orthographe, la voix reçoit la prononciation (règles de la voix choisie).
    fn spoken(&self, text: &str) -> String {
        self.orch.pronunciation.apply_for(text, &self.voice_id())
    }

    fn voice_id(&self) -> String {
        self.orch.status.d.borrow().voice.clone()
    }

    // --- Événements de l'orchestrateur destinés à la voix -------------------------------------
    // Appelé pour CHAQUE événement émis par l'orchestrateur, avant sa diffusion aux fenêtres.

    pub fn on_orchestrator_event(&self, ev: &J) {
        let typ = js::gs(ev, "type");
        let flag = |k: &str| js::truthy(&ev[k]);
        match typ.as_str() {
            "say" => {
                if flag("muted") || flag("local") {
                    return;
                }
                // Voix imposée (traduction dite dans sa langue) : le texte est dit tel quel, sans les règles de prononciation françaises.
                let forced = js::gs(ev, "voice");
                let spoken = if forced.is_empty() { self.spoken(&js::gs(ev, "text")) } else { js::gs(ev, "text") };
                self.remember_speech(&spoken);
                // Card Diction : les phrases de Jarvis, pas ses essais de prononciation ni une voix imposée.
                if forced.is_empty() && !flag("diction") {
                    self.orch.pronunciation.remember(&js::gs(ev, "text"), &spoken, &self.voice_id(), &js::gs(ev, "task"));
                }
                let mut m = json!({ "t": "say", "text": spoken });
                if !forced.is_empty() {
                    m["voice"] = json!(forced);
                }
                if flag("task") {
                    m["task"] = ev["task"].clone();
                }
                if flag("stream") {
                    m["stream"] = json!(true);
                }
                self.send(m);
            }
            "say_end" => {
                if flag("task") {
                    self.send(json!({ "t": "say_end", "task": ev["task"] }));
                }
            }
            "voice_stop" => self.send(json!({ "t": "stop" })),
            "context_mute" => self.visio_pause(flag("isMuted") && (flag("visio") || ev["activeApp"]["category"] == "visio")),
            "voice_tune" => self.send_spread("tune", &if ev["options"].is_object() { ev["options"].clone() } else { json!({}) }),
            "terminal_voice" => {
                self.terminal_listening.set(flag("on"));
                self.sync_gate();
            }
            "relay_voice" => {
                self.relay_listening.set(flag("on"));
                self.sync_gate();
            }
            "note_listen" => {
                self.note_listening.set(flag("on"));
                self.sync_gate();
            }
            "meeting_control" => {
                self.meeting_on.set(flag("on"));
                if self.meeting_on.get() {
                    self.conversation_until.set(0);
                }
                self.send(json!({ "t": "meeting", "on": self.meeting_on.get() }));
                self.sync_gate();
            }
            "meeting_name_voice" => {
                let typ = Some(js::gs(ev, "voiceType")).filter(|t| !t.is_empty()).unwrap_or_else(|| "visio".into());
                self.send(json!({ "t": "meeting_name_voice", "id": ev["id"], "name": ev["name"], "type": typ }));
            }
            "voice_command" => {
                let t = js::gs(&ev["payload"], "t");
                if ["corpus", "speaker_set_thresholds", "speaker_set_mode"].contains(&t.as_str()) {
                    self.send(ev["payload"].clone());
                }
            }
            "voice_config" => {
                // pitch / effect : voix propre au personnage du cœur (status.rs › speech()).
                let setting = json!({
                    "voice": ev["voice"], "rate": ev["rate"],
                    "pitch": Some(js::gs(ev, "pitch")).filter(|p| !p.is_empty()).unwrap_or_else(|| "+0Hz".into()),
                    "effect": Some(js::gs(ev, "effect")).filter(|p| !p.is_empty()).unwrap_or_else(|| "none".into())
                });
                *self.voice_setting.borrow_mut() = Some(setting.clone());
                let mut m = setting;
                m["sample"] = json!(flag("sample"));
                if flag("sampleText") {
                    m["sample_text"] = ev["sampleText"].clone();
                }
                self.send_spread("voice", &m);
            }
            "status" => {
                let lm = js::gs(ev, "listeningMode");
                if !lm.is_empty() && lm != *self.listening_mode.borrow() {
                    *self.listening_mode.borrow_mut() = lm.clone();
                    self.send(json!({ "t": "mode", "mode": if lm == "wakeword" { "wakeword" } else { "continuous" } }));
                    self.sync_gate();
                }
                // Voix réellement utilisée (celle du personnage du cœur s'il en a une), sinon celle des réglages.
                let sp = if js::truthy(&ev["speech"]) {
                    Some(ev["speech"].clone())
                } else if js::truthy(&ev["voice"]) {
                    Some(json!({ "voice": ev["voice"], "rate": ev["voiceRate"] }))
                } else {
                    None
                };
                if let Some(sp) = sp {
                    let want = json!({
                        "voice": sp["voice"], "rate": sp["rate"],
                        "pitch": Some(js::gs(&sp, "pitch")).filter(|p| !p.is_empty()).unwrap_or_else(|| "+0Hz".into()),
                        "effect": Some(js::gs(&sp, "effect")).filter(|p| !p.is_empty()).unwrap_or_else(|| "none".into())
                    });
                    let differs = match self.voice_setting.borrow().as_ref() {
                        None => true,
                        Some(cur) => ["voice", "rate", "pitch", "effect"].iter().any(|k| cur[*k] != want[*k]),
                    };
                    if differs {
                        *self.voice_setting.borrow_mut() = Some(want.clone());
                        let mut m = want;
                        m["sample"] = json!(false);
                        self.send_spread("voice", &m);
                    }
                }
            }
            "vocabulary" => {
                let v = json!({
                    "terms": if ev["terms"].is_array() { ev["terms"].clone() } else { json!([]) },
                    "corrections": if ev["corrections"].is_array() { ev["corrections"].clone() } else { json!([]) }
                });
                *self.vocabulary.borrow_mut() = Some(v.clone());
                self.send_spread("vocab", &v);
            }
            "audio_device" => {
                let mode = js::gs(ev, "mode");
                let cur = self.audio_mode.borrow().clone();
                if let Some(c) = cur {
                    if !mode.is_empty() && mode != c {
                        self.send(json!({ "t": "reopen_audio" }));
                    }
                }
                if !mode.is_empty() {
                    *self.audio_mode.borrow_mut() = Some(mode);
                }
            }
            "media" => {
                // Mesure du son du PC seulement pendant la lecture (animation de la note du cœur).
                let playing = flag("active") && js::gs(ev, "status") == "playing";
                if playing != self.music_playing.get() {
                    self.music_playing.set(playing);
                    self.send(json!({ "t": "pc_meter", "on": playing }));
                }
            }
            "speaker_enroll_start" => {
                let mut m = json!({
                    "t": "speaker_enroll_start",
                    "name": Some(js::gs(ev, "name")).filter(|n| !n.is_empty()).unwrap_or_else(|| "Johann".into()),
                    "count": js::gn(ev, "count").filter(|c| *c > 0.0).unwrap_or(8.0) as i64
                });
                if flag("speakerType") {
                    m["type"] = ev["speakerType"].clone();
                }
                self.send(m);
            }
            "speaker_enroll_cancel" => self.send(json!({ "t": "speaker_enroll_cancel" })),
            "speaker_delete" => self.send(json!({ "t": "speaker_delete", "id": ev["id"] })),
            "speaker_rename" => self.send(json!({ "t": "speaker_rename", "id": ev["id"], "name": ev["name"] })),
            "speaker_primary" => self.send(json!({ "t": "speaker_primary", "id": ev["id"] })),
            "speaker_set_enabled" => self.send(json!({ "t": "speaker_set_enabled", "enabled": flag("enabled") })),
            "speaker_set_threshold" => self.send(json!({ "t": "speaker_set_threshold", "threshold": js::num(&ev["threshold"]).unwrap_or(f64::NAN) })),
            "speaker_reset" => self.send(json!({ "t": "speaker_reset" })),
            "speaker_set_type" => self.send(json!({ "t": "speaker_set_type", "id": ev["id"], "type": ev["speakerType"] })),
            "speaker_set_options" => self.send(json!({ "t": "speaker_set_options", "doubtMargin": ev["doubtMargin"], "autoLearn": ev["autoLearn"], "learning": ev["learning"] })),
            // Suivi des tâches : une validation ou une question en attente garde l'écoute ouverte.
            "create" => {
                let id = js::s(&ev["id"]);
                let status = Some(js::gs(ev, "status")).filter(|s| !s.is_empty()).unwrap_or_else(|| "running".into());
                let conversational = crate::re!(r"^c\d+$").is_match(&id);
                self.pending.borrow_mut().insert(id, TaskState { status, background: flag("background"), conversational });
                self.sync_gate();
            }
            "update" => {
                let id = js::s(&ev["id"]);
                let status = js::gs(ev, "status");
                let changed = match self.pending.borrow_mut().get_mut(&id) {
                    Some(t) if !status.is_empty() => {
                        t.status = status;
                        true
                    }
                    _ => false,
                };
                if changed {
                    self.sync_gate();
                }
            }
            "finish" => {
                self.pending.borrow_mut().remove(&js::s(&ev["id"]));
                self.sync_gate();
            }
            _ => {}
        }
    }

    /// Appui-pour-parler (mode « ptt ») : état de la touche, envoyé par la fenêtre.
    pub fn set_ptt(&self, active: bool) {
        self.ptt_active.set(active);
        if !active {
            self.ptt_released_at.set(js::now_ms());
        }
    }

    pub fn stop(&self) {
        self.retry.clear();
        self.conn_seq.set(self.conn_seq.get() + 1);
        if let Some(tx) = self.tx.borrow_mut().take() {
            let _ = tx.send(Message::Close(None));
        }
    }
}

#[cfg(test)]
mod tests_wake_google {
    use super::*;

    #[test]
    fn ok_google_est_entendu_comme_ok_jarvis() {
        assert_eq!(normalize_wake("Ok Google, arrête la musique."), "Ok Jarvis, arrête la musique.");
        assert_eq!(normalize_wake("Hey Google mets du jazz"), "Hey Jarvis mets du jazz");
        assert_eq!(normalize_wake("okay google stop"), "okay Jarvis stop");
    }

    #[test]
    fn une_vraie_recherche_google_reste_inchangee() {
        assert_eq!(normalize_wake("Google météo demain"), "Google météo demain");
        assert_eq!(normalize_wake("cherche sur Google des recettes"), "cherche sur Google des recettes");
    }
}
