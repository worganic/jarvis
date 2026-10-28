// Briefing du matin (orchestrator/briefing.js) : « fais-moi le briefing », ou tout seul au premier
// « bonjour » de la matinée (config.briefing.auto). Date, météo, agenda, rappels du jour, mails récents,
// projets à commiter, Claude Code qui attendent, quota IA. Chaque source est bornée à 6 s.

use std::cell::RefCell;
use std::future::Future;
use std::path::PathBuf;

use chrono::{Local, Timelike};
use serde_json::{json, Value};

use super::intent::Intent;
use super::local_skills;
use super::reminders::when_label;
use super::{OrchRef, Up};
use crate::js::{self, J};

async fn within<T>(f: impl Future<Output = T>) -> Option<T> {
    tokio::time::timeout(std::time::Duration::from_millis(6000), f).await.ok()
}

fn plural(n: usize, word: &str) -> String {
    format!("{n} {word}{}", if n > 1 { "s" } else { "" })
}

fn sender(from: &J) -> String {
    let f = js::s(from);
    let f = crate::re!(r"<.*>").replace(&f, "");
    f.replace('"', "").trim().to_string()
}

pub struct Briefing {
    o: OrchRef,
    file: PathBuf,
    last_day: RefCell<String>,
}

struct Gathered {
    weather: Option<J>,
    events: Option<Vec<J>>,
    reminders: Vec<J>,
    mails: Option<Vec<J>>,
    dirty: Vec<J>,
    claude: Vec<J>,
    asking: Vec<String>,
    usage: J,
}

impl Briefing {
    pub fn new(o: OrchRef, file: PathBuf) -> Self {
        let last = js::read_json(&file).map(|d| js::gs(&d, "lastDay")).unwrap_or_default();
        Self { o, file, last_day: RefCell::new(last) }
    }

    pub fn today() -> String {
        Local::now().format("%Y-%m-%d").to_string()
    }

    pub fn done_today(&self) -> bool {
        *self.last_day.borrow() == Self::today()
    }

    pub fn mark_done(&self) {
        *self.last_day.borrow_mut() = Self::today();
        let _ = js::write_json(&self.file, &json!({ "lastDay": Self::today() }));
    }

    /// Briefing automatique au premier bonjour : le matin (5 h – 12 h), une fois par jour, si activé.
    pub fn wants_auto(&self) -> bool {
        let cfg = self.o.up().config.get("briefing");
        let h = Local::now().hour();
        cfg["auto"] != json!(false) && (5..12).contains(&h) && !self.done_today()
    }

    async fn gather(&self) -> Gathered {
        let o = self.o.up();
        let city = o.status.d.borrow().weather_city.clone().unwrap_or_default();
        let google = o.google.clone();
        let projects = o.projects.clone();
        let configured = google.is_configured();
        let (weather, agenda, mails, recent) = futures_util::join!(
            async {
                // Service trop lent : le dernier relevé daté vaut mieux qu'un briefing sans météo.
                if city.is_empty() { None } else { within(local_skills::weather(&city, None)).await.and_then(|r| r.ok()).or_else(|| local_skills::weather_from_cache(&city, None)) }
            },
            async { within(google.agenda_jour(&json!({}))).await },
            async {
                if configured { within(google.mail_derniers(&json!({ "limite": 10 }))).await } else { None }
            },
            async { within(projects.recent(8)).await }
        );
        let (claude, asking) = o.tmux_voice.claude_sessions();
        let usage = o.status.d.borrow().usage.clone();
        let since = js::now_ms() - 24 * 3_600_000;
        let mails = mails.filter(|m| js::gb(m, "ok")).map(|m| {
            js::arr(&m["mails"])
                .into_iter()
                .filter(|x| js::parse_date_ms(&js::gs(x, "date")).map(|t| t >= since).unwrap_or(true))
                .collect()
        });
        Gathered {
            weather: weather.filter(|w| js::gb(w, "ok")),
            events: agenda.filter(|a| js::gb(a, "ok")).map(|a| js::arr(&a["evenements"])),
            reminders: o.reminders.today(),
            mails,
            dirty: recent.unwrap_or_default().into_iter().filter(|p| js::gn_or(&p["git"], "changes", 0.0) > 0.0).collect(),
            claude,
            asking,
            usage,
        }
    }

