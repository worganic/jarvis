// Réunions enregistrées (orchestrator/meetings.js) : transcription en direct, voix des participants,
// archive consultable. « Jarvis, enregistre la réunion » (ou la bannière quand une page Google Meet
// s'ouvre) : le moteur vocal transcrit le micro (« moi ») et le son du PC (la visio). Une voix inconnue
// nommée dans la card est apprise et ses passages réattribués. À la fin, la réunion est archivée dans
// data/meetings/ (JSON) et indexée par le sens (mêmes embeddings que la mémoire).

use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};
use std::rc::Rc;

use serde_json::{json, Map, Value};

use super::automations::base36;
use super::embeddings::cosine;
use super::intent::Intent;
use super::notes::{long_date, meeting_start_re, meeting_stop_re, spoken_topic};
use super::{OrchRef, Up};
use crate::js::{self, Timer, J};

const OFFER_TITLE: &str = "Réunion Google Meet détectée";
const MEETING: &str = r"(?:reunion|reunions|visio|visioconference|conference|appel|call|meeting|meet|gmeet|gmit|g\s+mit|gemit|entretien)";
const NUM: &str = r"(\d{1,3}|un|une|premiere|premier|deux|trois|quatre|cinq|six|sept|huit|neuf|dix)";

fn meet_url_re() -> &'static regex::Regex {
    crate::re!(r"(?i)^https://meet\.google\.com/([a-z]{3,4}-[a-z]{4}-[a-z]{3,4})(?:[/?#]|$)")
}
fn meet_window_re() -> &'static regex::Regex {
    crate::re!(r"(?i)^Meet\s*[-–—:]\s*(.+?)\s*(?:[-–—]\s*(?:Google Chrome|Microsoft Edge|Brave|Opera|Vivaldi|Chromium|Mozilla Firefox)\s*)?$")
}
fn meet_code(s: &str) -> bool {
    crate::re!(r"(?i)^[a-z]{3,4}-[a-z]{4}-[a-z]{3,4}$").is_match(s)
}

macro_rules! lazy_re {
    ($name:ident, $build:expr) => {
        fn $name() -> &'static regex::Regex {
            static R: std::sync::OnceLock<regex::Regex> = std::sync::OnceLock::new();
            R.get_or_init(|| regex::Regex::new(&$build).unwrap())
        }
    };
}

lazy_re!(list_re, format!("^(?:(?:liste|affiche|montre|donne|ouvre|lance|active|quelles sont)(?: moi)? (?:(?:la liste (?:de |des )?)?(?:mes|les|toutes les|toutes mes|nos) {MEETING}(?: enregistrees| archivees| passees)?|(?:la |le )?(?:card|cards|carte|cartes|carde|cardes|module|modules|widget) (?:de |des |d |mes )?{MEETING}(?: \\d{{1,3}})?)|liste des {MEETING}|mes {MEETING})$"));
lazy_re!(open_re, format!("^(?:ouvre|affiche|montre|ressors|ressort|rouvre|remontre|reprends|retrouve|charge)(?: moi)? (?:la |le |l )?(?:derniere |precedente )?{MEETING}(?: (?:numero |n |no )?{NUM}| (?:avec|de|sur|du|des|d|concernant|a propos de|au sujet de) (.+)| derniere| precedente)?$"));
lazy_re!(summary_re, format!("^(?:(?:fais|fait|donne|ecris|redige|genere|prepare|faire|ressors|ressort|sors)(?: moi)? (?:un |le |une )?(?:resume|compte rendu|synthese)|resume(?: moi)?)(?: (?:de |du |des |d )?(?:la |cette |ma |notre |l |le )?(?:derniere |precedente )?{MEETING}(?: (?:numero |n |no )?{NUM}| (?:avec|de|sur|du|d) (.+)| derniere| precedente)?)$"));
lazy_re!(gdoc_re, format!("^(?:(?:mets|met|mettre|enregistre|sauvegarde|exporte|envoie|ecris|copie|cree|transfere)(?: moi)? (?:la |cette |le |ce |l )?(?:derniere )?(?:{MEETING}|compte rendu|transcription|resume)(?: (?:de |du |d )?(?:la |cette |l )?(?:derniere )?{MEETING})?(?: (?:numero |n |no )?{NUM})? (?:dans|sur|en|vers) (?:un |une |le |mon |google )*(?:google )?(?:docs?|document|drive)(?: google)?|(?:cree|fais|fait|genere)(?: moi)? un (?:google )?doc(?:ument)?(?: google)? (?:de|du|avec) (?:la |le |l )?(?:derniere )?(?:{MEETING}|compte rendu))$"));
lazy_re!(meeting_word_re, format!(r"\b{MEETING}\b"));

fn list_gmeet_re() -> &'static regex::Regex {
    crate::re!(r"(?i)^(?:(?:affiche|montre|ouvre|donne)(?: moi)? )?(?:la )?(?:card )?(?:liste|historique|enregistrements?)(?: des?)? (?:gmeet|google meet|meet|g\s+mit|gmit|gemit)|(?:liste|card) (?:gmeet|gmit|g\s+mit|gemit)$")
}
fn name_voice_re() -> &'static regex::Regex {
    crate::re!(r"^(?:la )?voix(?: inconnue)?(?: numero| n| no)? (\d{1,2}|un|une|deux|trois|quatre|cinq|six|sept|huit|neuf|dix)(?:,)? (?:c est|c etait|s appelle|est|appartient a) (.{2,40})$")
}

fn clock(ms: i64) -> String {
    js::local_of(ms).format("%H:%M").to_string()
}
fn clock_s(ms: i64) -> String {
    js::local_of(ms).format("%H:%M:%S").to_string()
}
fn short_date(ms: i64) -> String {
    js::local_of(ms).format("%d/%m/%Y à %H:%M").to_string()
}
fn fold(s: &str) -> String {
    let t = js::fold_lower(s);
    crate::re!(r"[^a-z0-9]+").replace_all(&t, " ").trim().to_string()
}
fn esc(s: &str) -> String {
    s.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}
fn number_of(s: &str) -> Option<i64> {
    if !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit()) {
        return s.parse().ok();
    }
    Some(match s {
        "un" | "une" | "premiere" | "premier" => 1,
        "deux" => 2,
        "trois" => 3,
        "quatre" => 4,
        "cinq" => 5,
        "six" => 6,
        "sept" => 7,
        "huit" => 8,
        "neuf" => 9,
        "dix" => 10,
        _ => return None,
    })
}
fn slug(text: &str) -> String {
    let t = js::slice_chars(&fold(text).replace(' ', "-"), 50);
    if t.is_empty() { "reunion".into() } else { t }
}
pub fn minutes_of(ms: i64) -> String {
    let m = ((ms as f64) / 60000.0).round().max(1.0) as i64;
    if m < 60 { format!("{m} min") } else { format!("{} h {:02}", m / 60, m % 60) }
}
fn brief_of(summary: &str) -> String {
    crate::re!(r"(?im)^En bref\s*:\s*(.+)$").captures(summary).map(|m| m[1].trim().to_string()).unwrap_or_default()
}

/// Réunion désignée : numéro, mots (participant, titre, date), ou la dernière.
#[derive(Clone, Default)]
pub struct Ref {
    pub n: Option<i64>,
    pub words: String,
    pub last: bool,
}

pub struct Meetings {
    o: OrchRef,
    pub dir: PathBuf,
    log: Rc<dyn Fn(String)>,
    auto_summarize: bool,
    /// Réunion affichée (en cours ou rouverte) : objet JSON (id, title, titleSource, created, ended,
    /// author, meet, participants, voices{}, lines[], recording, pending, engine, summary, doc, file, dirty).
    pub session: RefCell<Option<J>>,
    engine_session: RefCell<String>,
    engine_timer: Timer,
    done_timer: Timer,
    meet_poll: Timer,
    seq: Cell<u64>,
    offered: RefCell<HashSet<String>>,
    offer_code: RefCell<Option<String>>,
    offer_gone_at: Cell<i64>,
    meet_gone_at: Cell<i64>,
    last_list: RefCell<Vec<J>>,
    busy: RefCell<Vec<String>>,
    vectors_file: PathBuf,
    vectors: RefCell<Option<Map<String, Value>>>,
}

impl Meetings {
    pub fn new(o: OrchRef, dir: PathBuf, log: Rc<dyn Fn(String)>, auto_summarize: bool) -> Rc<Self> {
        let vectors_file = dir.join("vectors.json");
        Rc::new(Self {
            o,
            dir,
            log,
            auto_summarize,
            session: RefCell::new(None),
            engine_session: RefCell::new(String::new()),
            engine_timer: Timer::new(),
            done_timer: Timer::new(),
            meet_poll: Timer::new(),
            seq: Cell::new(0),
            offered: RefCell::new(HashSet::new()),
            offer_code: RefCell::new(None),
            offer_gone_at: Cell::new(0),
            meet_gone_at: Cell::new(0),
            last_list: RefCell::new(vec![]),
            busy: RefCell::new(vec![]),
            vectors_file,
            vectors: RefCell::new(None),
        })
    }

    pub fn recording(&self) -> bool {
        self.session.borrow().as_ref().map(|s| js::gb(s, "recording")).unwrap_or(false)
    }

    fn sid(&self) -> Option<String> {
        self.session.borrow().as_ref().map(|s| js::gs(s, "id"))
    }

    fn is_current(&self, s: &J) -> bool {
        self.sid().as_deref() == Some(js::gs(s, "id").as_str())
    }

    /// Modifie la réunion affichée.
    fn with<R>(&self, f: impl FnOnce(&mut J) -> R) -> Option<R> {
        self.session.borrow_mut().as_mut().map(f)
    }

    /// Remet à jour la réunion affichée si c'est celle-ci.
    fn sync(&self, s: &J) {
        if self.is_current(s) {
            *self.session.borrow_mut() = Some(s.clone());
        }
    }

    // --- Veille de Google Meet --------------------------------------------------------------------

    pub fn watch(self: &Rc<Self>) {
        let o = self.o.up();
        let me = Rc::downgrade(self);
        o.browser_bridge.on("change", move |_| {
            if let Some(m) = me.upgrade() {
                m.watch_meet();
            }
        });
        let me = Rc::downgrade(self);
        o.desktop_state.on("change", move |_| {
            if let Some(m) = me.upgrade() {
                m.watch_meet();
            }
        });
        let me = Rc::downgrade(self);
        js::set_timeout(4000, move || {
            if let Some(m) = me.upgrade() {
                m.watch_meet();
            }
        });
    }

