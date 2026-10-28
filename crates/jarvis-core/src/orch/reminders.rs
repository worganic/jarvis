// Rappels persistants (orchestrator/reminders.js) : « rappelle-moi demain à 9 h d'appeler Paul »,
// « dans 20 minutes », « tous les jours à 8 h ». Enregistrés dans data/rappels.json ; un rappel
// manqué pendant que Jarvis était fermé est donné au démarrage (« en retard »).

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use chrono::{Datelike, Duration, Local, NaiveTime, TimeZone, Timelike};
use serde_json::{json, Value};

use super::local_skills::{parse_day, parse_time, DAYS};
use crate::js::{self, Timer, J};

fn trigger(t: &str) -> bool {
    crate::re!(r"^(?:rappelle moi|fais moi penser|n oublie pas de me rappeler|previens moi|mets moi un rappel|ajoute un rappel|cree un rappel|programme un rappel|mets un rappel)\b").is_match(t)
}

fn num(w: &str) -> Option<i64> {
    Some(match w {
        "un" | "une" => 1,
        "deux" => 2,
        "trois" => 3,
        "quatre" => 4,
        "cinq" => 5,
        "six" => 6,
        "sept" => 7,
        "huit" => 8,
        "neuf" => 9,
        "dix" => 10,
        "quinze" => 15,
        "vingt" => 20,
        "trente" => 30,
        "quarante" => 40,
        "cinquante" => 50,
        _ => return None,
    })
}

/// « aujourd'hui à 14 h 30 », « demain à 9 h », « lundi 3 octobre à 18 h ».
pub fn when_label(at: i64) -> String {
    let d = js::local_of(at);
    let today = Local::now().date_naive();
    let diff = (d.date_naive() - today).num_days();
    let hm = crate::re!(r" h 00$").replace(&format!("{} h {:02}", d.hour(), d.minute()), " h").into_owned();
    match diff {
        0 => format!("aujourd'hui à {hm}"),
        1 => format!("demain à {hm}"),
        _ => format!("{} à {hm}", js::fr_long_date(&d)),
    }
}

/// Ce qu'il faut rappeler : la phrase sans ce qui désigne le moment.
pub fn extract_what(raw: &str) -> String {
    let s = crate::re!(r"(?i)^\s*(?:jarvis[\s,]+)?(?:rappelle[- ]moi|fais[- ]moi penser|n['’]oublie pas de me rappeler|préviens[- ]moi|previens[- ]moi|mets[- ]moi un rappel|ajoute un rappel|crée un rappel|cree un rappel|programme un rappel|mets un rappel)\s*").replace(raw, "");
    let s = crate::re!(r"(?i)\b(?:tous les jours|chaque jour|tous les matins|chaque matin|tous les soirs|chaque soir)\b").replace_all(&s, " ");
    let s = crate::re!(r"(?i)\b(?:tous les|chaque)\s+(?:lundis?|mardis?|mercredis?|jeudis?|vendredis?|samedis?|dimanches?)\b").replace_all(&s, " ");
    let s = crate::re!(r"(?i)\bdans\s+(?:un quart d['’]heure|une demi[- ]heure|une heure et demie|\d+|une?|deux|trois|quatre|cinq|six|sept|huit|neuf|dix|quinze|vingt|trente|quarante|cinquante)\s*(?:minutes?|min|heures?|h|secondes?|sec|jours?)?\b").replace_all(&s, " ");
    let s = crate::re!(r"(?i)\b(?:aujourd['’]hui|après-demain|apres-demain|demain|ce soir|ce matin|cet après-midi|cet apres-midi|(?:lundi|mardi|mercredi|jeudi|vendredi|samedi|dimanche)(?: prochain)?|le \d{1,2}(?:er)?(?: \p{L}+)?|\d{1,2}/\d{1,2})\b").replace_all(&s, " ");
    let s = crate::re!(r"(?i)(?:(?:^|\s)(?:à|a|vers)\s+)?\d{1,2}\s*(?:h|heures?|:)\s*\d{0,2}\b").replace_all(&s, " ");
    let s = crate::fre!(r"(?i)(?:^|\s)(?:à|a)\s+(?:midi|minuit)(?=\s|$)").replace_all(&s, " ");
    let s = crate::re!(r"\s+").replace_all(&s, " ").trim().to_string();
    let s = crate::re!(r"(?i)^(?:(?:de|que|pour|à)\s+|d['’]\s*|qu['’]\s*|[,:]\s*)").replace(&s, "");
    let s = crate::re!(r"(?i)(?:\s+(?:à|a|vers|le|pour|de|et))+$").replace(&s, "");
    crate::re!(r"[\s,.!?]+$").replace(&s, "").trim().to_string()
}