    pub async fn build(&self) -> (String, Vec<J>) {
        let d = self.gather().await;
        let call = js::gs(&self.o.up().addressing(), "call");
        let now = Local::now();
        let evening = now.hour() >= 18;
        let date = js::fr_long_date(&now);
        let hm = crate::re!(r" 00$").replace(&format!("{} heures {:02}", now.hour(), now.minute()), "").into_owned();
        let mut say = vec![format!("{}{}. Nous sommes {date}, il est {hm}.", if evening { "Bonsoir" } else { "Bonjour" }, if call.is_empty() { String::new() } else { format!(" {call}") })];
        let mut rows: Vec<J> = vec![];

        if let Some(w) = &d.weather {
            say.push(js::gs(w, "say"));
            let title = if js::truthy(&w["display"]) { w["display"].clone() } else { w["say"].clone() };
            rows.push(json!({ "section": "Météo" }));
            rows.push(json!({ "icon": "☀", "title": title, "say": "Quel temps fait-il ?" }));
        }
        if let Some(events) = &d.events {
            let key = |e: &J| if js::truthy(&e["heure"]) { js::gs(e, "heure") } else { js::gs(e, "debut") };
            let mut ev = events.clone();
            ev.sort_by_key(key);
            let at = match ev.first() {
                Some(e) if js::truthy(&e["heure"]) => {
                    let h = js::gs(e, "heure").replacen(':', " heures ", 1);
                    format!(" à {}", crate::re!(r" heures 00$").replace(&h, " heures"))
                }
                _ => String::new(),
            };
            say.push(if ev.is_empty() { "Rien dans ton agenda aujourd'hui.".to_string() } else { format!("Aujourd'hui, {} rendez-vous : le premier{at}, {}.", ev.len(), js::gs(&ev[0], "titre")) });
            rows.push(json!({ "section": "Agenda" }));
            if ev.is_empty() {
                rows.push(json!({ "icon": "▦", "title": "Aucun rendez-vous" }));
            }
            for e in ev.iter().take(6) {
                rows.push(json!({ "icon": "▦", "title": if js::truthy(&e["titre"]) { js::gs(e, "titre") } else { "Rendez-vous".into() }, "sub": js::gs(e, "lieu"), "meta": js::gs(e, "heure") }));
            }
        }
        if !d.reminders.is_empty() {
            let what = |r: &J| if js::truthy(&r["what"]) { js::gs(r, "what") } else { "rappel".into() };
            let at = |r: &J| when_label(r["at"].as_i64().unwrap_or(0));
            let first: Vec<String> = d.reminders.iter().take(2).map(|r| format!("{}, {}", what(r), at(r).replacen("aujourd'hui ", "", 1))).collect();
            say.push(format!("{} aujourd'hui : {}.", plural(d.reminders.len(), "rappel"), first.join(" ; ")));
            rows.push(json!({ "section": "Rappels" }));
            for r in d.reminders.iter().take(6) {
                rows.push(json!({ "icon": "⏰", "title": if js::truthy(&r["what"]) { js::gs(r, "what") } else { "Rappel".into() }, "meta": at(r).replacen("aujourd'hui à ", "", 1) }));
            }
        }
        if let Some(mails) = &d.mails {
            say.push(if mails.is_empty() {
                "Pas de nouveau mail depuis hier.".to_string()
            } else {
                let s = sender(&mails[0]["from"]);
                format!("{} depuis hier, dont un de {}.", plural(mails.len(), "mail"), if s.is_empty() { "quelqu’un".into() } else { s })
            });
            rows.push(json!({ "section": "Mails" }));
            for m in mails.iter().take(4) {
                rows.push(json!({ "icon": "✉", "title": if js::truthy(&m["subject"]) { js::gs(m, "subject") } else { "(sans objet)".into() }, "sub": sender(&m["from"]) }));
            }
            if !mails.is_empty() {
                rows.push(json!({ "icon": "▤", "title": "Lire mes mails", "say": "Lis mes mails" }));
            }
        }
        if !d.dirty.is_empty() {
            let changes = |p: &J| js::gn_or(&p["git"], "changes", 0.0) as usize;
            say.push(if d.dirty.len() == 1 {
                format!("Côté projets, {} a {} non commités.", js::gs(&d.dirty[0], "name"), plural(changes(&d.dirty[0]), "fichier"))
            } else {
                format!("Côté projets, {} projets ont des modifications non commitées, dont {}.", d.dirty.len(), js::gs(&d.dirty[0], "name"))
            });
            rows.push(json!({ "section": "Projets à commiter" }));
            for p in d.dirty.iter().take(5) {
                let name = js::gs(p, "name");
                rows.push(json!({ "icon": "!", "tone": "warn", "title": name, "sub": format!("{} non commités", plural(changes(p), "fichier")), "say": format!("Où en est le projet {name} ?") }));
            }
        }
        if !d.claude.is_empty() {
            let asking: Vec<&J> = d.claude.iter().filter(|i| d.asking.contains(&js::gs(i, "target"))).collect();
            say.push(if !asking.is_empty() {
                let labels: Vec<String> = asking.iter().map(|i| js::gs(i, "label")).collect();
                format!("{} attend{} ta réponse : {}.", plural(asking.len(), "Claude Code"), if asking.len() > 1 { "ent" } else { "" }, labels.join(", "))
            } else {
                format!("{} ouvert{} dans psmux.", plural(d.claude.len(), "Claude Code"), if d.claude.len() > 1 { "s" } else { "" })
            });
            rows.push(json!({ "section": "Claude Code" }));
            for i in &d.claude {
                let waiting = d.asking.contains(&js::gs(i, "target"));
                let label = js::gs(i, "label");
                let mut row = json!({ "icon": "◈", "title": label, "sub": if waiting { "attend ta réponse" } else { "ouvert" }, "say": format!("Parle à Claude {label}") });
                if waiting {
                    row["tone"] = json!("warn");
                }
                rows.push(row);
            }
        }
        if let Some(pct) = js::num(&d.usage["five"]["pct"]) {
            if pct >= 50.0 {
                say.push(format!("Tu as utilisé {} % de ton quota Claude sur 5 heures.", js::num_str(pct)));
            }
        }
        say.push(if evening { "Bonne soirée !" } else { "Bonne journée !" }.into());
        (say.join(" "), rows)
    }