    /// Pages Meet ouvertes : { code, url, title, tabId }.
    /// Premier onglet Google Meet ouvert (sourdine contextuelle).
    pub fn first_meet_tab(&self) -> Option<J> {
        self.meet_tabs().into_iter().next()
    }

    fn meet_tabs(&self) -> Vec<J> {
        let o = self.o.up();
        let mut out = vec![];
        if o.browser_bridge.connected() {
            for t in o.browser_bridge.all_tabs() {
                let url = js::gs(&t, "url");
                if let Some(m) = meet_url_re().captures(&url) {
                    out.push(json!({ "code": m[1].to_lowercase(), "url": t["url"], "title": js::gs(&t, "title"), "tabId": t["id"] }));
                }
            }
            return out;
        }
        if o.desktop_state.ready() {
            for w in o.desktop_state.windows() {
                let title = js::gs(&w, "title");
                let Some(m) = meet_window_re().captures(&title) else { continue };
                let name = m[1].trim().to_string();
                let is_code = meet_code(&name);
                let code = if is_code { name.to_lowercase() } else { format!("nom:{}", fold(&name)) };
                out.push(json!({ "code": code, "url": if is_code { json!(format!("https://meet.google.com/{code}")) } else { Value::Null }, "title": format!("Meet - {name}"), "tabId": null }));
            }
        }
        out
    }

    fn watch_meet(self: &Rc<Self>) {
        let tabs = self.meet_tabs();
        self.watch_offer(&tabs);
        let s = self.session.borrow().clone();
        let o = self.o.up();
        if let Some(s) = s.as_ref().filter(|s| js::gb(s, "recording")) {
            if s["meet"].is_object() {
                // La page fermée depuis 10 s arrête l'enregistrement (seulement avec l'extension Chrome).
                if !o.browser_bridge.connected() {
                    return;
                }
                let code = js::gs(&s["meet"], "code");
                if tabs.iter().any(|t| js::gs(t, "code") == code) {
                    self.meet_gone_at.set(0);
                } else if self.meet_gone_at.get() == 0 {
                    self.meet_gone_at.set(js::now_ms());
                    let me = Rc::downgrade(self);
                    let id = js::gs(s, "id");
                    js::set_timeout(10000, move || {
                        let Some(m) = me.upgrade() else { return };
                        let still = m.session.borrow().as_ref().map(|c| js::gs(c, "id") == id && js::gb(c, "recording")).unwrap_or(false);
                        if still && m.meet_gone_at.get() != 0 && js::now_ms() - m.meet_gone_at.get() >= 9950 && !m.meet_tabs().iter().any(|t| js::gs(t, "code") == code) {
                            let r = m.stop("meet");
                            m.o.up().emit(json!({ "type": "say", "text": format!("La page Google Meet est fermée : {}", js::gs(&r, "say")) }));
                        }
                    });
                }
            }
            return;
        }
        let fresh = tabs.into_iter().find(|t| !self.offered.borrow().contains(&js::gs(t, "code")));
        if let Some(t) = fresh {
            let me = self.clone();
            js::spawn(async move { me.offer(t).await });
        }
    }

    fn watch_offer(self: &Rc<Self>, tabs: &[J]) {
        let Some(code) = self.offer_code.borrow().clone() else { return };
        if tabs.iter().any(|t| js::gs(t, "code") == code) {
            self.offer_gone_at.set(0);
            return;
        }
        if self.offer_gone_at.get() != 0 {
            return;
        }
        self.offer_gone_at.set(js::now_ms());
        let me = Rc::downgrade(self);
        js::set_timeout(3000, move || {
            let Some(m) = me.upgrade() else { return };
            if m.offer_code.borrow().as_deref() != Some(code.as_str()) || m.offer_gone_at.get() == 0 || m.meet_tabs().iter().any(|t| js::gs(t, "code") == code) {
                return;
            }
            m.close_offer();
            m.offered.borrow_mut().remove(&code);
        });
    }

    fn close_offer(&self) {
        *self.offer_code.borrow_mut() = None;
        self.offer_gone_at.set(0);
        let o = self.o.up();
        let empty = self.session.borrow().as_ref().map(|s| !js::gb(s, "recording") && js::arr(&s["lines"]).is_empty()).unwrap_or(false);
        if empty {
            o.emit(json!({ "type": "widget_close", "key": "meeting" }));
            *self.session.borrow_mut() = None;
        }
        o.emit(json!({ "type": "widget_close", "key": format!("tool:{OFFER_TITLE}") }));
    }

    /// Bannière discrète et ouverture directe de la card GMeet.
    async fn offer(&self, tab: J) {
        let code = js::gs(&tab, "code");
        self.offered.borrow_mut().insert(code.clone());
        *self.offer_code.borrow_mut() = Some(code.clone());
        self.offer_gone_at.set(0);
        let info = self.meet_info(&tab).await;
        let from_tab = Self::title_from_tab(&tab);
        let title = info.as_ref().map(|i| js::gs(i, "title")).filter(|t| !t.is_empty()).or_else(|| Some(from_tab).filter(|t| !t.is_empty())).unwrap_or_else(|| format!("Google Meet {code}"));
        let o = self.o.up();
        o.record("reunion_detectee", json!({ "code": code, "titre": title }));
        let call = |action: &str| json!({ "method": "meetingAction", "args": ["", action, { "code": code }] });
        let participants: Vec<String> = info.as_ref().map(|i| js::str_list(&i["participants"])).unwrap_or_default();
        let sub = if !participants.is_empty() {
            format!("{} participant(s) : {}", participants.len(), participants.iter().take(4).cloned().collect::<Vec<_>>().join(", "))
        } else if meet_code(&code) {
            format!("meet.google.com/{code}")
        } else {
            "Google Meet".into()
        };
        o.show_widget(
            "tool",
            json!({
                "title": OFFER_TITLE, "tool": "meet", "ttl": 180000, "count": 1,
                "rows": [{
                    "icon": "◉", "title": title, "sub": sub, "meta": "enregistrer ?", "hint": "Clic : enregistrer et transcrire la réunion", "call": call("start_meet"),
                    "actions": [
                        { "icon": "◉", "title": "Enregistrer la réunion (transcription, voix des participants)", "call": call("start_meet") },
                        { "icon": "✕", "title": "Ignorer", "call": call("dismiss_offer") }
                    ]
                }],
                "footer": "Clic : enregistrer · ou dis « Jarvis, enregistre la réunion »"
            }),
        );
        if !self.recording() {
            let mut s = self.new_session(&title, Some(json!({ "code": code, "url": tab["url"] })));
            let names: Vec<String> = participants.iter().map(|n| n.trim().to_string()).filter(|n| !n.is_empty()).collect();
            if !names.is_empty() {
                s["participants"] = json!(names);
            }
            *self.session.borrow_mut() = Some(s);
            self.show();
        }
    }

    fn title_from_tab(tab: &J) -> String {
        let t = crate::re!(r"(?i)\s*[-–—]\s*(?:Google Chrome|Microsoft Edge)$").replace(&js::gs(tab, "title"), "").trim().to_string();
        let name = crate::re!(r"(?i)^Meet\s*[-–—:]\s*(.+)$").captures(&t).map(|m| m[1].trim().to_string()).unwrap_or_default();
        if !name.is_empty() && !meet_code(&name) { name } else { String::new() }
    }

    async fn meet_info(&self, tab: &J) -> Option<J> {
        let o = self.o.up();
        if !js::truthy(&tab["tabId"]) || !o.browser_bridge.connected() {
            return None;
        }
        let r = o.browser_bridge.request("meet", json!({ "tabId": tab["tabId"] }), 5000).await;
        js::gb(&r, "ok").then_some(r)
    }

    /// Titre et participants de la page Meet, pendant l'enregistrement.
    async fn refresh_meet(&self) {
        let Some(s) = self.session.borrow().clone() else { return };
        if !js::gb(&s, "recording") || !s["meet"].is_object() {
            return;
        }
        let code = js::gs(&s["meet"], "code");
        let Some(tab) = self.meet_tabs().into_iter().find(|t| js::gs(t, "code") == code) else { return };
        let Some(info) = self.meet_info(&tab).await else { return };
        if !self.is_current(&s) {
            return;
        }
        let changed = self
            .with(|s| {
                let mut changed = false;
                let t = js::gs(&info, "title");
                if !t.is_empty() && js::gs(s, "titleSource") != "user" && t != js::gs(s, "title") {
                    s["title"] = json!(t);
                    s["titleSource"] = json!("meet");
                    changed = true;
                }
                let mut parts = js::str_list(&s["participants"]);
                for n in js::str_list(&info["participants"]).into_iter().map(|n| n.trim().to_string()).filter(|n| !n.is_empty()) {
                    if !parts.contains(&n) {
                        parts.push(n);
                        changed = true;
                    }
                }
                s["participants"] = json!(parts);
                changed
            })
            .unwrap_or(false);
        if changed {
            let s = self.session.borrow().clone().unwrap();
            self.publish(json!({ "title": s["title"] }));
            self.publish_people(&s);
        }
    }

    // --- Séance -----------------------------------------------------------------------------------

    fn author(&self) -> String {
        self.o.up().notes.author()
    }

    fn new_session(&self, title: &str, meet: Option<J>) -> J {
        let created = js::now_ms();
        self.seq.set(self.seq.get() + 1);
        json!({
            "id": format!("reu{}-{}", self.seq.get(), base36(created as u64)),
            "title": if title.is_empty() { format!("Réunion du {}", short_date(created)) } else { title.to_string() },
            "titleSource": if title.is_empty() { "auto" } else { "user" },
            "created": created, "ended": null, "author": self.author(),
            "meet": meet, "participants": [], "voices": {}, "lines": [],
            "recording": false, "pending": false, "engine": null,
            "summary": "", "doc": null, "file": null, "dirty": false
        })
    }

