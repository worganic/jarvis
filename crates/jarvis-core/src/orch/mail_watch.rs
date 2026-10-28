// Surveillance des mails (orchestrator/mail-watch.js) : toutes les N minutes, si Gmail est configuré,
// Jarvis relève les derniers mails et annonce les nouveaux. Premier relevé : il mémorise la boîte
// sans rien annoncer. Déjà vus : data/mails-vus.json. Lecture seule.

use std::cell::{Cell, RefCell};
use std::collections::HashSet;
use std::path::PathBuf;
use std::rc::Rc;

use chrono::{DateTime, Datelike, Local, Timelike};
use serde_json::{json, Value};

use super::mail_filters::address_of;
use super::{OrchRef, Up};
use crate::js::{self, Timer, J};

pub const MAIL_FOOTER: &str = "🚫 : ne plus être prévenu des mails de cet expéditeur";

pub fn sender_name(from: &str) -> String {
    let n = crate::re!(r"<.*>").replace(from, "").replace('"', "").trim().to_string();
    if n.is_empty() {
        from.replace(['<', '>'], "")
    } else {
        n
    }
}

fn key_of(m: &J) -> String {
    format!("{}|{}|{}", js::gs(m, "from"), js::gs(m, "subject"), js::gs(m, "date"))
}

/// Date d'un mail (RFC 2822 ou ISO) en heure locale.
pub fn parse_mail_date(raw: &str) -> Option<DateTime<Local>> {
    let r = raw.trim();
    if r.is_empty() {
        return None;
    }
    let cleaned = crate::re!(r"\s*\([^)]*\)\s*$").replace(r, "");
    if let Ok(d) = DateTime::parse_from_rfc2822(&cleaned) {
        return Some(d.with_timezone(&Local));
    }
    js::parse_date_ms(r).map(js::local_of)
}

/// « 14:46 » aujourd'hui, sinon « 25/09 14:46 ».
pub fn short_date(raw: &str) -> String {
    let Some(d) = parse_mail_date(raw) else { return String::new() };
    let now = Local::now();
    let hm = format!("{:02}:{:02}", d.hour(), d.minute());
    if d.date_naive() == now.date_naive() {
        hm
    } else {
        format!("{:02}/{:02} {hm}", d.day(), d.month())
    }
}

/// Ligne de card d'un mail (« Nouveau mail » et « Derniers emails »).
pub fn mail_row(m: &J, n: Option<usize>) -> J {
    let from = js::gs(m, "from");
    let address = address_of(&from);
    let name = sender_name(&from);
    let who = if !address.is_empty() && !name.is_empty() && name != address {
        format!("{name} <{address}>")
    } else if !address.is_empty() {
        address.clone()
    } else {
        name.clone()
    };
    let uid = if !m["uid"].is_null() { Some(m["uid"].clone()) } else if js::truthy(&m["id"]) { js::num(&m["id"]).map(|f| json!(f as i64)) } else { None };
    let open_ref = uid.or_else(|| n.map(|x| json!(x)));
    let subject = { let s = js::gs(m, "subject"); if s.is_empty() { "(sans objet)".to_string() } else { s } };
    let first = name.split(' ').next().unwrap_or("").to_string();
    let mut row = json!({
        "icon": "✉", "title": format!("{}{subject}", n.map(|x| format!("{x}. ")).unwrap_or_default()), "sub": name, "meta": short_date(&js::gs(m, "date")),
        "say": match n { Some(x) => format!("Ouvre le mail {x}"), None => format!("Cherche les mails de {first}") }
    });
    let mut actions = vec![];
    if let Some(r) = &open_ref {
        row["call"] = json!({ "method": "mailOpen", "args": [r] });
        row["hint"] = json!("Clic : ouvrir ce mail");
        let mut a1 = json!({ "icon": "▣", "title": "Lire ce mail (card de lecture)", "call": { "method": "mailOpen", "args": [r] } });
        let mut a2 = json!({ "icon": "↗", "title": "Ouvrir ce mail dans Gmail", "call": { "method": "mailAction", "args": [r, "gmail"] } });
        if let Some(x) = n {
            a1["say"] = json!(format!("Ouvre le mail {x}"));
            a2["say"] = json!(format!("Ouvre le mail {x} dans Gmail"));
        }
        actions.push(a1);
        actions.push(a2);
    }
    actions.push(json!({
        "icon": "🚫", "tone": "danger", "title": format!("Ne plus m'annoncer les mails de {}", if address.is_empty() { &name } else { &address }),
        "say": format!("N'annonce plus les mails de {who}"), "done": "exclu"
    }));
    row["actions"] = Value::Array(actions);
    row
}

pub struct MailWatch {
    o: OrchRef,
    file: PathBuf,
    log: Rc<dyn Fn(String)>,
    seen: RefCell<Option<Vec<String>>>,
    timer: Timer,
    first: Timer,
    running: Cell<bool>,
}