fn at_time(date: chrono::NaiveDate, hm: &str) -> i64 {
    let (h, m) = hm.split_once(':').map(|(h, m)| (h.parse().unwrap_or(9), m.parse().unwrap_or(0))).unwrap_or((9, 0));
    let dt = date.and_time(NaiveTime::from_hms_opt(h, m, 0).unwrap_or_default());
    Local.from_local_datetime(&dt).earliest().map(|d| d.timestamp_millis()).unwrap_or_else(js::now_ms)
}

/// Phrase de rappel → { what, at, repeat } ; { missingWhen: true, what } si l'heure manque.
pub fn parse_reminder(t: &str, raw: &str) -> Option<J> {
    if !trigger(t) {
        return None;
    }
    let what = extract_what(raw);
    let now = Local::now();
    let now_ms = now.timestamp_millis();
    let mut repeat = Value::Null;
    let mut at: Option<i64> = None;
    if let Some(m) = crate::re!(r"\bdans (un quart d heure|une demi heure|une heure et demie|\d+|une?|deux|trois|quatre|cinq|six|sept|huit|neuf|dix|quinze|vingt|trente|quarante|cinquante) ?(minutes?|min|heures?|h|secondes?|sec|jours?)?\b").captures(t) {
        let ms = match &m[1] {
            "un quart d heure" => 15 * 60000,
            "une demi heure" => 30 * 60000,
            "une heure et demie" => 90 * 60000,
            w => {
                let n = w.parse::<i64>().ok().or_else(|| num(w)).unwrap_or(1);
                let unit = m.get(2).map(|u| u.as_str()).unwrap_or("minutes");
                n * if unit.starts_with('h') {
                    3_600_000
                } else if unit.starts_with('j') {
                    86_400_000
                } else if unit.starts_with('s') {
                    1000
                } else {
                    60000
                }
            }
        };
        at = Some(now_ms + ms);
    }
    let daily = crate::re!(r"\b(?:tous les jours|chaque jour|tous les matins|chaque matin|tous les soirs|chaque soir)\b").is_match(t);
    let weekly = crate::re!(r"\b(?:tous les|chaque) (lundi|mardi|mercredi|jeudi|vendredi|samedi|dimanche)s?\b").captures(t).map(|c| c[1].to_string());
    if at.is_none() && (daily || weekly.is_some()) {
        let time = parse_time(t).unwrap_or_else(|| if crate::re!(r"\bsoir").is_match(t) { "19:00".into() } else { "09:00".into() });
        let today = now.date_naive();
        let mut a = at_time(today, &time);
        if let Some(w) = &weekly {
            let target = DAYS.iter().position(|d| d == w).unwrap_or(0) as i64;
            let mut diff = (target - today.weekday().num_days_from_sunday() as i64 + 7) % 7;
            if diff == 0 && a <= now_ms {
                diff = 7;
            }
            a = at_time(today + Duration::days(diff), &time);
            repeat = json!({ "every": "week" });
        } else {
            if a <= now_ms {
                a = at_time(today + Duration::days(1), &time);
            }
            repeat = json!({ "every": "day" });
        }
        at = Some(a);
    }
    if at.is_none() {
        let day = parse_day(t);
        let time = parse_time(t);
        if day.is_none() && time.is_none() {
            return Some(json!({ "missingWhen": true, "what": what }));
        }
        let hm = time.unwrap_or_else(|| if crate::re!(r"\bsoir").is_match(t) { "19:00".into() } else { "09:00".into() });
        let base = day.as_ref().map(|d| d.date).unwrap_or_else(|| now.date_naive());
        let mut a = at_time(base, &hm);
        if day.is_none() && a <= now_ms {
            a = at_time(base + Duration::days(1), &hm);
        }
        at = Some(a);
    }
    Some(json!({ "what": what, "at": at, "repeat": repeat }))
}

pub struct Reminders {
    file: PathBuf,
    notify: Rc<dyn Fn(J)>,
    log: Rc<dyn Fn(String)>,
    pub items: RefCell<Vec<J>>,
    timer: Timer,
}

impl Reminders {
    pub fn new(file: PathBuf, notify: Rc<dyn Fn(J)>, log: Rc<dyn Fn(String)>) -> Rc<Self> {
        let items = js::read_json(&file).map(|v| js::arr(&v)).unwrap_or_default();
        Rc::new(Self { file, notify, log, items: RefCell::new(items), timer: Timer::new() })
    }