    pub async fn start(&self, p: &J) -> J {
        let (title, code) = (js::gs(p, "title"), js::gs(p, "code"));
        if self.recording() {
            return json!({ "ok": false, "say": "La réunion est déjà en cours d’enregistrement." });
        }
        let o = self.o.up();
        if o.notes.listening.get() {
            o.notes.set_listening(false);
        }
        let tabs = self.meet_tabs();
        let cur = self.session.borrow().clone();
        let cur_code = cur.as_ref().filter(|s| s["meet"].is_object()).map(|s| js::gs(&s["meet"], "code"));
        let tab = (!code.is_empty()).then(|| tabs.iter().find(|t| js::gs(t, "code") == code).cloned()).flatten()
            .or_else(|| cur_code.as_ref().and_then(|c| tabs.iter().find(|t| js::gs(t, "code") == *c).cloned()))
            .or_else(|| if code.is_empty() { tabs.first().cloned() } else { None });
        let meet = tab.as_ref().map(|t| json!({ "code": t["code"], "url": t["url"] }));
        if let Some(t) = &tab {
            self.offered.borrow_mut().insert(js::gs(t, "code"));
        }
        let reuse = match (&cur, &meet) {
            (Some(s), Some(m)) => js::arr(&s["lines"]).is_empty() && !js::truthy(&s["ended"]) && (!s["meet"].is_object() || s["meet"]["code"] == m["code"]),
            _ => false,
        };
        let mut s = if reuse {
            let mut s = cur.unwrap();
            if !title.is_empty() {
                s["title"] = json!(title);
                s["titleSource"] = json!("user");
            }
            if let Some(m) = &meet {
                s["meet"] = m.clone();
            }
            s
        } else {
            self.new_session(&title, meet.clone())
        };
        *self.engine_session.borrow_mut() = js::gs(&s, "id");
        s["recording"] = json!(true);
        s["pending"] = json!(true);
        self.meet_gone_at.set(0);
        if let Some(t) = &tab {
            if title.is_empty() && js::gs(&s, "titleSource") != "user" {
                let ft = Self::title_from_tab(t);
                if !ft.is_empty() {
                    s["title"] = json!(ft);
                    s["titleSource"] = json!("meet");
                }
            }
        }
        *self.session.borrow_mut() = Some(s.clone());
        self.close_offer();
        o.emit(json!({ "type": "meeting_control", "on": true }));
        o.record("reunion_debut", json!({ "titre": s["title"], "meet": meet.as_ref().map(|m| m["code"].clone()) }));
        self.show();
        let oc = self.o.clone();
        let id = js::gs(&s, "id");
        self.engine_timer.set(8000, move || {
            let orch = oc.up();
            let m = &orch.meetings;
            let stuck = m.session.borrow().as_ref().map(|c| js::gs(c, "id") == id && js::gb(c, "recording") && js::gb(c, "pending")).unwrap_or(false);
            if stuck {
                m.with(|c| c["recording"] = json!(false));
                orch.emit(json!({ "type": "meeting_control", "on": false }));
                m.publish(json!({ "recording": false }));
                orch.emit(json!({ "type": "say", "text": "Le moteur vocal ne répond pas : je ne peux pas enregistrer la réunion. Vérifie que l'écoute est active." }));
            }
        });
        self.meet_poll.clear();
        if meet.is_some() {
            let oc = self.o.clone();
            js::spawn(async move { oc.up().meetings.refresh_meet().await });
            let oc = self.o.clone();
            self.meet_poll.every(20000, move || {
                let oc = oc.clone();
                js::spawn(async move { oc.up().meetings.refresh_meet().await });
            });
        }
        json!({ "ok": true, "say": format!("J’enregistre la réunion{} : ta voix et celles des participants sont transcrites. Dis « Jarvis, arrête l’enregistrement » à la fin.", if meet.is_some() { " Google Meet" } else { "" }) })
    }

    pub fn stop(&self, reason: &str) -> J {
        if !self.recording() {
            return json!({ "ok": false, "say": "Aucune réunion n’est en cours d’enregistrement." });
        }
        let now = js::now_ms();
        let s = self
            .with(|s| {
                s["recording"] = json!(false);
                s["ended"] = json!(now);
                s.clone()
            })
            .unwrap();
        self.engine_timer.clear();
        self.meet_poll.clear();
        let o = self.o.up();
        let created = js::gn_or(&s, "created", 0.0) as i64;
        let lines = js::arr(&s["lines"]).len();
        o.emit(json!({ "type": "meeting_control", "on": false }));
        o.record("reunion_fin", json!({ "titre": s["title"], "duree": minutes_of(now - created), "phrases": lines, "origine": reason }));
        self.publish(json!({ "recording": false, "ended": now }));
        // Les derniers morceaux sont encore transcrits : archivage à « meeting_done » (20 s au plus).
        let oc = self.o.clone();
        let id = js::gs(&s, "id");
        self.done_timer.set(20000, move || {
            let oc2 = oc.clone();
            let id = id.clone();
            js::spawn(async move {
                let orch = oc2.up();
                let s = orch.meetings.session.borrow().clone().filter(|s| js::gs(s, "id") == id);
                if let Some(s) = s {
                    orch.meetings.finish(s).await;
                }
            });
        });
        let unknown = s["voices"].as_object().map(|v| v.values().filter(|x| !js::truthy(&x["person"])).count()).unwrap_or(0);
        json!({
            "ok": true,
            "say": format!(
                "Enregistrement arrêté après {}. {}{}.",
                minutes_of(now - created),
                if lines > 0 { "La réunion est archivée" } else { "Rien n’a encore été transcrit" },
                if unknown > 0 { format!(". {unknown} voix restent à nommer dans la card") } else { String::new() }
            )
        })
    }

    async fn finish(&self, s: J) {
        self.done_timer.clear();
        if js::gb(&s, "recording") {
            return;
        }
        let mut s = s;
        self.save(&mut s);
        if !js::arr(&s["lines"]).is_empty() {
            if self.auto_summarize && js::gs(&s, "summary").is_empty() && !self.busy.borrow().contains(&"summary".to_string()) {
                self.summarize(&mut s).await;
            }
            self.ingest_rag(&mut s);
        }
    }

    fn ingest_rag(&self, s: &mut J) {
        if js::arr(&s["lines"]).is_empty() {
            return;
        }
        self.save(s);
        let o = self.o.up();
        let people: Vec<String> = self.people(s).iter().filter(|p| js::gs(p, "kind") != "unknown").map(|p| js::gs(p, "name")).collect();
        let created = js::gn_or(s, "created", 0.0) as i64;
        let date = long_date(created);
        let duree = s["ended"].as_i64().map(|e| minutes_of(e - created)).unwrap_or_default();
        let title = js::gs(s, "title");
        let summary = js::gs(s, "summary");
        let entry = format!(
            "Réunion « {title} » du {date}{}.{}{}",
            if duree.is_empty() { String::new() } else { format!(" ({duree})") },
            if people.is_empty() { String::new() } else { format!(" Participants : {}.", people.join(", ")) },
            if summary.is_empty() { String::new() } else { format!("\nRésumé : {summary}") }
        );
        let prefix = format!("Réunion « {title} » du {date}");
        let existing = o.memory.list().into_iter().find(|e| js::gs(e, "category") == "reunion" && js::gs(e, "text").starts_with(&prefix));
        match existing {
            Some(e) => {
                o.memory.update(&js::gs(&e, "id"), Some(&entry), Some("reunion"));
            }
            None => {
                o.memory.add(&entry, "reunion");
            }
        }
        (self.log)(format!("réunions : réunion « {title} » enregistrée dans le RAG"));
    }

    // --- Moteur vocal -------------------------------------------------------------------------------

    pub fn on_engine(self: &Rc<Self>, msg: &J) {
        let t = js::gs(msg, "t");
        let o = self.o.up();
        let Some(s) = self.session.borrow().clone() else { return };
        match t.as_str() {
            "meeting_state" => {
                if js::gb(msg, "on") && js::gb(&s, "recording") {
                    self.engine_timer.clear();
                    let engine = json!({
                        "config": js::gs(msg, "config"), "micro": js::gs(msg, "micro"), "pc": js::gs(msg, "pc"),
                        "aec": js::gb(msg, "aec"), "pcError": js::gs(msg, "pc_error"), "voices": msg["voices"] != json!(false)
                    });
                    self.with(|s| {
                        s["pending"] = json!(false);
                        s["engine"] = engine.clone();
                    });
                    self.publish(json!({ "engine": engine }));
                    let pe = js::gs(msg, "pc_error");
                    if !pe.is_empty() && !pe.contains("fichier") {
                        o.emit(json!({ "type": "say", "text": "Attention : je n’arrive pas à capter le son du PC, seul le micro est transcrit." }));
                    }
                } else if !js::gb(msg, "on") && js::gb(&s, "recording") && !js::gb(&s, "pending") {
                    let now = js::now_ms();
                    let s = self
                        .with(|s| {
                            s["recording"] = json!(false);
                            s["ended"] = json!(now);
                            s.clone()
                        })
                        .unwrap();
                    self.publish(json!({ "recording": false, "ended": now }));
                    let me = self.clone();
                    js::spawn(async move { me.finish(s).await });
                }
            }
            "meeting_text" => {
                let text = js::gs(msg, "text");
                if text.is_empty() {
                    return;
                }
                let sp = &msg["speaker"];
                let who = if js::gs(msg, "source") == "participants" {
                    if sp.is_object() {
                        json!({ "id": sp["id"], "name": sp["name"], "known": js::gb(sp, "known"), "cluster": if js::truthy(&sp["cluster"]) { sp["cluster"].clone() } else { Value::Null } })
                    } else {
                        json!({ "id": "participants", "name": "Participants", "known": false })
                    }
                } else {
                    let a = js::gs(&s, "author");
                    json!({ "id": "moi", "name": if a.is_empty() { "Moi".into() } else { a }, "known": true, "self": true })
                };
                let at = js::num(&msg["at"]).filter(|x| *x != 0.0).map(|x| (x * 1000.0).round() as i64).unwrap_or_else(js::now_ms);
                let (line, sess) = self
                    .with(|s| {
                        if let Some(c) = who["cluster"].as_str() {
                            if let Some(v) = s["voices"].get_mut(c) {
                                v["count"] = json!(js::gn_or(v, "count", 0.0) as i64 + 1);
                            }
                        }
                        let n = js::arr(&s["lines"]).len() + 1;
                        let line = json!({ "n": n, "at": at, "source": msg["source"], "who": who, "text": text.trim() });
                        if let Some(l) = s["lines"].as_array_mut() {
                            l.push(line.clone());
                        }
                        s["dirty"] = json!(true);
                        (line, s.clone())
                    })
                    .unwrap();
                o.emit(json!({ "type": "meeting_line", "meetingId": sess["id"], "line": line }));
                if sp.is_object() {
                    self.publish_people(&sess);
                }
                if !js::gb(&sess, "recording") && js::truthy(&sess["file"]) {
                    let mut s2 = sess;
                    self.save(&mut s2);
                }
            }
            "meeting_voice" => {
                let id = js::gs(msg, "id");
                let suggest: Vec<J> = js::arr(&msg["suggest"]).iter().map(|x| json!({ "id": x["id"], "name": x["name"], "similarity": x["similarity"] })).collect();
                let sess = self
                    .with(|s| {
                        if !s["voices"].is_object() {
                            s["voices"] = json!({});
                        }
                        s["voices"][&id] = json!({ "id": id, "label": msg["label"], "person": null, "suggest": suggest, "count": 0 });
                        s.clone()
                    })
                    .unwrap();
                self.publish_people(&sess);
            }
            "meeting_voice_named" => {
                let id = js::gs(msg, "id");
                if !s["voices"][&id].is_object() {
                    return;
                }
                let wanted = js::gs(&s["voices"][&id], "wantedName");
                self.with(|s| {
                    s["voices"][&id]["pendingName"] = Value::Null;
                    s["voices"][&id]["wantedName"] = Value::Null;
                });
                if msg["ok"] == json!(false) {
                    if !wanted.is_empty() {
                        self.apply_name(&[id.clone()], &json!({ "id": format!("nom-{}", slug(&wanted)), "name": wanted }), false);
                    }
                    o.emit(json!({ "type": "say", "text": if wanted.is_empty() { "Je n’ai pas pu nommer cette voix.".to_string() } else { format!("J’ai noté {wanted} dans le compte rendu, mais je n’ai pas pu apprendre sa voix.") } }));
                    return;
                }
                let mut ids = vec![id.clone()];
                ids.extend(js::str_list(&msg["merged"]));
                self.apply_name(&ids, &msg["speaker"], true);
                o.record("reunion_voix_nommee", json!({
                    "reunion": s["title"], "voix": s["voices"][&id]["label"], "nom": msg["speaker"]["name"],
                    "nouveau_profil": js::gb(msg, "created"), "regroupees": js::arr(&msg["merged"]).len()
                }));
                o.publish_status();
            }
            "meeting_done" => {
                if !js::gb(&s, "recording") {
                    let me = self.clone();
                    js::spawn(async move { me.finish(s).await });
                }
            }
            _ => {}
        }
    }

