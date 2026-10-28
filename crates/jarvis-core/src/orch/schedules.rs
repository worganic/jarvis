// Programmations (orchestrator/schedules.js) : une action déclenchée par un compte à rebours, une heure
// ou la fin de Claude Code. RÈGLE FONDATRICE : une action SENSIBLE (éteindre, redémarrer, veille,
// hibernation, verrouillage, commande qui ferme ou supprime) n'est jamais exécutée avant 120 secondes
// AFFICHÉES DANS LE CŒUR, annulables à tout moment ; sans affichage confirmé sous 15 s, elle est
// annulée ; Jarvis arrêté pendant le décompte : rien n'est exécuté. Stockage : data/programmations.json.

use std::cell::RefCell;
use std::path::PathBuf;
use std::rc::Rc;

use chrono::{Duration, Local, NaiveTime, TimeZone, Timelike};
use serde_json::{json, Value};

use super::local_skills::{parse_day, parse_time};
use super::reminders::extract_what;
use super::{OrchRef, Up};
use crate::js::{self, Timer, J};

pub const COUNTDOWN_S: i64 = 120;
const DISPLAY_WAIT_MS: i64 = 15000;
const ANNOUNCE_AT: [i64; 3] = [60, 30, 10];
const MAX_DELAY_MS: i64 = 7 * 86_400_000;
pub const CARD_TITLE: &str = "Compte à rebours";
const LATE_MS: i64 = 5 * 60000;