    fn save(&self) {
        if let Err(e) = js::write_json(&self.file, &Value::Array(self.items.borrow().clone())) {
            (self.log)(format!("rappels : {e}"));
        }
    }

    pub fn add(&self, what: &str, at: i64, repeat: J) -> J {
        let id = self.items.borrow().iter().map(|r| r["id"].as_i64().unwrap_or(0)).max().unwrap_or(0) + 1;
        let item = json!({ "id": id, "what": what, "at": at, "repeat": repeat, "createdAt": js::now_ms() });
        self.items.borrow_mut().push(item.clone());
        self.save();
        item
    }

    /// Rappels à venir, du plus proche au plus lointain.
    pub fn upcoming(&self) -> Vec<J> {
        let mut l: Vec<J> = self.items.borrow().iter().filter(|r| !js::gb(r, "done")).cloned().collect();
        l.sort_by_key(|r| r["at"].as_i64().unwrap_or(0));
        l
    }

    /// Rappels à venir aujourd'hui (briefing).
    pub fn today(&self) -> Vec<J> {
        let now = js::now_ms();
        let end = Local::now().date_naive().and_hms_milli_opt(23, 59, 59, 999).and_then(|d| Local.from_local_datetime(&d).earliest()).map(|d| d.timestamp_millis()).unwrap_or(now);
        self.upcoming().into_iter().filter(|r| {
            let at = r["at"].as_i64().unwrap_or(0);
            at >= now - 60000 && at <= end
        }).collect()
    }

    pub fn cancel(&self, id: i64) -> bool {
        let before = self.items.borrow().len();
        self.items.borrow_mut().retain(|r| r["id"].as_i64() != Some(id));
        let changed = self.items.borrow().len() != before;
        if changed {
            self.save();
        }
        changed
    }

    pub fn cancel_all(&self) -> usize {
        let n = self.upcoming().len();
        self.items.borrow_mut().clear();
        self.save();
        n
    }

    pub fn get(&self, id: i64) -> Option<J> {
        self.items.borrow().iter().find(|r| r["id"].as_i64() == Some(id)).cloned()
    }

    pub fn update(&self, id: i64, patch: J) -> Option<J> {
        let r = {
            let mut items = self.items.borrow_mut();
            let r = items.iter_mut().find(|r| r["id"].as_i64() == Some(id))?;
            js::assign(r, patch);
            r.clone()
        };
        self.save();
        Some(r)
    }

    /// Échéances (toutes les 10 s) ; rappels manqués pendant que Jarvis était fermé.
    pub fn check(&self) {
        let now = js::now_ms();
        let mut notes = vec![];
        let mut changed = false;
        for r in self.items.borrow_mut().iter_mut() {
            let at = r["at"].as_i64().unwrap_or(0);
            if js::gb(r, "done") || at > now {
                continue;
            }
            let late = now - at > 5 * 60000;
            let what_s = js::gs(r, "what");
            let what = if what_s.is_empty() { "tu m’as demandé de te le rappeler".to_string() } else { what_s.clone() };
            let mut first = json!({ "icon": "⏰", "title": if what_s.is_empty() { "Rappel".to_string() } else { what_s.clone() }, "sub": if late { format!("prévu {}", when_label(at)) } else { when_label(at) } });
            if late {
                first["tone"] = json!("warn");
            }
            let mut rows = vec![first];
            if !what_s.is_empty() {
                rows.push(json!({ "icon": "↻", "title": "Me le rappeler dans 10 minutes", "say": format!("Rappelle-moi dans 10 minutes de {what_s}") }));
            }
            notes.push(json!({
                "text": if late { format!("Rappel en retard, prévu {} : {what}.", when_label(at)) } else { format!("Rappel : {what}.") },
                "title": "Rappel", "source": "rappel", "rows": rows
            }));
            if js::truthy(&r["repeat"]) {
                let step = if js::gs(&r["repeat"], "every") == "week" { 7 * 86_400_000 } else { 86_400_000 };
                let mut a = at;
                while a <= now {
                    a += step;
                }
                r["at"] = json!(a);
            } else {
                r["done"] = json!(true);
            }
            changed = true;
        }
        if changed {
            self.items.borrow_mut().retain(|r| !js::gb(r, "done"));
            self.save();
        }
        for n in notes {
            (self.notify)(n);
        }
    }

    pub fn start(self: &Rc<Self>) {
        self.check();
        let me = Rc::downgrade(self);
        self.timer.every(10000, move || {
            if let Some(s) = me.upgrade() {
                s.check();
            }
        });
    }

    pub fn stop(&self) {
        self.timer.clear();
    }
}