    /// Voix nommée : ses passages (et ceux des voix regroupées) portent désormais ce nom.
    fn apply_name(&self, ids: &[String], person: &J, learned: bool) {
        let who = json!({ "id": person["id"], "name": person["name"], "known": true });
        let Some(s) = self
            .with(|s| {
                for id in ids {
                    if s["voices"][id].is_object() {
                        s["voices"][id]["person"] = json!({ "id": person["id"], "name": person["name"] });
                        s["voices"][id]["learned"] = json!(learned);
                    }
                }
                if let Some(lines) = s["lines"].as_array_mut() {
                    for l in lines.iter_mut() {
                        if let Some(c) = l["who"]["cluster"].as_str().map(String::from) {
                            if ids.contains(&c) {
                                let mut w = who.clone();
                                w["cluster"] = json!(c);
                                l["who"] = w;
                            }
                        }
                    }
                }
                s["dirty"] = json!(true);
                s.clone()
            })
        else {
            return;
        };
        self.o.up().emit(json!({ "type": "meeting_relabel", "meetingId": s["id"], "clusters": ids, "who": who }));
        self.publish_people(&s);
        if js::truthy(&s["file"]) && !js::gb(&s, "recording") {
            let mut s2 = s;
            self.save(&mut s2);
        }
    }

    /// « Voix inconnue 2, c'est Marie » (card ou voix).
    pub fn name_voice(&self, cid: &str, raw_name: &str) -> J {
        let Some(s) = self.session.borrow().clone() else { return json!({ "ok": false, "say": "Cette voix n’est plus dans la réunion affichée." }) };
        let v = s["voices"][cid].clone();
        if !v.is_object() {
            return json!({ "ok": false, "say": "Cette voix n’est plus dans la réunion affichée." });
        }
        let mut name = js::slice_chars(crate::re!(r"\s+").replace_all(raw_name, " ").trim(), 40);
        if !crate::re!(r"^\p{L}[\p{L}\p{M}' .-]{0,39}$").is_match(&name) {
            return json!({ "ok": false, "say": "Ce prénom n’est pas valide." });
        }
        if let Some(in_meet) = js::str_list(&s["participants"]).into_iter().find(|n| fold(n) == fold(&name)) {
            name = in_meet;
        } else if name == name.to_lowercase() {
            name = crate::re!(r"(^|[\s'-])(\p{L})").replace_all(&name, |c: &regex::Captures| format!("{}{}", &c[1], c[2].to_uppercase())).into_owned();
        }
        let o = self.o.up();
        let status = o.get_speaker_status();
        let known = js::arr(&status["speakers"]).into_iter().find(|p| fold(&js::gs(p, "name")) == fold(&name));
        let who = known.as_ref().map(|k| js::gs(k, "name")).unwrap_or(name);
        let label = js::gs(&v, "label");
        let person = match &known {
            Some(k) => json!({ "id": k["id"], "name": k["name"] }),
            None => json!({ "id": format!("nom-{}", slug(&who)), "name": who }),
        };
        // Réunion rouverte depuis l'archive : le nom vaut pour ce compte rendu.
        if js::gs(&s, "id") != *self.engine_session.borrow() {
            self.apply_name(&[cid.to_string()], &person, false);
            return json!({ "ok": true, "say": format!("C’est noté : {label}, c’est {who}.") });
        }
        let sess = self
            .with(|s| {
                s["voices"][cid]["pendingName"] = json!(who);
                s["voices"][cid]["wantedName"] = json!(who);
                s.clone()
            })
            .unwrap();
        let vtype = known.as_ref().map(|k| if js::truthy(&k["type"]) { js::gs(k, "type") } else { "autre".into() }).unwrap_or_else(|| "visio".into());
        o.emit(json!({ "type": "meeting_name_voice", "id": cid, "name": who, "voiceType": vtype }));
        self.publish_people(&sess);
        // Moteur vocal absent : sans réponse, le nom vaut pour le compte rendu seulement.
        let oc = self.o.clone();
        let (cid2, who2) = (cid.to_string(), who.clone());
        js::set_timeout(10000, move || {
            let orch = oc.up();
            let m = &orch.meetings;
            let still = m.session.borrow().as_ref().map(|s| js::gs(&s["voices"][&cid2], "pendingName") == who2 && !js::truthy(&s["voices"][&cid2]["person"])).unwrap_or(false);
            if still {
                m.with(|s| {
                    s["voices"][&cid2]["pendingName"] = Value::Null;
                    s["voices"][&cid2]["wantedName"] = Value::Null;
                });
                m.apply_name(&[cid2.clone()], &person, false);
            }
        });
        json!({ "ok": true, "say": format!("C’est noté : {label}, c’est {who}. J’apprends sa voix.") })
    }

    // --- Card ---------------------------------------------------------------------------------------

    fn voices(s: &J) -> Vec<J> {
        s["voices"].as_object().map(|m| m.values().cloned().collect()).unwrap_or_default()
    }

    /// Personnes présentes : toi, les personnes reconnues, la liste de Meet, les voix inconnues.
    pub fn people(&self, s: &J) -> Vec<J> {
        let mut out: Vec<J> = vec![];
        let mut seen = HashSet::new();
        let mut add = |out: &mut Vec<J>, p: J| {
            let k = fold(&js::gs(&p, "name"));
            if k.is_empty() || !seen.insert(k) {
                return;
            }
            out.push(p);
        };
        let a = js::gs(s, "author");
        add(&mut out, json!({ "name": if a.is_empty() { "Moi".into() } else { a }, "kind": "self" }));
        for l in js::arr(&s["lines"]) {
            let w = &l["who"];
            if js::gb(w, "known") && !js::gb(w, "self") && js::gs(w, "id") != "participants" {
                add(&mut out, json!({ "name": w["name"], "kind": "voice" }));
            }
        }
        for v in Self::voices(s) {
            if js::truthy(&v["person"]) {
                add(&mut out, json!({ "name": v["person"]["name"], "kind": "voice" }));
            }
        }
        for n in js::str_list(&s["participants"]) {
            let k = fold(&n);
            let first = |x: &str| x.split(' ').next().unwrap_or("").to_string();
            if let Some(m) = out.iter_mut().find(|p| {
                let pn = fold(&js::gs(p, "name"));
                pn == k || first(&pn) == first(&k)
            }) {
                m["meet"] = json!(true);
            } else {
                add(&mut out, json!({ "name": n, "kind": "meet", "meet": true }));
            }
        }
        for v in Self::voices(s) {
            if !js::truthy(&v["person"]) {
                out.push(json!({ "name": v["label"], "kind": "unknown", "voice": v["id"] }));
            }
        }
        out
    }

    fn voice_list(&self, s: &J) -> Vec<J> {
        let status = self.o.up().get_speaker_status();
        let voices = Self::voices(s);
        let named: HashSet<String> = voices.iter().filter(|v| js::truthy(&v["person"])).map(|v| fold(&js::gs(&v["person"], "name"))).collect();
        let first = |n: &str| fold(n).split(' ').next().unwrap_or("").to_string();
        let author = js::gs(s, "author");
        let meet_free: Vec<String> = js::str_list(&s["participants"]).into_iter().filter(|n| !named.contains(&fold(n)) && first(n) != first(&author)).collect();
        let others: Vec<String> = js::arr(&status["speakers"]).iter().filter(|p| !js::gb(p, "isPrimary")).map(|p| js::gs(p, "name")).collect();
        voices
            .iter()
            .map(|v| {
                let mut names: Vec<String> = vec![];
                let mut push = |n: String| {
                    if !n.is_empty() && !names.iter().any(|x| fold(x) == fold(&n)) {
                        names.push(n);
                    }
                };
                for x in js::arr(&v["suggest"]).iter().filter(|x| js::gn_or(x, "similarity", 0.0) >= 0.3) {
                    push(js::gs(x, "name"));
                }
                for n in &meet_free {
                    push(n.clone());
                }
                for n in &others {
                    push(n.clone());
                }
                names.truncate(8);
                json!({
                    "id": v["id"], "label": v["label"], "person": v["person"],
                    "pending": if js::truthy(&v["pendingName"]) { v["pendingName"].clone() } else { Value::Null },
                    "count": v["count"], "learned": v["learned"], "suggest": names
                })
            })
            .collect()
    }