    /// Intention du routeur (« fais-moi le briefing », ou premier bonjour du matin).
    pub fn intent(&self, prefix: &str) -> Intent {
        let o = self.o.clone();
        let prefix = prefix.to_string();
        Intent::new("briefing", "Briefing", "point du jour", move || {
            let o = o.clone();
            let prefix = prefix.clone();
            async move {
                let orch = o.up();
                let (say, rows) = orch.briefing.build().await;
                orch.briefing.mark_done();
                orch.show_widget("tool", json!({ "title": "Briefing du jour", "tool": "briefing", "rows": rows, "ttl": 180000, "footer": js::fr_long_date(&Local::now()) }));
                json!({ "say": if prefix.is_empty() { say } else { format!("{prefix} {say}") }, "display": "Briefing du jour" })
            }
        })
    }
}

pub fn is_briefing(t: &str) -> bool {
    crate::re!(r"^(?:fais moi |donne moi |lance |dis moi )?(?:le |mon |un )?(?:briefing|brief|point du jour|point de la journee|resume du jour|recap du jour|recapitulatif du jour)(?: du matin| du jour| de la journee)?$|^(?:quoi de neuf|qu est ce qui m attend)(?: aujourd hui| ce matin)?$").is_match(t)
}

pub fn is_greeting(t: &str) -> bool {
    crate::re!(r"^(?:bonjour|salut|hello|coucou|bon matin|hey)(?: jarvis)?$").is_match(t)
}

#[allow(unused)]
fn _unused(_: Value) {}