fn fold(s: &str) -> String {
    let t = js::fold_lower(s);
    let t = crate::re!(r#"['’\-?!,;:.…«»"]"#).replace_all(&t, " ");
    crate::re!(r"\s+").replace_all(&t, " ").trim().to_string()
}

/// (libellé, verbe, phrase, sensible : "oui" | "non" | "auto")
pub fn action_def(t: &str) -> Option<(&'static str, &'static str, &'static str, &'static str)> {
    Some(match t {
        "message" => ("Afficher un message", "", "", "non"),
        "speak" => ("Dire un message à voix haute", "", "", "non"),
        "shutdown" => ("Éteindre l’ordinateur", "EXTINCTION", "L’ordinateur va s’éteindre", "oui"),
        "restart" => ("Redémarrer l’ordinateur", "REDÉMARRAGE", "L’ordinateur va redémarrer", "oui"),
        "sleep" => ("Mettre l’ordinateur en veille", "VEILLE", "L’ordinateur va se mettre en veille", "oui"),
        "hibernate" => ("Hiberner l’ordinateur", "HIBERNATION", "L’ordinateur va hiberner", "oui"),
        "lock" => ("Verrouiller la session", "VERROUILLAGE", "La session va être verrouillée", "oui"),
        "command" => ("Lancer une commande de Jarvis", "COMMANDE", "Une commande va être lancée", "auto"),
        _ => return None,
    })
}

pub fn is_sensitive(action: &J) -> bool {
    match action_def(&js::gs(action, "type")) {
        None => true,
        Some((_, _, _, "auto")) => crate::re!(r"\b(?:ferme|fermer|quitte|quitter|supprime|supprimer|efface|effacer|eteins|eteindre|redemarre|veille|hiberne|verrouille|arrete|arreter|deconnecte|deconnexion|tue|vide|vider|formate)\b").is_match(&fold(&js::gs(action, "phrase"))),
        Some((_, _, _, s)) => s == "oui",
    }
}

/// Action système exécutée après le compte à rebours. JARVIS_SCHEDULE_DRY_RUN : rien n'est exécuté.
pub async fn os_action(typ: &str) -> J {
    if js::env("JARVIS_SCHEDULE_DRY_RUN").is_some() {
        return json!({ "ok": true, "dry": true, "action": typ });
    }
    let (exe, args): (&str, Vec<&str>) = match typ {
        "shutdown" => ("shutdown.exe", vec!["/s", "/t", "10", "/c", "Jarvis : extinction programmée"]),
        "restart" => ("shutdown.exe", vec!["/r", "/t", "10", "/c", "Jarvis : redémarrage programmé"]),
        "hibernate" => ("shutdown.exe", vec!["/h"]),
        "sleep" => ("powershell.exe", vec!["-NoProfile", "-NonInteractive", "-Command", "Add-Type -AssemblyName System.Windows.Forms; [System.Windows.Forms.Application]::SetSuspendState([System.Windows.Forms.PowerState]::Suspend, $false, $false)"]),
        "lock" => ("rundll32.exe", vec!["user32.dll,LockWorkStation"]),
        _ => return json!({ "ok": false, "error": format!("action inconnue : {typ}") }),
    };
    match crate::proc::run(exe, &args.iter().map(|s| s.to_string()).collect::<Vec<_>>(), None, 30000).await {
        Ok(o) if o.code == 0 => json!({ "ok": true, "action": typ }),
        Ok(o) => json!({ "ok": false, "error": if o.stderr.trim().is_empty() { format!("code {}", o.code) } else { o.stderr.trim().to_string() } }),
        Err(e) => json!({ "ok": false, "error": e }),
    }
}

fn num(w: &str) -> Option<f64> {
    Some(match w {
        "un" | "une" => 1.0,
        "deux" => 2.0,
        "trois" => 3.0,
        "quatre" => 4.0,
        "cinq" => 5.0,
        "six" => 6.0,
        "sept" => 7.0,
        "huit" => 8.0,
        "neuf" => 9.0,
        "dix" => 10.0,
        "quinze" => 15.0,
        "vingt" => 20.0,
        "trente" => 30.0,
        "quarante" => 40.0,
        "cinquante" => 50.0,
        "soixante" => 60.0,
        _ => return None,
    })
}

/// « dans 30 minutes », « dans une heure et demie », « dans 2 heures et quart » → millisecondes.
pub fn parse_delay(t: &str) -> Option<i64> {
    let m = crate::re!(r"\bdans (un quart d heure|une demi heure|une heure et demie|\d+|une?|deux|trois|quatre|cinq|six|sept|huit|neuf|dix|quinze|vingt|trente|quarante|cinquante|soixante) ?(minutes?|min|heures?|h|secondes?|sec|jours?)?(?: et (demie?|quart))?\b").captures(t)?;
    match &m[1] {
        "un quart d heure" => return Some(15 * 60000),
        "une demi heure" => return Some(30 * 60000),
        "une heure et demie" => return Some(90 * 60000),
        _ => {}
    }
    let n = m[1].parse::<f64>().ok().or_else(|| num(&m[1])).unwrap_or(1.0);
    let u = m.get(2).map(|x| x.as_str()).unwrap_or("");
    let unit = if u.starts_with('h') {
        3_600_000.0
    } else if u.starts_with('j') {
        86_400_000.0
    } else if u.starts_with('s') {
        1000.0
    } else {
        60000.0
    };
    let extra = m.get(3).map(|x| if x.as_str().starts_with("demi") { 0.5 } else { 0.25 }).unwrap_or(0.0);
    Some(((n + extra) * unit).round() as i64)
}

fn local_ms(d: chrono::NaiveDate, hm: &str) -> i64 {
    let (h, m) = hm.split_once(':').map(|(h, m)| (h.parse().unwrap_or(0), m.parse().unwrap_or(0))).unwrap_or((0, 0));
    Local.from_local_datetime(&d.and_time(NaiveTime::from_hms_opt(h, m, 0).unwrap_or_default())).earliest().map(|x| x.timestamp_millis()).unwrap_or(0)
}

/// Échéance absolue : « à 23 heures », « demain à 7 h 30 » (l'heure est indispensable).
pub fn parse_at(t: &str) -> Option<i64> {
    let time = parse_time(t)?;
    let day = parse_day(t);
    let today = Local::now().date_naive();
    let base = day.as_ref().map(|d| d.date).unwrap_or(today);
    let mut at = local_ms(base, &time);
    if day.is_none() && at <= js::now_ms() {
        at = local_ms(base + Duration::days(1), &time);
    }
    Some(at)
}

const CLAUDE_CLAUSE: &str = r"(?iu)(?:quand|lorsque|d[eè]s que|une fois que|apr[eè]s que)\s+[^,;:]*?\bclaude\b[^,;:]*?(?:termin\p{L}*|fini\p{L}*|plus (?:rien|d['’]action\p{L}*|de travail)[^,;:]*)(?:\s+et\s+qu['’]il\s+n['’]y\s+a\s+plus\s+(?:d['’]action\p{L}*|rien\s+à\s+faire|de\s+travail))?";
const PC: &str = r"(?:l |le |mon |ce |cet )?(?:ordinateur|ordi|pc|machine|windows|poste)(?: portable)?";

/// Phrase → { trigger, action } | { error } | None.
pub fn parse_schedule(t: &str, raw: &str) -> Option<J> {
    let claude = regex::Regex::new(CLAUDE_CLAUSE).unwrap();
    let has_claude = claude.is_match(raw);
    let trigger = if has_claude {
        json!({ "type": "claude_done" })
    } else if let Some(delay) = parse_delay(t) {
        if !(1000..=MAX_DELAY_MS).contains(&delay) {
            return Some(json!({ "error": if delay < 1000 { "Durée trop courte." } else { "Durée trop longue : 7 jours au plus." } }));
        }
        json!({ "type": "countdown", "delayMs": delay })
    } else if let Some(at) = parse_at(t) {
        json!({ "type": "at", "dueAt": at })
    } else {
        return None;
    };
    let rest = crate::re!(r"(?i)^\s*(?:jarvis[\s,]+)?(?:programme|planifie|programmer|planifier)\s*(?:moi\s*)?(?:une?\s+)?(?:action\s*)?[:,]?\s*").replace(raw, "");
    let rest = claude.replace(&rest, " ");
    let rest = crate::re!(r"(?i)(\b(?:heures?|minutes?|h|min)\s+)et\s+(?:demie?|quart)\b").replace_all(&rest, "$1");
    let rest = extract_what(&crate::re!(r"(?i)^\s*(?:jarvis[\s,]+)").replace(&rest, ""));
    let f = fold(&rest);
    let re = |p: String| regex::Regex::new(&p).unwrap();
    let pc_word = re(format!(r"\b{PC}\b"));
    let default_text = if has_claude { "Claude a terminé." } else { "C’est l’heure." };
    let action = if re(format!(r"^(?:eteins|eteindre|arrete|arreter|coupe|shutdown)(?: moi)? {PC}$")).is_match(&f) || (crate::re!(r"^(?:eteins|eteindre)$").is_match(&f) && has_claude) {
        json!({ "type": "shutdown" })
    } else if re(format!(r"^(?:redemarre|redemarrer|relance|reboot)(?: moi)? {PC}$")).is_match(&f) {
        json!({ "type": "restart" })
    } else if re(format!(r"^(?:hiberne|hiberner)(?: moi)?(?: {PC})?$")).is_match(&f) || re(format!(r"^mets?(?: moi)? {PC} en hibernation$")).is_match(&f) {
        json!({ "type": "hibernate" })
    } else if re(format!(r"^(?:mets?|met|passe)(?: moi)? {PC} en veille$")).is_match(&f) || re(format!(r"^(?:mise en veille|veille)(?: {PC})?$")).is_match(&f) || re(format!(r"^endors(?: moi)? {PC}$")).is_match(&f) {
        json!({ "type": "sleep" })
    } else if re(format!(r"^verrouille(?: moi)?(?: {PC}| la session| l ecran| ma session| mon ecran)?$")).is_match(&f) {
        json!({ "type": "lock" })
    } else if crate::re!(r"^(?:affiche|montre|ecris)(?: moi)?(?: un| le| ce)? message\b").is_match(&f) {
        let text = crate::re!(r"(?i)^.*?\bmessage\b\s*(?:disant|qui dit|suivant|:)?\s*[:,]?\s*").replace(&rest, "");
        let text = crate::re!(r#"^[«"“]\s*|\s*[»"”]$"#).replace_all(&text, "").trim().to_string();
        json!({ "type": "message", "text": if text.is_empty() { default_text.to_string() } else { text } })
    } else if crate::re!(r"^(?:dis|dis moi|annonce|prononce|lis moi)\b").is_match(&f) {
        let text = crate::re!(r"(?i)^(?:dis(?:[- ]moi)?|annonce|prononce|lis[- ]moi)\s*").replace(&rest, "");
        let text = crate::re!(r#"^[«"“]\s*|\s*[»"”]$"#).replace_all(&text, "").trim().to_string();
        json!({ "type": "speak", "text": if text.is_empty() { default_text.to_string() } else { text } })
    } else if crate::re!(r"^(?:previens moi|prevenez moi|avertis moi|previens moi a voix haute)$").is_match(&f) {
        json!({ "type": "speak", "text": default_text })
    } else if crate::re!(r"^(?:lance|lancer|ouvre|ouvrir|ferme|fermer|mets|met|coupe|baisse|monte|active|desactive|joue|arrete|pause|passe|cherche|envoie|execute|fais|demarre|relance|reprends|lis)\b").is_match(&f) && !pc_word.is_match(&f) {
        json!({ "type": "command", "phrase": rest.trim() })
    } else {
        return None;
    };
    Some(json!({ "trigger": trigger, "action": action }))
}

pub fn action_text(action: &J) -> String {
    let t = js::gs(action, "type");
    let Some(a) = action_def(&t) else { return "Action inconnue".into() };
    match t.as_str() {
        "message" => format!("Afficher « {} »", js::gs(action, "text")),
        "speak" => format!("Dire « {} »", js::gs(action, "text")),
        "command" => format!("Lancer « {} »", js::gs(action, "phrase")),
        _ => a.0.to_string(),
    }
}

pub fn when_label(at: i64) -> String {
    let d = js::local_of(at);
    let diff = (d.date_naive() - Local::now().date_naive()).num_days();
    let hm = crate::re!(r" h 00$").replace(&format!("{} h {:02}", d.hour(), d.minute()), " h").into_owned();
    match diff {
        0 => format!("à {hm}"),
        1 => format!("demain à {hm}"),
        _ => {
            use chrono::Datelike;
            format!("le {} {} à {hm}", d.day(), js::MOIS[d.month0() as usize])
        }
    }
}

pub fn remaining_label(ms: i64) -> String {
    let s = ((ms as f64) / 1000.0).round().max(0.0) as i64;
    if s >= 3600 {
        format!("{} h {:02} min", s / 3600, (s % 3600) / 60)
    } else if s >= 120 {
        format!("{} min", (s as f64 / 60.0).round() as i64)
    } else {
        format!("{s} s")
    }
}

pub struct Schedules {
    o: OrchRef,
    file: PathBuf,
    log: Rc<dyn Fn(String)>,
    pub items: RefCell<Vec<J>>,
    timer: Timer,
}

impl Schedules {
    pub fn new(o: OrchRef, file: PathBuf, log: Rc<dyn Fn(String)>) -> Self {
        let items = js::read_json(&file).map(|d| js::arr(&d["items"])).unwrap_or_default();
        Self { o, file, log, items: RefCell::new(items), timer: Timer::new() }
    }

    fn save(&self) {
        if let Err(e) = js::write_json(&self.file, &json!({ "version": 1, "items": Value::Array(self.items.borrow().clone()) })) {
            (self.log)(format!("programmations : {e}"));
        }
    }

    fn remove_id(&self, id: i64) {
        self.items.borrow_mut().retain(|i| i["id"].as_i64() != Some(id));
    }

    pub fn add(&self, trigger: &J, action: &J) -> J {
        let typ = js::gs(action, "type");
        if !js::truthy(trigger) || action_def(&typ).is_none() {
            return json!({ "ok": false, "error": "Programmation invalide." });
        }
        let action = match typ.as_str() {
            "message" | "speak" => {
                let text = js::slice_chars(js::gs(action, "text").trim(), 300);
                if text.is_empty() {
                    return json!({ "ok": false, "error": "Il manque le texte du message." });
                }
                json!({ "type": typ, "text": text })
            }
            "command" => {
                let phrase = js::slice_chars(js::gs(action, "phrase").trim(), 300);
                if phrase.is_empty() {
                    return json!({ "ok": false, "error": "Il manque la commande à lancer." });
                }
                json!({ "type": "command", "phrase": phrase })
            }
            _ => json!({ "type": typ }),
        };
        let now = js::now_ms();
        let trig = match js::gs(trigger, "type").as_str() {
            "countdown" => {
                let delay = js::num(&trigger["delayMs"]).unwrap_or(0.0) as i64;
                if !(1000..=MAX_DELAY_MS).contains(&delay) {
                    return json!({ "ok": false, "error": "Durée invalide (1 seconde à 7 jours)." });
                }
                json!({ "type": "countdown", "delayMs": delay, "dueAt": now + delay })
            }
            "at" => {
                let at = js::num(&trigger["dueAt"]).unwrap_or(0.0) as i64;
                if at <= now {
                    return json!({ "ok": false, "error": "Cette heure est déjà passée." });
                }
                json!({ "type": "at", "dueAt": at })
            }
            "claude_done" => {
                let mut t = json!({ "type": "claude_done" });
                if js::truthy(&trigger["project"]) {
                    t["project"] = json!(js::slice_chars(&js::gs(trigger, "project"), 60));
                }
                t
            }
            _ => return json!({ "ok": false, "error": "Déclencheur inconnu." }),
        };
        let id = self.items.borrow().iter().map(|i| i["id"].as_i64().unwrap_or(0)).max().unwrap_or(0) + 1;
        let item = json!({ "id": id, "trigger": trig, "action": action, "status": "armed", "createdAt": now });
        self.items.borrow_mut().push(item.clone());
        self.save();
        self.o.up().record(
            "programmation_creee",
            json!({ "id": id, "declencheur": trig["type"], "action": action["type"], "sensible": is_sensitive(&action), "echeance": trig["dueAt"].as_i64().map(js::iso_of) }),
        );
        json!({ "ok": true, "item": item })
    }

    pub fn upcoming(&self) -> Vec<J> {
        let mut l: Vec<J> = self.items.borrow().iter().filter(|i| js::gs(i, "status") == "armed").cloned().collect();
        l.sort_by(|a, b| {
            let da = a["trigger"]["dueAt"].as_i64().unwrap_or(i64::MAX);
            let db = b["trigger"]["dueAt"].as_i64().unwrap_or(i64::MAX);
            da.cmp(&db).then(a["id"].as_i64().cmp(&b["id"].as_i64()))
        });
        l
    }

    pub fn active_countdown(&self) -> Option<J> {
        self.items.borrow().iter().find(|i| matches!(js::gs(i, "status").as_str(), "waiting_display" | "counting")).cloned()
    }

    pub fn when_text(&self, item: &J) -> String {
        let t = &item["trigger"];
        if js::gs(t, "type") == "claude_done" {
            let p = js::gs(t, "project");
            return format!("quand Claude a terminé{}", if p.is_empty() { String::new() } else { format!(" ({p})") });
        }
        let due = t["dueAt"].as_i64().unwrap_or(0);
        format!("{} · dans {}", when_label(due), remaining_label(due - js::now_ms()))
    }

    pub fn describe(&self, item: &J) -> J {
        json!({
            "id": item["id"], "status": item["status"], "action": item["action"], "actionLabel": action_text(&item["action"]),
            "sensitive": is_sensitive(&item["action"]), "trigger": item["trigger"], "when": self.when_text(item),
            "endsAt": if js::truthy(&item["endsAt"]) { item["endsAt"].clone() } else { Value::Null }
        })
    }

    pub fn cancel(&self, id: i64, reason: &str) -> bool {
        let Some(item) = self.items.borrow().iter().find(|i| i["id"].as_i64() == Some(id)).cloned() else { return false };
        if matches!(js::gs(&item, "status").as_str(), "waiting_display" | "counting") {
            return self.end_countdown(&item, reason).is_some();
        }
        self.remove_id(id);
        self.save();
        self.o.up().record("programmation_annulee", json!({ "id": id, "action": item["action"]["type"], "motif": reason }));
        true
    }

    pub fn cancel_all(&self) -> usize {
        let ids: Vec<i64> = self.items.borrow().iter().filter_map(|i| i["id"].as_i64()).collect();
        let n = ids.len();
        for id in ids {
            self.cancel(id, "toutes annulées");
        }
        n
    }

    /// Annule le compte à rebours affiché ; la programmation annulée.
    pub fn cancel_countdown(&self, reason: &str) -> Option<J> {
        let item = self.active_countdown()?;
        self.end_countdown(&item, reason)
    }

    fn end_countdown(&self, item: &J, reason: &str) -> Option<J> {
        let id = item["id"].as_i64().unwrap_or(0);
        self.remove_id(id);
        self.save();
        let o = self.o.up();
        o.emit(json!({ "type": "schedule_countdown_end", "id": id, "reason": reason }));
        o.emit(json!({ "type": "widget_close", "key": format!("tool:{CARD_TITLE}") }));
        o.record("programmation_annulee", json!({ "id": id, "action": item["action"]["type"], "motif": reason, "enCompteARebours": true }));
        Some(item.clone())
    }

    fn set_item(&self, item: &J) {
        let id = item["id"].as_i64();
        if let Some(i) = self.items.borrow_mut().iter_mut().find(|i| i["id"].as_i64() == id) {
            *i = item.clone();
        }
    }

    pub fn tick(&self) {
        let now = js::now_ms();
        let items = self.items.borrow().clone();
        for mut item in items {
            let status = js::gs(&item, "status");
            let due = item["trigger"]["dueAt"].as_i64();
            if status == "armed" && due.map(|d| d <= now).unwrap_or(false) {
                self.fire(item.clone(), now - due.unwrap() > LATE_MS);
            } else if status == "waiting_display" && now - item["startedAt"].as_i64().unwrap_or(now) > DISPLAY_WAIT_MS {
                self.end_countdown(&item, "compte à rebours non affiché");
                self.o.up().notify(json!({ "text": format!("Je n'ai pas pu afficher le compte à rebours : « {} » est annulé.", action_text(&item["action"])), "title": "Programmation", "source": "programmation" }));
            } else if status == "counting" {
                let ends = item["endsAt"].as_i64().unwrap_or(now);
                let left = ((ends - now) as f64 / 1000.0).ceil() as i64;
                let mut announced: Vec<i64> = item["announced"].as_array().map(|a| a.iter().filter_map(|x| x.as_i64()).collect()).unwrap_or_default();
                for s in ANNOUNCE_AT {
                    if left <= s && left > 0 && !announced.contains(&s) {
                        announced.push(s);
                        item["announced"] = json!(announced);
                        self.set_item(&item);
                        let say = action_def(&js::gs(&item["action"], "type")).map(|a| a.2).unwrap_or("");
                        self.o.up().emit(json!({ "type": "say", "text": format!("{say} dans {s} secondes. Dis annule pour l'arrêter.") }));
                        break;
                    }
                }
                if now >= ends {
                    let o = self.o.clone();
                    let it = item.clone();
                    js::spawn(async move {
                        o.up().schedules.execute(it, false).await;
                    });
                }
            }
        }
    }

    /// Claude a terminé dans tmux. all_idle : plus aucun Claude ne travaille.
    pub fn on_claude_done(&self, label: &str, all_idle: bool) -> usize {
        if !all_idle {
            return 0;
        }
        let items = self.items.borrow().clone();
        let mut n = 0;
        for item in items {
            if js::gs(&item, "status") != "armed" || js::gs(&item["trigger"], "type") != "claude_done" {
                continue;
            }
            let p = js::gs(&item["trigger"], "project");
            if !p.is_empty() && !fold(label).contains(&fold(&p)) {
                continue;
            }
            self.fire(item, false);
            n += 1;
        }
        n
    }

    fn fire(&self, mut item: J, late: bool) {
        let o = self.o.up();
        o.record("programmation_declenchee", json!({ "id": item["id"], "action": item["action"]["type"], "declencheur": item["trigger"]["type"], "enRetard": late }));
        if !is_sensitive(&item["action"]) {
            let o2 = self.o.clone();
            js::spawn(async move {
                o2.up().schedules.execute(item, late).await;
            });
            return;
        }
        item["status"] = json!("waiting_display");
        item["startedAt"] = json!(js::now_ms());
        item["announced"] = json!([]);
        self.set_item(&item);
        self.save();
        let t = js::gs(&item["action"], "type");
        let (_, verb, say, _) = action_def(&t).unwrap_or(("", "", "", ""));
        o.emit(json!({ "type": "schedule_countdown", "id": item["id"], "verb": verb, "label": action_text(&item["action"]), "total": COUNTDOWN_S, "action": t }));
        o.notify(json!({
            "text": format!("{say} dans {} minutes. Dis annule pour l'arrêter.", COUNTDOWN_S / 60),
            "title": CARD_TITLE, "source": "programmation",
            "rows": [{ "icon": "⏳", "tone": "warn", "title": format!("{verb} dans {} minutes", COUNTDOWN_S / 60), "sub": "dis « annule », Échap ou clique ici pour annuler", "say": "Annule le compte à rebours", "hint": "cliquer pour annuler" }],
            "footer": format!("{COUNTDOWN_S} secondes affichées dans le cœur avant toute action")
        }));
    }

    /// Une fenêtre affiche le compte à rebours : il démarre.
    pub fn shown(&self, id: i64) -> J {
        let Some(mut item) = self.items.borrow().iter().find(|i| i["id"].as_i64() == Some(id)).cloned() else { return json!({ "ok": false }) };
        if js::gs(&item, "status") == "waiting_display" {
            item["status"] = json!("counting");
            item["endsAt"] = json!(js::now_ms() + COUNTDOWN_S * 1000);
            self.set_item(&item);
            self.save();
            let o = self.o.up();
            o.emit(json!({ "type": "schedule_countdown_started", "id": id, "endsAt": item["endsAt"], "total": COUNTDOWN_S }));
            o.record("programmation_compte_a_rebours", json!({ "id": id, "action": item["action"]["type"], "secondes": COUNTDOWN_S }));
        }
        json!({ "ok": js::gs(&item, "status") == "counting", "endsAt": if js::truthy(&item["endsAt"]) { item["endsAt"].clone() } else { Value::Null } })
    }

    pub async fn execute(&self, item: J, late: bool) -> Option<J> {
        let now = js::now_ms();
        let sensitive = is_sensitive(&item["action"]);
        // Garde-fou : une action sensible ne part jamais avant la fin des 120 s affichées.
        let current = self.items.borrow().iter().find(|i| i["id"] == item["id"]).cloned();
        let Some(item) = current else { return None };
        if sensitive && !(js::gs(&item, "status") == "counting" && now >= item["endsAt"].as_i64().unwrap_or(i64::MAX)) {
            return None;
        }
        let a = item["action"].clone();
        let id = item["id"].as_i64().unwrap_or(0);
        self.remove_id(id);
        self.save();
        let o = self.o.up();
        o.emit(json!({ "type": "schedule_countdown_end", "id": id, "reason": "exécutée" }));
        o.emit(json!({ "type": "widget_close", "key": format!("tool:{CARD_TITLE}") }));
        o.record("programmation_executee", json!({ "id": id, "action": a["type"], "sensible": sensitive, "enRetard": late }));
        let tag = if late { " (en retard)" } else { "" };
        let typ = js::gs(&a, "type");
        match typ.as_str() {
            "message" => o.notify(json!({
                "text": format!("{}{tag}", js::gs(&a, "text")), "title": "Programmation", "source": "programmation", "speak": false,
                "rows": [{ "icon": "⏰", "tone": if late { "warn" } else { "ok" }, "title": a["text"], "sub": if late { "programmation en retard" } else { "programmation" } }]
            })),
            "speak" => o.notify(json!({ "text": format!("{}{tag}", js::gs(&a, "text")), "title": "Programmation", "source": "programmation" })),
            "command" => o.submit(js::gs(&a, "phrase"), json!({ "source": "programmation" })).await,
            _ => {
                let t2 = typ.clone();
                let r = o.local_tool("mcp__system__programmation_action", json!({ "action": typ }), Box::new(move |_| Box::pin(async move { os_action(&t2).await }))).await;
                if r["ok"] == json!(false) || !js::truthy(&r) {
                    let label = action_def(&typ).map(|d| d.0).unwrap_or("");
                    let why = if js::gb(&r, "denied") { "refusé par les réglages de sécurité".to_string() } else { format!("impossible ({})", if js::truthy(&r["error"]) { js::gs(&r, "error") } else { "échec".into() }) };
                    o.notify(json!({ "text": format!("{label} : {why}."), "title": "Programmation", "source": "programmation" }));
                }
            }
        }
        Some(a)
    }

    /// Au démarrage : rien de sensible ne part seul après une interruption.
    fn recover(&self) {
        let now = js::now_ms();
        let items = self.items.borrow().clone();
        let mut changed = false;
        let o = self.o.up();
        for item in items {
            let status = js::gs(&item, "status");
            let id = item["id"].as_i64().unwrap_or(0);
            if status == "waiting_display" || status == "counting" {
                self.remove_id(id);
                o.record("programmation_annulee", json!({ "id": id, "action": item["action"]["type"], "motif": "Jarvis arrêté pendant le compte à rebours" }));
                o.notify(json!({ "text": format!("Jarvis s'est arrêté pendant le compte à rebours : « {} » est annulé.", action_text(&item["action"])), "title": "Programmation", "source": "programmation" }));
                changed = true;
            } else if status == "armed" && item["trigger"]["dueAt"].as_i64().map(|d| d <= now).unwrap_or(false) && is_sensitive(&item["action"]) {
                self.remove_id(id);
                o.record("programmation_annulee", json!({ "id": id, "action": item["action"]["type"], "motif": "heure passée pendant que Jarvis était fermé" }));
                o.notify(json!({
                    "text": format!("La programmation « {} » prévue {} n'a pas été exécutée : Jarvis était fermé.", action_text(&item["action"]), when_label(item["trigger"]["dueAt"].as_i64().unwrap_or(0))),
                    "title": "Programmation manquée", "source": "programmation"
                }));
                changed = true;
            }
        }
        if changed {
            self.save();
        }
    }

    pub fn start(&self) {
        self.recover();
        self.tick();
        let o = self.o.clone();
        self.timer.every(1000, move || o.up().schedules.tick());
    }

    pub fn stop(&self) {
        self.timer.clear();
    }
}