    fn card_data(&self, s: &J) -> J {
        let lines = js::arr(&s["lines"]);
        let created = js::gn_or(s, "created", 0.0) as i64;
        let code = js::gs(&s["meet"], "code");
        json!({
            "meetingId": s["id"], "title": s["title"], "author": s["author"], "created": created, "dateLabel": long_date(created),
            "ended": s["ended"], "recording": js::gb(s, "recording"), "meet": if meet_code(&code) { code } else { String::new() },
            "engine": s["engine"], "people": self.people(s), "voices": self.voice_list(s),
            "lines": lines[lines.len().saturating_sub(400)..].to_vec(), "total": lines.len(),
            "summary": s["summary"], "doc": s["doc"],
            "file": s["file"].as_str().and_then(|f| Path::new(f).file_name().map(|x| x.to_string_lossy().to_string())).unwrap_or_default(),
            "busy": *self.busy.borrow()
        })
    }

    pub fn show(&self) {
        if let Some(s) = self.session.borrow().clone() {
            self.o.up().show_widget("meeting", self.card_data(&s));
        }
    }

    fn publish(&self, patch: J) {
        if let Some(id) = self.sid() {
            let mut ev = json!({ "type": "meeting_update", "meetingId": id });
            js::assign(&mut ev, patch);
            self.o.up().emit(ev);
        }
    }

    fn publish_people(&self, s: &J) {
        if self.is_current(s) {
            self.publish(json!({ "people": self.people(s), "voices": self.voice_list(s) }));
        }
    }

    pub fn set_title(&self, title: &str) -> J {
        let t = js::slice_chars(crate::re!(r"\s+").replace_all(title, " ").trim(), 200);
        if t.is_empty() {
            return json!({ "ok": false });
        }
        let Some(mut s) = self
            .with(|s| {
                s["title"] = json!(t);
                s["titleSource"] = json!("user");
                s["dirty"] = json!(true);
                s.clone()
            })
        else {
            return json!({ "ok": false });
        };
        if js::truthy(&s["file"]) && !js::gb(&s, "recording") {
            self.save(&mut s);
        }
        json!({ "ok": true })
    }

    // --- Archive ------------------------------------------------------------------------------------

    fn to_json(&self, s: &J) -> J {
        let mut speakers: Vec<(String, J)> = vec![];
        for l in js::arr(&s["lines"]) {
            let w = &l["who"];
            if w.is_object() {
                let id = js::s(&w["id"]);
                let entry = json!({ "id": w["id"], "name": w["name"], "known": w["known"] });
                match speakers.iter_mut().find(|(k, _)| *k == id) {
                    Some(x) => x.1 = entry,
                    None => speakers.push((id, entry)),
                }
            }
        }
        json!({
            "version": 1, "id": s["id"], "title": s["title"], "created": s["created"],
            "ended": if js::truthy(&s["ended"]) { s["ended"].clone() } else { json!(js::now_ms()) }, "author": s["author"],
            "meet": s["meet"], "participants": self.people(s).iter().filter(|p| js::gs(p, "kind") != "unknown").map(|p| p["name"].clone()).collect::<Vec<_>>(),
            "speakers": speakers.into_iter().map(|(_, v)| v).collect::<Vec<_>>(),
            "voices": Self::voices(s).iter().map(|v| json!({ "id": v["id"], "label": v["label"], "person": v["person"] })).collect::<Vec<_>>(),
            "lines": js::arr(&s["lines"]).iter().map(|l| json!({ "at": l["at"], "source": l["source"], "who": l["who"], "text": l["text"] })).collect::<Vec<_>>(),
            "summary": js::gs(s, "summary"), "doc": if js::truthy(&s["doc"]) { s["doc"].clone() } else { Value::Null }
        })
    }

    pub fn save(&self, s: &mut J) -> J {
        if js::arr(&s["lines"]).is_empty() {
            return json!({ "ok": false, "say": "La réunion est vide : rien à archiver." });
        }
        let res = (|| -> std::io::Result<PathBuf> {
            std::fs::create_dir_all(&self.dir)?;
            if !js::truthy(&s["file"]) {
                let created = js::gn_or(s, "created", 0.0) as i64;
                let stamp = js::local_of(created).format("%Y-%m-%d_%Hh%M").to_string();
                let sl = slug(&js::gs(s, "title"));
                let mut file = self.dir.join(format!("{stamp}_{sl}.json"));
                let mut i = 2;
                while file.exists() {
                    file = self.dir.join(format!("{stamp}_{sl}-{i}.json"));
                    i += 1;
                }
                s["file"] = json!(file.to_string_lossy());
            }
            let file = PathBuf::from(js::gs(s, "file"));
            std::fs::write(&file, serde_json::to_string_pretty(&self.to_json(s)).unwrap_or_default())?;
            Ok(file)
        })();
        let file = match res {
            Ok(f) => f,
            Err(e) => {
                (self.log)(format!("réunions : archivage impossible ({e})"));
                return json!({ "ok": false, "say": format!("Je n’ai pas pu archiver la réunion : {e}") });
            }
        };
        s["dirty"] = json!(false);
        self.sync(s);
        let o = self.o.up();
        o.record("reunion_archivee", json!({ "titre": s["title"], "fichier": file.to_string_lossy(), "phrases": js::arr(&s["lines"]).len() }));
        if self.is_current(s) {
            self.publish(json!({ "file": file.file_name().map(|f| f.to_string_lossy().to_string()) }));
        }
        let oc = self.o.clone();
        js::spawn(async move {
            let orch = oc.up();
            if let Err(e) = orch.meetings.index(None).await {
                (orch.meetings.log)(format!("réunions : indexation différée ({e})"));
            }
        });
        json!({ "ok": true, "file": file.to_string_lossy() })
    }

    pub fn list(&self) -> Vec<J> {
        let Ok(rd) = std::fs::read_dir(&self.dir) else { return vec![] };
        let mut out: Vec<J> = vec![];
        for e in rd.filter_map(|e| e.ok()) {
            let name = e.file_name().to_string_lossy().to_string();
            if !name.ends_with(".json") || name == "vectors.json" {
                continue;
            }
            let Some(j) = js::read_json(&e.path()) else { continue };
            out.push(json!({
                "file": e.path().to_string_lossy(), "title": j["title"], "created": j["created"], "ended": j["ended"],
                "participants": if j["participants"].is_array() { j["participants"].clone() } else { json!([]) },
                "lines": js::arr(&j["lines"]).len(), "summary": js::truthy(&j["summary"]), "doc": if js::truthy(&j["doc"]) { j["doc"].clone() } else { Value::Null }
            }));
        }
        out.sort_by(|a, b| js::gn_or(b, "created", 0.0).partial_cmp(&js::gn_or(a, "created", 0.0)).unwrap_or(std::cmp::Ordering::Equal));
        out
    }

    pub fn load(&self, file: &str) -> Option<J> {
        let j = js::read_json(Path::new(file))?;
        let mut s = self.new_session(&js::gs(&j, "title"), None);
        let author = if js::truthy(&j["author"]) { js::gs(&j, "author") } else { js::gs(&s, "author") };
        let lines: Vec<J> = js::arr(&j["lines"])
            .into_iter()
            .enumerate()
            .map(|(i, l)| {
                let mut x = json!({ "n": i + 1 });
                js::assign(&mut x, l);
                x
            })
            .collect();
        let ja = fold(&js::gs(&j, "author"));
        js::assign(
            &mut s,
            json!({
                "title": j["title"], "titleSource": "user", "created": j["created"], "ended": j["ended"], "author": author,
                "meet": if js::truthy(&j["meet"]) { j["meet"].clone() } else { Value::Null },
                "participants": js::str_list(&j["participants"]).into_iter().filter(|n| fold(n) != ja).collect::<Vec<_>>(),
                "summary": js::gs(&j, "summary"), "doc": if js::truthy(&j["doc"]) { j["doc"].clone() } else { Value::Null }, "file": file
            }),
        );
        let mut voices = Map::new();
        for v in js::arr(&j["voices"]) {
            let id = js::s(&v["id"]);
            let count = lines.iter().filter(|l| js::s(&l["who"]["cluster"]) == id).count();
            voices.insert(id, json!({ "id": v["id"], "label": v["label"], "person": v["person"], "suggest": [], "count": count, "learned": false }));
        }
        s["voices"] = Value::Object(voices);
        s["lines"] = json!(lines);
        Some(s)
    }

    pub fn open(&self, file: &str) -> J {
        if self.recording() {
            return json!({ "ok": false, "say": "Une réunion est en cours d’enregistrement : arrête-la d’abord." });
        }
        if let Some(mut cur) = self.session.borrow().clone().filter(|s| js::gb(s, "dirty") && !js::arr(&s["lines"]).is_empty()) {
            self.save(&mut cur);
        }
        let Some(s) = self.load(file) else { return json!({ "ok": false, "say": "Cette réunion est illisible." }) };
        *self.session.borrow_mut() = Some(s.clone());
        self.show();
        json!({ "ok": true, "say": format!("Réunion {}, {}.", js::gs(&s, "title"), short_date(js::gn_or(&s, "created", 0.0) as i64)), "session": s })
    }

    pub fn find(&self, r: &Ref) -> Option<J> {
        let all = self.list();
        if let Some(n) = r.n {
            let last = self.last_list.borrow();
            let src = if last.is_empty() { &all } else { &*last };
            return n.checked_sub(1).and_then(|i| src.get(i as usize)).cloned();
        }
        if r.last || r.words.is_empty() {
            return all.first().cloned();
        }
        let w: Vec<String> = fold(&r.words).split(' ').filter(|x| x.chars().count() > 2 && !["la", "les", "des", "avec", "sur"].contains(x)).map(String::from).collect();
        if w.is_empty() {
            return all.first().cloned();
        }
        let hay = |m: &J| fold(&format!("{} {} {}", js::gs(m, "title"), js::str_list(&m["participants"]).join(" "), long_date(js::gn_or(m, "created", 0.0) as i64)));
        all.iter().find(|m| w.iter().all(|x| hay(m).contains(x.as_str()))).or_else(|| all.iter().find(|m| w.iter().any(|x| hay(m).contains(x.as_str())))).cloned()
    }