impl MailWatch {
    pub fn new(o: OrchRef, file: PathBuf, log: Rc<dyn Fn(String)>) -> Self {
        let seen = js::read_json(&file).and_then(|v| v.as_array().map(|a| a.iter().map(js::s).collect()));
        Self { o, file, log, seen: RefCell::new(seen), timer: Timer::new(), first: Timer::new(), running: Cell::new(false) }
    }

    fn config(&self) -> J {
        js::spread(&json!({ "enabled": true, "minutes": 5 }), &self.o.up().config.get("mailWatch"))
    }

    fn save(&self) {
        let seen = self.seen.borrow().clone().unwrap_or_default();
        let skip = seen.len().saturating_sub(300);
        let _ = std::fs::create_dir_all(self.file.parent().unwrap_or(std::path::Path::new(".")));
        let _ = std::fs::write(&self.file, serde_json::to_string(&seen[skip..]).unwrap_or_default());
    }

    /// Un relevé : les nouveaux mails annoncés.
    pub async fn check(&self) -> Vec<J> {
        let o = self.o.up();
        let cfg = self.config();
        if self.running.get() || !js::gb(&cfg, "enabled") || !o.google.is_configured() {
            return vec![];
        }
        self.running.set(true);
        let res = self.check_inner().await;
        self.running.set(false);
        res
    }

    async fn check_inner(&self) -> Vec<J> {
        let o = self.o.up();
        let r = o.google.mail_derniers(&json!({ "limite": 10, "tous": true })).await;
        if !js::gb(&r, "ok") {
            return vec![];
        }
        let mails = js::arr(&r["mails"]);
        if self.seen.borrow().is_none() {
            *self.seen.borrow_mut() = Some(mails.iter().map(key_of).collect());
            self.save();
            return vec![];
        }
        let known: HashSet<String> = self.seen.borrow().as_ref().unwrap().iter().cloned().collect();
        let incoming: Vec<J> = mails.iter().filter(|m| !known.contains(&key_of(m))).cloned().collect();
        {
            let mut s = self.seen.borrow_mut();
            let list = s.as_mut().unwrap();
            for m in &mails {
                let k = key_of(m);
                if !list.contains(&k) {
                    list.push(k);
                }
            }
        }
        if incoming.is_empty() {
            return vec![];
        }
        self.save();
        let excluded: Vec<J> = incoming.iter().filter(|m| o.mail_filters.match_(m).is_some()).cloned().collect();
        let fresh: Vec<J> = incoming.into_iter().filter(|m| !excluded.contains(m)).collect();
        if !excluded.is_empty() {
            let mut senders: Vec<String> = vec![];
            for m in &excluded {
                let s = sender_name(&js::gs(m, "from"));
                if !senders.contains(&s) {
                    senders.push(s);
                }
            }
            senders.truncate(10);
            o.record("mails_non_annonces", json!({ "nombre": excluded.len(), "expediteurs": senders }));
        }
        if fresh.is_empty() {
            return vec![];
        }
        let first = &fresh[0];
        o.mails.remember(&fresh.iter().take(5).cloned().collect::<Vec<_>>());
        let hidden = if excluded.is_empty() {
            String::new()
        } else {
            let s = if excluded.len() > 1 { "s" } else { "" };
            format!("{} autre{s} masqué{s} par tes exclusions · ", excluded.len())
        };
        let subj = { let s = js::gs(first, "subject"); if s.is_empty() { "sans objet".to_string() } else { s } };
        let mut rows: Vec<J> = fresh.iter().take(5).enumerate().map(|(i, m)| mail_row(m, Some(i + 1))).collect();
        rows.push(json!({ "icon": "▤", "title": "Lire mes mails", "say": "Lis mes mails" }));
        o.notify(json!({
            "text": if fresh.len() == 1 { format!("Nouveau mail de {} : {subj}.", sender_name(&js::gs(first, "from"))) } else { format!("{} nouveaux mails, dont un de {} : {subj}.", fresh.len(), sender_name(&js::gs(first, "from"))) },
            "title": if fresh.len() == 1 { "Nouveau mail".to_string() } else { format!("{} nouveaux mails", fresh.len()) },
            "source": "mail",
            "rows": rows,
            "footer": format!("{hidden}{MAIL_FOOTER}")
        }));
        fresh
    }

    pub fn start(&self) {
        let minutes = js::gn_or(&self.config(), "minutes", 5.0).max(2.0);
        let o = self.o.clone();
        self.first.set(60000, move || {
            let o = o.clone();
            js::spawn(async move {
                o.up().mail_watch.check().await;
            });
        });
        let o = self.o.clone();
        self.timer.every((minutes * 60000.0) as u64, move || {
            let o = o.clone();
            js::spawn(async move {
                o.up().mail_watch.check().await;
            });
        });
    }

    pub fn stop(&self) {
        self.first.clear();
        self.timer.clear();
    }

    #[allow(dead_code)]
    fn log(&self, m: &str) {
        (self.log)(m.to_string())
    }
}