    pub fn show_list(&self, title: &str) -> Vec<J> {
        let all = self.list();
        let shown: Vec<J> = all.iter().take(30).cloned().collect();
        let call = |file: &J, action: &str| json!({ "method": "meetingAction", "args": ["", action, { "file": file }] });
        let rows: Vec<J> = shown
            .iter()
            .enumerate()
            .map(|(i, m)| {
                let created = js::gn_or(m, "created", 0.0) as i64;
                let sub: Vec<String> = [short_date(created), m["ended"].as_i64().map(|e| minutes_of(e - created)).unwrap_or_default(), js::str_list(&m["participants"]).into_iter().take(4).collect::<Vec<_>>().join(", ")]
                    .into_iter()
                    .filter(|s| !s.is_empty())
                    .collect();
                let n = m["lines"].as_i64().unwrap_or(0);
                json!({
                    "icon": "◉", "title": format!("{}. {}", i + 1, js::gs(m, "title")), "sub": sub.join(" · "),
                    "meta": format!("{n} phrase{}", if n > 1 { "s" } else { "" }), "call": call(&m["file"], "open"),
                    "hint": "Clic : ouvrir cette réunion", "say": format!("ouvre la réunion {}", i + 1),
                    "actions": [
                        { "icon": "▣", "title": "Ouvrir dans la card de réunion", "call": call(&m["file"], "open") },
                        { "icon": "✦", "title": "Résumé par l’IA", "call": call(&m["file"], "summary") },
                        { "icon": "G", "title": if js::truthy(&m["doc"]) { "Ouvrir le Google Doc" } else { "Créer le Google Doc" }, "call": call(&m["file"], "gdoc") }
                    ]
                })
            })
            .collect();
        *self.last_list.borrow_mut() = shown;
        let count = rows.len();
        self.o.up().show_widget(
            "tool",
            json!({
                "title": if title.is_empty() { "liste gmeet" } else { title }, "tool": "reunions", "rows": rows, "count": count, "ttl": 120000,
                "empty": "Aucune réunion enregistrée. Dis « Jarvis, enregistre la réunion ».",
                "footer": format!("{} · « résume la réunion 2 » · « ouvre la réunion avec Marie »", self.dir.to_string_lossy())
            }),
        );
        all
    }

    // --- Texte, résumé, Google Docs -----------------------------------------------------------------

    fn transcript(s: &J, with_time: bool) -> String {
        js::arr(&s["lines"])
            .iter()
            .map(|l| {
                let who = if l["who"].is_object() { js::gs(&l["who"], "name") } else { "Participants".into() };
                format!("{}{who} : {}", if with_time { format!("[{}] ", clock_s(js::gn_or(l, "at", 0.0) as i64)) } else { String::new() }, js::gs(l, "text"))
            })
            .collect::<Vec<_>>()
            .join("\n")
    }

    fn header(&self, s: &J) -> String {
        let people: Vec<String> = self.people(s).iter().filter(|p| js::gs(p, "kind") != "unknown").map(|p| js::gs(p, "name")).collect();
        let created = js::gn_or(s, "created", 0.0) as i64;
        let code = js::gs(&s["meet"], "code");
        [
            format!("Titre : {}", js::gs(s, "title")),
            format!("Date : {}{}", long_date(created), s["ended"].as_i64().map(|e| format!(" ({})", minutes_of(e - created))).unwrap_or_default()),
            if people.is_empty() { String::new() } else { format!("Participants : {}", people.join(", ")) },
            if meet_code(&code) { format!("Google Meet : {code}") } else { String::new() },
        ]
        .into_iter()
        .filter(|x| !x.is_empty())
        .collect::<Vec<_>>()
        .join("\n")
    }

    fn set_busy(&self, what: &str, on: bool, s: &J) {
        {
            let mut b = self.busy.borrow_mut();
            b.retain(|x| x != what);
            if on {
                b.push(what.to_string());
            }
        }
        if self.is_current(s) {
            self.publish(json!({ "busy": *self.busy.borrow() }));
        }
    }

    pub async fn summarize(&self, s: &mut J) -> J {
        if js::arr(&s["lines"]).is_empty() {
            return json!({ "ok": false, "say": "La réunion est vide : rien à résumer." });
        }
        if self.busy.borrow().iter().any(|x| x == "summary") {
            return json!({ "ok": false, "say": "Le résumé est déjà en cours." });
        }
        self.set_busy("summary", true, s);
        let o = self.o.up();
        let task_id = format!("resume-reunion{}", base36(js::now_ms() as u64));
        o.emit(json!({ "type": "create", "id": task_id, "label": "Résumé de la réunion", "detail": js::slice_chars(&format!("Sonnet · {}", js::gs(s, "title")), 60) }));
        let input = format!("{}\nType : transcription d’une réunion (chaque ligne : heure, personne qui parle, texte ; « Voix inconnue » = participant non identifié)\n\n{}", self.header(s), Self::transcript(s, true));
        let res = o.notes.claude_summary(input).await;
        let out = match res {
            Ok(summary) => {
                let summary = crate::re!(r"(?m)\*\*|^#+\s*").replace_all(&summary, "").trim().to_string();
                s["summary"] = json!(summary);
                s["dirty"] = json!(true);
                if !js::gb(s, "recording") {
                    self.save(s);
                } else {
                    self.sync(s);
                }
                if self.is_current(s) {
                    self.publish(json!({ "summary": summary }));
                }
                o.record("reunion_resume", json!({ "titre": s["title"], "resume": js::slice_chars(&summary, 2000) }));
                o.emit(json!({ "type": "finish", "id": task_id, "status": "done" }));
                self.ingest_rag(s);
                let brief = brief_of(&summary);
                json!({ "ok": true, "summary": summary, "say": format!("{} Le résumé complet est dans la card de la réunion.", if brief.is_empty() { "Résumé prêt.".to_string() } else { js::capitalize(&brief) }) })
            }
            Err(e) => {
                (self.log)(format!("réunions : résumé impossible ({e})"));
                o.emit(json!({ "type": "finish", "id": task_id, "status": "error" }));
                json!({ "ok": false, "say": format!("Je n’ai pas pu faire le résumé : {e}") })
            }
        };
        self.set_busy("summary", false, s);
        out
    }

    fn html(&self, s: &J) -> String {
        let people: Vec<String> = self.people(s).iter().filter(|p| js::gs(p, "kind") != "unknown").map(|p| js::gs(p, "name")).collect();
        let created = js::gn_or(s, "created", 0.0) as i64;
        let code = js::gs(&s["meet"], "code");
        let mut parts = vec![
            format!("<h1>{}</h1>", esc(&js::gs(s, "title"))),
            format!("<p><b>Date :</b> {}{}</p>", esc(&long_date(created)), s["ended"].as_i64().map(|e| format!(" · <b>Durée :</b> {}", esc(&minutes_of(e - created)))).unwrap_or_default()),
            if people.is_empty() { String::new() } else { format!("<p><b>Participants :</b> {}</p>", esc(&people.join(", "))) },
            if meet_code(&code) { format!("<p><b>Google Meet :</b> {}</p>", esc(&code)) } else { String::new() },
        ];
        let summary = js::gs(s, "summary");
        if !summary.is_empty() {
            parts.push("<h2>Résumé</h2>".into());
            let mut list = false;
            for raw in summary.split('\n') {
                let line = raw.trim();
                if line.is_empty() {
                    continue;
                }
                if let Some(item) = crate::re!(r"^[-•]\s*(.+)$").captures(line) {
                    if !list {
                        parts.push("<ul>".into());
                        list = true;
                    }
                    parts.push(format!("<li>{}</li>", esc(&item[1])));
                    continue;
                }
                if list {
                    parts.push("</ul>".into());
                    list = false;
                }
                match crate::re!(r"^([^:]{2,30}) :\s*(.*)$").captures(line) {
                    Some(h) => parts.push(format!("<p><b>{} :</b> {}</p>", esc(&h[1]), esc(&h[2]))),
                    None => parts.push(format!("<p>{}</p>", esc(line))),
                }
            }
            if list {
                parts.push("</ul>".into());
            }
        }
        parts.push("<h2>Transcription</h2>".into());
        for l in js::arr(&s["lines"]) {
            let who = if l["who"].is_object() { js::gs(&l["who"], "name") } else { "Participants".into() };
            parts.push(format!("<p><span style=\"color:#888\">[{}]</span> <b>{} :</b> {}</p>", clock(js::gn_or(&l, "at", 0.0) as i64), esc(&who), esc(&js::gs(&l, "text"))));
        }
        parts.push("<p style=\"color:#888\"><i>Transcription automatique par Jarvis.</i></p>".into());
        format!("<html><head><meta charset=\"utf-8\"></head><body>{}</body></html>", parts.into_iter().filter(|p| !p.is_empty()).collect::<Vec<_>>().join("\n"))
    }

    pub async fn export_doc(&self, s: &mut J) -> J {
        let o = self.o.up();
        if js::arr(&s["lines"]).is_empty() {
            return json!({ "ok": false, "say": "La réunion est vide : rien à mettre dans Google Docs." });
        }
        let url = js::gs(&s["doc"], "url");
        if !url.is_empty() {
            self.open_url(&url).await;
            return json!({ "ok": true, "url": url, "say": "J’ouvre le Google Doc de cette réunion." });
        }
        if !o.google_docs.configured() {
            o.request_setup(json!({ "connector": "gdocs", "missing": ["la connexion à Google Docs"] }), "reunion");
            return json!({ "ok": false, "say": "Google Docs n’est pas encore relié à Jarvis : je t’ouvre la page de configuration." });
        }
        if self.busy.borrow().iter().any(|x| x == "gdoc") {
            return json!({ "ok": false, "say": "Le document est déjà en cours de création." });
        }
        self.set_busy("gdoc", true, s);
        if js::gs(s, "summary").is_empty() && Self::transcript(s, true).chars().count() >= 200 {
            self.summarize(s).await;
        }
        let title = format!("{} — {}", js::gs(s, "title"), short_date(js::gn_or(s, "created", 0.0) as i64));
        let r = o.google_docs.create_document(&title, &self.html(s)).await;
        let out = if !js::gb(&r, "ok") {
            if js::gb(&r, "setup") {
                o.request_setup(json!({ "connector": "gdocs", "missing": ["la connexion à Google Docs"] }), "reunion");
            }
            json!({ "ok": false, "say": format!("Je n’ai pas pu créer le Google Doc : {}", js::gs(&r, "error")) })
        } else {
            s["doc"] = json!({ "id": r["id"], "url": r["url"], "at": js::now_ms() });
            s["dirty"] = json!(true);
            if !js::gb(s, "recording") {
                self.save(s);
            } else {
                self.sync(s);
            }
            if self.is_current(s) {
                self.publish(json!({ "doc": s["doc"] }));
            }
            o.record("reunion_google_doc", json!({ "titre": s["title"], "lien": r["url"] }));
            self.open_url(&js::gs(&r, "url")).await;
            json!({ "ok": true, "url": r["url"], "say": "Le compte rendu est dans Google Docs, dans le dossier « Réunions Jarvis ». Je l’ouvre." })
        };
        self.set_busy("gdoc", false, s);
        out
    }

    async fn open_url(&self, url: &str) {
        let o = self.o.up();
        if o.browser_bridge.connected() && js::gb(&o.browser_bridge.request("open", json!({ "url": url }), 8000).await, "ok") {
            return;
        }
        let _ = crate::orch::system::call("ouvrir_url", &json!({ "url": url })).await;
    }

    // --- Recherche par le sens (RAG) ----------------------------------------------------------------

    fn passages(m: &J) -> Vec<(String, i64, &'static str)> {
        let created = js::gn_or(m, "created", 0.0) as i64;
        let parts = js::str_list(&m["participants"]);
        let head = format!("Réunion « {} » du {}{}.", js::gs(m, "title"), long_date(created), if parts.is_empty() { String::new() } else { format!(", avec {}", parts.join(", ")) });
        let mut out = vec![];
        let summary = js::gs(m, "summary");
        if !summary.is_empty() {
            out.push((format!("{head}\nRésumé : {summary}"), created, "resume"));
        }
        let mut cur: Vec<String> = vec![];
        let mut size = 0;
        let mut at: Option<i64> = None;
        for l in js::arr(&m["lines"]) {
            let who = if l["who"].is_object() { js::gs(&l["who"], "name") } else { "Participants".into() };
            let line = format!("[{}] {who} : {}", clock(js::gn_or(&l, "at", 0.0) as i64), js::gs(&l, "text"));
            let len = line.encode_utf16().count();
            if size + len > 900 && !cur.is_empty() {
                out.push((format!("{head}\n{}", cur.join("\n")), at.unwrap_or(created), "texte"));
                cur.clear();
                size = 0;
                at = None;
            }
            if at.is_none() {
                at = Some(js::gn_or(&l, "at", 0.0) as i64);
            }
            cur.push(line);
            size += len;
        }
        if !cur.is_empty() {
            out.push((format!("{head}\n{}", cur.join("\n")), at.unwrap_or(created), "texte"));
        }
        out
    }

    fn corpus(&self) -> Vec<J> {
        let mut items = vec![];
        for f in self.list() {
            let file = js::gs(&f, "file");
            let Some(j) = js::read_json(Path::new(&file)) else { continue };
            let m = json!({ "title": j["title"], "created": j["created"], "participants": js::str_list(&j["participants"]), "summary": js::gs(&j, "summary"), "lines": js::arr(&j["lines"]) });
            for (text, at, kind) in Self::passages(&m) {
                let hash = sha1_smol::Sha1::from(&text).hexdigest()[..20].to_string();
                items.push(json!({ "text": text, "at": at, "kind": kind, "file": file, "title": j["title"], "created": j["created"], "participants": m["participants"], "hash": hash }));
            }
        }
        items
    }

    /// Passages de réunions indexés et vecteurs calculés (page d'aide).
    pub fn rag_stats(&self) -> J {
        self.load_vectors();
        let vectors = self.vectors.borrow().as_ref().map(|v| v.len()).unwrap_or(0);
        json!({ "passages": self.corpus().len(), "vecteurs": vectors })
    }

    fn load_vectors(&self) {
        if self.vectors.borrow().is_none() {
            let v = js::read_json(&self.vectors_file).and_then(|v| v.as_object().cloned()).unwrap_or_default();
            *self.vectors.borrow_mut() = Some(v);
        }
    }

    /// Vecteurs manquants calculés, ceux des passages disparus retirés.
    pub async fn index(&self, items: Option<Vec<J>>) -> Result<bool, String> {
        let o = self.o.up();
        let Some(emb) = o.embedder.clone().filter(|e| e.active()) else { return Ok(false) };
        let items = items.unwrap_or_else(|| self.corpus());
        self.load_vectors();
        let missing: Vec<J> = {
            let v = self.vectors.borrow();
            let v = v.as_ref().unwrap();
            items.iter().filter(|i| !v.contains_key(&js::gs(i, "hash"))).cloned().collect()
        };
        for batch in missing.chunks(16) {
            let vecs = emb.embed(batch.iter().map(|i| js::gs(i, "text")).collect(), "passage").await?;
            let mut v = self.vectors.borrow_mut();
            let v = v.as_mut().unwrap();
            for (i, vec) in batch.iter().zip(vecs) {
                v.insert(js::gs(i, "hash"), json!(vec.iter().map(|x| ((*x as f64) * 1e5).round() / 1e5).collect::<Vec<_>>()));
            }
        }
        let keep: HashSet<String> = items.iter().map(|i| js::gs(i, "hash")).collect();
        let removed = {
            let mut v = self.vectors.borrow_mut();
            let v = v.as_mut().unwrap();
            let before = v.len();
            v.retain(|k, _| keep.contains(k));
            before - v.len()
        };
        if !missing.is_empty() || removed > 0 {
            let _ = std::fs::create_dir_all(&self.dir);
            let text = serde_json::to_string(&Value::Object(self.vectors.borrow().clone().unwrap())).unwrap_or_default();
            std::fs::write(&self.vectors_file, text).map_err(|e| e.to_string())?;
        }
        Ok(true)
    }

    /// → [{ titre, date, participants, extrait, type, fichier, score }]
    pub async fn search(&self, query: &str, limit: usize) -> Vec<J> {
        let items = self.corpus();
        if items.is_empty() {
            return vec![];
        }
        let mut scored: Option<Vec<(J, f64)>> = None;
        let o = self.o.up();
        if let Some(emb) = o.embedder.clone().filter(|e| e.active()) {
            if !query.trim().is_empty() {
                let r: Result<Vec<(J, f64)>, String> = async {
                    self.index(Some(items.clone())).await?;
                    let q = emb.embed(vec![query.to_string()], "query").await?.into_iter().next().unwrap_or_default();
                    let v = self.vectors.borrow();
                    let v = v.as_ref().unwrap();
                    Ok(items
                        .iter()
                        .map(|i| {
                            let s = v.get(&js::gs(i, "hash")).and_then(|x| x.as_array()).map(|a| a.iter().map(|x| x.as_f64().unwrap_or(0.0) as f32).collect::<Vec<f32>>()).map(|vec| cosine(&q, &vec) as f64).unwrap_or(0.0);
                            (i.clone(), s)
                        })
                        .collect())
                }
                .await;
                match r {
                    Ok(s) => scored = Some(s),
                    Err(e) => (self.log)(format!("réunions : recherche par le sens indisponible ({e}), repli sur les mots")),
                }
            }
        }
        let mut scored = scored.unwrap_or_else(|| {
            let words: Vec<String> = fold(query).split(' ').filter(|w| w.chars().count() > 2).map(String::from).collect();
            items
                .iter()
                .map(|i| {
                    let hay = fold(&js::gs(i, "text"));
                    let s = if words.is_empty() { 0.0 } else { words.iter().filter(|w| hay.contains(w.as_str())).count() as f64 / words.len() as f64 };
                    (i.clone(), s)
                })
                .collect()
        });
        scored.sort_by(|a, b| b.1.partial_cmp(&a.1).unwrap_or(std::cmp::Ordering::Equal).then(js::gn_or(&b.0, "created", 0.0).partial_cmp(&js::gn_or(&a.0, "created", 0.0)).unwrap_or(std::cmp::Ordering::Equal)));
        scored
            .into_iter()
            .take(limit.clamp(1, 12))
            .map(|(i, score)| {
                let text = js::gs(&i, "text");
                json!({
                    "titre": i["title"], "date": long_date(js::gn_or(&i, "created", 0.0) as i64), "participants": i["participants"],
                    "extrait": text.split('\n').skip(1).collect::<Vec<_>>().join("\n"), "type": i["kind"],
                    "fichier": Path::new(&js::gs(&i, "file")).file_name().map(|f| f.to_string_lossy().to_string()), "score": (score * 1000.0).round() / 1000.0
                })
            })
            .collect()
    }

    /// Texte complet d'une réunion (outil reunion_lire) : « derniere », numéro, nom de fichier ou mots.
    pub fn read(&self, reference: &str) -> Option<J> {
        let r = reference.trim();
        let all = self.list();
        let base = |m: &J| Path::new(&js::gs(m, "file")).file_name().map(|f| f.to_string_lossy().to_string()).unwrap_or_default();
        let m = if r.is_empty() || crate::re!(r"(?i)^(?:derniere|dernière|last)$").is_match(r) {
            all.first().cloned()
        } else if crate::re!(r"^\d{1,3}$").is_match(r) {
            r.parse::<usize>().ok().and_then(|n| n.checked_sub(1)).and_then(|i| all.get(i).cloned())
        } else {
            all.iter().find(|x| base(x) == r).cloned().or_else(|| self.find(&Ref { words: r.to_string(), ..Default::default() }))
        }?;
        let s = self.load(&js::gs(&m, "file"))?;
        let summary = js::gs(&s, "summary");
        let text = format!("{}{}\n\nTranscription :\n{}", self.header(&s), if summary.is_empty() { String::new() } else { format!("\n\nRésumé :\n{summary}") }, Self::transcript(&s, true));
        let n = text.encode_utf16().count();
        Some(json!({ "titre": s["title"], "fichier": base(&m), "texte": js::slice_chars(&text, 60000), "tronque": n > 60000 }))
    }

    // --- Boutons de la card et de la bannière ----------------------------------------------------------

    pub async fn action(&self, id: &str, action: &str, payload: &J) -> J {
        let o = self.o.up();
        let say = |r: J| {
            if js::truthy(&r["say"]) {
                o.emit(json!({ "type": "say", "text": r["say"] }));
            }
            r
        };
        let p = if payload.is_object() { payload.clone() } else { json!({}) };
        match action {
            "start_meet" => return say(self.start(&json!({ "code": js::gs(&p, "code") })).await),
            "dismiss_offer" => {
                self.close_offer();
                return json!({ "ok": true });
            }
            "start" => return say(self.start(&json!({})).await),
            "stop" => return say(self.stop("demande")),
            "list" => {
                self.show_list("liste gmeet");
                return json!({ "ok": true });
            }
            "open" => {
                return match self.safe_file(&js::gs(&p, "file")) {
                    Some(f) => {
                        let mut r = self.open(&f);
                        if let Some(o) = r.as_object_mut() {
                            o.remove("session");
                        }
                        say(r)
                    }
                    None => json!({ "ok": false }),
                };
            }
            _ => {}
        }
        let cur = self.session.borrow().clone().filter(|s| id.is_empty() || js::gs(s, "id") == id);
        let mut target = cur;
        if js::truthy(&p["file"]) {
            let file = self.safe_file(&js::gs(&p, "file"));
            let in_session = self.session.borrow().clone().filter(|s| file.is_some() && js::gs(s, "file") == file.clone().unwrap_or_default());
            target = in_session.or_else(|| file.and_then(|f| self.load(&f)));
        }
        let Some(mut target) = target else { return json!({ "ok": false, "say": "Cette réunion n’est plus affichée." }) };
        match action {
            "title" => self.set_title(&js::gs(&p, "title")),
            "name_voice" => say(self.name_voice(&js::gs(&p, "voice"), &js::gs(&p, "name"))),
            "summary" => say(self.summarize(&mut target).await),
            "gdoc" => say(self.export_doc(&mut target).await),
            "save" => say(if js::gb(&self.save(&mut target), "ok") { json!({ "ok": true, "say": "Réunion archivée." }) } else { json!({ "ok": false, "say": "Rien à archiver." }) }),
            other => json!({ "ok": false, "error": format!("action inconnue : {other}") }),
        }
    }

    /// Fichier désigné par la page : seulement dans le dossier des réunions.
    fn safe_file(&self, file: &str) -> Option<String> {
        let base = Path::new(file).file_name()?;
        let f = self.dir.join(base);
        f.exists().then(|| f.to_string_lossy().to_string())
    }

    // --- Intentions du routeur ------------------------------------------------------------------------

    fn intent<F, Fu>(&self, kind: &str, label: &str, detail: &str, f: F) -> Intent
    where
        F: Fn(Rc<Meetings>) -> Fu + 'static,
        Fu: std::future::Future<Output = J> + 'static,
    {
        let o = self.o.clone();
        let f = Rc::new(f);
        Intent::new(kind, label, detail, move || {
            let (o, f) = (o.clone(), f.clone());
            async move { f(o.up().meetings.clone()).await }
        })
    }

    pub fn match_(&self, t: &str, raw: &str) -> Option<Intent> {
        let s = self.session.borrow().clone();
        if meeting_stop_re().is_match(t) && self.recording() {
            return Some(self.intent("meeting_stop", "Réunion", "fin de l’enregistrement", |m| async move {
                let r = m.stop("demande");
                json!({ "say": r["say"], "display": if js::gb(&r, "ok") { json!("Enregistrement arrêté") } else { r["say"].clone() } })
            }));
        }
        if meeting_start_re().is_match(t) {
            let title = spoken_topic(raw);
            let detail = if title.is_empty() { "enregistrement".to_string() } else { title.clone() };
            return Some(self.intent("meeting_start", "Réunion", &detail, move |m| {
                let title = title.clone();
                async move {
                    let r = m.start(&json!({ "title": title })).await;
                    json!({ "say": r["say"], "display": if js::gb(&r, "ok") { json!("Réunion enregistrée") } else { r["say"].clone() } })
                }
            }));
        }
        if let Some(named) = name_voice_re().captures(t) {
            if let Some(s) = s.as_ref().filter(|s| !Self::voices(s).is_empty()) {
                let n = number_of(&named[1]).unwrap_or(0);
                if let Some(v) = Self::voices(s).into_iter().find(|v| js::gs(v, "label") == format!("Voix inconnue {n}")) {
                    let (vid, name) = (js::gs(&v, "id"), named[2].to_string());
                    return Some(self.intent("meeting_name_voice", "Réunion", &format!("{} : {name}", js::gs(&v, "label")), move |m| {
                        let (vid, name) = (vid.clone(), name.clone());
                        async move {
                            let r = m.name_voice(&vid, &name);
                            json!({ "say": r["say"], "display": r["say"] })
                        }
                    }));
                }
            }
        }
        let list_say = |all: &[J]| {
            let n = all.len();
            let p = if n > 1 { "s" } else { "" };
            if n > 0 { format!("Tu as {n} réunion{p} enregistrée{p}. La plus récente : {}.", js::gs(&all[0], "title")) } else { "Aucune réunion enregistrée pour l’instant.".into() }
        };
        if list_gmeet_re().is_match(t) {
            return Some(self.intent("meetings_list_gmeet", "liste gmeet", "liste des réunions Google Meet", move |m| async move {
                let all = m.show_list("liste gmeet");
                json!({ "say": list_say(&all), "display": format!("{} réunions", all.len()) })
            }));
        }
        if list_re().is_match(t) {
            return Some(self.intent("meetings_list", "Mes réunions", "liste", move |m| async move {
                let all = m.show_list("Mes réunions");
                json!({ "say": list_say(&all), "display": format!("{} réunions", all.len()) })
            }));
        }
        let pick = |c: &regex::Captures| Ref { n: c.get(1).and_then(|x| number_of(x.as_str())), words: c.get(2).map(|x| x.as_str().to_string()).unwrap_or_default(), last: c.get(1).is_none() && c.get(2).is_none() };
        if let Some(g) = gdoc_re().captures(t) {
            let n = g.get(1).and_then(|x| number_of(x.as_str()));
            let detail = n.map(|n| format!("réunion {n}")).unwrap_or_else(|| "réunion".into());
            return Some(self.intent("meeting_gdoc", "Google Docs", &detail, move |m| async move {
                let cur = if n.is_none() { m.session.borrow().clone() } else { None };
                let mut target = match cur {
                    Some(s) => s,
                    None => {
                        let Some(found) = m.find(&Ref { n, last: n.is_none(), ..Default::default() }) else { return json!({ "say": "Je ne trouve pas cette réunion. Dis « liste mes réunions ».", "display": "Réunion introuvable" }) };
                        let r = m.open(&js::gs(&found, "file"));
                        if !r["session"].is_object() {
                            return json!({ "say": r["say"], "display": r["say"] });
                        }
                        r["session"].clone()
                    }
                };
                m.o.up().emit(json!({ "type": "say", "text": "Je prépare le compte rendu dans Google Docs." }));
                let r = m.export_doc(&mut target).await;
                json!({ "say": r["say"], "display": if js::gb(&r, "ok") { json!("Google Doc créé") } else { r["say"].clone() } })
            }));
        }
        if let Some(sum) = summary_re().captures(t) {
            let notes_open = self.o.up().notes.current.borrow().is_some();
            let about = meeting_word_re().is_match(t) || (!crate::re!(r"\bnotes?\b").is_match(t) && s.is_some() && !notes_open);
            if about {
                let r = pick(&sum);
                let explicit = r.n.is_some() || !r.words.is_empty() || crate::re!(r"\b(?:derniere|precedente)\b").is_match(t);
                let detail = r.n.map(|n| format!("réunion {n}")).unwrap_or_else(|| "réunion".into());
                return Some(self.intent("meeting_summary", "Résumé IA", &detail, move |m| {
                    let r = r.clone();
                    async move {
                        let cur = if explicit { None } else { m.session.borrow().clone() };
                        let mut target = match cur {
                            Some(s) => s,
                            None => {
                                let Some(found) = m.find(&r) else { return json!({ "say": "Je ne trouve pas cette réunion. Dis « liste mes réunions ».", "display": "Réunion introuvable" }) };
                                let res = m.open(&js::gs(&found, "file"));
                                if !res["session"].is_object() {
                                    return json!({ "say": res["say"], "display": res["say"] });
                                }
                                res["session"].clone()
                            }
                        };
                        let summary = js::gs(&target, "summary");
                        if !summary.is_empty() && !js::gb(&target, "recording") {
                            m.show();
                            let brief = brief_of(&summary);
                            return json!({ "say": if brief.is_empty() { "Voici le résumé de la réunion.".to_string() } else { format!("{} Le résumé complet est dans la card.", js::capitalize(&brief)) }, "display": target["title"] });
                        }
                        m.o.up().emit(json!({ "type": "say", "text": "Je prépare le résumé de la réunion." }));
                        let res = m.summarize(&mut target).await;
                        json!({ "say": res["say"], "display": if js::gb(&res, "ok") { json!("Résumé de la réunion") } else { res["say"].clone() } })
                    }
                }));
            }
        }
        if let Some(op) = open_re().captures(t) {
            let r = pick(&op);
            let detail = r.n.map(|n| format!("n° {n}")).unwrap_or_else(|| if r.words.is_empty() { "dernière".into() } else { r.words.clone() });
            return Some(self.intent("meeting_open", "Réunion", &detail, move |m| {
                let r = r.clone();
                async move {
                    if r.n.is_none() && r.words.is_empty() {
                        if let Some(s) = m.session.borrow().clone() {
                            m.show();
                            return json!({ "say": null, "display": s["title"] });
                        }
                    }
                    let Some(found) = m.find(&r) else {
                        return json!({ "say": format!("Je ne trouve pas de réunion{}. Dis « liste mes réunions ».", if r.words.is_empty() { String::new() } else { format!(" {}", r.words) }), "display": "Réunion introuvable" });
                    };
                    let res = m.open(&js::gs(&found, "file"));
                    json!({ "say": res["say"], "display": found["title"] })
                }
            }));
        }
        None
    }

    /// Jarvis s'arrête : la réunion en cours est arrêtée et archivée tout de suite.
    pub fn shutdown(&self) {
        if self.recording() {
            self.stop("arret");
        }
        self.meet_poll.clear();
        self.done_timer.clear();
        if let Some(mut s) = self.session.borrow().clone().filter(|s| js::gb(s, "dirty") && !js::arr(&s["lines"]).is_empty()) {
            self.save(&mut s);
        }
    }
}

#[allow(unused)]
fn _u(_: HashMap<String, String>) {}
